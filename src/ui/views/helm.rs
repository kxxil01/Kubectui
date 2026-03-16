//! Helm releases view.

use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

use ratatui::{
    layout::{Margin, Rect},
    prelude::{Color, Frame, Style},
    text::Span,
    widgets::{
        Cell, HighlightSpacing, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
        TableState,
    },
};

use crate::{
    app::{AppView, ResourceRef},
    bookmarks::BookmarkEntry,
    state::ClusterSnapshot,
    ui::{
        bookmarked_name_cell,
        components::{content_block, default_theme},
        filter_cache::{cached_filter_indices, data_fingerprint},
        loading_or_empty_message, table_viewport_rows, table_window,
        views::filtering::{filtered_helm_release_indices, filtered_helm_repo_indices},
    },
};

// ── Helm Release derived cell cache ─────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct HelmReleaseDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct HelmReleaseDerivedCell {
    chart_display: String,
    revision: String,
    updated: String,
}

type HelmReleaseDerivedCacheValue = Arc<Vec<HelmReleaseDerivedCell>>;
static HELM_RELEASE_DERIVED_CACHE: LazyLock<
    Mutex<Option<(HelmReleaseDerivedCacheKey, HelmReleaseDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

fn cached_helm_release_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> HelmReleaseDerivedCacheValue {
    let key = HelmReleaseDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.helm_releases, snapshot.snapshot_version),
    };

    if let Ok(cache) = HELM_RELEASE_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&rel_idx| {
                let rel = &snapshot.helm_releases[rel_idx];
                HelmReleaseDerivedCell {
                    chart_display: if rel.chart_version.is_empty() {
                        rel.chart.clone()
                    } else {
                        format!("{}-{}", rel.chart, rel.chart_version)
                    },
                    revision: rel.revision.to_string(),
                    updated: rel
                        .updated
                        .map(|ts| ts.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "-".to_string()),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = HELM_RELEASE_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

/// Renders the Helm releases table.
pub fn render_helm_releases(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    search_query: &str,
    focused: bool,
) {
    let theme = default_theme();
    let query = search_query.trim();

    let indices = cached_filter_indices(
        AppView::HelmReleases,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.helm_releases, cluster.snapshot_version),
        |q| filtered_helm_release_indices(&cluster.helm_releases, q),
    );

    let header = Row::new([
        Cell::from(Span::styled("  NAME", theme.header_style())),
        Cell::from(Span::styled("NAMESPACE", theme.header_style())),
        Cell::from(Span::styled("CHART", theme.header_style())),
        Cell::from(Span::styled("VERSION", theme.header_style())),
        Cell::from(Span::styled("STATUS", theme.header_style())),
        Cell::from(Span::styled("REVISION", theme.header_style())),
        Cell::from(Span::styled("UPDATED", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    if indices.is_empty() {
        let empty_msg = loading_or_empty_message(
            cluster,
            AppView::HelmReleases,
            query,
            "Loading Helm releases...",
            "No Helm releases found (Helm v3 stores releases as Kubernetes Secrets)",
            "No releases match search",
        );
        let title = if query.is_empty() {
            " Helm Releases (0) ".to_string()
        } else {
            let all = cluster.helm_releases.len();
            format!(" Helm Releases (0 of {all}) [/{query}]")
        };
        frame.render_widget(
            ratatui::widgets::Paragraph::new(Span::styled(
                format!("  {empty_msg}"),
                Style::default().fg(theme.fg_dim),
            ))
            .block(content_block(&title, focused)),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let derived = cached_helm_release_derived(cluster, query, &indices);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &release_idx)| {
            let idx = window.start + local_idx;
            let release = &cluster.helm_releases[release_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let status_style = match release.status.as_str() {
                "deployed" => Style::default().fg(Color::Green),
                "failed" => Style::default().fg(Color::Red),
                "pending-install" | "pending-upgrade" | "pending-rollback" => {
                    Style::default().fg(Color::Yellow)
                }
                "superseded" => Style::default().fg(Color::DarkGray),
                _ => Style::default().fg(theme.fg_dim),
            };

            let (chart_display, revision, updated) = if let Some(cell) = derived.get(idx) {
                (
                    Cow::Borrowed(cell.chart_display.as_str()),
                    Cow::Borrowed(cell.revision.as_str()),
                    Cow::Borrowed(cell.updated.as_str()),
                )
            } else {
                (
                    Cow::Owned(if release.chart_version.is_empty() {
                        release.chart.clone()
                    } else {
                        format!("{}-{}", release.chart, release.chart_version)
                    }),
                    Cow::Owned(release.revision.to_string()),
                    Cow::Owned(
                        release
                            .updated
                            .map(|ts| ts.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                )
            };

            Row::new(vec![
                bookmarked_name_cell(
                    &ResourceRef::HelmRelease(release.name.clone(), release.namespace.clone()),
                    bookmarks,
                    release.name.as_str(),
                    Style::default().fg(theme.fg),
                    &theme,
                ),
                Cell::from(Span::styled(
                    release.namespace.clone(),
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(Span::from(chart_display)).style(Style::default().fg(theme.fg_dim)),
                Cell::from(release.chart_version.clone()).style(Style::default().fg(theme.fg_dim)),
                Cell::from(release.status.clone()).style(status_style),
                Cell::from(Span::from(revision)).style(Style::default().fg(theme.fg_dim)),
                Cell::from(Span::from(updated)).style(Style::default().fg(theme.fg_dim)),
            ])
            .style(row_style)
        })
        .collect();

    let title = if query.is_empty() {
        format!(" Helm Releases ({total}) ")
    } else {
        let all = cluster.helm_releases.len();
        format!(" Helm Releases ({total} of {all}) [/{query}]")
    };
    let block = content_block(&title, focused);

    let table = Table::new(
        rows,
        [
            ratatui::layout::Constraint::Percentage(18),
            ratatui::layout::Constraint::Percentage(14),
            ratatui::layout::Constraint::Percentage(20),
            ratatui::layout::Constraint::Percentage(10),
            ratatui::layout::Constraint::Percentage(14),
            ratatui::layout::Constraint::Percentage(8),
            ratatui::layout::Constraint::Percentage(16),
        ],
    )
    .header(header)
    .block(block)
    .row_highlight_style(theme.selection_style())
    .highlight_symbol(theme.highlight_symbol())
    .highlight_spacing(HighlightSpacing::Always);

    let mut state = TableState::default().with_selected(Some(window.selected));
    frame.render_stateful_widget(table, area, &mut state);
    render_table_scrollbar(frame, area, total, selected);
}

/// Renders the Helm repositories table (local config from ~/.config/helm/repositories.yaml).
pub fn render_helm_repos(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    search_query: &str,
    focused: bool,
) {
    let theme = default_theme();
    let query = search_query.trim();

    let indices = cached_filter_indices(
        AppView::HelmCharts,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.helm_repositories, cluster.snapshot_version),
        |q| filtered_helm_repo_indices(&cluster.helm_repositories, q),
    );

    let header = Row::new([
        Cell::from(Span::styled("  NAME", theme.header_style())),
        Cell::from(Span::styled("URL", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    if indices.is_empty() {
        let empty_msg = loading_or_empty_message(
            cluster,
            AppView::HelmCharts,
            query,
            "Loading Helm repositories...",
            "No Helm repositories configured (helm repo add <name> <url>)",
            "No repositories match search",
        );
        let title = if query.is_empty() {
            " Helm Repositories (0) ".to_string()
        } else {
            let all = cluster.helm_repositories.len();
            format!(" Helm Repositories (0 of {all}) [/{query}]")
        };
        frame.render_widget(
            ratatui::widgets::Paragraph::new(Span::styled(
                format!("  {empty_msg}"),
                Style::default().fg(theme.fg_dim),
            ))
            .block(content_block(&title, focused)),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &repo_idx)| {
            let idx = window.start + local_idx;
            let repo = &cluster.helm_repositories[repo_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", repo.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    repo.url.clone(),
                    Style::default().fg(theme.accent2),
                )),
            ])
            .style(row_style)
        })
        .collect();

    let title = if query.is_empty() {
        format!(" Helm Repositories ({total}) ")
    } else {
        let all = cluster.helm_repositories.len();
        format!(" Helm Repositories ({total} of {all}) [/{query}]")
    };
    let block = content_block(&title, focused);

    let table = Table::new(
        rows,
        [
            ratatui::layout::Constraint::Percentage(30),
            ratatui::layout::Constraint::Percentage(70),
        ],
    )
    .header(header)
    .block(block)
    .row_highlight_style(theme.selection_style())
    .highlight_symbol(theme.highlight_symbol())
    .highlight_spacing(HighlightSpacing::Always);

    let mut state = TableState::default().with_selected(Some(window.selected));
    frame.render_stateful_widget(table, area, &mut state);
    render_table_scrollbar(frame, area, total, selected);
}

fn render_table_scrollbar(frame: &mut Frame, area: Rect, total: usize, selected: usize) {
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
