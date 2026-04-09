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
        ResourceTableConfig, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{
            DerivedRowsCache, DerivedRowsCacheKey, DerivedRowsCacheValue, cached_derived_rows,
            cached_filter_indices, data_fingerprint,
        },
        format_small_int, render_resource_table, striped_row_style,
        views::filtering::filtered_hpa_indices,
    },
};

const NARROW_HPA_WIDTH: u16 = 96;

fn hpa_widths(area: Rect) -> [Constraint; 6] {
    if area.width < NARROW_HPA_WIDTH {
        [
            Constraint::Min(18),
            Constraint::Length(14),
            Constraint::Min(18),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(10),
        ]
    } else {
        [
            Constraint::Percentage(23),
            Constraint::Percentage(18),
            Constraint::Percentage(29),
            Constraint::Percentage(8),
            Constraint::Percentage(8),
            Constraint::Percentage(14),
        ]
    }
}

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

    let derived = cached_hpa_derived(cluster, query, &indices);
    let widths = hpa_widths(area);
    render_resource_table(
        frame,
        area,
        &theme,
        ResourceTableConfig {
            snapshot: cluster,
            view: AppView::HPAs,
            label: "HorizontalPodAutoscalers",
            loading_message: "Loading horizontal pod autoscalers...",
            empty_message: "No horizontal pod autoscalers found",
            empty_query_message: "No horizontal pod autoscalers match the search query",
            query,
            focused,
            filtered_total: indices.len(),
            all_total: cluster.hpas.len(),
            selected_idx,
            widths: &widths,
            sort_suffix: "",
        },
        |theme| {
            Row::new([
                Cell::from(Span::styled("  NAME", theme.header_style())),
                Cell::from(Span::styled("NAMESPACE", theme.header_style())),
                Cell::from(Span::styled("REFERENCE", theme.header_style())),
                Cell::from(Span::styled("MIN", theme.header_style())),
                Cell::from(Span::styled("MAX", theme.header_style())),
                Cell::from(Span::styled("REPLICAS", theme.header_style())),
            ])
            .style(theme.header_style())
            .height(1)
        },
        |window, theme| {
            indices[window.start..window.end]
                .iter()
                .enumerate()
                .map(|(local_idx, &hpa_idx)| {
                    let idx = window.start + local_idx;
                    let hpa = &cluster.hpas[hpa_idx];
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
                                Cow::Owned(format!(
                                    "{}/{}",
                                    hpa.current_replicas, hpa.desired_replicas
                                )),
                            )
                        };
                    Row::new(vec![
                        bookmarked_name_cell(
                            || ResourceRef::Hpa(hpa.name.clone(), hpa.namespace.clone()),
                            bookmarks,
                            hpa.name.as_str(),
                            Style::default().fg(theme.fg),
                            theme,
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
                    .style(striped_row_style(idx, theme))
                })
                .collect()
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hpa_widths_switch_to_compact_profile() {
        let widths = hpa_widths(Rect::new(0, 0, 84, 20));
        assert_eq!(widths[0], Constraint::Min(18));
        assert_eq!(widths[1], Constraint::Length(14));
        assert_eq!(widths[2], Constraint::Min(18));
        assert_eq!(widths[5], Constraint::Length(10));
    }

    #[test]
    fn hpa_widths_keep_wide_profile() {
        let widths = hpa_widths(Rect::new(0, 0, 120, 20));
        assert_eq!(widths[0], Constraint::Percentage(23));
        assert_eq!(widths[2], Constraint::Percentage(29));
        assert_eq!(widths[5], Constraint::Percentage(14));
    }
}
