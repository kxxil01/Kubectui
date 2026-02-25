//! Events list view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Row, Table},
};

use crate::{state::ClusterSnapshot, ui::components::default_theme};

pub fn render_events(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected: usize,
    search: &str,
) {
    let theme = default_theme();
    let items: Vec<_> = cluster
        .events
        .iter()
        .filter(|e| {
            search.is_empty()
                || e.reason.contains(search)
                || e.message.contains(search)
                || e.involved_object.contains(search)
                || e.namespace.contains(search)
        })
        .collect();

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, ev)| {
            let style = if i == selected { theme.selection_style() } else { Style::default() };
            let type_style = if ev.type_ == "Warning" {
                theme.badge_warning_style()
            } else {
                theme.badge_success_style()
            };
            Row::new(vec![
                Cell::from(Span::styled(ev.type_.clone(), type_style)),
                Cell::from(ev.namespace.clone()),
                Cell::from(ev.involved_object.clone()),
                Cell::from(ev.reason.clone()),
                Cell::from(ev.count.to_string()),
                Cell::from(ev.message.chars().take(60).collect::<String>()),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec!["TYPE", "NAMESPACE", "OBJECT", "REASON", "COUNT", "MESSAGE"])
        .style(theme.header_style())
        .height(1);

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(8),
            Constraint::Percentage(12),
            Constraint::Percentage(20),
            Constraint::Percentage(15),
            Constraint::Percentage(5),
            Constraint::Percentage(40),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(Line::from(if search.is_empty() {
                vec![
                    Span::styled(" Events ", theme.title_style()),
                    Span::styled(format!("({}) ", items.len()), theme.muted_style()),
                ]
            } else {
                vec![
                    Span::styled(" Events ", theme.title_style()),
                    Span::styled(format!("({} of {}) ", items.len(), cluster.events.len()), theme.muted_style()),
                    Span::styled(format!("[/{search}]"), theme.muted_style()),
                ]
            }))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_active_style()),
    );

    frame.render_widget(table, area);
}
