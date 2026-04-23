use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Cell, Row},
};

use super::split_primary_detail;

use crate::{
    app::{AppView, ResourceRef, WorkloadSortColumn, WorkloadSortState},
    bookmarks::BookmarkEntry,
    icons::view_icon,
    k8s::dtos::RoleBindingSubject,
    state::ClusterSnapshot,
    ui::{
        SplitPaneFocus, TableFrame, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{cached_filter_indices_with_variant, data_fingerprint},
        format_age, format_small_int, render_centered_message, render_table_frame,
        resource_table_title, responsive_table_widths, sort_header_cell, table_viewport_rows,
        table_window,
        views::filtering::filtered_role_binding_indices,
        workload_sort_suffix,
    },
};
use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

// ── RoleBinding derived cell cache ─────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct RoleBindingDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
    variant: u64,
}

#[derive(Debug, Clone)]
struct RoleBindingDerivedCell {
    role_ref: String,
    subjects_count: String,
    age: String,
}

type RoleBindingDerivedCacheValue = Arc<Vec<RoleBindingDerivedCell>>;
static ROLE_BINDING_DERIVED_CACHE: LazyLock<
    Mutex<Option<(RoleBindingDerivedCacheKey, RoleBindingDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

fn cached_role_binding_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    variant: u64,
) -> RoleBindingDerivedCacheValue {
    let key = RoleBindingDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.role_bindings, snapshot.snapshot_version),
        variant,
    };

    if let Ok(cache) = ROLE_BINDING_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&rb_idx| {
                let rb = &snapshot.role_bindings[rb_idx];
                RoleBindingDerivedCell {
                    role_ref: format!("{}/{}", rb.role_ref_kind, rb.role_ref_name),
                    subjects_count: format_small_int(rb.subjects.len() as i64).into_owned(),
                    age: format_age(rb.age),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = ROLE_BINDING_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

// ── RoleBinding subjects detail cache ──────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct RoleBindingSubjectsCacheKey {
    theme_index: u8,
    snapshot_version: u64,
    namespace: String,
    name: String,
}

type RoleBindingSubjectsCacheValue = Arc<Vec<Line<'static>>>;
static ROLE_BINDING_SUBJECTS_CACHE: LazyLock<
    Mutex<Option<(RoleBindingSubjectsCacheKey, RoleBindingSubjectsCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

const ROLE_BINDINGS_NARROW_WIDTH: u16 = 92;

fn role_binding_widths(area: Rect) -> [Constraint; 5] {
    let wide = if area.width < ROLE_BINDINGS_NARROW_WIDTH {
        [
            Constraint::Min(20),
            Constraint::Length(14),
            Constraint::Length(22),
            Constraint::Length(8),
            Constraint::Length(8),
        ]
    } else {
        [
            Constraint::Min(24),
            Constraint::Length(16),
            Constraint::Length(34),
            Constraint::Length(9),
            Constraint::Length(9),
        ]
    };

    responsive_table_widths(area.width, wide)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_role_bindings(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    query: &str,
    sort: Option<WorkloadSortState>,
    detail_scroll: usize,
    focus: SplitPaneFocus,
) {
    let list_focused = matches!(focus, SplitPaneFocus::List);
    let detail_focused = matches!(focus, SplitPaneFocus::Detail);
    let query = query.trim();
    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::RoleBindings,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.role_bindings, cluster.snapshot_version),
        cache_variant,
        |q| filtered_role_binding_indices(&cluster.role_bindings, q, sort),
    );

    let theme = default_theme();

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::RoleBindings,
            query,
            "RoleBindings",
            "Loading rolebindings...",
            "No rolebindings found",
            "No rolebindings match the search query",
            list_focused,
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
        Cell::from(Span::styled("RoleRef", theme.header_style())),
        Cell::from(Span::styled("Subjects", theme.header_style())),
        sort_header_cell("Age", sort, WorkloadSortColumn::Age, &theme, false),
    ])
    .height(1)
    .style(theme.header_style());

    let derived = cached_role_binding_derived(cluster, query, &indices, cache_variant);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &rb_idx)| {
            let idx = window.start + local_idx;
            let rb = &cluster.role_bindings[rb_idx];
            let name_style = Style::default().fg(theme.fg);
            let dim_style = Style::default().fg(theme.fg_dim);
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let (role_ref, subjects_count, age): (Cow<'_, str>, Cow<'_, str>, Cow<'_, str>) =
                if let Some(cell) = derived.get(idx) {
                    (
                        Cow::Borrowed(cell.role_ref.as_str()),
                        Cow::Borrowed(cell.subjects_count.as_str()),
                        Cow::Borrowed(cell.age.as_str()),
                    )
                } else {
                    (
                        Cow::Owned(format!("{}/{}", rb.role_ref_kind, rb.role_ref_name)),
                        format_small_int(rb.subjects.len() as i64),
                        Cow::Owned(format_age(rb.age)),
                    )
                };
            Row::new(vec![
                bookmarked_name_cell(
                    || ResourceRef::RoleBinding(rb.name.clone(), rb.namespace.clone()),
                    bookmarks,
                    rb.name.as_str(),
                    name_style,
                    &theme,
                ),
                Cell::from(Span::styled(rb.namespace.as_str(), dim_style)),
                Cell::from(Span::styled(role_ref, Style::default().fg(theme.accent2))),
                Cell::from(Span::styled(subjects_count, dim_style)),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let sort_suffix = workload_sort_suffix(sort);
    let title = resource_table_title(
        view_icon(AppView::RoleBindings).active(),
        "RoleBindings",
        total,
        cluster.role_bindings.len(),
        query,
        &sort_suffix,
    );
    let widths = role_binding_widths(table_area);
    render_table_frame(
        frame,
        table_area,
        TableFrame {
            rows,
            header,
            widths: &widths,
            title: &title,
            focused: list_focused,
            window,
            total,
            selected,
        },
        &theme,
    );

    let sel_item = &cluster.role_bindings[indices[selected]];
    let detail = cached_subject_lines(
        crate::ui::theme::active_theme_index(),
        cluster.snapshot_version,
        &sel_item.namespace,
        &sel_item.name,
        &sel_item.subjects,
        &theme,
    );
    super::render_scrollable_security_detail(
        frame,
        detail_area,
        "Selected Binding Subjects",
        detail_focused,
        (*detail).clone(),
        detail_scroll,
    );
}

fn cached_subject_lines(
    theme_index: u8,
    snapshot_version: u64,
    namespace: &str,
    name: &str,
    subjects: &[RoleBindingSubject],
    theme: &crate::ui::theme::Theme,
) -> RoleBindingSubjectsCacheValue {
    let key = RoleBindingSubjectsCacheKey {
        theme_index,
        snapshot_version,
        namespace: namespace.to_string(),
        name: name.to_string(),
    };

    if let Ok(cache) = ROLE_BINDING_SUBJECTS_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(render_subjects(subjects, theme));
    if let Ok(mut cache) = ROLE_BINDING_SUBJECTS_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }
    built
}

fn render_subjects(
    subjects: &[RoleBindingSubject],
    theme: &crate::ui::theme::Theme,
) -> Vec<Line<'static>> {
    if subjects.is_empty() {
        return vec![Line::from(Span::styled(
            "No subjects",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Theme;

    #[test]
    fn subjects_render_as_human_readable_lines() {
        let theme = Theme::dark();
        let lines = render_subjects(
            &[RoleBindingSubject {
                kind: "ServiceAccount".to_string(),
                name: "builder".to_string(),
                namespace: Some("default".to_string()),
                api_group: None,
            }],
            &theme,
        );

        let text = lines[0].to_string();
        assert!(text.contains("ServiceAccount/builder"));
        assert!(text.contains("ns=default"));
    }

    #[test]
    fn role_binding_widths_compact_on_narrow_area() {
        let widths = role_binding_widths(Rect::new(0, 0, 80, 12));
        assert_eq!(widths[1], Constraint::Length(14));
        assert_eq!(widths[2], Constraint::Length(22));
    }
}
