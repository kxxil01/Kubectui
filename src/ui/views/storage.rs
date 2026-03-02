//! Storage views: PVCs, PVs, StorageClasses.

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
    app::AppView,
    state::ClusterSnapshot,
    ui::{
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices, data_fingerprint},
        loading_or_empty_message, table_viewport_rows, table_window,
    },
};

pub fn render_pvcs(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search: &str,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = cached_filter_indices(
        AppView::PersistentVolumeClaims,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.pvcs),
        |q| {
            if q.is_empty() {
                return (0..cluster.pvcs.len()).collect();
            }
            cluster
                .pvcs
                .iter()
                .enumerate()
                .filter_map(|(idx, pvc)| {
                    (contains_ci(&pvc.name, q) || contains_ci(&pvc.namespace, q)).then_some(idx)
                })
                .collect()
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
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

    let header = Row::new([
        Cell::from(Span::styled("  NAME", theme.header_style())),
        Cell::from(Span::styled("NAMESPACE", theme.header_style())),
        Cell::from(Span::styled("STATUS", theme.header_style())),
        Cell::from(Span::styled("CAPACITY", theme.header_style())),
        Cell::from(Span::styled("ACCESS MODES", theme.header_style())),
        Cell::from(Span::styled("STORAGECLASS", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

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
            let capacity = pvc.capacity.as_deref().unwrap_or("-");
            let sc = pvc.storage_class.as_deref().unwrap_or("-");
            let modes = if pvc.access_modes.is_empty() {
                "-".to_string()
            } else {
                pvc.access_modes.join(",")
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
                Cell::from(Span::styled(
                    capacity.to_string(),
                    Style::default().fg(theme.info),
                )),
                Cell::from(Span::styled(modes, Style::default().fg(theme.accent2))),
                Cell::from(Span::styled(
                    sc.to_string(),
                    Style::default().fg(theme.fg_dim),
                )),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let title = if query.is_empty() {
        format!(" PersistentVolumeClaims ({total}) ")
    } else {
        let all = cluster.pvcs.len();
        format!(" PersistentVolumeClaims ({total} of {all}) [/{query}]")
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

pub fn render_pvs(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search: &str,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = cached_filter_indices(
        AppView::PersistentVolumes,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.pvs),
        |q| {
            if q.is_empty() {
                return (0..cluster.pvs.len()).collect();
            }
            cluster
                .pvs
                .iter()
                .enumerate()
                .filter_map(|(idx, pv)| contains_ci(&pv.name, q).then_some(idx))
                .collect()
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
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

    let header = Row::new([
        Cell::from(Span::styled("  NAME", theme.header_style())),
        Cell::from(Span::styled("CAPACITY", theme.header_style())),
        Cell::from(Span::styled("ACCESS MODES", theme.header_style())),
        Cell::from(Span::styled("RECLAIM", theme.header_style())),
        Cell::from(Span::styled("STATUS", theme.header_style())),
        Cell::from(Span::styled("CLAIM", theme.header_style())),
        Cell::from(Span::styled("STORAGECLASS", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

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
            let capacity = pv.capacity.as_deref().unwrap_or("-");
            let sc = pv.storage_class.as_deref().unwrap_or("-");
            let claim = pv.claim.as_deref().unwrap_or("-");
            let modes = if pv.access_modes.is_empty() {
                "-".to_string()
            } else {
                pv.access_modes.join(",")
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", pv.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    capacity.to_string(),
                    Style::default().fg(theme.info),
                )),
                Cell::from(Span::styled(modes, Style::default().fg(theme.accent2))),
                Cell::from(Span::styled(
                    pv.reclaim_policy.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(pv.status.clone(), status_style)),
                Cell::from(Span::styled(
                    claim.to_string(),
                    Style::default().fg(theme.warning),
                )),
                Cell::from(Span::styled(
                    sc.to_string(),
                    Style::default().fg(theme.fg_dim),
                )),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let title = if query.is_empty() {
        format!(" PersistentVolumes ({total}) ")
    } else {
        let all = cluster.pvs.len();
        format!(" PersistentVolumes ({total} of {all}) [/{query}]")
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
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = cached_filter_indices(
        AppView::StorageClasses,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.storage_classes),
        |q| {
            if q.is_empty() {
                return (0..cluster.storage_classes.len()).collect();
            }
            cluster
                .storage_classes
                .iter()
                .enumerate()
                .filter_map(|(idx, storage_class)| {
                    contains_ci(&storage_class.name, q).then_some(idx)
                })
                .collect()
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
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

    let header = Row::new([
        Cell::from(Span::styled("  NAME", theme.header_style())),
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

    let title = if query.is_empty() {
        format!(" StorageClasses ({total}) ")
    } else {
        let all = cluster.storage_classes.len();
        format!(" StorageClasses ({total} of {all}) [/{query}]")
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
