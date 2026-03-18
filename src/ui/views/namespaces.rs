//! Namespaces list view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Row},
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
        render_centered_message, render_table_frame, resource_table_title, table_viewport_rows,
        table_window,
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

    let title = resource_table_title(
        view_icon(AppView::Namespaces).active(),
        "Namespaces",
        total,
        cluster.namespace_list.len(),
        query,
        "",
    );
    let widths = [Constraint::Percentage(75), Constraint::Percentage(25)];

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
