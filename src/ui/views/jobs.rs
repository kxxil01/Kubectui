//! Jobs list rendering.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::{
    state::{ClusterSnapshot, filters::filter_jobs},
    ui::components,
};

pub fn render_jobs(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let items = filter_jobs(&cluster.jobs, query, None);

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No jobs found").block(components::default_block("Jobs")),
            area,
        );
        return;
    }

    let rows = items.iter().enumerate().map(|(idx, job)| {
        let style = if idx == selected_idx {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        Row::new(vec![
            Cell::from(job.name.clone()),
            Cell::from(job.namespace.clone()),
            Cell::from(job.status.clone()).style(status_style(&job.status)),
            Cell::from(job.completions.clone()),
            Cell::from(job.duration.clone().unwrap_or_else(|| "-".to_string())),
            Cell::from(job.active_pods.to_string()),
            Cell::from(job.failed_pods.to_string()),
            Cell::from(format_age(job.age)),
        ])
        .style(style)
    });

    let header = Row::new([
        "Name",
        "Namespace",
        "Status",
        "Completions",
        "Duration",
        "Active",
        "Failed",
        "Age",
    ])
    .style(Style::default().fg(Color::Cyan));

    let table = Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(Block::default().title("Jobs").borders(Borders::ALL));

    frame.render_widget(table, area);
}

fn status_style(status: &str) -> Style {
    match status {
        "Succeeded" => Style::default().fg(Color::Green),
        "Running" => Style::default().fg(Color::Blue),
        "Failed" => Style::default().fg(Color::Red),
        _ => Style::default().fg(Color::Yellow),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_color_map() {
        assert_eq!(status_style("Succeeded").fg, Some(Color::Green));
        assert_eq!(status_style("Running").fg, Some(Color::Blue));
        assert_eq!(status_style("Failed").fg, Some(Color::Red));
        assert_eq!(status_style("Pending").fg, Some(Color::Yellow));
    }
}
