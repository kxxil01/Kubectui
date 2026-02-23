use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Style},
    widgets::{Cell, Paragraph, Row, Table},
};

use crate::{state::ClusterSnapshot, ui::components};

pub fn render_service_accounts(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let query = query.trim().to_ascii_lowercase();
    let mut items: Vec<_> = cluster
        .service_accounts
        .iter()
        .filter(|sa| {
            query.is_empty()
                || sa.name.to_ascii_lowercase().contains(&query)
                || sa.namespace.to_ascii_lowercase().contains(&query)
        })
        .collect();
    items.sort_by_key(|sa| {
        (
            sa.namespace.to_ascii_lowercase(),
            sa.name.to_ascii_lowercase(),
        )
    });

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No serviceaccounts found")
                .block(components::default_block("ServiceAccounts")),
            area,
        );
        return;
    }

    let rows = items.iter().enumerate().map(|(idx, sa)| {
        let style = if idx == selected_idx {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        Row::new(vec![
            Cell::from(sa.name.clone()),
            Cell::from(sa.namespace.clone()),
            Cell::from(sa.secrets_count.to_string()),
            Cell::from(sa.image_pull_secrets_count.to_string()),
            Cell::from(match sa.automount_service_account_token {
                Some(true) => "true".to_string(),
                Some(false) => "false".to_string(),
                None => "<unset>".to_string(),
            }),
            Cell::from(format_age(sa.age)),
        ])
        .style(style)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(24),
            Constraint::Length(18),
            Constraint::Length(8),
            Constraint::Length(11),
            Constraint::Length(10),
            Constraint::Fill(1),
        ],
    )
    .header(
        Row::new([
            "Name",
            "Namespace",
            "Secrets",
            "PullSecrets",
            "Automount",
            "Age",
        ])
        .style(Style::default().fg(Color::Cyan)),
    )
    .block(components::default_block("ServiceAccounts"));

    frame.render_widget(table, area);
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
