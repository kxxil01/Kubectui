//! LimitRanges list rendering.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::{
    state::{ClusterSnapshot, filters::filter_limit_ranges},
    ui::components,
};

pub fn render_limit_ranges(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let items = filter_limit_ranges(&cluster.limit_ranges, query, None);

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No limit ranges found")
                .block(components::default_block("Governance / LimitRanges")),
            area,
        );
        return;
    }

    let rows = items.iter().enumerate().map(|(idx, lr)| {
        let style = if idx == selected_idx {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        Row::new(vec![
            Cell::from(lr.name.clone()),
            Cell::from(lr.namespace.clone()),
            Cell::from(lr.limits.len().to_string()),
            Cell::from(limit_types_summary(lr)),
            Cell::from(format_age(lr.age)),
        ])
        .style(style)
    });

    let header = Row::new(["Name", "Namespace", "Specs", "Types", "Age"])
        .style(Style::default().fg(Color::Cyan));

    let table = Table::new(
        rows,
        [
            Constraint::Length(28),
            Constraint::Length(18),
            Constraint::Length(8),
            Constraint::Min(24),
            Constraint::Fill(1),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title("Governance / LimitRanges")
            .borders(Borders::ALL),
    );

    frame.render_widget(table, area);
}

fn limit_types_summary(lr: &crate::k8s::dtos::LimitRangeInfo) -> String {
    let mut types = lr
        .limits
        .iter()
        .map(|spec| spec.type_.clone())
        .collect::<Vec<_>>();
    types.sort();
    types.dedup();

    if types.is_empty() {
        "-".to_string()
    } else {
        types.join(",")
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
    use crate::k8s::dtos::{LimitRangeInfo, LimitSpec};

    use super::*;

    #[test]
    fn summary_deduplicates_types() {
        let info = LimitRangeInfo {
            limits: vec![
                LimitSpec {
                    type_: "Container".to_string(),
                    ..LimitSpec::default()
                },
                LimitSpec {
                    type_: "Container".to_string(),
                    ..LimitSpec::default()
                },
                LimitSpec {
                    type_: "Pod".to_string(),
                    ..LimitSpec::default()
                },
            ],
            ..LimitRangeInfo::default()
        };

        assert_eq!(limit_types_summary(&info), "Container,Pod");
    }
}
