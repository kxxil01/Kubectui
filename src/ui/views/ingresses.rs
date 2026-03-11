//! Ingresses list view.

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
    selected_idx: usize,
    search: &str,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = cached_filter_indices(
        AppView::Ingresses,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.ingresses, cluster.snapshot_version),
        |q| {
            if q.is_empty() {
                return (0..cluster.ingresses.len()).collect();
            }
            cluster
                .ingresses
                .iter()
                .enumerate()
                .filter_map(|(idx, ingress)| {
                    let host_matches = ingress.hosts.iter().any(|host| contains_ci(host, q));
                    (contains_ci(&ingress.name, q)
                        || contains_ci(&ingress.namespace, q)
                        || ingress
                            .class
                            .as_ref()
                            .is_some_and(|class| contains_ci(class, q))
                        || ingress
                            .address
                            .as_ref()
                            .is_some_and(|address| contains_ci(address, q))
                        || host_matches)
                        .then_some(idx)
                })
                .collect()
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::Ingresses,
            query,
            "  Loading ingresses...",
            "  No ingresses found",
            "  No ingresses match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("Ingresses")),
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
                Cell::from(Span::styled(
                    format!("  {}", ingress.name),
                    Style::default().fg(theme.fg),
                )),
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

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let title = if query.is_empty() {
        format!(" Ingresses ({total}) ")
    } else {
        let all = cluster.ingresses.len();
        format!(" Ingresses ({total} of {all}) [/{query}]")
    };

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(26),
            Constraint::Percentage(16),
            Constraint::Percentage(16),
            Constraint::Percentage(27),
            Constraint::Percentage(15),
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

pub fn render_ingress_classes(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search: &str,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = cached_filter_indices(
        AppView::IngressClasses,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.ingress_classes, cluster.snapshot_version),
        |q| {
            if q.is_empty() {
                return (0..cluster.ingress_classes.len()).collect();
            }
            cluster
                .ingress_classes
                .iter()
                .enumerate()
                .filter_map(|(idx, ingress_class)| {
                    (contains_ci(&ingress_class.name, q)
                        || contains_ci(&ingress_class.controller, q))
                    .then_some(idx)
                })
                .collect()
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::IngressClasses,
            query,
            "  Loading ingress classes...",
            "  No ingress classes found",
            "  No ingress classes match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("IngressClasses")),
            area,
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
                Cell::from(Span::styled(
                    format!("  {}", ingress_class.name),
                    Style::default().fg(theme.fg),
                )),
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

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let title = if query.is_empty() {
        format!(" IngressClasses ({total}) ")
    } else {
        let all = cluster.ingress_classes.len();
        format!(" IngressClasses ({total} of {all}) [/{query}]")
    };

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(34),
            Constraint::Percentage(54),
            Constraint::Percentage(12),
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
