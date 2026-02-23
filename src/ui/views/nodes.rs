//! Nodes list renderer.

use chrono::Utc;
use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Line, Style},
    text::Span,
    widgets::{Cell, Paragraph, Row, Table, TableState},
};

use crate::state::{
    ClusterSnapshot,
    filters::{NodeSortBy, sort_nodes},
};

/// Renders the nodes table with default sorting and search filtering.
pub fn render_nodes(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let mut nodes = crate::state::filters::filter_nodes(&snapshot.nodes, query, None, None);
    sort_nodes(&mut nodes, NodeSortBy::Name);

    if snapshot.nodes.is_empty() {
        let widget = Paragraph::new("No nodes available")
            .block(crate::ui::components::default_block("Nodes"));
        frame.render_widget(widget, area);
        return;
    }

    if nodes.is_empty() {
        let widget =
            Paragraph::new("No nodes found").block(crate::ui::components::default_block("Nodes"));
        frame.render_widget(widget, area);
        return;
    }

    let header = Row::new([
        Cell::from("Name"),
        Cell::from("Status"),
        Cell::from("Role"),
        Cell::from("CPU"),
        Cell::from("Memory"),
        Cell::from("Age"),
    ])
    .style(Style::default().fg(Color::Cyan).bold());

    let selected = selected_idx.min(nodes.len().saturating_sub(1));

    let rows = nodes.into_iter().map(|node| {
        let status_style = if node.ready {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Red)
        };

        let mut status_spans = vec![if node.ready {
            Span::styled("Ready ✓", status_style)
        } else {
            Span::styled("NotReady ✗", status_style)
        }];

        if node.memory_pressure {
            status_spans.push(Span::raw(" "));
            status_spans.push(Span::styled(
                "MemoryPressure ⚠",
                Style::default().fg(Color::Yellow),
            ));
        }

        if node.disk_pressure {
            status_spans.push(Span::raw(" "));
            status_spans.push(Span::styled(
                "DiskPressure ⚠",
                Style::default().fg(Color::Yellow),
            ));
        }

        Row::new(vec![
            Cell::from(node.name),
            Cell::from(Line::from(status_spans)),
            Cell::from(node.role),
            Cell::from(node.cpu_allocatable.unwrap_or_else(|| "N/A".to_string())),
            Cell::from(node.memory_allocatable.unwrap_or_else(|| "N/A".to_string())),
            Cell::from(format_age(node.created_at)),
        ])
    });

    let widths = [
        Constraint::Percentage(24),
        Constraint::Percentage(32),
        Constraint::Percentage(12),
        Constraint::Percentage(10),
        Constraint::Percentage(12),
        Constraint::Percentage(10),
    ];

    let mut table_state = TableState::default().with_selected(Some(selected));

    let table = Table::new(rows, widths)
        .header(header)
        .block(crate::ui::components::default_block("Nodes"))
        .row_highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(table, area, &mut table_state);
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
