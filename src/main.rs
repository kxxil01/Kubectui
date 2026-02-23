//! KubecTUI entry point.
//!
//! This module wires terminal lifecycle management, the application state machine,
//! the Kubernetes client, and the ratatui rendering pipeline.

use std::{io, time::Duration};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use kubectui::{
    app::{
        AppAction, AppState, AppView, DetailMetadata, DetailViewState, ResourceRef, load_config,
        save_config,
    },
    events::apply_action,
    k8s::client::K8sClient,
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

    let mut tick = tokio::time::interval(Duration::from_millis(200));

    loop {
        let snapshot = global_state.snapshot();
        terminal
            .draw(|frame| ui::render(frame, &app, &snapshot))
            .context("failed to render frame")?;

        if app.should_quit() {
            break;
        }

        if event::poll(Duration::from_millis(1)).context("failed to poll terminal events")?
            && let Event::Key(key) = event::read().context("failed to read terminal event")?
        {
            let action = if key.code == KeyCode::Enter
                && !app.is_search_mode()
                && !app.is_namespace_picker_open()
                && !app.is_context_picker_open()
            {
                selected_resource(&app, &snapshot)
                    .map(AppAction::OpenDetail)
                    .unwrap_or(AppAction::None)
            } else {
                app.handle_key_event(key)
            };

            match action {
                AppAction::None => {
                    sync_extensions_instances(&client, &mut app, &snapshot).await;
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
                        sync_extensions_instances(&client, &mut app, &global_state.snapshot())
                            .await;
                    }
                }
                AppAction::OpenDetail(resource) => {
                    app.detail_view = Some(initial_loading_state(resource.clone(), &snapshot));
                    match fetch_detail_view(&client, &snapshot, resource.clone()).await {
                        Ok(state) => app.detail_view = Some(state),
                        Err(err) => {
                            app.detail_view = Some(DetailViewState {
                                resource: Some(resource),
                                loading: false,
                                error: Some(err.to_string()),
                                ..DetailViewState::default()
                            })
                        }
                    }
                }
                AppAction::CloseDetail => {
                    app.detail_view = None;
                }
                // Apply all other component actions
                other => {
                    apply_action(other, &mut app);
                }
            }
        }

        tick.tick().await;
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

fn selected_resource(app: &AppState, snapshot: &ClusterSnapshot) -> Option<ResourceRef> {
    let idx = app.selected_idx();
    match app.view() {
        AppView::Dashboard => None,
        AppView::Nodes => snapshot
            .nodes
            .get(idx)
            .map(|n| ResourceRef::Node(n.name.clone())),
        AppView::Pods => snapshot
            .pods
            .get(idx)
            .map(|p| ResourceRef::Pod(p.name.clone(), p.namespace.clone())),
        AppView::Services => snapshot
            .services
            .get(idx)
            .map(|s| ResourceRef::Service(s.name.clone(), s.namespace.clone())),
        AppView::Deployments => snapshot
            .deployments
            .get(idx)
            .map(|d| ResourceRef::Deployment(d.name.clone(), d.namespace.clone())),
        AppView::StatefulSets => snapshot
            .statefulsets
            .get(idx)
            .map(|ss| ResourceRef::StatefulSet(ss.name.clone(), ss.namespace.clone())),
        AppView::ResourceQuotas => snapshot
            .resource_quotas
            .get(idx)
            .map(|rq| ResourceRef::ResourceQuota(rq.name.clone(), rq.namespace.clone())),
        AppView::LimitRanges => snapshot
            .limit_ranges
            .get(idx)
            .map(|lr| ResourceRef::LimitRange(lr.name.clone(), lr.namespace.clone())),
        AppView::PodDisruptionBudgets => snapshot
            .pod_disruption_budgets
            .get(idx)
            .map(|pdb| ResourceRef::PodDisruptionBudget(pdb.name.clone(), pdb.namespace.clone())),
        AppView::DaemonSets
        | AppView::Jobs
        | AppView::CronJobs
        | AppView::ServiceAccounts
        | AppView::Roles
        | AppView::RoleBindings
        | AppView::ClusterRoles
        | AppView::ClusterRoleBindings
        | AppView::Extensions => None,
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

    let yaml = client
        .fetch_resource_yaml(&kind, &name, namespace.as_deref())
        .await
        .ok();

    let events = match &resource {
        ResourceRef::Pod(name, ns) => client.fetch_pod_events(name, ns).await.unwrap_or_default(),
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

        if event::poll(Duration::from_millis(16)).context("failed to poll events")? {
            if let Event::Key(key) = event::read().context("failed to read event")? {
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
