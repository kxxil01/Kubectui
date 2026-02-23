//! Deployments list rendering.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::{
    state::{
        ClusterSnapshot,
        filters::{DeploymentHealth, deployment_health_from_ready, filter_deployments},
    },
    ui::components,
};

/// Renders the Deployments table for the current snapshot.
pub fn render_deployments(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let items = filter_deployments(&snapshot.deployments, query, None, None);

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No deployments found").block(components::default_block("Deployments")),
            area,
        );
        return;
    }

    let rows = items.iter().enumerate().map(|(idx, deploy)| {
        let selected_style = if idx == selected_idx {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        let health = deployment_health_from_ready(&deploy.ready);

        Row::new(vec![
            Cell::from(deploy.name.clone()),
            Cell::from(deploy.namespace.clone()),
            Cell::from(deploy.ready.clone()).style(health_style(health)),
            Cell::from(deploy.updated.to_string()),
            Cell::from(deploy.available.to_string()),
            Cell::from(format_age(deploy.age)),
            Cell::from(format_image(deploy.image.as_deref())),
        ])
        .style(selected_style)
    });

    let header = Row::new([
        "Name",
        "Namespace",
        "Ready",
        "Updated",
        "Available",
        "Age",
        "Image",
    ])
    .style(Style::default().fg(Color::Cyan));

    let table = Table::new(
        rows,
        [
            Constraint::Length(22),
            Constraint::Length(14),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Min(24),
        ],
    )
    .header(header)
    .block(Block::default().title("Deployments").borders(Borders::ALL));

    frame.render_widget(table, area);
}

fn health_style(health: DeploymentHealth) -> Style {
    match health {
        DeploymentHealth::Healthy => Style::default().fg(Color::Green),
        DeploymentHealth::Degraded => Style::default().fg(Color::Yellow),
        DeploymentHealth::Failed => Style::default().fg(Color::Red),
    }
}

fn format_image(image: Option<&str>) -> String {
    let Some(image) = image else {
        return "-".to_string();
    };

    const MAX_LEN: usize = 34;
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

    /// Verifies health style colors match deployment health state.
    #[test]
    fn health_style_mapping() {
        assert_eq!(
            health_style(DeploymentHealth::Healthy).fg,
            Some(Color::Green)
        );
        assert_eq!(
            health_style(DeploymentHealth::Degraded).fg,
            Some(Color::Yellow)
        );
        assert_eq!(health_style(DeploymentHealth::Failed).fg, Some(Color::Red));
    }

    /// Verifies image values are truncated when exceeding render width.
    #[test]
    fn format_image_truncates_long_strings() {
        let long = "registry.io/team/service:very-long-tag-1234567890";
        let out = format_image(Some(long));
        assert!(out.ends_with("..."));
        assert!(out.len() <= 37);
    }

    /// Verifies missing image renders a dash placeholder.
    #[test]
    fn format_image_empty_placeholder() {
        assert_eq!(format_image(None), "-");
    }
}
