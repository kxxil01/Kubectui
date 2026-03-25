//! KubecTUI entry point.
//!
//! This module wires terminal lifecycle management, the application state machine,
//! the Kubernetes client, and the ratatui rendering pipeline.

#![cfg_attr(test, allow(clippy::field_reassign_with_default))]

mod action;
mod async_types;
mod detail_fetch;
mod event_handlers;
mod flux_reconcile;
mod mutation_helpers;
mod runtime_helpers;
mod selection_helpers;
mod startup;
mod terminal;

use async_types::*;
use detail_fetch::*;
use event_handlers::*;
use flux_reconcile::*;
use mutation_helpers::*;
use runtime_helpers::{next_request_id, run_app, start_watch_manager};
use selection_helpers::*;
use std::{
    collections::HashMap,
    io,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream, KeyCode};
use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use ratatui::{Terminal, backend::CrosstermBackend};

use kubectui::ui::components::port_forward_dialog::PortForwardDialog;
use kubectui::{
    action_history::{ActionKind, ActionStatus},
    app::{
        AppAction, AppView, DetailViewState, LogsViewerState, ResourceRef, load_config, save_config,
    },
    coordinator::{UpdateCoordinator, UpdateMessage},
    events::apply_action,
    k8s::{
        client::K8sClient,
        exec::{ExecEvent, ExecSessionHandle, fetch_pod_containers, spawn_exec_session},
        logs::{LogsClient, PodRef},
        portforward::PortForwarderService,
        probes::extract_probes_from_pod,
        workload_logs::{MAX_WORKLOAD_LOG_STREAMS, resolve_workload_log_targets},
    },
    policy::DetailAction,
    secret::{decode_secret_yaml, encode_secret_yaml},
    state::{
        DataPhase, GlobalState, RefreshScope,
        watch::{WatchUpdate, WatchedResource},
    },
    ui,
    workbench::{WorkbenchTabKey, WorkbenchTabState},
};

use terminal::{pick_context_at_startup, restore_terminal, setup_terminal};

/// Main asynchronous runtime entrypoint.
#[tokio::main]
async fn main() -> Result<()> {
    if startup::initialize_process()? {
        return Ok(());
    }

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

/// Runs KubecTUI's event loop.
pub(crate) async fn run_app_inner(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<()> {
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
    let (resource_diff_tx, mut resource_diff_rx) =
        tokio::sync::mpsc::channel::<ResourceDiffAsyncResult>(16);
    let mut resource_diff_request_seq: u64 = 0;
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
    let (cronjob_suspend_tx, mut cronjob_suspend_rx) =
        tokio::sync::mpsc::channel::<SetCronJobSuspendAsyncResult>(16);
    let (node_ops_tx, mut node_ops_rx) = tokio::sync::mpsc::channel::<NodeOpsAsyncResult>(16);
    let mut node_op_in_flight: bool = false;
    let (probe_tx, mut probe_rx) = tokio::sync::mpsc::channel::<ProbeAsyncResult>(16);
    let (relations_tx, mut relations_rx) = tokio::sync::mpsc::channel::<RelationsAsyncResult>(16);
    let mut relations_request_seq: u64 = 0;
    let (exec_bootstrap_tx, mut exec_bootstrap_rx) =
        tokio::sync::mpsc::channel::<ExecBootstrapResult>(16);
    let (debug_dialog_bootstrap_tx, mut debug_dialog_bootstrap_rx) =
        tokio::sync::mpsc::channel::<DebugContainerDialogBootstrapResult>(16);
    let mut debug_dialog_request_seq: u64 = 0;
    let (debug_launch_tx, mut debug_launch_rx) =
        tokio::sync::mpsc::channel::<DebugContainerLaunchAsyncResult>(16);
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

    // Watch-backed resource caches — pushes live updates for core resources.
    let (watch_tx, mut watch_rx) = tokio::sync::mpsc::channel::<WatchUpdate>(32);
    let mut watch_manager =
        start_watch_manager(&client, refresh_state.context_generation, &app, &watch_tx).await;

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

            result = resource_diff_rx.recv() => {
                if let Some(result) = result {
                    let ResourceDiffAsyncResult {
                        request_id,
                        resource,
                        result,
                    } = result;
                    let workbench_waiting_for_this = app.workbench.tabs.iter().any(|tab| {
                        matches!(
                            &tab.state,
                            WorkbenchTabState::ResourceDiff(diff_tab)
                                if diff_tab.resource == resource
                                    && diff_tab.loading
                                    && diff_tab.pending_request_id == Some(request_id)
                        )
                    });
                    if !workbench_waiting_for_this {
                        continue;
                    }

                    needs_redraw = true;
                    match result {
                        Ok(live_yaml) => match kubectui::resource_diff::build_resource_diff(&live_yaml)
                        {
                            Ok(diff) => apply_resource_diff_result_to_workbench(
                                &mut app,
                                request_id,
                                &resource,
                                diff,
                            ),
                            Err(err) => apply_resource_diff_error_to_workbench(
                                &mut app,
                                request_id,
                                &resource,
                                &err.to_string(),
                            ),
                        },
                        Err(err) => apply_resource_diff_error_to_workbench(
                            &mut app,
                            request_id,
                            &resource,
                            &err,
                        ),
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
                                    &mut |a, msg| set_transient_status(a, &mut status_message_clear_at, msg),
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
                        if queued_refresh_requires_two_phase(queued.primary_scope, queued.options) {
                            request_refresh(
                                &refresh_tx,
                                &mut global_state,
                                &client,
                                queued.namespace,
                                RefreshDispatch {
                                    primary_scope: queued.primary_scope,
                                    options: queued.options,
                                },
                                &mut refresh_state,
                                &mut snapshot_dirty,
                            );
                        } else {
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

            // Watch-backed resource updates — live cluster changes via watch streams.
            Some(update) = watch_rx.recv() => {
                if update.context_generation == refresh_state.context_generation {
                    let watched_resource = update.resource;
                    global_state.apply_watch_update(update);
                    snapshot_dirty = true;
                    if watched_resource == WatchedResource::Namespaces {
                        needs_redraw = true;
                        app.set_available_namespaces(global_state.namespaces().to_vec());
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
                            apply_mutation_success(
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
                            apply_mutation_success(
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
                            apply_mutation_success(
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
                            apply_mutation_success(
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
                            apply_mutation_success(
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

            result = cronjob_suspend_rx.recv() => {
                if let Some(result) = result {
                    if result.context_generation != refresh_state.context_generation {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            format!(
                                "{} cancelled because the active context changed.",
                                if result.suspend { "Suspend" } else { "Resume" }
                            ),
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
                                format!(
                                    "{} requested for {}.",
                                    if result.suspend { "Suspend" } else { "Resume" },
                                    result.resource_label
                                ),
                                true,
                            );
                            apply_mutation_success(
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
                                    "{} requested for {}. Refreshing view...",
                                    if result.suspend { "Suspend" } else { "Resume" },
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
                                format!(
                                    "{} failed: {err}",
                                    if result.suspend { "Suspend" } else { "Resume" }
                                ),
                                true,
                            );
                            status_message_clear_at = None;
                            app.set_error(format!(
                                "{} failed: {err}",
                                if result.suspend { "Suspend" } else { "Resume" }
                            ));
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
                            apply_mutation_success(
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

            result = debug_dialog_bootstrap_rx.recv() => {
                if let Some(result) = result {
                    needs_redraw = true;
                    if let Some(detail) = app.detail_view.as_mut()
                        && detail.resource.as_ref() == Some(&result.resource)
                        && let Some(dialog) = detail.debug_dialog.as_mut()
                        && dialog.pending_request_id == Some(result.request_id)
                    {
                        match result.result {
                            Ok(containers) => dialog.set_target_containers(containers),
                            Err(err) => dialog.set_target_fetch_error(err),
                        }
                    }
                }
            }

            result = debug_launch_rx.recv() => {
                if let Some(result) = result {
                    needs_redraw = true;
                    if result.context_generation != refresh_state.context_generation {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            "Debug container launch was cancelled because the active context changed.",
                            true,
                        );
                        continue;
                    }

                    match result.result {
                        Ok(launch) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Succeeded,
                                format!(
                                    "Started debug container '{}' in Pod '{}'.",
                                    launch.container_name, launch.pod_name
                                ),
                                true,
                            );
                            apply_mutation_success(
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
                                    "Started debug container '{}' in Pod '{}'. Refreshing view...",
                                    launch.container_name, launch.pod_name
                                ),
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                            app.detail_view = None;
                            if let Some(existing_session_id) =
                                app.workbench().exec_session_id(&result.resource)
                                && let Some(handle) = exec_sessions.remove(&existing_session_id)
                            {
                                let _ = handle.cancel_tx.send(());
                            }
                            app.open_exec_tab_for_container(
                                result.resource.clone(),
                                result.session_id,
                                launch.pod_name.clone(),
                                launch.namespace.clone(),
                                launch.container_name.clone(),
                            );
                            match spawn_exec_session(
                                client.clone(),
                                result.session_id,
                                launch.pod_name,
                                launch.namespace,
                                launch.container_name.clone(),
                                exec_update_tx.clone(),
                            )
                            .await
                            {
                                Ok(handle) => {
                                    exec_sessions.insert(result.session_id, handle);
                                }
                                Err(err) => {
                                    if let Some(tab) = app
                                        .workbench_mut()
                                        .find_tab_mut(&WorkbenchTabKey::Exec(result.resource))
                                        && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
                                    {
                                        exec_tab.loading = false;
                                        exec_tab.error = Some(format!(
                                            "Debug container launched, but shell attach failed: {err:#}"
                                        ));
                                    }
                                    app.set_error(format!(
                                        "Debug container launched, but shell attach failed: {err:#}"
                                    ));
                                }
                            }
                        }
                        Err(err) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!("Debug container launch failed: {err}"),
                                true,
                            );
                            if let Some(detail) = app.detail_view.as_mut()
                                && detail.resource.as_ref() == Some(&result.resource)
                                && let Some(dialog) = detail.debug_dialog.as_mut()
                            {
                                dialog.set_pending_launch(false);
                                dialog.error_message = Some(err);
                            } else {
                                status_message_clear_at = None;
                                app.set_error(format!("Debug container launch failed: {err}"));
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

                            watch_manager.stop_all();

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
                            // Restart watches for the new context.
                            watch_manager = start_watch_manager(
                                &client,
                                refresh_state.context_generation,
                                &app,
                                &watch_tx,
                            )
                            .await;
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
                        trigger.dispatch,
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
                    let mut dispatch = refresh_options_for_view(app.view(), include_flux, false);
                    // Strip watched scopes — watches provide real-time updates
                    dispatch.primary_scope = dispatch.primary_scope.without(RefreshScope::WATCHED_SCOPES);
                    dispatch.options.scope = dispatch.options.scope.without(RefreshScope::WATCHED_SCOPES);
                    if !dispatch.options.scope.is_empty() || dispatch.options.include_cluster_info {
                        request_refresh(
                            &refresh_tx,
                            &mut global_state,
                            &client,
                            namespace_scope(app.get_namespace()).map(str::to_string),
                            dispatch,
                            &mut refresh_state,
                            &mut snapshot_dirty,
                        );
                    }
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
                    if let Some(message) = detail_action_block_message(
                        &app,
                        &client,
                        &reconcile_resource,
                        DetailAction::FluxReconcile,
                    )
                    .await
                    {
                        app.set_error(message);
                        continue;
                    }

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
                    let resource_ctx = if let Some(resource_ctx) = app
                        .detail_view
                        .as_ref()
                        .and_then(|d| d.resource_action_context())
                    {
                        Some(resource_ctx)
                    } else if let Some(mut resource_ctx) =
                        selected_resource_context(&app, &cached_snapshot)
                    {
                        resource_ctx.action_authorizations = client
                            .fetch_detail_action_authorizations(&resource_ctx.resource)
                            .await;
                        Some(resource_ctx)
                    } else {
                        None
                    };
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
                    app.navigate_to_view(view);
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
                    app.search_query.clear();
                    app.is_search_mode = false;
                    app.detail_view = None;
                    app.workbench.close_resource_tabs();
                    app.sync_workbench_focus();
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
                    watch_manager.stop_all();
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
                    app.sync_workbench_focus();

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
                    // Restart watches for the new namespace.
                    watch_manager = start_watch_manager(
                        &client,
                        refresh_state.context_generation,
                        &app,
                        &watch_tx,
                    )
                    .await;
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
                    if action::detail_tabs::handle_open_resource_yaml(
                        &mut app,
                        &client,
                        &cached_snapshot,
                        &detail_tx,
                        &mut detail_request_seq,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::OpenResourceDiff => {
                    if action::detail_tabs::handle_open_resource_diff(
                        &mut app,
                        &client,
                        &cached_snapshot,
                        &resource_diff_tx,
                        &mut resource_diff_request_seq,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::OpenDecodedSecret => {
                    if action::detail_tabs::handle_open_decoded_secret(
                        &mut app,
                        &client,
                        &cached_snapshot,
                        &detail_tx,
                        &mut detail_request_seq,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::ToggleBookmark => {
                    if action::detail_tabs::handle_toggle_bookmark(&mut app, &cached_snapshot) {
                        continue;
                    }
                }
                AppAction::OpenRelationships => {
                    if action::detail_tabs::handle_open_relationships(
                        &mut app,
                        &cached_snapshot,
                        &client,
                        &relations_tx,
                        &mut relations_request_seq,
                    ) {
                        continue;
                    }
                }
                AppAction::OpenResourceEvents => {
                    if action::detail_tabs::handle_open_resource_events(
                        &mut app,
                        &client,
                        &cached_snapshot,
                        &detail_tx,
                        &mut detail_request_seq,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::LogsViewerOpen => {
                    let resource = app
                        .detail_view
                        .as_ref()
                        .and_then(DetailViewState::selected_logs_resource)
                        .or_else(|| {
                            app.detail_view
                                .as_ref()
                                .and_then(|detail| detail.resource.clone())
                        })
                        .or_else(|| selected_resource(&app, &cached_snapshot));
                    let Some(resource) = resource else {
                        app.set_error("No resource selected for logs.".to_string());
                        continue;
                    };
                    if let Some(message) =
                        detail_action_block_message(&app, &client, &resource, DetailAction::Logs)
                            .await
                    {
                        app.set_error(message);
                        continue;
                    }
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
                    let resource = ResourceRef::Pod(pod_name.clone(), pod_ns.clone());
                    if let Some(message) =
                        detail_action_block_message(&app, &client, &resource, DetailAction::Exec)
                            .await
                    {
                        app.set_error(message);
                        continue;
                    }

                    if let Some(existing_session_id) = app.workbench().exec_session_id(&resource)
                        && let Some(handle) = exec_sessions.remove(&existing_session_id)
                    {
                        let _ = handle.cancel_tx.send(());
                    }

                    let session_id = next_exec_session_id;
                    next_exec_session_id = next_exec_session_id.wrapping_add(1).max(1);
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
                AppAction::DebugContainerDialogOpen => {
                    if action::debug::handle_debug_container_dialog_open(
                        &mut app,
                        &client,
                        &debug_dialog_bootstrap_tx,
                        &mut debug_dialog_request_seq,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::DebugContainerDialogSubmit => {
                    if action::debug::handle_debug_container_dialog_submit(
                        &mut app,
                        &client,
                        &debug_launch_tx,
                        &mut next_exec_session_id,
                        refresh_state.context_generation,
                    )
                    .await
                    {
                        continue;
                    }
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
                    let Some(resource) = resource else {
                        continue;
                    };
                    if let Some(message) = detail_action_block_message(
                        &app,
                        &client,
                        &resource,
                        DetailAction::PortForward,
                    )
                    .await
                    {
                        app.set_error(message);
                        continue;
                    }
                    app.detail_view = None;
                    app.open_port_forward_tab(Some(resource), dialog);
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
                    if action::scale::handle_scale_dialog_submit(
                        &mut app,
                        &client,
                        &scale_tx,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    )
                    .await
                    {
                        continue;
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
                        if let Some(message) = detail_action_block_message(
                            &app,
                            &client,
                            &resource,
                            DetailAction::Restart,
                        )
                        .await
                        {
                            app.set_error(message);
                            continue;
                        }
                        let resource_label =
                            format!("{} '{}' in namespace '{}'", kind, name, namespace);
                        let origin_view = app.view();
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
                    if action::delete::handle_delete_resource(
                        &mut app,
                        &client,
                        &delete_tx,
                        &mut delete_request_seq,
                        &mut delete_in_flight_id,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::ForceDeleteResource => {
                    if action::delete::handle_force_delete_resource(
                        &mut app,
                        &client,
                        &delete_tx,
                        &mut delete_request_seq,
                        &mut delete_in_flight_id,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::TriggerCronJob => {
                    if action::cronjob::handle_trigger_cronjob(
                        &mut app,
                        &client,
                        &trigger_cronjob_tx,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::ConfirmCronJobSuspend(suspend) => {
                    action::cronjob::handle_confirm_cronjob_suspend(&mut app, suspend);
                }
                AppAction::SetCronJobSuspend(suspend) => {
                    if action::cronjob::handle_set_cronjob_suspend(
                        &mut app,
                        &client,
                        &cronjob_suspend_tx,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                        suspend,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::ConfirmDrainNode => {
                    action::node_ops::handle_confirm_drain_node(&mut app);
                }
                AppAction::CordonNode if !node_op_in_flight => {
                    if action::node_ops::handle_cordon_node(
                        &mut app,
                        &client,
                        &node_ops_tx,
                        &mut node_op_in_flight,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::CordonNode => {} // in-flight guard
                AppAction::UncordonNode if !node_op_in_flight => {
                    if action::node_ops::handle_uncordon_node(
                        &mut app,
                        &client,
                        &node_ops_tx,
                        &mut node_op_in_flight,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::UncordonNode => {} // in-flight guard
                AppAction::DrainNode | AppAction::ForceDrainNode if !node_op_in_flight => {
                    let force = matches!(action, AppAction::ForceDrainNode);
                    if action::node_ops::handle_drain_node(
                        &mut app,
                        &client,
                        &node_ops_tx,
                        &mut node_op_in_flight,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                        force,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::DrainNode | AppAction::ForceDrainNode => {
                    action::node_ops::handle_drain_in_flight_guard(&mut app);
                }
                AppAction::CopyResourceName => {
                    action::copy_export::copy_resource_name(&mut app, &cached_snapshot);
                }
                AppAction::CopyResourceFullName => {
                    action::copy_export::copy_resource_full_name(&mut app, &cached_snapshot);
                }
                AppAction::CopyLogContent => {
                    action::copy_export::copy_log_content(&mut app);
                }
                AppAction::ExportLogs => {
                    action::copy_export::export_logs(&mut app);
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
                                r.clone(),
                                r.kind().to_ascii_lowercase(),
                                r.name().to_string(),
                                r.namespace().map(str::to_owned),
                                y.clone(),
                            )
                        })
                    });

                    if let Some((resource, kind, name, namespace, yaml_content)) = edit_info {
                        if let Some(message) = detail_action_block_message(
                            &app,
                            &client,
                            &resource,
                            DetailAction::EditYaml,
                        )
                        .await
                        {
                            app.set_error(message);
                            continue;
                        }
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
                                                        apply_mutation_success(
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
                    if let Some(message) = detail_action_block_message(
                        &app,
                        &client,
                        &resource,
                        DetailAction::EditYaml,
                    )
                    .await
                    {
                        app.set_error(message.clone());
                        app.complete_action_history(
                            action_history_id,
                            ActionStatus::Failed,
                            message,
                            true,
                        );
                        continue;
                    }
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
                            apply_mutation_success(
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
                    if action::scale::handle_scale_dialog_open(&mut app, &cached_snapshot) {
                        continue;
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
                        if let Some(message) = detail_action_block_message(
                            &app,
                            &client,
                            &resource,
                            DetailAction::Probes,
                        )
                        .await
                        {
                            app.set_error(message);
                            continue;
                        }
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
                AppAction::CycleIconMode => {
                    apply_action(AppAction::CycleIconMode, &mut app);
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

#[cfg(test)]
mod main_tests;
