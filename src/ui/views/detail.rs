//! Detail modal renderer.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Span, Style},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::DetailViewState;

/// Renders resource detail as a centered modal overlay.
pub fn render_detail(frame: &mut Frame, area: Rect, detail_state: &DetailViewState) {
    let popup = centered_rect(85, 85, area);
    frame.render_widget(Clear, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(9),
            Constraint::Length(6),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(popup);

    let title = if let Some(resource) = &detail_state.resource {
        format!(
            "{}: {}",
            resource.kind().to_ascii_uppercase(),
            resource.name()
        )
    } else {
        "DETAIL".to_string()
    };

    let header = Paragraph::new(Line::from(vec![Span::styled(
        title,
        Style::default().fg(Color::Cyan),
    )]))
    .block(Block::default().borders(Borders::ALL).title("Detail"));
    frame.render_widget(header, chunks[0]);

    let labels = if detail_state.metadata.labels.is_empty() {
        "-".to_string()
    } else {
        detail_state
            .metadata
            .labels
            .iter()
            .take(6)
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let metadata = vec![
        Line::from(format!("Name: {}", detail_state.metadata.name)),
        Line::from(format!(
            "Namespace: {}",
            detail_state
                .metadata
                .namespace
                .as_deref()
                .unwrap_or("<cluster-scope>")
        )),
        Line::from(format!(
            "Status: {}",
            detail_state.metadata.status.as_deref().unwrap_or("Unknown")
        )),
        Line::from(format!(
            "Node: {}",
            detail_state.metadata.node.as_deref().unwrap_or("n/a")
        )),
        Line::from(format!(
            "IP: {}",
            detail_state.metadata.ip.as_deref().unwrap_or("n/a")
        )),
        Line::from(format!(
            "Created: {}",
            detail_state.metadata.created.as_deref().unwrap_or("n/a")
        )),
        Line::from(format!("Labels: {labels}")),
    ];

    let metadata_widget = Paragraph::new(metadata)
        .block(Block::default().borders(Borders::ALL).title("Metadata"))
        .wrap(Wrap { trim: false });
    frame.render_widget(metadata_widget, chunks[1]);

    let mut resource_lines: Vec<Line<'_>> = detail_state
        .sections
        .iter()
        .map(|section| Line::from(section.clone()))
        .collect();
    if resource_lines.is_empty() {
        resource_lines.push(Line::from("No resource-specific details available."));
    }

    if !detail_state.events.is_empty() {
        resource_lines.push(Line::from(""));
        resource_lines.push(Line::from(Span::styled(
            "EVENTS",
            Style::default().fg(Color::Yellow),
        )));

        for event in detail_state.events.iter().take(4) {
            resource_lines.push(Line::from(format!(
                "{} {} (x{}): {}",
                if event.event_type.eq_ignore_ascii_case("warning") {
                    "⚠"
                } else {
                    "✓"
                },
                event.reason,
                event.count,
                event.message
            )));
        }
    }

    let resource_widget = Paragraph::new(resource_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Resource Details"),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(resource_widget, chunks[2]);

    let yaml_body = if detail_state.loading {
        "Loading detail...".to_string()
    } else if let Some(err) = &detail_state.error {
        format!("Error: {err}")
    } else {
        detail_state
            .yaml
            .clone()
            .unwrap_or_else(|| "YAML not available".to_string())
    };

    let yaml_widget = Paragraph::new(yaml_body)
        .block(Block::default().borders(Borders::ALL).title("YAML"))
        .wrap(Wrap { trim: false });
    frame.render_widget(yaml_widget, chunks[3]);

    let footer = Paragraph::new("[View YAML] [Logs] [Port Fwd] [Delete]    [Esc] Close")
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::ALL).title("Actions"));
    frame.render_widget(footer, chunks[4]);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
