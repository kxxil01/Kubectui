//! Events list view.

use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

use ratatui::{
    layout::{Constraint, Margin, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{
        Cell, HighlightSpacing, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState,
    },
};

use crate::{
    app::{AppView, ResourceRef},
    bookmarks::BookmarkEntry,
    state::ClusterSnapshot,
    ui::{
        bookmarked_name_cell,
        components::{content_block, default_theme},
        filter_cache::{cached_filter_indices, data_fingerprint},
        format_small_int, render_centered_message, responsive_table_widths, table_viewport_rows,
        table_window,
        views::filtering::filtered_event_indices,
    },
};

// ── Event derived cell cache ────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct EventDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct EventDerivedCell {
    count: String,
    message_truncated: String,
}

type EventDerivedCacheValue = Arc<Vec<EventDerivedCell>>;
static EVENT_DERIVED_CACHE: LazyLock<
    Mutex<Option<(EventDerivedCacheKey, EventDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

fn cached_event_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> EventDerivedCacheValue {
    let key = EventDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.events, snapshot.snapshot_version),
    };

    if let Ok(cache) = EVENT_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&ev_idx| {
                let ev = &snapshot.events[ev_idx];
                EventDerivedCell {
                    count: format_small_int(i64::from(ev.count)).into_owned(),
                    message_truncated: ev.message.chars().take(60).collect(),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = EVENT_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

pub fn render_events(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    search: &str,
    focused: bool,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = cached_filter_indices(
        AppView::Events,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.events, cluster.snapshot_version),
        |q| filtered_event_indices(&cluster.events, q),
    );

    if indices.is_empty() {
        if cluster.events.is_empty()
            && let Some(error) = cluster.events_last_error.as_deref()
        {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    format!("Failed to load events: {error}"),
                    theme.inactive_style(),
                ))
                .alignment(ratatui::layout::Alignment::Center)
                .block(crate::ui::components::content_block("Events", focused)),
                area,
            );
            return;
        }
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::Events,
            query,
            "Events",
            "Loading events...",
            "No events found",
            "No events match the search query",
            focused,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let header = Row::new([
        Cell::from(Span::styled("  TYPE", theme.header_style())),
        Cell::from(Span::styled("NAMESPACE", theme.header_style())),
        Cell::from(Span::styled("OBJECT", theme.header_style())),
        Cell::from(Span::styled("REASON", theme.header_style())),
        Cell::from(Span::styled("COUNT", theme.header_style())),
        Cell::from(Span::styled("MESSAGE", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let derived = cached_event_derived(cluster, query, &indices);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &event_idx)| {
            let idx = window.start + local_idx;
            let ev = &cluster.events[event_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let type_style = if ev.type_ == "Warning" {
                theme.badge_warning_style()
            } else {
                theme.badge_success_style()
            };
            let (count, msg): (Cow<'_, str>, Cow<'_, str>) = if let Some(cell) = derived.get(idx) {
                (
                    Cow::Borrowed(cell.count.as_str()),
                    Cow::Borrowed(cell.message_truncated.as_str()),
                )
            } else {
                (
                    format_small_int(i64::from(ev.count)),
                    Cow::Owned(ev.message.chars().take(60).collect()),
                )
            };
            Row::new(vec![
                bookmarked_name_cell(
                    &ResourceRef::Event(ev.name.clone(), ev.namespace.clone()),
                    bookmarks,
                    ev.type_.as_str(),
                    type_style,
                    &theme,
                ),
                Cell::from(ev.namespace.clone()),
                Cell::from(ev.involved_object.clone()),
                Cell::from(ev.reason.clone()),
                Cell::from(Span::from(count)),
                Cell::from(Span::from(msg)),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let title = if query.is_empty() {
        format!(
            " Events ({total}){} ",
            match cluster.view_load_state(AppView::Events) {
                crate::state::ViewLoadState::Refreshing => " [refreshing]",
                crate::state::ViewLoadState::Loading => " [loading]",
                _ => "",
            }
        )
    } else {
        let all = cluster.events.len();
        format!(
            " Events ({total} of {all}) [/{query}]{}",
            match cluster.view_load_state(AppView::Events) {
                crate::state::ViewLoadState::Refreshing => " [refreshing]",
                crate::state::ViewLoadState::Loading => " [loading]",
                _ => "",
            }
        )
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Length(10),
                Constraint::Length(16),
                Constraint::Length(24),
                Constraint::Length(16),
                Constraint::Length(8),
                Constraint::Min(20),
            ],
        ),
    )
    .header(header)
    .block(content_block(&title, focused))
    .row_highlight_style(theme.selection_style())
    .highlight_symbol(theme.highlight_symbol())
    .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(table, area, &mut table_state);

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"))
        .track_symbol(Some("│"))
        .thumb_symbol("█");

    let mut scrollbar_state = ScrollbarState::new(total).position(selected);
    frame.render_stateful_widget(
        scrollbar,
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}
