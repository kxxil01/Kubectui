//! Dashboard renderer for cluster overview and alert summaries.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, Paragraph, Wrap},
};

use crate::{
    k8s::dtos::{AlertItem, AlertSeverity},
    state::{
        ClusterSnapshot,
        alerts::{compute_alerts, compute_dashboard_stats},
    },
    ui::{components::default_theme, theme::Theme},
};

/// Renders the dashboard view with a rich 2-column layout.
pub fn render_dashboard(frame: &mut Frame, area: Rect, snapshot: &ClusterSnapshot) {
    let theme = default_theme();
    let stats = compute_dashboard_stats(snapshot);
    let alerts = compute_alerts(snapshot);

    // Top row: cluster info + resource counts side by side
    // Bottom row: health gauges + alerts
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(5),
            Constraint::Min(6),
        ])
        .split(area);

    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);

    render_cluster_info(frame, top_cols[0], snapshot, &theme);
    render_resource_counts(frame, top_cols[1], &stats, &theme);
    render_health_gauges(frame, rows[1], &stats, &theme);
    render_alerts(frame, rows[2], &alerts, &theme);
}

fn render_cluster_info(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    theme: &Theme,
) {
    let cluster_info = snapshot.cluster_info.as_ref();

    let context = cluster_info
        .and_then(|i| i.context.as_deref())
        .unwrap_or("unknown");
    let server = cluster_info
        .map(|i| i.server.as_str())
        .unwrap_or("unavailable");
    let version = cluster_info
        .and_then(|i| i.git_version.as_deref())
        .unwrap_or("unknown");
    let phase_label = format!("{}", snapshot.phase);

    let phase_style = match snapshot.phase {
        crate::state::DataPhase::Ready => theme.badge_success_style(),
        crate::state::DataPhase::Loading => theme.badge_warning_style(),
        crate::state::DataPhase::Error => theme.badge_error_style(),
        crate::state::DataPhase::Idle => theme.inactive_style(),
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("  Context   ", theme.inactive_style()),
            Span::styled(context, theme.title_style()),
        ]),
        Line::from(vec![
            Span::styled("  Server    ", theme.inactive_style()),
            Span::styled(server, Style::default().fg(theme.fg)),
        ]),
        Line::from(vec![
            Span::styled("  Version   ", theme.inactive_style()),
            Span::styled(version, Style::default().fg(theme.accent2)),
        ]),
        Line::from(vec![
            Span::styled("  Status    ", theme.inactive_style()),
            Span::styled(phase_label, phase_style),
        ]),
    ];

    let block = Block::default()
        .title(Span::styled(" ⎈ Cluster ", theme.title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_active_style())
        .style(Style::default().bg(theme.bg));

    let widget = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    frame.render_widget(widget, area);
}

fn render_resource_counts(
    frame: &mut Frame,
    area: Rect,
    stats: &crate::state::alerts::DashboardStats,
    theme: &Theme,
) {
    let node_style = if stats.ready_nodes == stats.total_nodes {
        theme.badge_success_style()
    } else {
        theme.badge_warning_style()
    };

    let pod_style = if stats.failed_pods > 0 {
        theme.badge_error_style()
    } else {
        theme.badge_success_style()
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("  Nodes      ", theme.inactive_style()),
            Span::styled(
                format!("{}/{} ready", stats.ready_nodes, stats.total_nodes),
                node_style,
            ),
        ]),
        Line::from(vec![
            Span::styled("  Pods       ", theme.inactive_style()),
            Span::styled(
                format!("{} running", stats.running_pods),
                theme.badge_success_style(),
            ),
            Span::styled("  ", theme.inactive_style()),
            Span::styled(
                format!("{} failed", stats.failed_pods),
                pod_style,
            ),
        ]),
        Line::from(vec![
            Span::styled("  Services   ", theme.inactive_style()),
            Span::styled(
                stats.services_count.to_string(),
                Style::default().fg(theme.info),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Namespaces ", theme.inactive_style()),
            Span::styled(
                stats.namespaces_count.to_string(),
                Style::default().fg(theme.accent2),
            ),
        ]),
    ];

    let block = Block::default()
        .title(Span::styled(" 📊 Resources ", theme.title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));

    let widget = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    frame.render_widget(widget, area);
}

fn render_health_gauges(
    frame: &mut Frame,
    area: Rect,
    stats: &crate::state::alerts::DashboardStats,
    theme: &Theme,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let node_pct = stats.ready_nodes_percent;
    let pod_pct = stats.running_pods_percent;

    let node_gauge = Gauge::default()
        .block(
            Block::default()
                .title(Span::styled(
                    format!(" Nodes Ready  {node_pct}% "),
                    theme.gauge_style(100 - node_pct),
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(theme.border_style()),
        )
        .gauge_style(theme.gauge_style(100 - node_pct))
        .percent(node_pct as u16)
        .use_unicode(true);

    let pod_gauge = Gauge::default()
        .block(
            Block::default()
                .title(Span::styled(
                    format!(" Pods Running  {pod_pct}% "),
                    theme.gauge_style(100 - pod_pct),
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(theme.border_style()),
        )
        .gauge_style(theme.gauge_style(100 - pod_pct))
        .percent(pod_pct as u16)
        .use_unicode(true);

    frame.render_widget(node_gauge, cols[0]);
    frame.render_widget(pod_gauge, cols[1]);
}

fn render_alerts(
    frame: &mut Frame,
    area: Rect,
    alerts: &[AlertItem],
    theme: &Theme,
) {
    let alert_lines: Vec<Line> = if alerts.is_empty() {
        vec![Line::from(vec![
            Span::styled("✓ ", theme.badge_success_style()),
            Span::styled(
                "All systems healthy — no active alerts",
                Style::default().fg(theme.fg_dim),
            ),
        ])]
    } else {
        alerts
            .iter()
            .map(|alert| {
                let (icon, style) = match alert.severity {
                    AlertSeverity::Error => ("✗ ", theme.badge_error_style()),
                    AlertSeverity::Warning => ("⚠ ", theme.badge_warning_style()),
                    AlertSeverity::Info => ("ℹ ", Style::default().fg(theme.info)),
                };
                Line::from(vec![
                    Span::styled(icon, style),
                    Span::styled(
                        format!("{}: ", alert.title),
                        Style::default().fg(style.fg.unwrap_or(theme.fg)).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(alert.message.clone(), Style::default().fg(theme.fg_dim)),
                ])
            })
            .collect()
    };

    let block = Block::default()
        .title(Span::styled(
            format!(" ⚡ Alerts ({}) ", alerts.len()),
            if alerts.iter().any(|a| a.severity == AlertSeverity::Error) {
                theme.badge_error_style()
            } else if alerts.iter().any(|a| a.severity == AlertSeverity::Warning) {
                theme.badge_warning_style()
            } else {
                theme.title_style()
            },
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(
            if alerts.iter().any(|a| a.severity == AlertSeverity::Error) {
                theme.badge_error_style()
            } else {
                theme.border_style()
            },
        )
        .style(Style::default().bg(theme.bg));

    let widget = Paragraph::new(alert_lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, area);
}
