//! Endpoints list view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Row, Table},
};

use crate::{state::ClusterSnapshot, ui::components::default_theme};

pub fn render_endpoints(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected: usize,
    search: &str,
) {
    let theme = default_theme();
    let items: Vec<_> = cluster
        .endpoints
        .iter()
        .filter(|e| search.is_empty() || e.name.contains(search) || e.namespace.contains(search))
        .collect();

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, ep)| {
            let style = if i == selected {
                theme.selection_style()
            } else {
                Style::default()
            };
            let addrs = if ep.addresses.is_empty() {
                "<none>".to_string()
            } else {
                ep.addresses.join(",")
            };
            let ports = if ep.ports.is_empty() {
                "<none>".to_string()
            } else {
                ep.ports.join(",")
            };
            Row::new(vec![
                Cell::from(ep.name.clone()),
                Cell::from(ep.namespace.clone()),
                Cell::from(addrs),
                Cell::from(ports),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec!["NAME", "NAMESPACE", "ADDRESSES", "PORTS"])
        .style(theme.header_style())
        .height(1);

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(20),
            Constraint::Percentage(30),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(Line::from(if search.is_empty() {
                vec![
                    Span::styled(" Endpoints ", theme.title_style()),
                    Span::styled(format!("({}) ", items.len()), theme.muted_style()),
                ]
            } else {
                vec![
                    Span::styled(" Endpoints ", theme.title_style()),
                    Span::styled(
                        format!("({} of {}) ", items.len(), cluster.endpoints.len()),
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
