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
    app::{AppView, WorkloadSortColumn, WorkloadSortState, filtered_workload_indices},
    k8s::dtos::RoleBindingSubject,
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

// ── ClusterRoleBinding derived cell cache ──────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClusterRoleBindingDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct ClusterRoleBindingDerivedCell {
    role_ref: String,
    subjects_count: String,
    age: String,
}

type ClusterRoleBindingDerivedCacheValue = Arc<Vec<ClusterRoleBindingDerivedCell>>;
static CLUSTER_ROLE_BINDING_DERIVED_CACHE: LazyLock<
    Mutex<Option<(ClusterRoleBindingDerivedCacheKey, ClusterRoleBindingDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

fn cached_cluster_role_binding_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> ClusterRoleBindingDerivedCacheValue {
    let key = ClusterRoleBindingDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(
            &snapshot.cluster_role_bindings,
            snapshot.snapshot_version,
        ),
    };

    if let Ok(cache) = CLUSTER_ROLE_BINDING_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&rb_idx| {
                let rb = &snapshot.cluster_role_bindings[rb_idx];
                ClusterRoleBindingDerivedCell {
                    role_ref: format!("{}/{}", rb.role_ref_kind, rb.role_ref_name),
                    subjects_count: format_small_int(rb.subjects.len() as i64).into_owned(),
                    age: format_age(rb.age),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = CLUSTER_ROLE_BINDING_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

// ── ClusterRoleBinding subjects detail cache ───────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClusterRoleBindingSubjectsCacheKey {
    theme_index: u8,
    snapshot_version: u64,
    name: String,
}

type ClusterRoleBindingSubjectsCacheValue = Arc<Vec<Line<'static>>>;
static CLUSTER_ROLE_BINDING_SUBJECTS_CACHE: LazyLock<
    Mutex<
        Option<(
            ClusterRoleBindingSubjectsCacheKey,
            ClusterRoleBindingSubjectsCacheValue,
        )>,
    >,
> = LazyLock::new(|| Mutex::new(None));

pub fn render_cluster_role_bindings(
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
        AppView::ClusterRoleBindings,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.cluster_role_bindings, cluster.snapshot_version),
        cache_variant,
        |q| {
            filtered_workload_indices(
                &cluster.cluster_role_bindings,
                q,
                sort,
                |rb, needle| {
                    needle.is_empty()
                        || contains_ci(&rb.name, needle)
                        || contains_ci(&rb.role_ref_name, needle)
                },
                |rb| rb.name.as_str(),
                |_rb| "",
                |rb| rb.age,
            )
        },
    );

    let theme = default_theme();

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::ClusterRoleBindings,
            query,
            "  Loading clusterrolebindings...",
            "  No clusterrolebindings found",
            "  No clusterrolebindings match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
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
    let window = table_window(total, selected, table_viewport_rows(chunks[0]));
    let name_header = workload_sort_header("Name", sort, WorkloadSortColumn::Name);
    let age_header = workload_sort_header("Age", sort, WorkloadSortColumn::Age);

    let header = Row::new([
        Cell::from(Span::styled(
            format!("  {name_header}"),
            theme.header_style(),
        )),
        Cell::from(Span::styled("RoleRef", theme.header_style())),
        Cell::from(Span::styled("Subjects", theme.header_style())),
        Cell::from(Span::styled(age_header, theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let derived = cached_cluster_role_binding_derived(cluster, query, &indices);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &rb_idx)| {
            let idx = window.start + local_idx;
            let rb = &cluster.cluster_role_bindings[rb_idx];
            let name_style = Style::default().fg(theme.fg);
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
                Cell::from(Span::styled(role_ref, Style::default().fg(theme.accent2))),
                Cell::from(Span::styled(subjects_count, Style::default().fg(theme.fg_dim))),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));
    let sort_suffix = workload_sort_suffix(sort);
    let title = format!(" 🔗 ClusterRoleBindings ({total}){sort_suffix} ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.cluster_role_bindings.len();
        active_block(&format!(
            " 🔗 ClusterRoleBindings ({total} of {all}) [/{query}]{sort_suffix}"
        ))
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Min(30),
                Constraint::Length(38),
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

    let sel_item = &cluster.cluster_role_bindings[indices[selected]];
    let detail = cached_subject_lines(
        crate::ui::theme::active_theme_index(),
        cluster.snapshot_version,
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
    name: &str,
    subjects: &[RoleBindingSubject],
    theme: &crate::ui::theme::Theme,
) -> ClusterRoleBindingSubjectsCacheValue {
    let key = ClusterRoleBindingSubjectsCacheKey {
        theme_index,
        snapshot_version,
        name: name.to_string(),
    };

    if let Ok(cache) = CLUSTER_ROLE_BINDING_SUBJECTS_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(render_subjects(subjects, theme));
    if let Ok(mut cache) = CLUSTER_ROLE_BINDING_SUBJECTS_CACHE.lock() {
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
