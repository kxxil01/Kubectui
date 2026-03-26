//! ReplicaSets list rendering.

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
        ResourceTableConfig, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{
            DerivedRowsCache, DerivedRowsCacheKey, DerivedRowsCacheValue, cached_derived_rows,
            cached_filter_indices_with_variant, data_fingerprint,
        },
        format_age, format_image, format_small_int, render_resource_table, sort_header_cell,
        striped_row_style,
        views::filtering::filtered_replicaset_indices,
        workload_sort_suffix,
    },
};

#[derive(Debug, Clone)]
struct ReplicaSetDerivedCell {
    image: String,
    age: String,
}

type ReplicaSetDerivedCacheValue = DerivedRowsCacheValue<ReplicaSetDerivedCell>;
static REPLICASET_DERIVED_CACHE: LazyLock<DerivedRowsCache<ReplicaSetDerivedCell>> =
    LazyLock::new(Default::default);

#[allow(clippy::too_many_arguments)]
pub fn render_replicasets(
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
        AppView::ReplicaSets,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.replicasets, cluster.snapshot_version),
        cache_variant,
        |q| filtered_replicaset_indices(&cluster.replicasets, q, sort),
    );
    let name_style = Style::default().fg(theme.fg);
    let dim_style = Style::default().fg(theme.fg_dim);
    let muted_style = Style::default().fg(theme.muted);
    let derived = cached_replicaset_derived(cluster, query, indices.as_ref(), cache_variant);
    let widths = [
        Constraint::Length(28),
        Constraint::Length(16),
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Length(11),
        Constraint::Min(24),
        Constraint::Length(9),
    ];
    let sort_suffix = workload_sort_suffix(sort);
    render_resource_table(
        frame,
        area,
        &theme,
        ResourceTableConfig {
            snapshot: cluster,
            view: AppView::ReplicaSets,
            label: "Replica Sets",
            loading_message: "Loading replica sets...",
            empty_message: "No replica sets found",
            empty_query_message: "No replica sets match the search query",
            query,
            focused,
            filtered_total: indices.len(),
            all_total: cluster.replicasets.len(),
            selected_idx,
            widths: &widths,
            sort_suffix: &sort_suffix,
        },
        |theme| {
            Row::new([
                sort_header_cell("Name", sort, WorkloadSortColumn::Name, theme, true),
                Cell::from(Span::styled("Namespace", theme.header_style())),
                Cell::from(Span::styled("Desired", theme.header_style())),
                Cell::from(Span::styled("Ready", theme.header_style())),
                Cell::from(Span::styled("Available", theme.header_style())),
                Cell::from(Span::styled("Image", theme.header_style())),
                sort_header_cell("Age", sort, WorkloadSortColumn::Age, theme, false),
            ])
            .height(1)
            .style(theme.header_style())
        },
        |window, theme| {
            let mut rows: Vec<Row> = Vec::with_capacity(window.end.saturating_sub(window.start));
            for (local_idx, &rs_idx) in indices[window.start..window.end].iter().enumerate() {
                let idx = window.start + local_idx;
                let rs = &cluster.replicasets[rs_idx];
                let ready_style = readiness_style(rs.ready, rs.desired, theme);
                let (image, age) = if let Some(cell) = derived.get(idx) {
                    (
                        Cow::Borrowed(cell.image.as_str()),
                        Cow::Borrowed(cell.age.as_str()),
                    )
                } else {
                    (
                        Cow::Owned(format_image(rs.image.as_deref(), 32)),
                        Cow::Owned(format_age(rs.age)),
                    )
                };

                rows.push(
                    Row::new(vec![
                        bookmarked_name_cell(
                            || ResourceRef::ReplicaSet(rs.name.clone(), rs.namespace.clone()),
                            bookmarks,
                            rs.name.as_str(),
                            name_style,
                            theme,
                        ),
                        Cell::from(Span::styled(rs.namespace.as_str(), dim_style)),
                        Cell::from(Span::styled(
                            format_small_int(i64::from(rs.desired)),
                            dim_style,
                        )),
                        Cell::from(Span::styled(
                            format_small_int(i64::from(rs.ready)),
                            ready_style,
                        )),
                        Cell::from(Span::styled(
                            format_small_int(i64::from(rs.available)),
                            dim_style,
                        )),
                        Cell::from(Span::styled(image, muted_style)),
                        Cell::from(Span::styled(age, theme.inactive_style())),
                    ])
                    .style(striped_row_style(idx, theme)),
                );
            }
            rows
        },
    );
}

fn cached_replicaset_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    variant: u64,
) -> ReplicaSetDerivedCacheValue {
    let key = DerivedRowsCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(&cluster.replicasets, cluster.snapshot_version),
        variant,
        freshness_bucket: 0,
    };

    cached_derived_rows(&REPLICASET_DERIVED_CACHE, key, || {
        indices
            .iter()
            .map(|&rs_idx| {
                let rs = &cluster.replicasets[rs_idx];
                ReplicaSetDerivedCell {
                    image: format_image(rs.image.as_deref(), 32),
                    age: format_age(rs.age),
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
        assert_eq!(readiness_style(3, 3, &theme).fg, Some(theme.success));
        assert_eq!(readiness_style(1, 3, &theme).fg, Some(theme.warning));
        assert_eq!(readiness_style(0, 3, &theme).fg, Some(theme.error));
    }
}
