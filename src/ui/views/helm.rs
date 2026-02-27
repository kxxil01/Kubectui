//! Helm releases view.

use ratatui::{
    layout::Rect,
    prelude::{Color, Frame, Style},
    text::Span,
    widgets::{Cell, Row, Table, TableState},
};

use crate::{
    state::ClusterSnapshot,
    ui::components::{active_block, default_theme},
};

use super::super::contains_ci;

/// Renders the Helm releases table.
pub fn render_helm_releases(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search_query: &str,
) {
    let theme = default_theme();

    let releases: Vec<_> = cluster
        .helm_releases
        .iter()
        .filter(|r| {
            search_query.is_empty()
                || contains_ci(&r.name, search_query)
                || contains_ci(&r.namespace, search_query)
                || contains_ci(&r.chart, search_query)
        })
        .collect();

    let header = Row::new(vec![
        Cell::from("NAME").style(theme.header_style()),
        Cell::from("NAMESPACE").style(theme.header_style()),
        Cell::from("CHART").style(theme.header_style()),
        Cell::from("VERSION").style(theme.header_style()),
        Cell::from("STATUS").style(theme.header_style()),
        Cell::from("REVISION").style(theme.header_style()),
        Cell::from("UPDATED").style(theme.header_style()),
    ])
    .height(1);

    let rows: Vec<Row> = releases
        .iter()
        .map(|r| {
            let status_style = match r.status.as_str() {
                "deployed" => Style::default().fg(Color::Green),
                "failed" => Style::default().fg(Color::Red),
                "pending-install" | "pending-upgrade" | "pending-rollback" => {
                    Style::default().fg(Color::Yellow)
                }
                "superseded" => Style::default().fg(Color::DarkGray),
                _ => Style::default().fg(theme.fg_dim),
            };

            let updated = r
                .updated
                .map(|ts| ts.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "-".to_string());

            let chart_display = if r.chart_version.is_empty() {
                r.chart.clone()
            } else {
                format!("{}-{}", r.chart, r.chart_version)
            };

            Row::new(vec![
                Cell::from(r.name.clone()).style(Style::default().fg(theme.fg)),
                Cell::from(r.namespace.clone()).style(Style::default().fg(theme.accent2)),
                Cell::from(chart_display).style(Style::default().fg(theme.fg_dim)),
                Cell::from(r.chart_version.clone()).style(Style::default().fg(theme.fg_dim)),
                Cell::from(r.status.clone()).style(status_style),
                Cell::from(r.revision.to_string()).style(Style::default().fg(theme.fg_dim)),
                Cell::from(updated).style(Style::default().fg(theme.fg_dim)),
            ])
        })
        .collect();

    let empty_msg = if cluster.helm_releases.is_empty() {
        "No Helm releases found (Helm v3 stores releases as Kubernetes Secrets)"
    } else {
        "No releases match search"
    };

    let title = if search_query.is_empty() {
        format!(" Helm Releases ({}) ", releases.len())
    } else {
        let all = cluster.helm_releases.len();
        format!(
            " Helm Releases ({} of {all}) [/{search_query}]",
            releases.len()
        )
    };
    let block = active_block(&title);

    if rows.is_empty() {
        frame.render_widget(
            ratatui::widgets::Paragraph::new(Span::styled(
                format!("  {empty_msg}"),
                Style::default().fg(theme.fg_dim),
            ))
            .block(block),
            area,
        );
        return;
    }

    let table = Table::new(
        rows,
        [
            ratatui::layout::Constraint::Percentage(18),
            ratatui::layout::Constraint::Percentage(14),
            ratatui::layout::Constraint::Percentage(20),
            ratatui::layout::Constraint::Percentage(10),
            ratatui::layout::Constraint::Percentage(14),
            ratatui::layout::Constraint::Percentage(8),
            ratatui::layout::Constraint::Percentage(16),
        ],
    )
    .header(header)
    .block(block)
    .row_highlight_style(theme.selection_style())
    .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);

    let clamped = selected_idx.min(releases.len().saturating_sub(1));
    let mut state = TableState::default().with_selected(Some(clamped));
    frame.render_stateful_widget(table, area, &mut state);
}

/// Renders the Helm repositories table (local config from ~/.config/helm/repositories.yaml).
pub fn render_helm_repos(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search_query: &str,
) {
    let theme = default_theme();

    let repos: Vec<_> = cluster
        .helm_repositories
        .iter()
        .filter(|r| {
            search_query.is_empty()
                || contains_ci(&r.name, search_query)
                || contains_ci(&r.url, search_query)
        })
        .collect();

    let header = Row::new(vec![
        Cell::from("NAME").style(theme.header_style()),
        Cell::from("URL").style(theme.header_style()),
    ])
    .height(1);

    let rows: Vec<Row> = repos
        .iter()
        .map(|r| {
            Row::new(vec![
                Cell::from(r.name.clone()).style(Style::default().fg(theme.fg)),
                Cell::from(r.url.clone()).style(Style::default().fg(theme.accent2)),
            ])
        })
        .collect();

    let empty_msg = if cluster.helm_repositories.is_empty() {
        "No Helm repositories configured (helm repo add <name> <url>)"
    } else {
        "No repositories match search"
    };

    let title = if search_query.is_empty() {
        format!(" Helm Repositories ({}) ", repos.len())
    } else {
        let all = cluster.helm_repositories.len();
        format!(
            " Helm Repositories ({} of {all}) [/{search_query}]",
            repos.len()
        )
    };
    let block = active_block(&title);

    if rows.is_empty() {
        frame.render_widget(
            ratatui::widgets::Paragraph::new(Span::styled(
                format!("  {empty_msg}"),
                Style::default().fg(theme.fg_dim),
            ))
            .block(block),
            area,
        );
        return;
    }

    let table = Table::new(
        rows,
        [
            ratatui::layout::Constraint::Percentage(30),
            ratatui::layout::Constraint::Percentage(70),
        ],
    )
    .header(header)
    .block(block)
    .row_highlight_style(theme.selection_style())
    .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);

    let clamped = selected_idx.min(repos.len().saturating_sub(1));
    let mut state = TableState::default().with_selected(Some(clamped));
    frame.render_stateful_widget(table, area, &mut state);
}
