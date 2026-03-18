//! ConfigMaps and Secrets list views.

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
    state::ClusterSnapshot,
    ui::{
        TableFrame, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{cached_filter_indices, data_fingerprint},
        format_small_int, render_centered_message, render_table_frame, table_viewport_rows,
        table_window,
        views::filtering::{filtered_config_map_indices, filtered_secret_indices},
    },
};

// ── ConfigMap derived cell cache ────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigMapDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct ConfigMapDerivedCell {
    data_count: String,
}

type ConfigMapDerivedCacheValue = Arc<Vec<ConfigMapDerivedCell>>;
static CONFIG_MAP_DERIVED_CACHE: LazyLock<
    Mutex<Option<(ConfigMapDerivedCacheKey, ConfigMapDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

fn cached_config_map_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> ConfigMapDerivedCacheValue {
    let key = ConfigMapDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.config_maps, snapshot.snapshot_version),
    };

    if let Ok(cache) = CONFIG_MAP_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&cm_idx| {
                let cm = &snapshot.config_maps[cm_idx];
                ConfigMapDerivedCell {
                    data_count: format_small_int(cm.data_count as i64).into_owned(),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = CONFIG_MAP_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

pub fn render_config_maps(
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
        AppView::ConfigMaps,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.config_maps, cluster.snapshot_version),
        |q| filtered_config_map_indices(&cluster.config_maps, q),
    );

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::ConfigMaps,
            query,
            "ConfigMaps",
            "Loading configmaps...",
            "No configmaps found",
            "No configmaps match the search query",
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
        Cell::from(Span::styled("DATA", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let derived = cached_config_map_derived(cluster, query, &indices);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &cm_idx)| {
            let idx = window.start + local_idx;
            let cm = &cluster.config_maps[cm_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let data_count: Cow<'_, str> = if let Some(cell) = derived.get(idx) {
                Cow::Borrowed(cell.data_count.as_str())
            } else {
                format_small_int(cm.data_count as i64)
            };
            Row::new(vec![
                bookmarked_name_cell(
                    &ResourceRef::ConfigMap(cm.name.clone(), cm.namespace.clone()),
                    bookmarks,
                    cm.name.as_str(),
                    Style::default().fg(theme.fg),
                    &theme,
                ),
                Cell::from(Span::styled(
                    cm.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(data_count, Style::default().fg(theme.info))),
            ])
            .style(row_style)
        })
        .collect();

    let title = if query.is_empty() {
        format!(" 📄 ConfigMaps ({total}) ")
    } else {
        let all = cluster.config_maps.len();
        format!(" 📄 ConfigMaps ({total} of {all}) [/{query}]")
    };
    let widths = [
        Constraint::Percentage(52),
        Constraint::Percentage(33),
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

// ── Secret derived cell cache ───────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct SecretDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct SecretDerivedCell {
    data_count: String,
}

type SecretDerivedCacheValue = Arc<Vec<SecretDerivedCell>>;
static SECRET_DERIVED_CACHE: LazyLock<
    Mutex<Option<(SecretDerivedCacheKey, SecretDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

fn cached_secret_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> SecretDerivedCacheValue {
    let key = SecretDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.secrets, snapshot.snapshot_version),
    };

    if let Ok(cache) = SECRET_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&secret_idx| {
                let secret = &snapshot.secrets[secret_idx];
                SecretDerivedCell {
                    data_count: format_small_int(secret.data_count as i64).into_owned(),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = SECRET_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

pub fn render_secrets(
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
        AppView::Secrets,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.secrets, cluster.snapshot_version),
        |q| filtered_secret_indices(&cluster.secrets, q),
    );

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::Secrets,
            query,
            "Secrets",
            "Loading secrets...",
            "No secrets found",
            "No secrets match the search query",
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
        Cell::from(Span::styled("TYPE", theme.header_style())),
        Cell::from(Span::styled("DATA", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let derived = cached_secret_derived(cluster, query, &indices);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &secret_idx)| {
            let idx = window.start + local_idx;
            let secret = &cluster.secrets[secret_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let data_count: Cow<'_, str> = if let Some(cell) = derived.get(idx) {
                Cow::Borrowed(cell.data_count.as_str())
            } else {
                format_small_int(secret.data_count as i64)
            };
            Row::new(vec![
                bookmarked_name_cell(
                    &ResourceRef::Secret(secret.name.clone(), secret.namespace.clone()),
                    bookmarks,
                    secret.name.as_str(),
                    Style::default().fg(theme.fg),
                    &theme,
                ),
                Cell::from(Span::styled(
                    secret.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    secret.type_.clone(),
                    Style::default().fg(theme.warning),
                )),
                Cell::from(Span::styled(data_count, Style::default().fg(theme.info))),
            ])
            .style(row_style)
        })
        .collect();

    let title = if query.is_empty() {
        format!(" 🔐 Secrets ({total}) ")
    } else {
        let all = cluster.secrets.len();
        format!(" 🔐 Secrets ({total} of {all}) [/{query}]")
    };
    let widths = [
        Constraint::Percentage(38),
        Constraint::Percentage(24),
        Constraint::Percentage(26),
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
