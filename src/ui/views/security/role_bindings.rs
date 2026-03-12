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
    app::{AppView, WorkloadSortColumn, WorkloadSortState},
    k8s::dtos::RoleBindingSubject,
    state::ClusterSnapshot,
    ui::{
        components::{active_block, default_block, default_theme},
        filter_cache::{cached_filter_indices_with_variant, data_fingerprint},
        format_age, format_small_int, loading_or_empty_message, responsive_table_widths,
        table_viewport_rows, table_window,
        views::filtering::filtered_role_binding_indices,
        workload_sort_header, workload_sort_suffix,
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

pub fn render_role_bindings(
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
        AppView::RoleBindings,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.role_bindings, cluster.snapshot_version),
        cache_variant,
        |q| filtered_role_binding_indices(&cluster.role_bindings, q, sort),
    );

    let theme = default_theme();

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::RoleBindings,
            query,
            "  Loading rolebindings...",
            "  No rolebindings found",
            "  No rolebindings match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("RoleBindings")),
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
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("RoleRef", theme.header_style())),
        Cell::from(Span::styled("Subjects", theme.header_style())),
        Cell::from(Span::styled(age_header, theme.header_style())),
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
                Cell::from(Line::from(vec![
                    Span::styled("  ", name_style),
                    Span::styled(rb.name.as_str(), name_style),
                ])),
                Cell::from(Span::styled(rb.namespace.as_str(), dim_style)),
                Cell::from(Span::styled(role_ref, Style::default().fg(theme.accent2))),
                Cell::from(Span::styled(subjects_count, dim_style)),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));
    let sort_suffix = workload_sort_suffix(sort);
    let title = format!(" 🔗 RoleBindings ({total}){sort_suffix} ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.role_bindings.len();
        active_block(&format!(
            " 🔗 RoleBindings ({total} of {all}) [/{query}]{sort_suffix}"
        ))
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Min(24),
                Constraint::Length(16),
                Constraint::Length(34),
                Constraint::Length(9),
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

    let sel_item = &cluster.role_bindings[indices[selected]];
    let detail = cached_subject_lines(
        crate::ui::theme::active_theme_index(),
        cluster.snapshot_version,
        &sel_item.namespace,
        &sel_item.name,
        &sel_item.subjects,
        &theme,
    );
    frame.render_widget(
        Paragraph::new((*detail).clone()).block(active_block("Selected Binding Subjects")),
        chunks[1],
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
}
