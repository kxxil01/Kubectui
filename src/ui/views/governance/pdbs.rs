//! PodDisruptionBudgets list rendering.

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
        format_age, format_small_int, render_centered_message, render_table_frame,
        resource_table_title, sort_header_cell, table_viewport_rows, table_window,
        views::filtering::filtered_pdb_indices,
        workload_sort_suffix,
    },
};

// ── PDB derived cell cache ──────────────────────────────────────────

#[derive(Debug, Clone)]
struct PdbDerivedCell {
    policy: String,
    healthy: String,
    disruptions: String,
    age: String,
}

type PdbDerivedCacheValue = DerivedRowsCacheValue<PdbDerivedCell>;
static PDB_DERIVED_CACHE: LazyLock<DerivedRowsCache<PdbDerivedCell>> =
    LazyLock::new(Default::default);

fn cached_pdb_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    variant: u64,
) -> PdbDerivedCacheValue {
    let key = DerivedRowsCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(
            &snapshot.pod_disruption_budgets,
            snapshot.snapshot_version,
        ),
        variant,
        freshness_bucket: 0,
    };

    cached_derived_rows(&PDB_DERIVED_CACHE, key, || {
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
            .collect()
    })
}

#[allow(clippy::too_many_arguments)]
pub fn render_pdbs(
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
        AppView::PodDisruptionBudgets,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.pod_disruption_budgets, cluster.snapshot_version),
        cache_variant,
        |q| filtered_pdb_indices(&cluster.pod_disruption_budgets, q, sort),
    );

    let theme = default_theme();

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::PodDisruptionBudgets,
            query,
            "PodDisruptionBudgets",
            "Loading pod disruption budgets...",
            "No pod disruption budgets found",
            "No pod disruption budgets match the search query",
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
        Cell::from(Span::styled("Policy", theme.header_style())),
        Cell::from(Span::styled("Healthy", theme.header_style())),
        Cell::from(Span::styled("Disruptions", theme.header_style())),
        sort_header_cell("Age", sort, WorkloadSortColumn::Age, &theme, false),
    ])
    .height(1)
    .style(theme.header_style());

    let derived = cached_pdb_derived(cluster, query, &indices, cache_variant);

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
                bookmarked_name_cell(
                    &ResourceRef::PodDisruptionBudget(pdb.name.clone(), pdb.namespace.clone()),
                    bookmarks,
                    pdb.name.as_str(),
                    Style::default().fg(theme.fg),
                    &theme,
                ),
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

    let sort_suffix = workload_sort_suffix(sort);
    let title = resource_table_title(
        "🛡️ ",
        "PodDisruptionBudgets",
        total,
        cluster.pod_disruption_budgets.len(),
        query,
        &sort_suffix,
    );
    let widths = [
        Constraint::Min(28),
        Constraint::Length(18),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Length(12),
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
