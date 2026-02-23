//! CronJobs list rendering.

use chrono::{DateTime, Local, Utc};
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
    state::{ClusterSnapshot, filters::filter_cronjobs},
    ui::components::{active_block, default_block, default_theme},
};

pub fn render_cronjobs(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let theme = default_theme();
    let items = filter_cronjobs(&cluster.cronjobs, query, None);

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("  No cronjobs found", theme.inactive_style()))
                .block(default_block("CronJobs")),
            area,
        );
        return;
    }

    let total = items.len();
    let selected = selected_idx.min(total.saturating_sub(1));

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Schedule", theme.header_style())),
        Cell::from(Span::styled("Last Run", theme.header_style())),
        Cell::from(Span::styled("Next Run", theme.header_style())),
        Cell::from(Span::styled("Active", theme.header_style())),
        Cell::from(Span::styled("Suspend", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(idx, cj)| {
            let suspend_style = if cj.suspend {
                theme.badge_warning_style()
            } else {
                theme.badge_success_style()
            };
            let row_style = if idx % 2 == 0 {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", cj.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    cj.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    cj.schedule.clone(),
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(Span::styled(
                    format_time(cj.last_schedule_time),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    format_time(cj.next_schedule_time),
                    Style::default().fg(theme.info),
                )),
                Cell::from(Span::styled(
                    cj.active_jobs.to_string(),
                    if cj.active_jobs > 0 {
                        Style::default().fg(theme.info)
                    } else {
                        theme.inactive_style()
                    },
                )),
                Cell::from(Span::styled(suspend_label(cj.suspend), suspend_style)),
                Cell::from(Span::styled(format_age(cj.age), theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(selected));

    let title = format!(" 🕐 CronJobs ({total}) ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        active_block(&format!("{title} [/{query}]"))
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Length(16),
            Constraint::Length(16),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(9),
        ],
    )
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

fn suspend_label(suspend: bool) -> &'static str {
    if suspend { "● Paused" } else { "● Active" }
}

fn format_time(ts: Option<DateTime<Utc>>) -> String {
    ts.map(|value| {
        value
            .with_timezone(&Local)
            .format("%m-%d %H:%M")
            .to_string()
    })
    .unwrap_or_else(|| "-".to_string())
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

    #[test]
    fn suspend_label_values() {
        assert_eq!(suspend_label(true), "● Paused");
        assert_eq!(suspend_label(false), "● Active");
    }
}
