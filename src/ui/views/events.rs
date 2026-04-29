//! Events list view.

use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Paragraph, Row},
};

use crate::{
    app::{AppView, ResourceRef},
    bookmarks::BookmarkEntry,
    icons::view_icon,
    state::ClusterSnapshot,
    ui::{
        TableFrame, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{cached_filter_indices, data_fingerprint},
        format_small_int, render_centered_message, render_table_frame, responsive_table_widths,
        table_viewport_rows, table_window, truncate_message,
        views::filtering::filtered_event_indices,
    },
};

const NARROW_EVENT_WIDTH: u16 = 104;

fn event_widths(area: Rect) -> [Constraint; 6] {
    if area.width < NARROW_EVENT_WIDTH {
        [
            Constraint::Length(8),
            Constraint::Length(14),
            Constraint::Min(18),
            Constraint::Length(12),
            Constraint::Length(6),
            Constraint::Min(16),
        ]
    } else {
        [
            Constraint::Length(10),
            Constraint::Length(16),
            Constraint::Length(24),
            Constraint::Length(16),
            Constraint::Length(8),
            Constraint::Min(20),
        ]
    }
}

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
                    message_truncated: truncate_message(&ev.message, 60).into_owned(),
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
                    Cow::Owned(truncate_message(&ev.message, 60).into_owned()),
                )
            };
            Row::new(vec![
                bookmarked_name_cell(
                    || ResourceRef::Event(ev.name.clone(), ev.namespace.clone()),
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

    let load_suffix = match cluster.view_load_state(AppView::Events) {
        crate::state::ViewLoadState::Refreshing => " [refreshing]",
        crate::state::ViewLoadState::Loading => " [loading]",
        _ => "",
    };
    let icon = view_icon(AppView::Events).active();
    let title = if query.is_empty() {
        format!(" {icon}Events ({total}){load_suffix} ")
    } else {
        let all = cluster.events.len();
        format!(" {icon}Events ({total} of {all}) [/{query}]{load_suffix}")
    };
    let widths = responsive_table_widths(area.width, event_widths(area));

    render_table_frame(
        frame,
        area,
        TableFrame {
            rows,
            header,
            widths: &widths,
            title: &title,
            focused,
            window,
            total,
            selected,
        },
        &theme,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{k8s::dtos::K8sEventInfo, state::ClusterSnapshot};

    #[test]
    fn event_widths_switch_to_compact_profile() {
        let widths = event_widths(Rect::new(0, 0, 96, 20));
        assert_eq!(widths[0], Constraint::Length(8));
        assert_eq!(widths[1], Constraint::Length(14));
        assert_eq!(widths[2], Constraint::Min(18));
        assert_eq!(widths[5], Constraint::Min(16));
    }

    #[test]
    fn event_widths_keep_wide_profile() {
        let widths = event_widths(Rect::new(0, 0, 120, 20));
        assert_eq!(widths[0], Constraint::Length(10));
        assert_eq!(widths[1], Constraint::Length(16));
        assert_eq!(widths[2], Constraint::Length(24));
        assert_eq!(widths[5], Constraint::Min(20));
    }

    #[test]
    fn event_derived_message_uses_canonical_truncation() {
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 17,
            ..ClusterSnapshot::default()
        };
        snapshot.events = vec![K8sEventInfo {
            message: "x".repeat(80),
            count: 7,
            ..K8sEventInfo::default()
        }];

        let derived = cached_event_derived(&snapshot, "", &[0]);

        assert_eq!(derived[0].message_truncated.len(), 60);
        assert!(derived[0].message_truncated.ends_with("..."));
    }
}
