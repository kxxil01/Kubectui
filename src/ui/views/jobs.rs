//! Jobs list rendering.

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
    state::{ClusterSnapshot, filters::filter_jobs},
    ui::components::{active_block, default_block, default_theme},
};

pub fn render_jobs(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let theme = default_theme();
    let items = filter_jobs(&cluster.jobs, query, None);

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("  No jobs found", theme.inactive_style()))
                .block(default_block("Jobs")),
            area,
        );
        return;
    }

    let total = items.len();
    let selected = selected_idx.min(total.saturating_sub(1));

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Status", theme.header_style())),
        Cell::from(Span::styled("Completions", theme.header_style())),
        Cell::from(Span::styled("Duration", theme.header_style())),
        Cell::from(Span::styled("Active", theme.header_style())),
        Cell::from(Span::styled("Failed", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(idx, job)| {
            let st = status_style(&job.status, &theme);
            let failed_style = if job.failed_pods > 0 {
                theme.badge_error_style()
            } else {
                theme.inactive_style()
            };
            let row_style = if idx % 2 == 0 {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", job.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    job.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(job.status.clone(), st)),
                Cell::from(Span::styled(
                    job.completions.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    job.duration.clone().unwrap_or_else(|| "-".to_string()),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    job.active_pods.to_string(),
                    Style::default().fg(theme.info),
                )),
                Cell::from(Span::styled(job.failed_pods.to_string(), failed_style)),
                Cell::from(Span::styled(format_age(job.age), theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(selected));

    let title = format!(" ⚙  Jobs ({total}) ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        active_block(&format!("{title} [/{query}]"))
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(22),
            Constraint::Length(16),
            Constraint::Length(11),
            Constraint::Length(13),
            Constraint::Length(11),
            Constraint::Length(8),
            Constraint::Length(8),
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

fn status_style(status: &str, theme: &crate::ui::theme::Theme) -> Style {
    match status {
        "Succeeded" | "Complete" => theme.badge_success_style(),
        "Running" => Style::default().fg(theme.info),
        "Failed" => theme.badge_error_style(),
        _ => theme.badge_warning_style(),
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
    fn status_color_map() {
        let theme = Theme::dark();
        assert_eq!(status_style("Succeeded", &theme).fg, Some(theme.success));
        assert_eq!(status_style("Running", &theme).fg, Some(theme.info));
        assert_eq!(status_style("Failed", &theme).fg, Some(theme.error));
        assert_eq!(status_style("Pending", &theme).fg, Some(theme.warning));
    }
}
