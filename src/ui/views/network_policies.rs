//! NetworkPolicies list view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Row, Table},
};

use crate::{state::ClusterSnapshot, ui::components::default_theme};

pub fn render_network_policies(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected: usize,
    search: &str,
) {
    let theme = default_theme();
    let items: Vec<_> = cluster
        .network_policies
        .iter()
        .filter(|np| search.is_empty() || np.name.contains(search) || np.namespace.contains(search))
        .collect();

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, np)| {
            let style = if i == selected { theme.selection_style() } else { Style::default() };
            Row::new(vec![
                Cell::from(np.name.clone()),
                Cell::from(np.namespace.clone()),
                Cell::from(np.pod_selector.clone()),
                Cell::from(np.ingress_rules.to_string()),
                Cell::from(np.egress_rules.to_string()),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec!["NAME", "NAMESPACE", "POD SELECTOR", "INGRESS", "EGRESS"])
        .style(theme.header_style())
        .height(1);

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(25),
            Constraint::Percentage(20),
            Constraint::Percentage(35),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(Line::from(if search.is_empty() {
                vec![
                    Span::styled(" NetworkPolicies ", theme.title_style()),
                    Span::styled(format!("({}) ", items.len()), theme.muted_style()),
                ]
            } else {
                vec![
                    Span::styled(" NetworkPolicies ", theme.title_style()),
                    Span::styled(format!("({} of {}) ", items.len(), cluster.network_policies.len()), theme.muted_style()),
                    Span::styled(format!("[/{search}]"), theme.muted_style()),
                ]
            }))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_active_style()),
    );

    frame.render_widget(table, area);
}
