//! Namespaces list view.

use ratatui::{
    layout::{Constraint, Margin, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{
        Cell, HighlightSpacing, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
        TableState,
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
        render_centered_message, table_viewport_rows, table_window,
        views::filtering::filtered_namespace_indices,
    },
};

pub fn render_namespaces(
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
        AppView::Namespaces,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.namespace_list, cluster.snapshot_version),
        |q| filtered_namespace_indices(&cluster.namespace_list, q),
    );

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::Namespaces,
            query,
            "Namespaces",
            "Loading namespaces...",
            "No namespaces found",
            "No namespaces match the search query",
            focused,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let header = Row::new([
        Cell::from(Span::styled("  NAME", theme.header_style())),
        Cell::from(Span::styled("STATUS", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &namespace_idx)| {
            let idx = window.start + local_idx;
            let namespace = &cluster.namespace_list[namespace_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let status_style = if namespace.status == "Active" {
                theme.badge_success_style()
            } else {
                theme.badge_error_style()
            };
            Row::new(vec![
                bookmarked_name_cell(
                    &ResourceRef::Namespace(namespace.name.clone()),
                    bookmarks,
                    namespace.name.as_str(),
                    Style::default().fg(theme.fg),
                    &theme,
                ),
                Cell::from(Span::styled(namespace.status.clone(), status_style)),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let title = if query.is_empty() {
        format!(" Namespaces ({total}) ")
    } else {
        let all = cluster.namespace_list.len();
        format!(" Namespaces ({total} of {all}) [/{query}]")
    };

    let table = Table::new(
        rows,
        [Constraint::Percentage(75), Constraint::Percentage(25)],
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
