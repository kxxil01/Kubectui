//! PodDisruptionBudgets list rendering.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::{
    state::{ClusterSnapshot, filters::filter_pod_disruption_budgets},
    ui::components,
};

pub fn render_pdbs(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let items = filter_pod_disruption_budgets(&cluster.pod_disruption_budgets, query, None);

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No pod disruption budgets found")
                .block(components::default_block("Governance / PDBs")),
            area,
        );
        return;
    }

    let rows = items.iter().enumerate().map(|(idx, pdb)| {
        let style = if idx == selected_idx {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        Row::new(vec![
            Cell::from(pdb.name.clone()),
            Cell::from(pdb.namespace.clone()),
            Cell::from(
                pdb.min_available
                    .clone()
                    .or_else(|| pdb.max_unavailable.clone())
                    .unwrap_or_else(|| "-".to_string()),
            ),
            Cell::from(format!("{}/{}", pdb.current_healthy, pdb.desired_healthy)),
            Cell::from(pdb.disruptions_allowed.to_string())
                .style(disruption_style(pdb.disruptions_allowed)),
            Cell::from(format_age(pdb.age)),
        ])
        .style(style)
    });

    let header = Row::new([
        "Name",
        "Namespace",
        "Policy",
        "Healthy",
        "Disruptions Allowed",
        "Age",
    ])
    .style(Style::default().fg(Color::Cyan));

    let table = Table::new(
        rows,
        [
            Constraint::Length(28),
            Constraint::Length(18),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(20),
            Constraint::Fill(1),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title("Governance / PodDisruptionBudgets")
            .borders(Borders::ALL),
    );

    frame.render_widget(table, area);
}

fn disruption_style(disruptions_allowed: i32) -> Style {
    if disruptions_allowed > 0 {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Yellow)
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
    fn disruption_style_expected_colors() {
        assert_eq!(disruption_style(2).fg, Some(Color::Green));
        assert_eq!(disruption_style(0).fg, Some(Color::Yellow));
    }
}
