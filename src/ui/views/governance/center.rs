//! Governance & Cost Center view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Cell, Row},
};

use crate::{
    app::AppView,
    governance::{NamespaceGovernanceSummary, compute_governance, filtered_governance_indices},
    icons::{StatusIcons, view_icon},
    k8s::dtos::AlertSeverity,
    state::{ClusterSnapshot, RefreshScope},
    ui::{
        SplitPaneFocus, TableFrame,
        components::{default_theme, render_scrollable_text_block},
        render_centered_message, render_table_frame, table_viewport_rows, table_window,
        vertical_primary_detail_chunks,
    },
};

const GOVERNANCE_COMPACT_HEIGHT: u16 = 24;
const GOVERNANCE_NARROW_WIDTH: u16 = 104;

fn governance_widths(area: Rect) -> [Constraint; 10] {
    if area.width < GOVERNANCE_NARROW_WIDTH {
        [
            Constraint::Length(3),
            Constraint::Min(14),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Min(10),
        ]
    } else {
        [
            Constraint::Length(3),
            Constraint::Length(18),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(7),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Min(14),
        ]
    }
}

pub(crate) fn render_governance(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search: &str,
    detail_scroll: usize,
    focus: SplitPaneFocus,
) {
    let list_focused = matches!(focus, SplitPaneFocus::List);
    let detail_focused = matches!(focus, SplitPaneFocus::Detail);
    let summaries = compute_governance(cluster);
    let indices = filtered_governance_indices(&summaries, search.trim());
    let loaded = cluster.scope_loaded(
        RefreshScope::CORE_OVERVIEW
            .union(RefreshScope::METRICS)
            .union(RefreshScope::LEGACY_SECONDARY)
            .union(RefreshScope::NETWORK)
            .union(RefreshScope::SECURITY),
    );

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::Governance,
            search.trim(),
            "Governance",
            if loaded {
                "Synthesizing governance and cost rollups..."
            } else {
                "Synthesizing governance and cost rollups... related snapshot buckets are still loading"
            },
            "No governance rows inferred from the current snapshot",
            "No governance rows match the search query",
            list_focused,
        );
        return;
    }

    let selected = selected_idx.min(indices.len().saturating_sub(1));
    let selected_summary = &summaries[indices[selected]];
    let (table_area, summary_area) =
        vertical_primary_detail_chunks(area, 58, 8, GOVERNANCE_COMPACT_HEIGHT);
    render_governance_table(
        frame,
        table_area,
        &summaries,
        &indices,
        selected,
        search.trim(),
        list_focused,
    );
    render_governance_summary(
        frame,
        summary_area,
        selected_summary,
        detail_scroll,
        detail_focused,
    );
}

fn render_governance_table(
    frame: &mut Frame,
    area: Rect,
    summaries: &[NamespaceGovernanceSummary],
    indices: &[usize],
    selected_idx: usize,
    query: &str,
    focused: bool,
) {
    let theme = default_theme();
    let total = indices.len();
    let window = table_window(total, selected_idx, table_viewport_rows(area));
    let header = Row::new([
        Cell::from(Span::styled("SEV", theme.header_style())),
        Cell::from(Span::styled("NAMESPACE", theme.header_style())),
        Cell::from(Span::styled("PROJ", theme.header_style())),
        Cell::from(Span::styled("WL", theme.header_style())),
        Cell::from(Span::styled("ISS", theme.header_style())),
        Cell::from(Span::styled("POL", theme.header_style())),
        Cell::from(Span::styled("VULN", theme.header_style())),
        Cell::from(Span::styled("CPU/R", theme.header_style())),
        Cell::from(Span::styled("MEM/R", theme.header_style())),
        Cell::from(Span::styled("IDLE REQ", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let rows = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &summary_idx)| {
            let absolute_idx = window.start + local_idx;
            let summary = &summaries[summary_idx];
            let row_style = if absolute_idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let (icon, icon_style) = severity_badge(summary.highest_severity);
            Row::new(vec![
                Cell::from(Span::styled(icon, icon_style)),
                Cell::from(summary.namespace.as_str()),
                Cell::from(summary.project_count_label.as_str()),
                Cell::from(summary.workload_count_label.as_str()),
                Cell::from(summary.total_issue_count_label.as_str()),
                Cell::from(summary.policy_surface_count_label.as_str()),
                Cell::from(summary.vulnerability_total_label.as_str()),
                Cell::from(summary.cpu_req_utilization_label.as_str()),
                Cell::from(summary.mem_req_utilization_label.as_str()),
                Cell::from(summary.idle_request_label.as_str()),
            ])
            .style(row_style)
        })
        .collect();

    let icon = view_icon(AppView::Governance).active();
    let title = if query.is_empty() {
        format!(" {icon}Governance ({total}) ")
    } else {
        format!(
            " {icon}Governance ({total} of {}) [/{query}] ",
            summaries.len()
        )
    };
    let widths = governance_widths(area);

    render_table_frame(
        frame,
        area,
        TableFrame {
            rows,
            header,
            widths: &widths,
            title: &title,
            focused,
            window,
            total,
            selected: selected_idx,
        },
        &theme,
    );
}

fn render_governance_summary(
    frame: &mut Frame,
    area: Rect,
    summary: &NamespaceGovernanceSummary,
    scroll: usize,
    focused: bool,
) {
    let theme = default_theme();
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(summary.namespace.as_str(), theme.header_style()),
        Span::styled("  Governance & Cost", Style::default().fg(theme.fg_dim)),
    ]));
    lines.push(Line::from(format!("Projects: {}", summary.projects_label)));
    lines.push(Line::from(summary.counts_summary_label.as_str()));
    lines.push(Line::from(summary.policy_surfaces_label.as_str()));
    lines.push(Line::from(summary.vulnerabilities_label.as_str()));
    lines.push(Line::from(summary.requests_label.as_str()));
    lines.push(Line::from(format!(
        "Idle request cost proxy: {}",
        summary.idle_request_label
    )));
    lines.push(Line::from(summary.coverage_gaps_label.as_str()));

    if summary.risk_signals.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Key signals: ", Style::default().fg(theme.fg_dim)),
            Span::styled("none", theme.badge_success_style()),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "Key signals:",
            Style::default().fg(theme.fg_dim),
        )));
        for signal in &summary.risk_signals {
            lines.push(Line::from(format!("• {signal}")));
        }
    }

    if summary.top_workloads.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Top workload risks: ", Style::default().fg(theme.fg_dim)),
            Span::styled("none", theme.badge_success_style()),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "Top workload risks:",
            Style::default().fg(theme.fg_dim),
        )));
        for workload in &summary.top_workloads {
            lines.push(Line::from(format!("• {}", workload.compact_label)));
        }
    }

    if let Some(representative) = &summary.representative {
        let accent = Style::default()
            .fg(theme.info)
            .add_modifier(ratatui::style::Modifier::BOLD);
        let mut spans = vec![
            Span::styled("Enter opens: ", Style::default().fg(theme.fg_dim)),
            Span::styled(representative.kind(), accent),
            Span::styled("/", accent),
        ];
        if let Some(namespace) = representative.namespace() {
            spans.push(Span::styled(namespace, accent));
            spans.push(Span::styled("/", accent));
        }
        spans.push(Span::styled(representative.name(), accent));
        lines.push(Line::from(spans));
    }

    render_scrollable_text_block(frame, area, "Governance Summary", focused, lines, scroll);
}

fn severity_badge(severity: AlertSeverity) -> (&'static str, Style) {
    let theme = default_theme();
    match severity {
        AlertSeverity::Error => (StatusIcons::error().active(), theme.badge_error_style()),
        AlertSeverity::Warning => (StatusIcons::warning().active(), theme.badge_warning_style()),
        AlertSeverity::Info => (
            StatusIcons::info().active(),
            Style::default().fg(theme.info),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::ResourceRef, governance::GovernanceWorkloadSummary};

    fn render_summary_to_string(summary: &NamespaceGovernanceSummary) -> String {
        let backend = ratatui::backend::TestBackend::new(120, 20);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                render_governance_summary(frame, frame.area(), summary, 0, true);
            })
            .expect("render");
        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn summary_with_representative(
        representative: Option<ResourceRef>,
    ) -> NamespaceGovernanceSummary {
        NamespaceGovernanceSummary {
            namespace: "team-a".to_string(),
            project_count: 1,
            project_count_label: "1".to_string(),
            workload_count: 1,
            workload_count_label: "1".to_string(),
            pod_count: 1,
            runtime_issue_count: 0,
            sanitizer_issue_count: 0,
            security_issue_count: 0,
            total_issue_count_label: "0".to_string(),
            vulnerability_total: 0,
            vulnerability_total_label: "0".to_string(),
            fixable_vulnerabilities: 0,
            quota_count: 0,
            limit_range_count: 0,
            pdb_gap_count: 0,
            policy_surface_count_label: "0".to_string(),
            missing_cpu_request_pods: 0,
            missing_mem_request_pods: 0,
            missing_limit_pods: 0,
            cpu_usage_m: 0,
            mem_usage_mib: 0,
            cpu_request_m: 0,
            mem_request_mib: 0,
            cpu_req_utilization_pct: None,
            cpu_req_utilization_label: "n/a".to_string(),
            mem_req_utilization_pct: None,
            mem_req_utilization_label: "n/a".to_string(),
            idle_cpu_request_m: 0,
            idle_mem_request_mib: 0,
            idle_request_label: "0m/0Mi".to_string(),
            highest_severity: AlertSeverity::Info,
            representative,
            projects: vec!["payments".to_string()],
            projects_label: "payments".to_string(),
            counts_summary_label:
                "Workloads: 1 • Pods: 1 • Issues: runtime 0 / sanitizer 0 / security 0".to_string(),
            policy_surfaces_label:
                "Policy surfaces: ResourceQuota 0 • LimitRange 0 • Missing PDB 0".to_string(),
            vulnerabilities_label: "Vulnerabilities: 0 total • 0 fixable".to_string(),
            requests_label: "Requests: CPU 0m/0m (n/a) • Mem 0Mi/0Mi (n/a)".to_string(),
            coverage_gaps_label:
                "Coverage gaps: missing CPU req 0 • missing Mem req 0 • missing limit 0".to_string(),
            risk_signals: vec!["Low request utilization".to_string()],
            top_workloads: vec![GovernanceWorkloadSummary {
                resource_ref: ResourceRef::Deployment("api".to_string(), "team-a".to_string()),
                issue_count: 0,
                vulnerability_total: 0,
                fixable_vulnerabilities: 0,
                cpu_request_m: 0,
                mem_request_mib: 0,
                cpu_usage_m: 0,
                mem_usage_mib: 0,
                missing_requests: 0,
                missing_limits: 0,
                highest_severity: AlertSeverity::Info,
                compact_label: "Deployment/api".to_string(),
            }],
        }
    }

    #[test]
    fn render_governance_empty_smoke() {
        let backend = ratatui::backend::TestBackend::new(120, 30);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                render_governance(
                    frame,
                    frame.area(),
                    &ClusterSnapshot::default(),
                    0,
                    "",
                    0,
                    SplitPaneFocus::List,
                );
            })
            .expect("render");
    }

    #[test]
    fn render_governance_summary_uses_actual_representative_label() {
        let summary = summary_with_representative(Some(ResourceRef::Deployment(
            "api".to_string(),
            "team-a".to_string(),
        )));

        let rendered = render_summary_to_string(&summary);
        assert!(rendered.contains("Enter opens:"));
        assert!(rendered.contains("Deployment/team-a/api"));
        assert!(!rendered.contains("Namespace/team-a"));
    }

    #[test]
    fn render_governance_summary_hides_enter_hint_without_valid_representative() {
        let summary = summary_with_representative(None);

        let rendered = render_summary_to_string(&summary);
        assert!(!rendered.contains("Enter opens:"));
    }

    #[test]
    fn governance_widths_switch_to_compact_profile() {
        let widths = governance_widths(Rect::new(0, 0, 96, 20));
        assert_eq!(widths[0], Constraint::Length(3));
        assert_eq!(widths[1], Constraint::Min(14));
        assert_eq!(widths[9], Constraint::Min(10));
    }

    #[test]
    fn governance_widths_keep_wide_profile() {
        let widths = governance_widths(Rect::new(0, 0, 132, 20));
        assert_eq!(widths[1], Constraint::Length(18));
        assert_eq!(widths[7], Constraint::Length(8));
        assert_eq!(widths[9], Constraint::Min(14));
    }
}
