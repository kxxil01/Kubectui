//! PodDisruptionBudgets list rendering.

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
    app::AppView,
    state::ClusterSnapshot,
    ui::{
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices, data_fingerprint},
        format_small_int, table_viewport_rows, table_window,
    },
};

pub fn render_pdbs(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let query = query.trim();
    let indices = cached_filter_indices(
        AppView::PodDisruptionBudgets,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.pod_disruption_budgets),
        |q| {
            if q.is_empty() {
                return (0..cluster.pod_disruption_budgets.len()).collect();
            }
            cluster
                .pod_disruption_budgets
                .iter()
                .enumerate()
                .filter_map(|(idx, pdb)| {
                    if contains_ci(&pdb.name, q)
                        || contains_ci(pdb.min_available.as_deref().unwrap_or_default(), q)
                        || contains_ci(pdb.max_unavailable.as_deref().unwrap_or_default(), q)
                    {
                        Some(idx)
                    } else {
                        None
                    }
                })
                .collect()
        },
    );

    let theme = default_theme();

    if indices.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "  No pod disruption budgets found",
                theme.inactive_style(),
            ))
            .block(default_block("PodDisruptionBudgets")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Policy", theme.header_style())),
        Cell::from(Span::styled("Healthy", theme.header_style())),
        Cell::from(Span::styled("Disruptions", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &pdb_idx)| {
            let idx = window.start + local_idx;
            let pdb = &cluster.pod_disruption_budgets[pdb_idx];
            let disrupt_style = disruption_style(pdb.disruptions_allowed, &theme);
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", pdb.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    pdb.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    pdb.min_available
                        .clone()
                        .or_else(|| pdb.max_unavailable.clone())
                        .unwrap_or_else(|| "-".to_string()),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    format!("{}/{}", pdb.current_healthy, pdb.desired_healthy),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    format_small_int(i64::from(pdb.disruptions_allowed)),
                    disrupt_style,
                )),
                Cell::from(Span::styled(format_age(pdb.age), theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));
    let title = format!(" 🛡️  PodDisruptionBudgets ({total}) ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.pod_disruption_budgets.len();
        active_block(&format!(
            " 🛡️  PodDisruptionBudgets ({total} of {all}) [/{query}]"
        ))
    };

    let table = Table::new(
        rows,
        [
            Constraint::Min(28),
            Constraint::Length(18),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(12),
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
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}

fn disruption_style(disruptions_allowed: i32, theme: &crate::ui::theme::Theme) -> Style {
    if disruptions_allowed > 0 {
        theme.badge_success_style()
    } else {
        theme.badge_warning_style()
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
    fn disruption_style_expected_colors() {
        let theme = Theme::dark();
        assert_eq!(disruption_style(2, &theme).fg, Some(theme.success));
        assert_eq!(disruption_style(0, &theme).fg, Some(theme.warning));
    }
}
