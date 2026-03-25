//! Rendering for the Relations workbench tab.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::k8s::relationships::{FlatNode, RelationKind, flatten_tree};
use crate::ui::theme::Theme;
use crate::workbench::RelationsTabState;

pub struct RelationTreeView<'a> {
    pub tree: &'a [crate::k8s::relationships::RelationNode],
    pub expanded: &'a std::collections::HashSet<usize>,
    pub cursor: usize,
    pub loading: bool,
    pub error: Option<&'a str>,
    pub loading_message: &'a str,
    pub empty_message: &'a str,
}

pub fn render_relations_tab(frame: &mut Frame, area: Rect, tab: &RelationsTabState, theme: &Theme) {
    render_relation_tree(
        frame,
        area,
        RelationTreeView {
            tree: &tab.tree,
            expanded: &tab.expanded,
            cursor: tab.cursor,
            loading: tab.loading,
            error: tab.error.as_deref(),
            loading_message: "Loading relationships...",
            empty_message: "No relationships found.",
        },
        theme,
    );
}

pub fn render_relation_tree(
    frame: &mut Frame,
    area: Rect,
    view: RelationTreeView<'_>,
    theme: &Theme,
) {
    if view.loading {
        let text = Paragraph::new(view.loading_message).style(Style::default().fg(theme.fg_dim));
        frame.render_widget(text, area);
        return;
    }

    if let Some(err) = view.error {
        let text = Paragraph::new(format!("Error: {err}")).style(Style::default().fg(theme.error));
        frame.render_widget(text, area);
        return;
    }

    if view.tree.is_empty() {
        let text = Paragraph::new(view.empty_message).style(Style::default().fg(theme.fg_dim));
        frame.render_widget(text, area);
        return;
    }

    let flat = flatten_tree(view.tree, view.expanded);
    let visible_height = area.height as usize;
    let scroll_offset = if flat.is_empty() {
        0
    } else {
        let cursor = view.cursor.min(flat.len().saturating_sub(1));
        if cursor < visible_height / 2 {
            0
        } else {
            cursor.saturating_sub(visible_height / 2)
        }
    };

    let lines = flat
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(idx, node)| render_flat_node(node, idx == view.cursor, theme))
        .collect::<Vec<_>>();

    frame.render_widget(Paragraph::new(lines), area);
}

fn render_flat_node(node: &FlatNode, is_cursor: bool, theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();

    if node.relation == RelationKind::SectionHeader {
        // Section header: "── Owner Chain ──────────"
        let header = format!("── {} ", node.label);
        let padding = "─".repeat(60usize.saturating_sub(header.len()));
        spans.push(Span::styled(
            format!("{header}{padding}"),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        // Indent with tree connectors
        for &parent_last in &node.parent_is_last {
            if parent_last {
                spans.push(Span::raw("  "));
            } else {
                spans.push(Span::styled("│ ", Style::default().fg(theme.fg_dim)));
            }
        }

        // Connector
        if node.depth > 0 {
            let connector = if node.is_last_child { "└ " } else { "├ " };
            spans.push(Span::styled(connector, Style::default().fg(theme.fg_dim)));
        }

        // Expand/collapse marker (or alignment padding for leaves)
        if node.has_children {
            let marker = if node.expanded { "▼ " } else { "▶ " };
            spans.push(Span::styled(marker, Style::default().fg(theme.fg_dim)));
        } else {
            spans.push(Span::raw("  "));
        }

        // Kind + name
        let (kind_part, name_part) = node.label.split_once(' ').unwrap_or(("", &node.label));

        let kind_style = if node.not_found {
            Style::default().fg(theme.fg_dim)
        } else {
            Style::default().fg(theme.accent)
        };
        spans.push(Span::styled(format!("{kind_part} "), kind_style));

        let name_style = if node.not_found {
            Style::default().fg(theme.fg_dim)
        } else {
            Style::default().fg(theme.fg)
        };
        spans.push(Span::styled(name_part.to_string(), name_style));

        // Namespace (dimmed)
        if let Some(ns) = &node.namespace {
            spans.push(Span::styled(
                format!(" {ns}"),
                Style::default().fg(theme.fg_dim),
            ));
        }

        // Status
        if let Some(status) = &node.status {
            let status_color = match status.as_str() {
                "Running" | "Ready" | "Bound" | "Active" => theme.success,
                "Pending" | "Waiting" | "Terminating" => theme.warning,
                "Failed" | "Error" | "CrashLoopBackOff" => theme.error,
                _ => theme.fg_dim,
            };
            spans.push(Span::styled(
                format!(" {status}"),
                Style::default().fg(status_color),
            ));
        }

        if node.not_found {
            spans.push(Span::styled(
                " (not found)",
                Style::default().fg(theme.fg_dim),
            ));
        }
    }

    let mut line = Line::from(spans);
    if is_cursor {
        line = line.style(
            Style::default()
                .bg(theme.selection_bg)
                .fg(theme.selection_fg),
        );
    }
    line
}
