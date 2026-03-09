//! User interface composition and rendering utilities.

pub mod components;
mod filter_cache;
pub mod profiling;
pub mod theme;
pub mod views;

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    prelude::Frame,
    text::{Line, Span},
    widgets::{
        Cell, HighlightSpacing, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState,
    },
};
use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

use crate::{
    app::{
        AppState, AppView, PodSortColumn, PodSortState, WorkloadSortColumn, WorkloadSortState,
        filtered_pod_indices,
    },
    policy::ViewAction,
    state::{ClusterSnapshot, ViewLoadState},
    ui::components::{active_block, default_block, default_theme},
};
use filter_cache::{cached_filter_indices_with_variant, data_fingerprint};

/// Case-insensitive substring match without allocating a new lowercase string.
#[inline]
pub(crate) fn contains_ci(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Formats small integer values without heap allocation for common cases.
#[inline]
pub(crate) fn format_small_int(value: i64) -> Cow<'static, str> {
    match value {
        0 => Cow::Borrowed("0"),
        1 => Cow::Borrowed("1"),
        2 => Cow::Borrowed("2"),
        3 => Cow::Borrowed("3"),
        4 => Cow::Borrowed("4"),
        5 => Cow::Borrowed("5"),
        6 => Cow::Borrowed("6"),
        7 => Cow::Borrowed("7"),
        8 => Cow::Borrowed("8"),
        9 => Cow::Borrowed("9"),
        10 => Cow::Borrowed("10"),
        _ => Cow::Owned(value.to_string()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TableWindow {
    pub start: usize,
    pub end: usize,
    pub selected: usize,
}

/// Computes how many table rows can be displayed inside a bordered table with a one-line header.
#[inline]
pub(crate) fn table_viewport_rows(area: Rect) -> usize {
    usize::from(area.height.saturating_sub(3)).max(1)
}

/// Computes the visible window for a selected row, centered when possible.
#[inline]
pub(crate) fn table_window(total: usize, selected: usize, viewport_rows: usize) -> TableWindow {
    if total == 0 {
        return TableWindow {
            start: 0,
            end: 0,
            selected: 0,
        };
    }
    let selected = selected.min(total.saturating_sub(1));
    let visible = viewport_rows.max(1).min(total);
    let mut start = selected.saturating_sub(visible / 2);
    let max_start = total.saturating_sub(visible);
    if start > max_start {
        start = max_start;
    }
    let end = start + visible;
    TableWindow {
        start,
        end,
        selected: selected.saturating_sub(start),
    }
}

pub(crate) fn responsive_table_widths<const N: usize>(
    area_width: u16,
    wide: [Constraint; N],
) -> [Constraint; N] {
    let usable_width = area_width.saturating_sub(3);
    let ideal_total = wide
        .iter()
        .map(|constraint| constraint_ideal_width(*constraint))
        .sum::<u16>();

    if usable_width >= ideal_total {
        return wide;
    }

    let mut percentages = [0u16; N];
    let total_weight = ideal_total.max(1) as u32;
    let mut assigned = 0u16;
    let mut remainders = [(0u32, 0usize); N];

    for (idx, constraint) in wide.iter().copied().enumerate() {
        let ideal = u32::from(constraint_ideal_width(constraint).max(1));
        let scaled = ideal * 100;
        let percentage = (scaled / total_weight) as u16;
        percentages[idx] = percentage;
        assigned = assigned.saturating_add(percentage);
        remainders[idx] = (scaled % total_weight, idx);
    }

    remainders.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    let remaining = 100u16.saturating_sub(assigned);
    for idx in 0..usize::from(remaining) {
        percentages[remainders[idx % N].1] = percentages[remainders[idx % N].1].saturating_add(1);
    }

    std::array::from_fn(|idx| Constraint::Percentage(percentages[idx]))
}

fn constraint_ideal_width(constraint: Constraint) -> u16 {
    match constraint {
        Constraint::Percentage(value) => value.max(1),
        Constraint::Ratio(numerator, denominator) => {
            if denominator == 0 {
                1
            } else {
                ((numerator.saturating_mul(100)) / denominator)
                    .try_into()
                    .unwrap_or(100)
            }
        }
        Constraint::Length(value) | Constraint::Min(value) | Constraint::Max(value) => value.max(1),
        Constraint::Fill(value) => value.max(1),
    }
}

pub(crate) fn loading_or_empty_message(
    snapshot: &ClusterSnapshot,
    view: AppView,
    query: &str,
    loading: &'static str,
    empty: &'static str,
    no_match: &'static str,
) -> &'static str {
    if matches!(
        snapshot.view_load_state(view),
        ViewLoadState::Idle | ViewLoadState::Loading | ViewLoadState::Refreshing
    ) {
        return loading;
    }
    if query.trim().is_empty() {
        empty
    } else {
        no_match
    }
}

pub(crate) fn loading_or_empty_message_no_search(
    snapshot: &ClusterSnapshot,
    view: AppView,
    loading: &'static str,
    empty: &'static str,
) -> &'static str {
    if matches!(
        snapshot.view_load_state(view),
        ViewLoadState::Idle | ViewLoadState::Loading | ViewLoadState::Refreshing
    ) {
        loading
    } else {
        empty
    }
}

fn effective_workbench_height(
    total_body_height: u16,
    requested_height: u16,
    open: bool,
    maximized: bool,
) -> u16 {
    if !open || total_body_height <= 12 {
        return 0;
    }

    if maximized {
        return total_body_height;
    }

    let max_height = total_body_height.saturating_sub(8);
    requested_height.min(max_height).max(6)
}

fn current_view_activity(snapshot: &ClusterSnapshot, view: AppView) -> Option<String> {
    match snapshot.view_load_state(view) {
        ViewLoadState::Loading => Some(format!("{} loading...", view.label())),
        ViewLoadState::Refreshing => Some(format!("{} refreshing...", view.label())),
        ViewLoadState::Idle | ViewLoadState::Ready => None,
    }
}

pub(crate) fn workload_sort_header(
    label: &str,
    sort: Option<WorkloadSortState>,
    column: WorkloadSortColumn,
) -> String {
    match sort {
        Some(WorkloadSortState {
            column: active,
            descending: true,
        }) if active == column => format!("{label}▼"),
        Some(WorkloadSortState {
            column: active,
            descending: false,
        }) if active == column => format!("{label}▲"),
        _ => label.to_string(),
    }
}

pub(crate) fn workload_sort_suffix(sort: Option<WorkloadSortState>) -> String {
    sort.map(|state| format!(" • sort: {}", state.short_label()))
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PodDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
    minute_bucket: i64,
}

#[derive(Debug, Clone)]
struct PodDerivedCell {
    age: String,
}

type PodDerivedCacheValue = Arc<Vec<PodDerivedCell>>;
static POD_DERIVED_CACHE: LazyLock<Mutex<Option<(PodDerivedCacheKey, PodDerivedCacheValue)>>> =
    LazyLock::new(|| Mutex::new(None));

fn cached_pod_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    now_unix: i64,
) -> PodDerivedCacheValue {
    let key = PodDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(&cluster.pods, cluster.snapshot_version),
        minute_bucket: now_unix / 60,
    };

    if let Ok(cache) = POD_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&pod_idx| {
                let pod = &cluster.pods[pod_idx];
                PodDerivedCell {
                    age: format_age_from_timestamp(pod.created_at, now_unix),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = POD_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

/// Renders a full frame for the current app and cluster state.
pub fn render(frame: &mut Frame, app: &AppState, cluster: &ClusterSnapshot) {
    let _frame_scope = profiling::frame_scope(app.view());
    let _render_scope = profiling::span_scope("render");
    let area = frame.area();

    // Guard against terminals too small to render the layout
    if area.height < 10 || area.width < 40 {
        let msg = format!(
            "Terminal too small ({}x{}). Need at least 40x10.",
            area.width, area.height
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, ratatui::style::Style::default())),
            area,
        );
        return;
    }

    let root = {
        let _layout_scope = profiling::span_scope("layout");
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(2),
            ])
            .split(frame.area())
    };

    {
        let _header_scope = profiling::span_scope("header");
        components::render_header(frame, root[0], "KubecTUI v0.1.0", cluster.cluster_summary());
    }

    let body_root = {
        let _body_layout_scope = profiling::span_scope("body_layout");
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(8),
                Constraint::Length(effective_workbench_height(
                    root[1].height,
                    app.workbench().height,
                    app.workbench().open,
                    app.workbench().maximized,
                )),
            ])
            .split(root[1])
    };

    let body = {
        let _body_layout_scope = profiling::span_scope("body_layout");
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(26), Constraint::Min(0)])
            .split(body_root[0])
    };

    {
        let _sidebar_scope = profiling::span_scope("sidebar");
        components::render_sidebar(
            frame,
            body[0],
            app.view(),
            app.sidebar_cursor,
            &app.collapsed_groups,
            app.focus,
        );
    }

    if app.workbench().open && body_root[1].height > 0 {
        let _workbench_scope = profiling::span_scope("workbench");
        components::render_workbench(frame, body_root[1], app, cluster);
    }

    let content = body[1];

    {
        let _view_scope = profiling::span_scope(app.view().profiling_key());
        match app.view() {
            AppView::Dashboard => views::dashboard::render_dashboard(frame, content, cluster),
            AppView::Nodes => views::nodes::render_nodes(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::Pods => {
                render_pods_widget(
                    frame,
                    content,
                    cluster,
                    app.selected_idx(),
                    app.search_query(),
                    app.pod_sort(),
                );
            }
            AppView::ReplicaSets => views::replicasets::render_replicasets(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::ReplicationControllers => {
                views::replication_controllers::render_replication_controllers(
                    frame,
                    content,
                    cluster,
                    app.selected_idx(),
                    app.search_query(),
                    app.workload_sort(),
                )
            }
            AppView::HelmCharts => views::helm::render_helm_repos(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
            ),
            AppView::HelmReleases => views::helm::render_helm_releases(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
            ),
            AppView::FluxCDAlertProviders
            | AppView::FluxCDAlerts
            | AppView::FluxCDAll
            | AppView::FluxCDArtifacts
            | AppView::FluxCDHelmReleases
            | AppView::FluxCDHelmRepositories
            | AppView::FluxCDImages
            | AppView::FluxCDKustomizations
            | AppView::FluxCDReceivers
            | AppView::FluxCDSources => views::flux::render_flux_resources(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.view(),
                app.workload_sort(),
            ),
            AppView::Endpoints => views::endpoints::render_endpoints(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
            ),
            AppView::Ingresses => views::ingresses::render_ingresses(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
            ),
            AppView::IngressClasses => views::ingresses::render_ingress_classes(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
            ),
            AppView::NetworkPolicies => views::network_policies::render_network_policies(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
            ),
            AppView::PortForwarding => views::port_forwarding::render_port_forwarding(
                frame,
                content,
                &app.tunnel_registry,
                app.selected_idx(),
                app.search_query(),
            ),
            AppView::ConfigMaps => views::config::render_config_maps(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
            ),
            AppView::Secrets => views::config::render_secrets(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
            ),
            AppView::HPAs => views::hpas::render_hpas(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
            ),
            AppView::PriorityClasses => views::priority_classes::render_priority_classes(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
            ),
            AppView::PersistentVolumeClaims => views::storage::render_pvcs(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::PersistentVolumes => views::storage::render_pvs(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::StorageClasses => views::storage::render_storage_classes(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::Namespaces => views::namespaces::render_namespaces(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
            ),
            AppView::Events => views::events::render_events(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
            ),
            AppView::Services => views::services::render_services(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::Deployments => views::deployments::render_deployments(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::StatefulSets => views::statefulsets::render_statefulsets(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::DaemonSets => views::daemonsets::render_daemonsets(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::Jobs => views::jobs::render_jobs(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::CronJobs => views::cronjobs::render_cronjobs(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::ServiceAccounts => views::security::service_accounts::render_service_accounts(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::Roles => views::security::roles::render_roles(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::RoleBindings => views::security::role_bindings::render_role_bindings(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::ClusterRoles => views::security::cluster_roles::render_cluster_roles(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::ClusterRoleBindings => {
                views::security::cluster_role_bindings::render_cluster_role_bindings(
                    frame,
                    content,
                    cluster,
                    app.selected_idx(),
                    app.search_query(),
                    app.workload_sort(),
                )
            }
            AppView::ResourceQuotas => views::governance::quotas::render_resource_quotas(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::LimitRanges => views::governance::limits::render_limit_ranges(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::PodDisruptionBudgets => views::governance::pdbs::render_pdbs(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
            ),
            AppView::Extensions => {
                views::extensions::render_extensions(frame, content, cluster, app)
            }
        }
    }

    let status = if let Some(err) = app.error_message() {
        format!("[{}] ERROR: {err}", app.get_namespace())
    } else if let Some(message) = app.status_message() {
        format!("[{}] {message}", app.get_namespace())
    } else if app.is_search_mode() {
        format!("[{}] Search: {}", app.get_namespace(), app.search_query())
    } else {
        let theme_name = theme::active_theme().name;
        let current_activity = current_view_activity(cluster, app.view())
            .map(|activity| format!(" {activity} •"))
            .unwrap_or_default();
        let sort_hint = if app.view() == AppView::Pods {
            let active = app.pod_sort().map_or("default", PodSortState::short_label);
            format!(" • [n/a] sort ({active}) • [1/2/3] pod-sort • [0] clear-sort")
        } else {
            let caps = app.view().shared_sort_capabilities();
            if caps.is_empty() {
                String::new()
            } else {
                let key_hint = if caps == [WorkloadSortColumn::Name] {
                    "[n]"
                } else {
                    "[n/a]"
                };
                let active = app
                    .workload_sort()
                    .map_or("default", WorkloadSortState::short_label);
                format!(" • {key_hint} sort ({active}) • [0] clear-sort")
            }
        };
        let flux_reconcile_hint = if app.detail_view.is_none()
            && app
                .view()
                .supports_view_action(ViewAction::SelectedFluxReconcile)
        {
            " • [R] reconcile"
        } else {
            ""
        };
        let workbench_hint = if app.workbench().open {
            " • [H] history • [b] workbench • [[]/]] tabs • [Ctrl+Up/Down] wb-size • [Ctrl+w] close-tab"
        } else {
            " • [H] history • [b] workbench"
        };
        format!(
            "[{}]{} [j/k] navigate • [/] search • [~] ns • [c] ctx • [T] theme:{theme_name}{sort_hint}{flux_reconcile_hint}{workbench_hint} • [r] refresh • [q] quit",
            app.get_namespace(),
            current_activity
        )
    };

    {
        let _status_scope = profiling::span_scope("status");
        components::render_status_bar(frame, root[2], &status, app.error_message().is_some());
    }

    if let Some(detail_state) = app.detail_view.as_ref() {
        let _detail_scope = profiling::span_scope("overlay.detail");
        views::detail::render_detail(frame, frame.area(), detail_state);
    }

    if app.is_namespace_picker_open() {
        let _namespace_scope = profiling::span_scope("overlay.namespace_picker");
        app.namespace_picker().render(frame, frame.area());
    }

    if app.is_context_picker_open() {
        let _context_scope = profiling::span_scope("overlay.context_picker");
        app.context_picker.render(frame, frame.area());
    }

    if app.command_palette.is_open() {
        let _command_scope = profiling::span_scope("overlay.command_palette");
        app.command_palette.render(frame, frame.area());
    }

    if app.confirm_quit {
        let _quit_scope = profiling::span_scope("overlay.quit_confirm");
        render_quit_confirm(frame, frame.area());
    }
}

fn render_quit_confirm(frame: &mut Frame, area: ratatui::layout::Rect) {
    use ratatui::{
        style::Modifier,
        text::Line,
        widgets::{Block, BorderType, Borders, Clear},
    };

    let theme = default_theme();

    let w = 36u16;
    let h = 5u16;
    let popup = ratatui::layout::Rect {
        x: (area.width.saturating_sub(w)) / 2,
        y: (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.badge_error_style())
        .style(ratatui::style::Style::default().bg(theme.bg));
    frame.render_widget(block, popup);

    let inner = ratatui::layout::Rect {
        x: popup.x + 1,
        y: popup.y + 1,
        width: popup.width.saturating_sub(2),
        height: popup.height.saturating_sub(2),
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(2)])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  Quit KubecTUI? ",
            theme.title_style(),
        )])),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "  [y/q/Enter] ",
                ratatui::style::Style::default()
                    .fg(theme.error)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("yes  ", theme.inactive_style()),
            Span::styled("[any] ", theme.keybind_key_style()),
            Span::styled("cancel", theme.keybind_desc_style()),
        ])),
        chunks[1],
    );
}

fn render_pods_widget(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
    pod_sort: Option<PodSortState>,
) {
    let theme = default_theme();
    let cache_variant = pod_sort.map_or(0, PodSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::Pods,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.pods, cluster.snapshot_version),
        cache_variant,
        |q| filtered_pod_indices(&cluster.pods, q, pod_sort),
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::Pods,
            query,
            "  Loading pods...",
            "  No pods available  (try pressing ~ to switch namespace, or select 'all')",
            "  No pods match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style())).block(default_block("Pods")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let name_header = match pod_sort {
        Some(PodSortState {
            column: PodSortColumn::Name,
            descending: true,
        }) => "Name▼",
        Some(PodSortState {
            column: PodSortColumn::Name,
            descending: false,
        }) => "Name▲",
        _ => "Name",
    };
    let age_header = match pod_sort {
        Some(PodSortState {
            column: PodSortColumn::Age,
            descending: true,
        }) => "Age▼",
        Some(PodSortState {
            column: PodSortColumn::Age,
            descending: false,
        }) => "Age▲",
        _ => "Age",
    };
    let status_header = match pod_sort {
        Some(PodSortState {
            column: PodSortColumn::Status,
            descending: true,
        }) => "Status▼",
        Some(PodSortState {
            column: PodSortColumn::Status,
            descending: false,
        }) => "Status▲",
        _ => "Status",
    };
    let restarts_header = match pod_sort {
        Some(PodSortState {
            column: PodSortColumn::Restarts,
            descending: true,
        }) => "Restarts▼",
        Some(PodSortState {
            column: PodSortColumn::Restarts,
            descending: false,
        }) => "Restarts▲",
        _ => "Restarts",
    };

    let header = Row::new([
        Cell::from(Span::styled(
            format!("  {name_header}"),
            theme.header_style(),
        )),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled(status_header, theme.header_style())),
        Cell::from(Span::styled("Node", theme.header_style())),
        Cell::from(Span::styled(restarts_header, theme.header_style())),
        Cell::from(Span::styled(age_header, theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());
    let name_style = ratatui::prelude::Style::default().fg(theme.fg);
    let dim_style = ratatui::prelude::Style::default().fg(theme.fg_dim);
    let now_unix = chrono::Utc::now().timestamp();
    let derived = cached_pod_derived(cluster, query, indices.as_ref(), now_unix);
    let mut rows: Vec<Row> = Vec::with_capacity(window.end.saturating_sub(window.start));
    for (local_idx, &pod_idx) in indices[window.start..window.end].iter().enumerate() {
        let idx = window.start + local_idx;
        let pod = &cluster.pods[pod_idx];
        let status = pod.status.as_str();
        let status_style = theme.get_status_style(status);
        let restart_style = if pod.restarts > 5 {
            theme.badge_error_style()
        } else if pod.restarts > 0 {
            theme.badge_warning_style()
        } else {
            theme.inactive_style()
        };
        let row_style = if idx.is_multiple_of(2) {
            ratatui::prelude::Style::default().bg(theme.bg)
        } else {
            theme.row_alt_style()
        };
        let age = derived
            .get(idx)
            .map(|cell| cell.age.as_str())
            .unwrap_or("-");

        rows.push(
            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled("  ", name_style),
                    Span::styled(pod.name.as_str(), name_style),
                ])),
                Cell::from(Span::styled(pod.namespace.as_str(), dim_style)),
                Cell::from(Span::styled(status, status_style)),
                Cell::from(Span::styled(
                    pod.node.as_deref().unwrap_or("n/a"),
                    dim_style,
                )),
                Cell::from(Span::styled(
                    format_small_int(i64::from(pod.restarts)),
                    restart_style,
                )),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style),
        );
    }

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let sort_suffix = pod_sort
        .map(|state| format!(" • sort: {}", state.short_label()))
        .unwrap_or_default();
    let title = format!(" 🐳 Pods ({total}){sort_suffix} ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.pods.len();
        active_block(&format!(
            " 🐳 Pods ({total} of {all}) [/{query}]{sort_suffix}"
        ))
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Min(28),
                Constraint::Length(18),
                Constraint::Length(20),
                Constraint::Length(22),
                Constraint::Length(10),
                Constraint::Length(9),
            ],
        ),
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

/// Formats a `Duration` as a human-readable age string (e.g. "3d 2h", "5h 12m", "7m").
pub fn format_age(age: Option<std::time::Duration>) -> String {
    let Some(age) = age else {
        return "-".to_string();
    };

    let secs = age.as_secs();
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;

    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

pub(crate) fn readiness_style(
    ready: i32,
    desired: i32,
    theme: &crate::ui::theme::Theme,
) -> ratatui::prelude::Style {
    if desired == 0 && ready == 0 {
        theme.inactive_style()
    } else if desired > 0 && ready >= desired {
        theme.badge_success_style()
    } else if ready > 0 {
        theme.badge_warning_style()
    } else {
        theme.badge_error_style()
    }
}

pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

#[inline]
fn format_age_from_timestamp(
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    now_unix: i64,
) -> String {
    let Some(created_at) = created_at else {
        return "-".to_string();
    };
    let age_secs = now_unix - created_at.timestamp();
    if age_secs < 0 {
        return "future".to_string();
    }
    let days = age_secs / 86_400;
    let hours = (age_secs % 86_400) / 3_600;
    let mins = (age_secs % 3_600) / 60;
    if days > 0 {
        format!("{days}d{hours}h")
    } else if hours > 0 {
        format!("{hours}h{mins}m")
    } else {
        format!("{mins}m")
    }
}

pub(crate) fn format_image(image: Option<&str>, max_len: usize) -> String {
    let Some(image) = image else {
        return "-".to_string();
    };
    if image.chars().count() <= max_len {
        image.to_string()
    } else {
        format!("{}...", &image.chars().take(max_len).collect::<String>())
    }
}
#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use crate::{
        app::{AppState, AppView, DetailMetadata, DetailViewState, ResourceRef},
        k8s::dtos::{
            ClusterRoleBindingInfo, ClusterRoleInfo, CronJobInfo, CustomResourceDefinitionInfo,
            CustomResourceInfo, DaemonSetInfo, DeploymentInfo, FluxResourceInfo, IngressClassInfo,
            IngressInfo, JobInfo, LimitRangeInfo, NetworkPolicyInfo, NodeInfo,
            PodDisruptionBudgetInfo, PodInfo, PvInfo, PvcInfo, ResourceQuotaInfo, RoleBindingInfo,
            RoleInfo, ServiceAccountInfo, ServiceInfo, StatefulSetInfo, StorageClassInfo,
        },
        state::{ClusterSnapshot, DataPhase, ViewLoadState},
    };

    use super::*;

    fn draw(app: &AppState, snapshot: &ClusterSnapshot) {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| render(frame, app, snapshot))
            .expect("render should not panic");
    }

    fn draw_with_size(app: &AppState, snapshot: &ClusterSnapshot, width: u16, height: u16) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| render(frame, app, snapshot))
            .expect("render should not panic");
    }

    fn app_with_view(view: AppView) -> AppState {
        let mut app = AppState::default();
        while app.view() != view {
            app.handle_key_event(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Tab,
            ));
        }
        app
    }

    #[test]
    fn table_window_keeps_selected_visible_near_top() {
        let window = table_window(100, 0, 10);
        assert_eq!(window.start, 0);
        assert_eq!(window.end, 10);
        assert_eq!(window.selected, 0);
    }

    #[test]
    fn table_window_centers_selection_in_middle() {
        let window = table_window(100, 50, 11);
        assert_eq!(window.start, 45);
        assert_eq!(window.end, 56);
        assert_eq!(window.selected, 5);
    }

    #[test]
    fn table_window_clamps_selection_near_bottom() {
        let window = table_window(100, 99, 10);
        assert_eq!(window.start, 90);
        assert_eq!(window.end, 100);
        assert_eq!(window.selected, 9);
    }

    #[test]
    fn table_window_handles_empty_lists() {
        let window = table_window(0, 0, 10);
        assert_eq!(window.start, 0);
        assert_eq!(window.end, 0);
        assert_eq!(window.selected, 0);
    }

    #[test]
    fn table_viewport_rows_has_minimum_one_row() {
        let area = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 2,
        };
        assert_eq!(table_viewport_rows(area), 1);
    }

    #[test]
    fn responsive_table_widths_preserves_wide_layouts_when_space_allows() {
        let wide = [
            Constraint::Length(24),
            Constraint::Length(16),
            Constraint::Length(9),
            Constraint::Min(20),
        ];

        assert_eq!(responsive_table_widths(96, wide), wide);
    }

    #[test]
    fn responsive_table_widths_falls_back_to_percentages_that_fill_width() {
        let widths = responsive_table_widths(
            40,
            [
                Constraint::Length(24),
                Constraint::Length(16),
                Constraint::Length(9),
                Constraint::Min(20),
            ],
        );

        assert!(
            widths
                .iter()
                .all(|constraint| matches!(constraint, Constraint::Percentage(_)))
        );
        let total: u16 = widths
            .iter()
            .map(|constraint| match constraint {
                Constraint::Percentage(value) => *value,
                _ => 0,
            })
            .sum();
        assert_eq!(total, 100);
    }

    #[test]
    fn loading_or_empty_message_respects_view_load_state_and_query() {
        let loading_snapshot = ClusterSnapshot::default();
        assert_eq!(
            loading_or_empty_message(
                &loading_snapshot,
                AppView::Pods,
                "",
                "loading",
                "empty",
                "no-match",
            ),
            "loading"
        );

        let mut loaded_snapshot = ClusterSnapshot {
            phase: DataPhase::Ready,
            ..ClusterSnapshot::default()
        };
        loaded_snapshot.view_load_states[AppView::Pods.index()] = ViewLoadState::Ready;
        assert_eq!(
            loading_or_empty_message(
                &loaded_snapshot,
                AppView::Pods,
                "",
                "loading",
                "empty",
                "no-match",
            ),
            "empty"
        );
        assert_eq!(
            loading_or_empty_message(
                &loaded_snapshot,
                AppView::Pods,
                "prod",
                "loading",
                "empty",
                "no-match",
            ),
            "no-match"
        );
    }

    #[test]
    fn loading_or_empty_message_no_search_respects_view_load_state() {
        let loading_snapshot = ClusterSnapshot::default();
        assert_eq!(
            loading_or_empty_message_no_search(
                &loading_snapshot,
                AppView::Nodes,
                "loading",
                "empty",
            ),
            "loading"
        );

        let mut loaded_snapshot = ClusterSnapshot {
            phase: DataPhase::Ready,
            ..ClusterSnapshot::default()
        };
        loaded_snapshot.view_load_states[AppView::Nodes.index()] = ViewLoadState::Ready;
        assert_eq!(
            loading_or_empty_message_no_search(
                &loaded_snapshot,
                AppView::Nodes,
                "loading",
                "empty",
            ),
            "empty"
        );
    }

    /// Verifies dashboard renders without panic for empty snapshot.
    #[test]
    fn render_dashboard_empty_snapshot_smoke() {
        let app = app_with_view(AppView::Dashboard);
        draw(&app, &ClusterSnapshot::default());
    }

    /// Verifies dashboard renders without panic for populated snapshot.
    #[test]
    fn render_dashboard_full_snapshot_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.nodes.push(NodeInfo {
            name: "n1".to_string(),
            ready: true,
            ..NodeInfo::default()
        });
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });
        snapshot.services.push(ServiceInfo {
            name: "svc".to_string(),
            namespace: "default".to_string(),
            type_: "ClusterIP".to_string(),
            ..ServiceInfo::default()
        });
        snapshot.deployments.push(DeploymentInfo {
            name: "dep".to_string(),
            namespace: "default".to_string(),
            ready: "1/1".to_string(),
            ..DeploymentInfo::default()
        });

        let app = app_with_view(AppView::Dashboard);
        draw(&app, &snapshot);
    }

    /// Verifies nodes view renders without panic for multiple list sizes.
    #[test]
    fn render_nodes_various_sizes_smoke() {
        let app = app_with_view(AppView::Nodes);

        for size in [0, 1, 100, 1000] {
            let mut snapshot = ClusterSnapshot::default();
            for i in 0..size {
                snapshot.nodes.push(NodeInfo {
                    name: format!("node-{i}"),
                    ready: i % 2 == 0,
                    role: if i % 3 == 0 { "master" } else { "worker" }.to_string(),
                    ..NodeInfo::default()
                });
            }
            draw(&app, &snapshot);
        }
    }

    #[test]
    fn render_pods_narrow_width_smoke() {
        let app = app_with_view(AppView::Pods);
        let mut snapshot = ClusterSnapshot::default();
        snapshot.view_load_states[AppView::Pods.index()] = ViewLoadState::Ready;
        snapshot.pods.push(PodInfo {
            name: "redpanda-0".to_string(),
            namespace: "staging".to_string(),
            status: "Running".to_string(),
            node: Some("gke-luxor-staging-redp".to_string()),
            restarts: 0,
            ..PodInfo::default()
        });
        snapshot.pods.push(PodInfo {
            name: "redpanda-console-7dc45cb5d8-4482g".to_string(),
            namespace: "staging".to_string(),
            status: "Running".to_string(),
            node: Some("gke-luxor-staging-redp".to_string()),
            restarts: 2,
            ..PodInfo::default()
        });

        draw_with_size(&app, &snapshot, 96, 20);
    }

    #[test]
    fn effective_workbench_height_preserves_main_content_budget() {
        assert_eq!(effective_workbench_height(10, 12, true, false), 0);
        assert_eq!(effective_workbench_height(20, 12, true, false), 12);
        assert_eq!(effective_workbench_height(20, 16, true, false), 12);
        assert_eq!(effective_workbench_height(20, 12, false, false), 0);
        // Maximized takes full height
        assert_eq!(effective_workbench_height(40, 12, true, true), 40);
        assert_eq!(effective_workbench_height(20, 12, true, true), 20);
        // Maximized but closed still returns 0
        assert_eq!(effective_workbench_height(20, 12, false, true), 0);
    }

    #[test]
    fn render_with_open_workbench_smoke() {
        let mut app = app_with_view(AppView::Dashboard);
        app.workbench.toggle_open();
        let snapshot = ClusterSnapshot::default();
        draw_with_size(&app, &snapshot, 120, 40);
    }

    /// Verifies services view renders without panic for mixed service types.
    #[test]
    fn render_services_mixed_types_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        for t in ["ClusterIP", "NodePort", "LoadBalancer", "ExternalName"] {
            snapshot.services.push(ServiceInfo {
                name: format!("svc-{t}"),
                namespace: "default".to_string(),
                type_: t.to_string(),
                ports: vec!["80/TCP".to_string(), "443/TCP".to_string()],
                ..ServiceInfo::default()
            });
        }

        let app = app_with_view(AppView::Services);
        draw(&app, &snapshot);
    }

    /// Verifies deployments view renders without panic for mixed health values.
    #[test]
    fn render_deployments_mixed_health_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        for ready in ["3/3", "1/3", "0/3"] {
            snapshot.deployments.push(DeploymentInfo {
                name: format!("dep-{ready}"),
                namespace: "default".to_string(),
                ready: ready.to_string(),
                ..DeploymentInfo::default()
            });
        }

        let app = app_with_view(AppView::Deployments);
        draw(&app, &snapshot);
    }

    /// Verifies StatefulSets view renders without panic for mixed readiness states.
    #[test]
    fn render_statefulsets_mixed_readiness_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.statefulsets.push(StatefulSetInfo {
            name: "db-ready".to_string(),
            namespace: "default".to_string(),
            desired_replicas: 3,
            ready_replicas: 3,
            service_name: "db-headless".to_string(),
            image: Some("postgres:16".to_string()),
            ..StatefulSetInfo::default()
        });
        snapshot.statefulsets.push(StatefulSetInfo {
            name: "db-partial".to_string(),
            namespace: "default".to_string(),
            desired_replicas: 3,
            ready_replicas: 1,
            service_name: "db-headless".to_string(),
            image: Some("postgres:16".to_string()),
            ..StatefulSetInfo::default()
        });

        let app = app_with_view(AppView::StatefulSets);
        draw(&app, &snapshot);
    }

    /// Verifies DaemonSets view renders without panic for mixed desired/ready/unavailable counts.
    #[test]
    fn render_daemonsets_mixed_counts_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.daemonsets.push(DaemonSetInfo {
            name: "agent-ok".to_string(),
            namespace: "kube-system".to_string(),
            desired_count: 10,
            ready_count: 10,
            unavailable_count: 0,
            image: Some("agent:1".to_string()),
            ..DaemonSetInfo::default()
        });
        snapshot.daemonsets.push(DaemonSetInfo {
            name: "agent-warn".to_string(),
            namespace: "kube-system".to_string(),
            desired_count: 10,
            ready_count: 8,
            unavailable_count: 2,
            image: Some("agent:2".to_string()),
            ..DaemonSetInfo::default()
        });

        let app = app_with_view(AppView::DaemonSets);
        draw(&app, &snapshot);
    }

    /// Verifies Jobs view renders without panic for mixed status values.
    #[test]
    fn render_jobs_mixed_status_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.jobs.push(JobInfo {
            name: "batch-success".to_string(),
            namespace: "default".to_string(),
            status: "Succeeded".to_string(),
            completions: "1/1".to_string(),
            ..JobInfo::default()
        });
        snapshot.jobs.push(JobInfo {
            name: "batch-running".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            completions: "0/1".to_string(),
            ..JobInfo::default()
        });

        let app = app_with_view(AppView::Jobs);
        draw(&app, &snapshot);
    }

    /// Verifies CronJobs view renders without panic for suspended and active rows.
    #[test]
    fn render_cronjobs_suspend_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.cronjobs.push(CronJobInfo {
            name: "nightly".to_string(),
            namespace: "default".to_string(),
            schedule: "0 0 * * *".to_string(),
            suspend: false,
            ..CronJobInfo::default()
        });
        snapshot.cronjobs.push(CronJobInfo {
            name: "paused".to_string(),
            namespace: "default".to_string(),
            schedule: "*/15 * * * *".to_string(),
            suspend: true,
            ..CronJobInfo::default()
        });

        let app = app_with_view(AppView::CronJobs);
        draw(&app, &snapshot);
    }

    /// Verifies ResourceQuotas governance view renders usage bands without panic.
    #[test]
    fn render_resource_quotas_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.resource_quotas.push(ResourceQuotaInfo {
            name: "rq-default".to_string(),
            namespace: "default".to_string(),
            percent_used: [("pods".to_string(), 85.0)].into_iter().collect(),
            ..ResourceQuotaInfo::default()
        });

        let app = app_with_view(AppView::ResourceQuotas);
        draw(&app, &snapshot);
    }

    /// Verifies LimitRanges governance view renders limits summary without panic.
    #[test]
    fn render_limit_ranges_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.limit_ranges.push(LimitRangeInfo {
            name: "limits-default".to_string(),
            namespace: "default".to_string(),
            ..LimitRangeInfo::default()
        });

        let app = app_with_view(AppView::LimitRanges);
        draw(&app, &snapshot);
    }

    /// Verifies PDB governance view renders disruption stats without panic.
    #[test]
    fn render_pdbs_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot
            .pod_disruption_budgets
            .push(PodDisruptionBudgetInfo {
                name: "web-pdb".to_string(),
                namespace: "default".to_string(),
                min_available: Some("1".to_string()),
                disruptions_allowed: 1,
                ..PodDisruptionBudgetInfo::default()
            });

        let app = app_with_view(AppView::PodDisruptionBudgets);
        draw(&app, &snapshot);
    }

    /// Verifies ServiceAccounts view renders without panic.
    #[test]
    fn render_service_accounts_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.service_accounts.push(ServiceAccountInfo {
            name: "default".to_string(),
            namespace: "default".to_string(),
            secrets_count: 1,
            image_pull_secrets_count: 0,
            automount_service_account_token: Some(true),
            ..ServiceAccountInfo::default()
        });

        let app = app_with_view(AppView::ServiceAccounts);
        draw(&app, &snapshot);
    }

    /// Verifies network-related views render without panic.
    #[test]
    fn render_network_views_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.ingresses.push(IngressInfo {
            name: "web".to_string(),
            namespace: "default".to_string(),
            class: Some("nginx".to_string()),
            hosts: vec!["app.example.test".to_string()],
            address: Some("10.0.0.10".to_string()),
            ports: vec!["80".to_string(), "443".to_string()],
            ..IngressInfo::default()
        });
        snapshot.ingress_classes.push(IngressClassInfo {
            name: "nginx".to_string(),
            controller: "k8s.io/ingress-nginx".to_string(),
            is_default: true,
            ..IngressClassInfo::default()
        });
        snapshot.network_policies.push(NetworkPolicyInfo {
            name: "deny-all".to_string(),
            namespace: "default".to_string(),
            pod_selector: "app=web".to_string(),
            ingress_rules: 0,
            egress_rules: 0,
            ..NetworkPolicyInfo::default()
        });

        draw(&app_with_view(AppView::Ingresses), &snapshot);
        draw(&app_with_view(AppView::IngressClasses), &snapshot);
        draw(&app_with_view(AppView::NetworkPolicies), &snapshot);
    }

    /// Verifies storage-related views render without panic.
    #[test]
    fn render_storage_views_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.pvcs.push(PvcInfo {
            name: "data-web-0".to_string(),
            namespace: "default".to_string(),
            status: "Bound".to_string(),
            volume: Some("pv-web-0".to_string()),
            capacity: Some("10Gi".to_string()),
            access_modes: vec!["ReadWriteOnce".to_string()],
            storage_class: Some("fast-ssd".to_string()),
            ..PvcInfo::default()
        });
        snapshot.pvs.push(PvInfo {
            name: "pv-web-0".to_string(),
            capacity: Some("10Gi".to_string()),
            access_modes: vec!["ReadWriteOnce".to_string()],
            reclaim_policy: "Delete".to_string(),
            status: "Bound".to_string(),
            claim: Some("default/data-web-0".to_string()),
            storage_class: Some("fast-ssd".to_string()),
            ..PvInfo::default()
        });
        snapshot.storage_classes.push(StorageClassInfo {
            name: "fast-ssd".to_string(),
            provisioner: "kubernetes.io/no-provisioner".to_string(),
            reclaim_policy: Some("Delete".to_string()),
            volume_binding_mode: Some("WaitForFirstConsumer".to_string()),
            allow_volume_expansion: true,
            is_default: true,
            ..StorageClassInfo::default()
        });

        draw(&app_with_view(AppView::PersistentVolumeClaims), &snapshot);
        draw(&app_with_view(AppView::PersistentVolumes), &snapshot);
        draw(&app_with_view(AppView::StorageClasses), &snapshot);
    }

    /// Verifies Roles view renders rule details without panic.
    #[test]
    fn render_roles_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.roles.push(RoleInfo {
            name: "reader".to_string(),
            namespace: "default".to_string(),
            ..RoleInfo::default()
        });

        let app = app_with_view(AppView::Roles);
        draw(&app, &snapshot);
    }

    /// Verifies RoleBindings view renders subject details without panic.
    #[test]
    fn render_role_bindings_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.role_bindings.push(RoleBindingInfo {
            name: "reader-binding".to_string(),
            namespace: "default".to_string(),
            role_ref_kind: "Role".to_string(),
            role_ref_name: "reader".to_string(),
            ..RoleBindingInfo::default()
        });

        let app = app_with_view(AppView::RoleBindings);
        draw(&app, &snapshot);
    }

    /// Verifies ClusterRoles and ClusterRoleBindings views render without panic.
    #[test]
    fn render_cluster_rbac_views_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.cluster_roles.push(ClusterRoleInfo {
            name: "cluster-admin".to_string(),
            ..ClusterRoleInfo::default()
        });
        snapshot.cluster_role_bindings.push(ClusterRoleBindingInfo {
            name: "cluster-admin-binding".to_string(),
            role_ref_kind: "ClusterRole".to_string(),
            role_ref_name: "cluster-admin".to_string(),
            ..ClusterRoleBindingInfo::default()
        });

        let app_roles = app_with_view(AppView::ClusterRoles);
        draw(&app_roles, &snapshot);

        let app_bindings = app_with_view(AppView::ClusterRoleBindings);
        draw(&app_bindings, &snapshot);
    }

    #[test]
    fn render_extensions_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot
            .custom_resource_definitions
            .push(CustomResourceDefinitionInfo {
                name: "widgets.demo.io".to_string(),
                group: "demo.io".to_string(),
                version: "v1".to_string(),
                kind: "Widget".to_string(),
                plural: "widgets".to_string(),
                scope: "Namespaced".to_string(),
                instances: 1,
            });

        let mut app = app_with_view(AppView::Extensions);
        app.extension_instances = vec![CustomResourceInfo {
            name: "sample".to_string(),
            namespace: Some("default".to_string()),
            ..CustomResourceInfo::default()
        }];

        draw(&app, &snapshot);
    }

    /// Verifies detail modal overlay renders on top of list view without panic.
    #[test]
    fn render_detail_overlay_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });

        let mut app = app_with_view(AppView::Pods);
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Pod("p1".to_string(), "default".to_string())),
            metadata: DetailMetadata {
                name: "p1".to_string(),
                namespace: Some("default".to_string()),
                ..DetailMetadata::default()
            },
            yaml: Some("kind: Pod\nmetadata:\n  name: p1\n".to_string()),
            ..DetailViewState::default()
        });

        draw(&app, &snapshot);
    }

    /// Verifies Extensions view renders with instance selection cursor without panic.
    #[test]
    fn render_extensions_with_instance_focus_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot
            .custom_resource_definitions
            .push(CustomResourceDefinitionInfo {
                name: "widgets.demo.io".to_string(),
                group: "demo.io".to_string(),
                version: "v1".to_string(),
                kind: "Widget".to_string(),
                plural: "widgets".to_string(),
                scope: "Namespaced".to_string(),
                instances: 2,
            });

        let mut app = app_with_view(AppView::Extensions);
        app.set_extension_instances(
            "widgets.demo.io".to_string(),
            vec![
                CustomResourceInfo {
                    name: "alpha".to_string(),
                    namespace: Some("default".to_string()),
                    ..CustomResourceInfo::default()
                },
                CustomResourceInfo {
                    name: "beta".to_string(),
                    namespace: Some("staging".to_string()),
                    ..CustomResourceInfo::default()
                },
            ],
            None,
        );
        app.extension_in_instances = true;
        app.extension_instance_cursor = 1;

        draw(&app, &snapshot);
    }

    /// Verifies Helm repositories view renders without panic.
    #[test]
    fn render_helm_repos_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot
            .helm_repositories
            .push(crate::k8s::dtos::HelmRepoInfo {
                name: "bitnami".to_string(),
                url: "https://charts.bitnami.com/bitnami".to_string(),
            });

        let app = app_with_view(AppView::HelmCharts);
        draw(&app, &snapshot);
    }

    /// Verifies Helm repos view renders empty state without panic.
    #[test]
    fn render_helm_repos_empty_smoke() {
        let app = app_with_view(AppView::HelmCharts);
        draw(&app, &ClusterSnapshot::default());
    }

    /// Verifies FluxCD "all" view renders without panic.
    #[test]
    fn render_fluxcd_all_view_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.flux_resources.push(FluxResourceInfo {
            name: "apps".to_string(),
            namespace: Some("flux-system".to_string()),
            kind: "Kustomization".to_string(),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            status: "Ready".to_string(),
            message: Some("Applied revision main@sha1:abc123".to_string()),
            ..FluxResourceInfo::default()
        });

        let app = app_with_view(AppView::FluxCDAll);
        draw(&app, &snapshot);
    }

    /// Verifies detail overlay renders for a CustomResource without panic.
    #[test]
    fn render_detail_custom_resource_smoke() {
        let snapshot = ClusterSnapshot::default();
        let mut app = app_with_view(AppView::Extensions);
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::CustomResource {
                name: "my-widget".to_string(),
                namespace: Some("default".to_string()),
                group: "demo.io".to_string(),
                version: "v1".to_string(),
                kind: "Widget".to_string(),
                plural: "widgets".to_string(),
            }),
            metadata: DetailMetadata {
                name: "my-widget".to_string(),
                namespace: Some("default".to_string()),
                status: Some("Widget.demo.io".to_string()),
                ..DetailMetadata::default()
            },
            yaml: Some(
                "apiVersion: demo.io/v1\nkind: Widget\nmetadata:\n  name: my-widget\n".to_string(),
            ),
            sections: vec![
                "CUSTOM RESOURCE".to_string(),
                "kind: Widget".to_string(),
                "apiVersion: demo.io/v1".to_string(),
            ],
            ..DetailViewState::default()
        });

        draw(&app, &snapshot);
    }

    /// Verifies detail overlay renders for a HelmRelease without panic.
    #[test]
    fn render_detail_helm_release_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot
            .helm_releases
            .push(crate::k8s::dtos::HelmReleaseInfo {
                name: "my-app".to_string(),
                namespace: "default".to_string(),
                chart: "nginx".to_string(),
                chart_version: "15.0.0".to_string(),
                status: "deployed".to_string(),
                revision: 3,
                ..crate::k8s::dtos::HelmReleaseInfo::default()
            });

        let mut app = app_with_view(AppView::HelmReleases);
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::HelmRelease(
                "my-app".to_string(),
                "default".to_string(),
            )),
            metadata: DetailMetadata {
                name: "my-app".to_string(),
                namespace: Some("default".to_string()),
                status: Some("deployed".to_string()),
                ..DetailMetadata::default()
            },
            yaml: Some(
                "apiVersion: v1\nkind: Secret\nmetadata:\n  name: sh.helm.release.v1.my-app.v3\n"
                    .to_string(),
            ),
            ..DetailViewState::default()
        });

        draw(&app, &snapshot);
    }
}
