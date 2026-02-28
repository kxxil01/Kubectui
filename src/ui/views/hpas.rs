//! HorizontalPodAutoscaler list view.

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

pub fn render_hpas(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected: usize,
    search: &str,
) {
    let theme = default_theme();
    let items: Vec<_> = cluster
        .hpas
        .iter()
        .filter(|h| search.is_empty() || h.name.contains(search) || h.namespace.contains(search))
        .collect();
    if items.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            search,
            "  Loading horizontal pod autoscalers...",
            "  No horizontal pod autoscalers found",
            "  No horizontal pod autoscalers match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("HorizontalPodAutoscalers")),
            area,
        );
        return;
    }

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, hpa)| {
            let style = if i == selected {
                theme.selection_style()
            } else {
                Style::default()
            };
            let min = hpa.min_replicas.unwrap_or(1);
            let replicas = format!("{}/{}", hpa.current_replicas, hpa.desired_replicas);
            Row::new(vec![
                Cell::from(hpa.name.clone()),
                Cell::from(hpa.namespace.clone()),
                Cell::from(hpa.reference.clone()),
                Cell::from(format_small_int(i64::from(min))),
                Cell::from(format_small_int(i64::from(hpa.max_replicas))),
                Cell::from(replicas),
            ])
            .style(style)
        })
        .collect();

    let header = Row::new(vec![
        "NAME",
        "NAMESPACE",
        "REFERENCE",
        "MIN",
        "MAX",
        "REPLICAS",
    ])
    .style(theme.header_style())
    .height(1);

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(22),
            Constraint::Percentage(18),
            Constraint::Percentage(30),
            Constraint::Percentage(8),
            Constraint::Percentage(8),
            Constraint::Percentage(14),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(Line::from(if search.is_empty() {
                vec![
                    Span::styled(" HorizontalPodAutoscalers ", theme.title_style()),
                    Span::styled(format!("({}) ", items.len()), theme.muted_style()),
                ]
            } else {
                vec![
                    Span::styled(" HorizontalPodAutoscalers ", theme.title_style()),
                    Span::styled(
                        format!("({} of {}) ", items.len(), cluster.hpas.len()),
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
