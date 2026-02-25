//! ReplicaSets list rendering.

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
    state::{ClusterSnapshot, filters::filter_replicasets},
    ui::components::{active_block, default_block, default_theme},
};

pub fn render_replicasets(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let theme = default_theme();
    let items = filter_replicasets(&cluster.replicasets, query, None);

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("  No replica sets found", theme.inactive_style()))
                .block(default_block("Replica Sets")),
            area,
        );
        return;
    }

    let total = items.len();
    let selected = selected_idx.min(total.saturating_sub(1));

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Desired", theme.header_style())),
        Cell::from(Span::styled("Ready", theme.header_style())),
        Cell::from(Span::styled("Available", theme.header_style())),
        Cell::from(Span::styled("Image", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(idx, rs)| {
            let ready_style = readiness_style(rs.ready, rs.desired, &theme);
            let row_style = if idx % 2 == 0 {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", rs.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    rs.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    rs.desired.to_string(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(rs.ready.to_string(), ready_style)),
                Cell::from(Span::styled(
                    rs.available.to_string(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    format_image(rs.image.as_deref()),
                    Style::default().fg(theme.muted),
                )),
                Cell::from(Span::styled(format_age(rs.age), theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(selected));

    let title = format!(" Replica Sets ({total}) ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.replicasets.len();
        active_block(&format!(" Replica Sets ({total} of {all}) [/{query}]"))
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(28),
            Constraint::Length(16),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(11),
            Constraint::Min(24),
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

fn readiness_style(ready: i32, desired: i32, theme: &crate::ui::theme::Theme) -> Style {
    if desired > 0 && ready >= desired {
        theme.badge_success_style()
    } else if ready > 0 {
        theme.badge_warning_style()
    } else {
        theme.badge_error_style()
    }
}

fn format_image(image: Option<&str>) -> String {
    let Some(image) = image else {
        return "-".to_string();
    };
    const MAX_LEN: usize = 32;
    if image.chars().count() <= MAX_LEN {
        image.to_string()
    } else {
        format!("{}...", image.chars().take(MAX_LEN.saturating_sub(3)).collect::<String>())
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
    fn readiness_style_maps_to_expected_colors() {
        let theme = Theme::dark();
        assert_eq!(readiness_style(3, 3, &theme).fg, Some(theme.success));
        assert_eq!(readiness_style(1, 3, &theme).fg, Some(theme.warning));
        assert_eq!(readiness_style(0, 3, &theme).fg, Some(theme.error));
    }
}
