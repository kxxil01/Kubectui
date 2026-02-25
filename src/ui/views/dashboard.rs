//! Dashboard renderer — rich overview with metrics charts, gauges, and alerts.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{
        BarChart, Block, BorderType, Borders, Gauge, LineGauge, Paragraph, Sparkline, Wrap,
    },
};

use crate::{
    k8s::dtos::{AlertItem, AlertSeverity},
    state::{
        ClusterSnapshot,
        alerts::{compute_alerts, compute_dashboard_stats},
    },
    ui::{components::default_theme, theme::Theme},
};

// ── metric parsing helpers ────────────────────────────────────────────────────

/// Maximum number of nodes shown in the bar chart to keep bars readable.
const MAX_CHART_NODES: usize = 8;

/// Parse a Kubernetes CPU quantity string (e.g. "250m", "2") into millicores.
fn parse_millicores(s: &str) -> u64 {
    if let Some(m) = s.strip_suffix('m') {
        m.parse().unwrap_or(0)
    } else {
        s.parse::<u64>().unwrap_or(0) * 1000
    }
}

/// Parse a Kubernetes memory quantity string (e.g. "512Mi", "1Gi", "1073741824") into MiB.
fn parse_mib(s: &str) -> u64 {
    if let Some(v) = s.strip_suffix("Ki") {
        return v.parse::<u64>().unwrap_or(0) / 1024;
    }
    if let Some(v) = s.strip_suffix("Mi") {
        return v.parse().unwrap_or(0);
    }
    if let Some(v) = s.strip_suffix("Gi") {
        return v.parse::<u64>().unwrap_or(0) * 1024;
    }
    if let Some(v) = s.strip_suffix("Ti") {
        return v.parse::<u64>().unwrap_or(0) * 1024 * 1024;
    }
    // raw bytes
    s.parse::<u64>().unwrap_or(0) / (1024 * 1024)
}

// ── top-level render ──────────────────────────────────────────────────────────

/// Renders the dashboard view.
pub fn render_dashboard(frame: &mut Frame, area: Rect, snapshot: &ClusterSnapshot) {
    let theme = default_theme();
    let stats = compute_dashboard_stats(snapshot);
    let alerts = compute_alerts(snapshot);

    // Layout:
    //  row 0 (7)  : cluster info | resource counts
    //  row 1 (5)  : node-ready gauge | pod-running gauge
    //  row 2 (9)  : node CPU barchart | node memory barchart
    //  row 3 (7)  : workload health (deployments/statefulsets/daemonsets LineGauges)
    //  row 4 (min): pod status sparkline | alerts
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
    render_resource_counts(frame, top_cols[1], &stats, &theme);
    render_health_gauges(frame, rows[1], &stats, &theme);
    render_node_metrics(frame, rows[2], snapshot, &theme);
    render_workload_health(frame, rows[3], snapshot, &theme);

    let bottom_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(rows[4]);

    render_pod_sparkline(frame, bottom_cols[0], snapshot, &theme);
    render_alerts(frame, bottom_cols[1], &alerts, &theme);
}

// ── cluster info ──────────────────────────────────────────────────────────────

fn render_cluster_info(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    theme: &Theme,
) {
    let cluster_info = snapshot.cluster_info.as_ref();
    let context = cluster_info.and_then(|i| i.context.as_deref()).unwrap_or("unknown");
    let server = cluster_info.map(|i| i.server.as_str()).unwrap_or("unavailable");
    let version = cluster_info.and_then(|i| i.git_version.as_deref()).unwrap_or("unknown");
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

    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
}

// ── resource counts ───────────────────────────────────────────────────────────

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
            Span::styled(format!("{}/{} ready", stats.ready_nodes, stats.total_nodes), node_style),
        ]),
        Line::from(vec![
            Span::styled("  Pods       ", theme.inactive_style()),
            Span::styled(format!("{} running", stats.running_pods), theme.badge_success_style()),
            Span::styled("  ", theme.inactive_style()),
            Span::styled(format!("{} failed", stats.failed_pods), pod_style),
        ]),
        Line::from(vec![
            Span::styled("  Services   ", theme.inactive_style()),
            Span::styled(stats.services_count.to_string(), Style::default().fg(theme.info)),
        ]),
        Line::from(vec![
            Span::styled("  Namespaces ", theme.inactive_style()),
            Span::styled(stats.namespaces_count.to_string(), Style::default().fg(theme.accent2)),
        ]),
    ];

    let block = Block::default()
        .title(Span::styled(" 📊 Resources ", theme.title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));

    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
}

// ── health gauges ─────────────────────────────────────────────────────────────

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

// ── node metrics bar charts ───────────────────────────────────────────────────

fn render_node_metrics(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    theme: &Theme,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    if snapshot.node_metrics.is_empty() {
        // metrics-server not available — show a friendly placeholder
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  ℹ  ", Style::default().fg(theme.info)),
                Span::styled(
                    "Node metrics unavailable — install metrics-server to enable CPU/memory charts",
                    Style::default().fg(theme.fg_dim),
                ),
            ]),
        ])
        .block(
            Block::default()
                .title(Span::styled(" 󰻠 Node Metrics ", theme.title_style()))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(theme.border_style())
                .style(Style::default().bg(theme.bg)),
        )
        .wrap(Wrap { trim: false });
        frame.render_widget(msg, area);
        return;
    }

    // Build bar data — cap to MAX_CHART_NODES so bars stay readable
    let metrics: Vec<_> = snapshot.node_metrics.iter().take(MAX_CHART_NODES).collect();

    // CPU bar chart (millicores)
    let cpu_data: Vec<(String, u64)> = metrics
        .iter()
        .map(|m| {
            let short_name = m.name.split('-').last().unwrap_or(&m.name).to_string();
            (short_name, parse_millicores(&m.cpu))
        })
        .collect();
    let cpu_max = cpu_data.iter().map(|(_, v)| *v).max().unwrap_or(1).max(1);
    let cpu_bar_data: Vec<(&str, u64)> = cpu_data.iter().map(|(k, v)| (k.as_str(), *v)).collect();

    let cpu_chart = BarChart::default()
        .block(
            Block::default()
                .title(Span::styled(" 󰻠 CPU (millicores) ", theme.title_style()))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(theme.border_style())
                .style(Style::default().bg(theme.bg)),
        )
        .data(&cpu_bar_data)
        .bar_width(5)
        .bar_gap(1)
        .max(cpu_max)
        .bar_style(Style::default().fg(theme.accent))
        .value_style(
            Style::default()
                .fg(theme.bg)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )
        .label_style(Style::default().fg(theme.fg_dim));

    // Memory bar chart (MiB)
    let mem_data: Vec<(String, u64)> = metrics
        .iter()
        .map(|m| {
            let short_name = m.name.split('-').last().unwrap_or(&m.name).to_string();
            (short_name, parse_mib(&m.memory))
        })
        .collect();
    let mem_max = mem_data.iter().map(|(_, v)| *v).max().unwrap_or(1).max(1);
    let mem_bar_data: Vec<(&str, u64)> = mem_data.iter().map(|(k, v)| (k.as_str(), *v)).collect();

    let mem_chart = BarChart::default()
        .block(
            Block::default()
                .title(Span::styled(" 󰍛 Memory (MiB) ", theme.title_style()))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(theme.border_style())
                .style(Style::default().bg(theme.bg)),
        )
        .data(&mem_bar_data)
        .bar_width(5)
        .bar_gap(1)
        .max(mem_max)
        .bar_style(Style::default().fg(theme.accent2))
        .value_style(
            Style::default()
                .fg(theme.bg)
                .bg(theme.accent2)
                .add_modifier(Modifier::BOLD),
        )
        .label_style(Style::default().fg(theme.fg_dim));

    frame.render_widget(cpu_chart, cols[0]);
    frame.render_widget(mem_chart, cols[1]);
}

// ── workload health line gauges ───────────────────────────────────────────────

fn render_workload_health(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    theme: &Theme,
) {
    let block = Block::default()
        .title(Span::styled(" 󰑓 Workload Health ", theme.title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Compute ready ratios
    let (dep_ready, dep_total) = snapshot.deployments.iter().fold((0i32, 0i32), |(r, t), d| {
        (r + d.ready_replicas.min(d.desired_replicas), t + d.desired_replicas.max(1))
    });
    let (ss_ready, ss_total) = snapshot.statefulsets.iter().fold((0i32, 0i32), |(r, t), s| {
        (r + s.ready_replicas.min(s.desired_replicas), t + s.desired_replicas.max(1))
    });
    let (ds_ready, ds_total) = snapshot.daemonsets.iter().fold((0i32, 0i32), |(r, t), d| {
        (r + d.ready_count.min(d.desired_count), t + d.desired_count.max(1))
    });

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    render_line_gauge(frame, rows[0], "Deployments ", dep_ready, dep_total, theme);
    render_line_gauge(frame, rows[1], "StatefulSets", ss_ready, ss_total, theme);
    render_line_gauge(frame, rows[2], "DaemonSets  ", ds_ready, ds_total, theme);
}

fn render_line_gauge(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    ready: i32,
    total: i32,
    theme: &Theme,
) {
    let ratio = if total > 0 { ready as f64 / total as f64 } else { 1.0 };
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

// ── pod status sparkline ──────────────────────────────────────────────────────

fn render_pod_sparkline(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    theme: &Theme,
) {
    // Build a per-namespace pod count sparkline (sorted by namespace name)
    let mut ns_counts: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    for pod in &snapshot.pods {
        *ns_counts.entry(pod.namespace.as_str()).or_default() += 1;
    }
    let spark_data: Vec<u64> = ns_counts.values().copied().collect();

    // Pod status breakdown as text lines
    let running = snapshot.pods.iter().filter(|p| p.status.eq_ignore_ascii_case("running")).count();
    let pending = snapshot.pods.iter().filter(|p| p.status.eq_ignore_ascii_case("pending")).count();
    let failed = snapshot.pods.iter().filter(|p| p.status.eq_ignore_ascii_case("failed")).count();
    let succeeded = snapshot.pods.iter().filter(|p| p.status.eq_ignore_ascii_case("succeeded")).count();
    let other = snapshot.pods.len().saturating_sub(running + pending + failed + succeeded);

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
            Span::styled(format!("{running} Running  "), Style::default().fg(theme.fg)),
            Span::styled("● ", Style::default().fg(theme.warning)),
            Span::styled(format!("{pending} Pending  "), Style::default().fg(theme.fg)),
        ]),
        Line::from(vec![
            Span::styled("  ● ", Style::default().fg(theme.error)),
            Span::styled(format!("{failed} Failed   "), Style::default().fg(theme.fg)),
            Span::styled("● ", Style::default().fg(theme.muted)),
            Span::styled(format!("{succeeded} Succeeded  {other} Other"), Style::default().fg(theme.fg_dim)),
        ]),
    ];

    let status_block = Block::default()
        .title(Span::styled(" Pod Status ", theme.title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));

    frame.render_widget(
        Paragraph::new(status_lines).block(status_block).wrap(Wrap { trim: false }),
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
        .title(Span::styled(format!(" ⚡ Alerts ({}) ", alerts.len()), title_style))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if alerts.iter().any(|a| a.severity == AlertSeverity::Error) {
            theme.badge_error_style()
        } else {
            theme.border_style()
        })
        .style(Style::default().bg(theme.bg));

    frame.render_widget(
        Paragraph::new(alert_lines).block(block).wrap(Wrap { trim: false }),
        area,
    );
}
