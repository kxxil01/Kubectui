//! Helm releases view.

use ratatui::{
    layout::{Margin, Rect},
    prelude::{Color, Frame, Style},
    text::Span,
    widgets::{
        Cell, HighlightSpacing, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
        TableState,
    },
};

use crate::ui::contains_ci;
use crate::{
    app::AppView,
    state::ClusterSnapshot,
    ui::{
        components::{active_block, default_theme},
        filter_cache::{cached_filter_indices, data_fingerprint},
        loading_or_empty_message, table_viewport_rows, table_window,
    },
};

/// Renders the Helm releases table.
pub fn render_helm_releases(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search_query: &str,
) {
    let theme = default_theme();
    let query = search_query.trim();

    let indices = cached_filter_indices(
        AppView::HelmReleases,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.helm_releases),
        |q| {
            if q.is_empty() {
                return (0..cluster.helm_releases.len()).collect();
            }
            cluster
                .helm_releases
                .iter()
                .enumerate()
                .filter_map(|(idx, release)| {
                    (contains_ci(&release.name, q)
                        || contains_ci(&release.namespace, q)
                        || contains_ci(&release.chart, q))
                    .then_some(idx)
                })
                .collect()
        },
    );

    let header = Row::new([
        Cell::from(Span::styled("  NAME", theme.header_style())),
        Cell::from(Span::styled("NAMESPACE", theme.header_style())),
        Cell::from(Span::styled("CHART", theme.header_style())),
        Cell::from(Span::styled("VERSION", theme.header_style())),
        Cell::from(Span::styled("STATUS", theme.header_style())),
        Cell::from(Span::styled("REVISION", theme.header_style())),
        Cell::from(Span::styled("UPDATED", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    if indices.is_empty() {
        let empty_msg = loading_or_empty_message(
            cluster,
            query,
            "Loading Helm releases...",
            "No Helm releases found (Helm v3 stores releases as Kubernetes Secrets)",
            "No releases match search",
        );
        let title = if query.is_empty() {
            " Helm Releases (0) ".to_string()
        } else {
            let all = cluster.helm_releases.len();
            format!(" Helm Releases (0 of {all}) [/{query}]")
        };
        frame.render_widget(
            ratatui::widgets::Paragraph::new(Span::styled(
                format!("  {empty_msg}"),
                Style::default().fg(theme.fg_dim),
            ))
            .block(active_block(&title)),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &release_idx)| {
            let idx = window.start + local_idx;
            let release = &cluster.helm_releases[release_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let status_style = match release.status.as_str() {
                "deployed" => Style::default().fg(Color::Green),
                "failed" => Style::default().fg(Color::Red),
                "pending-install" | "pending-upgrade" | "pending-rollback" => {
                    Style::default().fg(Color::Yellow)
                }
                "superseded" => Style::default().fg(Color::DarkGray),
                _ => Style::default().fg(theme.fg_dim),
            };

            let updated = release
                .updated
                .map(|ts| ts.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "-".to_string());

            let chart_display = if release.chart_version.is_empty() {
                release.chart.clone()
            } else {
                format!("{}-{}", release.chart, release.chart_version)
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", release.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    release.namespace.clone(),
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(chart_display).style(Style::default().fg(theme.fg_dim)),
                Cell::from(release.chart_version.clone()).style(Style::default().fg(theme.fg_dim)),
                Cell::from(release.status.clone()).style(status_style),
                Cell::from(release.revision.to_string()).style(Style::default().fg(theme.fg_dim)),
                Cell::from(updated).style(Style::default().fg(theme.fg_dim)),
            ])
            .style(row_style)
        })
        .collect();

    let title = if query.is_empty() {
        format!(" Helm Releases ({total}) ")
    } else {
        let all = cluster.helm_releases.len();
        format!(" Helm Releases ({total} of {all}) [/{query}]")
    };
    let block = active_block(&title);

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
    .highlight_symbol(theme.highlight_symbol())
    .highlight_spacing(HighlightSpacing::Always);

    let mut state = TableState::default().with_selected(Some(window.selected));
    frame.render_stateful_widget(table, area, &mut state);
    render_table_scrollbar(frame, area, total, selected);
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
    let query = search_query.trim();

    let indices = cached_filter_indices(
        AppView::HelmCharts,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.helm_repositories),
        |q| {
            if q.is_empty() {
                return (0..cluster.helm_repositories.len()).collect();
            }
            cluster
                .helm_repositories
                .iter()
                .enumerate()
                .filter_map(|(idx, repo)| {
                    (contains_ci(&repo.name, q) || contains_ci(&repo.url, q)).then_some(idx)
                })
                .collect()
        },
    );

    let header = Row::new([
        Cell::from(Span::styled("  NAME", theme.header_style())),
        Cell::from(Span::styled("URL", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    if indices.is_empty() {
        let empty_msg = loading_or_empty_message(
            cluster,
            query,
            "Loading Helm repositories...",
            "No Helm repositories configured (helm repo add <name> <url>)",
            "No repositories match search",
        );
        let title = if query.is_empty() {
            " Helm Repositories (0) ".to_string()
        } else {
            let all = cluster.helm_repositories.len();
            format!(" Helm Repositories (0 of {all}) [/{query}]")
        };
        frame.render_widget(
            ratatui::widgets::Paragraph::new(Span::styled(
                format!("  {empty_msg}"),
                Style::default().fg(theme.fg_dim),
            ))
            .block(active_block(&title)),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &repo_idx)| {
            let idx = window.start + local_idx;
            let repo = &cluster.helm_repositories[repo_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", repo.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    repo.url.clone(),
                    Style::default().fg(theme.accent2),
                )),
            ])
            .style(row_style)
        })
        .collect();

    let title = if query.is_empty() {
        format!(" Helm Repositories ({total}) ")
    } else {
        let all = cluster.helm_repositories.len();
        format!(" Helm Repositories ({total} of {all}) [/{query}]")
    };
    let block = active_block(&title);

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
    .highlight_symbol(theme.highlight_symbol())
    .highlight_spacing(HighlightSpacing::Always);

    let mut state = TableState::default().with_selected(Some(window.selected));
    frame.render_stateful_widget(table, area, &mut state);
    render_table_scrollbar(frame, area, total, selected);
}

fn render_table_scrollbar(frame: &mut Frame, area: Rect, total: usize, selected: usize) {
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"))
        .track_symbol(Some("│"))
        .thumb_symbol("█");
    let mut scrollbar_state = ScrollbarState::new(total).position(selected);
    frame.render_stateful_widget(
        scrollbar,
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}
