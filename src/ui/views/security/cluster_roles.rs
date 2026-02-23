use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Cell, HighlightSpacing, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState,
    },
};

use crate::{
    k8s::dtos::RbacRule,
    state::ClusterSnapshot,
    ui::components::{active_block, default_block, default_theme},
};

pub fn render_cluster_roles(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let query = query.trim().to_ascii_lowercase();
    let mut items: Vec<_> = cluster
        .cluster_roles
        .iter()
        .filter(|role| query.is_empty() || role.name.to_ascii_lowercase().contains(&query))
        .collect();
    items.sort_by_key(|r| r.name.to_ascii_lowercase());

    let theme = default_theme();

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("  No clusterroles found", theme.inactive_style()))
                .block(default_block("ClusterRoles")),
            area,
        );
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);

    let total = items.len();
    let selected = selected_idx.min(total.saturating_sub(1));

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("Rules", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ]).height(1).style(theme.header_style());

    let rows: Vec<Row> = items.iter().enumerate().map(|(idx, role)| {
        let row_style = if idx % 2 == 0 { Style::default().bg(theme.bg) } else { theme.row_alt_style() };
        Row::new(vec![
            Cell::from(Span::styled(format!("  {}", role.name), Style::default().fg(theme.fg))),
            Cell::from(Span::styled(role.rules.len().to_string(), Style::default().fg(theme.accent2))),
            Cell::from(Span::styled(format_age(role.age), theme.inactive_style())),
        ]).style(row_style)
    }).collect();

    let mut table_state = TableState::default().with_selected(Some(selected));
    let title = format!(" 🛡️  ClusterRoles ({total}) ");
    let block = if query.is_empty() { active_block(&title) } else { active_block(&format!("{title} [/{query}]")) };

    let table = Table::new(rows, [Constraint::Min(36), Constraint::Length(8), Constraint::Length(9)])
        .header(header).block(block)
        .row_highlight_style(theme.selection_style())
        .highlight_symbol(theme.highlight_symbol())
        .highlight_spacing(HighlightSpacing::Always);
    frame.render_stateful_widget(table, chunks[0], &mut table_state);

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲")).end_symbol(Some("▼")).track_symbol(Some("│")).thumb_symbol("█");
    let mut scrollbar_state = ScrollbarState::new(total).position(selected);
    frame.render_stateful_widget(scrollbar, chunks[0].inner(Margin { vertical: 1, horizontal: 0 }), &mut scrollbar_state);

    let sel_item = items[selected];
    let detail = render_rule_tree(&sel_item.rules, &theme);
    frame.render_widget(
        Paragraph::new(detail).block(active_block("Selected ClusterRole Rules")),
        chunks[1],
    );
}

fn render_rule_tree(rules: &[RbacRule], theme: &crate::ui::theme::Theme) -> Vec<Line<'static>> {
    if rules.is_empty() {
        return vec![Line::from(Span::styled("  No rules defined", theme.inactive_style()))];
    }
    let mut lines = Vec::new();
    for (idx, rule) in rules.iter().enumerate() {
        lines.push(Line::from(Span::styled(
            format!("  Rule {}", idx + 1),
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled("    verbs      ", theme.inactive_style()),
            Span::styled(join_or_all(&rule.verbs), Style::default().fg(theme.success)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    apiGroups  ", theme.inactive_style()),
            Span::styled(join_or_all(&rule.api_groups), Style::default().fg(theme.fg_dim)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    resources  ", theme.inactive_style()),
            Span::styled(join_or_all(&rule.resources), Style::default().fg(theme.accent2)),
        ]));
    }
    lines
}

fn join_or_all(items: &[String]) -> String {
    if items.is_empty() {
        "*".to_string()
    } else {
        items.join(", ")
    }
}

fn format_age(age: Option<std::time::Duration>) -> String {
    let Some(age) = age else {
        return "-".to_string();
    };
    let secs = age.as_secs();
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}
