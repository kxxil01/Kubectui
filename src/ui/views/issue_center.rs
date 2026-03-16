//! Issue Center view — problem-first cluster diagnostics.

use ratatui::{
    layout::{Constraint, Margin, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{
        Cell, HighlightSpacing, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState,
    },
};

use crate::{
    k8s::dtos::AlertSeverity,
    state::{
        ClusterSnapshot, RefreshScope,
        issues::{compute_issues, filtered_issue_indices},
    },
    ui::{
        components::{content_block, default_block, default_theme},
        loading_or_empty_message, responsive_table_widths, table_viewport_rows, table_window,
    },
};

use crate::app::AppView;

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
        let msg = loading_or_empty_message(
            cluster,
            AppView::Issues,
            query,
            if diagnostics_loaded {
                "  Scanning for issues..."
            } else {
                "  Scanning for issues... diagnostic backfill still running"
            },
            "  No issues detected — cluster looks healthy",
            "  No issues match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("Issues")),
            area,
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
                AlertSeverity::Error => ("✗", theme.badge_error_style()),
                AlertSeverity::Warning => ("⚠", theme.badge_warning_style()),
                AlertSeverity::Info => ("ℹ", theme.inactive_style()),
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

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let title = if query.is_empty() {
        if diagnostics_loaded {
            format!(" Issues ({total}) ")
        } else {
            format!(" Issues ({total}) [partial coverage] ")
        }
    } else {
        let all = all_issues.len();
        if diagnostics_loaded {
            format!(" Issues ({total} of {all}) [/{query}]")
        } else {
            format!(" Issues ({total} of {all}) [/{query}] [partial coverage]")
        }
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Length(3),
                Constraint::Length(20),
                Constraint::Length(14),
                Constraint::Min(20),
                Constraint::Length(16),
                Constraint::Min(20),
            ],
        ),
    )
    .header(header)
    .block(content_block(&title, focused))
    .row_highlight_style(theme.selection_style())
    .highlight_symbol(theme.highlight_symbol())
    .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(table, area, &mut table_state);

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"))
        .track_symbol(Some("│"))
        .thumb_symbol("█");

    let mut scrollbar_state = ScrollbarState::new(total).position(selected);
    frame.render_stateful_widget(
        scrollbar,
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}
