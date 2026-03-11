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
    app::{AppView, WorkloadSortColumn, WorkloadSortState, filtered_workload_indices},
    k8s::dtos::RbacRule,
    state::ClusterSnapshot,
    ui::{
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices_with_variant, data_fingerprint},
        format_age, format_small_int, loading_or_empty_message, responsive_table_widths,
        table_viewport_rows, table_window, workload_sort_header, workload_sort_suffix,
    },
};
use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

// ── ClusterRole derived cell cache ─────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClusterRoleDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct ClusterRoleDerivedCell {
    rules_count: String,
    age: String,
}

type ClusterRoleDerivedCacheValue = Arc<Vec<ClusterRoleDerivedCell>>;
static CLUSTER_ROLE_DERIVED_CACHE: LazyLock<
    Mutex<Option<(ClusterRoleDerivedCacheKey, ClusterRoleDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

fn cached_cluster_role_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> ClusterRoleDerivedCacheValue {
    let key = ClusterRoleDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.cluster_roles, snapshot.snapshot_version),
    };

    if let Ok(cache) = CLUSTER_ROLE_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&role_idx| {
                let role = &snapshot.cluster_roles[role_idx];
                ClusterRoleDerivedCell {
                    rules_count: format_small_int(role.rules.len() as i64).into_owned(),
                    age: format_age(role.age),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = CLUSTER_ROLE_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

// ── ClusterRole rules detail cache ─────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClusterRoleRulesCacheKey {
    theme_index: u8,
    snapshot_version: u64,
    name: String,
}

type ClusterRoleRulesCacheValue = Arc<Vec<Line<'static>>>;
static CLUSTER_ROLE_RULES_CACHE: LazyLock<
    Mutex<Option<(ClusterRoleRulesCacheKey, ClusterRoleRulesCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

pub fn render_cluster_roles(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
    sort: Option<WorkloadSortState>,
) {
    let query = query.trim();
    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::ClusterRoles,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.cluster_roles, cluster.snapshot_version),
        cache_variant,
        |q| {
            filtered_workload_indices(
                &cluster.cluster_roles,
                q,
                sort,
                |role, needle| needle.is_empty() || contains_ci(&role.name, needle),
                |role| role.name.as_str(),
                |_role| "",
                |role| role.age,
            )
        },
    );

    let theme = default_theme();

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::ClusterRoles,
            query,
            "  Loading clusterroles...",
            "  No clusterroles found",
            "  No clusterroles match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("ClusterRoles")),
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
    let name_header = workload_sort_header("Name", sort, WorkloadSortColumn::Name);
    let age_header = workload_sort_header("Age", sort, WorkloadSortColumn::Age);

    let header = Row::new([
        Cell::from(Span::styled(
            format!("  {name_header}"),
            theme.header_style(),
        )),
        Cell::from(Span::styled("Rules", theme.header_style())),
        Cell::from(Span::styled(age_header, theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let derived = cached_cluster_role_derived(cluster, query, &indices);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &role_idx)| {
            let idx = window.start + local_idx;
            let role = &cluster.cluster_roles[role_idx];
            let name_style = Style::default().fg(theme.fg);
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let (rules_count, age): (Cow<'_, str>, Cow<'_, str>) =
                if let Some(cell) = derived.get(idx) {
                    (
                        Cow::Borrowed(cell.rules_count.as_str()),
                        Cow::Borrowed(cell.age.as_str()),
                    )
                } else {
                    (
                        format_small_int(role.rules.len() as i64),
                        Cow::Owned(format_age(role.age)),
                    )
                };
            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled("  ", name_style),
                    Span::styled(role.name.as_str(), name_style),
                ])),
                Cell::from(Span::styled(rules_count, Style::default().fg(theme.accent2))),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));
    let sort_suffix = workload_sort_suffix(sort);
    let title = format!(" 🛡️  ClusterRoles ({total}){sort_suffix} ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.cluster_roles.len();
        active_block(&format!(
            " 🛡️  ClusterRoles ({total} of {all}) [/{query}]{sort_suffix}"
        ))
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Min(36),
                Constraint::Length(8),
                Constraint::Length(9),
            ],
        ),
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

    let sel_item = &cluster.cluster_roles[indices[selected]];
    let detail = cached_rule_lines(
        crate::ui::theme::active_theme_index(),
        cluster.snapshot_version,
        &sel_item.name,
        &sel_item.rules,
        &theme,
    );
    frame.render_widget(
        Paragraph::new((*detail).clone()).block(active_block("Selected ClusterRole Rules")),
        chunks[1],
    );
}

fn cached_rule_lines(
    theme_index: u8,
    snapshot_version: u64,
    name: &str,
    rules: &[RbacRule],
    theme: &crate::ui::theme::Theme,
) -> ClusterRoleRulesCacheValue {
    let key = ClusterRoleRulesCacheKey {
        theme_index,
        snapshot_version,
        name: name.to_string(),
    };

    if let Ok(cache) = CLUSTER_ROLE_RULES_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(render_rule_tree(rules, theme));
    if let Ok(mut cache) = CLUSTER_ROLE_RULES_CACHE.lock() {
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
            Span::styled("    verbs      ", theme.inactive_style()),
            Span::styled(join_or_all(&rule.verbs), Style::default().fg(theme.success)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    apiGroups  ", theme.inactive_style()),
            Span::styled(
                join_or_all(&rule.api_groups),
                Style::default().fg(theme.fg_dim),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    resources  ", theme.inactive_style()),
            Span::styled(
                join_or_all(&rule.resources),
                Style::default().fg(theme.accent2),
            ),
        ]));
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
