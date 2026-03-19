//! CronJobs list rendering.

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
    state::ClusterSnapshot,
    time::{AppTimestamp, format_local},
    ui::{
        ResourceTableConfig, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{cached_filter_indices_with_variant, data_fingerprint},
        format_age, format_small_int, render_resource_table, sort_header_cell, striped_row_style,
        views::filtering::filtered_cronjob_indices,
        workload_sort_suffix,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct CronJobDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
    variant: u64,
}

#[derive(Debug, Clone)]
struct CronJobDerivedCell {
    last_run: String,
    next_run: String,
    age: String,
}

type CronJobDerivedCacheValue = Arc<Vec<CronJobDerivedCell>>;
static CRONJOB_DERIVED_CACHE: LazyLock<
    Mutex<Option<(CronJobDerivedCacheKey, CronJobDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

#[allow(clippy::too_many_arguments)]
pub fn render_cronjobs(
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
        AppView::CronJobs,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.cronjobs, cluster.snapshot_version),
        cache_variant,
        |q| filtered_cronjob_indices(&cluster.cronjobs, q, sort),
    );
    let name_style = Style::default().fg(theme.fg);
    let dim_style = Style::default().fg(theme.fg_dim);
    let accent_style = Style::default().fg(theme.accent2);
    let info_style = Style::default().fg(theme.info);
    let derived = cached_cronjob_derived(cluster, query, indices.as_ref(), cache_variant);
    let widths = [
        Constraint::Length(20),
        Constraint::Length(16),
        Constraint::Length(16),
        Constraint::Length(14),
        Constraint::Length(14),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(9),
    ];
    let sort_suffix = workload_sort_suffix(sort);
    render_resource_table(
        frame,
        area,
        &theme,
        ResourceTableConfig {
            snapshot: cluster,
            view: AppView::CronJobs,
            label: "CronJobs",
            loading_message: "Loading cronjobs...",
            empty_message: "No cronjobs found",
            empty_query_message: "No cronjobs match the search query",
            query,
            focused,
            filtered_total: indices.len(),
            all_total: cluster.cronjobs.len(),
            selected_idx,
            widths: &widths,
            sort_suffix: &sort_suffix,
        },
        |theme| {
            Row::new([
                sort_header_cell("Name", sort, WorkloadSortColumn::Name, theme, true),
                Cell::from(Span::styled("Namespace", theme.header_style())),
                Cell::from(Span::styled("Schedule", theme.header_style())),
                Cell::from(Span::styled("Last Run", theme.header_style())),
                Cell::from(Span::styled("Next Run", theme.header_style())),
                Cell::from(Span::styled("Active", theme.header_style())),
                Cell::from(Span::styled("Suspend", theme.header_style())),
                sort_header_cell("Age", sort, WorkloadSortColumn::Age, theme, false),
            ])
            .height(1)
            .style(theme.header_style())
        },
        |window, theme| {
            let mut rows: Vec<Row> = Vec::with_capacity(window.end.saturating_sub(window.start));
            for (local_idx, &cj_idx) in indices[window.start..window.end].iter().enumerate() {
                let idx = window.start + local_idx;
                let cj = &cluster.cronjobs[cj_idx];
                let (last_run, next_run, age) = if let Some(cell) = derived.get(idx) {
                    (
                        Cow::Borrowed(cell.last_run.as_str()),
                        Cow::Borrowed(cell.next_run.as_str()),
                        Cow::Borrowed(cell.age.as_str()),
                    )
                } else {
                    (
                        Cow::Owned(format_time(cj.last_schedule_time)),
                        Cow::Owned(format_time(cj.next_schedule_time)),
                        Cow::Owned(format_age(cj.age)),
                    )
                };
                let suspend_style = if cj.suspend {
                    theme.badge_warning_style()
                } else {
                    theme.badge_success_style()
                };

                rows.push(
                    Row::new(vec![
                        bookmarked_name_cell(
                            &ResourceRef::CronJob(cj.name.clone(), cj.namespace.clone()),
                            bookmarks,
                            cj.name.as_str(),
                            name_style,
                            theme,
                        ),
                        Cell::from(Span::styled(cj.namespace.as_str(), dim_style)),
                        Cell::from(Span::styled(cj.schedule.as_str(), accent_style)),
                        Cell::from(Span::styled(last_run, dim_style)),
                        Cell::from(Span::styled(next_run, info_style)),
                        Cell::from(Span::styled(
                            format_small_int(i64::from(cj.active_jobs)),
                            if cj.active_jobs > 0 {
                                info_style
                            } else {
                                theme.inactive_style()
                            },
                        )),
                        Cell::from(Span::styled(suspend_label(cj.suspend), suspend_style)),
                        Cell::from(Span::styled(age, theme.inactive_style())),
                    ])
                    .style(striped_row_style(idx, theme)),
                );
            }
            rows
        },
    );
}

fn cached_cronjob_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    variant: u64,
) -> CronJobDerivedCacheValue {
    let key = CronJobDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(&cluster.cronjobs, cluster.snapshot_version),
        variant,
    };

    if let Ok(cache) = CRONJOB_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&cj_idx| {
                let cj = &cluster.cronjobs[cj_idx];
                CronJobDerivedCell {
                    last_run: format_time(cj.last_schedule_time),
                    next_run: format_time(cj.next_schedule_time),
                    age: format_age(cj.age),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = CRONJOB_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

fn suspend_label(suspend: bool) -> &'static str {
    if suspend { "● Paused" } else { "● Active" }
}

fn format_time(ts: Option<AppTimestamp>) -> String {
    if let Some(value) = ts {
        format_local(value, "%m-%d %H:%M")
    } else {
        "-".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suspend_label_values() {
        assert_eq!(suspend_label(true), "● Paused");
        assert_eq!(suspend_label(false), "● Active");
    }
}
