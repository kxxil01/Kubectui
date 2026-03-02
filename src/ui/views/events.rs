//! Events list view.

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
    app::AppView,
    state::ClusterSnapshot,
    ui::{
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices, data_fingerprint},
        format_small_int, loading_or_empty_message, table_viewport_rows, table_window,
    },
};

pub fn render_events(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search: &str,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = cached_filter_indices(
        AppView::Events,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.events),
        |q| {
            if q.is_empty() {
                return (0..cluster.events.len()).collect();
            }
            cluster
                .events
                .iter()
                .enumerate()
                .filter_map(|(idx, ev)| {
                    (contains_ci(&ev.type_, q)
                        || contains_ci(&ev.namespace, q)
                        || contains_ci(&ev.involved_object, q)
                        || contains_ci(&ev.reason, q)
                        || contains_ci(&ev.message, q))
                    .then_some(idx)
                })
                .collect()
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            query,
            "  Loading events...",
            "  No events found",
            "  No events match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("Events")),
            area,
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
            Row::new(vec![
                Cell::from(Span::styled(ev.type_.clone(), type_style)),
                Cell::from(ev.namespace.clone()),
                Cell::from(ev.involved_object.clone()),
                Cell::from(ev.reason.clone()),
                Cell::from(format_small_int(i64::from(ev.count))),
                Cell::from(ev.message.chars().take(60).collect::<String>()),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let title = if query.is_empty() {
        format!(" Events ({total}) ")
    } else {
        let all = cluster.events.len();
        format!(" Events ({total} of {all}) [/{query}]")
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(16),
            Constraint::Length(24),
            Constraint::Length(16),
            Constraint::Length(8),
            Constraint::Min(20),
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
