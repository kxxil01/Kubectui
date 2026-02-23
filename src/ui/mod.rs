//! User interface composition and rendering utilities.

pub mod components;

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

    let body = match app.view() {
        AppView::Dashboard => render_dashboard(cluster),
        AppView::Nodes => render_nodes(cluster, app.selected_idx()),
        AppView::Pods => render_pods(cluster, app.selected_idx()),
        AppView::Services => render_placeholder("Services", "Phase 1 scaffolding complete"),
        AppView::Deployments => render_placeholder("Deployments", "Phase 1 scaffolding complete"),
    };

    frame.render_widget(body, layout[2]);

    let status = if let Some(err) = app.error_message() {
        format!("ERROR: {err}")
    } else if app.is_search_mode() {
        format!("Search: {}", app.search_query())
    } else {
        "[Tab] switch view • [/] search • [r] refresh • [q] quit".to_string()
    };

    components::render_status_bar(frame, layout[3], &status, app.error_message().is_some());
}

fn render_dashboard(cluster: &ClusterSnapshot) -> Paragraph<'static> {
    let ready_nodes = cluster.nodes.iter().filter(|n| n.ready).count();
    let running_pods = cluster
        .pods
        .iter()
        .filter(|p| p.status.eq_ignore_ascii_case("running"))
        .count();

    let lines = vec![
        Line::from(Span::raw(format!(
            "Nodes: {ready_nodes}/{} Ready",
            cluster.nodes.len()
        ))),
        Line::from(Span::raw(format!(
            "Pods: {running_pods}/{} Running",
            cluster.pods.len()
        ))),
        Line::from(Span::raw(format!("Data phase: {}", cluster.phase))),
        Line::from(Span::raw(format!(
            "Last updated: {}",
            cluster
                .last_updated
                .map(|ts| ts.to_rfc3339())
                .unwrap_or_else(|| "never".to_string())
        ))),
    ];

    Paragraph::new(lines).block(Block::default().title("Dashboard").borders(Borders::ALL))
}

fn render_nodes(cluster: &ClusterSnapshot, selected_idx: usize) -> Paragraph<'static> {
    let mut lines = Vec::new();

    for (idx, node) in cluster.nodes.iter().take(25).enumerate() {
        let marker = if idx == selected_idx { ">" } else { " " };
        lines.push(Line::from(Span::raw(format!(
            "{marker} {} | ready={} | kubelet={} | os={}",
            node.name, node.ready, node.kubelet_version, node.os_image
        ))));
    }

    if lines.is_empty() {
        lines.push(Line::from("No nodes found"));
    }

    Paragraph::new(lines).block(Block::default().title("Nodes").borders(Borders::ALL))
}

fn render_pods(cluster: &ClusterSnapshot, selected_idx: usize) -> Paragraph<'static> {
    let mut lines = Vec::new();

    for (idx, pod) in cluster.pods.iter().take(25).enumerate() {
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

fn render_placeholder(title: &'static str, message: &'static str) -> Paragraph<'static> {
    Paragraph::new(message).block(Block::default().title(title).borders(Borders::ALL))
}
