//! CronJobs list rendering.

use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

use chrono::{DateTime, Local, Utc};
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
    app::{AppView, WorkloadSortColumn, WorkloadSortState},
    state::ClusterSnapshot,
    ui::{
        components::{active_block, default_block, default_theme},
        filter_cache::{cached_filter_indices_with_variant, data_fingerprint},
        format_age, format_small_int, loading_or_empty_message, responsive_table_widths,
        sort_header_cell, table_viewport_rows, table_window,
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

pub fn render_cronjobs(
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
        AppView::CronJobs,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.cronjobs, cluster.snapshot_version),
        cache_variant,
        |q| filtered_cronjob_indices(&cluster.cronjobs, q, sort),
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::CronJobs,
            query,
            "  Loading cronjobs...",
            "  No cronjobs found",
            "  No cronjobs match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("CronJobs")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));
    let header = Row::new([
        sort_header_cell("Name", sort, WorkloadSortColumn::Name, &theme, true),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Schedule", theme.header_style())),
        Cell::from(Span::styled("Last Run", theme.header_style())),
        Cell::from(Span::styled("Next Run", theme.header_style())),
        Cell::from(Span::styled("Active", theme.header_style())),
        Cell::from(Span::styled("Suspend", theme.header_style())),
        sort_header_cell("Age", sort, WorkloadSortColumn::Age, &theme, false),
    ])
    .height(1)
    .style(theme.header_style());
    let name_style = Style::default().fg(theme.fg);
    let dim_style = Style::default().fg(theme.fg_dim);
    let accent_style = Style::default().fg(theme.accent2);
    let info_style = Style::default().fg(theme.info);
    let derived = cached_cronjob_derived(cluster, query, indices.as_ref(), cache_variant);

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
        let row_style = if idx.is_multiple_of(2) {
            Style::default().bg(theme.bg)
        } else {
            theme.row_alt_style()
        };

        rows.push(
            Row::new(vec![
                Cell::from(Span::styled(format!("  {}", cj.name), name_style)),
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
            .style(row_style),
        );
    }

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let sort_suffix = workload_sort_suffix(sort);
    let title = format!(" 🕐 CronJobs ({total}){sort_suffix} ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.cronjobs.len();
        active_block(&format!(
            " 🕐 CronJobs ({total} of {all}) [/{query}]{sort_suffix}"
        ))
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Length(20),
                Constraint::Length(16),
                Constraint::Length(16),
                Constraint::Length(14),
                Constraint::Length(14),
                Constraint::Length(8),
                Constraint::Length(10),
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

fn format_time(ts: Option<DateTime<Utc>>) -> String {
    if let Some(value) = ts {
        value
            .with_timezone(&Local)
            .format("%m-%d %H:%M")
            .to_string()
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
