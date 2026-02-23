//! StatefulSets list rendering.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::{
    state::{ClusterSnapshot, filters},
    ui::components,
};

/// Renders the StatefulSets table for the current snapshot.
pub fn render_statefulsets(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let items = filters::filter_statefulsets(&cluster.statefulsets, query, None);

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No statefulsets found")
                .block(components::default_block("StatefulSets")),
            area,
        );
        return;
    }

    let rows = items.iter().enumerate().map(|(idx, ss)| {
        let selected_style = if idx == selected_idx {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        Row::new(vec![
            Cell::from(ss.name.clone()),
            Cell::from(ss.namespace.clone()),
            Cell::from(format!("{}/{}", ss.ready_replicas, ss.desired_replicas))
                .style(readiness_style(ss.ready_replicas, ss.desired_replicas)),
            Cell::from(ss.service_name.clone()),
            Cell::from(format_image(ss.image.as_deref())),
            Cell::from(format_age(ss.age)),
        ])
        .style(selected_style)
    });

    let header = Row::new(["Name", "Namespace", "Ready", "Service", "Image", "Age"])
        .style(Style::default().fg(Color::Cyan));

    let table = Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Length(15),
            Constraint::Length(10),
            Constraint::Length(20),
            Constraint::Length(25),
            Constraint::Fill(1),
        ],
    )
    .header(header)
    .block(Block::default().title("StatefulSets").borders(Borders::ALL));

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

fn format_image(image: Option<&str>) -> String {
    let Some(image) = image else {
        return "-".to_string();
    };

    const MAX_LEN: usize = 30;
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
        assert_eq!(readiness_style(3, 3).fg, Some(Color::Green));
        assert_eq!(readiness_style(1, 3).fg, Some(Color::Yellow));
        assert_eq!(readiness_style(0, 3).fg, Some(Color::Red));
    }
}
