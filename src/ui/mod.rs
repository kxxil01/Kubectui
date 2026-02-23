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
