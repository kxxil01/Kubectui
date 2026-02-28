//! FluxCD resources views.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, LazyLock, Mutex},
};

use chrono::{DateTime, Utc};
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
    k8s::dtos::FluxResourceInfo,
    state::ClusterSnapshot,
    ui::{
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices, data_fingerprint},
        loading_or_empty_message, table_viewport_rows, table_window,
    },
};

const MAX_FORMATTED_CACHE_ENTRIES: usize = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FluxMode {
    AlertProviders,
    Alerts,
    All,
    Artifacts,
    HelmReleases,
    Images,
    Kustomizations,
    Receivers,
    Sources,
}

impl FluxMode {
    fn from_view(view: AppView) -> Option<Self> {
        match view {
            AppView::FluxCDAlertProviders => Some(Self::AlertProviders),
            AppView::FluxCDAlerts => Some(Self::Alerts),
            AppView::FluxCDAll => Some(Self::All),
            AppView::FluxCDArtifacts => Some(Self::Artifacts),
            AppView::FluxCDHelmReleases => Some(Self::HelmReleases),
            AppView::FluxCDImages => Some(Self::Images),
            AppView::FluxCDKustomizations => Some(Self::Kustomizations),
            AppView::FluxCDReceivers => Some(Self::Receivers),
            AppView::FluxCDSources => Some(Self::Sources),
            _ => None,
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::AlertProviders => "Alert Providers",
            Self::Alerts => "Alerts",
            Self::All => "All",
            Self::Artifacts => "Artifacts",
            Self::HelmReleases => "HelmReleases",
            Self::Images => "Images",
            Self::Kustomizations => "Kustomizations",
            Self::Receivers => "Receivers",
            Self::Sources => "Sources",
        }
    }

    const fn loading_text(self) -> &'static str {
        match self {
            Self::AlertProviders => "  Loading FluxCD alert providers...",
            Self::Alerts => "  Loading FluxCD alerts...",
            Self::All => "  Loading FluxCD resources...",
            Self::Artifacts => "  Loading FluxCD artifacts...",
            Self::HelmReleases => "  Loading FluxCD helmreleases...",
            Self::Images => "  Loading FluxCD image resources...",
            Self::Kustomizations => "  Loading FluxCD kustomizations...",
            Self::Receivers => "  Loading FluxCD receivers...",
            Self::Sources => "  Loading FluxCD sources...",
        }
    }

    const fn empty_text(self) -> &'static str {
        match self {
            Self::AlertProviders => "  No FluxCD alert providers found",
            Self::Alerts => "  No FluxCD alerts found",
            Self::All => "  No FluxCD resources found",
            Self::Artifacts => "  No FluxCD artifacts found",
            Self::HelmReleases => "  No FluxCD helmreleases found",
            Self::Images => "  No FluxCD image resources found",
            Self::Kustomizations => "  No FluxCD kustomizations found",
            Self::Receivers => "  No FluxCD receivers found",
            Self::Sources => "  No FluxCD sources found",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FluxFormattedCacheKey {
    view: AppView,
    snapshot_version: u64,
    minute_bucket: i64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct FluxFormattedRow {
    name: String,
    namespace: String,
    kind: String,
    status: String,
    age: String,
    message: String,
    artifact: String,
}

#[derive(Debug, Default)]
struct FluxFormattedCache {
    map: HashMap<FluxFormattedCacheKey, Arc<Vec<FluxFormattedRow>>>,
    order: VecDeque<FluxFormattedCacheKey>,
}

impl FluxFormattedCache {
    fn get(&mut self, key: &FluxFormattedCacheKey) -> Option<Arc<Vec<FluxFormattedRow>>> {
        let value = self.map.get(key).cloned();
        if value.is_some() {
            self.touch(key);
        }
        value
    }

    fn insert(&mut self, key: FluxFormattedCacheKey, value: Arc<Vec<FluxFormattedRow>>) {
        if self.map.contains_key(&key) {
            self.map.insert(key.clone(), value);
            self.touch(&key);
            return;
        }
        self.map.insert(key.clone(), value);
        self.order.push_back(key);
        self.evict_if_needed();
    }

    fn touch(&mut self, key: &FluxFormattedCacheKey) {
        if let Some(pos) = self.order.iter().position(|item| item == key) {
            self.order.remove(pos);
            self.order.push_back(key.clone());
        }
    }

    fn evict_if_needed(&mut self) {
        while self.order.len() > MAX_FORMATTED_CACHE_ENTRIES {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
    }
}

static FLUX_FORMATTED_CACHE: LazyLock<Mutex<FluxFormattedCache>> =
    LazyLock::new(|| Mutex::new(FluxFormattedCache::default()));

pub fn filtered_flux_indices_for_view(
    view: AppView,
    cluster: &ClusterSnapshot,
    query: &str,
) -> Arc<Vec<usize>> {
    let Some(mode) = FluxMode::from_view(view) else {
        return Arc::new(Vec::new());
    };
    let query = query.trim();

    cached_filter_indices(
        view,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.flux_resources),
        |q| {
            cluster
                .flux_resources
                .iter()
                .enumerate()
                .filter_map(|(idx, resource)| {
                    if resource_matches_mode(mode, resource) && resource_matches_query(resource, q)
                    {
                        Some(idx)
                    } else {
                        None
                    }
                })
                .collect()
        },
    )
}

fn cached_formatted_rows(
    view: AppView,
    cluster: &ClusterSnapshot,
    now_unix: i64,
) -> Arc<Vec<FluxFormattedRow>> {
    let key = FluxFormattedCacheKey {
        view,
        snapshot_version: cluster.snapshot_version,
        minute_bucket: now_unix.div_euclid(60),
        data_fingerprint: data_fingerprint(&cluster.flux_resources),
    };

    if let Ok(mut cache) = FLUX_FORMATTED_CACHE.lock()
        && let Some(hit) = cache.get(&key)
    {
        return hit;
    }

    let built: Arc<Vec<FluxFormattedRow>> = Arc::new(
        cluster
            .flux_resources
            .iter()
            .map(|resource| FluxFormattedRow {
                name: resource.name.clone(),
                namespace: resource
                    .namespace
                    .clone()
                    .unwrap_or_else(|| "<cluster-scope>".to_string()),
                kind: resource.kind.clone(),
                status: resource.status.clone(),
                age: format_age_from_created(resource.created_at, now_unix),
                message: truncate_message(resource.message.as_deref().unwrap_or("-"), 56),
                artifact: truncate_message(resource.artifact.as_deref().unwrap_or("-"), 56),
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = FLUX_FORMATTED_CACHE.lock() {
        cache.insert(key, built.clone());
    }
    built
}

/// Renders FluxCD view content for a specific FluxCD command style.
pub fn render_flux_resources(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
    view: AppView,
) {
    let Some(mode) = FluxMode::from_view(view) else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "  FluxCD view is not available",
                default_theme().inactive_style(),
            ))
            .block(default_block("FluxCD")),
            area,
        );
        return;
    };

    let query = query.trim();
    let indices = filtered_flux_indices_for_view(view, cluster, query);

    let theme = default_theme();
    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            query,
            mode.loading_text(),
            mode.empty_text(),
            "  No FluxCD resources match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block(&format!("FluxCD · {}", mode.title()))),
            area,
        );
        return;
    }

    let total = indices.len();
    let mode_total = if query.is_empty() {
        total
    } else {
        filtered_flux_indices_for_view(view, cluster, "").len()
    };
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));
    let formatted_rows = cached_formatted_rows(view, cluster, Utc::now().timestamp());
    let detail_col_name = if mode == FluxMode::Artifacts {
        "Artifact"
    } else {
        "Message"
    };

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Kind", theme.header_style())),
        Cell::from(Span::styled("Status", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
        Cell::from(Span::styled(detail_col_name, theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &resource_idx)| {
            let idx = window.start + local_idx;
            let resource = &formatted_rows[resource_idx];
            let detail = if mode == FluxMode::Artifacts {
                resource.artifact.as_str()
            } else {
                resource.message.as_str()
            };
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", resource.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    resource.namespace.as_str(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    resource.kind.as_str(),
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(Span::styled(
                    resource.status.as_str(),
                    status_style(&resource.status, &theme),
                )),
                Cell::from(Span::styled(resource.age.as_str(), theme.inactive_style())),
                Cell::from(Span::styled(detail, Style::default().fg(theme.fg_dim))),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));
    let title = format!(" 🌀 FluxCD · {} ({total}) ", mode.title());
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        active_block(&format!(
            " 🌀 FluxCD · {} ({total} of {mode_total}) [/{query}]",
            mode.title()
        ))
    };

    let table = Table::new(
        rows,
        [
            Constraint::Min(22),
            Constraint::Length(18),
            Constraint::Length(18),
            Constraint::Length(11),
            Constraint::Length(9),
            Constraint::Min(28),
        ],
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

fn resource_matches_mode(mode: FluxMode, resource: &FluxResourceInfo) -> bool {
    match mode {
        FluxMode::AlertProviders => {
            resource.group == "notification.toolkit.fluxcd.io" && resource.kind == "AlertProvider"
        }
        FluxMode::Alerts => {
            resource.group == "notification.toolkit.fluxcd.io" && resource.kind == "Alert"
        }
        FluxMode::All => true,
        FluxMode::Artifacts => resource.artifact.is_some(),
        FluxMode::HelmReleases => {
            resource.group == "helm.toolkit.fluxcd.io" && resource.kind == "HelmRelease"
        }
        FluxMode::Images => resource.group == "image.toolkit.fluxcd.io",
        FluxMode::Kustomizations => {
            resource.group == "kustomize.toolkit.fluxcd.io" && resource.kind == "Kustomization"
        }
        FluxMode::Receivers => {
            resource.group == "notification.toolkit.fluxcd.io" && resource.kind == "Receiver"
        }
        FluxMode::Sources => resource.group == "source.toolkit.fluxcd.io",
    }
}

fn resource_matches_query(resource: &FluxResourceInfo, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    contains_ci(&resource.name, query)
        || contains_ci(resource.namespace.as_deref().unwrap_or_default(), query)
        || contains_ci(&resource.kind, query)
        || contains_ci(&resource.status, query)
        || contains_ci(resource.message.as_deref().unwrap_or_default(), query)
        || contains_ci(resource.artifact.as_deref().unwrap_or_default(), query)
}

fn status_style(status: &str, theme: &crate::ui::theme::Theme) -> Style {
    if status.eq_ignore_ascii_case("ready") {
        theme.badge_success_style()
    } else if status.eq_ignore_ascii_case("notready") {
        theme.badge_error_style()
    } else if status.eq_ignore_ascii_case("suspended") {
        theme.badge_warning_style()
    } else {
        theme.inactive_style()
    }
}

fn truncate_message(message: &str, max_chars: usize) -> String {
    if message.chars().count() <= max_chars {
        return message.to_string();
    }
    let mut out = message
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    out.push_str("...");
    out
}

fn format_age_from_created(created_at: Option<DateTime<Utc>>, now_unix: i64) -> String {
    let Some(created_at) = created_at else {
        return "-".to_string();
    };

    let age_secs = now_unix.saturating_sub(created_at.timestamp());
    let days = age_secs / 86_400;
    let hours = (age_secs % 86_400) / 3_600;
    let mins = (age_secs % 3_600) / 60;

    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Theme;

    #[test]
    fn status_style_maps_expected_levels() {
        let theme = Theme::dark();
        assert_eq!(status_style("Ready", &theme).fg, Some(theme.success));
        assert_eq!(status_style("NotReady", &theme).fg, Some(theme.error));
        assert_eq!(status_style("Suspended", &theme).fg, Some(theme.warning));
    }
}
