//! ResourceQuotas list rendering.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::{
    state::{ClusterSnapshot, filters::filter_resource_quotas},
    ui::components,
};

pub fn render_resource_quotas(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let items = filter_resource_quotas(&cluster.resource_quotas, query, None);

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No resource quotas found")
                .block(components::default_block("Governance / ResourceQuotas")),
            area,
        );
        return;
    }

    let rows = items.iter().enumerate().map(|(idx, rq)| {
        let style = if idx == selected_idx {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        let (tracked, max_pct) = quota_summary(rq);

        Row::new(vec![
            Cell::from(rq.name.clone()),
            Cell::from(rq.namespace.clone()),
            Cell::from(tracked.to_string()),
            Cell::from(format!("{max_pct:.0}%")).style(usage_style(max_pct)),
            Cell::from(format_age(rq.age)),
        ])
        .style(style)
    });

    let header = Row::new(["Name", "Namespace", "Tracked", "Max Used", "Age"])
        .style(Style::default().fg(Color::Cyan));

    let table = Table::new(
        rows,
        [
            Constraint::Length(28),
            Constraint::Length(18),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Fill(1),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title("Governance / ResourceQuotas")
            .borders(Borders::ALL),
    );

    frame.render_widget(table, area);
}

fn quota_summary(rq: &crate::k8s::dtos::ResourceQuotaInfo) -> (usize, f64) {
    let tracked = rq.percent_used.len();
    let max_pct = rq
        .percent_used
        .values()
        .fold(0.0_f64, |acc, value| acc.max(*value));
    (tracked, max_pct)
}

fn usage_style(percent: f64) -> Style {
    if percent >= 90.0 {
        Style::default().fg(Color::Red)
    } else if percent >= 70.0 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Green)
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
    fn usage_style_thresholds() {
        assert_eq!(usage_style(35.0).fg, Some(Color::Green));
        assert_eq!(usage_style(75.0).fg, Some(Color::Yellow));
        assert_eq!(usage_style(95.0).fg, Some(Color::Red));
    }
}
