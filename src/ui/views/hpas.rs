//! HorizontalPodAutoscaler list view.

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
        format_small_int, loading_or_empty_message, table_viewport_rows, table_window,
    },
};

// ── HPA derived cell cache ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct HpaDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct HpaDerivedCell {
    min: String,
    max: String,
    replicas: String,
}

type HpaDerivedCacheValue = Arc<Vec<HpaDerivedCell>>;
static HPA_DERIVED_CACHE: LazyLock<Mutex<Option<(HpaDerivedCacheKey, HpaDerivedCacheValue)>>> =
    LazyLock::new(|| Mutex::new(None));

fn cached_hpa_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> HpaDerivedCacheValue {
    let key = HpaDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.hpas, snapshot.snapshot_version),
    };

    if let Ok(cache) = HPA_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
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
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = HPA_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

pub fn render_hpas(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search: &str,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = cached_filter_indices(
        AppView::HPAs,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.hpas, cluster.snapshot_version),
        |q| {
            if q.is_empty() {
                return (0..cluster.hpas.len()).collect();
            }
            cluster
                .hpas
                .iter()
                .enumerate()
                .filter_map(|(idx, hpa)| {
                    (contains_ci(&hpa.name, q)
                        || contains_ci(&hpa.namespace, q)
                        || contains_ci(&hpa.reference, q))
                    .then_some(idx)
                })
                .collect()
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::HPAs,
            query,
            "  Loading horizontal pod autoscalers...",
            "  No horizontal pod autoscalers found",
            "  No horizontal pod autoscalers match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("HorizontalPodAutoscalers")),
            area,
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
                Cell::from(Span::styled(
                    format!("  {}", hpa.name),
                    Style::default().fg(theme.fg),
                )),
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

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let title = if query.is_empty() {
        format!(" HorizontalPodAutoscalers ({total}) ")
    } else {
        let all = cluster.hpas.len();
        format!(" HorizontalPodAutoscalers ({total} of {all}) [/{query}]")
    };

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(23),
            Constraint::Percentage(18),
            Constraint::Percentage(29),
            Constraint::Percentage(8),
            Constraint::Percentage(8),
            Constraint::Percentage(14),
        ],
    )
    .header(header)
    .block(active_block(&title))
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
