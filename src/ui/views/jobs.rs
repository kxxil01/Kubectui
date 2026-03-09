//! Jobs list rendering.

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
    app::{AppView, WorkloadSortColumn, WorkloadSortState, filtered_workload_indices},
    state::ClusterSnapshot,
    ui::{
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices_with_variant, data_fingerprint},
        format_age, format_small_int, loading_or_empty_message, responsive_table_widths,
        table_viewport_rows, table_window, workload_sort_header, workload_sort_suffix,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct JobDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct JobDerivedCell {
    duration: String,
    age: String,
}

type JobDerivedCacheValue = Arc<Vec<JobDerivedCell>>;
static JOB_DERIVED_CACHE: LazyLock<Mutex<Option<(JobDerivedCacheKey, JobDerivedCacheValue)>>> =
    LazyLock::new(|| Mutex::new(None));

pub fn render_jobs(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
    sort: Option<WorkloadSortState>,
) {
    let theme = default_theme();
    let query = query.trim();
    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::Jobs,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.jobs, cluster.snapshot_version),
        cache_variant,
        |q| {
            filtered_workload_indices(
                &cluster.jobs,
                q,
                sort,
                |job, needle| contains_ci(&job.name, needle) || contains_ci(&job.status, needle),
                |job| job.name.as_str(),
                |job| job.namespace.as_str(),
                |job| job.age,
            )
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::Jobs,
            query,
            "  Loading jobs...",
            "  No jobs found",
            "  No jobs match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style())).block(default_block("Jobs")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));
    let name_header = workload_sort_header("Name", sort, WorkloadSortColumn::Name);
    let age_header = workload_sort_header("Age", sort, WorkloadSortColumn::Age);

    let header = Row::new([
        Cell::from(Span::styled(
            format!("  {name_header}"),
            theme.header_style(),
        )),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Status", theme.header_style())),
        Cell::from(Span::styled("Completions", theme.header_style())),
        Cell::from(Span::styled("Duration", theme.header_style())),
        Cell::from(Span::styled("Active", theme.header_style())),
        Cell::from(Span::styled("Failed", theme.header_style())),
        Cell::from(Span::styled(age_header, theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let derived = cached_job_derived(cluster, query, indices.as_ref());
    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &job_idx)| {
            let idx = window.start + local_idx;
            let job = &cluster.jobs[job_idx];
            let (duration, age) = if let Some(cell) = derived.get(idx) {
                (
                    Cow::Borrowed(cell.duration.as_str()),
                    Cow::Borrowed(cell.age.as_str()),
                )
            } else {
                (
                    Cow::Owned(job.duration.clone().unwrap_or_else(|| "-".to_string())),
                    Cow::Owned(format_age(job.age)),
                )
            };
            let st = status_style(&job.status, &theme);
            let failed_style = if job.failed_pods > 0 {
                theme.badge_error_style()
            } else {
                theme.inactive_style()
            };
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", job.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    job.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(job.status.clone(), st)),
                Cell::from(Span::styled(
                    job.completions.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(duration, Style::default().fg(theme.fg_dim))),
                Cell::from(Span::styled(
                    format_small_int(i64::from(job.active_pods)),
                    Style::default().fg(theme.info),
                )),
                Cell::from(Span::styled(
                    format_small_int(i64::from(job.failed_pods)),
                    failed_style,
                )),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let sort_suffix = workload_sort_suffix(sort);
    let title = format!(" ⚙  Jobs ({total}){sort_suffix} ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.jobs.len();
        active_block(&format!(
            " ⚙  Jobs ({total} of {all}) [/{query}]{sort_suffix}"
        ))
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Length(22),
                Constraint::Length(16),
                Constraint::Length(11),
                Constraint::Length(13),
                Constraint::Length(11),
                Constraint::Length(8),
                Constraint::Length(8),
                Constraint::Length(9),
            ],
        ),
    )
    .header(header)
    .block(block)
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

fn cached_job_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> JobDerivedCacheValue {
    let key = JobDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(&cluster.jobs, cluster.snapshot_version),
    };

    if let Ok(cache) = JOB_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&job_idx| {
                let job = &cluster.jobs[job_idx];
                JobDerivedCell {
                    duration: job.duration.clone().unwrap_or_else(|| "-".to_string()),
                    age: format_age(job.age),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = JOB_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

fn status_style(status: &str, theme: &crate::ui::theme::Theme) -> Style {
    match status {
        "Succeeded" | "Complete" => theme.badge_success_style(),
        "Running" => Style::default().fg(theme.info),
        "Failed" => theme.badge_error_style(),
        _ => theme.badge_warning_style(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Theme;

    #[test]
    fn status_color_map() {
        let theme = Theme::dark();
        assert_eq!(status_style("Succeeded", &theme).fg, Some(theme.success));
        assert_eq!(status_style("Running", &theme).fg, Some(theme.info));
        assert_eq!(status_style("Failed", &theme).fg, Some(theme.error));
        assert_eq!(status_style("Pending", &theme).fg, Some(theme.warning));
    }
}
