//! StatefulSets list rendering.

use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

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
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices, data_fingerprint},
        loading_or_empty_message, responsive_table_widths, table_viewport_rows, table_window,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatefulSetDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct StatefulSetDerivedCell {
    ready: String,
    image: String,
    age: String,
}

type StatefulSetDerivedCacheValue = Arc<Vec<StatefulSetDerivedCell>>;
static STATEFULSET_DERIVED_CACHE: LazyLock<
    Mutex<Option<(StatefulSetDerivedCacheKey, StatefulSetDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

/// Renders the StatefulSets table with stateful selection and scrollbar.
pub fn render_statefulsets(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let theme = default_theme();
    let query = query.trim();
    let indices = cached_filter_indices(
        AppView::StatefulSets,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.statefulsets, cluster.snapshot_version),
        |q| {
            if q.is_empty() {
                return (0..cluster.statefulsets.len()).collect();
            }
            cluster
                .statefulsets
                .iter()
                .enumerate()
                .filter_map(|(idx, ss)| {
                    if contains_ci(&ss.name, q)
                        || contains_ci(ss.image.as_deref().unwrap_or_default(), q)
                    {
                        Some(idx)
                    } else {
                        None
                    }
                })
                .collect()
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::StatefulSets,
            query,
            "  Loading statefulsets...",
            "  No statefulsets found",
            "  No statefulsets match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("StatefulSets")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Ready", theme.header_style())),
        Cell::from(Span::styled("Service", theme.header_style())),
        Cell::from(Span::styled("Image", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());
    let derived = cached_statefulset_derived(cluster, query, indices.as_ref());
    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &ss_idx)| {
            let idx = window.start + local_idx;
            let ss = &cluster.statefulsets[ss_idx];
            let (ready, image, age) = if let Some(cell) = derived.get(idx) {
                (
                    Cow::Borrowed(cell.ready.as_str()),
                    Cow::Borrowed(cell.image.as_str()),
                    Cow::Borrowed(cell.age.as_str()),
                )
            } else {
                (
                    Cow::Owned(format!("{}/{}", ss.ready_replicas, ss.desired_replicas)),
                    Cow::Owned(format_image(ss.image.as_deref())),
                    Cow::Owned(format_age(ss.age)),
                )
            };
            let ready_style = readiness_style(ss.ready_replicas, ss.desired_replicas, &theme);
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", ss.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    ss.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(ready, ready_style)),
                Cell::from(Span::styled(
                    ss.service_name.clone(),
                    Style::default().fg(theme.info),
                )),
                Cell::from(Span::styled(image, Style::default().fg(theme.muted))),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let title = format!(" 🗄  StatefulSets ({total}) ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.statefulsets.len();
        active_block(&format!(" 🗄  StatefulSets ({total} of {all}) [/{query}]"))
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Length(22),
                Constraint::Length(16),
                Constraint::Length(10),
                Constraint::Length(22),
                Constraint::Min(20),
                Constraint::Length(9),
            ],
        ),
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

fn cached_statefulset_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> StatefulSetDerivedCacheValue {
    let key = StatefulSetDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(&cluster.statefulsets, cluster.snapshot_version),
    };

    if let Ok(cache) = STATEFULSET_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&ss_idx| {
                let ss = &cluster.statefulsets[ss_idx];
                StatefulSetDerivedCell {
                    ready: format!("{}/{}", ss.ready_replicas, ss.desired_replicas),
                    image: format_image(ss.image.as_deref()),
                    age: format_age(ss.age),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = STATEFULSET_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

fn readiness_style(ready: i32, desired: i32, theme: &crate::ui::theme::Theme) -> Style {
    if desired > 0 && ready >= desired {
        theme.badge_success_style()
    } else if ready > 0 {
        theme.badge_warning_style()
    } else {
        theme.badge_error_style()
    }
}

fn format_image(image: Option<&str>) -> String {
    let Some(image) = image else {
        return "-".to_string();
    };

    const MAX_LEN: usize = 30;
    if image.chars().count() <= MAX_LEN {
        image.to_string()
    } else {
        format!(
            "{}...",
            image
                .chars()
                .take(MAX_LEN.saturating_sub(3))
                .collect::<String>()
        )
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
    fn readiness_style_maps_to_expected_colors() {
        let theme = Theme::dark();
        assert_eq!(readiness_style(3, 3, &theme).fg, Some(theme.success));
        assert_eq!(readiness_style(1, 3, &theme).fg, Some(theme.warning));
        assert_eq!(readiness_style(0, 3, &theme).fg, Some(theme.error));
    }
}
