//! Port Forwarding list view — shows active tunnels.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table},
};

use crate::{
    k8s::portforward::TunnelState,
    state::port_forward::TunnelRegistry,
    ui::{components::default_theme, contains_ci},
};

pub fn render_port_forwarding(
    frame: &mut Frame,
    area: Rect,
    registry: &TunnelRegistry,
    selected: usize,
    search: &str,
) {
    let theme = default_theme();
    let tunnels = registry.ordered_tunnels();

    let items: Vec<_> = tunnels
        .iter()
        .filter(|t| {
            search.is_empty()
                || contains_ci(&t.target.pod_name, search)
                || contains_ci(&t.target.namespace, search)
        })
        .collect();

    let clamped_selected = if items.is_empty() {
        0
    } else {
        selected.min(items.len() - 1)
    };

    if items.is_empty() {
        let msg = if search.is_empty() {
            "No active port forwards. Open a Pod detail and press [f] to create one."
        } else {
            "No matching tunnels."
        };
        let block = Block::default()
            .title(Line::from(vec![
                Span::styled(" Port Forwarding ", theme.title_style()),
                Span::styled("(0) ", theme.muted_style()),
            ]))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_active_style());
        let p = Paragraph::new(Span::styled(msg, theme.muted_style())).block(block);
        frame.render_widget(p, area);
        return;
    }

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let style = if i == clamped_selected {
                theme.selection_style()
            } else {
                Style::default()
            };
            let state_str = match t.state {
                TunnelState::Starting => "Starting",
                TunnelState::Active => "Active",
                TunnelState::Error => "Error",
                TunnelState::Closing => "Closing",
                TunnelState::Closed => "Closed",
            };
            Row::new(vec![
                Cell::from(t.target.pod_name.clone()),
                Cell::from(t.target.namespace.clone()),
                Cell::from(t.local_addr.to_string()),
                Cell::from(t.target.remote_port.to_string()),
                Cell::from(state_str),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec!["POD", "NAMESPACE", "LOCAL", "REMOTE", "STATUS"])
        .style(theme.header_style())
        .height(1);

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(Line::from(if search.is_empty() {
                vec![
                    Span::styled(" Port Forwarding ", theme.title_style()),
                    Span::styled(format!("({}) ", items.len()), theme.muted_style()),
                ]
            } else {
                vec![
                    Span::styled(" Port Forwarding ", theme.title_style()),
                    Span::styled(
                        format!("({} of {}) ", items.len(), tunnels.len()),
                        theme.muted_style(),
                    ),
                    Span::styled(format!("[/{search}]"), theme.muted_style()),
                ]
            }))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_active_style()),
    );

    frame.render_widget(table, area);
}
