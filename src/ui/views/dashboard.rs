//! Dashboard renderer for cluster overview and alert summaries.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::{
    k8s::dtos::AlertSeverity,
    state::{
        ClusterSnapshot,
        alerts::{compute_alerts, compute_dashboard_stats},
    },
};

/// Renders the dashboard view.
///
/// The dashboard includes cluster metadata, key resource counts, readiness indicators,
/// and the top alert list with severity-based color coding.
pub fn render_dashboard(frame: &mut Frame, area: Rect, snapshot: &ClusterSnapshot) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(7),
            Constraint::Min(7),
        ])
        .split(area);

    let stats = compute_dashboard_stats(snapshot);
    let alerts = compute_alerts(snapshot);

    let cluster_info = snapshot.cluster_info.as_ref();
    let metadata_lines = vec![
        Line::from(vec![
            Span::styled("Cluster: ", Style::default().fg(Color::Cyan)),
            Span::raw(
                cluster_info
                    .and_then(|info| info.context.as_deref())
                    .unwrap_or("unknown"),
            ),
        ]),
        Line::from(vec![
            Span::styled("API Server: ", Style::default().fg(Color::Cyan)),
            Span::raw(
                cluster_info
                    .map(|info| info.server.as_str())
                    .unwrap_or("unavailable"),
            ),
        ]),
        Line::from(vec![
            Span::styled("K8s Version: ", Style::default().fg(Color::Cyan)),
            Span::raw(
                cluster_info
                    .and_then(|info| info.git_version.as_deref())
                    .unwrap_or("unknown"),
            ),
        ]),
    ];

    let metadata_widget = Paragraph::new(metadata_lines)
        .block(Block::default().title("Cluster Info").borders(Borders::ALL));
    frame.render_widget(metadata_widget, layout[0]);

    let stats_lines = vec![
        Line::from(format!(
            "Nodes: {}/{} ready",
            stats.ready_nodes, stats.total_nodes
        )),
        Line::from(format!(
            "Pods: {}/{} running (failed: {})",
            stats.running_pods, stats.total_pods, stats.failed_pods
        )),
        Line::from(format!("Services: {}", stats.services_count)),
        Line::from(format!("Namespaces: {}", stats.namespaces_count)),
        Line::from(vec![
            Span::raw("Ready Nodes: "),
            Span::styled(
                format!("{}%", stats.ready_nodes_percent),
                Style::default().fg(percent_color(stats.ready_nodes_percent)),
            ),
        ]),
        Line::from(vec![
            Span::raw("Running Pods: "),
            Span::styled(
                format!("{}%", stats.running_pods_percent),
                Style::default().fg(percent_color(stats.running_pods_percent)),
            ),
        ]),
    ];

    let stats_widget = Paragraph::new(stats_lines).block(
        Block::default()
            .title("Workload Stats")
            .borders(Borders::ALL),
    );
    frame.render_widget(stats_widget, layout[1]);

    let alert_lines = alerts
        .iter()
        .enumerate()
        .map(|(idx, alert)| {
            Line::from(vec![
                Span::styled(
                    format!("{}. {} — ", idx + 1, alert.title),
                    Style::default().fg(severity_color(alert.severity)),
                ),
                Span::raw(alert.message.clone()),
            ])
        })
        .collect::<Vec<_>>();

    let alerts_widget = Paragraph::new(alert_lines)
        .block(Block::default().title("Top Alerts").borders(Borders::ALL));
    frame.render_widget(alerts_widget, layout[2]);
}

fn percent_color(percent: u8) -> Color {
    if percent >= 90 {
        Color::Green
    } else if percent >= 70 {
        Color::Yellow
    } else {
        Color::Red
    }
}

fn severity_color(severity: AlertSeverity) -> Color {
    match severity {
        AlertSeverity::Error => Color::Red,
        AlertSeverity::Warning => Color::Yellow,
        AlertSeverity::Info => Color::Green,
    }
}
