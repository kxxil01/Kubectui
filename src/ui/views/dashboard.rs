//! Dashboard renderer — rich overview with health, saturation, and alerts.

use std::borrow::Cow;
use std::sync::{LazyLock, Mutex};

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Gauge, LineGauge, Paragraph, Row, Table, Wrap},
};

use crate::{
    k8s::dtos::{AlertItem, AlertSeverity},
    state::{
        ClusterSnapshot, RefreshScope,
        alerts::{
            ClusterResourceSummary, DashboardHealthState, DashboardInsights, DashboardStats,
            NamespaceUtilizationSummary, PodConsumerSummary, TOP_N, compute_alerts,
            compute_cluster_resource_summary, compute_dashboard_insights, compute_dashboard_stats,
            compute_namespace_utilization, compute_top_pod_consumers,
            compute_workload_ready_percent, format_mib, format_millicores,
        },
    },
    time::format_local,
    ui::{components::default_theme, theme::Theme, utilization_style, wrapped_line_count},
};

// ── dashboard computation cache ──────────────────────────────────────────────

#[derive(Clone)]
struct DashboardData {
    stats: DashboardStats,
    alerts: Vec<AlertItem>,
    insights: DashboardInsights,
    workload_pct: u8,
    ns_utilization: Vec<NamespaceUtilizationSummary>,
    cluster_resources: ClusterResourceSummary,
    top_cpu_pods: Vec<PodConsumerSummary>,
    top_mem_pods: Vec<PodConsumerSummary>,
}

struct DashboardCache {
    version: u64,
    data: DashboardData,
}

static DASHBOARD_CACHE: LazyLock<Mutex<Option<DashboardCache>>> =
    LazyLock::new(|| Mutex::new(None));

const NARROW_DASHBOARD_WIDTH: u16 = 84;
const COMPACT_GAUGE_WIDTH: u16 = 72;
const NARROW_NAMESPACE_UTIL_WIDTH: u16 = 88;

fn cached_dashboard(snapshot: &ClusterSnapshot) -> DashboardData {
    let mut guard = DASHBOARD_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref c) = *guard
        && c.version == snapshot.snapshot_version
    {
        return c.data.clone();
    }

    let stats = compute_dashboard_stats(snapshot);
    let alerts = compute_alerts(snapshot);
    let insights = compute_dashboard_insights(snapshot);
    let workload_pct = compute_workload_ready_percent(snapshot);
    let ns_utilization = compute_namespace_utilization(snapshot);
    let cluster_resources = compute_cluster_resource_summary(snapshot);
    let (top_cpu_pods, top_mem_pods) = compute_top_pod_consumers(snapshot);

    *guard = Some(DashboardCache {
        version: snapshot.snapshot_version,
        data: DashboardData {
            stats,
            alerts,
            insights,
            workload_pct,
            ns_utilization,
            cluster_resources,
            top_cpu_pods,
            top_mem_pods,
        },
    });

    guard.as_ref().unwrap().data.clone()
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

/// Compact 5-char bar + percentage for inline dashboard use: `▓▓░░░  45%`.
fn mini_bar<'a>(pct: u64, theme: &Theme) -> Vec<Span<'a>> {
    let pct = pct.min(100);
    const W: usize = 5;
    let filled = ((pct as usize) * W + 50) / 100;
    let empty = W - filled;
    let style = utilization_style(pct, theme);
    let dim = Style::default().fg(theme.fg_dim);
    vec![
        Span::styled("▓".repeat(filled), style),
        Span::styled("░".repeat(empty), dim),
        Span::styled(format!("{pct:>4}%"), style),
    ]
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

fn use_narrow_dashboard_layout(area: Rect) -> bool {
    area.width < NARROW_DASHBOARD_WIDTH
}

fn namespace_util_widths(area: Rect) -> [Constraint; 8] {
    if area.width < NARROW_NAMESPACE_UTIL_WIDTH {
        [
            Constraint::Min(14),
            Constraint::Length(5),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(9),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Min(9),
        ]
    } else {
        [
            Constraint::Min(16),
            Constraint::Length(6),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(11),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Min(11),
        ]
    }
}

fn compact_gauge_line<'a>(label: &'a str, pct: u64, theme: &Theme) -> Line<'a> {
    let mut spans = vec![Span::styled(
        format!("  {label:<14} "),
        theme.inactive_style(),
    )];
    spans.extend(mini_bar(pct, theme));
    Line::from(spans)
}

// ── top-level render ──────────────────────────────────────────────────────────

/// Renders the dashboard view.
pub fn render_dashboard(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    alert_scroll: usize,
    _focused: bool,
) {
    let theme = default_theme();
    let d = cached_dashboard(snapshot);

    if use_narrow_dashboard_layout(area) {
        render_narrow_dashboard(frame, area, snapshot, &d, &theme, alert_scroll);
        return;
    }

    // On small terminals (<40 rows) show a compact 4-row layout; otherwise full 7 rows.
    let compact = area.height < 40;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if compact {
            // Compact: cluster info + gauges + resource/overcommit + alerts
            vec![
                Constraint::Length(8),
                Constraint::Length(5),
                Constraint::Length(7),
                Constraint::Min(4),
            ]
        } else {
            vec![
                Constraint::Length(8), // row 0: cluster info | health summary
                Constraint::Length(5), // row 1: 5 gauges
                Constraint::Length(9), // row 2: node saturation | top node pressure
                Constraint::Length(7), // row 3: resource counts | overcommit & governance
                Constraint::Length(9), // row 4: namespace utilization
                Constraint::Length(9), // row 5: top CPU pods | top memory pods
                Constraint::Min(6),    // row 6: alerts
            ]
        })
        .split(area);

    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);

    render_cluster_info(frame, top_cols[0], snapshot, &theme);
    render_cluster_health_summary(
        frame,
        top_cols[1],
        &d.stats,
        &d.insights,
        snapshot.issue_count,
        &theme,
    );
    render_health_gauges(
        frame,
        rows[1],
        &d.stats,
        d.workload_pct,
        &d.cluster_resources,
        &theme,
    );

    if compact {
        // Compact layout: rows[2] = resource/overcommit, rows[3] = alerts
        let summary_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(rows[2]);
        render_resource_counts(frame, summary_cols[0], &d.stats, &theme);
        render_overcommit_governance(frame, summary_cols[1], &d.cluster_resources, &theme);
        render_alerts(frame, rows[3], &d.alerts, &theme, alert_scroll);
    } else {
        // Full layout: rows[2..6]
        let node_rows = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(rows[2]);
        render_node_utilization_summary(frame, node_rows[0], &d.insights, &theme);
        render_hot_nodes(frame, node_rows[1], &d.insights, &theme);

        let summary_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(rows[3]);
        render_resource_counts(frame, summary_cols[0], &d.stats, &theme);
        render_overcommit_governance(frame, summary_cols[1], &d.cluster_resources, &theme);
        render_namespace_utilization(frame, rows[4], &d.ns_utilization, &theme);
        render_top_pod_consumers(frame, rows[5], &d.top_cpu_pods, &d.top_mem_pods, &theme);
        render_alerts(frame, rows[6], &d.alerts, &theme, alert_scroll);
    }
}

fn render_narrow_dashboard(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    d: &DashboardData,
    theme: &Theme,
    alert_scroll: usize,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Min(6),
        ])
        .split(area);

    render_cluster_info(frame, rows[0], snapshot, theme);
    render_cluster_health_summary(
        frame,
        rows[1],
        &d.stats,
        &d.insights,
        snapshot.issue_count,
        theme,
    );
    render_health_gauges(
        frame,
        rows[2],
        &d.stats,
        d.workload_pct,
        &d.cluster_resources,
        theme,
    );
    render_alerts(frame, rows[3], &d.alerts, theme, alert_scroll);
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
    let phase_label = snapshot.phase.to_string();

    let phase_style = match snapshot.phase {
        crate::state::DataPhase::Ready => theme.badge_success_style(),
        crate::state::DataPhase::Loading => theme.badge_warning_style(),
        crate::state::DataPhase::Error => theme.badge_error_style(),
        crate::state::DataPhase::Idle => theme.inactive_style(),
    };

    let last_updated = snapshot
        .last_updated
        .map(|t| format_local(t, "%H:%M:%S"))
        .unwrap_or_else(|| "—".to_string());
    let (metrics_label, metrics_style) = if !snapshot.scope_loaded(RefreshScope::METRICS) {
        ("loading...", theme.badge_warning_style())
    } else if snapshot.node_metrics.is_empty() && snapshot.pod_metrics.is_empty() {
        ("unavailable", theme.badge_error_style())
    } else {
        ("ready", theme.badge_success_style())
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
            Span::styled("  updated ", theme.inactive_style()),
            Span::styled(last_updated, Style::default().fg(theme.fg_dim)),
        ]),
        Line::from(vec![
            Span::styled("  Metrics   ", theme.inactive_style()),
            Span::styled(metrics_label, metrics_style),
        ]),
    ];

    let block = Block::default()
        .title(Span::styled(
            format!(" {}Cluster ", crate::icons::chrome_icon("cluster").active()),
            theme.title_style(),
        ))
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
        .title(Span::styled(
            format!(
                " {}Resources ",
                crate::icons::chrome_icon("resources").active()
            ),
            theme.title_style(),
        ))
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
    if area.width < COMPACT_GAUGE_WIDTH {
        render_compact_health_gauges(frame, area, stats, workload_pct, cluster_res, theme);
        return;
    }

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

fn render_compact_health_gauges(
    frame: &mut Frame,
    area: Rect,
    stats: &DashboardStats,
    workload_pct: u8,
    cluster_res: &ClusterResourceSummary,
    theme: &Theme,
) {
    let block = Block::default()
        .title(Span::styled(" 󰄬 Health Gauges ", theme.title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        compact_gauge_line("Nodes Ready", u64::from(stats.ready_nodes_percent), theme),
        compact_gauge_line("Pods Running", u64::from(stats.running_pods_percent), theme),
        compact_gauge_line("Workload", u64::from(workload_pct), theme),
        compact_gauge_line("Cluster CPU", u64::from(cluster_res.cluster_cpu_pct), theme),
        compact_gauge_line("Cluster Mem", u64::from(cluster_res.cluster_mem_pct), theme),
    ];

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
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
    let style = utilization_style(u64::from(percent), theme);
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
        (ready as f64 / total as f64).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let pct = (ratio * 100.0) as u8;
    let color = if pct >= 90 {
        theme.error
    } else if pct >= 70 {
        theme.warning
    } else {
        theme.success
    };

    let gauge = LineGauge::default()
        .label(Line::from(vec![
            Span::styled(format!("  {label} "), theme.inactive_style()),
            Span::styled(
                format!("{ready}/{total}  {pct}%"),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]))
        .ratio(ratio)
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
            format!(
                " {}Commitment & Governance ",
                crate::icons::chrome_icon("governance").active()
            ),
            theme.title_style(),
        ))
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.bg));

    if res.total_running_pods == 0 && res.total_cpu_allocatable_m == 0 {
        let msg = Paragraph::new(Span::styled("No data available", theme.inactive_style()))
            .alignment(Alignment::Center)
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
            format!("  {}", "─".repeat(inner.width.saturating_sub(4) as usize)),
            dim,
        )),
        Line::from(vec![
            Span::styled("  No CPU Req  ", dim),
            Span::styled(
                res.pods_missing_cpu_request.to_string(),
                missing_style(res.pods_missing_cpu_request),
            ),
            Span::styled("    No Mem Req  ", dim),
            Span::styled(
                res.pods_missing_mem_request.to_string(),
                missing_style(res.pods_missing_mem_request),
            ),
        ]),
        Line::from(vec![
            Span::styled("  No Limits   ", dim),
            Span::styled(
                res.pods_missing_any_limit.to_string(),
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
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(title, theme.title_style()))
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.bg));

    if consumers.is_empty() {
        let msg = Paragraph::new(Span::styled("No metrics available", theme.inactive_style()))
            .alignment(Alignment::Center)
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
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(
            " Namespace Utilization (Top 5) ",
            theme.title_style(),
        ))
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.bg));

    if ns_util.is_empty()
        || ns_util
            .iter()
            .all(|n| n.cpu_usage_m == 0 && n.mem_usage_mib == 0)
    {
        let msg = Paragraph::new(Span::styled(
            "No metrics available (metrics-server not detected)",
            theme.inactive_style(),
        ))
        .alignment(Alignment::Center)
        .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let dim = Style::default().fg(theme.fg_dim);

    let header = Row::new([
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Pods", theme.header_style())),
        Cell::from(Span::styled("CPU Use", theme.header_style())),
        Cell::from(Span::styled("CPU Req", theme.header_style())),
        Cell::from(Span::styled("%CPU/R", theme.header_style())),
        Cell::from(Span::styled("Mem Use", theme.header_style())),
        Cell::from(Span::styled("Mem Req", theme.header_style())),
        Cell::from(Span::styled("%MEM/R", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let rows: Vec<Row> = ns_util
        .iter()
        .take(TOP_N)
        .map(|ns| {
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
            let ns_name = truncate_label(&ns.namespace, 24);

            let cpu_bar_spans = ns
                .cpu_req_utilization_pct
                .map(|p| mini_bar(u64::from(p), theme))
                .unwrap_or_else(|| vec![Span::styled("    -", dim)]);
            let mem_bar_spans = ns
                .mem_req_utilization_pct
                .map(|p| mini_bar(u64::from(p), theme))
                .unwrap_or_else(|| vec![Span::styled("    -", dim)]);

            Row::new(vec![
                Cell::from(Span::styled(ns_name.into_owned(), dim)),
                Cell::from(Span::styled(
                    format!("{}", ns.pod_count),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(cpu_used, Style::default().fg(theme.accent))),
                Cell::from(Span::styled(cpu_req, dim)),
                Cell::from(Line::from(cpu_bar_spans)),
                Cell::from(Span::styled(mem_used, Style::default().fg(theme.accent2))),
                Cell::from(Span::styled(mem_req, dim)),
                Cell::from(Line::from(mem_bar_spans)),
            ])
        })
        .collect();

    let table = Table::new(rows, namespace_util_widths(area))
        .header(header)
        .block(block);

    frame.render_widget(table, area);
}

// ── alerts ────────────────────────────────────────────────────────────────────

fn render_alerts(
    frame: &mut Frame,
    area: Rect,
    alerts: &[AlertItem],
    theme: &Theme,
    scroll: usize,
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

    let inner = block.inner(area);
    let total = wrapped_line_count(&alert_lines, inner.width);
    let position = scroll.min(total.saturating_sub(inner.height.max(1) as usize));
    frame.render_widget(
        Paragraph::new(alert_lines)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((position.min(u16::MAX as usize) as u16, 0)),
        area,
    );
    crate::ui::components::render_vertical_scrollbar(frame, inner, total, position);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_util_widths_switch_to_compact_profile() {
        let widths = namespace_util_widths(Rect::new(0, 0, 80, 20));
        assert_eq!(widths[0], Constraint::Min(14));
        assert_eq!(widths[1], Constraint::Length(5));
        assert_eq!(widths[4], Constraint::Length(9));
        assert_eq!(widths[7], Constraint::Min(9));
    }

    #[test]
    fn namespace_util_widths_keep_wide_profile() {
        let widths = namespace_util_widths(Rect::new(0, 0, 120, 20));
        assert_eq!(widths[0], Constraint::Min(16));
        assert_eq!(widths[1], Constraint::Length(6));
        assert_eq!(widths[4], Constraint::Length(11));
        assert_eq!(widths[7], Constraint::Min(11));
    }

    #[test]
    fn dashboard_alert_scroll_clamps_to_last_page() {
        let lines = vec![
            Line::from("alert 1"),
            Line::from("alert 2"),
            Line::from("alert 3"),
        ];
        let total = wrapped_line_count(&lines, 20);
        let position = 999usize.min(total.saturating_sub(2));
        assert_eq!(position, 1);
    }
}
