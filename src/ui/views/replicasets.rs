//! ReplicaSets list rendering.

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
    app::{AppView, ResourceRef, WorkloadSortColumn, WorkloadSortState},
    bookmarks::BookmarkEntry,
    state::ClusterSnapshot,
    ui::{
        bookmarked_name_cell,
        components::{active_block, default_block, default_theme},
        filter_cache::{cached_filter_indices_with_variant, data_fingerprint},
        format_age, format_image, format_small_int, loading_or_empty_message,
        responsive_table_widths, sort_header_cell, table_viewport_rows, table_window,
        views::filtering::filtered_replicaset_indices,
        workload_sort_suffix,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplicaSetDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
    variant: u64,
}

#[derive(Debug, Clone)]
struct ReplicaSetDerivedCell {
    image: String,
    age: String,
}

type ReplicaSetDerivedCacheValue = Arc<Vec<ReplicaSetDerivedCell>>;
static REPLICASET_DERIVED_CACHE: LazyLock<
    Mutex<Option<(ReplicaSetDerivedCacheKey, ReplicaSetDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

pub fn render_replicasets(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    query: &str,
    sort: Option<WorkloadSortState>,
) {
    let theme = default_theme();
    let query = query.trim();
    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::ReplicaSets,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.replicasets, cluster.snapshot_version),
        cache_variant,
        |q| filtered_replicaset_indices(&cluster.replicasets, q, sort),
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::ReplicaSets,
            query,
            "  Loading replica sets...",
            "  No replica sets found",
            "  No replica sets match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("Replica Sets")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));
    let header = Row::new([
        sort_header_cell("Name", sort, WorkloadSortColumn::Name, &theme, true),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Desired", theme.header_style())),
        Cell::from(Span::styled("Ready", theme.header_style())),
        Cell::from(Span::styled("Available", theme.header_style())),
        Cell::from(Span::styled("Image", theme.header_style())),
        sort_header_cell("Age", sort, WorkloadSortColumn::Age, &theme, false),
    ])
    .height(1)
    .style(theme.header_style());
    let name_style = Style::default().fg(theme.fg);
    let dim_style = Style::default().fg(theme.fg_dim);
    let muted_style = Style::default().fg(theme.muted);
    let derived = cached_replicaset_derived(cluster, query, indices.as_ref(), cache_variant);

    let mut rows: Vec<Row> = Vec::with_capacity(window.end.saturating_sub(window.start));
    for (local_idx, &rs_idx) in indices[window.start..window.end].iter().enumerate() {
        let idx = window.start + local_idx;
        let rs = &cluster.replicasets[rs_idx];
        let ready_style = readiness_style(rs.ready, rs.desired, &theme);
        let (image, age) = if let Some(cell) = derived.get(idx) {
            (
                Cow::Borrowed(cell.image.as_str()),
                Cow::Borrowed(cell.age.as_str()),
            )
        } else {
            (
                Cow::Owned(format_image(rs.image.as_deref(), 32)),
                Cow::Owned(format_age(rs.age)),
            )
        };
        let row_style = if idx.is_multiple_of(2) {
            Style::default().bg(theme.bg)
        } else {
            theme.row_alt_style()
        };

        rows.push(
            Row::new(vec![
                bookmarked_name_cell(
                    &ResourceRef::ReplicaSet(rs.name.clone(), rs.namespace.clone()),
                    bookmarks,
                    rs.name.as_str(),
                    name_style,
                    &theme,
                ),
                Cell::from(Span::styled(rs.namespace.as_str(), dim_style)),
                Cell::from(Span::styled(
                    format_small_int(i64::from(rs.desired)),
                    dim_style,
                )),
                Cell::from(Span::styled(
                    format_small_int(i64::from(rs.ready)),
                    ready_style,
                )),
                Cell::from(Span::styled(
                    format_small_int(i64::from(rs.available)),
                    dim_style,
                )),
                Cell::from(Span::styled(image, muted_style)),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style),
        );
    }

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let sort_suffix = workload_sort_suffix(sort);
    let title = format!(" Replica Sets ({total}){sort_suffix} ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.replicasets.len();
        active_block(&format!(
            " Replica Sets ({total} of {all}) [/{query}]{sort_suffix}"
        ))
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Length(28),
                Constraint::Length(16),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Length(11),
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

fn cached_replicaset_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    variant: u64,
) -> ReplicaSetDerivedCacheValue {
    let key = ReplicaSetDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(&cluster.replicasets, cluster.snapshot_version),
        variant,
    };

    if let Ok(cache) = REPLICASET_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&rs_idx| {
                let rs = &cluster.replicasets[rs_idx];
                ReplicaSetDerivedCell {
                    image: format_image(rs.image.as_deref(), 32),
                    age: format_age(rs.age),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = REPLICASET_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

use crate::ui::readiness_style;

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
