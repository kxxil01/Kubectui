//! DaemonSets list rendering.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::{
    state::{ClusterSnapshot, filters},
    ui::components,
};

/// Renders the DaemonSets table for the current snapshot.
pub fn render_daemonsets(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let items = filters::filter_daemonsets(&cluster.daemonsets, query, None);

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No daemonsets found").block(components::default_block("DaemonSets")),
            area,
        );
        return;
    }

    let rows = items.iter().enumerate().map(|(idx, ds)| {
        let selected_style = if idx == selected_idx {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        Row::new(vec![
            Cell::from(ds.name.clone()),
            Cell::from(ds.namespace.clone()),
            Cell::from(ds.desired_count.to_string()),
            Cell::from(ds.ready_count.to_string())
                .style(readiness_style(ds.ready_count, ds.desired_count)),
            Cell::from(ds.unavailable_count.to_string())
                .style(unavailable_style(ds.unavailable_count)),
            Cell::from(format_image(ds.image.as_deref())),
            Cell::from(format_age(ds.age)),
        ])
        .style(selected_style)
    });

    let header = Row::new([
        "Name",
        "Namespace",
        "Desired",
        "Ready",
        "Unavailable",
        "Image",
        "Age",
    ])
    .style(Style::default().fg(Color::Cyan));

    let table = Table::new(
        rows,
        [
            Constraint::Length(18),
            Constraint::Length(14),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(12),
            Constraint::Length(28),
            Constraint::Fill(1),
        ],
    )
    .header(header)
    .block(Block::default().title("DaemonSets").borders(Borders::ALL));

    frame.render_widget(table, area);
}

fn readiness_style(ready: i32, desired: i32) -> Style {
    if desired > 0 && ready >= desired {
        Style::default().fg(Color::Green)
    } else if ready > 0 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Red)
    }
}

fn unavailable_style(unavailable_count: i32) -> Style {
    if unavailable_count == 0 {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Red)
    }
}

fn format_image(image: Option<&str>) -> String {
    let Some(image) = image else {
        return "-".to_string();
    };

    const MAX_LEN: usize = 32;
    if image.chars().count() <= MAX_LEN {
        image.to_string()
    } else {
        format!(
            "{}...",
            image
                .chars()
                .take(MAX_LEN.saturating_sub(3))
                .collect::<String>()
        )
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
    fn readiness_style_maps_to_expected_colors() {
        assert_eq!(readiness_style(4, 4).fg, Some(Color::Green));
        assert_eq!(readiness_style(2, 4).fg, Some(Color::Yellow));
        assert_eq!(readiness_style(0, 4).fg, Some(Color::Red));
    }
}
