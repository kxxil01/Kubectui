//! PriorityClasses list view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Row, Table},
};

use crate::{
    state::ClusterSnapshot,
    ui::{components::default_theme, format_small_int},
};

pub fn render_priority_classes(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected: usize,
    search: &str,
) {
    let theme = default_theme();
    let items: Vec<_> = cluster
        .priority_classes
        .iter()
        .filter(|pc| search.is_empty() || pc.name.contains(search))
        .collect();

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, pc)| {
            let style = if i == selected {
                theme.selection_style()
            } else {
                Style::default()
            };
            let default_label = if pc.global_default { "✓" } else { "" };
            Row::new(vec![
                Cell::from(pc.name.clone()),
                Cell::from(format_small_int(i64::from(pc.value))),
                Cell::from(default_label),
                Cell::from(pc.description.chars().take(60).collect::<String>()),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec!["NAME", "VALUE", "GLOBAL DEFAULT", "DESCRIPTION"])
        .style(theme.header_style())
        .height(1);

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(10),
            Constraint::Percentage(15),
            Constraint::Percentage(45),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(Line::from(if search.is_empty() {
                vec![
                    Span::styled(" PriorityClasses ", theme.title_style()),
                    Span::styled(format!("({}) ", items.len()), theme.muted_style()),
                ]
            } else {
                vec![
                    Span::styled(" PriorityClasses ", theme.title_style()),
                    Span::styled(
                        format!("({} of {}) ", items.len(), cluster.priority_classes.len()),
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
