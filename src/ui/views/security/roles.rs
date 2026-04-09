use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row},
};

use super::{join_or_all, split_primary_detail};

const ROLE_NARROW_WIDTH: u16 = 88;

fn role_widths(area: Rect) -> [Constraint; 4] {
    if area.width < ROLE_NARROW_WIDTH {
        [
            Constraint::Min(18),
            Constraint::Length(14),
            Constraint::Length(7),
            Constraint::Length(8),
        ]
    } else {
        [
            Constraint::Min(28),
            Constraint::Length(18),
            Constraint::Length(8),
            Constraint::Length(9),
        ]
    }
}

use crate::{
    app::{AppView, ResourceRef, WorkloadSortColumn, WorkloadSortState},
    bookmarks::BookmarkEntry,
    icons::view_icon,
    k8s::dtos::RbacRule,
    state::ClusterSnapshot,
    ui::{
        TableFrame, bookmarked_name_cell,
        components::{content_block, default_theme},
        filter_cache::{cached_filter_indices_with_variant, data_fingerprint},
        format_age, format_small_int, render_centered_message, render_table_frame,
        resource_table_title, sort_header_cell, table_viewport_rows, table_window,
        views::filtering::filtered_role_indices,
        workload_sort_suffix,
    },
};
use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

use crate::ui::filter_cache::{
    DerivedRowsCache, DerivedRowsCacheKey, DerivedRowsCacheValue, cached_derived_rows,
};

// ── Role derived cell cache ────────────────────────────────────────

#[derive(Debug, Clone)]
struct RoleDerivedCell {
    rules_count: String,
    age: String,
}

type RoleDerivedCacheValue = DerivedRowsCacheValue<RoleDerivedCell>;
static ROLE_DERIVED_CACHE: LazyLock<DerivedRowsCache<RoleDerivedCell>> =
    LazyLock::new(Default::default);

fn cached_role_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    variant: u64,
) -> RoleDerivedCacheValue {
    let key = DerivedRowsCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.roles, snapshot.snapshot_version),
        variant,
        freshness_bucket: 0,
    };

    cached_derived_rows(&ROLE_DERIVED_CACHE, key, || {
        indices
            .iter()
            .map(|&role_idx| {
                let role = &snapshot.roles[role_idx];
                RoleDerivedCell {
                    rules_count: format_small_int(role.rules.len() as i64).into_owned(),
                    age: format_age(role.age),
                }
            })
            .collect()
    })
}

// ── Role rules detail cache ────────────────────────────────────────

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

#[allow(clippy::too_many_arguments)]
pub fn render_roles(
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
        AppView::Roles,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.roles, cluster.snapshot_version),
        cache_variant,
        |q| filtered_role_indices(&cluster.roles, q, sort),
    );

    let theme = default_theme();

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::Roles,
            query,
            "Roles",
            "Loading roles...",
            "No roles found",
            "No roles match the search query",
            focused,
        );
        return;
    }

    let (table_area, detail_area) = split_primary_detail(area);

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(table_area));
    let header = Row::new([
        sort_header_cell("Name", sort, WorkloadSortColumn::Name, &theme, true),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Rules", theme.header_style())),
        sort_header_cell("Age", sort, WorkloadSortColumn::Age, &theme, false),
    ])
    .height(1)
    .style(theme.header_style());

    let derived = cached_role_derived(cluster, query, &indices, cache_variant);

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
                    || ResourceRef::Role(role.name.clone(), role.namespace.clone()),
                    bookmarks,
                    role.name.as_str(),
                    name_style,
                    &theme,
                ),
                Cell::from(Span::styled(role.namespace.as_str(), dim_style)),
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
        view_icon(AppView::Roles).active(),
        "Roles",
        total,
        cluster.roles.len(),
        query,
        &sort_suffix,
    );
    let widths = role_widths(table_area);
    render_table_frame(
        frame,
        table_area,
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
        Paragraph::new((*detail).clone()).block(content_block("Selected Role Rules", focused)),
        detail_area,
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
            "No rules defined",
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

    #[test]
    fn role_widths_switch_to_compact_profile() {
        let widths = role_widths(Rect::new(0, 0, 80, 20));
        assert_eq!(widths[0], Constraint::Min(18));
        assert_eq!(widths[1], Constraint::Length(14));
        assert_eq!(widths[3], Constraint::Length(8));
    }

    #[test]
    fn role_widths_keep_wide_profile() {
        let widths = role_widths(Rect::new(0, 0, 120, 20));
        assert_eq!(widths[0], Constraint::Min(28));
        assert_eq!(widths[1], Constraint::Length(18));
        assert_eq!(widths[3], Constraint::Length(9));
    }
}
