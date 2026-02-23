//! KubecTUI entry point.
//!
//! This module wires terminal lifecycle management, the application state machine,
//! the Kubernetes client, and the ratatui rendering pipeline.

mod app;
mod k8s;
mod state;
mod ui;

use std::{io, time::Duration};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{
    app::{AppAction, AppState, DetailMetadata, DetailViewState, ResourceRef},
    k8s::client::K8sClient,
    state::{ClusterSnapshot, GlobalState},
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
    let client = K8sClient::connect()
        .await
        .context("unable to initialize Kubernetes client")?;

    let mut app = AppState::default();
    let mut global_state = GlobalState::default();

    if let Err(err) = global_state.refresh(&client).await {
        app.set_error(format!("Initial data refresh failed: {err:#}"));
    }

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
            let action = if key.code == KeyCode::Enter && !app.is_search_mode() {
                selected_resource(&app, &snapshot)
                    .map(AppAction::OpenDetail)
                    .unwrap_or(AppAction::None)
            } else {
                app.handle_key_event(key)
            };

            match action {
                AppAction::None => {}
                AppAction::Quit => break,
                AppAction::RefreshData => {
                    if let Err(err) = global_state.refresh(&client).await {
                        app.set_error(format!("Refresh failed: {err:#}"));
                    } else {
                        app.clear_error();
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
            }
        }

        tick.tick().await;
    }

    Ok(())
}

fn selected_resource(app: &AppState, snapshot: &ClusterSnapshot) -> Option<ResourceRef> {
    let idx = app.selected_idx();
    match app.view() {
        app::AppView::Dashboard => None,
        app::AppView::Nodes => snapshot
            .nodes
            .get(idx)
            .map(|n| ResourceRef::Node(n.name.clone())),
        app::AppView::Pods => snapshot
            .pods
            .get(idx)
            .map(|p| ResourceRef::Pod(p.name.clone(), p.namespace.clone())),
        app::AppView::Services => snapshot
            .services
            .get(idx)
            .map(|s| ResourceRef::Service(s.name.clone(), s.namespace.clone())),
        app::AppView::Deployments => snapshot
            .deployments
            .get(idx)
            .map(|d| ResourceRef::Deployment(d.name.clone(), d.namespace.clone())),
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

    let sections = sections_for_resource(snapshot, &resource);

    Ok(DetailViewState {
        resource: Some(resource),
        metadata,
        yaml,
        events,
        sections,
        loading: false,
        error: None,
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
