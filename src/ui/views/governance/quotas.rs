//! ResourceQuotas list rendering.

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
    state::{ClusterSnapshot, filters::filter_resource_quotas},
    ui::components::{active_block, default_block, default_theme},
};

pub fn render_resource_quotas(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let items = filter_resource_quotas(&cluster.resource_quotas, query, None);

    let theme = default_theme();

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("  No resource quotas found", theme.inactive_style()))
                .block(default_block("ResourceQuotas")),
            area,
        );
        return;
    }

    let total = items.len();
    let selected = selected_idx.min(total.saturating_sub(1));

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Tracked", theme.header_style())),
        Cell::from(Span::styled("Max Used", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ]).height(1).style(theme.header_style());

    let rows: Vec<Row> = items.iter().enumerate().map(|(idx, rq)| {
        let (tracked, max_pct) = quota_summary(rq);
        let pct_style = usage_style(max_pct, &theme);
        let row_style = if idx % 2 == 0 { Style::default().bg(theme.bg) } else { theme.row_alt_style() };
        Row::new(vec![
            Cell::from(Span::styled(format!("  {}", rq.name), Style::default().fg(theme.fg))),
            Cell::from(Span::styled(rq.namespace.clone(), Style::default().fg(theme.fg_dim))),
            Cell::from(Span::styled(tracked.to_string(), Style::default().fg(theme.fg_dim))),
            Cell::from(Span::styled(format!("{max_pct:.0}%"), pct_style)),
            Cell::from(Span::styled(format_age(rq.age), theme.inactive_style())),
        ]).style(row_style)
    }).collect();

    let mut table_state = TableState::default().with_selected(Some(selected));
    let title = format!(" 📊 ResourceQuotas ({total}) ");
    let block = if query.is_empty() { active_block(&title) } else { active_block(&format!("{title} [/{query}]")) };

    let table = Table::new(rows, [Constraint::Min(28), Constraint::Length(18), Constraint::Length(10), Constraint::Length(10), Constraint::Length(9)])
        .header(header).block(block)
        .row_highlight_style(theme.selection_style())
        .highlight_symbol(theme.highlight_symbol())
        .highlight_spacing(HighlightSpacing::Always);
    frame.render_stateful_widget(table, area, &mut table_state);

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲")).end_symbol(Some("▼")).track_symbol(Some("│")).thumb_symbol("█");
    let mut scrollbar_state = ScrollbarState::new(total).position(selected);
    frame.render_stateful_widget(scrollbar, area.inner(Margin { vertical: 1, horizontal: 0 }), &mut scrollbar_state);
}

fn quota_summary(rq: &crate::k8s::dtos::ResourceQuotaInfo) -> (usize, f64) {
    let tracked = rq.percent_used.len();
    let max_pct = rq
        .percent_used
        .values()
        .fold(0.0_f64, |acc, value| acc.max(*value));
    (tracked, max_pct)
}

fn usage_style(percent: f64, theme: &crate::ui::theme::Theme) -> Style {
    if percent >= 90.0 {
        theme.badge_error_style()
    } else if percent >= 70.0 {
        theme.badge_warning_style()
    } else {
        theme.badge_success_style()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Theme;

    #[test]
    fn usage_style_thresholds() {
        let theme = Theme::dark();
        assert_eq!(usage_style(35.0, &theme).fg, Some(theme.success));
        assert_eq!(usage_style(75.0, &theme).fg, Some(theme.warning));
        assert_eq!(usage_style(95.0, &theme).fg, Some(theme.error));
    }
}
