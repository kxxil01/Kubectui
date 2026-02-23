//! CRD picker renderer for Extensions view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Style},
    widgets::{Cell, Paragraph, Row, Table},
};

use crate::k8s::dtos::CustomResourceDefinitionInfo;

pub fn render_crd_picker(
    frame: &mut Frame,
    area: Rect,
    crds: &[CustomResourceDefinitionInfo],
    selected_idx: usize,
    query: &str,
) {
    let query_lc = query.trim().to_lowercase();
    let filtered: Vec<&CustomResourceDefinitionInfo> = crds
        .iter()
        .filter(|crd| {
            if query_lc.is_empty() {
                true
            } else {
                crd.name.to_lowercase().contains(&query_lc)
                    || crd.kind.to_lowercase().contains(&query_lc)
                    || crd.group.to_lowercase().contains(&query_lc)
            }
        })
        .collect();

    if filtered.is_empty() {
        frame.render_widget(
            Paragraph::new("No CRDs found").block(crate::ui::components::default_block("CRDs")),
            area,
        );
        return;
    }

    let rows = filtered.iter().enumerate().map(|(idx, crd)| {
        let style = if idx == selected_idx {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        Row::new(vec![
            Cell::from(crd.kind.clone()),
            Cell::from(crd.group.clone()),
            Cell::from(crd.scope.clone()),
            Cell::from(crd.instances.to_string()),
        ])
        .style(style)
    });

    let header =
        Row::new(["Kind", "Group", "Scope", "Instances"]).style(Style::default().fg(Color::Cyan));

    let table = Table::new(
        rows,
        [
            Constraint::Length(22),
            Constraint::Length(24),
            Constraint::Length(12),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .block(crate::ui::components::default_block("CRDs"));

    frame.render_widget(table, area);
}
