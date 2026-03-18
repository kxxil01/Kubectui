//! HorizontalPodAutoscaler list view.

use std::{borrow::Cow, sync::LazyLock};

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Row},
};

use crate::{
    app::{AppView, ResourceRef},
    bookmarks::BookmarkEntry,
    state::ClusterSnapshot,
    ui::{
        TableFrame, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{
            DerivedRowsCache, DerivedRowsCacheKey, DerivedRowsCacheValue, cached_derived_rows,
            cached_filter_indices, data_fingerprint,
        },
        format_small_int, render_centered_message, render_table_frame, table_viewport_rows,
        table_window,
        views::filtering::filtered_hpa_indices,
    },
};

// ── HPA derived cell cache ──────────────────────────────────────────

#[derive(Debug, Clone)]
struct HpaDerivedCell {
    min: String,
    max: String,
    replicas: String,
}

type HpaDerivedCacheValue = DerivedRowsCacheValue<HpaDerivedCell>;
static HPA_DERIVED_CACHE: LazyLock<DerivedRowsCache<HpaDerivedCell>> =
    LazyLock::new(Default::default);

fn cached_hpa_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> HpaDerivedCacheValue {
    let key = DerivedRowsCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.hpas, snapshot.snapshot_version),
        variant: 0,
        freshness_bucket: 0,
    };

    cached_derived_rows(&HPA_DERIVED_CACHE, key, || {
        indices
            .iter()
            .map(|&hpa_idx| {
                let hpa = &snapshot.hpas[hpa_idx];
                HpaDerivedCell {
                    min: format_small_int(i64::from(hpa.min_replicas.unwrap_or(1))).into_owned(),
                    max: format_small_int(i64::from(hpa.max_replicas)).into_owned(),
                    replicas: format!("{}/{}", hpa.current_replicas, hpa.desired_replicas),
                }
            })
            .collect()
    })
}

pub fn render_hpas(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    search: &str,
    focused: bool,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = cached_filter_indices(
        AppView::HPAs,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.hpas, cluster.snapshot_version),
        |q| filtered_hpa_indices(&cluster.hpas, q),
    );

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::HPAs,
            query,
            "HorizontalPodAutoscalers",
            "Loading horizontal pod autoscalers...",
            "No horizontal pod autoscalers found",
            "No horizontal pod autoscalers match the search query",
            focused,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let header = Row::new([
        Cell::from(Span::styled("  NAME", theme.header_style())),
        Cell::from(Span::styled("NAMESPACE", theme.header_style())),
        Cell::from(Span::styled("REFERENCE", theme.header_style())),
        Cell::from(Span::styled("MIN", theme.header_style())),
        Cell::from(Span::styled("MAX", theme.header_style())),
        Cell::from(Span::styled("REPLICAS", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let derived = cached_hpa_derived(cluster, query, &indices);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &hpa_idx)| {
            let idx = window.start + local_idx;
            let hpa = &cluster.hpas[hpa_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let (min, max, replicas): (Cow<'_, str>, Cow<'_, str>, Cow<'_, str>) =
                if let Some(cell) = derived.get(idx) {
                    (
                        Cow::Borrowed(cell.min.as_str()),
                        Cow::Borrowed(cell.max.as_str()),
                        Cow::Borrowed(cell.replicas.as_str()),
                    )
                } else {
                    (
                        format_small_int(i64::from(hpa.min_replicas.unwrap_or(1))),
                        format_small_int(i64::from(hpa.max_replicas)),
                        Cow::Owned(format!("{}/{}", hpa.current_replicas, hpa.desired_replicas)),
                    )
                };
            Row::new(vec![
                bookmarked_name_cell(
                    &ResourceRef::Hpa(hpa.name.clone(), hpa.namespace.clone()),
                    bookmarks,
                    hpa.name.as_str(),
                    Style::default().fg(theme.fg),
                    &theme,
                ),
                Cell::from(Span::styled(
                    hpa.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    hpa.reference.clone(),
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(Span::styled(min, Style::default().fg(theme.info))),
                Cell::from(Span::styled(max, Style::default().fg(theme.info))),
                Cell::from(Span::styled(replicas, Style::default().fg(theme.warning))),
            ])
            .style(row_style)
        })
        .collect();

    let title = if query.is_empty() {
        format!(" HorizontalPodAutoscalers ({total}) ")
    } else {
        let all = cluster.hpas.len();
        format!(" HorizontalPodAutoscalers ({total} of {all}) [/{query}]")
    };
    let widths = [
        Constraint::Percentage(23),
        Constraint::Percentage(18),
        Constraint::Percentage(29),
        Constraint::Percentage(8),
        Constraint::Percentage(8),
        Constraint::Percentage(14),
    ];

    render_table_frame(
        frame,
        area,
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
}
