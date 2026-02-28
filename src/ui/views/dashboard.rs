//! Dashboard renderer — rich overview with health, saturation, and alerts.

use std::collections::BTreeMap;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, LineGauge, Paragraph, Sparkline, Wrap},
};

use crate::{
    k8s::dtos::{AlertItem, AlertSeverity},
    state::{
        ClusterSnapshot,
        alerts::{
            DashboardHealthState, DashboardInsights, DashboardStats, compute_alerts,
            compute_dashboard_insights, compute_dashboard_stats, compute_workload_ready_percent,
        },
    },
    ui::{components::default_theme, theme::Theme},
};

// ── metric parsing helpers ────────────────────────────────────────────────────

fn gauge_severity_style(theme: &Theme, percent: u8) -> Style {
    if percent >= 95 {
        theme.badge_success_style()
    } else if percent >= 75 {
        theme.badge_warning_style()
    } else {
        theme.badge_error_style()
    }
}

fn truncate_label(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }
    let kept: String = s.chars().take(max_chars - 3).collect();
    format!("{kept}...")
}

// ── top-level render ──────────────────────────────────────────────────────────

/// Renders the dashboard view.
pub fn render_dashboard(frame: &mut Frame, area: Rect, snapshot: &ClusterSnapshot) {
    let theme = default_theme();
    let stats = compute_dashboard_stats(snapshot);
    let alerts = compute_alerts(snapshot);
    let insights = compute_dashboard_insights(snapshot);
    let workload_pct = compute_workload_ready_percent(snapshot);

    // Layout:
    //  row 0 (7)  : cluster info | health summary
    //  row 1 (5)  : node-ready | pod-running | workload-ready gauges
    //  row 2 (9)  : node utilization summary | hottest nodes
    //  row 3 (7)  : resource counts | pod status distribution
    //  row 4 (min): alerts
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(5),
            Constraint::Length(9),
            Constraint::Length(7),
            Constraint::Min(6),
        ])
        .split(area);

    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);

    render_cluster_info(frame, top_cols[0], snapshot, &theme);
    render_cluster_health_summary(frame, top_cols[1], &stats, &insights, &theme);
    render_health_gauges(frame, rows[1], &stats, workload_pct, &theme);

    let node_rows = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(rows[2]);
    render_node_utilization_summary(frame, node_rows[0], &insights, &theme);
    render_hot_nodes(frame, node_rows[1], &insights, &theme);

    let summary_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(rows[3]);

    render_resource_counts(frame, summary_cols[0], &stats, &theme);
    render_pod_sparkline(frame, summary_cols[1], snapshot, &theme);
    render_alerts(frame, rows[4], &alerts, &theme);
}

// ── cluster info ──────────────────────────────────────────────────────────────

fn render_cluster_info(frame: &mut Frame, area: Rect, snapshot: &ClusterSnapshot, theme: &Theme) {
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

    let last_updated = snapshot
        .last_updated
        .map(|t| t.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "—".to_string());

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
            Span::styled("  updated ", theme.inactive_style()),
            Span::styled(last_updated, Style::default().fg(theme.fg_dim)),
        ]),
    ];

    let block = Block::default()
        .title(Span::styled(" ⎈ Cluster ", theme.title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_active_style())
        .style(Style::default().bg(theme.bg));

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_cluster_health_summary(
    frame: &mut Frame,
    area: Rect,
    stats: &DashboardStats,
    insights: &DashboardInsights,
    theme: &Theme,
) {
    let (health_label, health_style) = match insights.health_state {
        DashboardHealthState::Healthy => ("HEALTHY", theme.badge_success_style()),
        DashboardHealthState::Warning => ("WARNING", theme.badge_warning_style()),
        DashboardHealthState::Critical => ("CRITICAL", theme.badge_error_style()),
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("  Overall   ", theme.inactive_style()),
            Span::styled(health_label, health_style),
        ]),
        Line::from(vec![
            Span::styled("  Nodes     ", theme.inactive_style()),
            Span::styled(
                format!(
                    "{}/{} ready  {} not-ready  {} pressure",
                    stats.ready_nodes,
                    stats.total_nodes,
                    insights.not_ready_nodes,
                    insights.pressure_nodes
                ),
                Style::default().fg(theme.fg),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Pods      ", theme.inactive_style()),
            Span::styled(
                format!(
                    "{} running  {} pending  {} failed",
                    stats.running_pods, insights.pending_pods, insights.failed_pods
                ),
                Style::default().fg(theme.fg),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Metrics   ", theme.inactive_style()),
            Span::styled(
                format!(
                    "{} reported  {} usable",
                    insights.metrics_reported_nodes, insights.utilization_nodes
                ),
                Style::default().fg(theme.accent2),
            ),
        ]),
    ];

    let border_style = match insights.health_state {
        DashboardHealthState::Critical => theme.badge_error_style(),
        DashboardHealthState::Warning => theme.badge_warning_style(),
        DashboardHealthState::Healthy => theme.border_style(),
    };

    let block = Block::default()
        .title(Span::styled(" 󰓦 Cluster Health ", theme.title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .style(Style::default().bg(theme.bg));

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

// ── resource counts ───────────────────────────────────────────────────────────

fn render_resource_counts(frame: &mut Frame, area: Rect, stats: &DashboardStats, theme: &Theme) {
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
            Span::styled(format!("{} failed", stats.failed_pods), pod_style),
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

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

// ── health gauges ─────────────────────────────────────────────────────────────

fn render_health_gauges(
    frame: &mut Frame,
    area: Rect,
    stats: &DashboardStats,
    workload_pct: u8,
    theme: &Theme,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);

    render_percent_gauge(
        frame,
        cols[0],
        "Nodes Ready",
        stats.ready_nodes_percent,
        theme,
    );
    render_percent_gauge(
        frame,
        cols[1],
        "Pods Running",
        stats.running_pods_percent,
        theme,
    );
    render_percent_gauge(frame, cols[2], "Workload Ready", workload_pct, theme);
}

fn render_percent_gauge(frame: &mut Frame, area: Rect, title: &str, percent: u8, theme: &Theme) {
    let style = gauge_severity_style(theme, percent);
    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(Span::styled(format!(" {title}  {percent}% "), style))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(theme.border_style())
                .style(Style::default().bg(theme.bg)),
        )
        .gauge_style(style)
        .percent(percent as u16)
        .use_unicode(true);

    frame.render_widget(gauge, area);
}

fn render_node_utilization_summary(
    frame: &mut Frame,
    area: Rect,
    insights: &DashboardInsights,
    theme: &Theme,
) {
    let block = Block::default()
        .title(Span::styled(" 󰅢 Node Saturation ", theme.title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    render_line_gauge(
        frame,
        rows[0],
        "Avg CPU Util",
        i32::from(insights.avg_cpu_pct),
        100,
        theme,
    );
    render_line_gauge(
        frame,
        rows[1],
        "Avg Mem Util",
        i32::from(insights.avg_mem_pct),
        100,
        theme,
    );

    let saturation_line = Line::from(vec![
        Span::styled("  >=80% CPU ", theme.inactive_style()),
        Span::styled(
            insights.high_cpu_nodes.to_string(),
            Style::default().fg(theme.warning),
        ),
        Span::styled("   >=80% Mem ", theme.inactive_style()),
        Span::styled(
            insights.high_mem_nodes.to_string(),
            Style::default().fg(theme.warning),
        ),
    ]);
    frame.render_widget(Paragraph::new(saturation_line), rows[2]);

    let coverage_line = Line::from(vec![
        Span::styled("  Coverage ", theme.inactive_style()),
        Span::styled(
            format!(
                "{} reported / {} usable",
                insights.metrics_reported_nodes, insights.utilization_nodes
            ),
            Style::default().fg(theme.accent2),
        ),
    ]);
    frame.render_widget(Paragraph::new(coverage_line), rows[3]);
}

fn render_line_gauge(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    ready: i32,
    total: i32,
    theme: &Theme,
) {
    let ratio = if total > 0 {
        ready as f64 / total as f64
    } else {
        1.0
    };
    let pct = (ratio * 100.0) as u8;
    let color = if pct >= 100 {
        theme.success
    } else if pct >= 70 {
        theme.warning
    } else {
        theme.error
    };

    let gauge = LineGauge::default()
        .label(Line::from(vec![
            Span::styled(format!("  {label} "), theme.inactive_style()),
            Span::styled(
                format!("{ready}/{total}  {pct}%"),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]))
        .ratio(ratio.clamp(0.0, 1.0))
        .filled_style(Style::default().fg(color).bg(theme.bg_surface))
        .filled_symbol("━")
        .unfilled_symbol("─");

    frame.render_widget(gauge, area);
}

fn render_hot_nodes(frame: &mut Frame, area: Rect, insights: &DashboardInsights, theme: &Theme) {
    let mut lines = Vec::new();
    if insights.utilization_nodes == 0 {
        lines.push(Line::from(vec![
            Span::styled("  ℹ  ", Style::default().fg(theme.info)),
            Span::styled(
                "Node utilization unavailable (missing metrics-server or allocatable data)",
                Style::default().fg(theme.fg_dim),
            ),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "  CPU hottest nodes",
            Style::default().fg(theme.accent),
        )));
        for (rank, node) in insights.hot_cpu_nodes.iter().enumerate() {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("   {}. ", rank + 1),
                    Style::default().fg(theme.inactive_style().fg.unwrap_or(theme.fg_dim)),
                ),
                Span::styled(
                    truncate_label(&node.name, 20),
                    Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "  {}% ({}/{})",
                        node.cpu_pct, node.cpu_used_m, node.cpu_alloc_m
                    ),
                    Style::default().fg(theme.fg_dim),
                ),
            ]));
        }

        lines.push(Line::from(Span::styled(
            "  Memory hottest nodes",
            Style::default().fg(theme.accent2),
        )));
        for (rank, node) in insights.hot_mem_nodes.iter().enumerate() {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("   {}. ", rank + 1),
                    Style::default().fg(theme.inactive_style().fg.unwrap_or(theme.fg_dim)),
                ),
                Span::styled(
                    truncate_label(&node.name, 20),
                    Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "  {}% ({}Mi/{}Mi)",
                        node.mem_pct, node.mem_used_mib, node.mem_alloc_mib
                    ),
                    Style::default().fg(theme.fg_dim),
                ),
            ]));
        }
    }

    let block = Block::default()
        .title(Span::styled(" 󰅬 Top Node Pressure ", theme.title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

// ── pod status sparkline ──────────────────────────────────────────────────────

fn render_pod_sparkline(frame: &mut Frame, area: Rect, snapshot: &ClusterSnapshot, theme: &Theme) {
    // Build a per-namespace pod count sparkline (sorted by namespace name)
    let mut ns_counts: BTreeMap<&str, u64> = BTreeMap::new();
    for pod in &snapshot.pods {
        *ns_counts.entry(pod.namespace.as_str()).or_default() += 1;
    }
    let spark_data: Vec<u64> = ns_counts.values().copied().collect();

    // Pod status breakdown as text lines (single pass to avoid repeated scans)
    let (mut running, mut pending, mut failed, mut succeeded) = (0usize, 0usize, 0usize, 0usize);
    for pod in &snapshot.pods {
        if pod.status.eq_ignore_ascii_case("running") {
            running += 1;
        } else if pod.status.eq_ignore_ascii_case("pending") {
            pending += 1;
        } else if pod.status.eq_ignore_ascii_case("failed") {
            failed += 1;
        } else if pod.status.eq_ignore_ascii_case("succeeded") {
            succeeded += 1;
        }
    }
    let other = snapshot
        .pods
        .len()
        .saturating_sub(running + pending + failed + succeeded);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(3)])
        .split(area);

    // Sparkline — pods per namespace
    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .title(Span::styled(" Pods / Namespace ", theme.title_style()))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(theme.border_style())
                .style(Style::default().bg(theme.bg)),
        )
        .data(&spark_data)
        .style(Style::default().fg(theme.accent));

    frame.render_widget(sparkline, rows[0]);

    // Status breakdown
    let status_lines = vec![
        Line::from(vec![
            Span::styled("  ● ", Style::default().fg(theme.success)),
            Span::styled(
                format!("{running} Running  "),
                Style::default().fg(theme.fg),
            ),
            Span::styled("● ", Style::default().fg(theme.warning)),
            Span::styled(
                format!("{pending} Pending  "),
                Style::default().fg(theme.fg),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ● ", Style::default().fg(theme.error)),
            Span::styled(format!("{failed} Failed   "), Style::default().fg(theme.fg)),
            Span::styled("● ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{succeeded} Succeeded  {other} Other"),
                Style::default().fg(theme.fg_dim),
            ),
        ]),
    ];

    let status_block = Block::default()
        .title(Span::styled(" Pod Status ", theme.title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));

    frame.render_widget(
        Paragraph::new(status_lines)
            .block(status_block)
            .wrap(Wrap { trim: false }),
        rows[1],
    );
}

// ── alerts ────────────────────────────────────────────────────────────────────

fn render_alerts(frame: &mut Frame, area: Rect, alerts: &[AlertItem], theme: &Theme) {
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
                        Style::default()
                            .fg(style.fg.unwrap_or(theme.fg))
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(alert.message.clone(), Style::default().fg(theme.fg_dim)),
                ])
            })
            .collect()
    };

    let title_style = if alerts.iter().any(|a| a.severity == AlertSeverity::Error) {
        theme.badge_error_style()
    } else if alerts.iter().any(|a| a.severity == AlertSeverity::Warning) {
        theme.badge_warning_style()
    } else {
        theme.title_style()
    };

    let block = Block::default()
        .title(Span::styled(
            format!(" ⚡ Alerts ({}) ", alerts.len()),
            title_style,
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

    frame.render_widget(
        Paragraph::new(alert_lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}
