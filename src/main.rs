//! KubecTUI entry point.
//!
//! This module wires terminal lifecycle management, the application state machine,
//! the Kubernetes client, and the ratatui rendering pipeline.

#![cfg_attr(test, allow(clippy::field_reassign_with_default))]

use std::{
    collections::HashMap,
    io,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use ratatui::{Terminal, backend::CrosstermBackend};

use kubectui::ui::components::port_forward_dialog::PortForwardDialog;
use kubectui::{
    action_history::{ActionKind, ActionStatus},
    app::{
        AppAction, AppState, AppView, DetailMetadata, DetailViewState, LogsViewerState,
        ResourceRef, load_config, save_config,
    },
    bookmarks::{bookmark_selected_index, resource_exists, selected_bookmark_resource},
    coordinator::{LogStreamStatus, UpdateCoordinator, UpdateMessage},
    events::apply_action,
    k8s::{
        client::K8sClient,
        dtos::FluxResourceInfo,
        exec::{ExecEvent, ExecSessionHandle, fetch_pod_containers, spawn_exec_session},
        logs::{LogsClient, PodRef},
        portforward::PortForwarderService,
        probes::extract_probes_from_pod,
        workload_logs::{MAX_WORKLOAD_LOG_STREAMS, resolve_workload_log_targets},
    },
    policy::{DetailAction, ResourceActionContext},
    secret::{decode_secret_yaml, encode_secret_yaml},
    state::{ClusterSnapshot, DataPhase, GlobalState, RefreshOptions},
    ui,
    workbench::{WorkbenchTabKey, WorkbenchTabState},
};

/// Main asynchronous runtime entrypoint.
#[tokio::main]
async fn main() -> Result<()> {
    // Send logs to a file instead of stderr so they don't corrupt the TUI.
    // kube-rs emits ERROR-level traces for transient connection drops which
    // would otherwise render on top of the ratatui display.
    let log_path = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("kubectui")
        .join("kubectui.log");
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        env_logger::Builder::from_default_env()
            .target(env_logger::Target::Pipe(Box::new(file)))
            .init();
    } else {
        // Fallback: silence logs entirely rather than corrupt the terminal.
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Off)
            .init();
    }
    ui::profiling::init_from_env();

    // Simple CLI flags:
    //   --theme <name>
    //   --profile-render
    //   --profile-output <dir>
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("kubectui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("KubecTUI — keyboard-driven terminal UI for Kubernetes\n");
        println!("USAGE: kubectui [OPTIONS]\n");
        println!("OPTIONS:");
        println!("  --theme <name>  Set color theme (dark, nord, dracula, catppuccin, light)");
        println!("  --profile-render  Enable render profiling (frame timings + folded stacks)");
        println!("  --profile-output <dir>  Profile output directory (default: target/profiles)");
        println!("  --version, -V   Show version");
        println!("  --help, -h      Show this help message");
        return Ok(());
    }
    if let Some(pos) = args.iter().position(|a| a == "--theme")
        && let Some(name) = args.get(pos + 1)
    {
        let idx = match name.to_lowercase().as_str() {
            "nord" => 1,
            "dracula" => 2,
            "catppuccin" | "mocha" => 3,
            "light" => 4,
            _ => 0,
        };
        kubectui::ui::theme::set_active_theme(idx);
    }
    if args.iter().any(|a| a == "--profile-render") {
        ui::profiling::set_enabled(true);
    }
    if let Some(pos) = args.iter().position(|a| a == "--profile-output")
        && let Some(dir) = args.get(pos + 1)
    {
        ui::profiling::set_output_dir(PathBuf::from(dir));
    }

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture
        );
        original_hook(info);
    }));

    let mut terminal = setup_terminal().context("failed to initialize terminal")?;
    let run_result = run_app(&mut terminal).await;
    let restore_result = restore_terminal(&mut terminal);

    if let Err(err) = ui::profiling::write_report_if_enabled() {
        eprintln!("failed to write profiling report: {err:#}");
    }

    if let Err(err) = restore_result {
        eprintln!("failed to restore terminal state: {err:#}");
    }

    run_result
}

/// Applies coordinator update messages to app state.
fn apply_coordinator_msg(msg: UpdateMessage, app: &mut AppState) {
    match msg {
        UpdateMessage::LogUpdate {
            pod_name,
            namespace,
            container_name,
            line,
        } => {
            for tab in &mut app.workbench.tabs {
                match &mut tab.state {
                    WorkbenchTabState::PodLogs(logs_tab) => {
                        let viewer = &mut logs_tab.viewer;
                        if viewer.pod_name == pod_name
                            && viewer.pod_namespace == namespace
                            && viewer.container_name == container_name
                        {
                            viewer.push_line(line.clone());
                            if viewer.follow_mode {
                                viewer.scroll_offset = viewer.lines.len().saturating_sub(1);
                            }
                        }
                    }
                    WorkbenchTabState::WorkloadLogs(logs_tab) => {
                        if logs_tab.sources.iter().any(|(pod, ns, container)| {
                            pod == &pod_name && ns == &namespace && container == &container_name
                        }) {
                            logs_tab.push_line(kubectui::workbench::WorkloadLogLine {
                                pod_name: pod_name.clone(),
                                container_name: container_name.clone(),
                                content: line.clone(),
                                is_stderr: false,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        UpdateMessage::ProbeUpdate {
            pod_name,
            namespace,
            probes,
        } => {
            if let Some(detail) = &mut app.detail_view
                && let Some(panel) = &mut detail.probe_panel
                && panel.pod_name == pod_name
                && panel.namespace == namespace
            {
                panel.update_probes(probes);
            }
        }
        UpdateMessage::LogStreamStatus {
            pod_name,
            namespace,
            container_name,
            status,
        } => {
            for tab in &mut app.workbench.tabs {
                match &mut tab.state {
                    WorkbenchTabState::PodLogs(logs_tab) => {
                        let viewer = &mut logs_tab.viewer;
                        if viewer.pod_name == pod_name
                            && viewer.pod_namespace == namespace
                            && viewer.container_name == container_name
                        {
                            match &status {
                                LogStreamStatus::Error(err) => {
                                    viewer.error = Some(err.clone());
                                    viewer.loading = false;
                                }
                                LogStreamStatus::Ended | LogStreamStatus::Cancelled => {
                                    viewer.follow_mode = false;
                                }
                                LogStreamStatus::Started => {}
                            }
                        }
                    }
                    WorkbenchTabState::WorkloadLogs(logs_tab) => {
                        if logs_tab.sources.iter().any(|(pod, ns, container)| {
                            pod == &pod_name && ns == &namespace && container == &container_name
                        }) {
                            logs_tab.loading = false;
                            if let LogStreamStatus::Error(err) = &status {
                                logs_tab.notice =
                                    Some(format!("{pod_name}/{container_name}: {err}"));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        UpdateMessage::ProbeError {
            pod_name,
            namespace,
            error,
        } => {
            if let Some(detail) = &mut app.detail_view
                && let Some(panel) = &mut detail.probe_panel
                && panel.pod_name == pod_name
                && panel.namespace == namespace
            {
                panel.error = Some(error);
            }
        }
    }
}

fn apply_detail_state_to_workbench(app: &mut AppState, request_id: u64, state: &DetailViewState) {
    let Some(resource) = state.resource.as_ref() else {
        return;
    };

    if let Some(tab) = app
        .workbench
        .find_tab_mut(&WorkbenchTabKey::ResourceYaml(resource.clone()))
        && let WorkbenchTabState::ResourceYaml(yaml_tab) = &mut tab.state
        && yaml_tab.pending_request_id == Some(request_id)
    {
        yaml_tab.yaml = state.yaml.clone();
        yaml_tab.loading = false;
        yaml_tab.error = state.error.clone();
        yaml_tab.pending_request_id = None;
    }

    if let Some(tab) = app
        .workbench
        .find_tab_mut(&WorkbenchTabKey::ResourceEvents(resource.clone()))
        && let WorkbenchTabState::ResourceEvents(events_tab) = &mut tab.state
        && events_tab.pending_request_id == Some(request_id)
    {
        events_tab.events = state.events.clone();
        events_tab.loading = false;
        events_tab.error = state.error.clone();
        events_tab.pending_request_id = None;
        events_tab.rebuild_timeline(&app.action_history);
    }

    if let Some(tab) = app
        .workbench
        .find_tab_mut(&WorkbenchTabKey::DecodedSecret(resource.clone()))
        && let WorkbenchTabState::DecodedSecret(secret_tab) = &mut tab.state
        && secret_tab.pending_request_id == Some(request_id)
    {
        secret_tab.source_yaml = state.yaml.clone();
        secret_tab.loading = false;
        secret_tab.pending_request_id = None;
        secret_tab.error = state.yaml_error.clone().or_else(|| {
            state
                .yaml
                .as_deref()
                .map(decode_secret_yaml)
                .transpose()
                .map(|entries| {
                    if let Some(entries) = entries {
                        secret_tab.entries = entries;
                        secret_tab.clamp_selected();
                    } else {
                        secret_tab.entries.clear();
                        secret_tab.clamp_selected();
                    }
                })
                .err()
                .map(|err| err.to_string())
        });
    }
}

fn apply_detail_error_to_workbench(
    app: &mut AppState,
    request_id: u64,
    resource: &ResourceRef,
    error: &str,
) {
    if let Some(tab) = app
        .workbench
        .find_tab_mut(&WorkbenchTabKey::ResourceYaml(resource.clone()))
        && let WorkbenchTabState::ResourceYaml(yaml_tab) = &mut tab.state
        && yaml_tab.pending_request_id == Some(request_id)
    {
        yaml_tab.loading = false;
        yaml_tab.error = Some(error.to_string());
        yaml_tab.pending_request_id = None;
    }

    if let Some(tab) = app
        .workbench
        .find_tab_mut(&WorkbenchTabKey::ResourceEvents(resource.clone()))
        && let WorkbenchTabState::ResourceEvents(events_tab) = &mut tab.state
        && events_tab.pending_request_id == Some(request_id)
    {
        events_tab.loading = false;
        events_tab.error = Some(error.to_string());
        events_tab.pending_request_id = None;
    }

    if let Some(tab) = app
        .workbench
        .find_tab_mut(&WorkbenchTabKey::DecodedSecret(resource.clone()))
        && let WorkbenchTabState::DecodedSecret(secret_tab) = &mut tab.state
        && secret_tab.pending_request_id == Some(request_id)
    {
        secret_tab.loading = false;
        secret_tab.error = Some(error.to_string());
        secret_tab.pending_request_id = None;
    }
}

fn workbench_follow_streams_to_stop(
    app: &AppState,
    action: AppAction,
) -> Vec<(String, String, String)> {
    let tabs: Vec<&WorkbenchTabState> = match action {
        AppAction::ToggleWorkbench if app.workbench().open => {
            app.workbench().tabs.iter().map(|tab| &tab.state).collect()
        }
        AppAction::WorkbenchCloseActiveTab => app
            .workbench()
            .active_tab()
            .map(|tab| vec![&tab.state])
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    tabs.into_iter()
        .filter_map(|state| match state {
            WorkbenchTabState::PodLogs(logs_tab) => {
                let viewer = &logs_tab.viewer;
                (viewer.follow_mode
                    && !viewer.pod_name.is_empty()
                    && !viewer.pod_namespace.is_empty()
                    && !viewer.container_name.is_empty())
                .then(|| {
                    (
                        viewer.pod_name.clone(),
                        viewer.pod_namespace.clone(),
                        viewer.container_name.clone(),
                    )
                })
            }
            _ => None,
        })
        .collect()
}

fn workbench_workload_log_sessions_to_stop(app: &AppState, action: AppAction) -> Vec<u64> {
    let tabs: Vec<&WorkbenchTabState> = match action {
        AppAction::ToggleWorkbench if app.workbench().open => {
            app.workbench().tabs.iter().map(|tab| &tab.state).collect()
        }
        AppAction::WorkbenchCloseActiveTab => app
            .workbench()
            .active_tab()
            .map(|tab| vec![&tab.state])
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    tabs.into_iter()
        .filter_map(|state| match state {
            WorkbenchTabState::WorkloadLogs(tab) => Some(tab.session_id),
            _ => None,
        })
        .collect()
}

fn workbench_exec_sessions_to_stop(app: &AppState, action: AppAction) -> Vec<u64> {
    let tabs: Vec<&WorkbenchTabState> = match action {
        AppAction::ToggleWorkbench if app.workbench().open => {
            app.workbench().tabs.iter().map(|tab| &tab.state).collect()
        }
        AppAction::WorkbenchCloseActiveTab => app
            .workbench()
            .active_tab()
            .map(|tab| vec![&tab.state])
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    tabs.into_iter()
        .filter_map(|state| match state {
            WorkbenchTabState::Exec(tab) => Some(tab.session_id),
            _ => None,
        })
        .collect()
}

fn refresh_port_forward_workbench(
    app: &mut AppState,
    port_forwarder: &PortForwarderService,
    status_message_clear_at: &mut Option<Instant>,
) {
    let tunnels = port_forwarder.list_tunnels();
    app.tunnel_registry.update_tunnels(tunnels.clone());
    if let Some(tab) = app
        .workbench_mut()
        .find_tab_mut(&WorkbenchTabKey::PortForward)
        && let WorkbenchTabState::PortForward(port_tab) = &mut tab.state
    {
        let mut registry = kubectui::state::port_forward::TunnelRegistry::new();
        registry.update_tunnels(tunnels);
        port_tab.dialog.update_registry(registry);
    }
    set_transient_status(
        app,
        status_message_clear_at,
        "Refreshed port-forward sessions.",
    );
}

fn next_request_id(sequence: &mut u64) -> u64 {
    *sequence = sequence.wrapping_add(1).max(1);
    *sequence
}

fn open_detail_for_resource(
    app: &mut AppState,
    snapshot: &ClusterSnapshot,
    client: &K8sClient,
    detail_tx: &tokio::sync::mpsc::Sender<DetailAsyncResult>,
    resource: ResourceRef,
    detail_request_seq: &mut u64,
) {
    let request_id = next_request_id(detail_request_seq);
    let mut state = initial_loading_state(resource.clone(), snapshot);
    state.pending_request_id = Some(request_id);
    app.detail_view = Some(state);
    let client_clone = client.clone();
    let snapshot_clone = snapshot.clone();
    let tx = detail_tx.clone();
    let requested_resource = resource.clone();
    tokio::spawn(async move {
        let result = fetch_detail_view(&client_clone, &snapshot_clone, requested_resource.clone())
            .await
            .map_err(|err| err.to_string());
        let _ = tx
            .send(DetailAsyncResult {
                request_id,
                resource: requested_resource,
                result,
            })
            .await;
    });
}

#[derive(Debug)]
enum LogsViewerAsyncResult {
    Containers {
        request_id: u64,
        pod_name: String,
        namespace: String,
        result: Result<Vec<String>, String>,
    },
    Tail {
        request_id: u64,
        pod_name: String,
        namespace: String,
        container_name: String,
        result: Result<Vec<String>, String>,
    },
}

#[derive(Debug)]
struct RefreshAsyncResult {
    request_id: u64,
    context_generation: u64,
    requested_namespace: Option<String>,
    result: Result<GlobalState, String>,
}

#[derive(Debug, Clone)]
struct QueuedRefresh {
    request_id: u64,
    namespace: Option<String>,
    options: RefreshOptions,
    context_generation: u64,
}

#[derive(Debug, Default)]
struct RefreshRuntimeState {
    request_seq: u64,
    in_flight_id: Option<u64>,
    in_flight_task: Option<tokio::task::JoinHandle<()>>,
    queued_refresh: Option<QueuedRefresh>,
    context_generation: u64,
}

#[derive(Debug)]
struct DeleteAsyncResult {
    request_id: u64,
    action_history_id: u64,
    context_generation: u64,
    origin_view: AppView,
    resource: ResourceRef,
    result: Result<(), String>,
}

#[derive(Debug)]
struct ScaleAsyncResult {
    action_history_id: u64,
    context_generation: u64,
    origin_view: AppView,
    resource: ResourceRef,
    target_replicas: i32,
    resource_label: String,
    result: Result<(), String>,
}

#[derive(Debug)]
struct RolloutRestartAsyncResult {
    action_history_id: u64,
    context_generation: u64,
    origin_view: AppView,
    resource_label: String,
    result: Result<(), String>,
}

#[derive(Debug)]
struct FluxReconcileAsyncResult {
    action_history_id: u64,
    context_generation: u64,
    origin_view: AppView,
    resource: ResourceRef,
    resource_label: String,
    baseline: Option<FluxReconcileObservedState>,
    result: Result<(), String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FluxReconcileObservedState {
    status: String,
    message: Option<String>,
    last_reconcile_time: Option<DateTime<Utc>>,
    last_applied_revision: Option<String>,
    last_attempted_revision: Option<String>,
    observed_generation: Option<i64>,
}

#[derive(Debug, Clone)]
struct PendingFluxReconcileVerification {
    action_history_id: u64,
    resource: ResourceRef,
    resource_label: String,
    baseline: Option<FluxReconcileObservedState>,
    deadline: Instant,
}

#[derive(Debug)]
struct TriggerCronJobAsyncResult {
    action_history_id: u64,
    context_generation: u64,
    origin_view: AppView,
    resource_label: String,
    result: Result<String, String>,
}

#[derive(Debug)]
struct ProbeAsyncResult {
    resource: ResourceRef,
    result: Result<Vec<(String, kubectui::k8s::probes::ContainerProbes)>, String>,
}

#[derive(Debug)]
struct ExecBootstrapResult {
    session_id: u64,
    resource: ResourceRef,
    result: Result<Vec<String>, String>,
}

#[derive(Debug)]
struct WorkloadLogsBootstrapResult {
    session_id: u64,
    resource: ResourceRef,
    result: Result<Vec<kubectui::k8s::workload_logs::WorkloadLogTarget>, String>,
}

#[derive(Debug)]
struct ExtensionFetchResult {
    crd_name: String,
    result: Result<Vec<kubectui::k8s::dtos::CustomResourceInfo>, String>,
}

#[derive(Debug)]
struct DetailAsyncResult {
    request_id: u64,
    resource: ResourceRef,
    result: Result<DetailViewState, String>,
}

#[derive(Debug)]
struct EventsAsyncResult {
    request_id: u64,
    context_generation: u64,
    requested_namespace: Option<String>,
    result: Result<Vec<kubectui::k8s::dtos::K8sEventInfo>, String>,
}

#[derive(Debug)]
struct RelationsAsyncResult {
    request_id: u64,
    resource: ResourceRef,
    result: Result<Vec<kubectui::k8s::relationships::RelationNode>, String>,
}

#[derive(Debug, Default)]
struct EventsFetchRuntimeState {
    request_seq: u64,
    in_flight_id: Option<u64>,
    in_flight_namespace: Option<Option<String>>,
    in_flight_task: Option<tokio::task::JoinHandle<()>>,
    queued_namespace: Option<Option<String>>,
}

#[derive(Debug, Clone, Copy)]
enum NodeOpKind {
    Cordon,
    Uncordon,
    Drain,
}

impl NodeOpKind {
    fn label(self) -> &'static str {
        match self {
            NodeOpKind::Cordon => "Cordon",
            NodeOpKind::Uncordon => "Uncordon",
            NodeOpKind::Drain => "Drain",
        }
    }
}

#[derive(Debug)]
struct NodeOpsAsyncResult {
    action_history_id: u64,
    context_generation: u64,
    origin_view: AppView,
    node_name: String,
    op_kind: NodeOpKind,
    result: Result<(), String>,
}

#[derive(Debug, Clone)]
struct DeferredRefreshTrigger {
    context_generation: u64,
    view: AppView,
    include_flux: bool,
    namespace: Option<String>,
}

struct MutationRuntime<'a> {
    global_state: &'a mut GlobalState,
    client: &'a K8sClient,
    refresh_tx: &'a tokio::sync::mpsc::Sender<RefreshAsyncResult>,
    deferred_refresh_tx: &'a tokio::sync::mpsc::Sender<DeferredRefreshTrigger>,
    refresh_state: &'a mut RefreshRuntimeState,
    snapshot_dirty: &'a mut bool,
    auto_refresh: &'a mut tokio::time::Interval,
    status_message_clear_at: &'a mut Option<Instant>,
}

const STARTUP_NAMESPACE_FETCH_TIMEOUT_SECS: u64 = 3;
const STARTUP_NAMESPACE_FETCH_ATTEMPTS: usize = 2;
const STARTUP_NAMESPACE_FETCH_RETRY_DELAY_MS: u64 = 150;
const FLUX_AUTO_REFRESH_EVERY: u64 = 3;
const STATUS_MESSAGE_TIMEOUT_SECS: u64 = 12;
const MUTATION_REFRESH_DELAYS_SECS: &[u64] = &[2, 5];
const FLUX_RECONCILE_REFRESH_DELAYS_SECS: &[u64] = &[2, 5, 9];
const MAX_RECENT_EVENTS_CACHE_ITEMS: usize = 250;

fn fast_refresh_options(include_flux: bool) -> RefreshOptions {
    RefreshOptions {
        include_flux,
        include_cluster_info: false,
        include_secondary_resources: false,
    }
}

fn queue_deferred_refreshes(
    tx: &tokio::sync::mpsc::Sender<DeferredRefreshTrigger>,
    context_generation: u64,
    view: AppView,
    namespace: Option<String>,
    include_flux: bool,
    delays_secs: &[u64],
) {
    for &delay_secs in delays_secs {
        let tx = tx.clone();
        let trigger = DeferredRefreshTrigger {
            context_generation,
            view,
            include_flux,
            namespace: namespace.clone(),
        };
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(delay_secs)).await;
            let _ = tx.send(trigger).await;
        });
    }
}

fn active_namespace_scope(app: &AppState) -> Option<String> {
    namespace_scope(app.get_namespace()).map(str::to_string)
}

fn flux_resource_matches(resource: &ResourceRef, candidate: &FluxResourceInfo) -> bool {
    let ResourceRef::CustomResource {
        name,
        namespace,
        group,
        version,
        kind,
        plural,
    } = resource
    else {
        return false;
    };

    candidate.name == *name
        && candidate.namespace == *namespace
        && candidate.group == *group
        && candidate.version == *version
        && candidate.kind == *kind
        && candidate.plural == *plural
}

fn flux_resource_for_ref<'a>(
    snapshot: &'a ClusterSnapshot,
    resource: &ResourceRef,
) -> Option<&'a FluxResourceInfo> {
    snapshot
        .flux_resources
        .iter()
        .find(|candidate| flux_resource_matches(resource, candidate))
}

fn flux_observed_state(resource: &FluxResourceInfo) -> FluxReconcileObservedState {
    FluxReconcileObservedState {
        status: resource.status.clone(),
        message: resource.message.clone(),
        last_reconcile_time: resource.last_reconcile_time,
        last_applied_revision: resource.last_applied_revision.clone(),
        last_attempted_revision: resource.last_attempted_revision.clone(),
        observed_generation: resource.observed_generation,
    }
}

fn flux_observed_state_for_resource(
    snapshot: &ClusterSnapshot,
    resource: &ResourceRef,
) -> Option<FluxReconcileObservedState> {
    flux_resource_for_ref(snapshot, resource).map(flux_observed_state)
}

fn flux_reconcile_progress_observed(
    baseline: Option<&FluxReconcileObservedState>,
    current: &FluxResourceInfo,
) -> bool {
    let Some(baseline) = baseline else {
        return true;
    };

    let current_state = flux_observed_state(current);
    current_state != *baseline
}

fn flux_reconcile_status_summary(resource: &FluxResourceInfo) -> String {
    let mut parts = vec![format!("status {}", resource.status)];

    if let Some(revision) = resource
        .last_applied_revision
        .as_ref()
        .or(resource.last_attempted_revision.as_ref())
    {
        parts.push(format!("revision {revision}"));
    }

    parts.join(", ")
}

fn process_flux_reconcile_verifications(
    app: &mut AppState,
    snapshot: &ClusterSnapshot,
    pending: &mut Vec<PendingFluxReconcileVerification>,
    status_message_clear_at: &mut Option<Instant>,
) -> bool {
    let mut changed = false;
    let now = Instant::now();
    let mut remaining = Vec::with_capacity(pending.len());

    for verification in pending.drain(..) {
        if let Some(current) = flux_resource_for_ref(snapshot, &verification.resource)
            && flux_reconcile_progress_observed(verification.baseline.as_ref(), current)
        {
            let message = format!(
                "Flux reconcile observed for {} ({})",
                verification.resource_label,
                flux_reconcile_status_summary(current)
            );
            app.complete_action_history(
                verification.action_history_id,
                ActionStatus::Succeeded,
                message.clone(),
                true,
            );
            set_transient_status(app, status_message_clear_at, message);
            changed = true;
            continue;
        }

        if now >= verification.deadline {
            let message = format!(
                "Reconcile requested for {}. Waiting for controller status update.",
                verification.resource_label
            );
            app.complete_action_history(
                verification.action_history_id,
                ActionStatus::Succeeded,
                message.clone(),
                true,
            );
            set_transient_status(app, status_message_clear_at, message);
            changed = true;
            continue;
        }

        remaining.push(verification);
    }

    *pending = remaining;
    changed
}

fn set_transient_status(
    app: &mut AppState,
    status_message_clear_at: &mut Option<Instant>,
    message: impl Into<String>,
) {
    let msg = message.into();
    app.push_toast(msg.clone(), false);
    app.set_status(msg);
    *status_message_clear_at =
        Some(Instant::now() + Duration::from_secs(STATUS_MESSAGE_TIMEOUT_SECS));
}

fn begin_detail_mutation(
    app: &mut AppState,
    status_message_clear_at: &mut Option<Instant>,
    message: impl Into<String>,
) {
    app.detail_view = None;
    app.focus = kubectui::app::Focus::Content;
    set_transient_status(app, status_message_clear_at, message);
}

fn finish_mutation_success(
    app: &mut AppState,
    runtime: &mut MutationRuntime<'_>,
    origin_view: AppView,
    message: impl Into<String>,
    force_include_flux: bool,
    delays_secs: &[u64],
) {
    let active_namespace_scope = active_namespace_scope(app);
    let include_flux = force_include_flux || origin_view.is_fluxcd();
    set_transient_status(app, runtime.status_message_clear_at, message);
    request_refresh(
        runtime.refresh_tx,
        runtime.global_state,
        runtime.client,
        active_namespace_scope.clone(),
        refresh_options_for_view(origin_view, include_flux, false),
        runtime.refresh_state,
        runtime.snapshot_dirty,
    );
    queue_deferred_refreshes(
        runtime.deferred_refresh_tx,
        runtime.refresh_state.context_generation,
        origin_view,
        active_namespace_scope,
        include_flux,
        delays_secs,
    );
    runtime.auto_refresh.reset();
}

fn full_refresh_options(include_flux: bool, include_cluster_info: bool) -> RefreshOptions {
    RefreshOptions {
        include_flux,
        include_cluster_info,
        include_secondary_resources: true,
    }
}

fn palette_detail_action_needs_detail(action: DetailAction) -> bool {
    matches!(
        action,
        DetailAction::Scale
            | DetailAction::Restart
            | DetailAction::Probes
            | DetailAction::Delete
            | DetailAction::EditYaml
            | DetailAction::Trigger
            | DetailAction::Cordon
            | DetailAction::Uncordon
            | DetailAction::Drain
    )
}

fn map_palette_detail_action(action: DetailAction) -> AppAction {
    match action {
        DetailAction::ViewYaml => AppAction::OpenResourceYaml,
        DetailAction::ViewDecodedSecret => AppAction::OpenDecodedSecret,
        DetailAction::ToggleBookmark => AppAction::ToggleBookmark,
        DetailAction::ViewEvents => AppAction::OpenResourceEvents,
        DetailAction::Logs => AppAction::LogsViewerOpen,
        DetailAction::Exec => AppAction::OpenExec,
        DetailAction::PortForward => AppAction::PortForwardOpen,
        DetailAction::Probes => AppAction::ProbePanelOpen,
        DetailAction::Scale => AppAction::ScaleDialogOpen,
        DetailAction::Restart => AppAction::RolloutRestart,
        DetailAction::FluxReconcile => AppAction::FluxReconcile,
        DetailAction::EditYaml => AppAction::EditYaml,
        DetailAction::Delete => AppAction::DeleteResource,
        DetailAction::Trigger => AppAction::TriggerCronJob,
        DetailAction::ViewRelationships => AppAction::OpenRelationships,
        DetailAction::Cordon => AppAction::CordonNode,
        DetailAction::Uncordon => AppAction::UncordonNode,
        DetailAction::Drain => AppAction::ConfirmDrainNode,
    }
}

fn palette_action_requires_loaded_detail(action: &AppAction) -> bool {
    matches!(
        action,
        AppAction::ScaleDialogOpen
            | AppAction::RolloutRestart
            | AppAction::ProbePanelOpen
            | AppAction::DeleteResource
            | AppAction::EditYaml
            | AppAction::TriggerCronJob
            | AppAction::CordonNode
            | AppAction::UncordonNode
            | AppAction::ConfirmDrainNode
    )
}

fn view_prefers_secondary_refresh(view: AppView) -> bool {
    !matches!(
        view,
        AppView::Dashboard
            | AppView::Nodes
            | AppView::Namespaces
            | AppView::Pods
            | AppView::Deployments
            | AppView::StatefulSets
            | AppView::DaemonSets
            | AppView::ReplicaSets
            | AppView::ReplicationControllers
            | AppView::Jobs
            | AppView::CronJobs
            | AppView::Services
            | AppView::Events
            | AppView::PortForwarding
            | AppView::HelmCharts
            | AppView::FluxCDAlertProviders
            | AppView::FluxCDAlerts
            | AppView::FluxCDAll
            | AppView::FluxCDArtifacts
            | AppView::FluxCDHelmReleases
            | AppView::FluxCDHelmRepositories
            | AppView::FluxCDImages
            | AppView::FluxCDKustomizations
            | AppView::FluxCDReceivers
            | AppView::FluxCDSources
    )
}

fn view_wants_cluster_info(view: AppView) -> bool {
    matches!(view, AppView::Dashboard)
}

fn refresh_options_for_view(
    view: AppView,
    include_flux: bool,
    force_cluster_info: bool,
) -> RefreshOptions {
    let include_cluster_info = force_cluster_info || view_wants_cluster_info(view);
    if view_prefers_secondary_refresh(view) {
        full_refresh_options(include_flux, include_cluster_info)
    } else {
        fast_refresh_options(include_flux)
    }
}

fn is_transient_transport_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        let text = cause.to_string();
        text.contains("SendRequest")
            || text.contains("Connection refused")
            || text.contains("connection reset")
            || text.contains("connection closed")
            || text.contains("broken pipe")
            || text.contains("timed out sending request")
    })
}

async fn fetch_namespaces_with_startup_retry(client: &K8sClient) -> Result<Vec<String>> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match tokio::time::timeout(
            Duration::from_secs(STARTUP_NAMESPACE_FETCH_TIMEOUT_SECS),
            client.fetch_namespaces(),
        )
        .await
        {
            Ok(Ok(namespaces)) => return Ok(namespaces),
            Ok(Err(err))
                if attempt < STARTUP_NAMESPACE_FETCH_ATTEMPTS
                    && is_transient_transport_error(&err) =>
            {
                tokio::time::sleep(Duration::from_millis(
                    STARTUP_NAMESPACE_FETCH_RETRY_DELAY_MS,
                ))
                .await;
            }
            Ok(Err(err)) => return Err(err),
            Err(_) if attempt < STARTUP_NAMESPACE_FETCH_ATTEMPTS => {
                tokio::time::sleep(Duration::from_millis(
                    STARTUP_NAMESPACE_FETCH_RETRY_DELAY_MS,
                ))
                .await;
            }
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "timed out fetching namespaces ({}s)",
                    STARTUP_NAMESPACE_FETCH_TIMEOUT_SECS
                ));
            }
        }
    }
}

fn spawn_refresh_task(
    refresh_tx: tokio::sync::mpsc::Sender<RefreshAsyncResult>,
    mut global_state: GlobalState,
    client: K8sClient,
    namespace: Option<String>,
    options: RefreshOptions,
    request_id: u64,
    context_generation: u64,
) -> tokio::task::JoinHandle<()> {
    let requested_namespace = namespace.clone();
    tokio::spawn(async move {
        let result = global_state
            .refresh_with_options(&client, namespace.as_deref(), options)
            .await
            .map(|_| global_state)
            .map_err(|err| err.to_string());
        let _ = refresh_tx
            .send(RefreshAsyncResult {
                request_id,
                context_generation,
                requested_namespace,
                result,
            })
            .await;
    })
}

fn abort_in_flight_refresh(refresh_state: &mut RefreshRuntimeState) {
    if let Some(task) = refresh_state.in_flight_task.take() {
        task.abort();
    }
    refresh_state.in_flight_id = None;
}

fn request_refresh(
    refresh_tx: &tokio::sync::mpsc::Sender<RefreshAsyncResult>,
    global_state: &mut GlobalState,
    client: &K8sClient,
    namespace: Option<String>,
    options: RefreshOptions,
    refresh_state: &mut RefreshRuntimeState,
    snapshot_dirty: &mut bool,
) {
    let snapshot = global_state.snapshot();
    let should_queue_secondary_backfill =
        !options.include_secondary_resources && !snapshot.secondary_resources_loaded;
    let visible_options = RefreshOptions {
        include_flux: options.include_flux,
        include_cluster_info: options.include_cluster_info,
        include_secondary_resources: options.include_secondary_resources
            || should_queue_secondary_backfill,
    };
    global_state.mark_refresh_requested(visible_options);
    *snapshot_dirty = true;

    refresh_state.request_seq = refresh_state.request_seq.wrapping_add(1);
    let request_id = refresh_state.request_seq;

    if refresh_state.in_flight_id.is_none() {
        let queued_namespace = namespace.clone();
        refresh_state.in_flight_id = Some(request_id);
        refresh_state.in_flight_task = Some(spawn_refresh_task(
            refresh_tx.clone(),
            global_state.clone(),
            client.clone(),
            namespace,
            options,
            request_id,
            refresh_state.context_generation,
        ));
        if should_queue_secondary_backfill {
            refresh_state.request_seq = refresh_state.request_seq.wrapping_add(1);
            refresh_state.queued_refresh = Some(QueuedRefresh {
                request_id: refresh_state.request_seq,
                namespace: queued_namespace,
                options: RefreshOptions {
                    include_flux: options.include_flux,
                    include_cluster_info: false,
                    include_secondary_resources: true,
                },
                context_generation: refresh_state.context_generation,
            });
        }
    } else {
        let merged_include_flux = refresh_state
            .queued_refresh
            .as_ref()
            .is_some_and(|queued| queued.options.include_flux)
            || options.include_flux;
        let merged_include_cluster_info = refresh_state
            .queued_refresh
            .as_ref()
            .is_some_and(|queued| queued.options.include_cluster_info)
            || options.include_cluster_info;
        let merged_include_secondary_resources = refresh_state
            .queued_refresh
            .as_ref()
            .is_some_and(|queued| queued.options.include_secondary_resources)
            || options.include_secondary_resources
            || should_queue_secondary_backfill;
        refresh_state.queued_refresh = Some(QueuedRefresh {
            request_id,
            namespace,
            options: RefreshOptions {
                include_flux: merged_include_flux,
                include_cluster_info: merged_include_cluster_info,
                include_secondary_resources: merged_include_secondary_resources,
            },
            context_generation: refresh_state.context_generation,
        });
    }
}

fn normalize_recent_events(
    mut events: Vec<kubectui::k8s::dtos::K8sEventInfo>,
) -> Vec<kubectui::k8s::dtos::K8sEventInfo> {
    events.sort_by(|left, right| right.last_seen.cmp(&left.last_seen));
    events.truncate(MAX_RECENT_EVENTS_CACHE_ITEMS);
    events
}

fn abort_in_flight_events_fetch(events_state: &mut EventsFetchRuntimeState) {
    if let Some(task) = events_state.in_flight_task.take() {
        task.abort();
    }
    events_state.in_flight_id = None;
    events_state.in_flight_namespace = None;
}

fn spawn_events_fetch_task(
    events_tx: tokio::sync::mpsc::Sender<EventsAsyncResult>,
    client: K8sClient,
    namespace: Option<String>,
    request_id: u64,
    context_generation: u64,
) -> tokio::task::JoinHandle<()> {
    let requested_namespace = namespace.clone();
    tokio::spawn(async move {
        let result = client
            .fetch_events(namespace.as_deref())
            .await
            .map(normalize_recent_events)
            .map_err(|err| err.to_string());
        let _ = events_tx
            .send(EventsAsyncResult {
                request_id,
                context_generation,
                requested_namespace,
                result,
            })
            .await;
    })
}

fn request_events_refresh(
    events_tx: &tokio::sync::mpsc::Sender<EventsAsyncResult>,
    global_state: &mut GlobalState,
    client: &K8sClient,
    namespace: Option<String>,
    context_generation: u64,
    events_state: &mut EventsFetchRuntimeState,
    snapshot_dirty: &mut bool,
) {
    if events_state.in_flight_id.is_some() {
        if events_state.in_flight_namespace == Some(namespace.clone())
            || events_state.queued_namespace == Some(namespace.clone())
        {
            return;
        }
        events_state.queued_namespace = Some(namespace);
        return;
    }

    if global_state.mark_events_refresh_requested() {
        *snapshot_dirty = true;
    }

    events_state.request_seq = events_state.request_seq.wrapping_add(1);
    let request_id = events_state.request_seq;
    events_state.in_flight_id = Some(request_id);
    events_state.in_flight_namespace = Some(namespace.clone());
    events_state.in_flight_task = Some(spawn_events_fetch_task(
        events_tx.clone(),
        client.clone(),
        namespace,
        request_id,
        context_generation,
    ));
}

#[allow(clippy::too_many_arguments)]
fn spawn_delete_task(
    delete_tx: tokio::sync::mpsc::Sender<DeleteAsyncResult>,
    client: K8sClient,
    resource: ResourceRef,
    request_id: u64,
    action_history_id: u64,
    context_generation: u64,
    origin_view: AppView,
    force: bool,
) {
    tokio::spawn(async move {
        let outcome = tokio::time::timeout(Duration::from_secs(20), async {
            match &resource {
                ResourceRef::CustomResource {
                    name,
                    namespace,
                    group,
                    version,
                    kind,
                    plural,
                } => {
                    client
                        .delete_custom_resource(
                            group,
                            version,
                            kind,
                            plural,
                            name,
                            namespace.as_deref(),
                        )
                        .await
                }
                _ => {
                    let kind = resource.kind().to_ascii_lowercase();
                    let name = resource.name().to_string();
                    let namespace = resource.namespace().map(str::to_owned);
                    if force {
                        client
                            .force_delete_resource(&kind, &name, namespace.as_deref())
                            .await
                    } else {
                        client
                            .delete_resource(&kind, &name, namespace.as_deref())
                            .await
                    }
                }
            }
        })
        .await;

        let result = match outcome {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.to_string()),
            Err(_) => Err("Delete request timed out after 20s".to_string()),
        };

        let _ = delete_tx
            .send(DeleteAsyncResult {
                request_id,
                action_history_id,
                context_generation,
                origin_view,
                resource,
                result,
            })
            .await;
    });
}

/// Runs KubecTUI's event loop.
async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = load_config();

    let mut client = pick_context_at_startup(terminal, &mut app).await?;

    let mut global_state = GlobalState::default();
    let mut startup_namespace_scope = namespace_scope(app.get_namespace()).map(str::to_string);
    // Validate persisted namespace against current cluster namespaces to avoid
    // starting in a stale namespace with empty resource lists.
    if app.get_namespace() != "all" {
        let selected_namespace = app.get_namespace().to_string();
        match fetch_namespaces_with_startup_retry(&client).await {
            Ok(namespaces) => {
                if !namespaces.iter().any(|ns| ns == &selected_namespace) {
                    app.set_namespace("all".to_string());
                    startup_namespace_scope = None;
                    app.needs_config_save = true;
                    app.set_error(format!(
                        "Namespace '{selected_namespace}' not found. Switched to 'all'."
                    ));
                }
                app.set_available_namespaces(namespaces);
            }
            // If we can't validate namespace quickly, load cluster-wide first to prevent blank startup.
            Err(err) => {
                app.set_namespace("all".to_string());
                startup_namespace_scope = None;
                app.set_error(format!(
                    "Namespace '{selected_namespace}' could not be validated quickly ({err}). Loaded 'all' namespaces."
                ));
            }
        }
    }

    let mut port_forwarder = PortForwarderService::new(std::sync::Arc::new(client.clone()));

    let (update_tx, mut update_rx) = tokio::sync::mpsc::channel::<UpdateMessage>(4096);
    let mut coordinator = UpdateCoordinator::new(client.clone(), update_tx.clone());

    // Channel for async detail view fetches — carries the requested resource to prevent stale writes.
    let (detail_tx, mut detail_rx) = tokio::sync::mpsc::channel::<DetailAsyncResult>(16);
    let mut detail_request_seq: u64 = 0;
    let (logs_viewer_tx, mut logs_viewer_rx) =
        tokio::sync::mpsc::channel::<LogsViewerAsyncResult>(64);
    let mut logs_viewer_request_seq: u64 = 0;
    let (delete_tx, mut delete_rx) = tokio::sync::mpsc::channel::<DeleteAsyncResult>(16);
    let mut delete_request_seq: u64 = 0;
    let mut delete_in_flight_id: Option<u64> = None;
    let (deferred_refresh_tx, mut deferred_refresh_rx) =
        tokio::sync::mpsc::channel::<DeferredRefreshTrigger>(32);
    let (scale_tx, mut scale_rx) = tokio::sync::mpsc::channel::<ScaleAsyncResult>(16);
    let (rollout_tx, mut rollout_rx) = tokio::sync::mpsc::channel::<RolloutRestartAsyncResult>(16);
    let (flux_reconcile_tx, mut flux_reconcile_rx) =
        tokio::sync::mpsc::channel::<FluxReconcileAsyncResult>(16);
    let (trigger_cronjob_tx, mut trigger_cronjob_rx) =
        tokio::sync::mpsc::channel::<TriggerCronJobAsyncResult>(16);
    let (node_ops_tx, mut node_ops_rx) = tokio::sync::mpsc::channel::<NodeOpsAsyncResult>(16);
    let mut node_op_in_flight: bool = false;
    let (probe_tx, mut probe_rx) = tokio::sync::mpsc::channel::<ProbeAsyncResult>(16);
    let (relations_tx, mut relations_rx) = tokio::sync::mpsc::channel::<RelationsAsyncResult>(16);
    let mut relations_request_seq: u64 = 0;
    let (exec_bootstrap_tx, mut exec_bootstrap_rx) =
        tokio::sync::mpsc::channel::<ExecBootstrapResult>(16);
    let (exec_update_tx, mut exec_update_rx) = tokio::sync::mpsc::channel::<ExecEvent>(128);
    let mut next_exec_session_id: u64 = 1;
    let mut exec_sessions: HashMap<u64, ExecSessionHandle> = HashMap::new();
    let (workload_logs_bootstrap_tx, mut workload_logs_bootstrap_rx) =
        tokio::sync::mpsc::channel::<WorkloadLogsBootstrapResult>(16);
    let mut next_workload_logs_session_id: u64 = 1;
    let mut workload_log_sessions: HashMap<u64, Vec<(String, String, String)>> = HashMap::new();
    let (extension_fetch_tx, mut extension_fetch_rx) =
        tokio::sync::mpsc::channel::<ExtensionFetchResult>(16);
    let (events_tx, mut events_rx) = tokio::sync::mpsc::channel::<EventsAsyncResult>(16);
    let mut events_state = EventsFetchRuntimeState::default();

    // Channel for background data refreshes — namespace switches, manual refresh, auto-refresh
    // all go through here so the UI stays responsive during API calls.
    let (refresh_tx, mut refresh_rx) = tokio::sync::mpsc::channel::<RefreshAsyncResult>(16);
    // Background refresh scheduling state — one in-flight + one coalesced queued request.
    let mut refresh_state = RefreshRuntimeState::default();
    let mut snapshot_dirty = false;

    // Start with view-scoped refresh (workload-first for core views, secondary deferred).
    let startup_include_flux = app.view().is_fluxcd();
    request_refresh(
        &refresh_tx,
        &mut global_state,
        &client,
        startup_namespace_scope,
        refresh_options_for_view(app.view(), startup_include_flux, true),
        &mut refresh_state,
        &mut snapshot_dirty,
    );
    if app.view() == AppView::Events {
        request_events_refresh(
            &events_tx,
            &mut global_state,
            &client,
            namespace_scope(app.get_namespace()).map(str::to_string),
            refresh_state.context_generation,
            &mut events_state,
            &mut snapshot_dirty,
        );
    }

    // Cached snapshot — only re-clone when state is marked dirty
    let mut cached_snapshot = global_state.snapshot();
    snapshot_dirty = false;

    // Render-skip: only redraw when state actually changed
    let mut needs_redraw = true;

    let mut tick = tokio::time::interval(Duration::from_millis(50));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Auto-refresh: periodically re-fetch cluster data
    let refresh_secs = if app.refresh_interval_secs == 0 {
        86400
    } else {
        app.refresh_interval_secs
    };
    let mut auto_refresh = tokio::time::interval(Duration::from_secs(refresh_secs));
    auto_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Skip the first immediate tick — we already fetched at startup
    auto_refresh.reset();

    // Track consecutive refresh failures for backoff
    let mut consecutive_refresh_failures: u32 = 0;
    let mut backoff_until: Option<Instant> = None;
    let mut last_config_save: Option<Instant> = None;
    let mut auto_refresh_count: u64 = 0;
    let mut status_message_clear_at: Option<Instant> = None;
    let mut pending_palette_action: Option<AppAction> = None;
    let mut pending_flux_reconcile_verifications: Vec<PendingFluxReconcileVerification> =
        Vec::new();
    let mut pending_context_switch: Option<(String, tokio::task::JoinHandle<Result<K8sClient>>)> =
        None;

    let mut event_stream = EventStream::new();

    loop {
        // Re-clone snapshot only when something changed
        if snapshot_dirty {
            cached_snapshot = global_state.snapshot();
            snapshot_dirty = false;
            needs_redraw = true;
        }

        if needs_redraw {
            terminal
                .draw(|frame| ui::render(frame, &app, &cached_snapshot))
                .context("failed to render frame")?;
            needs_redraw = false;
        }

        if app.should_quit() {
            break;
        }

        // Check if a deferred palette action is ready to dispatch.
        let pending_action_ready = pending_palette_action.as_ref().is_some_and(|a| {
            let needs_loaded_detail = palette_action_requires_loaded_detail(a);
            !needs_loaded_detail
                || app
                    .detail_view
                    .as_ref()
                    .is_some_and(|d| !d.loading && d.error.is_none())
        });

        let mut action_to_process: Option<AppAction> = None;

        // Wait concurrently on: tick, input event, coordinator update, detail fetch, or auto-refresh.
        // `biased` ensures coordinator messages and detail results are drained before blocking on input.
        tokio::select! {
            biased;

            // Coordinator updates (log lines, probe updates) — highest priority
            msg = update_rx.recv() => {
                if let Some(msg) = msg {
                    apply_coordinator_msg(msg, &mut app);
                    needs_redraw = true;
                }
                // Drain any additional queued messages without blocking
                let mut drain_count = 0;
                while drain_count < 100 {
                    match update_rx.try_recv() {
                        Ok(msg) => {
                            apply_coordinator_msg(msg, &mut app);
                            drain_count += 1;
                        }
                        Err(_) => break,
                    }
                }
            }

            // Detail view fetch completed in background task
            result = detail_rx.recv() => {
                if let Some(result) = result {
                    let DetailAsyncResult {
                        request_id,
                        resource: requested_resource,
                        result,
                    } = result;
                    let detail_still_waiting_for_this = app
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| {
                            detail.resource.as_ref() == Some(&requested_resource)
                                && detail.pending_request_id == Some(request_id)
                        });
                    let workbench_waiting_for_this = app.workbench.tabs.iter().any(|tab| {
                        matches!(
                            &tab.state,
                            WorkbenchTabState::ResourceYaml(yaml_tab)
                                if yaml_tab.resource == requested_resource
                                    && yaml_tab.loading
                                    && yaml_tab.pending_request_id == Some(request_id)
                        ) || matches!(
                            &tab.state,
                            WorkbenchTabState::ResourceEvents(events_tab)
                                if events_tab.resource == requested_resource
                                    && events_tab.loading
                                    && events_tab.pending_request_id == Some(request_id)
                        )
                    });
                    if !detail_still_waiting_for_this && !workbench_waiting_for_this {
                        continue;
                    }
                    needs_redraw = true;
                    match result {
                        Ok(state) => {
                            apply_detail_state_to_workbench(&mut app, request_id, &state);
                            if detail_still_waiting_for_this {
                                app.detail_view = Some(state);
                            }
                        }
                        Err(err) => {
                            apply_detail_error_to_workbench(
                                &mut app,
                                request_id,
                                &requested_resource,
                                &err,
                            );
                            if detail_still_waiting_for_this {
                                app.detail_view = Some(DetailViewState {
                                    resource: Some(requested_resource),
                                    pending_request_id: None,
                                    loading: false,
                                    error: Some(err),
                                    ..DetailViewState::default()
                                });
                            }
                        }
                    }
                }
            }

            // Logs viewer async responses (container discovery + tail snapshot)
            result = logs_viewer_rx.recv() => {
                if let Some(result) = result {
                    match result {
                        LogsViewerAsyncResult::Containers { request_id, pod_name, namespace, result } => {
                            let mut tail_request: Option<(u64, String, String, String)> = None;
                            for tab in &mut app.workbench.tabs {
                                if let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state {
                                    let viewer = &mut logs_tab.viewer;
                                    if viewer.pod_name == pod_name
                                        && viewer.pod_namespace == namespace
                                        && viewer.pending_container_request_id == Some(request_id)
                                    {
                                        needs_redraw = true;
                                        viewer.pending_container_request_id = None;
                                        match &result {
                                            Ok(containers) => {
                                                viewer.containers = containers.clone();
                                                viewer.container_cursor = 0;
                                                viewer.lines.clear();
                                                viewer.scroll_offset = 0;
                                                viewer.error = None;

                                                match containers.len() {
                                                    0 => {
                                                        viewer.container_name.clear();
                                                        viewer.picking_container = false;
                                                        viewer.pending_logs_request_id = None;
                                                        viewer.loading = false;
                                                        viewer.error = Some(
                                                            "No containers found for this pod.".to_string(),
                                                        );
                                                    }
                                                    1 => {
                                                        let container_name = containers[0].clone();
                                                        logs_viewer_request_seq =
                                                            logs_viewer_request_seq.wrapping_add(1);
                                                        let tail_request_id = logs_viewer_request_seq;
                                                        viewer.container_name = container_name.clone();
                                                        viewer.picking_container = false;
                                                        viewer.loading = true;
                                                        viewer.pending_logs_request_id = Some(tail_request_id);
                                                        tail_request = Some((
                                                            tail_request_id,
                                                            pod_name.clone(),
                                                            namespace.clone(),
                                                            container_name,
                                                        ));
                                                    }
                                                    _ => {
                                                        viewer.container_name.clear();
                                                        viewer.picking_container = true;
                                                        viewer.pending_logs_request_id = None;
                                                        viewer.loading = false;
                                                    }
                                                }
                                            }
                                            Err(err) => {
                                                viewer.picking_container = false;
                                                viewer.pending_logs_request_id = None;
                                                viewer.loading = false;
                                                viewer.error =
                                                    Some(format!("Failed to load containers: {err}"));
                                            }
                                        }
                                    }
                                }
                            }

                            if let Some((tail_request_id, pod_name, pod_ns, container_name)) =
                                tail_request
                            {
                                let client_clone = client.clone();
                                let tx = logs_viewer_tx.clone();
                                tokio::spawn(async move {
                                    let logs_client = LogsClient::new(client_clone.get_client());
                                    let pod_ref = PodRef::new(pod_name.clone(), pod_ns.clone());
                                    let result = logs_client
                                        .tail_logs(&pod_ref, Some(500), Some(container_name.as_str()))
                                        .await
                                        .map_err(|err| err.to_string());
                                    let _ = tx.send(LogsViewerAsyncResult::Tail {
                                        request_id: tail_request_id,
                                        pod_name,
                                        namespace: pod_ns,
                                        container_name,
                                        result,
                                    }).await;
                                });
                            }
                        }
                        LogsViewerAsyncResult::Tail {
                            request_id,
                            pod_name,
                            namespace,
                            container_name,
                            result,
                        } => {
                            for tab in &mut app.workbench.tabs {
                                if let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state {
                                    let viewer = &mut logs_tab.viewer;
                                    if viewer.pod_name == pod_name
                                        && viewer.pod_namespace == namespace
                                        && viewer.container_name == container_name
                                        && viewer.pending_logs_request_id == Some(request_id)
                                    {
                                        needs_redraw = true;
                                        viewer.pending_logs_request_id = None;
                                        viewer.loading = false;
                                        match &result {
                                            Ok(lines) => {
                                                viewer.lines = lines.clone();
                                                viewer.error = None;
                                            }
                                            Err(err) => {
                                                viewer.error = Some(err.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Background refresh completed (namespace switch, manual refresh, auto-refresh)
            result = refresh_rx.recv() => {
                if let Some(result) = result {
                    // Ignore stale refreshes from previous context generations or superseded requests.
                    if result.context_generation != refresh_state.context_generation
                        || refresh_state.in_flight_id != Some(result.request_id)
                    {
                        continue;
                    }

                    refresh_state.in_flight_id = None;
                    refresh_state.in_flight_task = None;
                    needs_redraw = true;
                    let active_namespace_scope = namespace_scope(app.get_namespace()).map(str::to_string);
                    let namespace_matches = result.requested_namespace == active_namespace_scope;

                    if namespace_matches {
                        match result.result {
                            Ok(new_state) => {
                                global_state = new_state;
                                consecutive_refresh_failures = 0;
                                backoff_until = None;
                                let previously_selected_namespace = app.get_namespace().to_string();
                                let namespace_still_exists = previously_selected_namespace == "all"
                                    || global_state
                                        .namespaces()
                                        .iter()
                                        .any(|ns| ns == &previously_selected_namespace);
                                if !namespace_still_exists {
                                    app.set_namespace("all".to_string());
                                    app.needs_config_save = true;
                                    app.set_error(format!(
                                        "Namespace '{previously_selected_namespace}' not found. Switched to 'all'."
                                    ));
                                    request_refresh(
                                        &refresh_tx,
                                        &mut global_state,
                                        &client,
                                        None,
                                        refresh_options_for_view(
                                            app.view(),
                                            app.view().is_fluxcd(),
                                            false,
                                        ),
                                        &mut refresh_state,
                                        &mut snapshot_dirty,
                                    );
                                    if app.view() == AppView::Events {
                                        request_events_refresh(
                                            &events_tx,
                                            &mut global_state,
                                            &client,
                                            None,
                                            refresh_state.context_generation,
                                            &mut events_state,
                                            &mut snapshot_dirty,
                                        );
                                    }
                                } else {
                                    app.clear_error();
                                }
                                app.set_available_namespaces(global_state.namespaces().to_vec());
                                if process_flux_reconcile_verifications(
                                    &mut app,
                                    &global_state.snapshot(),
                                    &mut pending_flux_reconcile_verifications,
                                    &mut status_message_clear_at,
                                ) {
                                    needs_redraw = true;
                                }
                                snapshot_dirty = true;
                                spawn_extensions_fetch(&client, &mut app, &global_state.snapshot(), &extension_fetch_tx);
                            }
                            Err(err) => {
                                consecutive_refresh_failures += 1;
                                let delay = match consecutive_refresh_failures {
                                    1 => 5,
                                    2 => 15,
                                    3 => 30,
                                    4 => 60,
                                    _ => 120,
                                };
                                backoff_until = Some(Instant::now() + Duration::from_secs(delay));
                                app.set_error(format!("Refresh failed: {err}"));
                            }
                        }
                    } else {
                        // Namespace changed while this refresh was running. Skip applying stale data.
                    }

                    if let Some(queued) = refresh_state.queued_refresh.take()
                        && queued.context_generation == refresh_state.context_generation
                    {
                        refresh_state.in_flight_id = Some(queued.request_id);
                        refresh_state.in_flight_task = Some(spawn_refresh_task(
                            refresh_tx.clone(),
                            global_state.clone(),
                            client.clone(),
                            queued.namespace,
                            queued.options,
                            queued.request_id,
                            queued.context_generation,
                        ));
                    }
                }
            }

            result = events_rx.recv() => {
                if let Some(result) = result {
                    if result.context_generation != refresh_state.context_generation
                        || events_state.in_flight_id != Some(result.request_id)
                    {
                        continue;
                    }

                    events_state.in_flight_id = None;
                    events_state.in_flight_namespace = None;
                    events_state.in_flight_task = None;
                    needs_redraw = true;

                    if result.requested_namespace
                        == namespace_scope(app.get_namespace()).map(str::to_string)
                    {
                        match result.result {
                            Ok(events) => {
                                global_state.apply_events_update(events);
                                snapshot_dirty = true;
                            }
                            Err(err) => {
                                global_state.fail_events_refresh(err.clone());
                                snapshot_dirty = true;
                                app.set_error(format!("Events refresh failed: {err}"));
                            }
                        }
                    }

                    if let Some(namespace) = events_state.queued_namespace.take() {
                        events_state.request_seq = events_state.request_seq.wrapping_add(1);
                        let request_id = events_state.request_seq;
                        events_state.in_flight_id = Some(request_id);
                        events_state.in_flight_namespace = Some(namespace.clone());
                        events_state.in_flight_task = Some(spawn_events_fetch_task(
                            events_tx.clone(),
                            client.clone(),
                            namespace,
                            request_id,
                            refresh_state.context_generation,
                        ));
                    }
                }
            }

            // Background delete completion — deletion itself runs off the UI loop.
            result = delete_rx.recv() => {
                if let Some(result) = result {
                    // Ignore stale results (e.g. context changed while request was in flight).
                    if result.context_generation != refresh_state.context_generation
                        || delete_in_flight_id != Some(result.request_id)
                    {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            "Delete verification was cancelled because the active context changed.",
                            true,
                        );
                        continue;
                    }

                    delete_in_flight_id = None;
                    needs_redraw = true;

                    match result.result {
                        Ok(()) => {
                            global_state.apply_optimistic_delete(&result.resource);
                            snapshot_dirty = true;
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Succeeded,
                                format!(
                                    "Deleted {} '{}'.",
                                    result.resource.kind(),
                                    result.resource.name()
                                ),
                                false,
                            );
                            finish_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                result.origin_view,
                                format!(
                                    "Deleted {} '{}'. Refreshing view...",
                                    result.resource.kind(),
                                    result.resource.name()
                                ),
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!("Delete failed: {err}"),
                                true,
                            );
                            status_message_clear_at = None;
                            app.set_error(format!("Delete failed: {err}"));
                        }
                    }
                }
            }

            result = scale_rx.recv() => {
                if let Some(result) = result {
                    if result.context_generation != refresh_state.context_generation {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            "Scale verification was cancelled because the active context changed.",
                            true,
                        );
                        continue;
                    }
                    needs_redraw = true;
                    match result.result {
                        Ok(()) => {
                            global_state
                                .apply_optimistic_scale(&result.resource, result.target_replicas);
                            snapshot_dirty = true;
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Succeeded,
                                format!("Scaled {}.", result.resource_label),
                                true,
                            );
                            finish_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                result.origin_view,
                                format!("Scaled {}. Refreshing view...", result.resource_label),
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!("Scale failed: {err}"),
                                true,
                            );
                            status_message_clear_at = None;
                            app.set_error(format!("Scale failed: {err}"));
                        }
                    }
                }
            }

            result = rollout_rx.recv() => {
                if let Some(result) = result {
                    if result.context_generation != refresh_state.context_generation {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            "Restart verification was cancelled because the active context changed.",
                            true,
                        );
                        continue;
                    }
                    needs_redraw = true;
                    match result.result {
                        Ok(()) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Succeeded,
                                format!("Restart requested for {}.", result.resource_label),
                                true,
                            );
                            finish_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                result.origin_view,
                                format!(
                                    "Restart requested for {}. Refreshing view...",
                                    result.resource_label
                                ),
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!("Restart failed: {err}"),
                                true,
                            );
                            status_message_clear_at = None;
                            app.set_error(format!("Restart failed: {err}"));
                        }
                    }
                }
            }

            result = flux_reconcile_rx.recv() => {
                if let Some(result) = result {
                    if result.context_generation != refresh_state.context_generation {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            "Reconcile verification was cancelled because the active context changed.",
                            true,
                        );
                        continue;
                    }

                    needs_redraw = true;
                    match result.result {
                        Ok(()) => {
                            pending_flux_reconcile_verifications
                                .retain(|pending| pending.resource != result.resource);
                            pending_flux_reconcile_verifications
                                .push(PendingFluxReconcileVerification {
                                    action_history_id: result.action_history_id,
                                    resource: result.resource,
                                    resource_label: result.resource_label.clone(),
                                    baseline: result.baseline,
                                    deadline: Instant::now()
                                        + Duration::from_secs(
                                            FLUX_RECONCILE_REFRESH_DELAYS_SECS
                                                .last()
                                                .copied()
                                                .unwrap_or_default()
                                                + 3,
                                        ),
                                });
                            finish_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                result.origin_view,
                                format!(
                                    "Reconcile requested for {}. Refreshing Flux status...",
                                    result.resource_label
                                ),
                                true,
                                FLUX_RECONCILE_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!("Flux reconcile failed: {err}"),
                                true,
                            );
                            status_message_clear_at = None;
                            app.set_error(format!("Flux reconcile failed: {err}"));
                        }
                    }
                }
            }

            result = trigger_cronjob_rx.recv() => {
                if let Some(result) = result {
                    if result.context_generation != refresh_state.context_generation {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            "Trigger cancelled because the active context changed.",
                            true,
                        );
                        continue;
                    }

                    needs_redraw = true;
                    match result.result {
                        Ok(job_name) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Succeeded,
                                format!("Created Job '{job_name}' from {}.", result.resource_label),
                                true,
                            );
                            finish_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                result.origin_view,
                                format!("Created Job '{job_name}'."),
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!("Trigger failed: {err}"),
                                true,
                            );
                            status_message_clear_at = None;
                            app.set_error(format!("Trigger failed: {err}"));
                        }
                    }
                }
            }

            result = node_ops_rx.recv() => {
                if let Some(result) = result {
                    if result.context_generation != refresh_state.context_generation {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            format!("{} cancelled because the active context changed.", result.op_kind.label()),
                            true,
                        );
                        continue;
                    }

                    needs_redraw = true;
                    node_op_in_flight = false;
                    match result.result {
                        Ok(()) => {
                            // Optimistic update: flip unschedulable in cache.
                            // Drain cordons the node, so it also sets unschedulable = true.
                            let new_val = !matches!(result.op_kind, NodeOpKind::Uncordon);
                            global_state.apply_optimistic_node_schedulable(&result.node_name, new_val);
                            snapshot_dirty = true;
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Succeeded,
                                format!("{} succeeded for Node '{}'.", result.op_kind.label(), result.node_name),
                                true,
                            );
                            finish_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                result.origin_view,
                                format!("{} succeeded for Node '{}'. Refreshing view...", result.op_kind.label(), result.node_name),
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!("{} failed: {err}", result.op_kind.label()),
                                true,
                            );
                            status_message_clear_at = None;
                            app.set_error(format!("{} failed: {err}", result.op_kind.label()));
                        }
                    }
                }
            }

            result = exec_bootstrap_rx.recv() => {
                if let Some(result) = result {
                    let mut start_session: Option<(u64, String, String, String)> = None;
                    if let Some(tab) = app
                        .workbench_mut()
                        .find_tab_mut(&WorkbenchTabKey::Exec(result.resource.clone()))
                        && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
                        && exec_tab.session_id == result.session_id
                    {
                        match result.result {
                            Ok(containers) => {
                                exec_tab.set_containers(containers.clone());
                                exec_tab.loading = exec_tab.picking_container;
                                exec_tab.error = None;
                                if containers.len() == 1 {
                                    start_session = Some((
                                        result.session_id,
                                        exec_tab.pod_name.clone(),
                                        exec_tab.namespace.clone(),
                                        containers[0].clone(),
                                    ));
                                }
                            }
                            Err(err) => {
                                exec_tab.loading = false;
                                exec_tab.error = Some(err);
                            }
                        }
                    }

                    if let Some((session_id, pod_name, namespace, container_name)) = start_session {
                        match spawn_exec_session(
                            client.clone(),
                            session_id,
                            pod_name,
                            namespace,
                            container_name.clone(),
                            exec_update_tx.clone(),
                        )
                        .await
                        {
                            Ok(handle) => {
                                exec_sessions.insert(session_id, handle);
                                if let Some(tab) = app
                                    .workbench_mut()
                                    .find_tab_mut(&WorkbenchTabKey::Exec(result.resource.clone()))
                                    && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
                                {
                                    exec_tab.container_name = container_name;
                                    exec_tab.loading = true;
                                }
                            }
                            Err(err) => {
                                if let Some(tab) = app
                                    .workbench_mut()
                                    .find_tab_mut(&WorkbenchTabKey::Exec(result.resource.clone()))
                                    && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
                                {
                                    exec_tab.loading = false;
                                    exec_tab.error = Some(format!("{err:#}"));
                                }
                            }
                        }
                    }
                }
            }

            result = exec_update_rx.recv() => {
                needs_redraw = true;
                if let Some(result) = result {
                    for tab in &mut app.workbench.tabs {
                        if let WorkbenchTabState::Exec(exec_tab) = &mut tab.state {
                            if exec_tab.session_id != match &result {
                                ExecEvent::Opened { session_id, .. }
                                | ExecEvent::Output { session_id, .. }
                                | ExecEvent::Exited { session_id, .. }
                                | ExecEvent::Error { session_id, .. } => *session_id,
                            } {
                                continue;
                            }

                            match result {
                                ExecEvent::Opened { shell, .. } => {
                                    exec_tab.shell_name = Some(shell);
                                    exec_tab.loading = false;
                                    exec_tab.exited = false;
                                    exec_tab.error = None;
                                }
                                ExecEvent::Output { chunk, is_stderr, .. } => {
                                    if is_stderr {
                                        exec_tab.append_output(&format!("[stderr] {chunk}"));
                                    } else {
                                        exec_tab.append_output(&chunk);
                                    }
                                }
                                ExecEvent::Exited { success, message, session_id } => {
                                    exec_tab.loading = false;
                                    exec_tab.exited = true;
                                    exec_tab.error = (!success).then_some(message.clone());
                                    exec_tab.append_output(&format!("{message}\n"));
                                    exec_sessions.remove(&session_id);
                                }
                                ExecEvent::Error { error, session_id } => {
                                    exec_tab.loading = false;
                                    exec_tab.error = Some(error.clone());
                                    exec_tab.exited = true;
                                    exec_sessions.remove(&session_id);
                                }
                            }
                            break;
                        }
                    }
                }
            }

            result = workload_logs_bootstrap_rx.recv() => {
                needs_redraw = true;
                if let Some(result) = result {
                    let mut sources_to_start = Vec::new();
                    if let Some(tab) = app
                        .workbench_mut()
                        .find_tab_mut(&WorkbenchTabKey::WorkloadLogs(result.resource.clone()))
                        && let WorkbenchTabState::WorkloadLogs(logs_tab) = &mut tab.state
                        && logs_tab.session_id == result.session_id
                    {
                        match result.result {
                            Ok(targets) => {
                                let mut sources = Vec::new();
                                for target in targets {
                                    for container in target.containers {
                                        if sources.len() >= MAX_WORKLOAD_LOG_STREAMS {
                                            logs_tab.notice = Some(format!(
                                                "Stream cap reached at {MAX_WORKLOAD_LOG_STREAMS} pod/container streams."
                                            ));
                                            break;
                                        }
                                        sources.push((
                                            target.pod_name.clone(),
                                            target.namespace.clone(),
                                            container,
                                        ));
                                    }
                                }
                                if sources.is_empty() {
                                    logs_tab.loading = false;
                                    logs_tab.error = Some("No pod/container streams were resolved.".to_string());
                                } else {
                                    logs_tab.sources = sources.clone();
                                    logs_tab.loading = false;
                                    workload_log_sessions.insert(result.session_id, sources.clone());
                                    sources_to_start = sources;
                                }
                            }
                            Err(err) => {
                                logs_tab.loading = false;
                                logs_tab.error = Some(err);
                            }
                        }
                    }

                    for (pod_name, namespace, container_name) in sources_to_start {
                        let _ = coordinator
                            .start_log_streaming(pod_name, namespace, container_name, true, false, false)
                            .await;
                    }
                }
            }

            result = probe_rx.recv() => {
                if let Some(result) = result {
                    needs_redraw = true;
                    if let Some(detail) = &mut app.detail_view
                        && detail.resource.as_ref() == Some(&result.resource)
                    {
                        use kubectui::ui::components::probe_panel::ProbePanelState;
                        if let ResourceRef::Pod(pod_name, namespace) = result.resource {
                            detail.probe_panel = Some(match result.result {
                                Ok(probes) => ProbePanelState::new(pod_name, namespace, probes),
                                Err(error) => {
                                    let mut state =
                                        ProbePanelState::new(pod_name, namespace, Vec::new());
                                    state.error = Some(error);
                                    state
                                }
                            });
                        }
                    }
                }
            }

            result = extension_fetch_rx.recv() => {
                if let Some(result) = result {
                    needs_redraw = true;
                    apply_extension_fetch_result(&mut app, result);
                }
            }

            result = relations_rx.recv() => {
                if let Some(result) = result {
                    let RelationsAsyncResult {
                        request_id,
                        resource: requested_resource,
                        result,
                    } = result;
                    let tab_key = WorkbenchTabKey::Relations(requested_resource.clone());
                    if let Some(tab) = app.workbench.find_tab_mut(&tab_key)
                        && let WorkbenchTabState::Relations(ref mut state) = tab.state
                        && state.pending_request_id == Some(request_id)
                    {
                        state.pending_request_id = None;
                        state.loading = false;
                        match result {
                            Ok(tree) => {
                                state.set_tree(tree);
                            }
                            Err(err) => {
                                state.error = Some(err);
                            }
                        }
                        needs_redraw = true;
                    }
                }
            }

            // Background context switch completed (TLS handshake finished)
            result = async {
                match &mut pending_context_switch {
                    Some((_, handle)) => handle.await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some((ctx, _)) = pending_context_switch.take() {
                    match result {
                        Ok(Ok(new_client)) => {
                            // Context-bound long-lived services must be rebuilt to avoid
                            // continuing background work against the previous cluster.
                            let _ = coordinator.shutdown().await;
                            for (_, handle) in exec_sessions.drain() {
                                let _ = handle.cancel_tx.send(());
                            }
                            workload_log_sessions.clear();
                            port_forwarder.stop_all().await;
                            app.tunnel_registry.update_tunnels(Vec::new());

                            client = new_client;
                            app.current_context_name = Some(ctx.clone());
                            coordinator = UpdateCoordinator::new(client.clone(), update_tx.clone());
                            port_forwarder =
                                PortForwarderService::new(std::sync::Arc::new(client.clone()));
                            // Invalidate stale async results from the previous client/context.
                            refresh_state.context_generation =
                                refresh_state.context_generation.wrapping_add(1);
                            abort_in_flight_refresh(&mut refresh_state);
                            abort_in_flight_events_fetch(&mut events_state);
                            events_state.queued_namespace = None;
                            refresh_state.queued_refresh = None;
                            delete_in_flight_id = None;
                            status_message_clear_at = None;
                            app.clear_status();
                            needs_redraw = true;

                            request_refresh(
                                &refresh_tx,
                                &mut global_state,
                                &client,
                                namespace_scope(app.get_namespace()).map(str::to_string),
                                refresh_options_for_view(app.view(), app.view().is_fluxcd(), true),
                                &mut refresh_state,
                                &mut snapshot_dirty,
                            );
                            if app.view() == AppView::Events {
                                request_events_refresh(
                                    &events_tx,
                                    &mut global_state,
                                    &client,
                                    namespace_scope(app.get_namespace()).map(str::to_string),
                                    refresh_state.context_generation,
                                    &mut events_state,
                                    &mut snapshot_dirty,
                                );
                            }
                        }
                        Ok(Err(err)) => {
                            global_state.set_phase(DataPhase::Error);
                            snapshot_dirty = true;
                            needs_redraw = true;
                            app.set_error(format!("Failed to connect to context '{ctx}': {err:#}"));
                        }
                        Err(join_err) => {
                            global_state.set_phase(DataPhase::Error);
                            snapshot_dirty = true;
                            needs_redraw = true;
                            app.set_error(format!("Context switch task panicked: {join_err}"));
                        }
                    }
                }
            }

            trigger = deferred_refresh_rx.recv() => {
                if let Some(trigger) = trigger {
                    if trigger.context_generation != refresh_state.context_generation {
                        continue;
                    }
                    let events_namespace = trigger.namespace.clone();
                    request_refresh(
                        &refresh_tx,
                        &mut global_state,
                        &client,
                        trigger.namespace,
                        refresh_options_for_view(trigger.view, trigger.include_flux, false),
                        &mut refresh_state,
                        &mut snapshot_dirty,
                    );
                    if trigger.view == AppView::Events {
                        request_events_refresh(
                            &events_tx,
                            &mut global_state,
                            &client,
                            events_namespace,
                            refresh_state.context_generation,
                            &mut events_state,
                            &mut snapshot_dirty,
                        );
                    }
                }
            }

            // Periodic tick — heartbeat for follow-mode log scrolling
            _ = tick.tick() => {
                if status_message_clear_at.is_some_and(|deadline| Instant::now() >= deadline) {
                    app.clear_status();
                    status_message_clear_at = None;
                    needs_redraw = true;
                }
                // Animate spinner during loading/refreshing phases
                let phase = cached_snapshot.phase;
                if matches!(phase, DataPhase::Loading | DataPhase::Idle)
                    || refresh_state.in_flight_id.is_some()
                {
                    app.advance_spinner();
                    needs_redraw = true;
                }
                // Expire old toasts
                if app.expire_toasts() {
                    needs_redraw = true;
                }
            }

            // Auto-refresh: re-fetch cluster data periodically
            _ = auto_refresh.tick() => {
                // Skip auto-refresh if a detail view is open (avoid disrupting user)
                // or if we're in a backoff period from consecutive failures
                // or if a refresh is already in flight
                let in_backoff = backoff_until.is_some_and(|t| Instant::now() < t);
                if app.detail_view.is_none() && !in_backoff {
                    auto_refresh_count = auto_refresh_count.wrapping_add(1);
                    let include_flux = app.view().is_fluxcd()
                        || auto_refresh_count.is_multiple_of(FLUX_AUTO_REFRESH_EVERY);
                    request_refresh(
                        &refresh_tx,
                        &mut global_state,
                        &client,
                        namespace_scope(app.get_namespace()).map(str::to_string),
                        refresh_options_for_view(app.view(), include_flux, false),
                        &mut refresh_state,
                        &mut snapshot_dirty,
                    );
                    if app.view() == AppView::Events {
                        request_events_refresh(
                            &events_tx,
                            &mut global_state,
                            &client,
                            namespace_scope(app.get_namespace()).map(str::to_string),
                            refresh_state.context_generation,
                            &mut events_state,
                            &mut snapshot_dirty,
                        );
                    }
                }
            }

            // Deferred palette action — fires immediately when detail view is loaded
            _ = std::future::ready(()), if pending_action_ready => {
                action_to_process = pending_palette_action.take();
                needs_redraw = true;
            }

            // Keyboard / terminal input — lowest priority so messages are drained first
            maybe_event = event_stream.next() => {
                let key = match maybe_event {
                    Some(Ok(Event::Resize(_, _))) => {
                        needs_redraw = true;
                        continue;
                    }
                    Some(Ok(Event::Key(key))) => key,
                    _ => continue,
                };
                needs_redraw = true;

                let action = if key.code == KeyCode::Enter
                    && !app.is_search_mode()
                    && !app.is_namespace_picker_open()
                    && !app.is_context_picker_open()
                    && !app.command_palette.is_open()
                    && app.detail_view.is_none()
                    && app.focus != kubectui::app::Focus::Workbench
                {
                    if app.focus == kubectui::app::Focus::Content
                        && app.view() == AppView::Extensions
                        && !app.extension_in_instances
                    {
                        // Drill into instances pane
                        if !app.extension_instances.is_empty() {
                            app.extension_in_instances = true;
                            app.extension_instance_cursor = 0;
                        }
                        AppAction::None
                    } else if app.focus == kubectui::app::Focus::Content
                        && app.view() == AppView::Bookmarks
                    {
                        match prepare_bookmark_target(&mut app, &cached_snapshot) {
                            Ok(resource) => AppAction::OpenDetail(resource),
                            Err(err) => {
                                app.set_error(err);
                                AppAction::None
                            }
                        }
                    } else if app.focus == kubectui::app::Focus::Content {
                        selected_resource(&app, &cached_snapshot)
                            .map(AppAction::OpenDetail)
                            .unwrap_or(AppAction::None)
                    } else {
                        app.sidebar_activate()
                    }
                } else if key.code == KeyCode::Esc
                    && app.view() == AppView::Extensions
                    && app.extension_in_instances
                    && app.detail_view.is_none()
                    && !app.is_search_mode()
                {
                    // Return from instances pane to CRD picker
                    app.extension_in_instances = false;
                    AppAction::None
                } else {
                    app.handle_key_event(key)
                };

                action_to_process = Some(action);
            }
        }

        // --- Action dispatch (shared between keyboard input and deferred palette actions) ---
        if let Some(action) = action_to_process {
            match action {
                AppAction::None => {
                    // No-op — don't call sync_extensions_instances on every unrecognized key
                }
                AppAction::Quit => break,
                AppAction::RefreshData => {
                    // Manual refresh bypasses backoff
                    consecutive_refresh_failures = 0;
                    backoff_until = None;
                    request_refresh(
                        &refresh_tx,
                        &mut global_state,
                        &client,
                        namespace_scope(app.get_namespace()).map(str::to_string),
                        full_refresh_options(true, true),
                        &mut refresh_state,
                        &mut snapshot_dirty,
                    );
                    if app.view() == AppView::Events {
                        request_events_refresh(
                            &events_tx,
                            &mut global_state,
                            &client,
                            namespace_scope(app.get_namespace()).map(str::to_string),
                            refresh_state.context_generation,
                            &mut events_state,
                            &mut snapshot_dirty,
                        );
                    }
                    // Reset auto-refresh timer after manual refresh
                    auto_refresh.reset();
                }
                AppAction::FluxReconcile => {
                    let reconcile_resource =
                        match selected_flux_reconcile_resource(&app, &cached_snapshot) {
                            Ok(resource) => resource,
                            Err(err) => {
                                app.set_error(err);
                                continue;
                            }
                        };

                    let resource_label = format!(
                        "{} '{}'",
                        reconcile_resource.kind(),
                        reconcile_resource.name()
                    );
                    let baseline =
                        flux_observed_state_for_resource(&cached_snapshot, &reconcile_resource);
                    let origin_view = app.view();
                    let action_history_id = app.record_action_pending(
                        ActionKind::FluxReconcile,
                        origin_view,
                        Some(reconcile_resource.clone()),
                        resource_label.clone(),
                        format!("Requesting reconcile for {resource_label}..."),
                    );
                    begin_detail_mutation(
                        &mut app,
                        &mut status_message_clear_at,
                        format!("Requesting reconcile for {resource_label}..."),
                    );
                    let tx = flux_reconcile_tx.clone();
                    let c = client.clone();
                    let context_generation = refresh_state.context_generation;
                    tokio::spawn(async move {
                        let result = match &reconcile_resource {
                            ResourceRef::CustomResource {
                                name,
                                namespace,
                                group,
                                version,
                                kind,
                                plural,
                            } => c
                                .request_flux_reconcile(
                                    group,
                                    version,
                                    kind,
                                    plural,
                                    name,
                                    namespace.as_deref(),
                                )
                                .await
                                .map_err(|err| format!("{err:#}")),
                            _ => Err("Flux reconcile is only available for custom resources."
                                .to_string()),
                        };
                        let _ = tx
                            .send(FluxReconcileAsyncResult {
                                action_history_id,
                                context_generation,
                                origin_view,
                                resource: reconcile_resource,
                                resource_label,
                                baseline,
                                result,
                            })
                            .await;
                    });
                }
                AppAction::ToggleWorkbench
                | AppAction::WorkbenchNextTab
                | AppAction::WorkbenchPreviousTab
                | AppAction::WorkbenchCloseActiveTab
                | AppAction::WorkbenchIncreaseHeight
                | AppAction::WorkbenchDecreaseHeight => {
                    let streams_to_stop = workbench_follow_streams_to_stop(&app, action.clone());
                    let workload_sessions_to_stop =
                        workbench_workload_log_sessions_to_stop(&app, action.clone());
                    let exec_sessions_to_stop =
                        workbench_exec_sessions_to_stop(&app, action.clone());
                    for (pod_name, namespace, container_name) in streams_to_stop {
                        let _ = coordinator
                            .stop_log_streaming(&pod_name, &namespace, &container_name)
                            .await;
                    }
                    for session_id in workload_sessions_to_stop {
                        if let Some(streams) = workload_log_sessions.remove(&session_id) {
                            for (pod_name, namespace, container_name) in streams {
                                let _ = coordinator
                                    .stop_log_streaming(&pod_name, &namespace, &container_name)
                                    .await;
                            }
                        }
                    }
                    for session_id in exec_sessions_to_stop {
                        if let Some(handle) = exec_sessions.remove(&session_id) {
                            let _ = handle.cancel_tx.send(());
                        }
                    }
                    apply_action(action, &mut app);
                    app.needs_config_save = true;
                }
                AppAction::OpenNamespacePicker => {
                    app.set_available_namespaces(global_state.namespaces().to_vec());
                    app.open_namespace_picker();
                }
                AppAction::CloseNamespacePicker => {
                    app.close_namespace_picker();
                }
                AppAction::OpenCommandPalette => {
                    let resource_ctx = app
                        .detail_view
                        .as_ref()
                        .and_then(|d| d.resource_action_context())
                        .or_else(|| selected_resource_context(&app, &cached_snapshot));
                    app.refresh_palette_columns();
                    app.command_palette.open_with_context(resource_ctx);
                }
                AppAction::CloseCommandPalette => {
                    app.command_palette.close();
                }
                AppAction::PaletteAction { action, resource } => {
                    app.command_palette.close();

                    let mapped = map_palette_detail_action(action);
                    let needs_detail = palette_detail_action_needs_detail(action);

                    if needs_detail && app.detail_view.is_none() {
                        open_detail_for_resource(
                            &mut app,
                            &cached_snapshot,
                            &client,
                            &detail_tx,
                            resource,
                            &mut detail_request_seq,
                        );
                    }

                    pending_palette_action = Some(mapped);
                }
                AppAction::NavigateTo(view) => {
                    app.command_palette.close();
                    app.view = view;
                    app.selected_idx = 0;
                    app.focus = kubectui::app::Focus::Content;
                    app.extension_in_instances = false;
                    if !matches!(
                        view,
                        kubectui::app::AppView::PortForwarding | kubectui::app::AppView::HelmCharts
                    ) {
                        request_refresh(
                            &refresh_tx,
                            &mut global_state,
                            &client,
                            namespace_scope(app.get_namespace()).map(str::to_string),
                            refresh_options_for_view(view, view.is_fluxcd(), false),
                            &mut refresh_state,
                            &mut snapshot_dirty,
                        );
                        if view == AppView::Events {
                            request_events_refresh(
                                &events_tx,
                                &mut global_state,
                                &client,
                                namespace_scope(app.get_namespace()).map(str::to_string),
                                refresh_state.context_generation,
                                &mut events_state,
                                &mut snapshot_dirty,
                            );
                        }
                    }
                    // Trigger extensions sync when navigating to Extensions view
                    if view == kubectui::app::AppView::Extensions {
                        spawn_extensions_fetch(
                            &client,
                            &mut app,
                            &cached_snapshot,
                            &extension_fetch_tx,
                        );
                    }
                }
                AppAction::OpenContextPicker => {
                    let contexts = K8sClient::list_contexts();
                    let current = kube::config::Kubeconfig::read()
                        .ok()
                        .and_then(|cfg| cfg.current_context);
                    app.open_context_picker(contexts, current);
                }
                AppAction::CloseContextPicker => {
                    app.close_context_picker();
                }
                AppAction::SelectContext(ctx) => {
                    app.close_context_picker();
                    pending_flux_reconcile_verifications.clear();
                    // Show loading state immediately; TLS handshake runs in background.
                    global_state.begin_loading_transition(true);
                    app.selected_idx = 0;
                    app.detail_view = None;
                    app.workbench.close_resource_tabs();
                    snapshot_dirty = true;
                    needs_redraw = true;

                    let ctx_clone = ctx.clone();
                    pending_context_switch = Some((
                        ctx,
                        tokio::spawn(async move {
                            tokio::time::timeout(
                                Duration::from_secs(15),
                                K8sClient::connect_with_context(&ctx_clone),
                            )
                            .await
                            .map_err(|_| anyhow::anyhow!("connection timed out after 15s"))?
                        }),
                    ));
                }
                AppAction::SelectNamespace(namespace) => {
                    app.set_namespace(namespace);
                    pending_flux_reconcile_verifications.clear();
                    app.selected_idx = 0;
                    app.close_namespace_picker();
                    app.needs_config_save = true;
                    // Invalidate stale async results from previous namespace selections.
                    refresh_state.context_generation =
                        refresh_state.context_generation.wrapping_add(1);
                    abort_in_flight_refresh(&mut refresh_state);
                    abort_in_flight_events_fetch(&mut events_state);
                    events_state.queued_namespace = None;
                    refresh_state.queued_refresh = None;
                    delete_in_flight_id = None;
                    for (_, handle) in exec_sessions.drain() {
                        let _ = handle.cancel_tx.send(());
                    }
                    for (_, streams) in workload_log_sessions.drain() {
                        for (pod_name, namespace, container_name) in streams {
                            let _ = coordinator
                                .stop_log_streaming(&pod_name, &namespace, &container_name)
                                .await;
                        }
                    }
                    status_message_clear_at = None;
                    app.clear_status();
                    // Drop old namespace data immediately to prevent inconsistent mixed views.
                    global_state.begin_loading_transition(false);
                    snapshot_dirty = true;
                    app.detail_view = None;
                    app.workbench.close_resource_tabs();

                    // Queue newest namespace refresh; if one is in flight it gets coalesced.
                    request_refresh(
                        &refresh_tx,
                        &mut global_state,
                        &client,
                        namespace_scope(app.get_namespace()).map(str::to_string),
                        refresh_options_for_view(app.view(), app.view().is_fluxcd(), false),
                        &mut refresh_state,
                        &mut snapshot_dirty,
                    );
                    if app.view() == AppView::Events {
                        request_events_refresh(
                            &events_tx,
                            &mut global_state,
                            &client,
                            namespace_scope(app.get_namespace()).map(str::to_string),
                            refresh_state.context_generation,
                            &mut events_state,
                            &mut snapshot_dirty,
                        );
                    }
                }
                AppAction::OpenDetail(resource) => {
                    open_detail_for_resource(
                        &mut app,
                        &cached_snapshot,
                        &client,
                        &detail_tx,
                        resource,
                        &mut detail_request_seq,
                    );
                    if app.focus == kubectui::app::Focus::Workbench {
                        app.focus = kubectui::app::Focus::Content;
                    }
                }
                AppAction::ActionHistoryOpenSelected => {
                    let Some(target) = app.selected_action_history_target().cloned() else {
                        app.set_error(
                            "Selected history entry does not have a jumpable resource.".to_string(),
                        );
                        continue;
                    };
                    app.view = target.view;
                    app.selected_idx = 0;
                    app.focus = kubectui::app::Focus::Content;
                    app.extension_in_instances = false;
                    open_detail_for_resource(
                        &mut app,
                        &cached_snapshot,
                        &client,
                        &detail_tx,
                        target.resource,
                        &mut detail_request_seq,
                    );
                }
                AppAction::CloseDetail => {
                    app.detail_view = None;
                }
                AppAction::OpenResourceYaml => {
                    let resource = app
                        .detail_view
                        .as_ref()
                        .and_then(|detail| detail.resource.clone())
                        .or_else(|| selected_resource(&app, &cached_snapshot));
                    let Some(resource) = resource else {
                        app.set_error("No resource selected for YAML inspection.".to_string());
                        continue;
                    };
                    let cached_yaml = app
                        .detail_view
                        .as_ref()
                        .and_then(|detail| {
                            (detail.resource.as_ref() == Some(&resource))
                                .then(|| detail.yaml.clone())
                        })
                        .flatten();
                    let pending_request_id = cached_yaml
                        .is_none()
                        .then(|| next_request_id(&mut detail_request_seq));
                    app.detail_view = None;
                    app.open_resource_yaml_tab(
                        resource.clone(),
                        cached_yaml.clone(),
                        None,
                        pending_request_id,
                    );
                    if let Some(request_id) = pending_request_id {
                        let client_clone = client.clone();
                        let snapshot_clone = cached_snapshot.clone();
                        let tx = detail_tx.clone();
                        let requested_resource = resource.clone();
                        tokio::spawn(async move {
                            let result = fetch_detail_view(
                                &client_clone,
                                &snapshot_clone,
                                requested_resource.clone(),
                            )
                            .await
                            .map_err(|err| err.to_string());
                            let _ = tx
                                .send(DetailAsyncResult {
                                    request_id,
                                    resource: requested_resource,
                                    result,
                                })
                                .await;
                        });
                    }
                }
                AppAction::OpenDecodedSecret => {
                    let resource = app
                        .detail_view
                        .as_ref()
                        .and_then(|detail| detail.resource.clone())
                        .or_else(|| selected_resource(&app, &cached_snapshot));
                    let Some(resource) = resource else {
                        app.set_error("No Secret selected for decoded inspection.".to_string());
                        continue;
                    };
                    if !matches!(resource, ResourceRef::Secret(_, _)) {
                        app.set_error(
                            "Decoded Secret view is only available for Secret resources."
                                .to_string(),
                        );
                        continue;
                    }
                    let cached_yaml = app
                        .detail_view
                        .as_ref()
                        .and_then(|detail| {
                            (detail.resource.as_ref() == Some(&resource))
                                .then(|| detail.yaml.clone())
                        })
                        .flatten();
                    let cached_error = app
                        .detail_view
                        .as_ref()
                        .and_then(|detail| {
                            (detail.resource.as_ref() == Some(&resource))
                                .then(|| detail.yaml_error.clone())
                        })
                        .flatten();
                    let pending_request_id = (cached_yaml.is_none() && cached_error.is_none())
                        .then(|| next_request_id(&mut detail_request_seq));
                    app.detail_view = None;
                    app.open_decoded_secret_tab(
                        resource.clone(),
                        cached_yaml.clone(),
                        cached_error,
                        pending_request_id,
                    );
                    if let Some(tab) = app
                        .workbench_mut()
                        .find_tab_mut(&WorkbenchTabKey::DecodedSecret(resource.clone()))
                        && let WorkbenchTabState::DecodedSecret(secret_tab) = &mut tab.state
                        && let Some(yaml) = cached_yaml.as_deref()
                    {
                        match decode_secret_yaml(yaml) {
                            Ok(entries) => {
                                secret_tab.source_yaml = Some(yaml.to_string());
                                secret_tab.entries = entries;
                                secret_tab.loading = false;
                                secret_tab.error = None;
                                secret_tab.clamp_selected();
                            }
                            Err(err) => {
                                secret_tab.loading = false;
                                secret_tab.error = Some(err.to_string());
                            }
                        }
                    }
                    if let Some(request_id) = pending_request_id {
                        let client_clone = client.clone();
                        let snapshot_clone = cached_snapshot.clone();
                        let tx = detail_tx.clone();
                        let requested_resource = resource.clone();
                        tokio::spawn(async move {
                            let result = fetch_detail_view(
                                &client_clone,
                                &snapshot_clone,
                                requested_resource.clone(),
                            )
                            .await
                            .map_err(|err| err.to_string());
                            let _ = tx
                                .send(DetailAsyncResult {
                                    request_id,
                                    resource: requested_resource,
                                    result,
                                })
                                .await;
                        });
                    }
                }
                AppAction::ToggleBookmark => {
                    let resource = app
                        .detail_view
                        .as_ref()
                        .and_then(|detail| detail.resource.clone())
                        .or_else(|| selected_resource(&app, &cached_snapshot));
                    let Some(resource) = resource else {
                        app.set_error("No resource selected to bookmark.".to_string());
                        continue;
                    };

                    match app.toggle_bookmark(resource.clone()) {
                        Ok(kubectui::bookmarks::BookmarkToggleResult::Added) => {
                            app.clear_error();
                            app.set_status(format!(
                                "Bookmarked {} '{}'{}.",
                                resource.kind(),
                                resource.name(),
                                resource
                                    .namespace()
                                    .map(|namespace| format!(" in namespace '{namespace}'"))
                                    .unwrap_or_default()
                            ));
                        }
                        Ok(kubectui::bookmarks::BookmarkToggleResult::Removed) => {
                            app.clear_error();
                            app.set_status(format!(
                                "Removed bookmark for {} '{}'{}.",
                                resource.kind(),
                                resource.name(),
                                resource
                                    .namespace()
                                    .map(|namespace| format!(" in namespace '{namespace}'"))
                                    .unwrap_or_default()
                            ));
                            if app.view() == AppView::Bookmarks {
                                app.selected_idx = app
                                    .selected_idx()
                                    .min(app.bookmark_count().saturating_sub(1));
                            }
                        }
                        Err(err) => app.set_error(err),
                    }
                }
                AppAction::OpenRelationships => {
                    let resource = app
                        .detail_view
                        .as_ref()
                        .and_then(|detail| detail.resource.clone())
                        .or_else(|| selected_resource(&app, &cached_snapshot));
                    let Some(resource) = resource else {
                        app.set_error(
                            "No resource selected for relationship exploration.".to_string(),
                        );
                        continue;
                    };
                    app.detail_view = None;
                    let request_id = next_request_id(&mut relations_request_seq);
                    let mut relations_tab =
                        kubectui::workbench::RelationsTabState::new(resource.clone());
                    relations_tab.pending_request_id = Some(request_id);
                    app.workbench
                        .open_tab(WorkbenchTabState::Relations(relations_tab));
                    app.focus = kubectui::app::Focus::Workbench;

                    let tx = relations_tx.clone();
                    let client_clone = client.clone();
                    let snapshot_clone = cached_snapshot.clone();
                    let requested_resource = resource.clone();
                    tokio::spawn(async move {
                        let result = kubectui::k8s::relationships::resolve_relationships(
                            &requested_resource,
                            &snapshot_clone,
                            &client_clone,
                        )
                        .await
                        .map_err(|err| format!("{err:#}"));
                        let _ = tx
                            .send(RelationsAsyncResult {
                                request_id,
                                resource: requested_resource,
                                result,
                            })
                            .await;
                    });
                }
                AppAction::OpenResourceEvents => {
                    let resource = app
                        .detail_view
                        .as_ref()
                        .and_then(|detail| detail.resource.clone())
                        .or_else(|| selected_resource(&app, &cached_snapshot));
                    let Some(resource) = resource else {
                        app.set_error("No resource selected for event inspection.".to_string());
                        continue;
                    };
                    let cached_events = app
                        .detail_view
                        .as_ref()
                        .and_then(|detail| {
                            (detail.resource.as_ref() == Some(&resource))
                                .then(|| detail.events.clone())
                        })
                        .unwrap_or_default();
                    let loading = cached_events.is_empty();
                    let pending_request_id =
                        loading.then(|| next_request_id(&mut detail_request_seq));
                    app.detail_view = None;
                    app.open_resource_events_tab(
                        resource.clone(),
                        cached_events,
                        loading,
                        None,
                        pending_request_id,
                    );
                    if let Some(request_id) = pending_request_id {
                        let client_clone = client.clone();
                        let snapshot_clone = cached_snapshot.clone();
                        let tx = detail_tx.clone();
                        let requested_resource = resource.clone();
                        tokio::spawn(async move {
                            let result = fetch_detail_view(
                                &client_clone,
                                &snapshot_clone,
                                requested_resource.clone(),
                            )
                            .await
                            .map_err(|err| err.to_string());
                            let _ = tx
                                .send(DetailAsyncResult {
                                    request_id,
                                    resource: requested_resource,
                                    result,
                                })
                                .await;
                        });
                    }
                }
                AppAction::LogsViewerOpen => {
                    let resource = app
                        .detail_view
                        .as_ref()
                        .and_then(|detail| detail.resource.clone())
                        .or_else(|| selected_resource(&app, &cached_snapshot));
                    let Some(resource) = resource else {
                        app.set_error("No resource selected for logs.".to_string());
                        continue;
                    };
                    let Some((pod_name, pod_ns, pod_resource)) = (match &resource {
                        ResourceRef::Pod(pod_name, pod_ns) => {
                            Some((pod_name.clone(), pod_ns.clone(), resource.clone()))
                        }
                        _ => None,
                    }) else {
                        if !matches!(
                            resource,
                            ResourceRef::Deployment(_, _)
                                | ResourceRef::StatefulSet(_, _)
                                | ResourceRef::DaemonSet(_, _)
                                | ResourceRef::ReplicaSet(_, _)
                                | ResourceRef::ReplicationController(_, _)
                                | ResourceRef::Job(_, _)
                        ) {
                            app.set_error(
                                    "Logs are only available for Pods and supported workload resources."
                                        .to_string(),
                                );
                            continue;
                        }
                        let session_id = next_workload_logs_session_id;
                        next_workload_logs_session_id =
                            next_workload_logs_session_id.wrapping_add(1).max(1);
                        app.detail_view = None;
                        app.open_workload_logs_tab(resource.clone(), session_id);
                        let tx = workload_logs_bootstrap_tx.clone();
                        let client_clone = client.get_client();
                        tokio::spawn(async move {
                            let result = resolve_workload_log_targets(client_clone, &resource)
                                .await
                                .map_err(|err| format!("{err:#}"));
                            let _ = tx
                                .send(WorkloadLogsBootstrapResult {
                                    session_id,
                                    resource,
                                    result,
                                })
                                .await;
                        });
                        continue;
                    };
                    let mut container_request: Option<(u64, String, String)> = None;
                    app.detail_view = None;
                    app.open_pod_logs_tab(pod_resource);
                    if let Some(tab) = app.workbench_mut().find_tab_mut(&WorkbenchTabKey::PodLogs(
                        ResourceRef::Pod(pod_name.clone(), pod_ns.clone()),
                    )) && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
                    {
                        logs_viewer_request_seq = logs_viewer_request_seq.wrapping_add(1);
                        let request_id = logs_viewer_request_seq;
                        logs_tab.viewer = LogsViewerState {
                            pod_name: pod_name.clone(),
                            pod_namespace: pod_ns.clone(),
                            loading: true,
                            pending_container_request_id: Some(request_id),
                            pending_logs_request_id: None,
                            container_cursor: 0,
                            container_name: String::new(),
                            containers: Vec::new(),
                            picking_container: false,
                            ..Default::default()
                        };
                        container_request = Some((request_id, pod_name, pod_ns));
                    }

                    if let Some((request_id, pod_name, pod_ns)) = container_request {
                        let client_clone = client.clone();
                        let tx = logs_viewer_tx.clone();
                        tokio::spawn(async move {
                            let pods_api: Api<Pod> =
                                Api::namespaced(client_clone.get_client(), &pod_ns);
                            let result = pods_api
                                .get(&pod_name)
                                .await
                                .map_err(|err| err.to_string())
                                .map(|pod| {
                                    pod.spec
                                        .map(|spec| {
                                            spec.containers
                                                .into_iter()
                                                .map(|container| container.name)
                                                .collect::<Vec<_>>()
                                        })
                                        .unwrap_or_default()
                                });
                            let _ = tx
                                .send(LogsViewerAsyncResult::Containers {
                                    request_id,
                                    pod_name,
                                    namespace: pod_ns,
                                    result,
                                })
                                .await;
                        });
                    }
                }
                AppAction::OpenExec => {
                    let resource = app
                        .detail_view
                        .as_ref()
                        .and_then(|detail| detail.resource.clone())
                        .or_else(|| selected_resource(&app, &cached_snapshot));
                    let Some(ResourceRef::Pod(pod_name, pod_ns)) = resource else {
                        app.set_error("Exec is only available for Pod resources.".to_string());
                        continue;
                    };

                    let session_id = next_exec_session_id;
                    next_exec_session_id = next_exec_session_id.wrapping_add(1).max(1);
                    let resource = ResourceRef::Pod(pod_name.clone(), pod_ns.clone());
                    app.detail_view = None;
                    app.open_exec_tab(
                        resource.clone(),
                        session_id,
                        pod_name.clone(),
                        pod_ns.clone(),
                    );
                    let tx = exec_bootstrap_tx.clone();
                    let client_clone = client.clone();
                    tokio::spawn(async move {
                        let result = fetch_pod_containers(&client_clone, &pod_name, &pod_ns)
                            .await
                            .map_err(|err| format!("{err:#}"));
                        let _ = tx
                            .send(ExecBootstrapResult {
                                session_id,
                                resource,
                                result,
                            })
                            .await;
                    });
                }
                AppAction::ExecSelectContainer(container_name) => {
                    let mut start_session: Option<(u64, ResourceRef, String, String, String)> =
                        None;
                    if let Some(tab) = app.workbench_mut().active_tab_mut()
                        && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
                    {
                        exec_tab.picking_container = false;
                        exec_tab.container_name = container_name.clone();
                        exec_tab.loading = true;
                        exec_tab.error = None;
                        start_session = Some((
                            exec_tab.session_id,
                            exec_tab.resource.clone(),
                            exec_tab.pod_name.clone(),
                            exec_tab.namespace.clone(),
                            container_name,
                        ));
                    }
                    if let Some((session_id, resource, pod_name, namespace, container_name)) =
                        start_session
                    {
                        match spawn_exec_session(
                            client.clone(),
                            session_id,
                            pod_name,
                            namespace,
                            container_name.clone(),
                            exec_update_tx.clone(),
                        )
                        .await
                        {
                            Ok(handle) => {
                                exec_sessions.insert(session_id, handle);
                            }
                            Err(err) => {
                                if let Some(tab) = app
                                    .workbench_mut()
                                    .find_tab_mut(&WorkbenchTabKey::Exec(resource))
                                    && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
                                {
                                    exec_tab.loading = false;
                                    exec_tab.error = Some(format!("{err:#}"));
                                }
                            }
                        }
                    }
                }
                AppAction::ExecSendInput => {
                    if let Some(tab) = app.workbench_mut().active_tab_mut()
                        && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
                    {
                        if exec_tab.input.is_empty() {
                            continue;
                        }
                        let mut bytes = exec_tab.input.clone().into_bytes();
                        bytes.push(b'\n');
                        let session_id = exec_tab.session_id;
                        exec_tab.input.clear();
                        if let Some(handle) = exec_sessions.get(&session_id) {
                            if let Err(err) = handle.input_tx.send(bytes).await {
                                exec_tab.error = Some(format!("failed to send exec input: {err}"));
                            }
                        } else {
                            exec_tab.error = Some(
                                "exec session is not running for the selected tab.".to_string(),
                            );
                        }
                    }
                }
                AppAction::LogsViewerSelectContainer(container) => {
                    let mut logs_request: Option<(u64, String, String, String)> = None;
                    if let Some(tab) = app.workbench_mut().active_tab_mut()
                        && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
                    {
                        let viewer = &mut logs_tab.viewer;
                        logs_viewer_request_seq = logs_viewer_request_seq.wrapping_add(1);
                        let request_id = logs_viewer_request_seq;
                        viewer.picking_container = false;
                        viewer.container_name = container.clone();
                        viewer.loading = true;
                        viewer.lines.clear();
                        viewer.scroll_offset = 0;
                        viewer.error = None;
                        viewer.pending_logs_request_id = Some(request_id);
                        logs_request = Some((
                            request_id,
                            viewer.pod_name.clone(),
                            viewer.pod_namespace.clone(),
                            container,
                        ));
                    }

                    if let Some((request_id, pod_name, pod_ns, container_name)) = logs_request {
                        let client_clone = client.clone();
                        let tx = logs_viewer_tx.clone();
                        tokio::spawn(async move {
                            let logs_client = LogsClient::new(client_clone.get_client());
                            let pod_ref = PodRef::new(pod_name.clone(), pod_ns.clone());
                            let result = logs_client
                                .tail_logs(&pod_ref, Some(500), Some(container_name.as_str()))
                                .await
                                .map_err(|err| err.to_string());
                            let _ = tx
                                .send(LogsViewerAsyncResult::Tail {
                                    request_id,
                                    pod_name,
                                    namespace: pod_ns,
                                    container_name,
                                    result,
                                })
                                .await;
                        });
                    }
                }
                AppAction::LogsViewerSelectAllContainers => {
                    // Gather pod info from the current PodLogs tab, then replace
                    // it with a WorkloadLogs tab streaming all containers.
                    let all_info: Option<(String, String, Vec<String>, ResourceRef)> =
                        app.workbench().active_tab().and_then(|tab| {
                            if let WorkbenchTabState::PodLogs(logs_tab) = &tab.state {
                                let v = &logs_tab.viewer;
                                Some((
                                    v.pod_name.clone(),
                                    v.pod_namespace.clone(),
                                    v.containers.clone(),
                                    logs_tab.resource.clone(),
                                ))
                            } else {
                                None
                            }
                        });

                    if let Some((pod_name, pod_ns, containers, resource)) = all_info {
                        // Close the single-container PodLogs tab
                        app.workbench.close_active_tab();

                        // Open a WorkloadLogs tab for the same pod
                        let session_id = next_workload_logs_session_id;
                        next_workload_logs_session_id =
                            next_workload_logs_session_id.wrapping_add(1).max(1);
                        app.open_workload_logs_tab(resource, session_id);

                        // Build sources: all containers from this single pod
                        let sources: Vec<(String, String, String)> = containers
                            .iter()
                            .map(|c| (pod_name.clone(), pod_ns.clone(), c.clone()))
                            .collect();

                        // Configure the tab directly
                        if let Some(tab) = app.workbench_mut().active_tab_mut()
                            && let WorkbenchTabState::WorkloadLogs(logs_tab) = &mut tab.state
                            && logs_tab.session_id == session_id
                        {
                            logs_tab.sources = sources.clone();
                            logs_tab.loading = false;
                            workload_log_sessions.insert(session_id, sources.clone());
                        }

                        // Start streaming each container
                        for (pod, ns, container) in sources {
                            let _ = coordinator
                                .start_log_streaming(pod, ns, container, true, false, false)
                                .await;
                        }
                    }
                }
                AppAction::LogsViewerToggleFollow => {
                    let follow_info = app.workbench().active_tab().and_then(|tab| {
                        if let WorkbenchTabState::PodLogs(logs_tab) = &tab.state {
                            let v = &logs_tab.viewer;
                            Some((
                                v.pod_name.clone(),
                                v.pod_namespace.clone(),
                                v.container_name.clone(),
                                v.follow_mode,
                                v.picking_container,
                                v.show_timestamps,
                            ))
                        } else {
                            None
                        }
                    });
                    if let Some((
                        pod_name,
                        pod_ns,
                        container_name,
                        was_following,
                        picking_container,
                        timestamps,
                    )) = follow_info
                    {
                        if !was_following
                            && (pod_name.is_empty()
                                || container_name.is_empty()
                                || picking_container)
                        {
                            if let Some(tab) = app.workbench_mut().active_tab_mut()
                                && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
                            {
                                let viewer = &mut logs_tab.viewer;
                                viewer.error = Some(
                                    "Select a container before enabling follow mode.".to_string(),
                                );
                            }
                        } else {
                            apply_action(AppAction::LogsViewerToggleFollow, &mut app);
                            if !was_following {
                                let _ = coordinator
                                    .start_log_streaming(
                                        pod_name,
                                        pod_ns,
                                        container_name,
                                        true,
                                        false,
                                        timestamps,
                                    )
                                    .await;
                            } else if !pod_name.is_empty() && !container_name.is_empty() {
                                let _ = coordinator
                                    .stop_log_streaming(&pod_name, &pod_ns, &container_name)
                                    .await;
                            }
                        }
                    }
                }
                AppAction::LogsViewerTogglePrevious => {
                    let prev_info = app.workbench().active_tab().and_then(|tab| {
                        if let WorkbenchTabState::PodLogs(logs_tab) = &tab.state {
                            let v = &logs_tab.viewer;
                            if v.picking_container || v.container_name.is_empty() {
                                return None;
                            }
                            Some((
                                v.pod_name.clone(),
                                v.pod_namespace.clone(),
                                v.container_name.clone(),
                                v.previous_logs,
                                v.follow_mode,
                            ))
                        } else {
                            None
                        }
                    });
                    if let Some((pod_name, pod_ns, container_name, was_previous, was_following)) =
                        prev_info
                    {
                        // Cancel any current log stream
                        if was_following || was_previous {
                            let _ = coordinator
                                .stop_log_streaming(&pod_name, &pod_ns, &container_name)
                                .await;
                        }

                        // Toggle previous_logs and reset viewer state
                        if let Some(tab) = app.workbench_mut().active_tab_mut()
                            && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
                        {
                            let viewer = &mut logs_tab.viewer;
                            viewer.previous_logs = !was_previous;
                            viewer.follow_mode = false;
                            viewer.lines.clear();
                            viewer.scroll_offset = 0;
                            viewer.loading = true;
                            viewer.error = None;

                            logs_viewer_request_seq = logs_viewer_request_seq.wrapping_add(1);
                            let request_id = logs_viewer_request_seq;
                            viewer.pending_logs_request_id = Some(request_id);

                            let new_previous = viewer.previous_logs;
                            let tx = logs_viewer_tx.clone();
                            let client_clone = client.clone();
                            let pn = pod_name.clone();
                            let pns = pod_ns.clone();
                            let cn = container_name.clone();
                            tokio::spawn(async move {
                                let logs_client = LogsClient::new(client_clone.get_client());
                                let pod_ref = PodRef::new(pn.clone(), pns.clone());
                                let tail = Some(500);
                                let result = if new_previous {
                                    logs_client
                                        .tail_previous_logs(&pod_ref, tail, Some(cn.as_str()))
                                        .await
                                        .map_err(|err| err.to_string())
                                } else {
                                    logs_client
                                        .tail_logs(&pod_ref, tail, Some(cn.as_str()))
                                        .await
                                        .map_err(|err| err.to_string())
                                };
                                let _ = tx
                                    .send(LogsViewerAsyncResult::Tail {
                                        request_id,
                                        pod_name: pn,
                                        namespace: pns,
                                        container_name: cn,
                                        result,
                                    })
                                    .await;
                            });
                        }
                    }
                }
                AppAction::LogsViewerToggleTimestamps => {
                    let ts_info = app.workbench().active_tab().and_then(|tab| {
                        if let WorkbenchTabState::PodLogs(logs_tab) = &tab.state {
                            let v = &logs_tab.viewer;
                            if v.picking_container || v.container_name.is_empty() {
                                return None;
                            }
                            Some((
                                v.pod_name.clone(),
                                v.pod_namespace.clone(),
                                v.container_name.clone(),
                                v.show_timestamps,
                                v.follow_mode,
                                v.previous_logs,
                            ))
                        } else {
                            None
                        }
                    });
                    if let Some((
                        pod_name,
                        pod_ns,
                        container_name,
                        was_timestamps,
                        was_following,
                        is_previous,
                    )) = ts_info
                    {
                        if was_following || was_timestamps {
                            let _ = coordinator
                                .stop_log_streaming(&pod_name, &pod_ns, &container_name)
                                .await;
                        }
                        if let Some(tab) = app.workbench_mut().active_tab_mut()
                            && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
                        {
                            let viewer = &mut logs_tab.viewer;
                            viewer.show_timestamps = !viewer.show_timestamps;
                            viewer.lines.clear();
                            viewer.scroll_offset = 0;
                            viewer.loading = true;
                            viewer.error = None;
                        }
                        let new_timestamps = !was_timestamps;
                        let follow = !is_previous;
                        let _ = coordinator
                            .start_log_streaming(
                                pod_name,
                                pod_ns,
                                container_name,
                                follow,
                                is_previous,
                                new_timestamps,
                            )
                            .await;
                    }
                }
                AppAction::PortForwardOpen => {
                    let resource = app
                        .detail_view
                        .as_ref()
                        .and_then(|detail| detail.resource.clone())
                        .or_else(|| selected_resource(&app, &cached_snapshot));
                    let dialog = match &resource {
                        Some(ResourceRef::Pod(name, ns)) => {
                            PortForwardDialog::with_target(ns, name, 0)
                        }
                        _ => {
                            app.set_error(
                                "Port forwarding is only available for Pod resources.".to_string(),
                            );
                            continue;
                        }
                    };
                    app.detail_view = None;
                    app.open_port_forward_tab(resource, dialog);
                    let tunnels = port_forwarder.list_tunnels();
                    app.tunnel_registry.update_tunnels(tunnels.clone());
                    if let Some(tab) = app
                        .workbench_mut()
                        .find_tab_mut(&WorkbenchTabKey::PortForward)
                        && let WorkbenchTabState::PortForward(port_tab) = &mut tab.state
                    {
                        let mut registry = kubectui::state::port_forward::TunnelRegistry::new();
                        registry.update_tunnels(tunnels);
                        port_tab.dialog.update_registry(registry);
                    }
                }
                AppAction::ScaleDialogSubmit => {
                    if !app
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::Scale))
                    {
                        app.set_error(
                            "Scale is unavailable for the selected resource.".to_string(),
                        );
                        continue;
                    }
                    let scale_info = app.detail_view.as_ref().and_then(|d| {
                        let replicas = d.scale_dialog.as_ref()?.desired_replicas_as_int()?;
                        match d.resource.as_ref()? {
                            ResourceRef::Deployment(name, namespace) => Some((
                                ResourceRef::Deployment(name.clone(), namespace.clone()),
                                name.clone(),
                                namespace.clone(),
                                "Deployment",
                                replicas,
                            )),
                            ResourceRef::StatefulSet(name, namespace) => Some((
                                ResourceRef::StatefulSet(name.clone(), namespace.clone()),
                                name.clone(),
                                namespace.clone(),
                                "StatefulSet",
                                replicas,
                            )),
                            _ => None,
                        }
                    });
                    if let Some((resource, name, namespace, kind_label, replicas)) = scale_info {
                        let resource_label =
                            format!("{kind_label} '{name}' in namespace '{namespace}'");
                        let origin_view = app.view();
                        let action_history_id = app.record_action_pending(
                            ActionKind::Scale,
                            origin_view,
                            Some(resource.clone()),
                            resource_label.clone(),
                            format!("Scaling {resource_label} to {replicas}..."),
                        );
                        begin_detail_mutation(
                            &mut app,
                            &mut status_message_clear_at,
                            format!("Scaling {resource_label} to {replicas}..."),
                        );
                        let tx = scale_tx.clone();
                        let c = client.clone();
                        let context_generation = refresh_state.context_generation;
                        tokio::spawn(async move {
                            let result = match &resource {
                                ResourceRef::Deployment(..) => {
                                    c.scale_deployment(&name, &namespace, replicas).await
                                }
                                ResourceRef::StatefulSet(..) => {
                                    c.scale_statefulset(&name, &namespace, replicas).await
                                }
                                _ => unreachable!("validated scalable resource"),
                            }
                            .map_err(|e| format!("{e:#}"));
                            let _ = tx
                                .send(ScaleAsyncResult {
                                    action_history_id,
                                    context_generation,
                                    origin_view,
                                    resource,
                                    target_replicas: replicas,
                                    resource_label,
                                    result,
                                })
                                .await;
                        });
                    }
                }
                AppAction::RolloutRestart => {
                    if !app
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::Restart))
                    {
                        app.set_error(
                            "Restart is unavailable for the selected resource.".to_string(),
                        );
                        continue;
                    }
                    let restart_info = app.detail_view.as_ref().and_then(|d| {
                        d.resource.as_ref().and_then(|r| match r {
                            ResourceRef::Deployment(name, ns) => {
                                Some(("deployment".to_string(), name.clone(), ns.clone()))
                            }
                            ResourceRef::StatefulSet(name, ns) => {
                                Some(("statefulset".to_string(), name.clone(), ns.clone()))
                            }
                            ResourceRef::DaemonSet(name, ns) => {
                                Some(("daemonset".to_string(), name.clone(), ns.clone()))
                            }
                            _ => None,
                        })
                    });
                    if let Some((kind, name, namespace)) = restart_info {
                        let resource_label =
                            format!("{} '{}' in namespace '{}'", kind, name, namespace);
                        let origin_view = app.view();
                        let resource = match kind.as_str() {
                            "deployment" => {
                                ResourceRef::Deployment(name.clone(), namespace.clone())
                            }
                            "statefulset" => {
                                ResourceRef::StatefulSet(name.clone(), namespace.clone())
                            }
                            "daemonset" => ResourceRef::DaemonSet(name.clone(), namespace.clone()),
                            _ => unreachable!("validated restartable resource"),
                        };
                        let action_history_id = app.record_action_pending(
                            ActionKind::Restart,
                            origin_view,
                            Some(resource),
                            resource_label.clone(),
                            format!("Requesting restart for {resource_label}..."),
                        );
                        begin_detail_mutation(
                            &mut app,
                            &mut status_message_clear_at,
                            format!("Requesting restart for {resource_label}..."),
                        );
                        let tx = rollout_tx.clone();
                        let c = client.clone();
                        let context_generation = refresh_state.context_generation;
                        tokio::spawn(async move {
                            let result = c
                                .rollout_restart(&kind, &name, &namespace)
                                .await
                                .map_err(|e| format!("{e:#}"));
                            let _ = tx
                                .send(RolloutRestartAsyncResult {
                                    action_history_id,
                                    context_generation,
                                    origin_view,
                                    resource_label,
                                    result,
                                })
                                .await;
                        });
                    }
                }
                AppAction::DeleteResource => {
                    if !app
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::Delete))
                    {
                        app.set_error(
                            "Delete is unavailable for the selected resource.".to_string(),
                        );
                        continue;
                    }
                    let delete_resource = app.detail_view.as_ref().and_then(|d| d.resource.clone());
                    if let Some(resource) = delete_resource {
                        if delete_in_flight_id.is_some() {
                            app.set_error("Delete already in progress".to_string());
                            continue;
                        }

                        if let Some(detail) = &mut app.detail_view {
                            detail.confirm_delete = false;
                            detail.loading = true;
                        }

                        delete_request_seq = delete_request_seq.wrapping_add(1);
                        let request_id = delete_request_seq;
                        delete_in_flight_id = Some(request_id);
                        let resource_label = format!("{} '{}'", resource.kind(), resource.name());
                        let origin_view = app.view();
                        let action_history_id = app.record_action_pending(
                            ActionKind::Delete,
                            origin_view,
                            Some(resource.clone()),
                            resource_label.clone(),
                            format!("Deleting {resource_label}..."),
                        );
                        begin_detail_mutation(
                            &mut app,
                            &mut status_message_clear_at,
                            format!("Deleting {resource_label}..."),
                        );
                        spawn_delete_task(
                            delete_tx.clone(),
                            client.clone(),
                            resource,
                            request_id,
                            action_history_id,
                            refresh_state.context_generation,
                            origin_view,
                            false,
                        );
                    }
                }
                AppAction::ForceDeleteResource => {
                    if !app
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::Delete))
                    {
                        app.set_error(
                            "Delete is unavailable for the selected resource.".to_string(),
                        );
                        continue;
                    }
                    let delete_resource = app.detail_view.as_ref().and_then(|d| d.resource.clone());
                    if let Some(resource) = delete_resource {
                        if delete_in_flight_id.is_some() {
                            app.set_error("Delete already in progress".to_string());
                            continue;
                        }

                        if let Some(detail) = &mut app.detail_view {
                            detail.confirm_delete = false;
                            detail.loading = true;
                        }

                        delete_request_seq = delete_request_seq.wrapping_add(1);
                        let request_id = delete_request_seq;
                        delete_in_flight_id = Some(request_id);
                        let resource_label = format!("{} '{}'", resource.kind(), resource.name());
                        let origin_view = app.view();
                        let action_history_id = app.record_action_pending(
                            ActionKind::Delete,
                            origin_view,
                            Some(resource.clone()),
                            resource_label.clone(),
                            format!("Force deleting {resource_label}..."),
                        );
                        begin_detail_mutation(
                            &mut app,
                            &mut status_message_clear_at,
                            format!("Force deleting {resource_label}..."),
                        );
                        spawn_delete_task(
                            delete_tx.clone(),
                            client.clone(),
                            resource,
                            request_id,
                            action_history_id,
                            refresh_state.context_generation,
                            origin_view,
                            true,
                        );
                    }
                }
                AppAction::TriggerCronJob => {
                    let cronjob_info = app.detail_view.as_ref().and_then(|d| {
                        if let Some(ResourceRef::CronJob(name, ns)) = &d.resource {
                            Some((name.clone(), ns.clone()))
                        } else {
                            None
                        }
                    });
                    if let Some((name, namespace)) = cronjob_info {
                        let resource_label = format!("CronJob '{name}'");
                        let origin_view = app.view();
                        let action_history_id = app.record_action_pending(
                            ActionKind::Trigger,
                            origin_view,
                            app.detail_view.as_ref().and_then(|d| d.resource.clone()),
                            resource_label.clone(),
                            format!("Triggering {resource_label}..."),
                        );
                        begin_detail_mutation(
                            &mut app,
                            &mut status_message_clear_at,
                            format!("Triggering {resource_label}..."),
                        );
                        let tx = trigger_cronjob_tx.clone();
                        let c = client.clone();
                        let context_generation = refresh_state.context_generation;
                        tokio::spawn(async move {
                            let result = c
                                .trigger_cronjob(&name, &namespace)
                                .await
                                .map_err(|e| format!("{e:#}"));
                            let _ = tx
                                .send(TriggerCronJobAsyncResult {
                                    action_history_id,
                                    context_generation,
                                    origin_view,
                                    resource_label,
                                    result,
                                })
                                .await;
                        });
                    }
                }
                AppAction::ConfirmDrainNode => {
                    if let Some(detail) = &mut app.detail_view
                        && detail.supports_action(DetailAction::Drain)
                    {
                        detail.confirm_drain = true;
                    }
                }
                AppAction::CordonNode if !node_op_in_flight => {
                    let node_name = app.detail_view.as_ref().and_then(|d| {
                        if let Some(ResourceRef::Node(name)) = &d.resource {
                            Some(name.clone())
                        } else {
                            None
                        }
                    });
                    if let Some(name) = node_name {
                        node_op_in_flight = true;
                        let resource_label = format!("Node '{name}'");
                        let origin_view = app.view();
                        let action_history_id = app.record_action_pending(
                            ActionKind::Cordon,
                            origin_view,
                            app.detail_view.as_ref().and_then(|d| d.resource.clone()),
                            resource_label.clone(),
                            format!("Cordoning {resource_label}..."),
                        );
                        begin_detail_mutation(
                            &mut app,
                            &mut status_message_clear_at,
                            format!("Cordoning {resource_label}..."),
                        );
                        let tx = node_ops_tx.clone();
                        let c = client.clone();
                        let context_generation = refresh_state.context_generation;
                        tokio::spawn(async move {
                            let result = c.cordon_node(&name).await.map_err(|e| format!("{e:#}"));
                            let _ = tx
                                .send(NodeOpsAsyncResult {
                                    action_history_id,
                                    context_generation,
                                    origin_view,
                                    node_name: name,
                                    op_kind: NodeOpKind::Cordon,
                                    result,
                                })
                                .await;
                        });
                    }
                }
                AppAction::CordonNode => {} // in-flight guard
                AppAction::UncordonNode if !node_op_in_flight => {
                    let node_name = app.detail_view.as_ref().and_then(|d| {
                        if let Some(ResourceRef::Node(name)) = &d.resource {
                            Some(name.clone())
                        } else {
                            None
                        }
                    });
                    if let Some(name) = node_name {
                        node_op_in_flight = true;
                        let resource_label = format!("Node '{name}'");
                        let origin_view = app.view();
                        let action_history_id = app.record_action_pending(
                            ActionKind::Uncordon,
                            origin_view,
                            app.detail_view.as_ref().and_then(|d| d.resource.clone()),
                            resource_label.clone(),
                            format!("Uncordoning {resource_label}..."),
                        );
                        begin_detail_mutation(
                            &mut app,
                            &mut status_message_clear_at,
                            format!("Uncordoning {resource_label}..."),
                        );
                        let tx = node_ops_tx.clone();
                        let c = client.clone();
                        let context_generation = refresh_state.context_generation;
                        tokio::spawn(async move {
                            let result = c.uncordon_node(&name).await.map_err(|e| format!("{e:#}"));
                            let _ = tx
                                .send(NodeOpsAsyncResult {
                                    action_history_id,
                                    context_generation,
                                    origin_view,
                                    node_name: name,
                                    op_kind: NodeOpKind::Uncordon,
                                    result,
                                })
                                .await;
                        });
                    }
                }
                AppAction::UncordonNode => {} // in-flight guard
                AppAction::DrainNode | AppAction::ForceDrainNode if !node_op_in_flight => {
                    let force = matches!(action, AppAction::ForceDrainNode);
                    // Always dismiss the confirmation dialog, even if node_name is None.
                    if let Some(detail) = &mut app.detail_view {
                        detail.confirm_drain = false;
                    }
                    let node_name = app.detail_view.as_ref().and_then(|d| {
                        if let Some(ResourceRef::Node(name)) = &d.resource {
                            Some(name.clone())
                        } else {
                            None
                        }
                    });
                    if let Some(name) = node_name {
                        node_op_in_flight = true;
                        let force_label = if force { " (force)" } else { "" };
                        let resource_label = format!("Node '{name}'");
                        let origin_view = app.view();
                        let action_history_id = app.record_action_pending(
                            ActionKind::Drain,
                            origin_view,
                            app.detail_view.as_ref().and_then(|d| d.resource.clone()),
                            resource_label.clone(),
                            format!("Draining{force_label} {resource_label}..."),
                        );
                        begin_detail_mutation(
                            &mut app,
                            &mut status_message_clear_at,
                            format!("Draining{force_label} {resource_label}..."),
                        );
                        let tx = node_ops_tx.clone();
                        let c = client.clone();
                        let context_generation = refresh_state.context_generation;
                        tokio::spawn(async move {
                            let result = c
                                .drain_node(&name, 300, 30, force)
                                .await
                                .map_err(|e| format!("{e:#}"));
                            let _ = tx
                                .send(NodeOpsAsyncResult {
                                    action_history_id,
                                    context_generation,
                                    origin_view,
                                    node_name: name,
                                    op_kind: NodeOpKind::Drain,
                                    result,
                                })
                                .await;
                        });
                    }
                }
                AppAction::DrainNode | AppAction::ForceDrainNode => {
                    // In-flight guard — dismiss dialog even when blocked.
                    if let Some(detail) = &mut app.detail_view {
                        detail.confirm_drain = false;
                    }
                }
                AppAction::CopyResourceName => {
                    let name = app
                        .detail_view
                        .as_ref()
                        .and_then(|d| d.resource.as_ref())
                        .map(|r| r.name().to_string())
                        .or_else(|| {
                            selected_resource(&app, &cached_snapshot).map(|r| r.name().to_string())
                        });
                    if let Some(name) = name {
                        if let Err(e) = kubectui::clipboard::copy_to_clipboard(&name) {
                            app.set_error(format!("Clipboard error: {e}"));
                        } else {
                            app.status_message = Some(format!("Copied: {name}"));
                        }
                    }
                }
                AppAction::CopyResourceFullName => {
                    let full = app
                        .detail_view
                        .as_ref()
                        .and_then(|d| d.resource.as_ref())
                        .map(|r| match r.namespace() {
                            Some(ns) => format!("{ns}/{}", r.name()),
                            None => r.name().to_string(),
                        })
                        .or_else(|| {
                            selected_resource(&app, &cached_snapshot).map(|r| match r.namespace() {
                                Some(ns) => format!("{ns}/{}", r.name()),
                                None => r.name().to_string(),
                            })
                        });
                    if let Some(full) = full {
                        if let Err(e) = kubectui::clipboard::copy_to_clipboard(&full) {
                            app.set_error(format!("Clipboard error: {e}"));
                        } else {
                            app.status_message = Some(format!("Copied: {full}"));
                        }
                    }
                }
                AppAction::CopyLogContent => {
                    let content = app
                        .workbench()
                        .active_tab()
                        .and_then(|tab| match &tab.state {
                            WorkbenchTabState::PodLogs(logs_tab) => {
                                if logs_tab.viewer.lines.is_empty() {
                                    None
                                } else {
                                    Some(logs_tab.viewer.lines.join("\n"))
                                }
                            }
                            WorkbenchTabState::WorkloadLogs(wl_tab) => {
                                if wl_tab.lines.is_empty() {
                                    None
                                } else {
                                    Some(
                                        wl_tab
                                            .lines
                                            .iter()
                                            .map(|l| {
                                                format!(
                                                    "{}:{} {}",
                                                    l.pod_name, l.container_name, l.content
                                                )
                                            })
                                            .collect::<Vec<_>>()
                                            .join("\n"),
                                    )
                                }
                            }
                            _ => None,
                        });
                    if let Some(content) = content {
                        let line_count = content.lines().count();
                        if let Err(e) = kubectui::clipboard::copy_to_clipboard(&content) {
                            app.set_error(format!("Clipboard error: {e}"));
                        } else {
                            app.status_message = Some(format!("Copied {line_count} log lines"));
                        }
                    }
                }
                AppAction::ExportLogs => {
                    let export_data =
                        app.workbench()
                            .active_tab()
                            .and_then(|tab| match &tab.state {
                                WorkbenchTabState::PodLogs(logs_tab) => {
                                    if logs_tab.viewer.lines.is_empty() {
                                        None
                                    } else {
                                        let label = format!(
                                            "{}-{}",
                                            logs_tab.viewer.pod_name,
                                            logs_tab.viewer.container_name,
                                        );
                                        Some((label, logs_tab.viewer.lines.join("\n")))
                                    }
                                }
                                WorkbenchTabState::WorkloadLogs(wl_tab) => {
                                    if wl_tab.lines.is_empty() {
                                        None
                                    } else {
                                        let label = tab.state.title().replace(' ', "-");
                                        let content = wl_tab
                                            .lines
                                            .iter()
                                            .map(|l| {
                                                format!(
                                                    "{}:{} {}",
                                                    l.pod_name, l.container_name, l.content
                                                )
                                            })
                                            .collect::<Vec<_>>()
                                            .join("\n");
                                        Some((label, content))
                                    }
                                }
                                _ => None,
                            });
                    if let Some((label, content)) = export_data {
                        match kubectui::export::save_logs_to_file(&label, &content) {
                            Ok(path) => {
                                app.status_message = Some(format!("Saved to {}", path.display()));
                            }
                            Err(e) => {
                                app.set_error(format!("Export error: {e}"));
                            }
                        }
                    }
                }
                AppAction::EditYaml => {
                    if !app
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::EditYaml))
                    {
                        app.set_error(
                            "YAML editing is unavailable for the selected resource.".to_string(),
                        );
                        continue;
                    }
                    // Gather what we need before suspending the TUI
                    let edit_info = app.detail_view.as_ref().and_then(|d| {
                        d.resource.as_ref().zip(d.yaml.as_ref()).map(|(r, y)| {
                            (
                                r.kind().to_ascii_lowercase(),
                                r.name().to_string(),
                                r.namespace().map(str::to_owned),
                                y.clone(),
                            )
                        })
                    });

                    if let Some((kind, name, namespace, yaml_content)) = edit_info {
                        // Write YAML to a temp file with unique suffix to prevent
                        // symlink attacks from predictable paths.
                        let nonce = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0u64, |d| d.as_nanos() as u64)
                            ^ std::process::id() as u64;
                        let tmp_path = std::env::temp_dir()
                            .join(format!("kubectui-{kind}-{name}-{nonce:016x}.yaml"));
                        if let Err(err) = std::fs::write(&tmp_path, &yaml_content) {
                            app.set_error(format!("Failed to write temp file: {err}"));
                        } else {
                            // Suspend TUI — restore terminal to canonical mode
                            let _ = restore_terminal(terminal);

                            // Spawn $EDITOR (fallback: vi)
                            let editor = std::env::var("EDITOR")
                                .or_else(|_| std::env::var("VISUAL"))
                                .unwrap_or_else(|_| "vi".to_string());

                            let status =
                                std::process::Command::new(&editor).arg(&tmp_path).status();

                            // Re-init TUI regardless of editor outcome
                            match setup_terminal() {
                                Ok(new_terminal) => *terminal = new_terminal,
                                Err(err) => {
                                    eprintln!("Failed to restore terminal: {err:#}");
                                    app.should_quit = true;
                                    continue;
                                }
                            }

                            match status {
                                Err(err) => {
                                    app.set_error(format!(
                                        "Failed to launch editor '{editor}': {err}"
                                    ));
                                }
                                Ok(exit) if !exit.success() => {
                                    // Editor exited non-zero (e.g. :cq in vim) — treat as cancel
                                }
                                Ok(_) => {
                                    // Read back the edited file
                                    match std::fs::read_to_string(&tmp_path) {
                                        Err(err) => {
                                            app.set_error(format!(
                                                "Failed to read edited file: {err}"
                                            ));
                                        }
                                        Ok(edited_yaml) => {
                                            if edited_yaml.trim() == yaml_content.trim() {
                                                // No changes — skip apply
                                            } else {
                                                let origin_view = app.view();
                                                let resource_label = format!(
                                                    "{} '{}'{}",
                                                    kind,
                                                    name,
                                                    namespace
                                                        .as_deref()
                                                        .map(|ns| format!(" in namespace '{ns}'"))
                                                        .unwrap_or_default()
                                                );
                                                let jump_resource = app
                                                    .detail_view
                                                    .as_ref()
                                                    .and_then(|detail| detail.resource.clone());
                                                let action_history_id = app.record_action_pending(
                                                    ActionKind::ApplyYaml,
                                                    origin_view,
                                                    jump_resource,
                                                    resource_label.clone(),
                                                    format!(
                                                        "Applying changes to {resource_label}..."
                                                    ),
                                                );
                                                match client
                                                    .apply_resource_yaml(
                                                        &edited_yaml,
                                                        &kind,
                                                        &name,
                                                        namespace.as_deref(),
                                                    )
                                                    .await
                                                {
                                                    Ok(()) => {
                                                        app.complete_action_history(
                                                                action_history_id,
                                                                ActionStatus::Succeeded,
                                                                format!(
                                                                    "Applied changes to {resource_label}."
                                                                ),
                                                                true,
                                                            );
                                                        app.detail_view = None;
                                                        app.focus = kubectui::app::Focus::Content;
                                                        finish_mutation_success(
                                                            &mut app,
                                                            &mut MutationRuntime {
                                                                global_state: &mut global_state,
                                                                client: &client,
                                                                refresh_tx: &refresh_tx,
                                                                deferred_refresh_tx:
                                                                    &deferred_refresh_tx,
                                                                refresh_state: &mut refresh_state,
                                                                snapshot_dirty: &mut snapshot_dirty,
                                                                auto_refresh: &mut auto_refresh,
                                                                status_message_clear_at:
                                                                    &mut status_message_clear_at,
                                                            },
                                                            origin_view,
                                                            format!(
                                                                "Applied changes to {} '{}'. Refreshing view...",
                                                                kind, name
                                                            ),
                                                            false,
                                                            MUTATION_REFRESH_DELAYS_SECS,
                                                        );
                                                    }
                                                    Err(err) => {
                                                        app.complete_action_history(
                                                            action_history_id,
                                                            ActionStatus::Failed,
                                                            format!("Apply failed: {err:#}"),
                                                            true,
                                                        );
                                                        app.set_error(format!(
                                                            "Apply failed: {err:#}"
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            // Clean up temp file
                            let _ = std::fs::remove_file(&tmp_path);
                        }
                    }
                }
                AppAction::SaveDecodedSecret => {
                    let decoded_state =
                        app.workbench()
                            .active_tab()
                            .and_then(|tab| match &tab.state {
                                WorkbenchTabState::DecodedSecret(secret_tab) => Some((
                                    secret_tab.resource.clone(),
                                    secret_tab.source_yaml.clone(),
                                    secret_tab.entries.clone(),
                                )),
                                _ => None,
                            });

                    let Some((resource, source_yaml, entries)) = decoded_state else {
                        app.set_error(
                            "Decoded Secret save is only available from the decoded Secret tab."
                                .to_string(),
                        );
                        continue;
                    };
                    let Some(source_yaml) = source_yaml else {
                        app.set_error("Secret YAML is not loaded yet.".to_string());
                        continue;
                    };
                    let encoded_yaml = match encode_secret_yaml(&source_yaml, &entries) {
                        Ok(encoded_yaml) => encoded_yaml,
                        Err(err) => {
                            app.set_error(format!("Failed to encode Secret data: {err:#}"));
                            continue;
                        }
                    };
                    let kind = resource.kind().to_ascii_lowercase();
                    let name = resource.name().to_string();
                    let namespace = resource.namespace().map(str::to_owned);
                    let origin_view = app.view();
                    let resource_label = format!(
                        "{} '{}'{}",
                        resource.kind(),
                        name,
                        namespace
                            .as_deref()
                            .map(|ns| format!(" in namespace '{ns}'"))
                            .unwrap_or_default()
                    );
                    let action_history_id = app.record_action_pending(
                        ActionKind::ApplyYaml,
                        origin_view,
                        Some(resource.clone()),
                        resource_label.clone(),
                        format!("Applying decoded Secret changes to {resource_label}..."),
                    );
                    match client
                        .apply_resource_yaml(&encoded_yaml, &kind, &name, namespace.as_deref())
                        .await
                    {
                        Ok(()) => {
                            app.complete_action_history(
                                action_history_id,
                                ActionStatus::Succeeded,
                                format!("Applied decoded Secret changes to {resource_label}."),
                                true,
                            );
                            if let Some(tab) = app
                                .workbench_mut()
                                .find_tab_mut(&WorkbenchTabKey::DecodedSecret(resource.clone()))
                                && let WorkbenchTabState::DecodedSecret(secret_tab) = &mut tab.state
                            {
                                match decode_secret_yaml(&encoded_yaml) {
                                    Ok(decoded_entries) => {
                                        secret_tab.source_yaml = Some(encoded_yaml.clone());
                                        secret_tab.entries = decoded_entries;
                                        secret_tab.editing = false;
                                        secret_tab.edit_input.clear();
                                        secret_tab.error = None;
                                        secret_tab.loading = false;
                                        secret_tab.clamp_selected();
                                    }
                                    Err(err) => {
                                        secret_tab.error = Some(err.to_string());
                                    }
                                }
                            }
                            finish_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                origin_view,
                                format!(
                                    "Applied decoded Secret changes to {} '{}'. Refreshing view...",
                                    resource.kind(),
                                    name
                                ),
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            app.complete_action_history(
                                action_history_id,
                                ActionStatus::Failed,
                                format!("Apply failed: {err:#}"),
                                true,
                            );
                            app.set_error(format!("Apply failed: {err:#}"));
                        }
                    }
                }
                AppAction::PortForwardCreate((target, config)) => {
                    match port_forwarder.create_tunnel_async(target, config).await {
                        Ok(tunnel_id) => {
                            app.clear_error();
                            let tunnels = port_forwarder.list_tunnels();
                            app.tunnel_registry.update_tunnels(tunnels.clone());
                            if let Some(tab) = app
                                .workbench_mut()
                                .find_tab_mut(&WorkbenchTabKey::PortForward)
                                && let WorkbenchTabState::PortForward(port_tab) = &mut tab.state
                            {
                                port_tab.dialog.success =
                                    Some(format!("Tunnel created: {tunnel_id}"));
                                let mut registry =
                                    kubectui::state::port_forward::TunnelRegistry::new();
                                registry.update_tunnels(tunnels);
                                port_tab.dialog.update_registry(registry);
                            }
                        }
                        Err(err) => {
                            if let Some(tab) = app
                                .workbench_mut()
                                .find_tab_mut(&WorkbenchTabKey::PortForward)
                                && let WorkbenchTabState::PortForward(port_tab) = &mut tab.state
                            {
                                port_tab.dialog.error = Some(format!("{err}"));
                            }
                        }
                    }
                }
                AppAction::PortForwardRefresh => {
                    refresh_port_forward_workbench(
                        &mut app,
                        &port_forwarder,
                        &mut status_message_clear_at,
                    );
                }
                AppAction::PortForwardStop(tunnel_id) => {
                    match port_forwarder.stop_forward(&tunnel_id).await {
                        Ok(()) => {
                            let tunnels = port_forwarder.list_tunnels();
                            app.tunnel_registry.update_tunnels(tunnels.clone());
                            if let Some(tab) = app
                                .workbench_mut()
                                .find_tab_mut(&WorkbenchTabKey::PortForward)
                                && let WorkbenchTabState::PortForward(port_tab) = &mut tab.state
                            {
                                let mut registry =
                                    kubectui::state::port_forward::TunnelRegistry::new();
                                registry.update_tunnels(tunnels);
                                port_tab.dialog.success =
                                    Some(format!("Closed tunnel: {tunnel_id}"));
                                port_tab.dialog.update_registry(registry);
                            }
                        }
                        Err(err) => {
                            if let Some(tab) = app
                                .workbench_mut()
                                .find_tab_mut(&WorkbenchTabKey::PortForward)
                                && let WorkbenchTabState::PortForward(port_tab) = &mut tab.state
                            {
                                port_tab.dialog.error = Some(format!("{err:#}"));
                            }
                        }
                    }
                }
                AppAction::ScaleDialogOpen => {
                    if !app
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::Scale))
                    {
                        app.set_error(
                            "Scale is unavailable for the selected resource.".to_string(),
                        );
                        continue;
                    }
                    // Read actual replica count from snapshot before opening dialog
                    let scale_info = app.detail_view.as_ref().and_then(|d| {
                        d.resource.as_ref().and_then(|r| match r {
                            ResourceRef::Deployment(name, ns) => {
                                let replicas = cached_snapshot
                                    .deployments
                                    .iter()
                                    .find(|d| &d.name == name && &d.namespace == ns)
                                    .map(|d| d.desired_replicas)
                                    .unwrap_or(1);
                                Some((name.clone(), ns.clone(), replicas))
                            }
                            ResourceRef::StatefulSet(name, ns) => {
                                let replicas = cached_snapshot
                                    .statefulsets
                                    .iter()
                                    .find(|ss| &ss.name == name && &ss.namespace == ns)
                                    .map(|ss| ss.desired_replicas)
                                    .unwrap_or(1);
                                Some((name.clone(), ns.clone(), replicas))
                            }
                            _ => None,
                        })
                    });
                    if let Some((name, namespace, replicas)) = scale_info
                        && let Some(detail) = &mut app.detail_view
                    {
                        detail.scale_dialog = Some(
                                    kubectui::ui::components::scale_dialog::ScaleDialogState::new(
                                        match detail.resource.as_ref() {
                                            Some(ResourceRef::Deployment(_, _)) => {
                                                kubectui::ui::components::scale_dialog::ScaleTargetKind::Deployment
                                            }
                                            Some(ResourceRef::StatefulSet(_, _)) => {
                                                kubectui::ui::components::scale_dialog::ScaleTargetKind::StatefulSet
                                            }
                                            _ => {
                                                kubectui::ui::components::scale_dialog::ScaleTargetKind::Deployment
                                            }
                                        },
                                        name,
                                        namespace,
                                        replicas,
                                    ),
                                );
                    }
                }
                AppAction::ProbePanelOpen => {
                    if !app
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::Probes))
                    {
                        app.set_error(
                            "Probe inspection is only available for Pod resources.".to_string(),
                        );
                        continue;
                    }
                    let pod_info = app.detail_view.as_ref().and_then(|d| {
                        d.resource.as_ref().and_then(|r| match r {
                            ResourceRef::Pod(name, ns) => Some((name.clone(), ns.clone())),
                            _ => None,
                        })
                    });
                    if let Some((pod_name, pod_ns)) = pod_info {
                        let tx = probe_tx.clone();
                        let k = client.get_client();
                        let resource = ResourceRef::Pod(pod_name.clone(), pod_ns.clone());
                        tokio::spawn(async move {
                            let pods_api: Api<Pod> = Api::namespaced(k, &pod_ns);
                            let result = match pods_api.get(&pod_name).await {
                                Ok(pod) => Ok(extract_probes_from_pod(&pod).unwrap_or_default()),
                                Err(err) => Err(format!("Failed to load probes: {err}")),
                            };
                            let _ = tx.send(ProbeAsyncResult { resource, result }).await;
                        });
                    } else {
                        apply_action(AppAction::ProbePanelOpen, &mut app);
                    }
                }
                AppAction::CycleTheme => {
                    apply_action(AppAction::CycleTheme, &mut app);
                    app.needs_config_save = true;
                }
                other => {
                    apply_action(other, &mut app);
                }
            }

            // Clear stale deferred palette action if detail view was closed or errored
            if pending_palette_action.is_some()
                && (app.detail_view.is_none()
                    || app.detail_view.as_ref().is_some_and(|d| d.error.is_some()))
            {
                pending_palette_action = None;
            }

            // Persist preferences when dirty flag is set (debounced to at most once per second)
            if app.needs_config_save {
                let should_save =
                    last_config_save.is_none_or(|t| t.elapsed() >= Duration::from_secs(1));
                if should_save {
                    app.needs_config_save = false;
                    save_config(&app);
                    last_config_save = Some(Instant::now());
                }
            }
        }
    }

    // Final flush: persist any pending config changes before exit
    if app.needs_config_save {
        save_config(&app);
    }

    for (_, handle) in exec_sessions.drain() {
        let _ = handle.cancel_tx.send(());
    }
    let _ = coordinator.shutdown().await;
    port_forwarder.stop_all().await;

    Ok(())
}

fn spawn_extensions_fetch(
    client: &K8sClient,
    app: &mut AppState,
    snapshot: &ClusterSnapshot,
    tx: &tokio::sync::mpsc::Sender<ExtensionFetchResult>,
) {
    if app.view() != AppView::Extensions {
        return;
    }

    let Some(crd) = selected_extension_crd(app, snapshot).cloned() else {
        app.extension_instances.clear();
        app.extension_error = None;
        app.extension_selected_crd = None;
        return;
    };

    if app.extension_selected_crd.as_deref() == Some(crd.name.as_str()) {
        return;
    }

    app.begin_extension_instances_load(crd.name.clone());

    let namespace_owned = if crd.scope.eq_ignore_ascii_case("Namespaced") {
        namespace_scope(app.get_namespace()).map(ToString::to_string)
    } else {
        None
    };

    let client = client.clone();
    let crd = crd.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = client
            .fetch_custom_resources(&crd, namespace_owned.as_deref())
            .await
            .map_err(|e| e.to_string());
        let _ = tx
            .send(ExtensionFetchResult {
                crd_name: crd.name.clone(),
                result,
            })
            .await;
    });
}

fn apply_extension_fetch_result(app: &mut AppState, result: ExtensionFetchResult) {
    if app.extension_selected_crd.as_deref() != Some(result.crd_name.as_str()) {
        return;
    }

    match result.result {
        Ok(items) => app.set_extension_instances(result.crd_name, items, None),
        Err(err) => app.set_extension_instances(result.crd_name, Vec::new(), Some(err)),
    }
}

fn selected_extension_crd<'a>(
    app: &AppState,
    snapshot: &'a ClusterSnapshot,
) -> Option<&'a kubectui::k8s::dtos::CustomResourceDefinitionInfo> {
    kubectui::ui::views::extensions::crds::selected_crd(
        &snapshot.custom_resource_definitions,
        app.search_query(),
        app.selected_idx(),
    )
}

fn namespace_scope(namespace: &str) -> Option<&str> {
    if namespace == "all" {
        None
    } else {
        Some(namespace)
    }
}

fn filtered_index(indices: &[usize], idx: usize) -> Option<usize> {
    indices
        .get(idx.min(indices.len().saturating_sub(1)))
        .copied()
}

fn selected_resource(app: &AppState, snapshot: &ClusterSnapshot) -> Option<ResourceRef> {
    let idx = app.selected_idx();
    match app.view() {
        AppView::Dashboard => None,
        AppView::Bookmarks => selected_bookmark_resource(app.bookmarks(), idx, app.search_query()),
        AppView::Issues => {
            let issues = kubectui::state::issues::compute_issues(snapshot);
            let query = app.search_query().trim();
            let indices = kubectui::state::issues::filtered_issue_indices(&issues, query);
            filtered_index(&indices, idx).map(|i| issues[i].resource_ref.clone())
        }
        AppView::HelmCharts => None, // Helm repos are local config, no detail view
        AppView::PortForwarding => None, // Port forwards are managed via the dialog
        AppView::Extensions => {
            // The Extensions view has a split pane: CRDs (left) and instances (right).
            // When extension_in_instances is true, Enter opens the selected instance's detail.
            if !app.extension_in_instances {
                return None;
            }
            let crd = app.extension_selected_crd.as_ref().and_then(|crd_name| {
                snapshot
                    .custom_resource_definitions
                    .iter()
                    .find(|c| &c.name == crd_name)
            })?;
            let inst = app.extension_instances.get(
                app.extension_instance_cursor
                    .min(app.extension_instances.len().saturating_sub(1)),
            )?;
            Some(ResourceRef::CustomResource {
                name: inst.name.clone(),
                namespace: inst.namespace.clone(),
                group: crd.group.clone(),
                version: crd.version.clone(),
                kind: crd.kind.clone(),
                plural: crd.plural.clone(),
            })
        }
        AppView::FluxCDAlertProviders
        | AppView::FluxCDAlerts
        | AppView::FluxCDAll
        | AppView::FluxCDArtifacts
        | AppView::FluxCDHelmReleases
        | AppView::FluxCDHelmRepositories
        | AppView::FluxCDImages
        | AppView::FluxCDKustomizations
        | AppView::FluxCDReceivers
        | AppView::FluxCDSources
        | AppView::Nodes
        | AppView::Pods
        | AppView::Services
        | AppView::ResourceQuotas
        | AppView::LimitRanges
        | AppView::PodDisruptionBudgets
        | AppView::Deployments
        | AppView::StatefulSets
        | AppView::DaemonSets
        | AppView::ReplicaSets
        | AppView::ReplicationControllers
        | AppView::Jobs
        | AppView::CronJobs
        | AppView::Endpoints
        | AppView::Ingresses
        | AppView::IngressClasses
        | AppView::NetworkPolicies
        | AppView::ConfigMaps
        | AppView::Secrets
        | AppView::HPAs
        | AppView::PriorityClasses
        | AppView::PersistentVolumeClaims
        | AppView::PersistentVolumes
        | AppView::StorageClasses
        | AppView::Namespaces
        | AppView::Events
        | AppView::ServiceAccounts
        | AppView::Roles
        | AppView::RoleBindings
        | AppView::ClusterRoles
        | AppView::ClusterRoleBindings
        | AppView::HelmReleases => {
            let filtered = kubectui::ui::views::filtering::filtered_indices_for_view(
                app.view(),
                snapshot,
                app.search_query(),
                app.workload_sort(),
                app.pod_sort(),
            );
            let resource_idx = filtered_index(&filtered, idx)?;
            match app.view() {
                AppView::Nodes => {
                    Some(ResourceRef::Node(snapshot.nodes[resource_idx].name.clone()))
                }
                AppView::Pods => {
                    let pod = &snapshot.pods[resource_idx];
                    Some(ResourceRef::Pod(pod.name.clone(), pod.namespace.clone()))
                }
                AppView::Services => {
                    let service = &snapshot.services[resource_idx];
                    Some(ResourceRef::Service(
                        service.name.clone(),
                        service.namespace.clone(),
                    ))
                }
                AppView::ResourceQuotas => {
                    let quota = &snapshot.resource_quotas[resource_idx];
                    Some(ResourceRef::ResourceQuota(
                        quota.name.clone(),
                        quota.namespace.clone(),
                    ))
                }
                AppView::LimitRanges => {
                    let limit_range = &snapshot.limit_ranges[resource_idx];
                    Some(ResourceRef::LimitRange(
                        limit_range.name.clone(),
                        limit_range.namespace.clone(),
                    ))
                }
                AppView::PodDisruptionBudgets => {
                    let pdb = &snapshot.pod_disruption_budgets[resource_idx];
                    Some(ResourceRef::PodDisruptionBudget(
                        pdb.name.clone(),
                        pdb.namespace.clone(),
                    ))
                }
                AppView::Deployments => {
                    let deployment = &snapshot.deployments[resource_idx];
                    Some(ResourceRef::Deployment(
                        deployment.name.clone(),
                        deployment.namespace.clone(),
                    ))
                }
                AppView::StatefulSets => {
                    let statefulset = &snapshot.statefulsets[resource_idx];
                    Some(ResourceRef::StatefulSet(
                        statefulset.name.clone(),
                        statefulset.namespace.clone(),
                    ))
                }
                AppView::DaemonSets => {
                    let daemonset = &snapshot.daemonsets[resource_idx];
                    Some(ResourceRef::DaemonSet(
                        daemonset.name.clone(),
                        daemonset.namespace.clone(),
                    ))
                }
                AppView::ReplicaSets => {
                    let replicaset = &snapshot.replicasets[resource_idx];
                    Some(ResourceRef::ReplicaSet(
                        replicaset.name.clone(),
                        replicaset.namespace.clone(),
                    ))
                }
                AppView::ReplicationControllers => {
                    let controller = &snapshot.replication_controllers[resource_idx];
                    Some(ResourceRef::ReplicationController(
                        controller.name.clone(),
                        controller.namespace.clone(),
                    ))
                }
                AppView::Jobs => {
                    let job = &snapshot.jobs[resource_idx];
                    Some(ResourceRef::Job(job.name.clone(), job.namespace.clone()))
                }
                AppView::CronJobs => {
                    let cronjob = &snapshot.cronjobs[resource_idx];
                    Some(ResourceRef::CronJob(
                        cronjob.name.clone(),
                        cronjob.namespace.clone(),
                    ))
                }
                AppView::Endpoints => {
                    let endpoint = &snapshot.endpoints[resource_idx];
                    Some(ResourceRef::Endpoint(
                        endpoint.name.clone(),
                        endpoint.namespace.clone(),
                    ))
                }
                AppView::Ingresses => {
                    let ingress = &snapshot.ingresses[resource_idx];
                    Some(ResourceRef::Ingress(
                        ingress.name.clone(),
                        ingress.namespace.clone(),
                    ))
                }
                AppView::IngressClasses => Some(ResourceRef::IngressClass(
                    snapshot.ingress_classes[resource_idx].name.clone(),
                )),
                AppView::NetworkPolicies => {
                    let policy = &snapshot.network_policies[resource_idx];
                    Some(ResourceRef::NetworkPolicy(
                        policy.name.clone(),
                        policy.namespace.clone(),
                    ))
                }
                AppView::ConfigMaps => {
                    let config_map = &snapshot.config_maps[resource_idx];
                    Some(ResourceRef::ConfigMap(
                        config_map.name.clone(),
                        config_map.namespace.clone(),
                    ))
                }
                AppView::Secrets => {
                    let secret = &snapshot.secrets[resource_idx];
                    Some(ResourceRef::Secret(
                        secret.name.clone(),
                        secret.namespace.clone(),
                    ))
                }
                AppView::HPAs => {
                    let hpa = &snapshot.hpas[resource_idx];
                    Some(ResourceRef::Hpa(hpa.name.clone(), hpa.namespace.clone()))
                }
                AppView::PriorityClasses => Some(ResourceRef::PriorityClass(
                    snapshot.priority_classes[resource_idx].name.clone(),
                )),
                AppView::PersistentVolumeClaims => {
                    let pvc = &snapshot.pvcs[resource_idx];
                    Some(ResourceRef::Pvc(pvc.name.clone(), pvc.namespace.clone()))
                }
                AppView::PersistentVolumes => {
                    Some(ResourceRef::Pv(snapshot.pvs[resource_idx].name.clone()))
                }
                AppView::StorageClasses => Some(ResourceRef::StorageClass(
                    snapshot.storage_classes[resource_idx].name.clone(),
                )),
                AppView::Namespaces => Some(ResourceRef::Namespace(
                    snapshot.namespace_list[resource_idx].name.clone(),
                )),
                AppView::Events => {
                    let event = &snapshot.events[resource_idx];
                    Some(ResourceRef::Event(
                        event.name.clone(),
                        event.namespace.clone(),
                    ))
                }
                AppView::ServiceAccounts => {
                    let service_account = &snapshot.service_accounts[resource_idx];
                    Some(ResourceRef::ServiceAccount(
                        service_account.name.clone(),
                        service_account.namespace.clone(),
                    ))
                }
                AppView::Roles => {
                    let role = &snapshot.roles[resource_idx];
                    Some(ResourceRef::Role(role.name.clone(), role.namespace.clone()))
                }
                AppView::RoleBindings => {
                    let role_binding = &snapshot.role_bindings[resource_idx];
                    Some(ResourceRef::RoleBinding(
                        role_binding.name.clone(),
                        role_binding.namespace.clone(),
                    ))
                }
                AppView::ClusterRoles => Some(ResourceRef::ClusterRole(
                    snapshot.cluster_roles[resource_idx].name.clone(),
                )),
                AppView::ClusterRoleBindings => Some(ResourceRef::ClusterRoleBinding(
                    snapshot.cluster_role_bindings[resource_idx].name.clone(),
                )),
                AppView::HelmReleases => {
                    let release = &snapshot.helm_releases[resource_idx];
                    Some(ResourceRef::HelmRelease(
                        release.name.clone(),
                        release.namespace.clone(),
                    ))
                }
                AppView::FluxCDAlertProviders
                | AppView::FluxCDAlerts
                | AppView::FluxCDAll
                | AppView::FluxCDArtifacts
                | AppView::FluxCDHelmReleases
                | AppView::FluxCDHelmRepositories
                | AppView::FluxCDImages
                | AppView::FluxCDKustomizations
                | AppView::FluxCDReceivers
                | AppView::FluxCDSources => {
                    snapshot
                        .flux_resources
                        .get(resource_idx)
                        .map(|r| ResourceRef::CustomResource {
                            name: r.name.clone(),
                            namespace: r.namespace.clone(),
                            group: r.group.clone(),
                            version: r.version.clone(),
                            kind: r.kind.clone(),
                            plural: r.plural.clone(),
                        })
                }
                _ => None,
            }
        }
    }
}

fn selected_resource_context(
    app: &AppState,
    snapshot: &ClusterSnapshot,
) -> Option<ResourceActionContext> {
    let resource = selected_resource(app, snapshot)?;
    let node_unschedulable = match &resource {
        ResourceRef::Node(name) => snapshot
            .nodes
            .iter()
            .find(|node| &node.name == name)
            .map(|node| node.unschedulable),
        _ => None,
    };
    Some(ResourceActionContext {
        resource,
        node_unschedulable,
    })
}

fn prepare_bookmark_target(
    app: &mut AppState,
    snapshot: &ClusterSnapshot,
) -> Result<ResourceRef, String> {
    let resource = app
        .selected_bookmark_resource()
        .ok_or_else(|| "No bookmarked resource is selected.".to_string())?;

    if !resource_exists(snapshot, &resource) {
        return Err(format!(
            "Bookmarked {} '{}' is no longer present in the current snapshot.",
            resource.kind(),
            resource.name()
        ));
    }

    app.search_query.clear();
    app.is_search_mode = false;

    if let Some(view) = resource.primary_view() {
        app.view = view;
        app.focus = kubectui::app::Focus::Content;
        app.selected_idx = 0;
        app.apply_sort_from_preferences(kubectui::columns::view_key(view));
        if let Some(selected_idx) = bookmark_selected_index(
            view,
            snapshot,
            &resource,
            app.workload_sort(),
            app.pod_sort(),
        ) {
            app.selected_idx = selected_idx;
        }
    }

    Ok(resource)
}

fn selected_flux_reconcile_resource(
    app: &AppState,
    snapshot: &ClusterSnapshot,
) -> Result<ResourceRef, String> {
    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(app, snapshot))
        .ok_or_else(|| "No Flux resource is selected.".to_string())?;

    if let Some(reason) = resource.flux_reconcile_disabled_reason() {
        return Err(reason.to_string());
    }

    let ResourceRef::CustomResource {
        name,
        namespace,
        group,
        version,
        kind,
        plural,
    } = &resource
    else {
        return Err("Flux reconcile is only available for Flux toolkit resources.".to_string());
    };

    let is_suspended = snapshot.flux_resources.iter().any(|candidate| {
        candidate.name == *name
            && candidate.namespace == *namespace
            && candidate.group == *group
            && candidate.version == *version
            && candidate.kind == *kind
            && candidate.plural == *plural
            && candidate.suspended
    });

    if is_suspended {
        return Err(format!(
            "Flux reconcile is unavailable because {kind} '{name}' is suspended."
        ));
    }

    Ok(resource)
}

fn initial_loading_state(resource: ResourceRef, snapshot: &ClusterSnapshot) -> DetailViewState {
    let metadata = metadata_for_resource(snapshot, &resource);
    DetailViewState {
        resource: Some(resource),
        pending_request_id: None,
        metadata,
        loading: true,
        ..DetailViewState::default()
    }
}

async fn fetch_detail_view(
    client: &K8sClient,
    snapshot: &ClusterSnapshot,
    resource: ResourceRef,
) -> Result<DetailViewState> {
    let metadata = metadata_for_resource(snapshot, &resource);
    let sections = sections_for_resource(snapshot, &resource);

    // Run YAML, events, and metrics fetches concurrently (no dependencies between them).
    let ((yaml, yaml_error), events, (pod_metrics, node_metrics, metrics_unavailable_message)) = tokio::join!(
        fetch_detail_yaml(client, &resource),
        fetch_detail_events(client, &resource),
        fetch_detail_metrics(client, &resource),
    );

    Ok(DetailViewState {
        resource: Some(resource),
        pending_request_id: None,
        metadata,
        yaml,
        yaml_error,
        events,
        sections,
        pod_metrics,
        node_metrics,
        metrics_unavailable_message,
        loading: false,
        error: None,
        scale_dialog: None,
        probe_panel: None,
        confirm_delete: false,
        confirm_drain: false,
        metadata_expanded: false,
    })
}

async fn fetch_detail_yaml(
    client: &K8sClient,
    resource: &ResourceRef,
) -> (Option<String>, Option<String>) {
    let result = match resource {
        ResourceRef::CustomResource {
            group,
            version,
            kind,
            plural,
            name,
            namespace,
        } => {
            client
                .fetch_custom_resource_yaml(
                    group,
                    version,
                    kind,
                    plural,
                    name,
                    namespace.as_deref(),
                )
                .await
        }
        ResourceRef::HelmRelease(name, ns) => client.fetch_helm_release_yaml(name, ns).await,
        _ => {
            let kind = resource.kind().to_ascii_lowercase();
            let name = resource.name();
            let namespace = resource.namespace();
            client.fetch_resource_yaml(&kind, name, namespace).await
        }
    };
    match result {
        Ok(yaml) => (Some(yaml), None),
        Err(e) => (None, Some(format!("YAML fetch failed: {e}"))),
    }
}

async fn fetch_detail_events(
    client: &K8sClient,
    resource: &ResourceRef,
) -> Vec<kubectui::k8s::events::EventInfo> {
    match resource {
        ResourceRef::Pod(name, ns) => client.fetch_pod_events(name, ns).await.unwrap_or_default(),
        ResourceRef::Deployment(name, ns)
        | ResourceRef::StatefulSet(name, ns)
        | ResourceRef::DaemonSet(name, ns)
        | ResourceRef::ReplicaSet(name, ns)
        | ResourceRef::Job(name, ns)
        | ResourceRef::CronJob(name, ns)
        | ResourceRef::Service(name, ns)
        | ResourceRef::Ingress(name, ns)
        | ResourceRef::ConfigMap(name, ns)
        | ResourceRef::Pvc(name, ns)
        | ResourceRef::HelmRelease(name, ns) => {
            let kind = resource.kind();
            client
                .fetch_resource_events(kind, name, ns)
                .await
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

async fn fetch_detail_metrics(
    client: &K8sClient,
    resource: &ResourceRef,
) -> (
    Option<kubectui::k8s::dtos::PodMetricsInfo>,
    Option<kubectui::k8s::dtos::NodeMetricsInfo>,
    Option<String>,
) {
    match resource {
        ResourceRef::Pod(name, ns) => match client.fetch_pod_metrics(name, ns).await {
            Ok(Some(metrics)) => (Some(metrics), None, None),
            Ok(None) => (
                None,
                None,
                Some(
                    "metrics unavailable (metrics-server not installed or inaccessible)"
                        .to_string(),
                ),
            ),
            Err(err) => (None, None, Some(format!("metrics unavailable: {err}"))),
        },
        ResourceRef::Node(name) => match client.fetch_node_metrics(name).await {
            Ok(Some(metrics)) => (None, Some(metrics), None),
            Ok(None) => (
                None,
                None,
                Some(
                    "metrics unavailable (metrics-server not installed or inaccessible)"
                        .to_string(),
                ),
            ),
            Err(err) => (None, None, Some(format!("metrics unavailable: {err}"))),
        },
        _ => (None, None, None),
    }
}

fn find_flux_resource<'a>(
    snapshot: &'a ClusterSnapshot,
    name: &str,
    namespace: &Option<String>,
    group: &str,
    kind: &str,
) -> Option<&'a kubectui::k8s::dtos::FluxResourceInfo> {
    snapshot
        .flux_resources
        .iter()
        .find(|f| f.name == name && f.namespace == *namespace && f.group == group && f.kind == kind)
}

fn metadata_for_resource(snapshot: &ClusterSnapshot, resource: &ResourceRef) -> DetailMetadata {
    match resource {
        ResourceRef::Node(name) => {
            if let Some(node) = snapshot.nodes.iter().find(|n| &n.name == name) {
                DetailMetadata {
                    name: node.name.clone(),
                    namespace: None,
                    status: Some(if node.ready { "Ready" } else { "NotReady" }.to_string()),
                    node_unschedulable: Some(node.unschedulable),
                    node: Some(node.name.clone()),
                    ip: None,
                    created: None,
                    labels: Vec::new(),
                    ..DetailMetadata::default()
                }
            } else {
                DetailMetadata {
                    name: name.clone(),
                    ..DetailMetadata::default()
                }
            }
        }
        ResourceRef::Pod(name, ns) => {
            if let Some(pod) = snapshot
                .pods
                .iter()
                .find(|p| &p.name == name && &p.namespace == ns)
            {
                DetailMetadata {
                    name: pod.name.clone(),
                    namespace: Some(pod.namespace.clone()),
                    status: Some(pod.status.clone()),
                    node: pod.node.clone(),
                    ip: pod.pod_ip.clone(),
                    created: pod
                        .created_at
                        .map(|ts: chrono::DateTime<chrono::Utc>| ts.to_rfc3339()),
                    labels: pod.labels.clone(),
                    annotations: pod.annotations.clone(),
                    owner_references: pod.owner_references.clone(),
                    ..DetailMetadata::default()
                }
            } else {
                DetailMetadata {
                    name: name.clone(),
                    namespace: Some(ns.clone()),
                    ..DetailMetadata::default()
                }
            }
        }
        ResourceRef::Service(name, ns) => DetailMetadata {
            name: name.clone(),
            namespace: Some(ns.clone()),
            status: Some("Active".to_string()),
            ..DetailMetadata::default()
        },
        ResourceRef::Deployment(name, ns) => {
            let status = snapshot
                .deployments
                .iter()
                .find(|d| &d.name == name && &d.namespace == ns)
                .map(|d| format!("Ready {}/{}", d.ready_replicas, d.desired_replicas));

            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::StatefulSet(name, ns) => {
            let status = snapshot
                .statefulsets
                .iter()
                .find(|ss| &ss.name == name && &ss.namespace == ns)
                .map(|ss| format!("Ready {}/{}", ss.ready_replicas, ss.desired_replicas));

            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::ResourceQuota(name, ns) => {
            let status = snapshot
                .resource_quotas
                .iter()
                .find(|rq| &rq.name == name && &rq.namespace == ns)
                .map(|rq| {
                    let max_pct = rq
                        .percent_used
                        .values()
                        .fold(0.0_f64, |acc, value| acc.max(*value));
                    format!("Max usage {:.0}%", max_pct)
                });

            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::LimitRange(name, ns) => {
            let status = snapshot
                .limit_ranges
                .iter()
                .find(|lr| &lr.name == name && &lr.namespace == ns)
                .map(|lr| format!("{} limit specs", lr.limits.len()));

            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::PodDisruptionBudget(name, ns) => {
            let status = snapshot
                .pod_disruption_budgets
                .iter()
                .find(|pdb| &pdb.name == name && &pdb.namespace == ns)
                .map(|pdb| format!("Healthy {}/{}", pdb.current_healthy, pdb.desired_healthy));

            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::DaemonSet(name, ns) => {
            let status = snapshot
                .daemonsets
                .iter()
                .find(|ds| &ds.name == name && &ds.namespace == ns)
                .map(|ds| format!("Ready {}/{}", ds.ready_count, ds.desired_count));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::ReplicaSet(name, ns) => {
            let status = snapshot
                .replicasets
                .iter()
                .find(|rs| &rs.name == name && &rs.namespace == ns)
                .map(|rs| format!("Ready {}/{}", rs.ready, rs.desired));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::ReplicationController(name, ns) => {
            let status = snapshot
                .replication_controllers
                .iter()
                .find(|rc| &rc.name == name && &rc.namespace == ns)
                .map(|rc| format!("Ready {}/{}", rc.ready, rc.desired));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::Job(name, ns) => {
            let status = snapshot
                .jobs
                .iter()
                .find(|j| &j.name == name && &j.namespace == ns)
                .map(|j| j.status.clone());
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::CronJob(name, ns) => {
            let status = snapshot
                .cronjobs
                .iter()
                .find(|cj| &cj.name == name && &cj.namespace == ns)
                .map(|cj| {
                    if cj.suspend {
                        "Suspended".to_string()
                    } else {
                        "Active".to_string()
                    }
                });
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::Endpoint(name, ns) => DetailMetadata {
            name: name.clone(),
            namespace: Some(ns.clone()),
            status: Some("Active".to_string()),
            ..DetailMetadata::default()
        },
        ResourceRef::Ingress(name, ns) => {
            let status = snapshot
                .ingresses
                .iter()
                .find(|i| &i.name == name && &i.namespace == ns)
                .and_then(|i| i.address.clone());
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::IngressClass(name) => DetailMetadata {
            name: name.clone(),
            namespace: None,
            status: Some("Active".to_string()),
            ..DetailMetadata::default()
        },
        ResourceRef::NetworkPolicy(name, ns) => DetailMetadata {
            name: name.clone(),
            namespace: Some(ns.clone()),
            status: Some("Active".to_string()),
            ..DetailMetadata::default()
        },
        ResourceRef::ConfigMap(name, ns) => {
            let status = snapshot
                .config_maps
                .iter()
                .find(|cm| &cm.name == name && &cm.namespace == ns)
                .map(|cm| format!("{} keys", cm.data_count));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::Secret(name, ns) => {
            let status = snapshot
                .secrets
                .iter()
                .find(|s| &s.name == name && &s.namespace == ns)
                .map(|s| format!("{} ({} keys)", s.type_, s.data_count));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::Hpa(name, ns) => {
            let status = snapshot
                .hpas
                .iter()
                .find(|h| &h.name == name && &h.namespace == ns)
                .map(|h| format!("{}/{} replicas", h.current_replicas, h.max_replicas));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::PriorityClass(name) => {
            let status = snapshot
                .priority_classes
                .iter()
                .find(|pc| &pc.name == name)
                .map(|pc| format!("value: {}", pc.value));
            DetailMetadata {
                name: name.clone(),
                namespace: None,
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::Pvc(name, ns) => {
            let status = snapshot
                .pvcs
                .iter()
                .find(|pvc| &pvc.name == name && &pvc.namespace == ns)
                .map(|pvc| pvc.status.clone());
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::Pv(name) => {
            let status = snapshot
                .pvs
                .iter()
                .find(|pv| &pv.name == name)
                .map(|pv| pv.status.clone());
            DetailMetadata {
                name: name.clone(),
                namespace: None,
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::StorageClass(name) => DetailMetadata {
            name: name.clone(),
            namespace: None,
            status: Some("Active".to_string()),
            ..DetailMetadata::default()
        },
        ResourceRef::Namespace(name) => {
            let status = snapshot
                .namespace_list
                .iter()
                .find(|ns| &ns.name == name)
                .map(|ns| ns.status.clone());
            DetailMetadata {
                name: name.clone(),
                namespace: None,
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::Event(name, ns) => {
            let status = snapshot
                .events
                .iter()
                .find(|ev| &ev.name == name && &ev.namespace == ns)
                .map(|ev| ev.reason.clone());
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::ServiceAccount(name, ns) => DetailMetadata {
            name: name.clone(),
            namespace: Some(ns.clone()),
            status: Some("Active".to_string()),
            ..DetailMetadata::default()
        },
        ResourceRef::Role(name, ns) => {
            let status = snapshot
                .roles
                .iter()
                .find(|r| &r.name == name && &r.namespace == ns)
                .map(|r| format!("{} rules", r.rules.len()));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::RoleBinding(name, ns) => {
            let status = snapshot
                .role_bindings
                .iter()
                .find(|rb| &rb.name == name && &rb.namespace == ns)
                .map(|rb| format!("-> {}/{}", rb.role_ref_kind, rb.role_ref_name));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::ClusterRole(name) => {
            let status = snapshot
                .cluster_roles
                .iter()
                .find(|cr| &cr.name == name)
                .map(|cr| format!("{} rules", cr.rules.len()));
            DetailMetadata {
                name: name.clone(),
                namespace: None,
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::ClusterRoleBinding(name) => {
            let status = snapshot
                .cluster_role_bindings
                .iter()
                .find(|crb| &crb.name == name)
                .map(|crb| format!("-> {}/{}", crb.role_ref_kind, crb.role_ref_name));
            DetailMetadata {
                name: name.clone(),
                namespace: None,
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::HelmRelease(name, ns) => {
            let status = snapshot
                .helm_releases
                .iter()
                .find(|r| &r.name == name && &r.namespace == ns)
                .map(|r| r.status.clone());
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::CustomResource {
            name,
            namespace,
            kind,
            group,
            ..
        } => {
            if let Some(flux) = find_flux_resource(snapshot, name, namespace, group, kind) {
                DetailMetadata {
                    name: name.clone(),
                    namespace: namespace.clone(),
                    status: Some(flux.status.clone()),
                    created: flux
                        .created_at
                        .map(|ts: chrono::DateTime<chrono::Utc>| ts.to_rfc3339()),
                    flux_reconcile_enabled: !flux.suspended,
                    ..DetailMetadata::default()
                }
            } else {
                DetailMetadata {
                    name: name.clone(),
                    namespace: namespace.clone(),
                    status: Some(format!("{kind}.{group}")),
                    ..DetailMetadata::default()
                }
            }
        }
    }
}

fn sections_for_resource(snapshot: &ClusterSnapshot, resource: &ResourceRef) -> Vec<String> {
    match resource {
        ResourceRef::Node(name) => snapshot
            .nodes
            .iter()
            .find(|n| &n.name == name)
            .map(|node| {
                vec![
                    format!("Kubelet: {}", node.kubelet_version),
                    format!("OS Image: {}", node.os_image),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Pod(name, ns) => snapshot
            .pods
            .iter()
            .find(|p| &p.name == name && &p.namespace == ns)
            .map(|pod| {
                vec![
                    "CONTAINERS".to_string(),
                    format!("- restarts: {}", pod.restarts),
                    format!("- node: {}", pod.node.as_deref().unwrap_or("n/a")),
                    format!("- pod IP: {}", pod.pod_ip.as_deref().unwrap_or("n/a")),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Service(name, ns) => snapshot
            .services
            .iter()
            .find(|s| &s.name == name && &s.namespace == ns)
            .map(|svc| {
                vec![
                    "PORTS".to_string(),
                    if svc.ports.is_empty() {
                        "- none".to_string()
                    } else {
                        format!("- {}", svc.ports.join(", "))
                    },
                    format!("type: {}", svc.type_),
                    format!(
                        "cluster IP: {}",
                        svc.cluster_ip.as_deref().unwrap_or("None")
                    ),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Deployment(name, ns) => snapshot
            .deployments
            .iter()
            .find(|d| &d.name == name && &d.namespace == ns)
            .map(|dep| {
                vec![
                    "REPLICAS".to_string(),
                    format!("desired: {}", dep.desired_replicas),
                    format!("ready: {}", dep.ready_replicas),
                    format!("available: {}", dep.available_replicas),
                    format!("updated: {}", dep.updated_replicas),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::StatefulSet(name, ns) => snapshot
            .statefulsets
            .iter()
            .find(|ss| &ss.name == name && &ss.namespace == ns)
            .map(|ss| {
                vec![
                    "REPLICAS".to_string(),
                    format!("desired: {}", ss.desired_replicas),
                    format!("ready: {}", ss.ready_replicas),
                    format!("service: {}", ss.service_name),
                    format!("pod management: {}", ss.pod_management_policy),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::ResourceQuota(name, ns) => snapshot
            .resource_quotas
            .iter()
            .find(|rq| &rq.name == name && &rq.namespace == ns)
            .map(|rq| {
                let mut lines = vec!["QUOTAS".to_string()];
                for (key, hard) in rq.hard.iter().take(12) {
                    let used = rq.used.get(key).cloned().unwrap_or_else(|| "-".to_string());
                    let pct = rq
                        .percent_used
                        .get(key)
                        .map(|v| format!(" ({v:.0}%)"))
                        .unwrap_or_default();
                    lines.push(format!("{key}: {used}/{hard}{pct}"));
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::LimitRange(name, ns) => snapshot
            .limit_ranges
            .iter()
            .find(|lr| &lr.name == name && &lr.namespace == ns)
            .map(|lr| {
                let mut lines = vec!["LIMIT SPECS".to_string()];
                for spec in lr.limits.iter().take(8) {
                    lines.push(format!("type: {}", spec.type_));
                    if !spec.default.is_empty() {
                        lines.push(format!("  default: {}", map_to_kv(&spec.default)));
                    }
                    if !spec.default_request.is_empty() {
                        lines.push(format!(
                            "  defaultRequest: {}",
                            map_to_kv(&spec.default_request)
                        ));
                    }
                    if !spec.min.is_empty() {
                        lines.push(format!("  min: {}", map_to_kv(&spec.min)));
                    }
                    if !spec.max.is_empty() {
                        lines.push(format!("  max: {}", map_to_kv(&spec.max)));
                    }
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::PodDisruptionBudget(name, ns) => snapshot
            .pod_disruption_budgets
            .iter()
            .find(|pdb| &pdb.name == name && &pdb.namespace == ns)
            .map(|pdb| {
                vec![
                    "AVAILABILITY".to_string(),
                    format!(
                        "minAvailable: {}",
                        pdb.min_available.as_deref().unwrap_or("-")
                    ),
                    format!(
                        "maxUnavailable: {}",
                        pdb.max_unavailable.as_deref().unwrap_or("-")
                    ),
                    format!("currentHealthy: {}", pdb.current_healthy),
                    format!("desiredHealthy: {}", pdb.desired_healthy),
                    format!("disruptionsAllowed: {}", pdb.disruptions_allowed),
                    format!("expectedPods: {}", pdb.expected_pods),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::DaemonSet(name, ns) => snapshot
            .daemonsets
            .iter()
            .find(|ds| &ds.name == name && &ds.namespace == ns)
            .map(|ds| {
                vec![
                    "STATUS".to_string(),
                    format!("desired: {}", ds.desired_count),
                    format!("ready: {}", ds.ready_count),
                    format!("unavailable: {}", ds.unavailable_count),
                    format!("updateStrategy: {}", ds.update_strategy),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::ReplicaSet(name, ns) => snapshot
            .replicasets
            .iter()
            .find(|rs| &rs.name == name && &rs.namespace == ns)
            .map(|rs| {
                vec![
                    "REPLICAS".to_string(),
                    format!("desired: {}", rs.desired),
                    format!("ready: {}", rs.ready),
                    format!("available: {}", rs.available),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::ReplicationController(name, ns) => snapshot
            .replication_controllers
            .iter()
            .find(|rc| &rc.name == name && &rc.namespace == ns)
            .map(|rc| {
                vec![
                    "REPLICAS".to_string(),
                    format!("desired: {}", rc.desired),
                    format!("ready: {}", rc.ready),
                    format!("available: {}", rc.available),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Job(name, ns) => snapshot
            .jobs
            .iter()
            .find(|j| &j.name == name && &j.namespace == ns)
            .map(|j| {
                vec![
                    "JOB STATUS".to_string(),
                    format!("status: {}", j.status),
                    format!("completions: {}", j.completions),
                    format!("parallelism: {}", j.parallelism),
                    format!("active: {}", j.active_pods),
                    format!("failed: {}", j.failed_pods),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::CronJob(name, ns) => snapshot
            .cronjobs
            .iter()
            .find(|cj| &cj.name == name && &cj.namespace == ns)
            .map(|cj| {
                vec![
                    "SCHEDULE".to_string(),
                    format!("schedule: {}", cj.schedule),
                    format!("suspended: {}", cj.suspend),
                    format!("active: {}", cj.active_jobs),
                    format!(
                        "lastSchedule: {}",
                        cj.last_schedule_time
                            .map(|t| t.to_rfc3339())
                            .unwrap_or_else(|| "never".to_string())
                    ),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Endpoint(name, ns) => snapshot
            .endpoints
            .iter()
            .find(|e| &e.name == name && &e.namespace == ns)
            .map(|e| {
                let mut lines = vec!["ADDRESSES".to_string()];
                for addr in e.addresses.iter().take(10) {
                    lines.push(format!("- {addr}"));
                }
                if !e.ports.is_empty() {
                    lines.push("PORTS".to_string());
                    for port in e.ports.iter().take(10) {
                        lines.push(format!("- {port}"));
                    }
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::Ingress(name, ns) => snapshot
            .ingresses
            .iter()
            .find(|i| &i.name == name && &i.namespace == ns)
            .map(|i| {
                let mut lines = vec!["RULES".to_string()];
                for host in i.hosts.iter().take(10) {
                    lines.push(format!("- {host}"));
                }
                if let Some(addr) = &i.address {
                    lines.push(format!("address: {addr}"));
                }
                if let Some(class) = &i.class {
                    lines.push(format!("class: {class}"));
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::IngressClass(name) => snapshot
            .ingress_classes
            .iter()
            .find(|ic| &ic.name == name)
            .map(|ic| {
                vec![
                    format!("controller: {}", ic.controller),
                    format!("default: {}", ic.is_default),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::NetworkPolicy(name, ns) => snapshot
            .network_policies
            .iter()
            .find(|np| &np.name == name && &np.namespace == ns)
            .map(|np| {
                vec![
                    format!("podSelector: {}", np.pod_selector),
                    format!("ingressRules: {}", np.ingress_rules),
                    format!("egressRules: {}", np.egress_rules),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::ConfigMap(name, ns) => snapshot
            .config_maps
            .iter()
            .find(|cm| &cm.name == name && &cm.namespace == ns)
            .map(|cm| vec![format!("keys: {}", cm.data_count)])
            .unwrap_or_default(),
        ResourceRef::Secret(name, ns) => snapshot
            .secrets
            .iter()
            .find(|s| &s.name == name && &s.namespace == ns)
            .map(|s| {
                vec![
                    format!("type: {}", s.type_),
                    format!("keys: {}", s.data_count),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Hpa(name, ns) => snapshot
            .hpas
            .iter()
            .find(|h| &h.name == name && &h.namespace == ns)
            .map(|h| {
                vec![
                    format!("reference: {}", h.reference),
                    format!("minReplicas: {}", h.min_replicas.unwrap_or(1)),
                    format!("maxReplicas: {}", h.max_replicas),
                    format!("currentReplicas: {}", h.current_replicas),
                    format!("desiredReplicas: {}", h.desired_replicas),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::PriorityClass(name) => snapshot
            .priority_classes
            .iter()
            .find(|pc| &pc.name == name)
            .map(|pc| {
                vec![
                    format!("value: {}", pc.value),
                    format!("globalDefault: {}", pc.global_default),
                    format!("description: {}", pc.description),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Pvc(name, ns) => snapshot
            .pvcs
            .iter()
            .find(|pvc| &pvc.name == name && &pvc.namespace == ns)
            .map(|pvc| {
                vec![
                    format!("status: {}", pvc.status),
                    format!("capacity: {}", pvc.capacity.as_deref().unwrap_or("-")),
                    format!("accessModes: {}", pvc.access_modes.join(", ")),
                    format!(
                        "storageClass: {}",
                        pvc.storage_class.as_deref().unwrap_or("-")
                    ),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Pv(name) => snapshot
            .pvs
            .iter()
            .find(|pv| &pv.name == name)
            .map(|pv| {
                vec![
                    format!("status: {}", pv.status),
                    format!("capacity: {}", pv.capacity.as_deref().unwrap_or("-")),
                    format!("accessModes: {}", pv.access_modes.join(", ")),
                    format!("reclaimPolicy: {}", pv.reclaim_policy),
                    format!("claim: {}", pv.claim.as_deref().unwrap_or("-")),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::StorageClass(name) => snapshot
            .storage_classes
            .iter()
            .find(|sc| &sc.name == name)
            .map(|sc| {
                vec![
                    format!("provisioner: {}", sc.provisioner),
                    format!(
                        "reclaimPolicy: {}",
                        sc.reclaim_policy.as_deref().unwrap_or("-")
                    ),
                    format!(
                        "volumeBindingMode: {}",
                        sc.volume_binding_mode.as_deref().unwrap_or("-")
                    ),
                    format!("allowVolumeExpansion: {}", sc.allow_volume_expansion),
                    format!("default: {}", sc.is_default),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Namespace(name) => snapshot
            .namespace_list
            .iter()
            .find(|ns| &ns.name == name)
            .map(|ns| vec![format!("status: {}", ns.status)])
            .unwrap_or_default(),
        ResourceRef::Event(name, ns) => snapshot
            .events
            .iter()
            .find(|ev| &ev.name == name && &ev.namespace == ns)
            .map(|ev| {
                vec![
                    format!("reason: {}", ev.reason),
                    format!("type: {}", ev.type_),
                    format!("count: {}", ev.count),
                    format!("object: {}", ev.involved_object),
                    format!("message: {}", ev.message),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::ServiceAccount(name, ns) => snapshot
            .service_accounts
            .iter()
            .find(|sa| &sa.name == name && &sa.namespace == ns)
            .map(|sa| {
                vec![
                    format!("secrets: {}", sa.secrets_count),
                    format!("imagePullSecrets: {}", sa.image_pull_secrets_count),
                    format!(
                        "automountToken: {}",
                        sa.automount_service_account_token
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "unset".to_string())
                    ),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Role(name, ns) => snapshot
            .roles
            .iter()
            .find(|r| &r.name == name && &r.namespace == ns)
            .map(|r| {
                let mut lines = vec![format!("rules: {}", r.rules.len())];
                for rule in r.rules.iter().take(5) {
                    lines.push(format!(
                        "  {} on {}",
                        rule.verbs.join(","),
                        rule.resources.join(",")
                    ));
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::RoleBinding(name, ns) => snapshot
            .role_bindings
            .iter()
            .find(|rb| &rb.name == name && &rb.namespace == ns)
            .map(|rb| {
                let mut lines = vec![
                    format!("roleRef: {}/{}", rb.role_ref_kind, rb.role_ref_name),
                    format!("subjects: {}", rb.subjects.len()),
                ];
                for subj in rb.subjects.iter().take(5) {
                    lines.push(format!("  {} {}", subj.kind, subj.name));
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::ClusterRole(name) => snapshot
            .cluster_roles
            .iter()
            .find(|cr| &cr.name == name)
            .map(|cr| {
                let mut lines = vec![format!("rules: {}", cr.rules.len())];
                for rule in cr.rules.iter().take(5) {
                    lines.push(format!(
                        "  {} on {}",
                        rule.verbs.join(","),
                        rule.resources.join(",")
                    ));
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::ClusterRoleBinding(name) => snapshot
            .cluster_role_bindings
            .iter()
            .find(|crb| &crb.name == name)
            .map(|crb| {
                let mut lines = vec![
                    format!("roleRef: {}/{}", crb.role_ref_kind, crb.role_ref_name),
                    format!("subjects: {}", crb.subjects.len()),
                ];
                for subj in crb.subjects.iter().take(5) {
                    lines.push(format!("  {} {}", subj.kind, subj.name));
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::HelmRelease(name, ns) => snapshot
            .helm_releases
            .iter()
            .find(|r| &r.name == name && &r.namespace == ns)
            .map(|r| {
                let updated = r
                    .updated
                    .map(|ts| ts.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| "-".to_string());
                vec![
                    "HELM RELEASE".to_string(),
                    format!("chart: {}", r.chart),
                    format!("chartVersion: {}", r.chart_version),
                    format!("appVersion: {}", r.app_version),
                    format!("revision: {}", r.revision),
                    format!("status: {}", r.status),
                    format!("updated: {updated}"),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::CustomResource {
            name,
            namespace,
            kind,
            group,
            version,
            ..
        } => {
            if let Some(flux) = find_flux_resource(snapshot, name, namespace, group, kind) {
                flux_detail_sections(flux)
            } else {
                vec![
                    "CUSTOM RESOURCE".to_string(),
                    format!("kind: {kind}"),
                    format!("apiVersion: {group}/{version}"),
                ]
            }
        }
    }
}

fn flux_detail_sections(flux: &kubectui::k8s::dtos::FluxResourceInfo) -> Vec<String> {
    let mut lines = Vec::new();

    lines.push("RECONCILIATION".to_string());
    if let Some(ref sr) = flux.source_ref {
        lines.push(format!("source: {sr}"));
    }
    if let Some(ref interval) = flux.interval {
        lines.push(format!("interval: {interval}"));
    }
    if let Some(ref timeout) = flux.timeout {
        lines.push(format!("timeout: {timeout}"));
    }
    if let Some(ts) = flux.last_reconcile_time {
        lines.push(format!(
            "last reconcile: {}",
            ts.format("%Y-%m-%d %H:%M:%S UTC")
        ));
    }
    if flux.suspended {
        lines.push("suspended: true".to_string());
    }

    if flux.last_applied_revision.is_some() || flux.last_attempted_revision.is_some() {
        lines.push(String::new());
        lines.push("REVISIONS".to_string());
        if let Some(ref rev) = flux.last_applied_revision {
            let display = if rev.len() > 48 {
                &rev[..rev.floor_char_boundary(48)]
            } else {
                rev
            };
            lines.push(format!("applied: {display}"));
        }
        if let Some(ref rev) = flux.last_attempted_revision {
            let display = if rev.len() > 48 {
                &rev[..rev.floor_char_boundary(48)]
            } else {
                rev
            };
            lines.push(format!("attempted: {display}"));
        }
    }

    if flux.generation.is_some() || flux.observed_generation.is_some() {
        lines.push(String::new());
        lines.push("GENERATION".to_string());
        let cur = flux
            .generation
            .map(|g| g.to_string())
            .unwrap_or_else(|| "-".to_string());
        let obs = flux
            .observed_generation
            .map(|g| g.to_string())
            .unwrap_or_else(|| "-".to_string());
        let sync = if flux.observed_generation == flux.generation {
            "in sync"
        } else {
            "OUTDATED"
        };
        lines.push(format!("current: {cur}  observed: {obs}  ({sync})"));
    }

    if let Some(ref artifact) = flux.artifact {
        lines.push(String::new());
        lines.push("ARTIFACT".to_string());
        lines.push(artifact.clone());
    }

    if !flux.conditions.is_empty() {
        lines.push(String::new());
        lines.push("CONDITIONS".to_string());
        for cond in &flux.conditions {
            let ts = cond
                .timestamp
                .map(|t| t.format("%H:%M:%S").to_string())
                .unwrap_or_else(|| "-".to_string());
            let reason = cond.reason.as_deref().unwrap_or("-");
            let msg = cond.message.as_deref().unwrap_or("");
            let msg_display = if msg.len() > 60 {
                let end = msg.floor_char_boundary(57);
                format!("{}...", &msg[..end])
            } else {
                msg.to_string()
            };
            lines.push(format!(
                "  {}={} ({reason}) [{ts}] {msg_display}",
                cond.type_, cond.status
            ));
        }
    }

    lines
}

fn map_to_kv(map: &std::collections::BTreeMap<String, String>) -> String {
    map.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Shows a context picker at startup and returns a connected `K8sClient`.
/// If only one context exists or the user presses Esc, connects to the default context.
async fn pick_context_at_startup(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut AppState,
) -> Result<K8sClient> {
    let contexts = K8sClient::list_contexts();
    let current = kube::config::Kubeconfig::read()
        .ok()
        .and_then(|cfg| cfg.current_context);

    if contexts.len() <= 1 {
        return K8sClient::connect()
            .await
            .context("unable to initialize Kubernetes client");
    }

    app.open_context_picker(contexts, current);

    loop {
        let snapshot = kubectui::state::ClusterSnapshot::default();
        terminal
            .draw(|frame| ui::render(frame, app, &snapshot))
            .context("failed to render startup context picker")?;

        if event::poll(Duration::from_millis(16)).context("failed to poll events")?
            && let Event::Key(key) = event::read().context("failed to read event")?
        {
            match app.handle_key_event(key) {
                AppAction::SelectContext(ctx) => {
                    app.close_context_picker();
                    return K8sClient::connect_with_context(&ctx)
                        .await
                        .with_context(|| format!("failed to connect to context '{ctx}'"));
                }
                AppAction::CloseContextPicker | AppAction::Quit => {
                    app.close_context_picker();
                    return K8sClient::connect()
                        .await
                        .context("unable to initialize Kubernetes client");
                }
                _ => {}
            }
        }
    }
}

/// Configures terminal in alternate screen + raw mode for TUI rendering.
fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode().context("failed enabling raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("failed entering alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend).context("failed creating terminal backend")?;
    Ok(terminal)
}

/// Restores terminal state back to canonical mode.
fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode().context("failed disabling raw mode")?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .context("failed leaving alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ExtensionFetchResult, MAX_RECENT_EVENTS_CACHE_ITEMS, PendingFluxReconcileVerification,
        apply_extension_fetch_result, flux_observed_state, flux_reconcile_progress_observed,
        map_palette_detail_action, normalize_recent_events, palette_action_requires_loaded_detail,
        prepare_bookmark_target, process_flux_reconcile_verifications, refresh_options_for_view,
        selected_extension_crd, selected_flux_reconcile_resource, selected_resource,
        workbench_follow_streams_to_stop,
    };
    use chrono::{DateTime, Utc};
    use kubectui::{
        action_history::ActionKind,
        app::{AppAction, AppState, AppView, DetailViewState, ResourceRef},
        bookmarks::BookmarkEntry,
        k8s::dtos::{CustomResourceDefinitionInfo, FluxResourceInfo, K8sEventInfo, NodeInfo},
        policy::DetailAction,
        state::ClusterSnapshot,
        workbench::{PodLogsTabState, WorkbenchTabState},
    };
    use std::time::{Duration, Instant};

    #[test]
    fn selected_flux_reconcile_resource_rejects_suspended_flux_objects() {
        let mut app = AppState::default();
        app.view = AppView::FluxCDKustomizations;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.flux_resources.push(FluxResourceInfo {
            name: "apps".to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
            suspended: true,
            ..FluxResourceInfo::default()
        });

        let err = selected_flux_reconcile_resource(&app, &snapshot).expect_err("must reject");
        assert_eq!(
            err,
            "Flux reconcile is unavailable because Kustomization 'apps' is suspended."
        );
    }

    #[test]
    fn selected_flux_reconcile_resource_uses_detail_resource_when_present() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::CustomResource {
                name: "backend".to_string(),
                namespace: Some("flux-system".to_string()),
                group: "helm.toolkit.fluxcd.io".to_string(),
                version: "v2".to_string(),
                kind: "HelmRelease".to_string(),
                plural: "helmreleases".to_string(),
            }),
            ..DetailViewState::default()
        });

        let resource = selected_flux_reconcile_resource(&app, &ClusterSnapshot::default())
            .expect("detail flux resource is selected");
        assert_eq!(resource.kind(), "HelmRelease");
        assert_eq!(resource.name(), "backend");
        assert_eq!(resource.namespace(), Some("flux-system"));
    }

    #[test]
    fn closing_active_logs_tab_collects_follow_stream_to_stop() {
        let mut app = AppState::default();
        app.open_pod_logs_tab(ResourceRef::Pod("pod-0".to_string(), "ns".to_string()));
        if let Some(tab) = app.workbench_mut().active_tab_mut()
            && let WorkbenchTabState::PodLogs(PodLogsTabState { viewer, .. }) = &mut tab.state
        {
            viewer.pod_name = "pod-0".to_string();
            viewer.pod_namespace = "ns".to_string();
            viewer.container_name = "main".to_string();
            viewer.follow_mode = true;
        }

        let streams = workbench_follow_streams_to_stop(&app, AppAction::WorkbenchCloseActiveTab);
        assert_eq!(
            streams,
            vec![("pod-0".to_string(), "ns".to_string(), "main".to_string())]
        );
    }

    #[test]
    fn closing_non_logs_workbench_does_not_collect_streams() {
        let app = AppState::default();
        let streams = workbench_follow_streams_to_stop(&app, AppAction::WorkbenchCloseActiveTab);
        assert!(streams.is_empty());
    }

    #[test]
    fn selected_resource_clamps_to_last_visible_filtered_row() {
        let mut app = AppState::default();
        app.view = AppView::Nodes;
        app.focus = kubectui::app::Focus::Content;
        app.selected_idx = 8;
        app.search_query = "worker".to_string();

        let mut snapshot = ClusterSnapshot::default();
        snapshot.nodes = vec![
            NodeInfo {
                name: "control-plane".to_string(),
                created_at: Some(Utc::now()),
                ..NodeInfo::default()
            },
            NodeInfo {
                name: "worker-a".to_string(),
                created_at: Some(Utc::now()),
                ..NodeInfo::default()
            },
        ];

        let selected = selected_resource(&app, &snapshot).expect("selected resource");
        assert_eq!(selected, ResourceRef::Node("worker-a".to_string()));
    }

    #[test]
    fn selected_resource_uses_bookmark_view_selection() {
        let mut app = AppState::default();
        app.view = AppView::Bookmarks;
        app.current_context_name = Some("prod".to_string());
        app.cluster_preferences = Some(std::collections::HashMap::from([(
            "prod".to_string(),
            kubectui::preferences::ClusterPreferences {
                views: std::collections::HashMap::new(),
                bookmarks: vec![BookmarkEntry {
                    resource: ResourceRef::Secret("app-secret".to_string(), "default".to_string()),
                    bookmarked_at_unix: 1,
                }],
            },
        )]));

        let selected = selected_resource(&app, &ClusterSnapshot::default()).expect("bookmark");
        assert_eq!(
            selected,
            ResourceRef::Secret("app-secret".to_string(), "default".to_string())
        );
    }

    #[test]
    fn prepare_bookmark_target_navigates_to_resource_view() {
        let mut app = AppState::default();
        app.view = AppView::Bookmarks;
        app.focus = kubectui::app::Focus::Content;
        app.current_context_name = Some("prod".to_string());
        app.cluster_preferences = Some(std::collections::HashMap::from([(
            "prod".to_string(),
            kubectui::preferences::ClusterPreferences {
                views: std::collections::HashMap::new(),
                bookmarks: vec![BookmarkEntry {
                    resource: ResourceRef::Secret("app-secret".to_string(), "default".to_string()),
                    bookmarked_at_unix: 1,
                }],
            },
        )]));
        app.search_query = "app".to_string();

        let mut snapshot = ClusterSnapshot::default();
        snapshot.secrets.push(kubectui::k8s::dtos::SecretInfo {
            name: "app-secret".to_string(),
            namespace: "default".to_string(),
            ..Default::default()
        });

        let target = prepare_bookmark_target(&mut app, &snapshot).expect("bookmark target");
        assert_eq!(target.kind(), "Secret");
        assert_eq!(app.view, AppView::Secrets);
        assert_eq!(app.selected_idx, 0);
        assert!(app.search_query.is_empty());
    }

    #[test]
    fn selected_extension_crd_uses_filtered_query_selection() {
        let mut app = AppState::default();
        app.view = AppView::Extensions;
        app.search_query = "gadget".to_string();

        let mut snapshot = ClusterSnapshot::default();
        snapshot.custom_resource_definitions = vec![
            CustomResourceDefinitionInfo {
                name: "widgets.demo.io".to_string(),
                group: "demo.io".to_string(),
                version: "v1".to_string(),
                kind: "Widget".to_string(),
                plural: "widgets".to_string(),
                scope: "Namespaced".to_string(),
                instances: 1,
            },
            CustomResourceDefinitionInfo {
                name: "gadgets.demo.io".to_string(),
                group: "demo.io".to_string(),
                version: "v1".to_string(),
                kind: "Gadget".to_string(),
                plural: "gadgets".to_string(),
                scope: "Namespaced".to_string(),
                instances: 2,
            },
        ];

        let selected = selected_extension_crd(&app, &snapshot).expect("selected CRD");
        assert_eq!(selected.name, "gadgets.demo.io");
    }

    #[test]
    fn stale_extension_fetch_results_are_ignored() {
        let mut app = AppState::default();
        app.begin_extension_instances_load("widgets.demo.io".to_string());

        apply_extension_fetch_result(
            &mut app,
            ExtensionFetchResult {
                crd_name: "gadgets.demo.io".to_string(),
                result: Ok(Vec::new()),
            },
        );

        assert_eq!(
            app.extension_selected_crd.as_deref(),
            Some("widgets.demo.io")
        );
        assert!(app.extension_instances.is_empty());
        assert!(app.extension_error.is_none());
    }

    #[test]
    fn normalize_recent_events_sorts_and_truncates() {
        let now = Utc::now();
        let events = (0..300)
            .map(|idx| K8sEventInfo {
                name: format!("event-{idx}"),
                last_seen: Some(now - chrono::TimeDelta::seconds(i64::from(idx))),
                ..K8sEventInfo::default()
            })
            .collect::<Vec<_>>();

        let normalized = normalize_recent_events(events);
        assert_eq!(normalized.len(), MAX_RECENT_EVENTS_CACHE_ITEMS);
        assert_eq!(normalized[0].name, "event-0");
        assert_eq!(
            normalized.last().map(|event| event.name.as_str()),
            Some("event-249")
        );
    }

    #[test]
    fn events_view_uses_fast_refresh_profile() {
        let options = refresh_options_for_view(AppView::Events, false, false);
        assert!(!options.include_secondary_resources);
        assert!(!options.include_cluster_info);
    }

    #[test]
    fn palette_node_actions_require_loaded_detail() {
        assert!(palette_action_requires_loaded_detail(
            &AppAction::CordonNode
        ));
        assert!(palette_action_requires_loaded_detail(
            &AppAction::UncordonNode
        ));
        assert!(palette_action_requires_loaded_detail(
            &AppAction::ConfirmDrainNode
        ));
    }

    #[test]
    fn palette_drain_maps_to_confirmation_action() {
        assert_eq!(
            map_palette_detail_action(DetailAction::Drain),
            AppAction::ConfirmDrainNode
        );
    }

    fn test_flux_resource(
        status: &str,
        last_reconcile_time: Option<DateTime<Utc>>,
    ) -> FluxResourceInfo {
        FluxResourceInfo {
            name: "apps".to_string(),
            namespace: Some("flux-system".to_string()),
            kind: "Kustomization".to_string(),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            plural: "kustomizations".to_string(),
            status: status.to_string(),
            message: Some(String::new()),
            suspended: false,
            last_reconcile_time,
            ..FluxResourceInfo::default()
        }
    }

    fn test_flux_resource_ref() -> ResourceRef {
        ResourceRef::CustomResource {
            name: "apps".to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
        }
    }

    #[test]
    fn flux_reconcile_progress_detects_last_reconcile_change() {
        let baseline_time = Utc::now();
        let baseline = flux_observed_state(&test_flux_resource("Ready", Some(baseline_time)));
        let current = test_flux_resource(
            "Ready",
            Some(baseline_time + chrono::TimeDelta::seconds(30)),
        );

        assert!(flux_reconcile_progress_observed(Some(&baseline), &current));
    }

    #[test]
    fn process_flux_reconcile_verifications_marks_success_when_status_changes() {
        let mut app = AppState::default();
        let resource = test_flux_resource_ref();
        let entry_id = app.record_action_pending(
            ActionKind::FluxReconcile,
            AppView::FluxCDKustomizations,
            Some(resource.clone()),
            "Kustomization 'apps'".to_string(),
            "Requesting reconcile".to_string(),
        );
        let baseline_time = Utc::now();
        let baseline = flux_observed_state(&test_flux_resource("Ready", Some(baseline_time)));
        let snapshot = ClusterSnapshot {
            flux_resources: vec![test_flux_resource(
                "Ready",
                Some(baseline_time + chrono::TimeDelta::seconds(30)),
            )],
            ..ClusterSnapshot::default()
        };
        let mut pending = vec![PendingFluxReconcileVerification {
            action_history_id: entry_id,
            resource,
            resource_label: "Kustomization 'apps'".to_string(),
            baseline: Some(baseline),
            deadline: Instant::now() + Duration::from_secs(5),
        }];
        let mut clear_at = None;

        assert!(process_flux_reconcile_verifications(
            &mut app,
            &snapshot,
            &mut pending,
            &mut clear_at,
        ));
        assert!(pending.is_empty());
        assert!(
            app.action_history()
                .find_by_id(entry_id)
                .expect("history entry")
                .message
                .contains("Flux reconcile observed")
        );
        assert!(
            app.status_message()
                .expect("status message")
                .contains("Flux reconcile observed")
        );
    }

    #[test]
    fn process_flux_reconcile_verifications_reports_waiting_when_deadline_expires() {
        let mut app = AppState::default();
        let resource = test_flux_resource_ref();
        let entry_id = app.record_action_pending(
            ActionKind::FluxReconcile,
            AppView::FluxCDKustomizations,
            Some(resource.clone()),
            "Kustomization 'apps'".to_string(),
            "Requesting reconcile".to_string(),
        );
        let snapshot = ClusterSnapshot {
            flux_resources: vec![test_flux_resource("Ready", None)],
            ..ClusterSnapshot::default()
        };
        let mut pending = vec![PendingFluxReconcileVerification {
            action_history_id: entry_id,
            resource,
            resource_label: "Kustomization 'apps'".to_string(),
            baseline: Some(flux_observed_state(&test_flux_resource("Ready", None))),
            deadline: Instant::now() - Duration::from_secs(1),
        }];
        let mut clear_at = None;

        assert!(process_flux_reconcile_verifications(
            &mut app,
            &snapshot,
            &mut pending,
            &mut clear_at,
        ));
        assert!(pending.is_empty());
        assert!(
            app.action_history()
                .find_by_id(entry_id)
                .expect("history entry")
                .message
                .contains("Waiting for controller status update")
        );
    }
}
