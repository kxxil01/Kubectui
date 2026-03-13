//! Dashboard renderer — rich overview with health, saturation, and alerts.

use std::borrow::Cow;
use std::sync::{LazyLock, Mutex};

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, LineGauge, Paragraph, Wrap},
};

use crate::{
    k8s::dtos::{AlertItem, AlertSeverity},
    state::{
        ClusterSnapshot,
        alerts::{
            ClusterResourceSummary, DashboardHealthState, DashboardInsights, DashboardStats,
            NamespaceUtilizationSummary, PodConsumerSummary, compute_alerts,
            compute_cluster_resource_summary, compute_dashboard_insights, compute_dashboard_stats,
            compute_namespace_utilization, compute_top_pod_consumers,
            compute_workload_ready_percent,
        },
    },
    ui::{components::default_theme, theme::Theme},
};

// ── dashboard computation cache ──────────────────────────────────────────────

struct DashboardCache {
    version: u64,
    stats: DashboardStats,
    alerts: Vec<AlertItem>,
    insights: DashboardInsights,
    workload_pct: u8,
    status_counts: (usize, usize, usize, usize, usize),
    ns_utilization: Vec<NamespaceUtilizationSummary>,
    cluster_resources: ClusterResourceSummary,
    top_cpu_pods: Vec<PodConsumerSummary>,
    top_mem_pods: Vec<PodConsumerSummary>,
}

static DASHBOARD_CACHE: LazyLock<Mutex<Option<DashboardCache>>> =
    LazyLock::new(|| Mutex::new(None));

type StatusCounts = (usize, usize, usize, usize, usize);
type DashboardData = (
    DashboardStats,
    Vec<AlertItem>,
    DashboardInsights,
    u8,
    StatusCounts,
    Vec<NamespaceUtilizationSummary>,
    ClusterResourceSummary,
    Vec<PodConsumerSummary>,
    Vec<PodConsumerSummary>,
);

fn cached_dashboard(snapshot: &ClusterSnapshot) -> DashboardData {
    let mut guard = DASHBOARD_CACHE.lock().unwrap();
    if let Some(ref c) = *guard
        && c.version == snapshot.snapshot_version
    {
        return (
            c.stats,
            c.alerts.clone(),
            c.insights.clone(),
            c.workload_pct,
            c.status_counts,
            c.ns_utilization.clone(),
            c.cluster_resources,
            c.top_cpu_pods.clone(),
            c.top_mem_pods.clone(),
        );
    }

    let stats = compute_dashboard_stats(snapshot);
    let alerts = compute_alerts(snapshot);
    let insights = compute_dashboard_insights(snapshot);
    let workload_pct = compute_workload_ready_percent(snapshot);

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
    let status_counts = (running, pending, failed, succeeded, other);

    let ns_utilization = compute_namespace_utilization(snapshot);
    let cluster_resources = compute_cluster_resource_summary(snapshot);
    let (top_cpu_pods, top_mem_pods) = compute_top_pod_consumers(snapshot);

    *guard = Some(DashboardCache {
        version: snapshot.snapshot_version,
        stats,
        alerts: alerts.clone(),
        insights: insights.clone(),
        workload_pct,
        status_counts,
        ns_utilization: ns_utilization.clone(),
        cluster_resources,
        top_cpu_pods: top_cpu_pods.clone(),
        top_mem_pods: top_mem_pods.clone(),
    });

    (
        stats,
        alerts,
        insights,
        workload_pct,
        status_counts,
        ns_utilization,
        cluster_resources,
        top_cpu_pods,
        top_mem_pods,
    )
}

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

fn truncate_label(s: &str, max_chars: usize) -> Cow<'_, str> {
    if s.chars().count() <= max_chars {
        Cow::Borrowed(s)
    } else if max_chars <= 3 {
        Cow::Owned(".".repeat(max_chars))
    } else {
        let kept: String = s.chars().take(max_chars - 3).collect();
        Cow::Owned(format!("{kept}..."))
    }
}

// ── top-level render ──────────────────────────────────────────────────────────

/// Renders the dashboard view.
pub fn render_dashboard(frame: &mut Frame, area: Rect, snapshot: &ClusterSnapshot) {
    let theme = default_theme();
    let (
        stats,
        alerts,
        insights,
        workload_pct,
        _status_counts,
        ns_utilization,
        cluster_resources,
        top_cpu_pods,
        top_mem_pods,
    ) = cached_dashboard(snapshot);

    // Layout:
    //  row 0 (8)  : cluster info | health summary
    //  row 1 (5)  : 5 gauges (nodes ready, pods running, workload ready, cluster CPU, cluster mem)
    //  row 2 (9)  : node utilization summary | top node pressure
    //  row 3 (7)  : resource counts | overcommit & governance
    //  row 4 (9)  : namespace utilization (enhanced, top 5)
    //  row 5 (9)  : top CPU pods | top memory pods
    //  row 6 (min): alerts
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),
            Constraint::Length(5),
            Constraint::Length(9),
            Constraint::Length(7),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Min(6),
        ])
        .split(area);

    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);

    render_cluster_info(frame, top_cols[0], snapshot, &theme);
    render_cluster_health_summary(
        frame,
        top_cols[1],
        &stats,
        &insights,
        snapshot.issue_count,
        &theme,
    );
    render_health_gauges(
        frame,
        rows[1],
        &stats,
        workload_pct,
        &cluster_resources,
        &theme,
    );

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
    render_overcommit_governance(frame, summary_cols[1], &cluster_resources, &theme);
    render_namespace_utilization(frame, rows[4], &ns_utilization, &theme);
    render_top_pod_consumers(frame, rows[5], &top_cpu_pods, &top_mem_pods, &theme);
    render_alerts(frame, rows[6], &alerts, &theme);
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
    issue_count: usize,
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
        Line::from(vec![
            Span::styled("  Issues    ", theme.inactive_style()),
            Span::styled(
                format!("{issue_count}"),
                if issue_count > 0 {
                    theme.badge_warning_style()
                } else {
                    theme.badge_success_style()
                },
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
    cluster_res: &ClusterResourceSummary,
    theme: &Theme,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
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
    render_utilization_gauge(
        frame,
        cols[3],
        "Cluster CPU",
        cluster_res.cluster_cpu_pct,
        theme,
    );
    render_utilization_gauge(
        frame,
        cols[4],
        "Cluster Mem",
        cluster_res.cluster_mem_pct,
        theme,
    );
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

/// Gauge with inverted severity: high utilization = bad (red).
fn render_utilization_gauge(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    percent: u8,
    theme: &Theme,
) {
    let style = crate::ui::utilization_style(u64::from(percent), theme);
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

// ── overcommit & governance ───────────────────────────────────────────────────

fn commitment_style(pct: u16, theme: &Theme) -> Style {
    if pct >= 120 {
        theme.badge_error_style()
    } else if pct >= 100 {
        theme.badge_warning_style()
    } else {
        theme.badge_success_style()
    }
}

fn render_overcommit_governance(
    frame: &mut Frame,
    area: Rect,
    res: &ClusterResourceSummary,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(
            " ⚖ Commitment & Governance ",
            theme.title_style(),
        ))
        .border_style(Style::default().fg(theme.border));

    if res.total_running_pods == 0 && res.total_cpu_allocatable_m == 0 {
        let msg = Paragraph::new(Span::styled("  No data available", theme.inactive_style()))
            .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let dim = Style::default().fg(theme.fg_dim);
    let missing_style = |count: usize| -> Style {
        if count > 0 {
            theme.badge_warning_style()
        } else {
            theme.badge_success_style()
        }
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("  CPU Req  ", dim),
            Span::styled(
                format!("{:>4}%", res.cpu_request_commitment_pct),
                commitment_style(res.cpu_request_commitment_pct, theme),
            ),
            Span::styled("    CPU Lim  ", dim),
            Span::styled(
                format!("{:>4}%", res.cpu_limit_commitment_pct),
                commitment_style(res.cpu_limit_commitment_pct, theme),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Mem Req  ", dim),
            Span::styled(
                format!("{:>4}%", res.mem_request_commitment_pct),
                commitment_style(res.mem_request_commitment_pct, theme),
            ),
            Span::styled("    Mem Lim  ", dim),
            Span::styled(
                format!("{:>4}%", res.mem_limit_commitment_pct),
                commitment_style(res.mem_limit_commitment_pct, theme),
            ),
        ]),
        Line::from(Span::styled(
            "  ─────────────────────────────────────────",
            dim,
        )),
        Line::from(vec![
            Span::styled("  No CPU Req  ", dim),
            Span::styled(
                format!("{}", res.pods_missing_cpu_request),
                missing_style(res.pods_missing_cpu_request),
            ),
            Span::styled("    No Mem Req  ", dim),
            Span::styled(
                format!("{}", res.pods_missing_mem_request),
                missing_style(res.pods_missing_mem_request),
            ),
        ]),
        Line::from(vec![
            Span::styled("  No Limits   ", dim),
            Span::styled(
                format!("{}", res.pods_missing_any_limit),
                missing_style(res.pods_missing_any_limit),
            ),
            Span::styled(
                format!("    of {} running pods", res.total_running_pods),
                dim,
            ),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}

// ── top pod consumers ────────────────────────────────────────────────────────

fn render_top_pod_consumers(
    frame: &mut Frame,
    area: Rect,
    top_cpu: &[PodConsumerSummary],
    top_mem: &[PodConsumerSummary],
    theme: &Theme,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_consumer_panel(frame, cols[0], " 󰅬 Top CPU Pods ", top_cpu, true, theme);
    render_consumer_panel(frame, cols[1], " 󰍛 Top Memory Pods ", top_mem, false, theme);
}

fn render_consumer_panel(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    consumers: &[PodConsumerSummary],
    is_cpu: bool,
    theme: &Theme,
) {
    use crate::state::alerts::{format_mib, format_millicores};

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(title, theme.title_style()))
        .border_style(Style::default().fg(theme.border));

    if consumers.is_empty() {
        let msg = Paragraph::new(Span::styled(
            "  No metrics available",
            theme.inactive_style(),
        ))
        .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let dim = Style::default().fg(theme.fg_dim);
    let accent = Style::default().fg(theme.accent);

    let mut lines = Vec::with_capacity(consumers.len());
    for (i, p) in consumers.iter().enumerate() {
        let name = truncate_label(&p.name, 22);
        let ns = truncate_label(&p.namespace, 10);
        let val = if is_cpu {
            format_millicores(p.cpu_usage_m)
        } else {
            format_mib(p.mem_usage_mib)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {}. ", i + 1), dim),
            Span::styled(format!("{:<22} ", name), Style::default().fg(theme.fg)),
            Span::styled(format!("{:<10} ", ns), dim),
            Span::styled(format!("{:>8}", val), accent),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

// ── namespace utilization ─────────────────────────────────────────────────────

fn render_namespace_utilization(
    frame: &mut Frame,
    area: Rect,
    ns_util: &[NamespaceUtilizationSummary],
    theme: &Theme,
) {
    use crate::state::alerts::{format_mib, format_millicores};

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(
            " Namespace Utilization (Top 5) ",
            theme.title_style(),
        ))
        .border_style(Style::default().fg(theme.border));

    if ns_util.is_empty()
        || ns_util
            .iter()
            .all(|n| n.cpu_usage_m == 0 && n.mem_usage_mib == 0)
    {
        let msg = Paragraph::new(Span::styled(
            "  No metrics available (metrics-server not detected)",
            theme.inactive_style(),
        ))
        .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let dim = Style::default().fg(theme.fg_dim);
    let header = Line::from(vec![Span::styled(
        format!(
            "  {:<18} {:>5} {:>8} {:>8} {:>6} {:>8} {:>8} {:>6}",
            "Namespace", "Pods", "CPU Use", "CPU Req", "%CPU/R", "Mem Use", "Mem Req", "%MEM/R"
        ),
        theme.header_style(),
    )]);

    let mut lines = Vec::with_capacity(7);
    lines.push(header);

    for ns in ns_util.iter().take(5) {
        let cpu_used = format_millicores(ns.cpu_usage_m);
        let cpu_req = if ns.cpu_request_m > 0 {
            format_millicores(ns.cpu_request_m)
        } else {
            "-".to_string()
        };
        let mem_used = format_mib(ns.mem_usage_mib);
        let mem_req = if ns.mem_request_mib > 0 {
            format_mib(ns.mem_request_mib)
        } else {
            "-".to_string()
        };
        let ns_name = truncate_label(&ns.namespace, 18);

        let prefix = format!(
            "  {:<18} {:>5} {:>8} {:>8} ",
            ns_name, ns.pod_count, cpu_used, cpu_req
        );
        let cpu_pct_str = ns
            .cpu_req_utilization_pct
            .map(|p| format!("{:>5}%", p))
            .unwrap_or_else(|| "    -".to_string());
        let cpu_pct_style = ns
            .cpu_req_utilization_pct
            .map(|p| crate::ui::utilization_style(u64::from(p), theme))
            .unwrap_or(dim);
        let mid = format!(" {:>8} {:>8} ", mem_used, mem_req);
        let mem_pct_str = ns
            .mem_req_utilization_pct
            .map(|p| format!("{:>5}%", p))
            .unwrap_or_else(|| "    -".to_string());
        let mem_pct_style = ns
            .mem_req_utilization_pct
            .map(|p| crate::ui::utilization_style(u64::from(p), theme))
            .unwrap_or(dim);

        lines.push(Line::from(vec![
            Span::styled(prefix, dim),
            Span::styled(cpu_pct_str, cpu_pct_style),
            Span::styled(mid, dim),
            Span::styled(mem_pct_str, mem_pct_style),
        ]));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
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
