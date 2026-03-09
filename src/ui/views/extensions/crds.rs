//! CRD picker renderer for Extensions view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Style},
    widgets::{Cell, Paragraph, Row, Table},
};

use crate::k8s::dtos::CustomResourceDefinitionInfo;
use crate::ui::{contains_ci, responsive_table_widths};

pub fn render_crd_picker(
    frame: &mut Frame,
    area: Rect,
    crds: &[CustomResourceDefinitionInfo],
    is_loading: bool,
    selected_idx: usize,
    query: &str,
    is_focused: bool,
) {
    let query_trimmed = query.trim();
    let filtered: Vec<&CustomResourceDefinitionInfo> = crds
        .iter()
        .filter(|crd| {
            if query_trimmed.is_empty() {
                true
            } else {
                contains_ci(&crd.name, query_trimmed)
                    || contains_ci(&crd.kind, query_trimmed)
                    || contains_ci(&crd.group, query_trimmed)
            }
        })
        .collect();

    if filtered.is_empty() {
        let empty_msg = if is_loading {
            "Loading CRDs..."
        } else if query_trimmed.is_empty() {
            "No CRDs found"
        } else {
            "No CRDs match search"
        };
        frame.render_widget(
            Paragraph::new(empty_msg).block(crate::ui::components::default_block("CRDs")),
            area,
        );
        return;
    }

    let clamped_idx = selected_idx.min(filtered.len().saturating_sub(1));
    let rows = filtered.iter().enumerate().map(|(idx, crd)| {
        let style = if is_focused && idx == clamped_idx {
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
        responsive_table_widths(
            area.width,
            [
                Constraint::Length(22),
                Constraint::Length(24),
                Constraint::Length(12),
                Constraint::Length(10),
            ],
        ),
    )
    .header(header)
    .block(crate::ui::components::default_block("CRDs"));

    frame.render_widget(table, area);
}
