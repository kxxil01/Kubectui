//! CronJobs list rendering.

use chrono::{DateTime, Local, Utc};
use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::{
    state::{ClusterSnapshot, filters::filter_cronjobs},
    ui::components,
};

pub fn render_cronjobs(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let items = filter_cronjobs(&cluster.cronjobs, query, None);

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No cronjobs found").block(components::default_block("CronJobs")),
            area,
        );
        return;
    }

    let rows = items.iter().enumerate().map(|(idx, cj)| {
        let style = if idx == selected_idx {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        Row::new(vec![
            Cell::from(cj.name.clone()),
            Cell::from(cj.namespace.clone()),
            Cell::from(cj.schedule.clone()),
            Cell::from(format_time(cj.last_schedule_time)),
            Cell::from(format_time(cj.next_schedule_time)),
            Cell::from(cj.active_jobs.to_string()),
            Cell::from(suspend_indicator(cj.suspend)),
            Cell::from(format_age(cj.age)),
        ])
        .style(style)
    });

    let header = Row::new([
        "Name",
        "Namespace",
        "Schedule",
        "Last Run",
        "Next Run",
        "Active",
        "Suspend",
        "Age",
    ])
    .style(Style::default().fg(Color::Cyan));

    let table = Table::new(
        rows,
        [
            Constraint::Length(18),
            Constraint::Length(14),
            Constraint::Length(16),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(Block::default().title("CronJobs").borders(Borders::ALL));

    frame.render_widget(table, area);
}

fn suspend_indicator(suspend: bool) -> &'static str {
    if suspend { "🚫" } else { "✅" }
}

fn format_time(ts: Option<DateTime<Utc>>) -> String {
    ts.map(|value| {
        value
            .with_timezone(&Local)
            .format("%m-%d %H:%M")
            .to_string()
    })
    .unwrap_or_else(|| "-".to_string())
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
    fn suspend_icon() {
        assert_eq!(suspend_indicator(true), "🚫");
        assert_eq!(suspend_indicator(false), "✅");
    }
}
