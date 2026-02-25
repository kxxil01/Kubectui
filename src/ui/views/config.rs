//! ConfigMaps and Secrets list views.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Row, Table},
};

use crate::{state::ClusterSnapshot, ui::components::default_theme};

pub fn render_config_maps(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected: usize,
    search: &str,
) {
    let theme = default_theme();
    let items: Vec<_> = cluster
        .config_maps
        .iter()
        .filter(|cm| search.is_empty() || cm.name.contains(search) || cm.namespace.contains(search))
        .collect();

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, cm)| {
            let style = if i == selected { theme.selection_style() } else { Style::default() };
            Row::new(vec![
                Cell::from(cm.name.clone()),
                Cell::from(cm.namespace.clone()),
                Cell::from(cm.data_count.to_string()),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec!["NAME", "NAMESPACE", "DATA"])
        .style(theme.header_style())
        .height(1);

    let table = Table::new(
        rows,
        [Constraint::Percentage(45), Constraint::Percentage(35), Constraint::Percentage(20)],
    )
    .header(header)
    .block(
        Block::default()
            .title(Line::from(if search.is_empty() {
                vec![
                    Span::styled(" ConfigMaps ", theme.title_style()),
                    Span::styled(format!("({}) ", items.len()), theme.muted_style()),
                ]
            } else {
                vec![
                    Span::styled(" ConfigMaps ", theme.title_style()),
                    Span::styled(format!("({} of {}) ", items.len(), cluster.config_maps.len()), theme.muted_style()),
                    Span::styled(format!("[/{search}]"), theme.muted_style()),
                ]
            }))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_active_style()),
    );

    frame.render_widget(table, area);
}

pub fn render_secrets(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected: usize,
    search: &str,
) {
    let theme = default_theme();
    let items: Vec<_> = cluster
        .secrets
        .iter()
        .filter(|s| search.is_empty() || s.name.contains(search) || s.namespace.contains(search))
        .collect();

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let style = if i == selected { theme.selection_style() } else { Style::default() };
            Row::new(vec![
                Cell::from(s.name.clone()),
                Cell::from(s.namespace.clone()),
                Cell::from(s.type_.clone()),
                Cell::from(s.data_count.to_string()),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec!["NAME", "NAMESPACE", "TYPE", "DATA"])
        .style(theme.header_style())
        .height(1);

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(35),
            Constraint::Percentage(25),
            Constraint::Percentage(30),
            Constraint::Percentage(10),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(Line::from(if search.is_empty() {
                vec![
                    Span::styled(" Secrets ", theme.title_style()),
                    Span::styled(format!("({}) ", items.len()), theme.muted_style()),
                ]
            } else {
                vec![
                    Span::styled(" Secrets ", theme.title_style()),
                    Span::styled(format!("({} of {}) ", items.len(), cluster.secrets.len()), theme.muted_style()),
                    Span::styled(format!("[/{search}]"), theme.muted_style()),
                ]
            }))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_active_style()),
    );

    frame.render_widget(table, area);
}
