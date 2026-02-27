//! Namespaces list view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Row, Table},
};

use crate::{state::ClusterSnapshot, ui::components::default_theme};

pub fn render_namespaces(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected: usize,
    search: &str,
) {
    let theme = default_theme();
    let items: Vec<_> = cluster
        .namespace_list
        .iter()
        .filter(|ns| search.is_empty() || ns.name.contains(search))
        .collect();

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, ns)| {
            let style = if i == selected {
                theme.selection_style()
            } else {
                Style::default()
            };
            let status_style = if ns.status == "Active" {
                theme.badge_success_style()
            } else {
                theme.badge_error_style()
            };
            Row::new(vec![
                Cell::from(ns.name.clone()),
                Cell::from(Span::styled(ns.status.clone(), status_style)),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec!["NAME", "STATUS"])
        .style(theme.header_style())
        .height(1);

    let table = Table::new(
        rows,
        [Constraint::Percentage(75), Constraint::Percentage(25)],
    )
    .header(header)
    .block(
        Block::default()
            .title(Line::from(if search.is_empty() {
                vec![
                    Span::styled(" Namespaces ", theme.title_style()),
                    Span::styled(format!("({}) ", items.len()), theme.muted_style()),
                ]
            } else {
                vec![
                    Span::styled(" Namespaces ", theme.title_style()),
                    Span::styled(
                        format!("({} of {}) ", items.len(), cluster.namespace_list.len()),
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
