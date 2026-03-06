//! Nodes list renderer.

use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

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
    app::AppView,
    state::ClusterSnapshot,
    ui::{
        cmp_ci,
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices, data_fingerprint},
        loading_or_empty_message, loading_or_empty_message_no_search, table_viewport_rows,
        table_window,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct NodeDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
    minute_bucket: i64,
}

#[derive(Debug, Clone)]
struct NodeDerivedCell {
    age: String,
}

type NodeDerivedCacheValue = Arc<Vec<NodeDerivedCell>>;
static NODE_DERIVED_CACHE: LazyLock<Mutex<Option<(NodeDerivedCacheKey, NodeDerivedCacheValue)>>> =
    LazyLock::new(|| Mutex::new(None));

/// Renders the nodes table with stateful selection, scrollbar, and theme-aware styling.
pub fn render_nodes(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let theme = default_theme();
    let query = query.trim();

    if snapshot.nodes.is_empty() {
        let msg = loading_or_empty_message_no_search(
            snapshot,
            AppView::Nodes,
            "  Loading nodes...",
            "  No nodes available",
        );
        let widget = Paragraph::new(Line::from(vec![
            Span::styled("  ", theme.inactive_style()),
            Span::styled(msg, theme.inactive_style()),
        ]))
        .block(default_block("Nodes"));
        frame.render_widget(widget, area);
        return;
    }

    let indices = cached_filter_indices(
        AppView::Nodes,
        query,
        snapshot.snapshot_version,
        data_fingerprint(&snapshot.nodes, snapshot.snapshot_version),
        |q| {
            let mut out: Vec<usize> = if q.is_empty() {
                (0..snapshot.nodes.len()).collect()
            } else {
                snapshot
                    .nodes
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, node)| contains_ci(&node.name, q).then_some(idx))
                    .collect()
            };
            out.sort_unstable_by(|a, b| cmp_ci(&snapshot.nodes[*a].name, &snapshot.nodes[*b].name));
            out
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            snapshot,
            AppView::Nodes,
            query,
            "  Loading nodes...",
            "  No nodes available",
            "  No nodes match the search query",
        );
        let widget = Paragraph::new(Line::from(vec![Span::styled(msg, theme.inactive_style())]))
            .block(default_block("Nodes"));
        frame.render_widget(widget, area);
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

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
    let name_style = Style::default().fg(theme.fg);
    let accent_style = Style::default().fg(theme.accent2);
    let dim_style = Style::default().fg(theme.fg_dim);
    let warn_style = theme.badge_warning_style();
    let now_unix = Utc::now().timestamp();
    let derived = cached_node_derived(snapshot, query, indices.as_ref(), now_unix);

    let mut rows: Vec<Row> = Vec::with_capacity(window.end.saturating_sub(window.start));
    for (local_idx, &node_idx) in indices[window.start..window.end].iter().enumerate() {
        let idx = window.start + local_idx;
        let node = &snapshot.nodes[node_idx];
        let age = derived
            .get(idx)
            .map(|cell| Cow::Borrowed(cell.age.as_str()))
            .unwrap_or_else(|| Cow::Owned(format_age(node.created_at, now_unix)));
        let status_style = if node.ready {
            theme.badge_success_style()
        } else {
            theme.badge_error_style()
        };
        let status_text = if node.ready {
            "● Ready"
        } else {
            "✗ NotReady"
        };

        let mut status_spans = Vec::with_capacity(3);
        status_spans.push(Span::styled(status_text, status_style));
        if node.memory_pressure {
            status_spans.push(Span::styled("  ⚠ Mem", warn_style));
        }
        if node.disk_pressure {
            status_spans.push(Span::styled("  ⚠ Disk", warn_style));
        }

        let row_style = if idx.is_multiple_of(2) {
            Style::default().bg(theme.bg)
        } else {
            theme.row_alt_style()
        };

        rows.push(
            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled("  ", name_style),
                    Span::styled(node.name.as_str(), name_style),
                ])),
                Cell::from(Line::from(status_spans)),
                Cell::from(Span::styled(node.role.as_str(), accent_style)),
                Cell::from(Span::styled(
                    node.cpu_allocatable.as_deref().unwrap_or("N/A"),
                    dim_style,
                )),
                Cell::from(Span::styled(
                    node.memory_allocatable.as_deref().unwrap_or("N/A"),
                    dim_style,
                )),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style),
        );
    }

    let widths = [
        Constraint::Percentage(26),
        Constraint::Percentage(28),
        Constraint::Percentage(12),
        Constraint::Percentage(12),
        Constraint::Percentage(12),
        Constraint::Percentage(10),
    ];

    let mut table_state = TableState::default().with_selected(Some(window.selected));

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
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}

fn cached_node_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    now_unix: i64,
) -> NodeDerivedCacheValue {
    let key = NodeDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.nodes, snapshot.snapshot_version),
        minute_bucket: now_unix / 60,
    };

    if let Ok(cache) = NODE_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&node_idx| {
                let node = &snapshot.nodes[node_idx];
                NodeDerivedCell {
                    age: format_age(node.created_at, now_unix),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = NODE_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

#[inline]
fn format_age(created_at: Option<chrono::DateTime<Utc>>, now_unix: i64) -> String {
    let Some(created_at) = created_at else {
        return "N/A".to_string();
    };
    let age_secs = now_unix.saturating_sub(created_at.timestamp());
    let days = age_secs / 86_400;
    let hours = (age_secs % 86_400) / 3_600;
    let mins = (age_secs % 3_600) / 60;

    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{}m", mins.max(0))
    }
}
