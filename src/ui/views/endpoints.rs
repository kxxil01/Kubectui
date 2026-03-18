//! Endpoints list view.

use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Row},
};

use crate::{
    app::{AppView, ResourceRef},
    bookmarks::BookmarkEntry,
    icons::view_icon,
    state::ClusterSnapshot,
    ui::{
        TableFrame, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{cached_filter_indices, data_fingerprint},
        render_centered_message, render_table_frame, resource_table_title, table_viewport_rows,
        table_window,
        views::filtering::filtered_endpoint_indices,
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
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    search: &str,
    focused: bool,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = cached_filter_indices(
        AppView::Endpoints,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.endpoints, cluster.snapshot_version),
        |q| filtered_endpoint_indices(&cluster.endpoints, q),
    );

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::Endpoints,
            query,
            "Endpoints",
            "Loading endpoints...",
            "No endpoints found",
            "No endpoints match the search query",
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
                bookmarked_name_cell(
                    &ResourceRef::Endpoint(endpoint.name.clone(), endpoint.namespace.clone()),
                    bookmarks,
                    endpoint.name.as_str(),
                    Style::default().fg(theme.fg),
                    &theme,
                ),
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

    let title = resource_table_title(
        view_icon(AppView::Endpoints).active(),
        "Endpoints",
        total,
        cluster.endpoints.len(),
        query,
        "",
    );
    let widths = [
        Constraint::Percentage(28),
        Constraint::Percentage(20),
        Constraint::Percentage(30),
        Constraint::Percentage(22),
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

fn cached_endpoint_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> EndpointDerivedCacheValue {
    let key = EndpointDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(&cluster.endpoints, cluster.snapshot_version),
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
