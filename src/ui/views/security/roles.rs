use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Cell, HighlightSpacing, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState,
    },
};

use crate::{
    app::AppView,
    k8s::dtos::RbacRule,
    state::ClusterSnapshot,
    ui::{
        cmp_ci,
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices, data_fingerprint},
        format_small_int, loading_or_empty_message, table_viewport_rows, table_window,
    },
};
use std::sync::{Arc, LazyLock, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
struct RoleRulesCacheKey {
    theme_index: u8,
    snapshot_version: u64,
    namespace: String,
    name: String,
}

type RoleRulesCacheValue = Arc<Vec<Line<'static>>>;
static ROLE_RULES_CACHE: LazyLock<Mutex<Option<(RoleRulesCacheKey, RoleRulesCacheValue)>>> =
    LazyLock::new(|| Mutex::new(None));

pub fn render_roles(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let query = query.trim();
    let indices = cached_filter_indices(
        AppView::Roles,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.roles, cluster.snapshot_version),
        |q| {
            let mut out: Vec<usize> = cluster
                .roles
                .iter()
                .enumerate()
                .filter_map(|(idx, role)| {
                    if q.is_empty() || contains_ci(&role.name, q) || contains_ci(&role.namespace, q)
                    {
                        Some(idx)
                    } else {
                        None
                    }
                })
                .collect();
            out.sort_unstable_by(|a, b| {
                let left = &cluster.roles[*a];
                let right = &cluster.roles[*b];
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
        let msg = loading_or_empty_message(
            cluster,
            AppView::Roles,
            query,
            "  Loading roles...",
            "  No roles found",
            "  No roles match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style())).block(default_block("Roles")),
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
    let window = table_window(total, selected, table_viewport_rows(chunks[0]));

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Rules", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &role_idx)| {
            let idx = window.start + local_idx;
            let role = &cluster.roles[role_idx];
            let name_style = Style::default().fg(theme.fg);
            let dim_style = Style::default().fg(theme.fg_dim);
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled("  ", name_style),
                    Span::styled(role.name.as_str(), name_style),
                ])),
                Cell::from(Span::styled(role.namespace.as_str(), dim_style)),
                Cell::from(Span::styled(
                    format_small_int(role.rules.len() as i64),
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(Span::styled(format_age(role.age), theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));
    let title = format!(" 🛡️  Roles ({total}) ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.roles.len();
        active_block(&format!(" 🛡️  Roles ({total} of {all}) [/{query}]"))
    };

    let table = Table::new(
        rows,
        [
            Constraint::Min(28),
            Constraint::Length(18),
            Constraint::Length(8),
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

    let sel_item = &cluster.roles[indices[selected]];
    let detail = cached_rule_lines(
        crate::ui::theme::active_theme_index(),
        cluster.snapshot_version,
        &sel_item.namespace,
        &sel_item.name,
        &sel_item.rules,
        &theme,
    );
    frame.render_widget(
        Paragraph::new((*detail).clone()).block(active_block("Selected Role Rules")),
        chunks[1],
    );
}

fn cached_rule_lines(
    theme_index: u8,
    snapshot_version: u64,
    namespace: &str,
    name: &str,
    rules: &[RbacRule],
    theme: &crate::ui::theme::Theme,
) -> RoleRulesCacheValue {
    let key = RoleRulesCacheKey {
        theme_index,
        snapshot_version,
        namespace: namespace.to_string(),
        name: name.to_string(),
    };

    if let Ok(cache) = ROLE_RULES_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(render_rule_tree(rules, theme));
    if let Ok(mut cache) = ROLE_RULES_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }
    built
}

fn render_rule_tree(rules: &[RbacRule], theme: &crate::ui::theme::Theme) -> Vec<Line<'static>> {
    if rules.is_empty() {
        return vec![Line::from(Span::styled(
            "  No rules defined",
            theme.inactive_style(),
        ))];
    }

    let mut lines = Vec::new();
    for (idx, rule) in rules.iter().enumerate() {
        lines.push(Line::from(Span::styled(
            format!("  Rule {}", idx + 1),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled("    verbs       ", theme.inactive_style()),
            Span::styled(join_or_all(&rule.verbs), Style::default().fg(theme.success)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    apiGroups   ", theme.inactive_style()),
            Span::styled(
                join_or_all(&rule.api_groups),
                Style::default().fg(theme.fg_dim),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    resources   ", theme.inactive_style()),
            Span::styled(
                join_or_all(&rule.resources),
                Style::default().fg(theme.accent2),
            ),
        ]));
        if !rule.resource_names.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("    names      ", theme.inactive_style()),
                Span::styled(
                    rule.resource_names.join(", "),
                    Style::default().fg(theme.fg_dim),
                ),
            ]));
        }
        if !rule.non_resource_urls.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("    urls       ", theme.inactive_style()),
                Span::styled(
                    rule.non_resource_urls.join(", "),
                    Style::default().fg(theme.muted),
                ),
            ]));
        }
    }
    lines
}

fn join_or_all(items: &[String]) -> String {
    if items.is_empty() {
        "*".to_string()
    } else {
        items.join(", ")
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
    fn rule_tree_renders_rules_and_fields() {
        let theme = Theme::dark();
        let lines = render_rule_tree(
            &[RbacRule {
                verbs: vec!["get".to_string()],
                api_groups: vec!["apps".to_string()],
                resources: vec!["deployments".to_string()],
                resource_names: vec!["api".to_string()],
                non_resource_urls: vec![],
            }],
            &theme,
        );

        let content = lines
            .into_iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(content.contains("Rule 1"));
        assert!(content.contains("get"));
        assert!(content.contains("deployments"));
    }
}
