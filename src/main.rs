//! KubecTUI entry point.
//!
//! This module wires terminal lifecycle management, the application state machine,
//! the Kubernetes client, and the ratatui rendering pipeline.

#![cfg_attr(test, allow(clippy::field_reassign_with_default))]

use std::{io, time::Duration};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use ratatui::{Terminal, backend::CrosstermBackend};

use kubectui::{
    app::{
        AppAction, AppState, AppView, DetailMetadata, DetailViewState, LogsViewerState, ResourceRef,
        load_config, save_config,
    },
    coordinator::{LogStreamStatus, UpdateCoordinator, UpdateMessage},
    events::apply_action,
    k8s::{
        client::K8sClient,
        logs::{LogsClient, PodRef},
        portforward::PortForwarderService,
        probes::extract_probes_from_pod,
    },
    state::{ClusterSnapshot, GlobalState},
    ui,
};

/// Main asynchronous runtime entrypoint.
#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let mut terminal = setup_terminal().context("failed to initialize terminal")?;
    let run_result = run_app(&mut terminal).await;
    let restore_result = restore_terminal(&mut terminal);

    if let Err(err) = restore_result {
        eprintln!("failed to restore terminal state: {err:#}");
    }

    run_result
}

/// Applies coordinator update messages to app state.
fn apply_coordinator_msg(msg: UpdateMessage, app: &mut AppState) {
    match msg {
        UpdateMessage::LogUpdate { pod_name, line, .. } => {
            if let Some(detail) = &mut app.detail_view
                && let Some(viewer) = &mut detail.logs_viewer
                    && viewer.pod_name == pod_name {
                        viewer.lines.push(line);
                        if viewer.follow_mode {
                            viewer.scroll_offset = viewer.lines.len().saturating_sub(1);
                        }
                    }
        }
        UpdateMessage::ProbeUpdate { pod_name, namespace, probes } => {
            if let Some(detail) = &mut app.detail_view
                && let Some(panel) = &mut detail.probe_panel
                    && panel.pod_name == pod_name && panel.namespace == namespace {
                        panel.update_probes(probes);
                    }
        }
        UpdateMessage::LogStreamStatus { pod_name, status, .. } => {
            if let Some(detail) = &mut app.detail_view
                && let Some(viewer) = &mut detail.logs_viewer
                    && viewer.pod_name == pod_name {
                        match status {
                            LogStreamStatus::Error(err) => {
                                viewer.error = Some(err);
                                viewer.loading = false;
                            }
                            LogStreamStatus::Ended | LogStreamStatus::Cancelled => {
                                viewer.follow_mode = false;
                            }
                            LogStreamStatus::Started => {}
                        }
                    }
        }
        UpdateMessage::ProbeError { pod_name, namespace, error } => {
            if let Some(detail) = &mut app.detail_view
                && let Some(panel) = &mut detail.probe_panel
                    && panel.pod_name == pod_name && panel.namespace == namespace {
                        panel.error = Some(error);
                    }
        }
    }
}

/// Runs KubecTUI's event loop.
async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = load_config();

    let mut client = pick_context_at_startup(terminal, &mut app).await?;

    let mut global_state = GlobalState::default();

    if let Err(err) = global_state
        .refresh(&client, namespace_scope(app.get_namespace()))
        .await
    {
        app.set_error(format!("Initial data refresh failed: {err:#}"));
    }
    app.set_available_namespaces(global_state.namespaces().to_vec());
    sync_extensions_instances(&client, &mut app, &global_state.snapshot()).await;

    let port_forwarder = PortForwarderService::new(std::sync::Arc::new(client.clone()));

    let (update_tx, mut update_rx) = tokio::sync::mpsc::unbounded_channel::<UpdateMessage>();
    let coordinator = UpdateCoordinator::new(client.clone(), update_tx);

    // Channel for async detail view fetches — keeps the UI responsive while YAML/events load
    let (detail_tx, mut detail_rx) =
        tokio::sync::mpsc::unbounded_channel::<Result<DetailViewState, (ResourceRef, String)>>();

    // Cached snapshot — only re-clone when state is marked dirty
    let mut cached_snapshot = global_state.snapshot();
    let mut snapshot_dirty = false;

    let mut tick = tokio::time::interval(Duration::from_millis(200));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut event_stream = EventStream::new();

    loop {
        // Re-clone snapshot only when something changed
        if snapshot_dirty {
            cached_snapshot = global_state.snapshot();
            snapshot_dirty = false;
        }

        terminal
            .draw(|frame| ui::render(frame, &app, &cached_snapshot))
            .context("failed to render frame")?;

        if app.should_quit() {
            break;
        }

        // Wait concurrently on: tick, input event, coordinator update, or detail fetch result.
        // `biased` ensures coordinator messages and detail results are drained before blocking on input.
        tokio::select! {
            biased;

            // Coordinator updates (log lines, probe updates) — highest priority
            msg = update_rx.recv() => {
                if let Some(msg) = msg {
                    apply_coordinator_msg(msg, &mut app);
                }
                // Drain any additional queued messages without blocking
                while let Ok(msg) = update_rx.try_recv() {
                    apply_coordinator_msg(msg, &mut app);
                }
            }

            // Detail view fetch completed in background task
            result = detail_rx.recv() => {
                if let Some(result) = result {
                    match result {
                        Ok(state) => app.detail_view = Some(state),
                        Err((resource, err)) => {
                            app.detail_view = Some(DetailViewState {
                                resource: Some(resource),
                                loading: false,
                                error: Some(err),
                                ..DetailViewState::default()
                            });
                        }
                    }
                }
            }

            // Periodic tick — just a heartbeat to keep rendering at ~200ms when idle
            _ = tick.tick() => {}

            // Keyboard / terminal input — lowest priority so messages are drained first
            maybe_event = event_stream.next() => {
                let Some(Ok(Event::Key(key))) = maybe_event else { continue; };

                let action = if key.code == KeyCode::Enter
                    && !app.is_search_mode()
                    && !app.is_namespace_picker_open()
                    && !app.is_context_picker_open()
                    && !app.command_palette.is_open()
                {
                    if app.detail_view.is_some() {
                        selected_resource(&app, &cached_snapshot)
                            .map(AppAction::OpenDetail)
                            .unwrap_or(AppAction::None)
                    } else if app.focus == kubectui::app::Focus::Content
                        && app.view() == AppView::Extensions
                        && !app.extension_in_instances
                    {
                        // Drill into instances pane
                        if !app.extension_instances.is_empty() {
                            app.extension_in_instances = true;
                            app.extension_instance_cursor = 0;
                        }
                        AppAction::None
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

                match action {
                    AppAction::None => {
                        // No-op — don't call sync_extensions_instances on every unrecognized key
                    }
                    AppAction::Quit => break,
                    AppAction::RefreshData => {
                        if let Err(err) = global_state
                            .refresh(&client, namespace_scope(app.get_namespace()))
                            .await
                        {
                            app.set_error(format!("Refresh failed: {err:#}"));
                        } else {
                            app.clear_error();
                            app.set_available_namespaces(global_state.namespaces().to_vec());
                            snapshot_dirty = true;
                            sync_extensions_instances(&client, &mut app, &global_state.snapshot())
                                .await;
                        }
                    }
                    AppAction::OpenNamespacePicker => {
                        app.set_available_namespaces(global_state.namespaces().to_vec());
                        app.open_namespace_picker();
                    }
                    AppAction::CloseNamespacePicker => {
                        app.close_namespace_picker();
                    }
                    AppAction::OpenCommandPalette => {
                        app.command_palette.open();
                    }
                    AppAction::CloseCommandPalette => {
                        app.command_palette.close();
                    }
                    AppAction::NavigateTo(view) => {
                        app.command_palette.close();
                        app.view = view;
                        app.selected_idx = 0;
                        app.focus = kubectui::app::Focus::Content;
                        app.extension_in_instances = false;
                        // Trigger extensions sync when navigating to Extensions view
                        if view == kubectui::app::AppView::Extensions {
                            sync_extensions_instances(&client, &mut app, &cached_snapshot).await;
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
                        match K8sClient::connect_with_context(&ctx).await {
                            Ok(new_client) => {
                                global_state = GlobalState::default();
                                if let Err(err) = global_state
                                    .refresh(&new_client, namespace_scope(app.get_namespace()))
                                    .await
                                {
                                    app.set_error(format!("Refresh failed after context switch: {err:#}"));
                                } else {
                                    app.clear_error();
                                    app.set_available_namespaces(global_state.namespaces().to_vec());
                                    snapshot_dirty = true;
                                    sync_extensions_instances(&new_client, &mut app, &global_state.snapshot()).await;
                                }
                                client = new_client;
                            }
                            Err(err) => {
                                app.set_error(format!("Failed to connect to context '{ctx}': {err:#}"));
                            }
                        }
                    }
                    AppAction::SelectNamespace(namespace) => {
                        app.set_namespace(namespace);
                        app.close_namespace_picker();
                        save_config(&app);

                        if let Err(err) = global_state
                            .refresh(&client, namespace_scope(app.get_namespace()))
                            .await
                        {
                            app.set_error(format!("Refresh failed: {err:#}"));
                        } else {
                            app.clear_error();
                            app.set_available_namespaces(global_state.namespaces().to_vec());
                            snapshot_dirty = true;
                            sync_extensions_instances(&client, &mut app, &global_state.snapshot())
                                .await;
                        }
                    }
                    AppAction::OpenDetail(resource) => {
                        // Show loading state immediately — fetch happens in a background task
                        app.detail_view = Some(initial_loading_state(resource.clone(), &cached_snapshot));
                        let client_clone = client.clone();
                        let snapshot_clone = cached_snapshot.clone();
                        let tx = detail_tx.clone();
                        tokio::spawn(async move {
                            match fetch_detail_view(&client_clone, &snapshot_clone, resource.clone()).await {
                                Ok(state) => { let _ = tx.send(Ok(state)); }
                                Err(err) => { let _ = tx.send(Err((resource, err.to_string()))); }
                            }
                        });
                    }
                    AppAction::CloseDetail => {
                        if let Some(detail) = &app.detail_view
                            && let Some(viewer) = &detail.logs_viewer
                                && viewer.follow_mode && !viewer.pod_name.is_empty() {
                                    let _ = coordinator
                                        .stop_log_streaming(
                                            &viewer.pod_name,
                                            &viewer.pod_namespace,
                                            "default",
                                        )
                                        .await;
                                }
                        app.detail_view = None;
                    }
                    AppAction::LogsViewerOpen => {
                        if let Some(detail) = &mut app.detail_view {
                            let (pod_name, pod_ns) = detail
                                .resource
                                .as_ref()
                                .and_then(|r| match r {
                                    ResourceRef::Pod(name, ns) => Some((name.clone(), ns.clone())),
                                    _ => None,
                                })
                                .unwrap_or_default();

                            // Fetch container list for this pod
                            let containers: Vec<String> = {
                                let pods_api: Api<Pod> = Api::namespaced(client.get_client(), &pod_ns);
                                pods_api.get(&pod_name).await.ok()
                                    .and_then(|pod| pod.spec)
                                    .map(|spec| spec.containers.iter().map(|c| c.name.clone()).collect())
                                    .unwrap_or_default()
                            };

                            let picking = containers.len() > 1;
                            let first_container = containers.first().cloned().unwrap_or_default();

                            detail.logs_viewer = Some(LogsViewerState {
                                pod_name: pod_name.clone(),
                                pod_namespace: pod_ns.clone(),
                                container_name: if picking { String::new() } else { first_container.clone() },
                                containers: containers.clone(),
                                picking_container: picking,
                                container_cursor: 0,
                                loading: !picking,
                                ..Default::default()
                            });

                            // If single container, fetch logs immediately
                            if !picking && !pod_name.is_empty() {
                                let logs_client = LogsClient::new(client.get_client());
                                let pod_ref = PodRef::new(pod_name, pod_ns);
                                let cname = if first_container.is_empty() { None } else { Some(first_container.as_str()) };
                                match logs_client.tail_logs(&pod_ref, Some(500), cname).await {
                                    Ok(lines) => {
                                        if let Some(detail) = &mut app.detail_view
                                            && let Some(viewer) = &mut detail.logs_viewer {
                                                viewer.lines = lines;
                                                viewer.loading = false;
                                            }
                                    }
                                    Err(err) => {
                                        if let Some(detail) = &mut app.detail_view
                                            && let Some(viewer) = &mut detail.logs_viewer {
                                                viewer.loading = false;
                                                viewer.error = Some(err.to_string());
                                            }
                                    }
                                }
                            }
                        }
                    }
                    AppAction::LogsViewerSelectContainer(container) => {
                        // User picked a container from the picker — fetch its logs
                        let (pod_name, pod_ns) = app.detail_view.as_ref()
                            .and_then(|d| d.logs_viewer.as_ref())
                            .map(|v| (v.pod_name.clone(), v.pod_namespace.clone()))
                            .unwrap_or_default();

                        if let Some(detail) = &mut app.detail_view
                            && let Some(viewer) = &mut detail.logs_viewer {
                                viewer.picking_container = false;
                                viewer.container_name = container.clone();
                                viewer.loading = true;
                                viewer.lines.clear();
                                viewer.error = None;
                            }

                        if !pod_name.is_empty() {
                            let logs_client = LogsClient::new(client.get_client());
                            let pod_ref = PodRef::new(pod_name, pod_ns);
                            match logs_client.tail_logs(&pod_ref, Some(500), Some(container.as_str())).await {
                                Ok(lines) => {
                                    if let Some(detail) = &mut app.detail_view
                                        && let Some(viewer) = &mut detail.logs_viewer {
                                            viewer.lines = lines;
                                            viewer.loading = false;
                                        }
                                }
                                Err(err) => {
                                    if let Some(detail) = &mut app.detail_view
                                        && let Some(viewer) = &mut detail.logs_viewer {
                                            viewer.loading = false;
                                            viewer.error = Some(err.to_string());
                                        }
                                }
                            }
                        }
                    }
                    AppAction::LogsViewerToggleFollow => {
                        let follow_info = app.detail_view.as_ref().and_then(|d| {
                            d.logs_viewer.as_ref().map(|v| {
                                (v.pod_name.clone(), v.pod_namespace.clone(), v.container_name.clone(), v.follow_mode)
                            })
                        });
                        apply_action(AppAction::LogsViewerToggleFollow, &mut app);
                        if let Some((pod_name, pod_ns, container_name, was_following)) = follow_info
                            && !pod_name.is_empty() {
                                let cname = if container_name.is_empty() { "default".to_string() } else { container_name };
                                if !was_following {
                                    let _ = coordinator
                                        .start_log_streaming(pod_name, pod_ns, cname, true)
                                        .await;
                                } else {
                                    let _ = coordinator
                                        .stop_log_streaming(&pod_name, &pod_ns, &cname)
                                        .await;
                                }
                            }
                    }
                    AppAction::ScaleDialogSubmit => {
                        let scale_info = app.detail_view.as_ref().and_then(|d| {
                            d.scale_dialog.as_ref().and_then(|s| {
                                s.desired_replicas_as_int().map(|replicas| {
                                    (s.deployment_name.clone(), s.namespace.clone(), replicas)
                                })
                            })
                        });
                        if let Some((name, namespace, replicas)) = scale_info {
                            match client.scale_deployment(&name, &namespace, replicas).await {
                                Ok(()) => {
                                    app.clear_error();
                                    if let Some(detail) = &mut app.detail_view {
                                        detail.scale_dialog = None;
                                    }
                                }
                                Err(err) => {
                                    if let Some(detail) = &mut app.detail_view
                                        && let Some(scale) = &mut detail.scale_dialog {
                                            scale.error_message = Some(format!("Scale failed: {err:#}"));
                                        }
                                }
                            }
                        }
                    }
                    AppAction::RolloutRestart => {
                        let restart_info = app.detail_view.as_ref().and_then(|d| {
                            d.resource.as_ref().and_then(|r| match r {
                                ResourceRef::Deployment(name, ns) => Some(("deployment".to_string(), name.clone(), ns.clone())),
                                ResourceRef::StatefulSet(name, ns) => Some(("statefulset".to_string(), name.clone(), ns.clone())),
                                ResourceRef::DaemonSet(name, ns) => Some(("daemonset".to_string(), name.clone(), ns.clone())),
                                _ => None,
                            })
                        });
                        if let Some((kind, name, namespace)) = restart_info {
                            match client.rollout_restart(&kind, &name, &namespace).await {
                                Ok(()) => {
                                    app.clear_error();
                                    if let Some(detail) = &mut app.detail_view {
                                        detail.error = None;
                                    }
                                }
                                Err(err) => {
                                    app.set_error(format!("Restart failed: {err:#}"));
                                }
                            }
                        }
                    }
                    AppAction::DeleteResource => {
                        let delete_info = app.detail_view.as_ref().and_then(|d| {
                            d.resource.clone()
                        });
                        if let Some(resource) = delete_info {
                            let result = match &resource {
                                ResourceRef::CustomResource { name, namespace, group, version, kind, plural } => {
                                    client.delete_custom_resource(
                                        group, version, kind, plural, name, namespace.as_deref(),
                                    ).await
                                }
                                _ => {
                                    client.delete_resource(
                                        &resource.kind().to_ascii_lowercase(),
                                        resource.name(),
                                        resource.namespace(),
                                    ).await
                                }
                            };
                            match result {
                                Ok(()) => {
                                    app.clear_error();
                                    app.detail_view = None;
                                    // Refresh data to reflect the deletion
                                    if let Err(err) = global_state
                                        .refresh(&client, namespace_scope(app.get_namespace()))
                                        .await
                                    {
                                        app.set_error(format!("Refresh failed: {err:#}"));
                                    } else {
                                        app.set_available_namespaces(global_state.namespaces().to_vec());
                                        snapshot_dirty = true;
                                    }
                                }
                                Err(err) => {
                                    app.set_error(format!("Delete failed: {err:#}"));
                                    if let Some(detail) = &mut app.detail_view {
                                        detail.confirm_delete = false;
                                    }
                                }
                            }
                        }
                    }
                    AppAction::EditYaml => {
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
                            // Write YAML to a temp file
                            let tmp_path = std::env::temp_dir()
                                .join(format!("kubectui-{kind}-{name}.yaml"));
                            if let Err(err) = std::fs::write(&tmp_path, &yaml_content) {
                                app.set_error(format!("Failed to write temp file: {err}"));
                            } else {
                                // Suspend TUI — restore terminal to canonical mode
                                let _ = restore_terminal(terminal);

                                // Spawn $EDITOR (fallback: vi)
                                let editor = std::env::var("EDITOR")
                                    .or_else(|_| std::env::var("VISUAL"))
                                    .unwrap_or_else(|_| "vi".to_string());

                                let status = std::process::Command::new(&editor)
                                    .arg(&tmp_path)
                                    .status();

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
                                        app.set_error(format!("Failed to launch editor '{editor}': {err}"));
                                    }
                                    Ok(exit) if !exit.success() => {
                                        // Editor exited non-zero (e.g. :cq in vim) — treat as cancel
                                    }
                                    Ok(_) => {
                                        // Read back the edited file
                                        match std::fs::read_to_string(&tmp_path) {
                                            Err(err) => {
                                                app.set_error(format!("Failed to read edited file: {err}"));
                                            }
                                            Ok(edited_yaml) => {
                                                if edited_yaml.trim() == yaml_content.trim() {
                                                    // No changes — skip apply
                                                } else {
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
                                                            app.clear_error();
                                                            // Reload the detail view with fresh YAML
                                                            if let Some(detail) = &mut app.detail_view {
                                                                detail.yaml = Some(edited_yaml);
                                                                detail.yaml_scroll = 0;
                                                            }
                                                        }
                                                        Err(err) => {
                                                            app.set_error(format!("Apply failed: {err:#}"));
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
                    AppAction::PortForwardCreate((target, config)) => {
                        match port_forwarder.create_tunnel_async(target, config).await {
                            Ok(tunnel_id) => {
                                app.clear_error();
                                if let Some(detail) = &mut app.detail_view
                                    && let Some(dialog) = &mut detail.port_forward_dialog {
                                        dialog.success = Some(format!("Tunnel created: {tunnel_id}"));
                                        let tunnels = port_forwarder.list_tunnels();
                                        let mut registry = kubectui::state::port_forward::TunnelRegistry::new();
                                        registry.update_tunnels(tunnels);
                                        dialog.update_registry(registry);
                                    }
                            }
                            Err(err) => {
                                if let Some(detail) = &mut app.detail_view
                                    && let Some(dialog) = &mut detail.port_forward_dialog {
                                        dialog.error = Some(format!("{err}"));
                                    }
                            }
                        }
                    }
                    AppAction::ScaleDialogOpen => {
                        // Read actual replica count from snapshot before opening dialog
                        let scale_info = app.detail_view.as_ref().and_then(|d| {
                            d.resource.as_ref().and_then(|r| match r {
                                ResourceRef::Deployment(name, ns) => {
                                    let replicas = cached_snapshot.deployments.iter()
                                        .find(|d| &d.name == name && &d.namespace == ns)
                                        .map(|d| d.desired_replicas)
                                        .unwrap_or(1);
                                    Some((name.clone(), ns.clone(), replicas))
                                }
                                ResourceRef::StatefulSet(name, ns) => {
                                    let replicas = cached_snapshot.statefulsets.iter()
                                        .find(|ss| &ss.name == name && &ss.namespace == ns)
                                        .map(|ss| ss.desired_replicas)
                                        .unwrap_or(1);
                                    Some((name.clone(), ns.clone(), replicas))
                                }
                                _ => None,
                            })
                        });
                        if let Some((name, namespace, replicas)) = scale_info
                            && let Some(detail) = &mut app.detail_view {
                                detail.scale_dialog = Some(kubectui::ui::components::scale_dialog::ScaleDialogState::new(name, namespace, replicas));
                            }
                    }
                    AppAction::ProbePanelOpen => {
                        let pod_info = app.detail_view.as_ref().and_then(|d| {
                            d.resource.as_ref().and_then(|r| match r {
                                ResourceRef::Pod(name, ns) => Some((name.clone(), ns.clone())),
                                _ => None,
                            })
                        });
                        if let Some((pod_name, pod_ns)) = pod_info {
                            let pods_api: Api<Pod> = Api::namespaced(client.get_client(), &pod_ns);
                            let container_probes = match pods_api.get(&pod_name).await {
                                Ok(pod) => extract_probes_from_pod(&pod).unwrap_or_default(),
                                Err(_) => Vec::new(),
                            };
                            if let Some(detail) = &mut app.detail_view {
                                use kubectui::ui::components::probe_panel::ProbePanelState;
                                detail.probe_panel = Some(ProbePanelState::new(
                                    pod_name,
                                    pod_ns,
                                    container_probes,
                                ));
                            }
                        } else {
                            apply_action(AppAction::ProbePanelOpen, &mut app);
                        }
                    }
                    other => {
                        apply_action(other, &mut app);
                    }
                }
            }
        }
    }

    Ok(())
}

async fn sync_extensions_instances(
    client: &K8sClient,
    app: &mut AppState,
    snapshot: &ClusterSnapshot,
) {
    if app.view() != AppView::Extensions {
        return;
    }

    let Some(crd) = snapshot.custom_resource_definitions.get(
        app.selected_idx()
            .min(snapshot.custom_resource_definitions.len().saturating_sub(1)),
    ) else {
        app.extension_instances.clear();
        app.extension_error = None;
        app.extension_selected_crd = None;
        return;
    };

    if app.extension_selected_crd.as_deref() == Some(crd.name.as_str()) {
        return;
    }

    let namespace_owned = if crd.scope.eq_ignore_ascii_case("Namespaced") {
        namespace_scope(app.get_namespace()).map(ToString::to_string)
    } else {
        None
    };

    match client
        .fetch_custom_resources(crd, namespace_owned.as_deref())
        .await
    {
        Ok(items) => app.set_extension_instances(crd.name.clone(), items, None),
        Err(err) => {
            app.set_extension_instances(crd.name.clone(), Vec::new(), Some(err.to_string()))
        }
    }
}

fn namespace_scope(namespace: &str) -> Option<&str> {
    if namespace == "all" {
        None
    } else {
        Some(namespace)
    }
}

/// Filters a slice by a text query matching name and optional namespace fields,
/// then returns the item at the given index in the filtered result.
/// Case-insensitive substring match without allocating a new lowercase string.
#[inline]
fn contains_ci(haystack: &str, needle: &str) -> bool {
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

fn filtered_get<'a, T, F>(items: &'a [T], idx: usize, query: &str, text_fields: F) -> Option<&'a T>
where
    F: Fn(&T, &str) -> bool,
{
    if query.is_empty() {
        return items.get(idx);
    }
    items
        .iter()
        .filter(|item| text_fields(item, query))
        .nth(idx)
}

fn selected_resource(app: &AppState, snapshot: &ClusterSnapshot) -> Option<ResourceRef> {
    let idx = app.selected_idx();
    let q = app.search_query();
    match app.view() {
        AppView::Dashboard => None,
        AppView::Nodes => filtered_get(&snapshot.nodes, idx, q, |n, q| contains_ci(&n.name, q) || contains_ci(&n.role, q))
            .map(|n| ResourceRef::Node(n.name.clone())),
        AppView::Pods => filtered_get(&snapshot.pods, idx, q, |p, q| contains_ci(&p.name, q) || contains_ci(&p.namespace, q) || contains_ci(&p.status, q))
            .map(|p| ResourceRef::Pod(p.name.clone(), p.namespace.clone())),
        AppView::Services => filtered_get(&snapshot.services, idx, q, |s, q| contains_ci(&s.name, q) || contains_ci(&s.namespace, q) || contains_ci(&s.type_, q))
            .map(|s| ResourceRef::Service(s.name.clone(), s.namespace.clone())),
        AppView::Deployments => filtered_get(&snapshot.deployments, idx, q, |d, q| contains_ci(&d.name, q) || contains_ci(&d.namespace, q))
            .map(|d| ResourceRef::Deployment(d.name.clone(), d.namespace.clone())),
        AppView::StatefulSets => filtered_get(&snapshot.statefulsets, idx, q, |ss, q| contains_ci(&ss.name, q) || contains_ci(&ss.namespace, q))
            .map(|ss| ResourceRef::StatefulSet(ss.name.clone(), ss.namespace.clone())),
        AppView::ResourceQuotas => filtered_get(&snapshot.resource_quotas, idx, q, |rq, q| contains_ci(&rq.name, q) || contains_ci(&rq.namespace, q))
            .map(|rq| ResourceRef::ResourceQuota(rq.name.clone(), rq.namespace.clone())),
        AppView::LimitRanges => filtered_get(&snapshot.limit_ranges, idx, q, |lr, q| contains_ci(&lr.name, q) || contains_ci(&lr.namespace, q))
            .map(|lr| ResourceRef::LimitRange(lr.name.clone(), lr.namespace.clone())),
        AppView::PodDisruptionBudgets => filtered_get(&snapshot.pod_disruption_budgets, idx, q, |pdb, q| contains_ci(&pdb.name, q) || contains_ci(&pdb.namespace, q))
            .map(|pdb| ResourceRef::PodDisruptionBudget(pdb.name.clone(), pdb.namespace.clone())),
        AppView::DaemonSets => filtered_get(&snapshot.daemonsets, idx, q, |ds, q| contains_ci(&ds.name, q) || contains_ci(&ds.namespace, q))
            .map(|ds| ResourceRef::DaemonSet(ds.name.clone(), ds.namespace.clone())),
        AppView::ReplicaSets => filtered_get(&snapshot.replicasets, idx, q, |rs, q| contains_ci(&rs.name, q) || contains_ci(&rs.namespace, q))
            .map(|rs| ResourceRef::ReplicaSet(rs.name.clone(), rs.namespace.clone())),
        AppView::ReplicationControllers => filtered_get(&snapshot.replication_controllers, idx, q, |rc, q| contains_ci(&rc.name, q) || contains_ci(&rc.namespace, q))
            .map(|rc| ResourceRef::ReplicationController(rc.name.clone(), rc.namespace.clone())),
        AppView::Jobs => filtered_get(&snapshot.jobs, idx, q, |j, q| contains_ci(&j.name, q) || contains_ci(&j.namespace, q) || contains_ci(&j.status, q))
            .map(|j| ResourceRef::Job(j.name.clone(), j.namespace.clone())),
        AppView::CronJobs => filtered_get(&snapshot.cronjobs, idx, q, |cj, q| contains_ci(&cj.name, q) || contains_ci(&cj.namespace, q) || contains_ci(&cj.schedule, q))
            .map(|cj| ResourceRef::CronJob(cj.name.clone(), cj.namespace.clone())),
        AppView::Endpoints => filtered_get(&snapshot.endpoints, idx, q, |e, q| contains_ci(&e.name, q) || contains_ci(&e.namespace, q))
            .map(|e| ResourceRef::Endpoint(e.name.clone(), e.namespace.clone())),
        AppView::Ingresses => filtered_get(&snapshot.ingresses, idx, q, |i, q| contains_ci(&i.name, q) || contains_ci(&i.namespace, q))
            .map(|i| ResourceRef::Ingress(i.name.clone(), i.namespace.clone())),
        AppView::IngressClasses => filtered_get(&snapshot.ingress_classes, idx, q, |ic, q| contains_ci(&ic.name, q))
            .map(|ic| ResourceRef::IngressClass(ic.name.clone())),
        AppView::NetworkPolicies => filtered_get(&snapshot.network_policies, idx, q, |np, q| contains_ci(&np.name, q) || contains_ci(&np.namespace, q))
            .map(|np| ResourceRef::NetworkPolicy(np.name.clone(), np.namespace.clone())),
        AppView::ConfigMaps => filtered_get(&snapshot.config_maps, idx, q, |cm, q| contains_ci(&cm.name, q) || contains_ci(&cm.namespace, q))
            .map(|cm| ResourceRef::ConfigMap(cm.name.clone(), cm.namespace.clone())),
        AppView::Secrets => filtered_get(&snapshot.secrets, idx, q, |s, q| contains_ci(&s.name, q) || contains_ci(&s.namespace, q) || contains_ci(&s.type_, q))
            .map(|s| ResourceRef::Secret(s.name.clone(), s.namespace.clone())),
        AppView::HPAs => filtered_get(&snapshot.hpas, idx, q, |h, q| contains_ci(&h.name, q) || contains_ci(&h.namespace, q))
            .map(|h| ResourceRef::Hpa(h.name.clone(), h.namespace.clone())),
        AppView::PriorityClasses => filtered_get(&snapshot.priority_classes, idx, q, |pc, q| contains_ci(&pc.name, q))
            .map(|pc| ResourceRef::PriorityClass(pc.name.clone())),
        AppView::PersistentVolumeClaims => filtered_get(&snapshot.pvcs, idx, q, |pvc, q| contains_ci(&pvc.name, q) || contains_ci(&pvc.namespace, q))
            .map(|pvc| ResourceRef::Pvc(pvc.name.clone(), pvc.namespace.clone())),
        AppView::PersistentVolumes => filtered_get(&snapshot.pvs, idx, q, |pv, q| contains_ci(&pv.name, q))
            .map(|pv| ResourceRef::Pv(pv.name.clone())),
        AppView::StorageClasses => filtered_get(&snapshot.storage_classes, idx, q, |sc, q| contains_ci(&sc.name, q))
            .map(|sc| ResourceRef::StorageClass(sc.name.clone())),
        AppView::Namespaces => filtered_get(&snapshot.namespace_list, idx, q, |ns, q| contains_ci(&ns.name, q) || contains_ci(&ns.status, q))
            .map(|ns| ResourceRef::Namespace(ns.name.clone())),
        AppView::Events => filtered_get(&snapshot.events, idx, q, |ev, q| contains_ci(&ev.name, q) || contains_ci(&ev.namespace, q) || contains_ci(&ev.reason, q))
            .map(|ev| ResourceRef::Event(ev.name.clone(), ev.namespace.clone())),
        AppView::ServiceAccounts => filtered_get(&snapshot.service_accounts, idx, q, |sa, q| contains_ci(&sa.name, q) || contains_ci(&sa.namespace, q))
            .map(|sa| ResourceRef::ServiceAccount(sa.name.clone(), sa.namespace.clone())),
        AppView::Roles => filtered_get(&snapshot.roles, idx, q, |r, q| contains_ci(&r.name, q) || contains_ci(&r.namespace, q))
            .map(|r| ResourceRef::Role(r.name.clone(), r.namespace.clone())),
        AppView::RoleBindings => filtered_get(&snapshot.role_bindings, idx, q, |rb, q| contains_ci(&rb.name, q) || contains_ci(&rb.namespace, q))
            .map(|rb| ResourceRef::RoleBinding(rb.name.clone(), rb.namespace.clone())),
        AppView::ClusterRoles => filtered_get(&snapshot.cluster_roles, idx, q, |cr, q| contains_ci(&cr.name, q))
            .map(|cr| ResourceRef::ClusterRole(cr.name.clone())),
        AppView::ClusterRoleBindings => filtered_get(&snapshot.cluster_role_bindings, idx, q, |crb, q| contains_ci(&crb.name, q))
            .map(|crb| ResourceRef::ClusterRoleBinding(crb.name.clone())),
        AppView::HelmCharts => None, // Helm repos are local config, no detail view
        AppView::Extensions => {
            // The Extensions view has a split pane: CRDs (left) and instances (right).
            // When extension_in_instances is true, Enter opens the selected instance's detail.
            if !app.extension_in_instances {
                return None;
            }
            let crd = app.extension_selected_crd.as_ref().and_then(|crd_name| {
                snapshot.custom_resource_definitions.iter().find(|c| &c.name == crd_name)
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
        AppView::HelmReleases => filtered_get(&snapshot.helm_releases, idx, q, |r, q| contains_ci(&r.name, q) || contains_ci(&r.namespace, q) || contains_ci(&r.chart, q))
            .map(|r| ResourceRef::HelmRelease(r.name.clone(), r.namespace.clone())),
    }
}

fn initial_loading_state(resource: ResourceRef, snapshot: &ClusterSnapshot) -> DetailViewState {
    let metadata = metadata_for_resource(snapshot, &resource);
    DetailViewState {
        resource: Some(resource),
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
    let kind = resource.kind().to_ascii_lowercase();
    let name = resource.name().to_string();
    let namespace = resource.namespace().map(str::to_owned);

    let yaml = match &resource {
        ResourceRef::CustomResource { group, version, kind, plural, name, namespace } => {
            client
                .fetch_custom_resource_yaml(group, version, kind, plural, name, namespace.as_deref())
                .await
                .ok()
        }
        ResourceRef::HelmRelease(name, ns) => {
            // Helm releases are stored as Secrets — fetch the latest revision secret
            client
                .fetch_helm_release_yaml(name, ns)
                .await
                .ok()
        }
        _ => {
            client
                .fetch_resource_yaml(&kind, &name, namespace.as_deref())
                .await
                .ok()
        }
    };

    let events = match &resource {
        ResourceRef::Pod(name, ns) => client.fetch_pod_events(name, ns).await.unwrap_or_default(),
        // Fetch events for namespaced workload and config resources
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
            client.fetch_resource_events(kind, name, ns).await.unwrap_or_default()
        }
        _ => Vec::new(),
    };

    let (pod_metrics, node_metrics, metrics_unavailable_message) = match &resource {
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
    };

    let sections = sections_for_resource(snapshot, &resource);

    Ok(DetailViewState {
        resource: Some(resource),
        metadata,
        yaml,
        yaml_scroll: 0,
        events,
        sections,
        pod_metrics,
        node_metrics,
        metrics_unavailable_message,
        loading: false,
        error: None,
        logs_viewer: None,
        port_forward_dialog: None,
        scale_dialog: None,
        probe_panel: None,
        confirm_delete: false,
    })
}

fn metadata_for_resource(snapshot: &ClusterSnapshot, resource: &ResourceRef) -> DetailMetadata {
    match resource {
        ResourceRef::Node(name) => {
            if let Some(node) = snapshot.nodes.iter().find(|n| &n.name == name) {
                DetailMetadata {
                    name: node.name.clone(),
                    namespace: None,
                    status: Some(if node.ready { "Ready" } else { "NotReady" }.to_string()),
                    node: Some(node.name.clone()),
                    ip: None,
                    created: None,
                    labels: Vec::new(),
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
                .map(|cj| if cj.suspend { "Suspended".to_string() } else { "Active".to_string() });
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
        ResourceRef::CustomResource { name, namespace, kind, group, .. } => {
            DetailMetadata {
                name: name.clone(),
                namespace: namespace.clone(),
                status: Some(format!("{kind}.{group}")),
                ..DetailMetadata::default()
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
                    format!("type: {}", svc.service_type),
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
        ResourceRef::CustomResource { kind, group, version, .. } => {
            vec![
                "CUSTOM RESOURCE".to_string(),
                format!("kind: {kind}"),
                format!("apiVersion: {group}/{version}"),
            ]
        }
    }
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
            && let Event::Key(key) = event::read().context("failed to read event")? {
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
