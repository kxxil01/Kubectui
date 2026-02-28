//! ConfigMaps and Secrets list views.

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
    if items.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            search,
            "  Loading configmaps...",
            "  No configmaps found",
            "  No configmaps match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("ConfigMaps")),
            area,
        );
        return;
    }

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, cm)| {
            let style = if i == selected {
                theme.selection_style()
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(cm.name.clone()),
                Cell::from(cm.namespace.clone()),
                Cell::from(format_small_int(cm.data_count as i64)),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec!["NAME", "NAMESPACE", "DATA"])
        .style(theme.header_style())
        .height(1);

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(45),
            Constraint::Percentage(35),
            Constraint::Percentage(20),
        ],
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
                    Span::styled(
                        format!("({} of {}) ", items.len(), cluster.config_maps.len()),
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
    if items.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            search,
            "  Loading secrets...",
            "  No secrets found",
            "  No secrets match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("Secrets")),
            area,
        );
        return;
    }

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let style = if i == selected {
                theme.selection_style()
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(s.name.clone()),
                Cell::from(s.namespace.clone()),
                Cell::from(s.type_.clone()),
                Cell::from(format_small_int(s.data_count as i64)),
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
                    Span::styled(
                        format!("({} of {}) ", items.len(), cluster.secrets.len()),
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
