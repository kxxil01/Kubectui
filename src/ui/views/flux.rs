//! FluxCD resources views.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, LazyLock, Mutex},
};

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Paragraph, Row},
};

use crate::{
    app::{AppView, ResourceRef, WorkloadSortColumn, WorkloadSortState, filtered_workload_indices},
    bookmarks::BookmarkEntry,
    icons::view_icon,
    k8s::dtos::FluxResourceInfo,
    state::ClusterSnapshot,
    time::now_unix_seconds,
    ui::{
        TableFrame, bookmarked_name_cell,
        components::{content_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices_with_variant, data_fingerprint},
        render_centered_message, render_table_frame, sort_header_cell, table_viewport_rows,
        table_window,
        views::filtering::age_duration_now,
        workload_sort_suffix,
    },
};

const MAX_FORMATTED_CACHE_ENTRIES: usize = 96;
const NARROW_FLUX_WIDTH: u16 = 120;

fn flux_widths(area: Rect) -> [Constraint; 7] {
    if area.width < NARROW_FLUX_WIDTH {
        [
            Constraint::Min(18),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Min(18),
        ]
    } else {
        [
            Constraint::Min(22),
            Constraint::Length(18),
            Constraint::Length(18),
            Constraint::Length(13),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Min(24),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FluxMode {
    AlertProviders,
    Alerts,
    All,
    Artifacts,
    HelmReleases,
    HelmRepositories,
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
            AppView::FluxCDHelmRepositories => Some(Self::HelmRepositories),
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
            Self::HelmRepositories => "HelmRepositories",
            Self::Images => "Images",
            Self::Kustomizations => "Kustomizations",
            Self::Receivers => "Receivers",
            Self::Sources => "Sources",
        }
    }

    const fn loading_text(self) -> &'static str {
        match self {
            Self::AlertProviders => "Loading FluxCD alert providers...",
            Self::Alerts => "Loading FluxCD alerts...",
            Self::All => "Loading FluxCD resources...",
            Self::Artifacts => "Loading FluxCD artifacts...",
            Self::HelmReleases => "Loading FluxCD helmreleases...",
            Self::HelmRepositories => "Loading FluxCD helmrepositories...",
            Self::Images => "Loading FluxCD image resources...",
            Self::Kustomizations => "Loading FluxCD kustomizations...",
            Self::Receivers => "Loading FluxCD receivers...",
            Self::Sources => "Loading FluxCD sources...",
        }
    }

    const fn empty_text(self) -> &'static str {
        match self {
            Self::AlertProviders => "No FluxCD alert providers found",
            Self::Alerts => "No FluxCD alerts found",
            Self::All => "No FluxCD resources found",
            Self::Artifacts => "No FluxCD artifacts found",
            Self::HelmReleases => "No FluxCD helmreleases found",
            Self::HelmRepositories => "No FluxCD helmrepositories found",
            Self::Images => "No FluxCD image resources found",
            Self::Kustomizations => "No FluxCD kustomizations found",
            Self::Receivers => "No FluxCD receivers found",
            Self::Sources => "No FluxCD sources found",
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
    last_reconcile: String,
    gen_mismatch: bool,
    message: String,
    artifact: String,
    source_url: String,
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
        if self.order.back().is_some_and(|k| k == key) {
            return;
        }
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
    sort: Option<WorkloadSortState>,
) -> Arc<Vec<usize>> {
    let Some(mode) = FluxMode::from_view(view) else {
        return Arc::new(Vec::new());
    };
    let query = query.trim();

    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    cached_filter_indices_with_variant(
        view,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.flux_resources, cluster.snapshot_version),
        cache_variant,
        |q| {
            filtered_workload_indices(
                &cluster.flux_resources,
                q,
                sort,
                |resource, needle| {
                    resource_matches_mode(mode, resource)
                        && resource_matches_query(resource, needle)
                },
                |resource| resource.name.as_str(),
                |resource| resource.namespace.as_deref().unwrap_or(""),
                |resource| resource.created_at.map(age_duration_now),
            )
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
        data_fingerprint: data_fingerprint(&cluster.flux_resources, cluster.snapshot_version),
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
                age: crate::ui::format_age_from_timestamp(resource.created_at, now_unix),
                last_reconcile: crate::ui::format_age_from_timestamp(
                    resource.last_reconcile_time,
                    now_unix,
                ),
                gen_mismatch: resource
                    .observed_generation
                    .zip(resource.generation)
                    .is_some_and(|(obs, cur)| obs < cur),
                message: crate::ui::truncate_message(
                    resource.message.as_deref().unwrap_or("-"),
                    56,
                )
                .into_owned(),
                artifact: crate::ui::truncate_message(
                    resource.artifact.as_deref().unwrap_or("-"),
                    56,
                )
                .into_owned(),
                source_url: crate::ui::truncate_message(
                    resource.source_url.as_deref().unwrap_or("-"),
                    56,
                )
                .into_owned(),
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = FLUX_FORMATTED_CACHE.lock() {
        cache.insert(key, built.clone());
    }
    built
}

/// Renders FluxCD view content for a specific FluxCD command style.
#[allow(clippy::too_many_arguments)]
pub fn render_flux_resources(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    query: &str,
    view: AppView,
    sort: Option<WorkloadSortState>,
    focused: bool,
) {
    let Some(mode) = FluxMode::from_view(view) else {
        frame.render_widget(
            Paragraph::new(ratatui::text::Line::from(vec![
                Span::styled("○ ", Style::default().fg(default_theme().fg_dim)),
                Span::styled(
                    "FluxCD view is not available",
                    default_theme().inactive_style(),
                ),
            ]))
            .alignment(ratatui::layout::Alignment::Center)
            .block(content_block("FluxCD", focused)),
            area,
        );
        return;
    };

    let query = query.trim();
    let indices = filtered_flux_indices_for_view(view, cluster, query, sort);

    let theme = default_theme();
    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            view,
            query,
            &format!("FluxCD · {}", mode.title()),
            mode.loading_text(),
            mode.empty_text(),
            "No FluxCD resources match the search query",
            focused,
        );
        return;
    }

    let total = indices.len();
    let mode_total = if query.is_empty() {
        total
    } else {
        filtered_flux_indices_for_view(view, cluster, "", sort).len()
    };
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));
    let formatted_rows = cached_formatted_rows(view, cluster, now_unix_seconds());
    let detail_col_name = match mode {
        FluxMode::Artifacts => "Artifact",
        FluxMode::HelmRepositories => "URL",
        _ => "Message",
    };
    let header = Row::new([
        sort_header_cell("Name", sort, WorkloadSortColumn::Name, &theme, true),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Kind", theme.header_style())),
        Cell::from(Span::styled("Status", theme.header_style())),
        Cell::from(Span::styled("Reconcile", theme.header_style())),
        sort_header_cell("Age", sort, WorkloadSortColumn::Age, &theme, false),
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
            let raw_resource = &cluster.flux_resources[resource_idx];
            let detail = match mode {
                FluxMode::Artifacts => resource.artifact.as_str(),
                FluxMode::HelmRepositories => resource.source_url.as_str(),
                _ => resource.message.as_str(),
            };
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let status_display = if resource.gen_mismatch {
                format!("{} ⟳", resource.status)
            } else {
                resource.status.clone()
            };
            Row::new(vec![
                bookmarked_name_cell(
                    || ResourceRef::CustomResource {
                        name: raw_resource.name.clone(),
                        namespace: raw_resource.namespace.clone(),
                        group: raw_resource.group.clone(),
                        version: raw_resource.version.clone(),
                        kind: raw_resource.kind.clone(),
                        plural: raw_resource.plural.clone(),
                    },
                    bookmarks,
                    resource.name.as_str(),
                    Style::default().fg(theme.fg),
                    &theme,
                ),
                Cell::from(Span::styled(
                    resource.namespace.as_str(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    resource.kind.as_str(),
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(Span::styled(
                    status_display,
                    status_style(&resource.status, &theme),
                )),
                Cell::from(Span::styled(
                    resource.last_reconcile.as_str(),
                    theme.inactive_style(),
                )),
                Cell::from(Span::styled(resource.age.as_str(), theme.inactive_style())),
                Cell::from(Span::styled(detail, Style::default().fg(theme.fg_dim))),
            ])
            .style(row_style)
        })
        .collect();

    let sort_suffix = workload_sort_suffix(sort);
    let icon = view_icon(AppView::FluxCDAll).active();
    let title = if query.is_empty() {
        format!(" {icon}FluxCD · {} ({total}){sort_suffix} ", mode.title())
    } else {
        format!(
            " {icon}FluxCD · {} ({total} of {mode_total}) [/{query}]{sort_suffix}",
            mode.title()
        )
    };
    let widths = flux_widths(area);

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
        FluxMode::HelmRepositories => {
            resource.group == "source.toolkit.fluxcd.io" && resource.kind == "HelmRepository"
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
        || contains_ci(resource.source_url.as_deref().unwrap_or_default(), query)
}

fn status_style(status: &str, theme: &crate::ui::theme::Theme) -> Style {
    if status.eq_ignore_ascii_case("ready") {
        theme.badge_success_style()
    } else if status.eq_ignore_ascii_case("stalled") {
        Style::default()
            .fg(theme.error)
            .add_modifier(ratatui::prelude::Modifier::BOLD)
    } else if status.eq_ignore_ascii_case("notready") {
        theme.badge_error_style()
    } else if status.eq_ignore_ascii_case("suspended") {
        theme.badge_warning_style()
    } else {
        theme.inactive_style()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::dtos::FluxResourceInfo;
    use crate::ui::theme::Theme;

    #[test]
    fn status_style_maps_expected_levels() {
        let theme = Theme::dark();
        assert_eq!(status_style("Ready", &theme).fg, Some(theme.success));
        assert_eq!(status_style("NotReady", &theme).fg, Some(theme.error));
        assert_eq!(status_style("Suspended", &theme).fg, Some(theme.warning));
    }

    #[test]
    fn flux_widths_switch_to_compact_profile() {
        let widths = flux_widths(Rect::new(0, 0, 104, 20));
        assert_eq!(widths[0], Constraint::Min(18));
        assert_eq!(widths[1], Constraint::Length(14));
        assert_eq!(widths[6], Constraint::Min(18));
    }

    #[test]
    fn flux_widths_keep_wide_profile() {
        let widths = flux_widths(Rect::new(0, 0, 140, 20));
        assert_eq!(widths[0], Constraint::Min(22));
        assert_eq!(widths[1], Constraint::Length(18));
        assert_eq!(widths[6], Constraint::Min(24));
    }

    #[test]
    fn from_view_maps_helm_repositories() {
        assert_eq!(
            FluxMode::from_view(AppView::FluxCDHelmRepositories),
            Some(FluxMode::HelmRepositories)
        );
    }

    #[test]
    fn helm_repositories_mode_matches_only_helm_repository_kind() {
        let mut repo = FluxResourceInfo::default();
        repo.group = "source.toolkit.fluxcd.io".to_string();
        repo.kind = "HelmRepository".to_string();
        assert!(resource_matches_mode(FluxMode::HelmRepositories, &repo));

        let mut release = FluxResourceInfo::default();
        release.group = "helm.toolkit.fluxcd.io".to_string();
        release.kind = "HelmRelease".to_string();
        assert!(!resource_matches_mode(FluxMode::HelmRepositories, &release));
    }

    #[test]
    fn resource_query_matches_source_url() {
        let resource = FluxResourceInfo {
            source_url: Some("https://charts.example.com".to_string()),
            ..FluxResourceInfo::default()
        };
        assert!(resource_matches_query(&resource, "charts.example.com"));
    }

    #[test]
    fn flux_helm_view_filters_by_kind() {
        let snapshot = ClusterSnapshot {
            snapshot_version: 1,
            flux_resources: vec![
                FluxResourceInfo {
                    name: "hr-default".to_string(),
                    namespace: Some("default".to_string()),
                    group: "helm.toolkit.fluxcd.io".to_string(),
                    kind: "HelmRelease".to_string(),
                    ..FluxResourceInfo::default()
                },
                FluxResourceInfo {
                    name: "hr-demo".to_string(),
                    namespace: Some("demo".to_string()),
                    group: "helm.toolkit.fluxcd.io".to_string(),
                    kind: "HelmRelease".to_string(),
                    ..FluxResourceInfo::default()
                },
                FluxResourceInfo {
                    name: "ks-demo".to_string(),
                    namespace: Some("demo".to_string()),
                    group: "kustomize.toolkit.fluxcd.io".to_string(),
                    kind: "Kustomization".to_string(),
                    ..FluxResourceInfo::default()
                },
            ],
            ..ClusterSnapshot::default()
        };

        let filtered =
            filtered_flux_indices_for_view(AppView::FluxCDHelmReleases, &snapshot, "", None);
        assert_eq!(filtered.len(), 2);
        assert!(
            filtered
                .iter()
                .all(|idx| snapshot.flux_resources[*idx].kind == "HelmRelease")
        );
    }

    #[test]
    fn flux_kustomizations_view_filters_by_kind() {
        let snapshot = ClusterSnapshot {
            snapshot_version: 1,
            flux_resources: vec![
                FluxResourceInfo {
                    name: "ks-default".to_string(),
                    namespace: Some("default".to_string()),
                    group: "kustomize.toolkit.fluxcd.io".to_string(),
                    kind: "Kustomization".to_string(),
                    ..FluxResourceInfo::default()
                },
                FluxResourceInfo {
                    name: "ks-demo".to_string(),
                    namespace: Some("demo".to_string()),
                    group: "kustomize.toolkit.fluxcd.io".to_string(),
                    kind: "Kustomization".to_string(),
                    ..FluxResourceInfo::default()
                },
                FluxResourceInfo {
                    name: "hr-default".to_string(),
                    namespace: Some("default".to_string()),
                    group: "helm.toolkit.fluxcd.io".to_string(),
                    kind: "HelmRelease".to_string(),
                    ..FluxResourceInfo::default()
                },
            ],
            ..ClusterSnapshot::default()
        };

        let filtered =
            filtered_flux_indices_for_view(AppView::FluxCDKustomizations, &snapshot, "", None);
        assert_eq!(filtered.len(), 2);
    }
}
