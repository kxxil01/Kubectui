//! Nodes list renderer.

use std::{borrow::Cow, collections::HashMap, sync::LazyLock};

use ratatui::{
    layout::Rect,
    prelude::{Frame, Line, Style},
    text::Span,
    widgets::{Cell, Row},
};

use crate::{
    app::{AppView, ResourceRef, WorkloadSortColumn, WorkloadSortState},
    bookmarks::BookmarkEntry,
    columns::ColumnDef,
    icons::view_icon,
    k8s::dtos::NodeMetricsInfo,
    state::{
        ClusterSnapshot,
        alerts::{format_mib, format_millicores, parse_mib, parse_millicores},
    },
    time::now_unix_seconds,
    ui::{
        TableFrame, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{
            DerivedRowsCache, DerivedRowsCacheKey, DerivedRowsCacheValue, cached_derived_rows,
            cached_filter_indices_with_variant, data_fingerprint,
        },
        render_centered_message, render_table_frame, resource_table_title, sort_header_cell,
        table_viewport_rows, table_window, utilization_bar_labeled,
        views::filtering::filtered_node_indices,
        workload_sort_suffix,
    },
};

#[derive(Debug, Clone)]
struct NodeDerivedCell {
    age: String,
}

type NodeDerivedCacheValue = DerivedRowsCacheValue<NodeDerivedCell>;
static NODE_DERIVED_CACHE: LazyLock<DerivedRowsCache<NodeDerivedCell>> =
    LazyLock::new(Default::default);

/// Renders the nodes table with stateful selection, scrollbar, and theme-aware styling.
#[allow(clippy::too_many_arguments)]
pub fn render_nodes(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    query: &str,
    sort: Option<WorkloadSortState>,
    visible_columns: &[ColumnDef],
    focused: bool,
) {
    let theme = default_theme();
    let query = query.trim();

    if snapshot.nodes.is_empty() {
        render_centered_message(
            frame,
            area,
            snapshot,
            AppView::Nodes,
            "",
            "Nodes",
            "Loading nodes...",
            "No nodes available",
            "No nodes available",
            focused,
        );
        return;
    }

    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::Nodes,
        query,
        snapshot.snapshot_version,
        data_fingerprint(&snapshot.nodes, snapshot.snapshot_version),
        cache_variant,
        |q| filtered_node_indices(&snapshot.nodes, q, sort),
    );

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            snapshot,
            AppView::Nodes,
            query,
            "Nodes",
            "Loading nodes...",
            "No nodes available",
            "No nodes match the search query",
            focused,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let header_cells: Vec<Cell> = visible_columns
        .iter()
        .map(|col| match col.id {
            "name" => sort_header_cell(col.label, sort, WorkloadSortColumn::Name, &theme, true),
            "age" => sort_header_cell(col.label, sort, WorkloadSortColumn::Age, &theme, false),
            _ => Cell::from(Span::styled(col.label.to_string(), theme.header_style())),
        })
        .collect();
    let header = Row::new(header_cells).height(1).style(theme.header_style());

    let name_style = Style::default().fg(theme.fg);
    let accent_style = Style::default().fg(theme.accent2);
    let dim_style = Style::default().fg(theme.fg_dim);
    let warn_style = theme.badge_warning_style();
    let now_unix = now_unix_seconds();
    let derived = cached_node_derived(snapshot, query, indices.as_ref(), now_unix, cache_variant);

    // Build node metrics lookup only when metric columns are visible
    let needs_metrics = visible_columns
        .iter()
        .any(|c| matches!(c.id, "cpu" | "memory"));
    let metrics_by_node: HashMap<&str, &NodeMetricsInfo> = if needs_metrics {
        snapshot
            .node_metrics
            .iter()
            .map(|nm| (nm.name.as_str(), nm))
            .collect()
    } else {
        HashMap::new()
    };

    let mut rows: Vec<Row> = Vec::with_capacity(window.end.saturating_sub(window.start));
    for (local_idx, &node_idx) in indices[window.start..window.end].iter().enumerate() {
        let idx = window.start + local_idx;
        let node = &snapshot.nodes[node_idx];
        let age = derived
            .get(idx)
            .map(|cell| Cow::Borrowed(cell.age.as_str()))
            .unwrap_or_else(|| {
                Cow::Owned(crate::ui::format_age_from_timestamp(
                    node.created_at,
                    now_unix,
                ))
            });
        let (status_text, status_style) = match (node.ready, node.unschedulable) {
            (true, false) => ("● Ready", theme.badge_success_style()),
            (true, true) => ("● Ready SchedulingDisabled", theme.badge_warning_style()),
            (false, false) => ("✗ NotReady", theme.badge_error_style()),
            (false, true) => ("✗ NotReady SchedulingDisabled", theme.badge_error_style()),
        };

        let mut status_spans = Vec::with_capacity(3);
        status_spans.push(Span::styled(status_text, status_style));
        if node.memory_pressure {
            status_spans.push(Span::styled("  ⚠ Mem", warn_style));
        }
        if node.disk_pressure {
            status_spans.push(Span::styled("  ⚠ Disk", warn_style));
        }
        let mut status_spans = Some(status_spans);

        let row_style = if idx.is_multiple_of(2) {
            Style::default().bg(theme.bg)
        } else {
            theme.row_alt_style()
        };

        let cells: Vec<Cell> = visible_columns
            .iter()
            .map(|col| match col.id {
                "name" => bookmarked_name_cell(
                    || ResourceRef::Node(node.name.clone()),
                    bookmarks,
                    node.name.as_str(),
                    name_style,
                    &theme,
                ),
                "status" => Cell::from(Line::from(status_spans.take().unwrap_or_default())),
                "roles" => Cell::from(Span::styled(node.role.as_str(), accent_style)),
                "cpu" => {
                    let alloc = node.cpu_allocatable.as_deref().unwrap_or("N/A");
                    match metrics_by_node.get(node.name.as_str()) {
                        Some(nm) => {
                            let used = parse_millicores(&nm.cpu);
                            let alloc_m = parse_millicores(alloc);
                            let pct = if alloc_m > 0 { used * 100 / alloc_m } else { 0 };
                            let label = format!(
                                "{}/{}",
                                format_millicores(used),
                                format_millicores(alloc_m)
                            );
                            Cell::from(utilization_bar_labeled(&label, pct, &theme))
                        }
                        None => Cell::from(Span::styled(alloc, dim_style)),
                    }
                }
                "memory" => {
                    let alloc = node.memory_allocatable.as_deref().unwrap_or("N/A");
                    match metrics_by_node.get(node.name.as_str()) {
                        Some(nm) => {
                            let used = parse_mib(&nm.memory);
                            let alloc_mib = parse_mib(alloc);
                            let pct = if alloc_mib > 0 {
                                used * 100 / alloc_mib
                            } else {
                                0
                            };
                            let label = format!("{}/{}", format_mib(used), format_mib(alloc_mib));
                            Cell::from(utilization_bar_labeled(&label, pct, &theme))
                        }
                        None => Cell::from(Span::styled(alloc, dim_style)),
                    }
                }
                "age" => Cell::from(Span::styled(age.to_string(), theme.inactive_style())),
                _ => Cell::from(""),
            })
            .collect();
        rows.push(Row::new(cells).style(row_style));
    }

    let sort_suffix = workload_sort_suffix(sort);
    let title = resource_table_title(
        view_icon(AppView::Nodes).active(),
        "Nodes",
        total,
        snapshot.nodes.len(),
        query,
        &sort_suffix,
    );
    let widths = crate::columns::visible_constraints(visible_columns);

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

fn cached_node_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    now_unix: i64,
    variant: u64,
) -> NodeDerivedCacheValue {
    let key = DerivedRowsCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.nodes, snapshot.snapshot_version),
        variant,
        freshness_bucket: now_unix / 60,
    };

    cached_derived_rows(&NODE_DERIVED_CACHE, key, || {
        indices
            .iter()
            .map(|&node_idx| {
                let node = &snapshot.nodes[node_idx];
                NodeDerivedCell {
                    age: crate::ui::format_age_from_timestamp(node.created_at, now_unix),
                }
            })
            .collect()
    })
}
