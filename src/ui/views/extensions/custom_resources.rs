//! Custom resource instance list renderer for Extensions view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Style},
    widgets::{Cell, Paragraph, Row, Table},
};

use crate::k8s::dtos::CustomResourceInfo;
use crate::ui::format_age;

pub fn render_custom_resources(
    frame: &mut Frame,
    area: Rect,
    resources: &[CustomResourceInfo],
    error: Option<&str>,
    selected_idx: usize,
    is_focused: bool,
) {
    if let Some(err) = error {
        let theme = crate::ui::components::default_theme();
        frame.render_widget(
            Paragraph::new(ratatui::text::Line::from(vec![
                ratatui::text::Span::styled("⊘ ", Style::default().fg(theme.warning)),
                ratatui::text::Span::styled(
                    format!("Metrics/instances unavailable: {err}"),
                    theme.inactive_style(),
                ),
            ]))
            .alignment(ratatui::layout::Alignment::Center)
            .block(crate::ui::components::content_block(
                "Custom Resources",
                is_focused,
            )),
            area,
        );
        return;
    }

    if resources.is_empty() {
        let theme = crate::ui::components::default_theme();
        frame.render_widget(
            Paragraph::new(ratatui::text::Line::from(vec![
                ratatui::text::Span::styled("○ ", Style::default().fg(theme.fg_dim)),
                ratatui::text::Span::styled(
                    "Select a CRD to browse instances",
                    theme.inactive_style(),
                ),
            ]))
            .alignment(ratatui::layout::Alignment::Center)
            .block(crate::ui::components::content_block(
                "Custom Resources",
                is_focused,
            )),
            area,
        );
        return;
    }

    let clamped_idx = selected_idx.min(resources.len().saturating_sub(1));
    let rows = resources.iter().enumerate().map(|(idx, item)| {
        let style = if is_focused && idx == clamped_idx {
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
