//! LimitRanges list rendering.

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

// ── LimitRange derived cell cache ───────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct LimitRangeDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
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
) -> LimitRangeDerivedCacheValue {
    let key = LimitRangeDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.limit_ranges, snapshot.snapshot_version),
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

pub fn render_limit_ranges(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
    sort: Option<WorkloadSortState>,
) {
    let query = query.trim();
    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::LimitRanges,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.limit_ranges, cluster.snapshot_version),
        cache_variant,
        |q| {
            filtered_workload_indices(
                &cluster.limit_ranges,
                q,
                sort,
                |lr, needle| {
                    let type_match = lr
                        .limits
                        .iter()
                        .any(|spec| contains_ci(&spec.type_, needle));
                    contains_ci(&lr.name, needle) || type_match
                },
                |lr| lr.name.as_str(),
                |lr| lr.namespace.as_str(),
                |lr| lr.age,
            )
        },
    );

    let theme = default_theme();

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::LimitRanges,
            query,
            "  Loading limit ranges...",
            "  No limit ranges found",
            "  No limit ranges match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("LimitRanges")),
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
        Cell::from(Span::styled("Specs", theme.header_style())),
        Cell::from(Span::styled("Types", theme.header_style())),
        Cell::from(Span::styled(age_header, theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let derived = cached_limit_range_derived(cluster, query, &indices);

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
                Cell::from(Span::styled(
                    format!("  {}", lr.name),
                    Style::default().fg(theme.fg),
                )),
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

    let mut table_state = TableState::default().with_selected(Some(window.selected));
    let sort_suffix = workload_sort_suffix(sort);
    let title = format!(" ⚖️  LimitRanges ({total}){sort_suffix} ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.limit_ranges.len();
        active_block(&format!(
            " ⚖️  LimitRanges ({total} of {all}) [/{query}]{sort_suffix}"
        ))
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Min(28),
                Constraint::Length(18),
                Constraint::Length(8),
                Constraint::Min(24),
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
