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
        cmp_ci,
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices, data_fingerprint},
        format_small_int,
    },
};

pub fn render_service_accounts(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let query = query.trim();
    let indices = cached_filter_indices(
        AppView::ServiceAccounts,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.service_accounts),
        |q| {
            let mut out: Vec<usize> = cluster
                .service_accounts
                .iter()
                .enumerate()
                .filter_map(|(idx, sa)| {
                    if q.is_empty() || contains_ci(&sa.name, q) || contains_ci(&sa.namespace, q) {
                        Some(idx)
                    } else {
                        None
                    }
                })
                .collect();
            out.sort_unstable_by(|a, b| {
                let left = &cluster.service_accounts[*a];
                let right = &cluster.service_accounts[*b];
                let ns_order = cmp_ci(&left.namespace, &right.namespace);
                if ns_order == std::cmp::Ordering::Equal {
                    cmp_ci(&left.name, &right.name)
                } else {
                    ns_order
                }
            });
            out
        },
    );

    let theme = default_theme();

    if indices.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "  No serviceaccounts found",
                theme.inactive_style(),
            ))
            .block(default_block("ServiceAccounts")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Secrets", theme.header_style())),
        Cell::from(Span::styled("PullSecrets", theme.header_style())),
        Cell::from(Span::styled("Automount", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let rows: Vec<Row> = indices
        .iter()
        .enumerate()
        .map(|(idx, &sa_idx)| {
            let sa = &cluster.service_accounts[sa_idx];
            let row_style = if idx % 2 == 0 {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let automount_style = match sa.automount_service_account_token {
                Some(true) => theme.badge_success_style(),
                Some(false) => theme.badge_warning_style(),
                None => theme.inactive_style(),
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", sa.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    sa.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    format_small_int(sa.secrets_count as i64),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    format_small_int(sa.image_pull_secrets_count as i64),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    match sa.automount_service_account_token {
                        Some(true) => "true",
                        Some(false) => "false",
                        None => "—",
                    },
                    automount_style,
                )),
                Cell::from(Span::styled(format_age(sa.age), theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(selected));
    let title = format!(" 🔑 ServiceAccounts ({total}) ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.service_accounts.len();
        active_block(&format!(
            " 🔑 ServiceAccounts ({total} of {all}) [/{query}]"
        ))
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(26),
            Constraint::Length(18),
            Constraint::Length(9),
            Constraint::Length(13),
            Constraint::Length(11),
            Constraint::Fill(1),
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
