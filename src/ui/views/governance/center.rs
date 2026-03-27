//! Governance & Cost Center view.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Wrap},
};

use crate::{
    app::AppView,
    governance::{NamespaceGovernanceSummary, compute_governance, filtered_governance_indices},
    icons::{StatusIcons, view_icon},
    k8s::dtos::AlertSeverity,
    state::{ClusterSnapshot, RefreshScope},
    ui::{
        TableFrame,
        components::{content_block, default_theme},
        render_centered_message, render_table_frame, table_viewport_rows, table_window,
    },
};

pub fn render_governance(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search: &str,
    focused: bool,
) {
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
            focused,
        );
        return;
    }

    let selected = selected_idx.min(indices.len().saturating_sub(1));
    let selected_summary = &summaries[indices[selected]];
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    render_governance_table(
        frame,
        chunks[0],
        &summaries,
        &indices,
        selected,
        search.trim(),
        focused,
    );
    render_governance_summary(frame, chunks[1], selected_summary, focused);
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
                Cell::from(summary.namespace.clone()),
                Cell::from(summary.project_count_label.clone()),
                Cell::from(summary.workload_count_label.clone()),
                Cell::from(summary.total_issue_count_label.clone()),
                Cell::from(summary.policy_surface_count_label.clone()),
                Cell::from(summary.vulnerability_total_label.clone()),
                Cell::from(summary.cpu_req_utilization_label.clone()),
                Cell::from(summary.mem_req_utilization_label.clone()),
                Cell::from(summary.idle_request_label.clone()),
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
    let widths = [
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
    ];

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

    if summary.namespace != "cluster" {
        lines.push(Line::from(vec![
            Span::styled("Enter opens: ", Style::default().fg(theme.fg_dim)),
            Span::styled(
                format!("Namespace/{}", summary.namespace),
                Style::default()
                    .fg(theme.info)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
        ]));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(content_block("Governance Summary", focused)),
        area,
    );
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
                    true,
                );
            })
            .expect("render");
    }
}
