//! Services list rendering.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::{
    state::{ClusterSnapshot, filters::filter_services},
    ui::components,
};

/// Renders the Services table for the current snapshot.
pub fn render_services(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let items = filter_services(&snapshot.services, query, None, None);

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No services found").block(components::default_block("Services")),
            area,
        );
        return;
    }

    let rows = items.iter().enumerate().map(|(idx, svc)| {
        let selected_style = if idx == selected_idx {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        Row::new(vec![
            Cell::from(svc.name.clone()),
            Cell::from(svc.namespace.clone()),
            Cell::from(svc.type_.clone()).style(service_type_style(&svc.type_)),
            Cell::from(svc.cluster_ip.clone().unwrap_or_else(|| "-".to_string())),
            Cell::from(format_ports(&svc.ports)),
            Cell::from(format_age(svc.age)),
        ])
        .style(selected_style)
    });

    let header = Row::new(["Name", "Namespace", "Type", "ClusterIP", "Ports", "Age"])
        .style(Style::default().fg(Color::Cyan));

    let table = Table::new(
        rows,
        [
            Constraint::Length(22),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Length(15),
            Constraint::Min(20),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(Block::default().title("Services").borders(Borders::ALL));

    frame.render_widget(table, area);
}

fn service_type_style(type_: &str) -> Style {
    if type_.eq_ignore_ascii_case("ClusterIP") {
        Style::default().fg(Color::Blue)
    } else if type_.eq_ignore_ascii_case("NodePort") {
        Style::default().fg(Color::Yellow)
    } else if type_.eq_ignore_ascii_case("LoadBalancer") {
        Style::default().fg(Color::Green)
    } else if type_.eq_ignore_ascii_case("ExternalName") {
        Style::default().fg(Color::Magenta)
    } else {
        Style::default().fg(Color::Gray)
    }
}

fn format_ports(ports: &[String]) -> String {
    if ports.is_empty() {
        return "-".to_string();
    }

    let joined = ports.join(", ");
    const MAX_LEN: usize = 28;

    if joined.chars().count() <= MAX_LEN {
        return joined;
    }

    let head = ports.first().cloned().unwrap_or_else(|| joined.clone());
    format!("{head}, ...")
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
