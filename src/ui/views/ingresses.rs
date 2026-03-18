//! Ingresses list view.

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
        views::filtering::{filtered_ingress_class_indices, filtered_ingress_indices},
    },
};

// ── Ingress derived cell cache ──────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct IngressDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct IngressDerivedCell {
    hosts: String,
    address: String,
    class: String,
}

type IngressDerivedCacheValue = Arc<Vec<IngressDerivedCell>>;
static INGRESS_DERIVED_CACHE: LazyLock<
    Mutex<Option<(IngressDerivedCacheKey, IngressDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

fn cached_ingress_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> IngressDerivedCacheValue {
    let key = IngressDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.ingresses, snapshot.snapshot_version),
    };

    if let Ok(cache) = INGRESS_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&ing_idx| {
                let ing = &snapshot.ingresses[ing_idx];
                IngressDerivedCell {
                    hosts: if ing.hosts.is_empty() {
                        "*".to_string()
                    } else {
                        ing.hosts.join(",")
                    },
                    address: ing.address.as_deref().unwrap_or("<pending>").to_string(),
                    class: ing.class.as_deref().unwrap_or("<none>").to_string(),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = INGRESS_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

pub fn render_ingresses(
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
        AppView::Ingresses,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.ingresses, cluster.snapshot_version),
        |q| filtered_ingress_indices(&cluster.ingresses, q),
    );

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::Ingresses,
            query,
            "Ingresses",
            "Loading ingresses...",
            "No ingresses found",
            "No ingresses match the search query",
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
        Cell::from(Span::styled("CLASS", theme.header_style())),
        Cell::from(Span::styled("HOSTS", theme.header_style())),
        Cell::from(Span::styled("ADDRESS", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let derived = cached_ingress_derived(cluster, query, &indices);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &ingress_idx)| {
            let idx = window.start + local_idx;
            let ingress = &cluster.ingresses[ingress_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let (hosts, class, address) = if let Some(cell) = derived.get(idx) {
                (
                    Cow::Borrowed(cell.hosts.as_str()),
                    Cow::Borrowed(cell.class.as_str()),
                    Cow::Borrowed(cell.address.as_str()),
                )
            } else {
                (
                    Cow::Owned(if ingress.hosts.is_empty() {
                        "*".to_string()
                    } else {
                        ingress.hosts.join(",")
                    }),
                    Cow::Owned(ingress.class.as_deref().unwrap_or("<none>").to_string()),
                    Cow::Owned(
                        ingress
                            .address
                            .as_deref()
                            .unwrap_or("<pending>")
                            .to_string(),
                    ),
                )
            };
            Row::new(vec![
                bookmarked_name_cell(
                    &ResourceRef::Ingress(ingress.name.clone(), ingress.namespace.clone()),
                    bookmarks,
                    ingress.name.as_str(),
                    Style::default().fg(theme.fg),
                    &theme,
                ),
                Cell::from(Span::styled(
                    ingress.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(class, Style::default().fg(theme.info))),
                Cell::from(Span::styled(hosts, Style::default().fg(theme.accent2))),
                Cell::from(Span::styled(address, Style::default().fg(theme.warning))),
            ])
            .style(row_style)
        })
        .collect();

    let title = resource_table_title(
        view_icon(AppView::Ingresses).active(),
        "Ingresses",
        total,
        cluster.ingresses.len(),
        query,
        "",
    );
    let widths = [
        Constraint::Percentage(26),
        Constraint::Percentage(16),
        Constraint::Percentage(16),
        Constraint::Percentage(27),
        Constraint::Percentage(15),
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

pub fn render_ingress_classes(
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
        AppView::IngressClasses,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.ingress_classes, cluster.snapshot_version),
        |q| filtered_ingress_class_indices(&cluster.ingress_classes, q),
    );

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::IngressClasses,
            query,
            "IngressClasses",
            "Loading ingress classes...",
            "No ingress classes found",
            "No ingress classes match the search query",
            focused,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let header = Row::new([
        Cell::from(Span::styled("  NAME", theme.header_style())),
        Cell::from(Span::styled("CONTROLLER", theme.header_style())),
        Cell::from(Span::styled("DEFAULT", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &ingress_class_idx)| {
            let idx = window.start + local_idx;
            let ingress_class = &cluster.ingress_classes[ingress_class_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let default_label = if ingress_class.is_default { "✓" } else { "" };
            Row::new(vec![
                bookmarked_name_cell(
                    &ResourceRef::IngressClass(ingress_class.name.clone()),
                    bookmarks,
                    ingress_class.name.as_str(),
                    Style::default().fg(theme.fg),
                    &theme,
                ),
                Cell::from(Span::styled(
                    ingress_class.controller.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    default_label,
                    if ingress_class.is_default {
                        Style::default().fg(theme.success)
                    } else {
                        Style::default().fg(theme.muted)
                    },
                )),
            ])
            .style(row_style)
        })
        .collect();

    let title = resource_table_title(
        view_icon(AppView::IngressClasses).active(),
        "IngressClasses",
        total,
        cluster.ingress_classes.len(),
        query,
        "",
    );
    let widths = [
        Constraint::Percentage(34),
        Constraint::Percentage(54),
        Constraint::Percentage(12),
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
