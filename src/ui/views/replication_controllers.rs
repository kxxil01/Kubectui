//! ReplicationControllers list rendering.

use std::{borrow::Cow, sync::LazyLock};

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Row},
};

use crate::{
    app::{AppView, ResourceRef, WorkloadSortColumn, WorkloadSortState},
    bookmarks::BookmarkEntry,
    state::ClusterSnapshot,
    ui::{
        TableFrame, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{
            DerivedRowsCache, DerivedRowsCacheKey, DerivedRowsCacheValue, cached_derived_rows,
            cached_filter_indices_with_variant, data_fingerprint,
        },
        format_age, format_image, format_small_int, render_centered_message, render_table_frame,
        sort_header_cell, table_viewport_rows, table_window,
        views::filtering::filtered_replication_controller_indices,
        workload_sort_suffix,
    },
};

#[derive(Debug, Clone)]
struct ReplicationControllerDerivedCell {
    image: String,
    age: String,
}

type ReplicationControllerDerivedCacheValue =
    DerivedRowsCacheValue<ReplicationControllerDerivedCell>;
static REPLICATION_CONTROLLER_DERIVED_CACHE: LazyLock<
    DerivedRowsCache<ReplicationControllerDerivedCell>,
> = LazyLock::new(Default::default);

#[allow(clippy::too_many_arguments)]
pub fn render_replication_controllers(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    query: &str,
    sort: Option<WorkloadSortState>,
    focused: bool,
) {
    let theme = default_theme();
    let query = query.trim();
    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::ReplicationControllers,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.replication_controllers, cluster.snapshot_version),
        cache_variant,
        |q| filtered_replication_controller_indices(&cluster.replication_controllers, q, sort),
    );

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::ReplicationControllers,
            query,
            "Replication Controllers",
            "Loading replication controllers...",
            "No replication controllers found",
            "No replication controllers match the search query",
            focused,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));
    let header = Row::new([
        sort_header_cell("Name", sort, WorkloadSortColumn::Name, &theme, true),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Desired", theme.header_style())),
        Cell::from(Span::styled("Ready", theme.header_style())),
        Cell::from(Span::styled("Available", theme.header_style())),
        Cell::from(Span::styled("Image", theme.header_style())),
        sort_header_cell("Age", sort, WorkloadSortColumn::Age, &theme, false),
    ])
    .height(1)
    .style(theme.header_style());
    let name_style = Style::default().fg(theme.fg);
    let dim_style = Style::default().fg(theme.fg_dim);
    let muted_style = Style::default().fg(theme.muted);
    let derived =
        cached_replication_controller_derived(cluster, query, indices.as_ref(), cache_variant);

    let mut rows: Vec<Row> = Vec::with_capacity(window.end.saturating_sub(window.start));
    for (local_idx, &rc_idx) in indices[window.start..window.end].iter().enumerate() {
        let idx = window.start + local_idx;
        let rc = &cluster.replication_controllers[rc_idx];
        let ready_style = readiness_style(rc.ready, rc.desired, &theme);
        let (image, age) = if let Some(cell) = derived.get(idx) {
            (
                Cow::Borrowed(cell.image.as_str()),
                Cow::Borrowed(cell.age.as_str()),
            )
        } else {
            (
                Cow::Owned(format_image(rc.image.as_deref(), 32)),
                Cow::Owned(format_age(rc.age)),
            )
        };
        let row_style = if idx.is_multiple_of(2) {
            Style::default().bg(theme.bg)
        } else {
            theme.row_alt_style()
        };

        rows.push(
            Row::new(vec![
                bookmarked_name_cell(
                    &ResourceRef::ReplicationController(rc.name.clone(), rc.namespace.clone()),
                    bookmarks,
                    rc.name.as_str(),
                    name_style,
                    &theme,
                ),
                Cell::from(Span::styled(rc.namespace.as_str(), dim_style)),
                Cell::from(Span::styled(
                    format_small_int(i64::from(rc.desired)),
                    dim_style,
                )),
                Cell::from(Span::styled(
                    format_small_int(i64::from(rc.ready)),
                    ready_style,
                )),
                Cell::from(Span::styled(
                    format_small_int(i64::from(rc.available)),
                    dim_style,
                )),
                Cell::from(Span::styled(image, muted_style)),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style),
        );
    }

    let sort_suffix = workload_sort_suffix(sort);
    let title = if query.is_empty() {
        format!(" Replication Controllers ({total}){sort_suffix} ")
    } else {
        let all = cluster.replication_controllers.len();
        format!(" Replication Controllers ({total} of {all}) [/{query}]{sort_suffix}")
    };
    let widths = [
        Constraint::Length(28),
        Constraint::Length(16),
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Length(11),
        Constraint::Min(24),
        Constraint::Length(9),
    ];
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

fn cached_replication_controller_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    variant: u64,
) -> ReplicationControllerDerivedCacheValue {
    let key = DerivedRowsCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(
            &cluster.replication_controllers,
            cluster.snapshot_version,
        ),
        variant,
        freshness_bucket: 0,
    };

    cached_derived_rows(&REPLICATION_CONTROLLER_DERIVED_CACHE, key, || {
        indices
            .iter()
            .map(|&rc_idx| {
                let rc = &cluster.replication_controllers[rc_idx];
                ReplicationControllerDerivedCell {
                    image: format_image(rc.image.as_deref(), 32),
                    age: format_age(rc.age),
                }
            })
            .collect()
    })
}

use crate::ui::readiness_style;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Theme;

    #[test]
    fn readiness_style_maps_to_expected_colors() {
        let theme = Theme::dark();
        assert_eq!(readiness_style(2, 2, &theme).fg, Some(theme.success));
        assert_eq!(readiness_style(1, 2, &theme).fg, Some(theme.warning));
        assert_eq!(readiness_style(0, 2, &theme).fg, Some(theme.error));
    }
}
