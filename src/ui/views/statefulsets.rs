//! StatefulSets list rendering.

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
        format_age, format_image, render_resource_table, sort_header_cell, striped_row_style,
        views::filtering::filtered_statefulset_indices,
        workload_sort_suffix,
    },
};

#[derive(Debug, Clone)]
struct StatefulSetDerivedCell {
    ready: String,
    image: String,
    age: String,
}

type StatefulSetDerivedCacheValue = DerivedRowsCacheValue<StatefulSetDerivedCell>;
static STATEFULSET_DERIVED_CACHE: LazyLock<DerivedRowsCache<StatefulSetDerivedCell>> =
    LazyLock::new(Default::default);

/// Renders the StatefulSets table with stateful selection and scrollbar.
#[allow(clippy::too_many_arguments)]
pub fn render_statefulsets(
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
        AppView::StatefulSets,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.statefulsets, cluster.snapshot_version),
        cache_variant,
        |q| filtered_statefulset_indices(&cluster.statefulsets, q, sort),
    );
    let derived = cached_statefulset_derived(cluster, query, indices.as_ref(), cache_variant);
    let widths = [
        Constraint::Length(22),
        Constraint::Length(16),
        Constraint::Length(10),
        Constraint::Length(22),
        Constraint::Min(20),
        Constraint::Length(9),
    ];
    let sort_suffix = workload_sort_suffix(sort);
    render_resource_table(
        frame,
        area,
        &theme,
        ResourceTableConfig {
            snapshot: cluster,
            view: AppView::StatefulSets,
            label: "StatefulSets",
            loading_message: "Loading statefulsets...",
            empty_message: "No statefulsets found",
            empty_query_message: "No statefulsets match the search query",
            query,
            focused,
            filtered_total: indices.len(),
            all_total: cluster.statefulsets.len(),
            selected_idx,
            widths: &widths,
            sort_suffix: &sort_suffix,
        },
        |theme| {
            Row::new([
                sort_header_cell("Name", sort, WorkloadSortColumn::Name, theme, true),
                Cell::from(Span::styled("Namespace", theme.header_style())),
                Cell::from(Span::styled("Ready", theme.header_style())),
                Cell::from(Span::styled("Service", theme.header_style())),
                Cell::from(Span::styled("Image", theme.header_style())),
                sort_header_cell("Age", sort, WorkloadSortColumn::Age, theme, false),
            ])
            .height(1)
            .style(theme.header_style())
        },
        |window, theme| {
            indices[window.start..window.end]
                .iter()
                .enumerate()
                .map(|(local_idx, &ss_idx)| {
                    let idx = window.start + local_idx;
                    let ss = &cluster.statefulsets[ss_idx];
                    let (ready, image, age) = if let Some(cell) = derived.get(idx) {
                        (
                            Cow::Borrowed(cell.ready.as_str()),
                            Cow::Borrowed(cell.image.as_str()),
                            Cow::Borrowed(cell.age.as_str()),
                        )
                    } else {
                        (
                            Cow::Owned(format!("{}/{}", ss.ready_replicas, ss.desired_replicas)),
                            Cow::Owned(format_image(ss.image.as_deref(), 30)),
                            Cow::Owned(format_age(ss.age)),
                        )
                    };
                    let ready_style =
                        readiness_style(ss.ready_replicas, ss.desired_replicas, theme);

                    Row::new(vec![
                        bookmarked_name_cell(
                            &ResourceRef::StatefulSet(ss.name.clone(), ss.namespace.clone()),
                            bookmarks,
                            ss.name.as_str(),
                            Style::default().fg(theme.fg),
                            theme,
                        ),
                        Cell::from(Span::styled(
                            ss.namespace.clone(),
                            Style::default().fg(theme.fg_dim),
                        )),
                        Cell::from(Span::styled(ready, ready_style)),
                        Cell::from(Span::styled(
                            ss.service_name.clone(),
                            Style::default().fg(theme.info),
                        )),
                        Cell::from(Span::styled(image, Style::default().fg(theme.muted))),
                        Cell::from(Span::styled(age, theme.inactive_style())),
                    ])
                    .style(striped_row_style(idx, theme))
                })
                .collect()
        },
    );
}

fn cached_statefulset_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    variant: u64,
) -> StatefulSetDerivedCacheValue {
    let key = DerivedRowsCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(&cluster.statefulsets, cluster.snapshot_version),
        variant,
        freshness_bucket: 0,
    };

    cached_derived_rows(&STATEFULSET_DERIVED_CACHE, key, || {
        indices
            .iter()
            .map(|&ss_idx| {
                let ss = &cluster.statefulsets[ss_idx];
                StatefulSetDerivedCell {
                    ready: format!("{}/{}", ss.ready_replicas, ss.desired_replicas),
                    image: format_image(ss.image.as_deref(), 30),
                    age: format_age(ss.age),
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
