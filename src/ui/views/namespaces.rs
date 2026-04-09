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
    state::ClusterSnapshot,
    ui::{
        ResourceTableConfig, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{cached_filter_indices, data_fingerprint},
        render_resource_table, striped_row_style,
        views::filtering::filtered_namespace_indices,
    },
};

const NARROW_NAMESPACE_WIDTH: u16 = 72;

fn namespace_widths(area: Rect) -> [Constraint; 2] {
    if area.width < NARROW_NAMESPACE_WIDTH {
        [Constraint::Min(18), Constraint::Length(10)]
    } else {
        [Constraint::Percentage(75), Constraint::Percentage(25)]
    }
}

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
    let widths = namespace_widths(area);
    render_resource_table(
        frame,
        area,
        &theme,
        ResourceTableConfig {
            snapshot: cluster,
            view: AppView::Namespaces,
            label: "Namespaces",
            loading_message: "Loading namespaces...",
            empty_message: "No namespaces found",
            empty_query_message: "No namespaces match the search query",
            query,
            focused,
            filtered_total: indices.len(),
            all_total: cluster.namespace_list.len(),
            selected_idx,
            widths: &widths,
            sort_suffix: "",
        },
        |theme| {
            Row::new([
                Cell::from(Span::styled("  NAME", theme.header_style())),
                Cell::from(Span::styled("STATUS", theme.header_style())),
            ])
            .style(theme.header_style())
            .height(1)
        },
        |window, theme| {
            indices[window.start..window.end]
                .iter()
                .enumerate()
                .map(|(local_idx, &namespace_idx)| {
                    let idx = window.start + local_idx;
                    let namespace = &cluster.namespace_list[namespace_idx];
                    let status_style = if namespace.status == "Active" {
                        theme.badge_success_style()
                    } else {
                        theme.badge_error_style()
                    };
                    Row::new(vec![
                        bookmarked_name_cell(
                            || ResourceRef::Namespace(namespace.name.clone()),
                            bookmarks,
                            namespace.name.as_str(),
                            Style::default().fg(theme.fg),
                            theme,
                        ),
                        Cell::from(Span::styled(namespace.status.clone(), status_style)),
                    ])
                    .style(striped_row_style(idx, theme))
                })
                .collect()
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_widths_switch_to_compact_profile() {
        let widths = namespace_widths(Rect::new(0, 0, 64, 20));
        assert_eq!(widths[0], Constraint::Min(18));
        assert_eq!(widths[1], Constraint::Length(10));
    }

    #[test]
    fn namespace_widths_keep_wide_profile() {
        let widths = namespace_widths(Rect::new(0, 0, 96, 20));
        assert_eq!(widths[0], Constraint::Percentage(75));
        assert_eq!(widths[1], Constraint::Percentage(25));
    }
}
