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
    app::AppView,
    k8s::dtos::RoleBindingSubject,
    state::ClusterSnapshot,
    ui::{
        cmp_ci,
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices, data_fingerprint},
        format_small_int,
    },
};

pub fn render_cluster_role_bindings(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let query = query.trim();
    let indices = cached_filter_indices(
        AppView::ClusterRoleBindings,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.cluster_role_bindings),
        |q| {
            let mut out: Vec<usize> = cluster
                .cluster_role_bindings
                .iter()
                .enumerate()
                .filter_map(|(idx, rb)| {
                    if q.is_empty() || contains_ci(&rb.name, q) || contains_ci(&rb.role_ref_name, q)
                    {
                        Some(idx)
                    } else {
                        None
                    }
                })
                .collect();
            out.sort_unstable_by(|a, b| {
                cmp_ci(
                    &cluster.cluster_role_bindings[*a].name,
                    &cluster.cluster_role_bindings[*b].name,
                )
            });
            out
        },
    );

    let theme = default_theme();

    if indices.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "  No clusterrolebindings found",
                theme.inactive_style(),
            ))
            .block(default_block("ClusterRoleBindings")),
            area,
        );
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("RoleRef", theme.header_style())),
        Cell::from(Span::styled("Subjects", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let rows: Vec<Row> = indices
        .iter()
        .enumerate()
        .map(|(idx, &rb_idx)| {
            let rb = &cluster.cluster_role_bindings[rb_idx];
            let row_style = if idx % 2 == 0 {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", rb.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    format!("{}/{}", rb.role_ref_kind, rb.role_ref_name),
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(Span::styled(
                    format_small_int(rb.subjects.len() as i64),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(format_age(rb.age), theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(selected));
    let title = format!(" 🔗 ClusterRoleBindings ({total}) ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.cluster_role_bindings.len();
        active_block(&format!(
            " 🔗 ClusterRoleBindings ({total} of {all}) [/{query}]"
        ))
    };

    let table = Table::new(
        rows,
        [
            Constraint::Min(30),
            Constraint::Length(38),
            Constraint::Length(9),
            Constraint::Length(9),
        ],
    )
    .header(header)
    .block(block)
    .row_highlight_style(theme.selection_style())
    .highlight_symbol(theme.highlight_symbol())
    .highlight_spacing(HighlightSpacing::Always);
    frame.render_stateful_widget(table, chunks[0], &mut table_state);

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"))
        .track_symbol(Some("│"))
        .thumb_symbol("█");
    let mut scrollbar_state = ScrollbarState::new(total).position(selected);
    frame.render_stateful_widget(
        scrollbar,
        chunks[0].inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );

    let sel_item = &cluster.cluster_role_bindings[indices[selected]];
    let detail = render_subjects(&sel_item.subjects, &theme);
    frame.render_widget(
        Paragraph::new(detail).block(active_block("Selected Binding Subjects")),
        chunks[1],
    );
}

fn render_subjects(
    subjects: &[RoleBindingSubject],
    theme: &crate::ui::theme::Theme,
) -> Vec<Line<'static>> {
    if subjects.is_empty() {
        return vec![Line::from(Span::styled(
            "  No subjects",
            theme.inactive_style(),
        ))];
    }
    subjects
        .iter()
        .map(|subject| {
            let ns = subject.namespace.as_deref().unwrap_or("—");
            let api_group = subject.api_group.as_deref().unwrap_or("—");
            Line::from(vec![
                Span::styled("  ● ", theme.title_style()),
                Span::styled(
                    format!("{}/{}", subject.kind, subject.name),
                    Style::default().fg(theme.fg),
                ),
                Span::styled(
                    format!("  ns={ns}  apiGroup={api_group}"),
                    theme.inactive_style(),
                ),
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
