//! DaemonSets list rendering.

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
        format_small_int, loading_or_empty_message, responsive_table_widths, table_viewport_rows,
        table_window,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct DaemonSetDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct DaemonSetDerivedCell {
    image: String,
    age: String,
}

type DaemonSetDerivedCacheValue = Arc<Vec<DaemonSetDerivedCell>>;
static DAEMONSET_DERIVED_CACHE: LazyLock<
    Mutex<Option<(DaemonSetDerivedCacheKey, DaemonSetDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

/// Renders the DaemonSets table with stateful selection and scrollbar.
pub fn render_daemonsets(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let theme = default_theme();
    let query = query.trim();
    let indices = cached_filter_indices(
        AppView::DaemonSets,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.daemonsets, cluster.snapshot_version),
        |q| {
            if q.is_empty() {
                return (0..cluster.daemonsets.len()).collect();
            }
            cluster
                .daemonsets
                .iter()
                .enumerate()
                .filter_map(|(idx, ds)| {
                    let label_match = ds
                        .labels
                        .iter()
                        .any(|(k, v)| contains_ci(k, q) || contains_ci(v, q));
                    if contains_ci(&ds.name, q)
                        || contains_ci(&ds.selector, q)
                        || contains_ci(ds.image.as_deref().unwrap_or_default(), q)
                        || label_match
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
            AppView::DaemonSets,
            query,
            "  Loading daemonsets...",
            "  No daemonsets found",
            "  No daemonsets match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("DaemonSets")),
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
        Cell::from(Span::styled("Desired", theme.header_style())),
        Cell::from(Span::styled("Ready", theme.header_style())),
        Cell::from(Span::styled("Unavailable", theme.header_style())),
        Cell::from(Span::styled("Image", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());
    let derived = cached_daemonset_derived(cluster, query, indices.as_ref());
    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &ds_idx)| {
            let idx = window.start + local_idx;
            let ds = &cluster.daemonsets[ds_idx];
            let (image, age) = if let Some(cell) = derived.get(idx) {
                (
                    Cow::Borrowed(cell.image.as_str()),
                    Cow::Borrowed(cell.age.as_str()),
                )
            } else {
                (
                    Cow::Owned(format_image(ds.image.as_deref())),
                    Cow::Owned(format_age(ds.age)),
                )
            };
            let ready_style = readiness_style(ds.ready_count, ds.desired_count, &theme);
            let unavail_style = unavailable_style(ds.unavailable_count, &theme);
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", ds.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    ds.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    format_small_int(i64::from(ds.desired_count)),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    format_small_int(i64::from(ds.ready_count)),
                    ready_style,
                )),
                Cell::from(Span::styled(
                    format_small_int(i64::from(ds.unavailable_count)),
                    unavail_style,
                )),
                Cell::from(Span::styled(image, Style::default().fg(theme.muted))),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let title = format!(" 👾 DaemonSets ({total}) ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.daemonsets.len();
        active_block(&format!(" 👾 DaemonSets ({total} of {all}) [/{query}]"))
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Length(20),
                Constraint::Length(16),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Length(13),
                Constraint::Min(24),
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

fn cached_daemonset_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> DaemonSetDerivedCacheValue {
    let key = DaemonSetDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(&cluster.daemonsets, cluster.snapshot_version),
    };

    if let Ok(cache) = DAEMONSET_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&ds_idx| {
                let ds = &cluster.daemonsets[ds_idx];
                DaemonSetDerivedCell {
                    image: format_image(ds.image.as_deref()),
                    age: format_age(ds.age),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = DAEMONSET_DERIVED_CACHE.lock() {
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

fn unavailable_style(unavailable_count: i32, theme: &crate::ui::theme::Theme) -> Style {
    if unavailable_count == 0 {
        theme.badge_success_style()
    } else {
        theme.badge_error_style()
    }
}

fn format_image(image: Option<&str>) -> String {
    let Some(image) = image else {
        return "-".to_string();
    };

    const MAX_LEN: usize = 32;
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
        assert_eq!(readiness_style(4, 4, &theme).fg, Some(theme.success));
        assert_eq!(readiness_style(2, 4, &theme).fg, Some(theme.warning));
        assert_eq!(readiness_style(0, 4, &theme).fg, Some(theme.error));
    }
}
