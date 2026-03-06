//! PriorityClasses list view.

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

pub fn render_priority_classes(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search: &str,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = cached_filter_indices(
        AppView::PriorityClasses,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.priority_classes, cluster.snapshot_version),
        |q| {
            if q.is_empty() {
                return (0..cluster.priority_classes.len()).collect();
            }
            cluster
                .priority_classes
                .iter()
                .enumerate()
                .filter_map(|(idx, priority_class)| {
                    contains_ci(&priority_class.name, q).then_some(idx)
                })
                .collect()
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::PriorityClasses,
            query,
            "  Loading priority classes...",
            "  No priority classes found",
            "  No priority classes match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("PriorityClasses")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let header = Row::new([
        Cell::from(Span::styled("  NAME", theme.header_style())),
        Cell::from(Span::styled("VALUE", theme.header_style())),
        Cell::from(Span::styled("GLOBAL DEFAULT", theme.header_style())),
        Cell::from(Span::styled("DESCRIPTION", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &priority_class_idx)| {
            let idx = window.start + local_idx;
            let priority_class = &cluster.priority_classes[priority_class_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let default_label = if priority_class.global_default {
                "✓"
            } else {
                ""
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", priority_class.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    format_small_int(i64::from(priority_class.value)),
                    Style::default().fg(theme.info),
                )),
                Cell::from(Span::styled(
                    default_label,
                    if priority_class.global_default {
                        Style::default().fg(theme.success)
                    } else {
                        Style::default().fg(theme.muted)
                    },
                )),
                Cell::from(Span::styled(
                    priority_class
                        .description
                        .chars()
                        .take(60)
                        .collect::<String>(),
                    Style::default().fg(theme.fg_dim),
                )),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let title = if query.is_empty() {
        format!(" PriorityClasses ({total}) ")
    } else {
        let all = cluster.priority_classes.len();
        format!(" PriorityClasses ({total} of {all}) [/{query}]")
    };

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(10),
            Constraint::Percentage(15),
            Constraint::Percentage(45),
        ],
    )
    .header(header)
    .block(active_block(&title))
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
