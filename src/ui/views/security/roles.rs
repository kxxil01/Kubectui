use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Style},
    text::Line,
    widgets::{Cell, Paragraph, Row, Table},
};

use crate::{k8s::dtos::RbacRule, state::ClusterSnapshot, ui::components};

pub fn render_roles(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let query = query.trim().to_ascii_lowercase();
    let mut items: Vec<_> = cluster
        .roles
        .iter()
        .filter(|role| {
            query.is_empty()
                || role.name.to_ascii_lowercase().contains(&query)
                || role.namespace.to_ascii_lowercase().contains(&query)
        })
        .collect();
    items.sort_by_key(|r| {
        (
            r.namespace.to_ascii_lowercase(),
            r.name.to_ascii_lowercase(),
        )
    });

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No roles found").block(components::default_block("Roles")),
            area,
        );
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);

    let rows = items.iter().enumerate().map(|(idx, role)| {
        let style = if idx == selected_idx {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        Row::new(vec![
            Cell::from(role.name.clone()),
            Cell::from(role.namespace.clone()),
            Cell::from(role.rules.len().to_string()),
            Cell::from(format_age(role.age)),
        ])
        .style(style)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(28),
            Constraint::Length(18),
            Constraint::Length(8),
            Constraint::Fill(1),
        ],
    )
    .header(Row::new(["Name", "Namespace", "Rules", "Age"]).style(Style::default().fg(Color::Cyan)))
    .block(components::default_block("Roles"));
    frame.render_widget(table, chunks[0]);

    let idx = selected_idx.min(items.len().saturating_sub(1));
    let selected = items[idx];
    let detail = render_rule_tree(&selected.rules);
    frame.render_widget(
        Paragraph::new(detail).block(components::default_block("Selected Role Rules")),
        chunks[1],
    );
}

fn render_rule_tree(rules: &[RbacRule]) -> Vec<Line<'static>> {
    if rules.is_empty() {
        return vec![Line::from("No rules")];
    }

    let mut lines = Vec::new();
    for (idx, rule) in rules.iter().enumerate() {
        lines.push(Line::from(format!("Rule {}", idx + 1)));
        lines.push(Line::from(format!(
            "  ├─ verbs: {}",
            join_or_all(&rule.verbs)
        )));
        lines.push(Line::from(format!(
            "  ├─ apiGroups: {}",
            join_or_all(&rule.api_groups)
        )));
        lines.push(Line::from(format!(
            "  ├─ resources: {}",
            join_or_all(&rule.resources)
        )));
        if !rule.resource_names.is_empty() {
            lines.push(Line::from(format!(
                "  ├─ resourceNames: {}",
                rule.resource_names.join(", ")
            )));
        }
        if !rule.non_resource_urls.is_empty() {
            lines.push(Line::from(format!(
                "  └─ nonResourceURLs: {}",
                rule.non_resource_urls.join(", ")
            )));
        }
    }
    lines
}

fn join_or_all(items: &[String]) -> String {
    if items.is_empty() {
        "*".to_string()
    } else {
        items.join(", ")
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
    fn rule_tree_renders_rules_and_fields() {
        let lines = render_rule_tree(&[RbacRule {
            verbs: vec!["get".to_string()],
            api_groups: vec!["apps".to_string()],
            resources: vec!["deployments".to_string()],
            resource_names: vec!["api".to_string()],
            non_resource_urls: vec![],
        }]);

        let content = lines
            .into_iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(content.contains("Rule 1"));
        assert!(content.contains("verbs: get"));
        assert!(content.contains("resources: deployments"));
    }
}
