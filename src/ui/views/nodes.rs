//! Nodes list renderer.

use chrono::Utc;
use ratatui::{
    layout::{Constraint, Margin, Rect},
    prelude::{Frame, Line, Style},
    text::Span,
    widgets::{
        Cell, HighlightSpacing, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState,
    },
};

use crate::{
    state::{
        ClusterSnapshot,
        filters::{NodeSortBy, sort_nodes},
    },
    ui::components::{active_block, default_block, default_theme},
};

/// Renders the nodes table with stateful selection, scrollbar, and theme-aware styling.
pub fn render_nodes(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let theme = default_theme();

    let mut nodes = crate::state::filters::filter_nodes(&snapshot.nodes, query, None, None);
    sort_nodes(&mut nodes, NodeSortBy::Name);

    if snapshot.nodes.is_empty() {
        let widget = Paragraph::new(Line::from(vec![
            Span::styled("  ", theme.inactive_style()),
            Span::styled("No nodes available", theme.inactive_style()),
        ]))
        .block(default_block("Nodes"));
        frame.render_widget(widget, area);
        return;
    }

    if nodes.is_empty() {
        let widget = Paragraph::new(Line::from(vec![
            Span::styled("  No nodes match the search query", theme.inactive_style()),
        ]))
        .block(default_block("Nodes"));
        frame.render_widget(widget, area);
        return;
    }

    let total = nodes.len();
    let selected = selected_idx.min(total.saturating_sub(1));

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("Status", theme.header_style())),
        Cell::from(Span::styled("Role", theme.header_style())),
        Cell::from(Span::styled("CPU", theme.header_style())),
        Cell::from(Span::styled("Memory", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let rows: Vec<Row> = nodes
        .into_iter()
        .enumerate()
        .map(|(idx, node)| {
            let status_style = if node.ready {
                theme.badge_success_style()
            } else {
                theme.badge_error_style()
            };

            let status_text = if node.ready { "● Ready" } else { "✗ NotReady" };

            let mut status_spans = vec![Span::styled(status_text, status_style)];

            if node.memory_pressure {
                status_spans.push(Span::styled("  ⚠ Mem", theme.badge_warning_style()));
            }
            if node.disk_pressure {
                status_spans.push(Span::styled("  ⚠ Disk", theme.badge_warning_style()));
            }

            let row_style = if idx % 2 == 0 {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", node.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Line::from(status_spans)),
                Cell::from(Span::styled(node.role, Style::default().fg(theme.accent2))),
                Cell::from(Span::styled(
                    node.cpu_allocatable.unwrap_or_else(|| "N/A".to_string()),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    node.memory_allocatable.unwrap_or_else(|| "N/A".to_string()),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    format_age(node.created_at),
                    theme.inactive_style(),
                )),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Percentage(26),
        Constraint::Percentage(28),
        Constraint::Percentage(12),
        Constraint::Percentage(12),
        Constraint::Percentage(12),
        Constraint::Percentage(10),
    ];

    let mut table_state = TableState::default().with_selected(Some(selected));

    let title = format!(" 🖥  Nodes ({total}) ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = snapshot.nodes.len();
        active_block(&format!(" 🖥  Nodes ({total} of {all}) [/{query}]"))
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
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
        area.inner(Margin { vertical: 1, horizontal: 0 }),
        &mut scrollbar_state,
    );
}

fn format_age(created_at: Option<chrono::DateTime<Utc>>) -> String {
    let Some(created_at) = created_at else {
        return "N/A".to_string();
    };

    let delta = Utc::now().signed_duration_since(created_at);
    let days = delta.num_days();
    let hours = delta.num_hours() % 24;
    let mins = delta.num_minutes() % 60;

    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{}m", mins.max(0))
    }
}
