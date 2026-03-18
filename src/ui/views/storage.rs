//! Storage views: PVCs, PVs, StorageClasses.

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
    icons::view_icon,
    state::ClusterSnapshot,
    ui::{
        TableFrame, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{
            DerivedRowsCache, DerivedRowsCacheKey, DerivedRowsCacheValue, cached_derived_rows,
            cached_filter_indices_with_variant, data_fingerprint,
        },
        render_centered_message, render_table_frame, resource_table_title, sort_header_cell,
        table_viewport_rows, table_window,
        views::filtering::{
            filtered_pv_indices, filtered_pvc_indices, filtered_storage_class_indices,
        },
        workload_sort_suffix,
    },
};

// ── PVC derived cell cache ──────────────────────────────────────────

#[derive(Debug, Clone)]
struct PvcDerivedCell {
    capacity: String,
    access_modes: String,
    storage_class: String,
}

type PvcDerivedCacheValue = DerivedRowsCacheValue<PvcDerivedCell>;
static PVC_DERIVED_CACHE: LazyLock<DerivedRowsCache<PvcDerivedCell>> =
    LazyLock::new(Default::default);

fn cached_pvc_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    variant: u64,
) -> PvcDerivedCacheValue {
    let key = DerivedRowsCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.pvcs, snapshot.snapshot_version),
        variant,
        freshness_bucket: 0,
    };

    cached_derived_rows(&PVC_DERIVED_CACHE, key, || {
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
            .collect()
    })
}

#[allow(clippy::too_many_arguments)]
pub fn render_pvcs(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    search: &str,
    sort: Option<WorkloadSortState>,
    focused: bool,
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
        |q| filtered_pvc_indices(&cluster.pvcs, q, sort),
    );

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::PersistentVolumeClaims,
            query,
            "PersistentVolumeClaims",
            "Loading persistent volume claims...",
            "No persistent volume claims found",
            "No persistent volume claims match the search query",
            focused,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let header = Row::new([
        sort_header_cell("NAME", sort, WorkloadSortColumn::Name, &theme, true),
        Cell::from(Span::styled("NAMESPACE", theme.header_style())),
        Cell::from(Span::styled("STATUS", theme.header_style())),
        Cell::from(Span::styled("CAPACITY", theme.header_style())),
        Cell::from(Span::styled("ACCESS MODES", theme.header_style())),
        Cell::from(Span::styled("STORAGECLASS", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let derived = cached_pvc_derived(cluster, query, &indices, cache_variant);

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
                bookmarked_name_cell(
                    &ResourceRef::Pvc(pvc.name.clone(), pvc.namespace.clone()),
                    bookmarks,
                    pvc.name.as_str(),
                    Style::default().fg(theme.fg),
                    &theme,
                ),
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

    let sort_suffix = workload_sort_suffix(sort);
    let title = resource_table_title(
        view_icon(AppView::PersistentVolumeClaims).active(),
        "PersistentVolumeClaims",
        total,
        cluster.pvcs.len(),
        query,
        &sort_suffix,
    );
    let widths = [
        Constraint::Percentage(25),
        Constraint::Percentage(15),
        Constraint::Percentage(10),
        Constraint::Percentage(12),
        Constraint::Percentage(18),
        Constraint::Percentage(20),
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

// ── PV derived cell cache ───────────────────────────────────────────

#[derive(Debug, Clone)]
struct PvDerivedCell {
    capacity: String,
    access_modes: String,
    claim: String,
    storage_class: String,
}

type PvDerivedCacheValue = DerivedRowsCacheValue<PvDerivedCell>;
static PV_DERIVED_CACHE: LazyLock<DerivedRowsCache<PvDerivedCell>> =
    LazyLock::new(Default::default);

fn cached_pv_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    variant: u64,
) -> PvDerivedCacheValue {
    let key = DerivedRowsCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.pvs, snapshot.snapshot_version),
        variant,
        freshness_bucket: 0,
    };

    cached_derived_rows(&PV_DERIVED_CACHE, key, || {
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
            .collect()
    })
}

#[allow(clippy::too_many_arguments)]
pub fn render_pvs(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    search: &str,
    sort: Option<WorkloadSortState>,
    focused: bool,
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
        |q| filtered_pv_indices(&cluster.pvs, q, sort),
    );

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::PersistentVolumes,
            query,
            "PersistentVolumes",
            "Loading persistent volumes...",
            "No persistent volumes found",
            "No persistent volumes match the search query",
            focused,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let header = Row::new([
        sort_header_cell("NAME", sort, WorkloadSortColumn::Name, &theme, true),
        Cell::from(Span::styled("CAPACITY", theme.header_style())),
        Cell::from(Span::styled("ACCESS MODES", theme.header_style())),
        Cell::from(Span::styled("RECLAIM", theme.header_style())),
        Cell::from(Span::styled("STATUS", theme.header_style())),
        Cell::from(Span::styled("CLAIM", theme.header_style())),
        Cell::from(Span::styled("STORAGECLASS", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let derived = cached_pv_derived(cluster, query, &indices, cache_variant);

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
                bookmarked_name_cell(
                    &ResourceRef::Pv(pv.name.clone()),
                    bookmarks,
                    pv.name.as_str(),
                    Style::default().fg(theme.fg),
                    &theme,
                ),
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

    let sort_suffix = workload_sort_suffix(sort);
    let title = resource_table_title(
        view_icon(AppView::PersistentVolumes).active(),
        "PersistentVolumes",
        total,
        cluster.pvs.len(),
        query,
        &sort_suffix,
    );
    let widths = [
        Constraint::Percentage(20),
        Constraint::Percentage(10),
        Constraint::Percentage(15),
        Constraint::Percentage(10),
        Constraint::Percentage(10),
        Constraint::Percentage(20),
        Constraint::Percentage(15),
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

// ── StorageClass derived cell cache ─────────────────────────────────

#[derive(Debug, Clone)]
struct StorageClassDerivedCell {
    default_label: &'static str,
    reclaim: String,
    binding: String,
    expand: &'static str,
}

type StorageClassDerivedCacheValue = DerivedRowsCacheValue<StorageClassDerivedCell>;
static STORAGE_CLASS_DERIVED_CACHE: LazyLock<DerivedRowsCache<StorageClassDerivedCell>> =
    LazyLock::new(Default::default);

fn cached_storage_class_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    variant: u64,
) -> StorageClassDerivedCacheValue {
    let key = DerivedRowsCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.storage_classes, snapshot.snapshot_version),
        variant,
        freshness_bucket: 0,
    };

    cached_derived_rows(&STORAGE_CLASS_DERIVED_CACHE, key, || {
        indices
            .iter()
            .map(|&sc_idx| {
                let sc = &snapshot.storage_classes[sc_idx];
                StorageClassDerivedCell {
                    default_label: if sc.is_default { "(default)" } else { "" },
                    reclaim: sc.reclaim_policy.as_deref().unwrap_or("Delete").to_string(),
                    binding: sc
                        .volume_binding_mode
                        .as_deref()
                        .unwrap_or("Immediate")
                        .to_string(),
                    expand: if sc.allow_volume_expansion { "✓" } else { "" },
                }
            })
            .collect()
    })
}

#[allow(clippy::too_many_arguments)]
pub fn render_storage_classes(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    search: &str,
    sort: Option<WorkloadSortState>,
    focused: bool,
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
        |q| filtered_storage_class_indices(&cluster.storage_classes, q, sort),
    );

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::StorageClasses,
            query,
            "StorageClasses",
            "Loading storage classes...",
            "No storage classes found",
            "No storage classes match the search query",
            focused,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let header = Row::new([
        sort_header_cell("NAME", sort, WorkloadSortColumn::Name, &theme, true),
        Cell::from(Span::styled("PROVISIONER", theme.header_style())),
        Cell::from(Span::styled("RECLAIM", theme.header_style())),
        Cell::from(Span::styled("BINDING MODE", theme.header_style())),
        Cell::from(Span::styled("EXPAND", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let derived = cached_storage_class_derived(cluster, query, &indices, cache_variant);

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
            let (default_label, reclaim, binding, expand) = if let Some(cell) = derived.get(idx) {
                (
                    cell.default_label,
                    Cow::Borrowed(cell.reclaim.as_str()),
                    Cow::Borrowed(cell.binding.as_str()),
                    cell.expand,
                )
            } else {
                (
                    if storage_class.is_default {
                        "(default)"
                    } else {
                        ""
                    },
                    Cow::Owned(
                        storage_class
                            .reclaim_policy
                            .as_deref()
                            .unwrap_or("Delete")
                            .to_string(),
                    ),
                    Cow::Owned(
                        storage_class
                            .volume_binding_mode
                            .as_deref()
                            .unwrap_or("Immediate")
                            .to_string(),
                    ),
                    if storage_class.allow_volume_expansion {
                        "✓"
                    } else {
                        ""
                    },
                )
            };
            let display_name = if default_label.is_empty() {
                storage_class.name.clone()
            } else {
                format!("{} {}", storage_class.name, default_label)
            };
            Row::new(vec![
                bookmarked_name_cell(
                    &ResourceRef::StorageClass(storage_class.name.clone()),
                    bookmarks,
                    display_name,
                    Style::default().fg(theme.fg),
                    &theme,
                ),
                Cell::from(Span::styled(
                    storage_class.provisioner.clone(),
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(Span::styled(reclaim, Style::default().fg(theme.fg_dim))),
                Cell::from(Span::styled(binding, Style::default().fg(theme.info))),
                Cell::from(Span::styled(expand, Style::default().fg(theme.success))),
            ])
            .style(row_style)
        })
        .collect();

    let sort_suffix = workload_sort_suffix(sort);
    let title = resource_table_title(
        view_icon(AppView::StorageClasses).active(),
        "StorageClasses",
        total,
        cluster.storage_classes.len(),
        query,
        &sort_suffix,
    );
    let widths = [
        Constraint::Percentage(25),
        Constraint::Percentage(35),
        Constraint::Percentage(15),
        Constraint::Percentage(18),
        Constraint::Percentage(7),
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
