use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{
        Cell, HighlightSpacing, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState,
    },
};

use crate::{
    k8s::dtos::RoleBindingSubject,
    state::ClusterSnapshot,
    ui::components::{active_block, default_block, default_theme},
};

pub fn render_role_bindings(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let query = query.trim().to_ascii_lowercase();
    let mut items: Vec<_> = cluster
        .role_bindings
        .iter()
        .filter(|rb| {
            query.is_empty()
                || rb.name.to_ascii_lowercase().contains(&query)
                || rb.namespace.to_ascii_lowercase().contains(&query)
                || rb.role_ref_name.to_ascii_lowercase().contains(&query)
        })
        .collect();
    items.sort_by_key(|rb| {
        (
            rb.namespace.to_ascii_lowercase(),
            rb.name.to_ascii_lowercase(),
        )
    });

    let theme = default_theme();

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("  No rolebindings found", theme.inactive_style()))
                .block(default_block("RoleBindings")),
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
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("RoleRef", theme.header_style())),
        Cell::from(Span::styled("Subjects", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ]).height(1).style(theme.header_style());

    let rows: Vec<Row> = items.iter().enumerate().map(|(idx, rb)| {
        let row_style = if idx % 2 == 0 { Style::default().bg(theme.bg) } else { theme.row_alt_style() };
        Row::new(vec![
            Cell::from(Span::styled(format!("  {}", rb.name), Style::default().fg(theme.fg))),
            Cell::from(Span::styled(rb.namespace.clone(), Style::default().fg(theme.fg_dim))),
            Cell::from(Span::styled(format!("{}/{}", rb.role_ref_kind, rb.role_ref_name), Style::default().fg(theme.accent2))),
            Cell::from(Span::styled(rb.subjects.len().to_string(), Style::default().fg(theme.fg_dim))),
            Cell::from(Span::styled(format_age(rb.age), theme.inactive_style())),
        ]).style(row_style)
    }).collect();

    let mut table_state = TableState::default().with_selected(Some(selected));
    let title = format!(" 🔗 RoleBindings ({total}) ");
    let block = if query.is_empty() { active_block(&title) } else { active_block(&format!("{title} [/{query}]")) };

    let table = Table::new(rows, [Constraint::Min(24), Constraint::Length(16), Constraint::Length(34), Constraint::Length(9), Constraint::Length(9)])
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
    let detail = render_subjects(&sel_item.subjects, &theme);
    frame.render_widget(
        Paragraph::new(detail).block(active_block("Selected Binding Subjects")),
        chunks[1],
    );
}

fn render_subjects(subjects: &[RoleBindingSubject], theme: &crate::ui::theme::Theme) -> Vec<Line<'static>> {
    if subjects.is_empty() {
        return vec![Line::from(Span::styled("  No subjects", theme.inactive_style()))];
    }
    subjects
        .iter()
        .map(|subject| {
            let ns = subject.namespace.as_deref().unwrap_or("—");
            let api_group = subject.api_group.as_deref().unwrap_or("—");
            Line::from(vec![
                Span::styled("  ● ", theme.title_style()),
                Span::styled(format!("{}/{}", subject.kind, subject.name), Style::default().fg(theme.fg)),
                Span::styled(format!("  ns={ns}  apiGroup={api_group}"), theme.inactive_style()),
            ])
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Theme;

    #[test]
    fn subjects_render_as_human_readable_lines() {
        let theme = Theme::dark();
        let lines = render_subjects(&[RoleBindingSubject {
            kind: "ServiceAccount".to_string(),
            name: "builder".to_string(),
            namespace: Some("default".to_string()),
            api_group: None,
        }], &theme);

        let text = lines[0].to_string();
        assert!(text.contains("ServiceAccount/builder"));
        assert!(text.contains("ns=default"));
    }
}
