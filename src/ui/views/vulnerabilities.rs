//! Vulnerability center view — Trivy Operator workload vulnerability aggregation.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Row},
};

use crate::{
    app::AppView,
    icons::StatusIcons,
    state::{
        ClusterSnapshot,
        vulnerabilities::{compute_vulnerability_findings, filtered_vulnerability_indices},
    },
    ui::{
        ResourceTableConfig, components::default_theme, format_small_int, render_resource_table,
        striped_row_style,
    },
};

pub fn render_vulnerabilities(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search: &str,
    focused: bool,
) {
    let theme = default_theme();
    let query = search.trim();
    let findings = compute_vulnerability_findings(cluster);
    let indices = filtered_vulnerability_indices(&findings, query);
    let widths = [
        Constraint::Length(3),
        Constraint::Length(22),
        Constraint::Length(18),
        Constraint::Length(16),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Min(22),
    ];

    render_resource_table(
        frame,
        area,
        &theme,
        ResourceTableConfig {
            snapshot: cluster,
            view: AppView::Vulnerabilities,
            label: "Vulnerabilities",
            loading_message: "Loading vulnerability reports...",
            empty_message: "No vulnerability reports found",
            empty_query_message: "No vulnerability reports match the search query",
            query,
            focused,
            filtered_total: indices.len(),
            all_total: findings.len(),
            selected_idx,
            widths: &widths,
            sort_suffix: "",
        },
        |theme| {
            Row::new([
                Cell::from(Span::styled("SEV", theme.header_style())),
                Cell::from(Span::styled("NAME", theme.header_style())),
                Cell::from(Span::styled("NAMESPACE", theme.header_style())),
                Cell::from(Span::styled("KIND", theme.header_style())),
                Cell::from(Span::styled("CRIT", theme.header_style())),
                Cell::from(Span::styled("HIGH", theme.header_style())),
                Cell::from(Span::styled("MED", theme.header_style())),
                Cell::from(Span::styled("FIX", theme.header_style())),
                Cell::from(Span::styled("ARTIFACTS", theme.header_style())),
            ])
            .height(1)
            .style(theme.header_style())
        },
        |window, theme| {
            indices[window.start..window.end]
                .iter()
                .enumerate()
                .map(|(local_idx, &finding_idx)| {
                    let idx = window.start + local_idx;
                    let finding = &findings[finding_idx];
                    let (icon, icon_style) = match finding.severity {
                        crate::k8s::dtos::AlertSeverity::Error => {
                            (StatusIcons::error().active(), theme.badge_error_style())
                        }
                        crate::k8s::dtos::AlertSeverity::Warning => {
                            (StatusIcons::warning().active(), theme.badge_warning_style())
                        }
                        crate::k8s::dtos::AlertSeverity::Info => {
                            (StatusIcons::info().active(), theme.inactive_style())
                        }
                    };
                    let artifact_label = if finding.artifacts.is_empty() {
                        "—".to_string()
                    } else {
                        finding.artifacts.join(", ")
                    };
                    let namespace = if finding.namespace.is_empty() {
                        "cluster".to_string()
                    } else {
                        finding.namespace.clone()
                    };
                    Row::new(vec![
                        Cell::from(Span::styled(icon, icon_style)),
                        Cell::from(Span::styled(
                            finding.resource_name.as_str(),
                            Style::default().fg(theme.fg),
                        )),
                        Cell::from(Span::styled(namespace, Style::default().fg(theme.fg_dim))),
                        Cell::from(Span::styled(
                            finding.resource_kind.as_str(),
                            Style::default().fg(theme.accent2),
                        )),
                        Cell::from(Span::styled(
                            format_small_int(finding.counts.critical as i64),
                            theme.badge_error_style(),
                        )),
                        Cell::from(Span::styled(
                            format_small_int(finding.counts.high as i64),
                            theme.badge_error_style(),
                        )),
                        Cell::from(Span::styled(
                            format_small_int(finding.counts.medium as i64),
                            theme.badge_warning_style(),
                        )),
                        Cell::from(Span::styled(
                            format_small_int(finding.fixable_count as i64),
                            if finding.fixable_count > 0 {
                                theme.badge_success_style()
                            } else {
                                theme.inactive_style()
                            },
                        )),
                        Cell::from(Span::styled(
                            artifact_label,
                            Style::default().fg(theme.info),
                        )),
                    ])
                    .style(striped_row_style(idx, theme))
                })
                .collect()
        },
    );
}
