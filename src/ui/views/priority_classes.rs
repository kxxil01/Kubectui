//! PriorityClasses list view.

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
        components::{active_block, default_block, default_theme},
        filter_cache::{cached_filter_indices, data_fingerprint},
        format_small_int, loading_or_empty_message, table_viewport_rows, table_window,
        views::filtering::filtered_priority_class_indices,
    },
};

// ── PriorityClass derived cell cache ────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct PriorityClassDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct PriorityClassDerivedCell {
    value: String,
    description_truncated: String,
}

type PriorityClassDerivedCacheValue = Arc<Vec<PriorityClassDerivedCell>>;
static PRIORITY_CLASS_DERIVED_CACHE: LazyLock<
    Mutex<Option<(PriorityClassDerivedCacheKey, PriorityClassDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

fn cached_priority_class_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> PriorityClassDerivedCacheValue {
    let key = PriorityClassDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.priority_classes, snapshot.snapshot_version),
    };

    if let Ok(cache) = PRIORITY_CLASS_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&pc_idx| {
                let pc = &snapshot.priority_classes[pc_idx];
                PriorityClassDerivedCell {
                    value: format_small_int(i64::from(pc.value)).into_owned(),
                    description_truncated: pc.description.chars().take(60).collect(),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = PRIORITY_CLASS_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

pub fn render_priority_classes(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    search: &str,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = cached_filter_indices(
        AppView::PriorityClasses,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.priority_classes, cluster.snapshot_version),
        |q| filtered_priority_class_indices(&cluster.priority_classes, q),
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::PriorityClasses,
            query,
            "  Loading priority classes...",
            "  No priority classes found",
            "  No priority classes match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("PriorityClasses")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let header = Row::new([
        Cell::from(Span::styled("  NAME", theme.header_style())),
        Cell::from(Span::styled("VALUE", theme.header_style())),
        Cell::from(Span::styled("GLOBAL DEFAULT", theme.header_style())),
        Cell::from(Span::styled("DESCRIPTION", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let derived = cached_priority_class_derived(cluster, query, &indices);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &priority_class_idx)| {
            let idx = window.start + local_idx;
            let priority_class = &cluster.priority_classes[priority_class_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let default_label = if priority_class.global_default {
                "✓"
            } else {
                ""
            };
            let (value, desc): (Cow<'_, str>, Cow<'_, str>) = if let Some(cell) = derived.get(idx) {
                (
                    Cow::Borrowed(cell.value.as_str()),
                    Cow::Borrowed(cell.description_truncated.as_str()),
                )
            } else {
                (
                    format_small_int(i64::from(priority_class.value)),
                    Cow::Owned(priority_class.description.chars().take(60).collect()),
                )
            };
            Row::new(vec![
                bookmarked_name_cell(
                    &ResourceRef::PriorityClass(priority_class.name.clone()),
                    bookmarks,
                    priority_class.name.as_str(),
                    Style::default().fg(theme.fg),
                    &theme,
                ),
                Cell::from(Span::styled(value, Style::default().fg(theme.info))),
                Cell::from(Span::styled(
                    default_label,
                    if priority_class.global_default {
                        Style::default().fg(theme.success)
                    } else {
                        Style::default().fg(theme.muted)
                    },
                )),
                Cell::from(Span::styled(desc, Style::default().fg(theme.fg_dim))),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let title = if query.is_empty() {
        format!(" PriorityClasses ({total}) ")
    } else {
        let all = cluster.priority_classes.len();
        format!(" PriorityClasses ({total} of {all}) [/{query}]")
    };

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(10),
            Constraint::Percentage(15),
            Constraint::Percentage(45),
        ],
    )
    .header(header)
    .block(active_block(&title))
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
