//! Custom resource instance list renderer for Extensions view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Style},
    widgets::{Cell, Paragraph, Row, Table},
};

use crate::k8s::dtos::CustomResourceInfo;

pub fn render_custom_resources(
    frame: &mut Frame,
    area: Rect,
    resources: &[CustomResourceInfo],
    error: Option<&str>,
    selected_idx: usize,
    is_focused: bool,
) {
    if let Some(err) = error {
        frame.render_widget(
            Paragraph::new(format!("Metrics/instances unavailable: {err}"))
                .block(crate::ui::components::default_block("Custom Resources")),
            area,
        );
        return;
    }

    if resources.is_empty() {
        frame.render_widget(
            Paragraph::new("Select a CRD to browse instances")
                .block(crate::ui::components::default_block("Custom Resources")),
            area,
        );
        return;
    }

    let rows = resources.iter().enumerate().map(|(idx, item)| {
        let style = if is_focused && idx == selected_idx {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };
        Row::new(vec![
            Cell::from(item.name.clone()),
            Cell::from(
                item.namespace
                    .clone()
                    .unwrap_or_else(|| "<cluster-scope>".to_string()),
            ),
            Cell::from(format_age(item.age)),
        ])
        .style(style)
    });

    let header = Row::new(["Name", "Namespace", "Age"]).style(Style::default().fg(Color::Cyan));

    let title = if is_focused {
        format!("Custom Resources ({}) ▸ Enter to view", resources.len())
    } else {
        format!("Custom Resources ({})", resources.len())
    };

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(45),
            Constraint::Percentage(35),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(crate::ui::components::default_block(&title));

    frame.render_widget(table, area);
}

fn format_age(age: Option<std::time::Duration>) -> String {
    let Some(age) = age else {
        return "-".to_string();
    };

    let secs = age.as_secs();
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;

    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}
