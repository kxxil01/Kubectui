//! Storage views: PVCs, PVs, StorageClasses.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Row, Table},
};

use crate::{state::ClusterSnapshot, ui::components::default_theme};

pub fn render_pvcs(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected: usize,
    search: &str,
) {
    let theme = default_theme();
    let items: Vec<_> = cluster
        .pvcs
        .iter()
        .filter(|p| search.is_empty() || p.name.contains(search) || p.namespace.contains(search))
        .collect();

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, pvc)| {
            let style = if i == selected { theme.selection_style() } else { Style::default() };
            let status_style = match pvc.status.as_str() {
                "Bound" => theme.badge_success_style(),
                "Pending" => theme.badge_warning_style(),
                _ => theme.badge_error_style(),
            };
            let capacity = pvc.capacity.as_deref().unwrap_or("-");
            let sc = pvc.storage_class.as_deref().unwrap_or("-");
            let modes = pvc.access_modes.join(",");
            Row::new(vec![
                Cell::from(pvc.name.clone()),
                Cell::from(pvc.namespace.clone()),
                Cell::from(Span::styled(pvc.status.clone(), status_style)),
                Cell::from(capacity.to_string()),
                Cell::from(modes),
                Cell::from(sc.to_string()),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec!["NAME", "NAMESPACE", "STATUS", "CAPACITY", "ACCESS MODES", "STORAGECLASS"])
        .style(theme.header_style())
        .height(1);

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
    .block(
        Block::default()
            .title(Line::from(if search.is_empty() {
                vec![
                    Span::styled(" PersistentVolumeClaims ", theme.title_style()),
                    Span::styled(format!("({}) ", items.len()), theme.muted_style()),
                ]
            } else {
                vec![
                    Span::styled(" PersistentVolumeClaims ", theme.title_style()),
                    Span::styled(format!("({} of {}) ", items.len(), cluster.pvcs.len()), theme.muted_style()),
                    Span::styled(format!("[/{search}]"), theme.muted_style()),
                ]
            }))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_active_style()),
    );

    frame.render_widget(table, area);
}

pub fn render_pvs(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected: usize,
    search: &str,
) {
    let theme = default_theme();
    let items: Vec<_> = cluster
        .pvs
        .iter()
        .filter(|p| search.is_empty() || p.name.contains(search))
        .collect();

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, pv)| {
            let style = if i == selected { theme.selection_style() } else { Style::default() };
            let status_style = match pv.status.as_str() {
                "Bound" => theme.badge_success_style(),
                "Available" => theme.badge_warning_style(),
                _ => theme.badge_error_style(),
            };
            let capacity = pv.capacity.as_deref().unwrap_or("-");
            let sc = pv.storage_class.as_deref().unwrap_or("-");
            let claim = pv.claim.as_deref().unwrap_or("-");
            let modes = pv.access_modes.join(",");
            Row::new(vec![
                Cell::from(pv.name.clone()),
                Cell::from(capacity.to_string()),
                Cell::from(modes),
                Cell::from(pv.reclaim_policy.clone()),
                Cell::from(Span::styled(pv.status.clone(), status_style)),
                Cell::from(claim.to_string()),
                Cell::from(sc.to_string()),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec!["NAME", "CAPACITY", "ACCESS MODES", "RECLAIM", "STATUS", "CLAIM", "STORAGECLASS"])
        .style(theme.header_style())
        .height(1);

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
    .block(
        Block::default()
            .title(Line::from(if search.is_empty() {
                vec![
                    Span::styled(" PersistentVolumes ", theme.title_style()),
                    Span::styled(format!("({}) ", items.len()), theme.muted_style()),
                ]
            } else {
                vec![
                    Span::styled(" PersistentVolumes ", theme.title_style()),
                    Span::styled(format!("({} of {}) ", items.len(), cluster.pvs.len()), theme.muted_style()),
                    Span::styled(format!("[/{search}]"), theme.muted_style()),
                ]
            }))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_active_style()),
    );

    frame.render_widget(table, area);
}

pub fn render_storage_classes(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected: usize,
    search: &str,
) {
    let theme = default_theme();
    let items: Vec<_> = cluster
        .storage_classes
        .iter()
        .filter(|sc| search.is_empty() || sc.name.contains(search))
        .collect();

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, sc)| {
            let style = if i == selected { theme.selection_style() } else { Style::default() };
            let default_label = if sc.is_default { "(default)" } else { "" };
            let reclaim = sc.reclaim_policy.as_deref().unwrap_or("Delete");
            let binding = sc.volume_binding_mode.as_deref().unwrap_or("Immediate");
            let expand = if sc.allow_volume_expansion { "✓" } else { "" };
            Row::new(vec![
                Cell::from(format!("{} {}", sc.name, default_label)),
                Cell::from(sc.provisioner.clone()),
                Cell::from(reclaim.to_string()),
                Cell::from(binding.to_string()),
                Cell::from(expand),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec!["NAME", "PROVISIONER", "RECLAIM", "BINDING MODE", "EXPAND"])
        .style(theme.header_style())
        .height(1);

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
    .block(
        Block::default()
            .title(Line::from(if search.is_empty() {
                vec![
                    Span::styled(" StorageClasses ", theme.title_style()),
                    Span::styled(format!("({}) ", items.len()), theme.muted_style()),
                ]
            } else {
                vec![
                    Span::styled(" StorageClasses ", theme.title_style()),
                    Span::styled(format!("({} of {}) ", items.len(), cluster.storage_classes.len()), theme.muted_style()),
                    Span::styled(format!("[/{search}]"), theme.muted_style()),
                ]
            }))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_active_style()),
    );

    frame.render_widget(table, area);
}
