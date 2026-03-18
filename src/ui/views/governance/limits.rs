//! LimitRanges list rendering.

use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Row},
};

use crate::{
    app::{AppView, ResourceRef, WorkloadSortColumn, WorkloadSortState},
    bookmarks::BookmarkEntry,
    icons::view_icon,
    state::ClusterSnapshot,
    ui::{
        TableFrame, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{cached_filter_indices_with_variant, data_fingerprint},
        format_age, format_small_int, render_centered_message, render_table_frame,
        resource_table_title, sort_header_cell, table_viewport_rows, table_window,
        views::filtering::filtered_limit_range_indices,
        workload_sort_suffix,
    },
};

// ── LimitRange derived cell cache ───────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct LimitRangeDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
    variant: u64,
}

#[derive(Debug, Clone)]
struct LimitRangeDerivedCell {
    specs_count: String,
    types_summary: String,
    age: String,
}

type LimitRangeDerivedCacheValue = Arc<Vec<LimitRangeDerivedCell>>;
static LIMIT_RANGE_DERIVED_CACHE: LazyLock<
    Mutex<Option<(LimitRangeDerivedCacheKey, LimitRangeDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

fn cached_limit_range_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    variant: u64,
) -> LimitRangeDerivedCacheValue {
    let key = LimitRangeDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.limit_ranges, snapshot.snapshot_version),
        variant,
    };

    if let Ok(cache) = LIMIT_RANGE_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&lr_idx| {
                let lr = &snapshot.limit_ranges[lr_idx];
                LimitRangeDerivedCell {
                    specs_count: format_small_int(lr.limits.len() as i64).into_owned(),
                    types_summary: limit_types_summary(lr),
                    age: format_age(lr.age),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = LIMIT_RANGE_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

#[allow(clippy::too_many_arguments)]
pub fn render_limit_ranges(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    query: &str,
    sort: Option<WorkloadSortState>,
    focused: bool,
) {
    let query = query.trim();
    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::LimitRanges,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.limit_ranges, cluster.snapshot_version),
        cache_variant,
        |q| filtered_limit_range_indices(&cluster.limit_ranges, q, sort),
    );

    let theme = default_theme();

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::LimitRanges,
            query,
            "LimitRanges",
            "Loading limit ranges...",
            "No limit ranges found",
            "No limit ranges match the search query",
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
        Cell::from(Span::styled("Specs", theme.header_style())),
        Cell::from(Span::styled("Types", theme.header_style())),
        sort_header_cell("Age", sort, WorkloadSortColumn::Age, &theme, false),
    ])
    .height(1)
    .style(theme.header_style());

    let derived = cached_limit_range_derived(cluster, query, &indices, cache_variant);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &lr_idx)| {
            let idx = window.start + local_idx;
            let lr = &cluster.limit_ranges[lr_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let (specs_count, types_summary, age): (Cow<'_, str>, Cow<'_, str>, Cow<'_, str>) =
                if let Some(cell) = derived.get(idx) {
                    (
                        Cow::Borrowed(cell.specs_count.as_str()),
                        Cow::Borrowed(cell.types_summary.as_str()),
                        Cow::Borrowed(cell.age.as_str()),
                    )
                } else {
                    (
                        format_small_int(lr.limits.len() as i64),
                        Cow::Owned(limit_types_summary(lr)),
                        Cow::Owned(format_age(lr.age)),
                    )
                };
            Row::new(vec![
                bookmarked_name_cell(
                    &ResourceRef::LimitRange(lr.name.clone(), lr.namespace.clone()),
                    bookmarks,
                    lr.name.as_str(),
                    Style::default().fg(theme.fg),
                    &theme,
                ),
                Cell::from(Span::styled(
                    lr.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(specs_count, Style::default().fg(theme.fg_dim))),
                Cell::from(Span::styled(
                    types_summary,
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let sort_suffix = workload_sort_suffix(sort);
    let title = resource_table_title(
        view_icon(AppView::LimitRanges).active(),
        "LimitRanges",
        total,
        cluster.limit_ranges.len(),
        query,
        &sort_suffix,
    );
    let widths = [
        Constraint::Min(28),
        Constraint::Length(18),
        Constraint::Length(8),
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

fn limit_types_summary(lr: &crate::k8s::dtos::LimitRangeInfo) -> String {
    let mut types = lr
        .limits
        .iter()
        .map(|spec| spec.type_.clone())
        .collect::<Vec<_>>();
    types.sort();
    types.dedup();

    if types.is_empty() {
        "-".to_string()
    } else {
        types.join(",")
    }
}

#[cfg(test)]
mod tests {
    use crate::k8s::dtos::{LimitRangeInfo, LimitSpec};

    use super::*;

    #[test]
    fn summary_deduplicates_types() {
        let info = LimitRangeInfo {
            limits: vec![
                LimitSpec {
                    type_: "Container".to_string(),
                    ..LimitSpec::default()
                },
                LimitSpec {
                    type_: "Container".to_string(),
                    ..LimitSpec::default()
                },
                LimitSpec {
                    type_: "Pod".to_string(),
                    ..LimitSpec::default()
                },
            ],
            ..LimitRangeInfo::default()
        };

        assert_eq!(limit_types_summary(&info), "Container,Pod");
    }
}
