//! Ingresses list view.

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
        loading_or_empty_message,
    },
};

pub fn render_ingresses(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected: usize,
    search: &str,
) {
    let theme = default_theme();
    let items: Vec<_> = cluster
        .ingresses
        .iter()
        .filter(|i| search.is_empty() || i.name.contains(search) || i.namespace.contains(search))
        .collect();
    if items.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            search,
            "  Loading ingresses...",
            "  No ingresses found",
            "  No ingresses match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("Ingresses")),
            area,
        );
        return;
    }

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, ing)| {
            let style = if i == selected {
                theme.selection_style()
            } else {
                Style::default()
            };
            let hosts = if ing.hosts.is_empty() {
                "*".to_string()
            } else {
                ing.hosts.join(",")
            };
            let address = ing.address.as_deref().unwrap_or("<pending>");
            let class = ing.class.as_deref().unwrap_or("<none>");
            Row::new(vec![
                Cell::from(ing.name.clone()),
                Cell::from(ing.namespace.clone()),
                Cell::from(class.to_string()),
                Cell::from(hosts),
                Cell::from(address.to_string()),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec!["NAME", "NAMESPACE", "CLASS", "HOSTS", "ADDRESS"])
        .style(theme.header_style())
        .height(1);

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(25),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(30),
            Constraint::Percentage(15),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(Line::from(if search.is_empty() {
                vec![
                    Span::styled(" Ingresses ", theme.title_style()),
                    Span::styled(format!("({}) ", items.len()), theme.muted_style()),
                ]
            } else {
                vec![
                    Span::styled(" Ingresses ", theme.title_style()),
                    Span::styled(
                        format!("({} of {}) ", items.len(), cluster.ingresses.len()),
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

pub fn render_ingress_classes(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected: usize,
    search: &str,
) {
    let theme = default_theme();
    let items: Vec<_> = cluster
        .ingress_classes
        .iter()
        .filter(|ic| search.is_empty() || ic.name.contains(search))
        .collect();
    if items.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            search,
            "  Loading ingress classes...",
            "  No ingress classes found",
            "  No ingress classes match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("IngressClasses")),
            area,
        );
        return;
    }

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, ic)| {
            let style = if i == selected {
                theme.selection_style()
            } else {
                Style::default()
            };
            let default_label = if ic.is_default { "✓" } else { "" };
            Row::new(vec![
                Cell::from(ic.name.clone()),
                Cell::from(ic.controller.clone()),
                Cell::from(default_label),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec!["NAME", "CONTROLLER", "DEFAULT"])
        .style(theme.header_style())
        .height(1);

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(35),
            Constraint::Percentage(55),
            Constraint::Percentage(10),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(Line::from(if search.is_empty() {
                vec![
                    Span::styled(" IngressClasses ", theme.title_style()),
                    Span::styled(format!("({}) ", items.len()), theme.muted_style()),
                ]
            } else {
                vec![
                    Span::styled(" IngressClasses ", theme.title_style()),
                    Span::styled(
                        format!("({} of {}) ", items.len(), cluster.ingress_classes.len()),
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
