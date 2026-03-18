//! Issue Center view — problem-first cluster diagnostics.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Row},
};

use crate::{
    app::AppView,
    icons::{StatusIcons, view_icon},
    k8s::dtos::AlertSeverity,
    state::{
        ClusterSnapshot, RefreshScope,
        issues::{compute_issues, filtered_issue_indices},
    },
    ui::{
        TableFrame, components::default_theme, render_centered_message, render_table_frame,
        table_viewport_rows, table_window,
    },
};

pub fn render_issues(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search: &str,
    focused: bool,
) {
    let theme = default_theme();
    let query = search.trim();
    let all_issues = compute_issues(cluster);
    let diagnostics_loaded = cluster.scope_loaded(
        RefreshScope::CORE_OVERVIEW
            .union(RefreshScope::LEGACY_SECONDARY)
            .union(RefreshScope::FLUX),
    );

    let indices = filtered_issue_indices(&all_issues, query);

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::Issues,
            query,
            "Issues",
            if diagnostics_loaded {
                "Scanning for issues..."
            } else {
                "Scanning for issues... diagnostic backfill still running"
            },
            "No issues detected — cluster looks healthy",
            "No issues match the search query",
            focused,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let header = Row::new([
        Cell::from(Span::styled("SEV", theme.header_style())),
        Cell::from(Span::styled("CATEGORY", theme.header_style())),
        Cell::from(Span::styled("KIND", theme.header_style())),
        Cell::from(Span::styled("NAME", theme.header_style())),
        Cell::from(Span::styled("NAMESPACE", theme.header_style())),
        Cell::from(Span::styled("MESSAGE", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &issue_idx)| {
            let idx = window.start + local_idx;
            let issue = &all_issues[issue_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };

            let (sev_icon, sev_style) = match issue.severity {
                AlertSeverity::Error => (StatusIcons::error().active(), theme.badge_error_style()),
                AlertSeverity::Warning => {
                    (StatusIcons::warning().active(), theme.badge_warning_style())
                }
                AlertSeverity::Info => (StatusIcons::info().active(), theme.inactive_style()),
            };

            Row::new(vec![
                Cell::from(Span::styled(sev_icon, sev_style)),
                Cell::from(issue.category.label()),
                Cell::from(issue.resource_kind),
                Cell::from(issue.resource_name.as_str()),
                Cell::from(issue.namespace.as_str()),
                Cell::from(issue.message.chars().take(80).collect::<String>()),
            ])
            .style(row_style)
        })
        .collect();

    let icon = view_icon(AppView::Issues).active();
    let coverage_suffix = if diagnostics_loaded {
        ""
    } else {
        " [partial coverage]"
    };
    let title = if query.is_empty() {
        format!(" {icon}Issues ({total}){coverage_suffix} ")
    } else {
        let all = all_issues.len();
        format!(" {icon}Issues ({total} of {all}) [/{query}]{coverage_suffix}")
    };
    let widths = [
        Constraint::Length(3),
        Constraint::Length(20),
        Constraint::Length(14),
        Constraint::Min(20),
        Constraint::Length(16),
        Constraint::Min(20),
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
            selected,
        },
        &theme,
    );
}
