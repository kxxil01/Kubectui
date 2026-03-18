use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row},
};

use super::join_or_all;

use crate::{
    app::{AppView, ResourceRef, WorkloadSortColumn, WorkloadSortState},
    bookmarks::BookmarkEntry,
    k8s::dtos::RbacRule,
    state::ClusterSnapshot,
    ui::{
        TableFrame, bookmarked_name_cell,
        components::{content_block, default_theme},
        filter_cache::{cached_filter_indices_with_variant, data_fingerprint},
        format_age, format_small_int, render_centered_message, render_table_frame,
        resource_table_title, sort_header_cell, table_viewport_rows, table_window,
        views::filtering::filtered_cluster_role_indices,
        workload_sort_suffix,
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
    variant: u64,
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
    variant: u64,
) -> ClusterRoleDerivedCacheValue {
    let key = ClusterRoleDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.cluster_roles, snapshot.snapshot_version),
        variant,
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

#[allow(clippy::too_many_arguments)]
pub fn render_cluster_roles(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    query: &str,
    sort: Option<WorkloadSortState>,
    focused: bool,
) {
    let query = query.trim();
    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::ClusterRoles,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.cluster_roles, cluster.snapshot_version),
        cache_variant,
        |q| filtered_cluster_role_indices(&cluster.cluster_roles, q, sort),
    );

    let theme = default_theme();

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::ClusterRoles,
            query,
            "ClusterRoles",
            "Loading clusterroles...",
            "No clusterroles found",
            "No clusterroles match the search query",
            focused,
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
        sort_header_cell("Name", sort, WorkloadSortColumn::Name, &theme, true),
        Cell::from(Span::styled("Rules", theme.header_style())),
        sort_header_cell("Age", sort, WorkloadSortColumn::Age, &theme, false),
    ])
    .height(1)
    .style(theme.header_style());

    let derived = cached_cluster_role_derived(cluster, query, &indices, cache_variant);

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
                bookmarked_name_cell(
                    &ResourceRef::ClusterRole(role.name.clone()),
                    bookmarks,
                    role.name.as_str(),
                    name_style,
                    &theme,
                ),
                Cell::from(Span::styled(
                    rules_count,
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let sort_suffix = workload_sort_suffix(sort);
    let title = resource_table_title(
        "🛡️ ",
        "ClusterRoles",
        total,
        cluster.cluster_roles.len(),
        query,
        &sort_suffix,
    );
    let widths = [
        Constraint::Min(36),
        Constraint::Length(8),
        Constraint::Length(9),
    ];
    render_table_frame(
        frame,
        chunks[0],
        TableFrame {
            rows,
            header,
            widths: &widths,
            title: &title,
            focused,
            window,
            total,
            selected,
        },
        &theme,
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
        Paragraph::new((*detail).clone())
            .block(content_block("Selected ClusterRole Rules", focused)),
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
