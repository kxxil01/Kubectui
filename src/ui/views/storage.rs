//! Storage views: PVCs, PVs, StorageClasses.

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
        loading_or_empty_message, table_viewport_rows, table_window, workload_sort_header,
        workload_sort_suffix,
    },
};

// ── PVC derived cell cache ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct PvcDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct PvcDerivedCell {
    capacity: String,
    access_modes: String,
    storage_class: String,
}

type PvcDerivedCacheValue = Arc<Vec<PvcDerivedCell>>;
static PVC_DERIVED_CACHE: LazyLock<Mutex<Option<(PvcDerivedCacheKey, PvcDerivedCacheValue)>>> =
    LazyLock::new(|| Mutex::new(None));

fn cached_pvc_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> PvcDerivedCacheValue {
    let key = PvcDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.pvcs, snapshot.snapshot_version),
    };

    if let Ok(cache) = PVC_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&pvc_idx| {
                let pvc = &snapshot.pvcs[pvc_idx];
                PvcDerivedCell {
                    capacity: pvc.capacity.as_deref().unwrap_or("-").to_string(),
                    access_modes: if pvc.access_modes.is_empty() {
                        "-".to_string()
                    } else {
                        pvc.access_modes.join(",")
                    },
                    storage_class: pvc.storage_class.as_deref().unwrap_or("-").to_string(),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = PVC_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

pub fn render_pvcs(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search: &str,
    sort: Option<WorkloadSortState>,
) {
    let theme = default_theme();
    let query = search.trim();
    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::PersistentVolumeClaims,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.pvcs, cluster.snapshot_version),
        cache_variant,
        |q| {
            filtered_workload_indices(
                &cluster.pvcs,
                q,
                sort,
                |pvc, needle| contains_ci(&pvc.name, needle) || contains_ci(&pvc.namespace, needle),
                |pvc| pvc.name.as_str(),
                |pvc| pvc.namespace.as_str(),
                |_pvc| None,
            )
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::PersistentVolumeClaims,
            query,
            "  Loading persistent volume claims...",
            "  No persistent volume claims found",
            "  No persistent volume claims match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("PersistentVolumeClaims")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let name_header = workload_sort_header("NAME", sort, WorkloadSortColumn::Name);
    let header = Row::new([
        Cell::from(Span::styled(
            format!("  {name_header}"),
            theme.header_style(),
        )),
        Cell::from(Span::styled("NAMESPACE", theme.header_style())),
        Cell::from(Span::styled("STATUS", theme.header_style())),
        Cell::from(Span::styled("CAPACITY", theme.header_style())),
        Cell::from(Span::styled("ACCESS MODES", theme.header_style())),
        Cell::from(Span::styled("STORAGECLASS", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let derived = cached_pvc_derived(cluster, query, &indices);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &pvc_idx)| {
            let idx = window.start + local_idx;
            let pvc = &cluster.pvcs[pvc_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let status_style = match pvc.status.as_str() {
                "Bound" => theme.badge_success_style(),
                "Pending" => theme.badge_warning_style(),
                _ => theme.badge_error_style(),
            };
            let (capacity, modes, sc) = if let Some(cell) = derived.get(idx) {
                (
                    Cow::Borrowed(cell.capacity.as_str()),
                    Cow::Borrowed(cell.access_modes.as_str()),
                    Cow::Borrowed(cell.storage_class.as_str()),
                )
            } else {
                (
                    Cow::Owned(pvc.capacity.as_deref().unwrap_or("-").to_string()),
                    Cow::Owned(if pvc.access_modes.is_empty() {
                        "-".to_string()
                    } else {
                        pvc.access_modes.join(",")
                    }),
                    Cow::Owned(pvc.storage_class.as_deref().unwrap_or("-").to_string()),
                )
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", pvc.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    pvc.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(pvc.status.clone(), status_style)),
                Cell::from(Span::styled(capacity, Style::default().fg(theme.info))),
                Cell::from(Span::styled(modes, Style::default().fg(theme.accent2))),
                Cell::from(Span::styled(sc, Style::default().fg(theme.fg_dim))),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let sort_suffix = workload_sort_suffix(sort);
    let title = if query.is_empty() {
        format!(" PersistentVolumeClaims ({total}){sort_suffix} ")
    } else {
        let all = cluster.pvcs.len();
        format!(" PersistentVolumeClaims ({total} of {all}) [/{query}]{sort_suffix}")
    };

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(25),
            Constraint::Percentage(15),
            Constraint::Percentage(10),
            Constraint::Percentage(12),
            Constraint::Percentage(18),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(active_block(&title))
    .row_highlight_style(theme.selection_style())
    .highlight_symbol(theme.highlight_symbol())
    .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(table, area, &mut table_state);
    render_table_scrollbar(frame, area, total, selected);
}

// ── PV derived cell cache ───────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct PvDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct PvDerivedCell {
    capacity: String,
    access_modes: String,
    claim: String,
    storage_class: String,
}

type PvDerivedCacheValue = Arc<Vec<PvDerivedCell>>;
static PV_DERIVED_CACHE: LazyLock<Mutex<Option<(PvDerivedCacheKey, PvDerivedCacheValue)>>> =
    LazyLock::new(|| Mutex::new(None));

fn cached_pv_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> PvDerivedCacheValue {
    let key = PvDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.pvs, snapshot.snapshot_version),
    };

    if let Ok(cache) = PV_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&pv_idx| {
                let pv = &snapshot.pvs[pv_idx];
                PvDerivedCell {
                    capacity: pv.capacity.as_deref().unwrap_or("-").to_string(),
                    access_modes: if pv.access_modes.is_empty() {
                        "-".to_string()
                    } else {
                        pv.access_modes.join(",")
                    },
                    claim: pv.claim.as_deref().unwrap_or("-").to_string(),
                    storage_class: pv.storage_class.as_deref().unwrap_or("-").to_string(),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = PV_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

pub fn render_pvs(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search: &str,
    sort: Option<WorkloadSortState>,
) {
    let theme = default_theme();
    let query = search.trim();
    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::PersistentVolumes,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.pvs, cluster.snapshot_version),
        cache_variant,
        |q| {
            filtered_workload_indices(
                &cluster.pvs,
                q,
                sort,
                |pv, needle| contains_ci(&pv.name, needle),
                |pv| pv.name.as_str(),
                |_pv| "",
                |_pv| None,
            )
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::PersistentVolumes,
            query,
            "  Loading persistent volumes...",
            "  No persistent volumes found",
            "  No persistent volumes match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("PersistentVolumes")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let name_header = workload_sort_header("NAME", sort, WorkloadSortColumn::Name);
    let header = Row::new([
        Cell::from(Span::styled(
            format!("  {name_header}"),
            theme.header_style(),
        )),
        Cell::from(Span::styled("CAPACITY", theme.header_style())),
        Cell::from(Span::styled("ACCESS MODES", theme.header_style())),
        Cell::from(Span::styled("RECLAIM", theme.header_style())),
        Cell::from(Span::styled("STATUS", theme.header_style())),
        Cell::from(Span::styled("CLAIM", theme.header_style())),
        Cell::from(Span::styled("STORAGECLASS", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let derived = cached_pv_derived(cluster, query, &indices);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &pv_idx)| {
            let idx = window.start + local_idx;
            let pv = &cluster.pvs[pv_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let status_style = match pv.status.as_str() {
                "Bound" => theme.badge_success_style(),
                "Available" => theme.badge_warning_style(),
                _ => theme.badge_error_style(),
            };
            let (capacity, modes, claim, sc) = if let Some(cell) = derived.get(idx) {
                (
                    Cow::Borrowed(cell.capacity.as_str()),
                    Cow::Borrowed(cell.access_modes.as_str()),
                    Cow::Borrowed(cell.claim.as_str()),
                    Cow::Borrowed(cell.storage_class.as_str()),
                )
            } else {
                (
                    Cow::Owned(pv.capacity.as_deref().unwrap_or("-").to_string()),
                    Cow::Owned(if pv.access_modes.is_empty() {
                        "-".to_string()
                    } else {
                        pv.access_modes.join(",")
                    }),
                    Cow::Owned(pv.claim.as_deref().unwrap_or("-").to_string()),
                    Cow::Owned(pv.storage_class.as_deref().unwrap_or("-").to_string()),
                )
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", pv.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(capacity, Style::default().fg(theme.info))),
                Cell::from(Span::styled(modes, Style::default().fg(theme.accent2))),
                Cell::from(Span::styled(
                    pv.reclaim_policy.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(pv.status.clone(), status_style)),
                Cell::from(Span::styled(claim, Style::default().fg(theme.warning))),
                Cell::from(Span::styled(sc, Style::default().fg(theme.fg_dim))),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let sort_suffix = workload_sort_suffix(sort);
    let title = if query.is_empty() {
        format!(" PersistentVolumes ({total}){sort_suffix} ")
    } else {
        let all = cluster.pvs.len();
        format!(" PersistentVolumes ({total} of {all}) [/{query}]{sort_suffix}")
    };

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(20),
            Constraint::Percentage(10),
            Constraint::Percentage(15),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
            Constraint::Percentage(20),
            Constraint::Percentage(15),
        ],
    )
    .header(header)
    .block(active_block(&title))
    .row_highlight_style(theme.selection_style())
    .highlight_symbol(theme.highlight_symbol())
    .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(table, area, &mut table_state);
    render_table_scrollbar(frame, area, total, selected);
}

pub fn render_storage_classes(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search: &str,
    sort: Option<WorkloadSortState>,
) {
    let theme = default_theme();
    let query = search.trim();
    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::StorageClasses,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.storage_classes, cluster.snapshot_version),
        cache_variant,
        |q| {
            filtered_workload_indices(
                &cluster.storage_classes,
                q,
                sort,
                |storage_class, needle| contains_ci(&storage_class.name, needle),
                |storage_class| storage_class.name.as_str(),
                |_storage_class| "",
                |_storage_class| None,
            )
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::StorageClasses,
            query,
            "  Loading storage classes...",
            "  No storage classes found",
            "  No storage classes match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("StorageClasses")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let name_header = workload_sort_header("NAME", sort, WorkloadSortColumn::Name);
    let header = Row::new([
        Cell::from(Span::styled(
            format!("  {name_header}"),
            theme.header_style(),
        )),
        Cell::from(Span::styled("PROVISIONER", theme.header_style())),
        Cell::from(Span::styled("RECLAIM", theme.header_style())),
        Cell::from(Span::styled("BINDING MODE", theme.header_style())),
        Cell::from(Span::styled("EXPAND", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &storage_class_idx)| {
            let idx = window.start + local_idx;
            let storage_class = &cluster.storage_classes[storage_class_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let default_label = if storage_class.is_default {
                "(default)"
            } else {
                ""
            };
            let reclaim = storage_class.reclaim_policy.as_deref().unwrap_or("Delete");
            let binding = storage_class
                .volume_binding_mode
                .as_deref()
                .unwrap_or("Immediate");
            let expand = if storage_class.allow_volume_expansion {
                "✓"
            } else {
                ""
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {} {}", storage_class.name, default_label),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    storage_class.provisioner.clone(),
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(Span::styled(
                    reclaim.to_string(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    binding.to_string(),
                    Style::default().fg(theme.info),
                )),
                Cell::from(Span::styled(expand, Style::default().fg(theme.success))),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let sort_suffix = workload_sort_suffix(sort);
    let title = if query.is_empty() {
        format!(" StorageClasses ({total}){sort_suffix} ")
    } else {
        let all = cluster.storage_classes.len();
        format!(" StorageClasses ({total} of {all}) [/{query}]{sort_suffix}")
    };

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(25),
            Constraint::Percentage(35),
            Constraint::Percentage(15),
            Constraint::Percentage(18),
            Constraint::Percentage(7),
        ],
    )
    .header(header)
    .block(active_block(&title))
    .row_highlight_style(theme.selection_style())
    .highlight_symbol(theme.highlight_symbol())
    .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(table, area, &mut table_state);
    render_table_scrollbar(frame, area, total, selected);
}

fn render_table_scrollbar(frame: &mut Frame, area: Rect, total: usize, selected: usize) {
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
