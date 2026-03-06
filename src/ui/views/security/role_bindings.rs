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
        format_small_int, loading_or_empty_message, responsive_table_widths, table_viewport_rows,
        table_window,
    },
};
use std::sync::{Arc, LazyLock, Mutex};

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
) {
    let query = query.trim();
    let indices = cached_filter_indices(
        AppView::RoleBindings,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.role_bindings, cluster.snapshot_version),
        |q| {
            let mut out: Vec<usize> = cluster
                .role_bindings
                .iter()
                .enumerate()
                .filter_map(|(idx, rb)| {
                    if q.is_empty()
                        || contains_ci(&rb.name, q)
                        || contains_ci(&rb.namespace, q)
                        || contains_ci(&rb.role_ref_name, q)
                    {
                        Some(idx)
                    } else {
                        None
                    }
                })
                .collect();
            out.sort_unstable_by(|a, b| {
                let left = &cluster.role_bindings[*a];
                let right = &cluster.role_bindings[*b];
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

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("RoleRef", theme.header_style())),
        Cell::from(Span::styled("Subjects", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

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
            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled("  ", name_style),
                    Span::styled(rb.name.as_str(), name_style),
                ])),
                Cell::from(Span::styled(rb.namespace.as_str(), dim_style)),
                Cell::from(Span::styled(
                    format!("{}/{}", rb.role_ref_kind, rb.role_ref_name),
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(Span::styled(
                    format_small_int(rb.subjects.len() as i64),
                    dim_style,
                )),
                Cell::from(Span::styled(format_age(rb.age), theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));
    let title = format!(" 🔗 RoleBindings ({total}) ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.role_bindings.len();
        active_block(&format!(" 🔗 RoleBindings ({total} of {all}) [/{query}]"))
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
