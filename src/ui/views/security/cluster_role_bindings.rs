use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Style},
    text::Line,
    widgets::{Cell, Paragraph, Row, Table},
};

use crate::{k8s::dtos::RoleBindingSubject, state::ClusterSnapshot, ui::components};

pub fn render_cluster_role_bindings(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let query = query.trim().to_ascii_lowercase();
    let mut items: Vec<_> = cluster
        .cluster_role_bindings
        .iter()
        .filter(|rb| {
            query.is_empty()
                || rb.name.to_ascii_lowercase().contains(&query)
                || rb.role_ref_name.to_ascii_lowercase().contains(&query)
        })
        .collect();
    items.sort_by_key(|rb| rb.name.to_ascii_lowercase());

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No clusterrolebindings found")
                .block(components::default_block("ClusterRoleBindings")),
            area,
        );
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);

    let rows = items.iter().enumerate().map(|(idx, rb)| {
        let style = if idx == selected_idx {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        Row::new(vec![
            Cell::from(rb.name.clone()),
            Cell::from(format!("{}/{}", rb.role_ref_kind, rb.role_ref_name)),
            Cell::from(rb.subjects.len().to_string()),
            Cell::from(format_age(rb.age)),
        ])
        .style(style)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(30),
            Constraint::Length(38),
            Constraint::Length(9),
            Constraint::Fill(1),
        ],
    )
    .header(
        Row::new(["Name", "RoleRef", "Subjects", "Age"]).style(Style::default().fg(Color::Cyan)),
    )
    .block(components::default_block("ClusterRoleBindings"));
    frame.render_widget(table, chunks[0]);

    let idx = selected_idx.min(items.len().saturating_sub(1));
    let selected = items[idx];
    let detail = render_subjects(&selected.subjects);
    frame.render_widget(
        Paragraph::new(detail).block(components::default_block("Selected Binding Subjects")),
        chunks[1],
    );
}

fn render_subjects(subjects: &[RoleBindingSubject]) -> Vec<Line<'static>> {
    if subjects.is_empty() {
        return vec![Line::from("No subjects")];
    }

    subjects
        .iter()
        .map(|subject| {
            let ns = subject.namespace.as_deref().unwrap_or("-");
            let api_group = subject.api_group.as_deref().unwrap_or("-");
            Line::from(format!(
                "- {}/{} (ns={}, apiGroup={})",
                subject.kind, subject.name, ns, api_group
            ))
        })
        .collect()
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
