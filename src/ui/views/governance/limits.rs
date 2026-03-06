//! LimitRanges list rendering.

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
        format_small_int, loading_or_empty_message, table_viewport_rows, table_window,
    },
};

pub fn render_limit_ranges(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let query = query.trim();
    let indices = cached_filter_indices(
        AppView::LimitRanges,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.limit_ranges, cluster.snapshot_version),
        |q| {
            if q.is_empty() {
                return (0..cluster.limit_ranges.len()).collect();
            }
            cluster
                .limit_ranges
                .iter()
                .enumerate()
                .filter_map(|(idx, lr)| {
                    let type_match = lr.limits.iter().any(|spec| contains_ci(&spec.type_, q));
                    if contains_ci(&lr.name, q) || type_match {
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
        let msg = loading_or_empty_message(
            cluster,
            query,
            "  Loading limit ranges...",
            "  No limit ranges found",
            "  No limit ranges match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("LimitRanges")),
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
        Cell::from(Span::styled("Specs", theme.header_style())),
        Cell::from(Span::styled("Types", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &lr_idx)| {
            let idx = window.start + local_idx;
            let lr = &cluster.limit_ranges[lr_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", lr.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    lr.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    format_small_int(lr.limits.len() as i64),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    limit_types_summary(lr),
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(Span::styled(format_age(lr.age), theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));
    let title = format!(" ⚖️  LimitRanges ({total}) ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.limit_ranges.len();
        active_block(&format!(" ⚖️  LimitRanges ({total} of {all}) [/{query}]"))
    };

    let table = Table::new(
        rows,
        [
            Constraint::Min(28),
            Constraint::Length(18),
            Constraint::Length(8),
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
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}

fn limit_types_summary(lr: &crate::k8s::dtos::LimitRangeInfo) -> String {
    let mut types = lr
        .limits
        .iter()
        .map(|spec| spec.type_.clone())
        .collect::<Vec<_>>();
    types.sort();
    types.dedup();

    if types.is_empty() {
        "-".to_string()
    } else {
        types.join(",")
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
    use crate::k8s::dtos::{LimitRangeInfo, LimitSpec};

    use super::*;

    #[test]
    fn summary_deduplicates_types() {
        let info = LimitRangeInfo {
            limits: vec![
                LimitSpec {
                    type_: "Container".to_string(),
                    ..LimitSpec::default()
                },
                LimitSpec {
                    type_: "Container".to_string(),
                    ..LimitSpec::default()
                },
                LimitSpec {
                    type_: "Pod".to_string(),
                    ..LimitSpec::default()
                },
            ],
            ..LimitRangeInfo::default()
        };

        assert_eq!(limit_types_summary(&info), "Container,Pod");
    }
}
