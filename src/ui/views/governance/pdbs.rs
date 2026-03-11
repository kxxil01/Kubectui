//! PodDisruptionBudgets list rendering.

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

// ── PDB derived cell cache ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct PdbDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct PdbDerivedCell {
    policy: String,
    healthy: String,
    disruptions: String,
    age: String,
}

type PdbDerivedCacheValue = Arc<Vec<PdbDerivedCell>>;
static PDB_DERIVED_CACHE: LazyLock<Mutex<Option<(PdbDerivedCacheKey, PdbDerivedCacheValue)>>> =
    LazyLock::new(|| Mutex::new(None));

fn cached_pdb_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> PdbDerivedCacheValue {
    let key = PdbDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(
            &snapshot.pod_disruption_budgets,
            snapshot.snapshot_version,
        ),
    };

    if let Ok(cache) = PDB_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&pdb_idx| {
                let pdb = &snapshot.pod_disruption_budgets[pdb_idx];
                PdbDerivedCell {
                    policy: pdb
                        .min_available
                        .clone()
                        .or_else(|| pdb.max_unavailable.clone())
                        .unwrap_or_else(|| "-".to_string()),
                    healthy: format!("{}/{}", pdb.current_healthy, pdb.desired_healthy),
                    disruptions: format_small_int(i64::from(pdb.disruptions_allowed)).into_owned(),
                    age: format_age(pdb.age),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = PDB_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

pub fn render_pdbs(
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
        AppView::PodDisruptionBudgets,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.pod_disruption_budgets, cluster.snapshot_version),
        cache_variant,
        |q| {
            filtered_workload_indices(
                &cluster.pod_disruption_budgets,
                q,
                sort,
                |pdb, needle| {
                    contains_ci(&pdb.name, needle)
                        || contains_ci(pdb.min_available.as_deref().unwrap_or_default(), needle)
                        || contains_ci(pdb.max_unavailable.as_deref().unwrap_or_default(), needle)
                },
                |pdb| pdb.name.as_str(),
                |pdb| pdb.namespace.as_str(),
                |pdb| pdb.age,
            )
        },
    );

    let theme = default_theme();

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::PodDisruptionBudgets,
            query,
            "  Loading pod disruption budgets...",
            "  No pod disruption budgets found",
            "  No pod disruption budgets match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("PodDisruptionBudgets")),
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
        Cell::from(Span::styled("Policy", theme.header_style())),
        Cell::from(Span::styled("Healthy", theme.header_style())),
        Cell::from(Span::styled("Disruptions", theme.header_style())),
        Cell::from(Span::styled(age_header, theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let derived = cached_pdb_derived(cluster, query, &indices);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &pdb_idx)| {
            let idx = window.start + local_idx;
            let pdb = &cluster.pod_disruption_budgets[pdb_idx];
            let disrupt_style = disruption_style(pdb.disruptions_allowed, &theme);
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let (policy, healthy, disruptions, age) = if let Some(cell) = derived.get(idx) {
                (
                    Cow::Borrowed(cell.policy.as_str()),
                    Cow::Borrowed(cell.healthy.as_str()),
                    Cow::Borrowed(cell.disruptions.as_str()),
                    Cow::Borrowed(cell.age.as_str()),
                )
            } else {
                (
                    Cow::Owned(
                        pdb.min_available
                            .clone()
                            .or_else(|| pdb.max_unavailable.clone())
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    Cow::Owned(format!("{}/{}", pdb.current_healthy, pdb.desired_healthy)),
                    format_small_int(i64::from(pdb.disruptions_allowed)),
                    Cow::Owned(format_age(pdb.age)),
                )
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", pdb.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    pdb.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(policy, Style::default().fg(theme.fg_dim))),
                Cell::from(Span::styled(healthy, Style::default().fg(theme.fg_dim))),
                Cell::from(Span::styled(disruptions, disrupt_style)),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));
    let sort_suffix = workload_sort_suffix(sort);
    let title = format!(" 🛡️  PodDisruptionBudgets ({total}){sort_suffix} ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.pod_disruption_budgets.len();
        active_block(&format!(
            " 🛡️  PodDisruptionBudgets ({total} of {all}) [/{query}]{sort_suffix}"
        ))
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Min(28),
                Constraint::Length(18),
                Constraint::Length(12),
                Constraint::Length(10),
                Constraint::Length(12),
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

fn disruption_style(disruptions_allowed: i32, theme: &crate::ui::theme::Theme) -> Style {
    if disruptions_allowed > 0 {
        theme.badge_success_style()
    } else {
        theme.badge_warning_style()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Theme;

    #[test]
    fn disruption_style_expected_colors() {
        let theme = Theme::dark();
        assert_eq!(disruption_style(2, &theme).fg, Some(theme.success));
        assert_eq!(disruption_style(0, &theme).fg, Some(theme.warning));
    }
}
