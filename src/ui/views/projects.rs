//! Project/application scope view built from snapshot-cached native label inference.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Cell, Row},
};

use crate::{
    app::AppView,
    icons::{StatusIcons, view_icon},
    k8s::dtos::AlertSeverity,
    projects::{ProjectSummary, compute_projects, filtered_project_indices},
    state::ClusterSnapshot,
    ui::{
        SplitPaneFocus, TableFrame,
        components::{default_theme, render_scrollable_text_block},
        render_centered_message, render_table_frame, responsive_table_widths, table_viewport_rows,
        table_window, vertical_primary_detail_chunks,
    },
};

const PROJECTS_COMPACT_HEIGHT: u16 = 24;
const PROJECTS_NARROW_WIDTH: u16 = 96;

pub(crate) fn render_projects(
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
    let projects = compute_projects(cluster);
    let indices = filtered_project_indices(&projects, search.trim());
    let loaded = cluster.scope_loaded(
        crate::state::RefreshScope::CORE_OVERVIEW
            .union(crate::state::RefreshScope::LEGACY_SECONDARY)
            .union(crate::state::RefreshScope::NETWORK)
            .union(crate::state::RefreshScope::SECURITY),
    );

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::Projects,
            search.trim(),
            "Projects",
            if loaded {
                "Scanning native labels for application scopes..."
            } else {
                "Scanning native labels for application scopes... related snapshot buckets are still loading"
            },
            "No inferred projects found from current native labels",
            "No inferred projects match the search query",
            list_focused,
        );
        return;
    }

    let selected = selected_idx.min(indices.len().saturating_sub(1));
    let selected_project = &projects[indices[selected]];
    let (table_area, summary_area) =
        vertical_primary_detail_chunks(area, 60, 8, PROJECTS_COMPACT_HEIGHT);
    render_project_table(
        frame,
        table_area,
        &projects,
        &indices,
        selected,
        search.trim(),
        list_focused,
    );
    render_project_summary(
        frame,
        summary_area,
        selected_project,
        detail_scroll,
        detail_focused,
    );
}

fn project_table_widths(area: Rect) -> [Constraint; 8] {
    let wide = if area.width < PROJECTS_NARROW_WIDTH {
        [
            Constraint::Length(3),
            Constraint::Min(18),
            Constraint::Min(22),
            Constraint::Min(16),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(6),
            Constraint::Length(6),
        ]
    } else {
        [
            Constraint::Length(3),
            Constraint::Length(22),
            Constraint::Length(28),
            Constraint::Length(22),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(8),
        ]
    };

    responsive_table_widths(area.width, wide)
}

fn render_project_table(
    frame: &mut Frame,
    area: Rect,
    projects: &[ProjectSummary],
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
        Cell::from(Span::styled("PROJECT", theme.header_style())),
        Cell::from(Span::styled("SOURCE", theme.header_style())),
        Cell::from(Span::styled("NAMESPACES", theme.header_style())),
        Cell::from(Span::styled("WORKLOADS", theme.header_style())),
        Cell::from(Span::styled("SERVICES", theme.header_style())),
        Cell::from(Span::styled("PODS", theme.header_style())),
        Cell::from(Span::styled("ISSUES", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let rows = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &project_idx)| {
            let absolute_idx = window.start + local_idx;
            let project = &projects[project_idx];
            let (icon, icon_style) = severity_badge(project.highest_severity, project.issue_count);
            let row_style = if absolute_idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };

            Row::new(vec![
                Cell::from(Span::styled(icon, icon_style.patch(theme.header_style()))),
                Cell::from(project.name.as_str()),
                Cell::from(project.source_label.as_str()),
                Cell::from(project.namespaces_label.as_str()),
                Cell::from(project.workload_count_label.as_str()),
                Cell::from(project.services_label.as_str()),
                Cell::from(project.pods_label.as_str()),
                Cell::from(project.issue_count_label.as_str()),
            ])
            .style(row_style)
        })
        .collect();

    let icon = view_icon(AppView::Projects).active();
    let title = if query.is_empty() {
        format!(" {icon}Projects ({total}) ")
    } else {
        format!(
            " {icon}Projects ({total} of {}) [/{query}] ",
            projects.len()
        )
    };
    let widths = project_table_widths(area);

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

fn render_project_summary(
    frame: &mut Frame,
    area: Rect,
    project: &ProjectSummary,
    scroll: usize,
    focused: bool,
) {
    let theme = default_theme();
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(project.name.as_str(), theme.header_style()),
        Span::styled(
            format!("  [{}]", project.source_label),
            Style::default().fg(theme.fg_dim),
        ),
    ]));
    lines.push(Line::from(format!(
        "Namespaces: {}",
        project.namespaces_label
    )));
    lines.push(Line::from(format!(
        "Workloads: {} (deployments {}, stateful sets {}, daemon sets {}, jobs {}, cron jobs {})",
        project.workload_count(),
        project.deployments,
        project.statefulsets,
        project.daemonsets,
        project.jobs,
        project.cronjobs
    )));
    lines.push(Line::from(format!(
        "Traffic: {} service(s), {} ingress(es), {} gateway route(s) • Pods: {}",
        project.services,
        project.ingresses,
        project.http_routes + project.grpc_routes,
        project.pods
    )));
    lines.push(Line::from(format!(
        "Health: {} issue(s) • Highest severity: {}",
        project.issue_count,
        severity_label(project.highest_severity)
    )));

    if !project.sample_workloads.is_empty() {
        lines.push(Line::from(format!(
            "Workload sample: {}",
            project.sample_workloads.join(", ")
        )));
    }
    if !project.sample_services.is_empty() {
        lines.push(Line::from(format!(
            "Services: {}",
            project.sample_services.join(", ")
        )));
    }
    if !project.sample_ingresses.is_empty() {
        lines.push(Line::from(format!(
            "Ingresses: {}",
            project.sample_ingresses.join(", ")
        )));
    }
    if !project.sample_routes.is_empty() {
        lines.push(Line::from(format!(
            "Gateway routes: {}",
            project.sample_routes.join(", ")
        )));
    }

    if project.recent_issues.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Recent issues: ", Style::default().fg(theme.fg_dim)),
            Span::styled("none", theme.badge_success_style()),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "Recent issues:",
            Style::default().fg(theme.fg_dim),
        )));
        for issue in &project.recent_issues {
            lines.push(Line::from(format!("• {issue}")));
        }
    }

    if let Some(representative) = &project.representative {
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

    render_scrollable_text_block(frame, area, "Project Summary", focused, lines, scroll);
}

fn severity_badge(severity: AlertSeverity, issue_count: usize) -> (&'static str, Style) {
    let theme = default_theme();
    if issue_count == 0 {
        return (
            StatusIcons::bookmark().active(),
            theme.badge_success_style(),
        );
    }

    match severity {
        AlertSeverity::Error => (StatusIcons::error().active(), theme.badge_error_style()),
        AlertSeverity::Warning => (StatusIcons::warning().active(), theme.badge_warning_style()),
        AlertSeverity::Info => (
            StatusIcons::info().active(),
            Style::default().fg(theme.info),
        ),
    }
}

fn severity_label(severity: AlertSeverity) -> &'static str {
    match severity {
        AlertSeverity::Error => "error",
        AlertSeverity::Warning => "warning",
        AlertSeverity::Info => "info",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::dtos::PodInfo;

    #[test]
    fn render_projects_empty_smoke() {
        let backend = ratatui::backend::TestBackend::new(120, 30);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                render_projects(
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
    fn render_projects_rows_smoke() {
        let backend = ratatui::backend::TestBackend::new(120, 30);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 9,
            ..ClusterSnapshot::default()
        };
        snapshot.pods.push(PodInfo {
            name: "api-123".into(),
            namespace: "payments".into(),
            labels: vec![("app.kubernetes.io/part-of".into(), "checkout".into())],
            ..PodInfo::default()
        });
        terminal
            .draw(|frame| {
                render_projects(
                    frame,
                    frame.area(),
                    &snapshot,
                    0,
                    "",
                    0,
                    SplitPaneFocus::List,
                );
            })
            .expect("render");
    }

    #[test]
    fn project_table_widths_compact_on_narrow_area() {
        let widths = project_table_widths(Rect::new(0, 0, 84, 20));
        assert!(matches!(widths[1], Constraint::Percentage(_)));
        assert!(matches!(widths[2], Constraint::Percentage(_)));
        assert!(matches!(widths[3], Constraint::Percentage(_)));
    }

    #[test]
    fn render_projects_compact_height_smoke() {
        let backend = ratatui::backend::TestBackend::new(96, 18);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 9,
            ..ClusterSnapshot::default()
        };
        snapshot.pods.push(PodInfo {
            name: "api-123".into(),
            namespace: "payments".into(),
            labels: vec![("app.kubernetes.io/part-of".into(), "checkout".into())],
            ..PodInfo::default()
        });
        terminal
            .draw(|frame| {
                render_projects(
                    frame,
                    frame.area(),
                    &snapshot,
                    0,
                    "",
                    0,
                    SplitPaneFocus::List,
                );
            })
            .expect("render");
    }

    #[test]
    fn render_projects_highlights_summary_when_secondary_pane_focused() {
        let backend = ratatui::backend::TestBackend::new(120, 30);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 9,
            ..ClusterSnapshot::default()
        };
        snapshot.pods.push(PodInfo {
            name: "api-123".into(),
            namespace: "payments".into(),
            labels: vec![("app.kubernetes.io/part-of".into(), "checkout".into())],
            ..PodInfo::default()
        });

        let area = Rect::new(0, 0, 120, 30);
        let (table_area, summary_area) =
            crate::ui::vertical_primary_detail_chunks(area, 60, 8, PROJECTS_COMPACT_HEIGHT);

        terminal
            .draw(|frame| {
                render_projects(frame, area, &snapshot, 0, "", 0, SplitPaneFocus::Detail);
            })
            .expect("render");

        let buffer = terminal.backend().buffer();
        let table_border = buffer[(table_area.x, table_area.y)].style().fg;
        let summary_border = buffer[(summary_area.x, summary_area.y)].style().fg;
        assert_ne!(table_border, summary_border);
        assert_eq!(summary_border, default_theme().border_active_style().fg,);
    }
}
