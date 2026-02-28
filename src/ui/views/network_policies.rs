//! NetworkPolicies list view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table},
};

use crate::{
    state::ClusterSnapshot,
    ui::{
        components::{default_block, default_theme},
        format_small_int, loading_or_empty_message,
    },
};

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
    if items.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            search,
            "  Loading network policies...",
            "  No network policies found",
            "  No network policies match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("NetworkPolicies")),
            area,
        );
        return;
    }

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, np)| {
            let style = if i == selected {
                theme.selection_style()
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(np.name.clone()),
                Cell::from(np.namespace.clone()),
                Cell::from(np.pod_selector.clone()),
                Cell::from(format_small_int(np.ingress_rules as i64)),
                Cell::from(format_small_int(np.egress_rules as i64)),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec![
        "NAME",
        "NAMESPACE",
        "POD SELECTOR",
        "INGRESS",
        "EGRESS",
    ])
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
                    Span::styled(
                        format!("({} of {}) ", items.len(), cluster.network_policies.len()),
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
