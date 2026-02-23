//! User interface composition and rendering utilities.

pub mod components;
pub mod views;

use ratatui::{
    layout::{Constraint, Direction, Layout},
    prelude::Frame,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::{
    app::{AppState, AppView},
    state::ClusterSnapshot,
};

/// Renders a full frame for the current app and cluster state.
pub fn render(frame: &mut Frame, app: &AppState, cluster: &ClusterSnapshot) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(frame.area());

    components::render_header(
        frame,
        layout[0],
        "KubecTUI v0.1.0",
        cluster.cluster_summary(),
    );
    components::render_tabs(frame, layout[1], AppView::tabs(), app.view());

    match app.view() {
        AppView::Dashboard => views::dashboard::render_dashboard(frame, layout[2], cluster),
        AppView::Nodes => views::nodes::render_nodes(
            frame,
            layout[2],
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::Pods => {
            let body = render_pods(cluster, app.selected_idx());
            frame.render_widget(body, layout[2]);
        }
        AppView::Services => views::services::render_services(
            frame,
            layout[2],
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::Deployments => views::deployments::render_deployments(
            frame,
            layout[2],
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
    }

    let status = if let Some(err) = app.error_message() {
        format!("ERROR: {err}")
    } else if app.is_search_mode() {
        format!("Search: {}", app.search_query())
    } else {
        "[Tab] switch view • [/] search • [Enter] detail • [r] refresh • [q] quit".to_string()
    };

    components::render_status_bar(frame, layout[3], &status, app.error_message().is_some());

    if let Some(detail_state) = app.detail_view.as_ref() {
        views::detail::render_detail(frame, frame.area(), detail_state);
    }
}

fn render_pods(cluster: &ClusterSnapshot, selected_idx: usize) -> Paragraph<'static> {
    let mut lines = Vec::new();

    for (idx, pod) in cluster.pods.iter().take(50).enumerate() {
        let marker = if idx == selected_idx { ">" } else { " " };
        lines.push(Line::from(Span::raw(format!(
            "{marker} {}/{} | status={} | node={} | restarts={}",
            pod.namespace,
            pod.name,
            pod.status,
            pod.node.as_deref().unwrap_or("n/a"),
            pod.restarts
        ))));
    }

    if lines.is_empty() {
        lines.push(Line::from("No pods found"));
    }

    Paragraph::new(lines).block(Block::default().title("Pods").borders(Borders::ALL))
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use crate::{
        app::{AppState, AppView, DetailMetadata, DetailViewState, ResourceRef},
        k8s::dtos::{DeploymentInfo, NodeInfo, PodInfo, ServiceInfo},
        state::ClusterSnapshot,
    };

    use super::*;

    fn draw(app: &AppState, snapshot: &ClusterSnapshot) {
        let backend = TestBackend::new(120, 40);
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
            service_type: "ClusterIP".to_string(),
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

    /// Verifies services view renders without panic for mixed service types.
    #[test]
    fn render_services_mixed_types_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        for t in ["ClusterIP", "NodePort", "LoadBalancer", "ExternalName"] {
            snapshot.services.push(ServiceInfo {
                name: format!("svc-{t}"),
                namespace: "default".to_string(),
                type_: t.to_string(),
                service_type: t.to_string(),
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
}
