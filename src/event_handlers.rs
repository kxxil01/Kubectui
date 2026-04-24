//! Pre-run_app helper functions extracted from `main.rs`.
//!
//! These cover coordinator message application, workbench session cleanup,
//! refresh orchestration, and events fetching.

use std::time::{Duration, Instant};

use anyhow::Result;

use kubectui::{
    app::{AppAction, AppState, AppView, DetailViewState, ResourceRef},
    coordinator::{LogStreamStatus, UpdateMessage},
    k8s::{
        client::K8sClient, helm::HelmHistoryResult, portforward::PortForwarderService,
        rollout::RolloutInspection,
    },
    resource_diff::{ResourceDiffResult, YamlDocumentDiffResult},
    secret::decode_secret_yaml,
    state::{GlobalState, RefreshOptions, RefreshScope},
    workbench::{WorkbenchTabKey, WorkbenchTabState},
};

use crate::async_types::{
    EventsAsyncResult, EventsFetchRuntimeState, MutationRuntime, QueuedRefresh, RefreshAsyncResult,
    RefreshDispatch, RefreshRuntimeState,
};
use crate::mutation_helpers::{
    finish_mutation_success, queue_deferred_refreshes, set_transient_status,
};

pub(crate) const STARTUP_NAMESPACE_FETCH_TIMEOUT_SECS: u64 = 3;
pub(crate) const STARTUP_NAMESPACE_FETCH_ATTEMPTS: usize = 2;
pub(crate) const STARTUP_NAMESPACE_FETCH_RETRY_DELAY_MS: u64 = 150;
pub(crate) const FLUX_AUTO_REFRESH_EVERY: u64 = 3;
pub(crate) const MUTATION_REFRESH_DELAYS_SECS: &[u64] = &[2, 5];
pub(crate) const FLUX_RECONCILE_REFRESH_DELAYS_SECS: &[u64] = &[2, 5, 9];
pub(crate) const MAX_RECENT_EVENTS_CACHE_ITEMS: usize = 250;

/// Applies coordinator update messages to app state.
pub(crate) fn apply_coordinator_msg(msg: UpdateMessage, app: &mut AppState) {
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
                    WorkbenchTabState::WorkloadLogs(logs_tab)
                        if logs_tab.sources.iter().any(|(pod, ns, container)| {
                            pod == &pod_name && ns == &namespace && container == &container_name
                        }) =>
                    {
                        logs_tab.push_line(kubectui::workbench::WorkloadLogLine {
                            pod_name: pod_name.clone(),
                            container_name: container_name.clone(),
                            entry: kubectui::log_investigation::LogEntry::from_raw(line.clone()),
                            is_stderr: false,
                        });
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
                    WorkbenchTabState::WorkloadLogs(logs_tab)
                        if logs_tab.sources.iter().any(|(pod, ns, container)| {
                            pod == &pod_name && ns == &namespace && container == &container_name
                        }) =>
                    {
                        logs_tab.loading = false;
                        if let LogStreamStatus::Error(err) = &status {
                            logs_tab.notice = Some(format!("{pod_name}/{container_name}: {err}"));
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

pub(crate) fn apply_detail_state_to_workbench(
    app: &mut AppState,
    request_id: u64,
    state: &DetailViewState,
) {
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

pub(crate) fn apply_detail_error_to_workbench(
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

pub(crate) fn apply_resource_diff_result_to_workbench(
    app: &mut AppState,
    request_id: u64,
    resource: &ResourceRef,
    diff: ResourceDiffResult,
) {
    if let Some(tab) = app
        .workbench
        .find_tab_mut(&WorkbenchTabKey::ResourceDiff(resource.clone()))
        && let WorkbenchTabState::ResourceDiff(diff_tab) = &mut tab.state
        && diff_tab.pending_request_id == Some(request_id)
    {
        diff_tab.apply_result(diff);
    }
}

pub(crate) fn apply_resource_diff_error_to_workbench(
    app: &mut AppState,
    request_id: u64,
    resource: &ResourceRef,
    error: &str,
) {
    if let Some(tab) = app
        .workbench
        .find_tab_mut(&WorkbenchTabKey::ResourceDiff(resource.clone()))
        && let WorkbenchTabState::ResourceDiff(diff_tab) = &mut tab.state
        && diff_tab.pending_request_id == Some(request_id)
    {
        diff_tab.set_error(error.to_string());
    }
}

pub(crate) fn apply_rollout_inspection_result_to_workbench(
    app: &mut AppState,
    request_id: u64,
    resource: &ResourceRef,
    inspection: RolloutInspection,
) {
    if let Some(tab) = app
        .workbench
        .find_tab_mut(&WorkbenchTabKey::Rollout(resource.clone()))
        && let WorkbenchTabState::Rollout(rollout_tab) = &mut tab.state
        && rollout_tab.pending_request_id == Some(request_id)
    {
        rollout_tab.apply_inspection(inspection);
    }
}

pub(crate) fn apply_rollout_inspection_error_to_workbench(
    app: &mut AppState,
    request_id: u64,
    resource: &ResourceRef,
    error: &str,
) {
    if let Some(tab) = app
        .workbench
        .find_tab_mut(&WorkbenchTabKey::Rollout(resource.clone()))
        && let WorkbenchTabState::Rollout(rollout_tab) = &mut tab.state
        && rollout_tab.pending_request_id == Some(request_id)
    {
        rollout_tab.set_error(error.to_string());
    }
}

pub(crate) fn apply_helm_history_result_to_workbench(
    app: &mut AppState,
    request_id: u64,
    resource: &ResourceRef,
    history: HelmHistoryResult,
) {
    if let Some(tab) = app
        .workbench
        .find_tab_mut(&WorkbenchTabKey::HelmHistory(resource.clone()))
        && let WorkbenchTabState::HelmHistory(history_tab) = &mut tab.state
        && history_tab.pending_history_request_id == Some(request_id)
    {
        history_tab.apply_history(history);
    }
}

pub(crate) fn apply_helm_history_error_to_workbench(
    app: &mut AppState,
    request_id: u64,
    resource: &ResourceRef,
    error: &str,
) {
    if let Some(tab) = app
        .workbench
        .find_tab_mut(&WorkbenchTabKey::HelmHistory(resource.clone()))
        && let WorkbenchTabState::HelmHistory(history_tab) = &mut tab.state
        && history_tab.pending_history_request_id == Some(request_id)
    {
        history_tab.set_history_error(error.to_string());
    }
}

pub(crate) fn apply_helm_values_diff_result_to_workbench(
    app: &mut AppState,
    request_id: u64,
    resource: &ResourceRef,
    diff: YamlDocumentDiffResult,
) {
    if let Some(tab) = app
        .workbench
        .find_tab_mut(&WorkbenchTabKey::HelmHistory(resource.clone()))
        && let WorkbenchTabState::HelmHistory(history_tab) = &mut tab.state
        && let Some(diff_state) = history_tab.diff.as_mut()
        && diff_state.pending_request_id == Some(request_id)
    {
        diff_state.apply_result(diff);
    }
}

pub(crate) fn apply_helm_values_diff_error_to_workbench(
    app: &mut AppState,
    request_id: u64,
    resource: &ResourceRef,
    error: &str,
) {
    if let Some(tab) = app
        .workbench
        .find_tab_mut(&WorkbenchTabKey::HelmHistory(resource.clone()))
        && let WorkbenchTabState::HelmHistory(history_tab) = &mut tab.state
        && let Some(diff_state) = history_tab.diff.as_mut()
        && diff_state.pending_request_id == Some(request_id)
    {
        diff_state.set_error(error.to_string());
    }
}

pub(crate) fn workbench_follow_streams_to_stop(
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

pub(crate) fn workbench_workload_log_sessions_to_stop(
    app: &AppState,
    action: AppAction,
) -> Vec<u64> {
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

pub(crate) fn workbench_exec_sessions_to_stop(app: &AppState, action: AppAction) -> Vec<u64> {
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

pub(crate) fn refresh_port_forward_workbench(
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

pub(crate) fn apply_mutation_success(
    app: &mut AppState,
    runtime: &mut MutationRuntime<'_>,
    origin_view: AppView,
    message: impl Into<String>,
    force_include_flux: bool,
    delays_secs: &[u64],
) {
    let plan = finish_mutation_success(
        app,
        runtime.status_message_clear_at,
        origin_view,
        message,
        force_include_flux,
    );
    request_refresh(
        runtime.refresh_tx,
        runtime.global_state,
        runtime.client,
        plan.namespace.clone(),
        plan.dispatch,
        runtime.refresh_state,
        runtime.snapshot_dirty,
    );
    queue_deferred_refreshes(
        runtime.deferred_refresh_tx,
        runtime.refresh_state.context_generation,
        plan.origin_view,
        plan.namespace,
        plan.dispatch,
        delays_secs,
    );
    runtime.auto_refresh.reset();
}

pub(crate) fn is_transient_transport_error(err: &anyhow::Error) -> bool {
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

pub(crate) async fn fetch_namespaces_with_startup_retry(client: &K8sClient) -> Result<Vec<String>> {
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

pub(crate) fn spawn_refresh_task(
    refresh_tx: tokio::sync::mpsc::Sender<RefreshAsyncResult>,
    mut global_state: GlobalState,
    client: K8sClient,
    request: RefreshTaskRequest,
) -> tokio::task::JoinHandle<()> {
    let requested_namespace = request.namespace.clone();
    let start_flux_target_fingerprints = global_state.flux_target_fingerprints();
    tokio::spawn(async move {
        let result = global_state
            .refresh_view_with_options(
                &client,
                request.namespace.as_deref(),
                request.options,
                request.target_view,
            )
            .await
            .map(|_| global_state)
            .map_err(|err| err.to_string());
        let _ = refresh_tx
            .send(RefreshAsyncResult {
                request_id: request.request_id,
                context_generation: request.context_generation,
                requested_namespace,
                start_flux_target_fingerprints,
                result,
            })
            .await;
    })
}

#[derive(Debug, Clone)]
pub(crate) struct RefreshTaskRequest {
    pub namespace: Option<String>,
    pub options: RefreshOptions,
    pub target_view: Option<AppView>,
    pub request_id: u64,
    pub context_generation: u64,
}

pub(crate) fn abort_in_flight_refresh(refresh_state: &mut RefreshRuntimeState) {
    if let Some(task) = refresh_state.in_flight_task.take() {
        task.abort();
    }
    refresh_state.in_flight_id = None;
    refresh_state.in_flight_options = None;
    refresh_state.in_flight_namespace = None;
    refresh_state.in_flight_target_view = None;
}

pub(crate) fn refresh_scope_pending(
    refresh_state: &RefreshRuntimeState,
    scope: RefreshScope,
) -> bool {
    refresh_state
        .in_flight_options
        .is_some_and(|options| options.scope.intersects(scope))
        || refresh_state
            .queued_refresh
            .as_ref()
            .is_some_and(|queued| queued.options.scope.intersects(scope))
}

pub(crate) fn request_refresh(
    refresh_tx: &tokio::sync::mpsc::Sender<RefreshAsyncResult>,
    global_state: &mut GlobalState,
    client: &K8sClient,
    namespace: Option<String>,
    dispatch: RefreshDispatch,
    refresh_state: &mut RefreshRuntimeState,
    snapshot_dirty: &mut bool,
) {
    if dispatch.options.scope.is_empty() {
        return;
    }

    let options = dispatch.options;
    let primary_scope = dispatch.primary_scope.intersection(options.scope);
    let immediate_scope = if primary_scope.is_empty() {
        options.scope
    } else {
        primary_scope
    };
    let background_scope = options.scope.without(immediate_scope);
    let target_view = dispatch.target_view;

    let core_options = RefreshOptions {
        scope: immediate_scope,
        include_cluster_info: options.include_cluster_info
            && immediate_scope.contains(RefreshScope::METRICS),
        skip_core: false,
    };

    let visible_options = RefreshOptions {
        scope: immediate_scope.union(background_scope),
        include_cluster_info: false,
        skip_core: false,
    };
    let immediate_target_view = scoped_target_view(core_options.scope, target_view);
    let background_target_view = scoped_target_view(background_scope, target_view);
    if let Some(view) = dispatch.target_view {
        global_state.mark_view_refresh_requested(view);
    } else {
        global_state.mark_refresh_requested(visible_options);
    }
    *snapshot_dirty = true;

    refresh_state.request_seq = refresh_state.request_seq.wrapping_add(1);
    let request_id = refresh_state.request_seq;

    let can_preempt_in_flight_secondary = immediate_target_view.is_some()
        && refresh_state
            .in_flight_options
            .is_some_and(|in_flight| in_flight.skip_core)
        && refresh_state.in_flight_target_view.is_none();

    if can_preempt_in_flight_secondary {
        let background_refresh = QueuedRefresh {
            request_id: refresh_state.request_seq.wrapping_add(1),
            namespace: refresh_state.in_flight_namespace.clone(),
            primary_scope: RefreshScope::NONE,
            options: refresh_state.in_flight_options.unwrap_or_default(),
            target_view: None,
            context_generation: refresh_state.context_generation,
        };
        abort_in_flight_refresh(refresh_state);
        refresh_state.queued_refresh = Some(match refresh_state.queued_refresh.take() {
            Some(existing) => QueuedRefresh {
                request_id: existing.request_id,
                namespace: existing.namespace.or(background_refresh.namespace),
                primary_scope: existing
                    .primary_scope
                    .union(background_refresh.primary_scope),
                options: RefreshOptions {
                    scope: existing
                        .options
                        .scope
                        .union(background_refresh.options.scope),
                    include_cluster_info: existing.options.include_cluster_info
                        || background_refresh.options.include_cluster_info,
                    skip_core: existing.options.skip_core && background_refresh.options.skip_core,
                },
                target_view: None,
                context_generation: existing.context_generation,
            },
            None => background_refresh,
        });
    }

    if refresh_state.in_flight_id.is_none() {
        let queued_namespace = namespace.clone();
        refresh_state.in_flight_id = Some(request_id);
        refresh_state.in_flight_options = Some(core_options);
        refresh_state.in_flight_namespace = namespace.clone();
        refresh_state.in_flight_target_view = immediate_target_view;
        refresh_state.in_flight_task = Some(spawn_refresh_task(
            refresh_tx.clone(),
            global_state.clone(),
            client.clone(),
            RefreshTaskRequest {
                namespace,
                options: core_options,
                target_view: immediate_target_view,
                request_id,
                context_generation: refresh_state.context_generation,
            },
        ));
        if !background_scope.is_empty() {
            refresh_state.request_seq = refresh_state.request_seq.wrapping_add(1);
            refresh_state.queued_refresh = Some(QueuedRefresh {
                request_id: refresh_state.request_seq,
                namespace: queued_namespace,
                primary_scope: RefreshScope::NONE,
                options: RefreshOptions {
                    scope: background_scope,
                    include_cluster_info: options.include_cluster_info,
                    skip_core: true,
                },
                target_view: background_target_view,
                context_generation: refresh_state.context_generation,
            });
        }
    } else {
        // Merge into the already-queued request.
        let merged_scope = refresh_state
            .queued_refresh
            .as_ref()
            .map_or(RefreshScope::NONE, |queued| queued.options.scope)
            .union(options.scope)
            .union(background_scope);
        let merged_primary_scope = refresh_state
            .queued_refresh
            .as_ref()
            .map_or(RefreshScope::NONE, |queued| queued.primary_scope)
            .union(dispatch.primary_scope);
        let merged_skip_core = refresh_state
            .queued_refresh
            .as_ref()
            .is_some_and(|queued| queued.options.skip_core)
            && merged_primary_scope.is_empty();
        let existing_target_view = refresh_state
            .queued_refresh
            .as_ref()
            .and_then(|queued| scoped_target_view(merged_scope, queued.target_view));
        let incoming_target_view = scoped_target_view(merged_scope, dispatch.target_view);
        let merged_target_view = match (existing_target_view, incoming_target_view) {
            (Some(existing), Some(next)) if existing == next => Some(existing),
            (Some(existing), None) => Some(existing),
            (None, Some(next)) => Some(next),
            _ => None,
        };
        refresh_state.queued_refresh = Some(QueuedRefresh {
            request_id,
            namespace,
            primary_scope: merged_primary_scope,
            options: RefreshOptions {
                scope: merged_scope,
                include_cluster_info: refresh_state
                    .queued_refresh
                    .as_ref()
                    .is_some_and(|queued| queued.options.include_cluster_info)
                    || options.include_cluster_info,
                skip_core: merged_skip_core,
            },
            target_view: merged_target_view,
            context_generation: refresh_state.context_generation,
        });
    }
}

fn scoped_target_view(scope: RefreshScope, target_view: Option<AppView>) -> Option<AppView> {
    let view = target_view?;
    (scope == GlobalState::view_ready_scope(view)).then_some(view)
}

pub(crate) fn queued_refresh_requires_two_phase(
    primary_scope: RefreshScope,
    options: RefreshOptions,
) -> bool {
    !primary_scope.is_empty()
        && !options.skip_core
        && !options.scope.without(primary_scope).is_empty()
}

pub(crate) fn normalize_recent_events(
    mut events: Vec<kubectui::k8s::dtos::K8sEventInfo>,
) -> Vec<kubectui::k8s::dtos::K8sEventInfo> {
    events.sort_unstable_by_key(|event| std::cmp::Reverse(event.last_seen));
    events.truncate(MAX_RECENT_EVENTS_CACHE_ITEMS);
    events
}

pub(crate) fn abort_in_flight_events_fetch(events_state: &mut EventsFetchRuntimeState) {
    if let Some(task) = events_state.in_flight_task.take() {
        task.abort();
    }
    events_state.in_flight_id = None;
    events_state.in_flight_namespace = None;
}

pub(crate) fn spawn_events_fetch_task(
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

pub(crate) fn request_events_refresh(
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
