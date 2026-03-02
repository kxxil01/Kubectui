//! Endpoints list view.

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
        loading_or_empty_message, table_viewport_rows, table_window,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct EndpointDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct EndpointDerivedCell {
    addresses: String,
    ports: String,
}

type EndpointDerivedCacheValue = Arc<Vec<EndpointDerivedCell>>;
static ENDPOINT_DERIVED_CACHE: LazyLock<
    Mutex<Option<(EndpointDerivedCacheKey, EndpointDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

pub fn render_endpoints(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search: &str,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = cached_filter_indices(
        AppView::Endpoints,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.endpoints),
        |q| {
            if q.is_empty() {
                return (0..cluster.endpoints.len()).collect();
            }
            cluster
                .endpoints
                .iter()
                .enumerate()
                .filter_map(|(idx, endpoint)| {
                    (contains_ci(&endpoint.name, q) || contains_ci(&endpoint.namespace, q))
                        .then_some(idx)
                })
                .collect()
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            query,
            "  Loading endpoints...",
            "  No endpoints found",
            "  No endpoints match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("Endpoints")),
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
        Cell::from(Span::styled("ADDRESSES", theme.header_style())),
        Cell::from(Span::styled("PORTS", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let derived = cached_endpoint_derived(cluster, query, indices.as_ref());
    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &endpoint_idx)| {
            let idx = window.start + local_idx;
            let endpoint = &cluster.endpoints[endpoint_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let (addrs, ports) = if let Some(cell) = derived.get(idx) {
                (
                    Cow::Borrowed(cell.addresses.as_str()),
                    Cow::Borrowed(cell.ports.as_str()),
                )
            } else {
                (
                    Cow::Owned(if endpoint.addresses.is_empty() {
                        "<none>".to_string()
                    } else {
                        endpoint.addresses.join(",")
                    }),
                    Cow::Owned(if endpoint.ports.is_empty() {
                        "<none>".to_string()
                    } else {
                        endpoint.ports.join(",")
                    }),
                )
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", endpoint.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    endpoint.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(addrs, Style::default().fg(theme.accent2))),
                Cell::from(Span::styled(ports, Style::default().fg(theme.info))),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let title = if query.is_empty() {
        format!(" Endpoints ({total}) ")
    } else {
        let all = cluster.endpoints.len();
        format!(" Endpoints ({total} of {all}) [/{query}]")
    };

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(28),
            Constraint::Percentage(20),
            Constraint::Percentage(30),
            Constraint::Percentage(22),
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

fn cached_endpoint_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> EndpointDerivedCacheValue {
    let key = EndpointDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(&cluster.endpoints),
    };

    if let Ok(cache) = ENDPOINT_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&endpoint_idx| {
                let endpoint = &cluster.endpoints[endpoint_idx];
                EndpointDerivedCell {
                    addresses: if endpoint.addresses.is_empty() {
                        "<none>".to_string()
                    } else {
                        endpoint.addresses.join(",")
                    },
                    ports: if endpoint.ports.is_empty() {
                        "<none>".to_string()
                    } else {
                        endpoint.ports.join(",")
                    },
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = ENDPOINT_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}
