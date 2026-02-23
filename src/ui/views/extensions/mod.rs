//! Extensions view: CRD picker + custom resources list.

pub mod crds;
pub mod custom_resources;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::Frame,
};

use crate::{app::AppState, state::ClusterSnapshot};

/// Renders extensions split-pane with CRDs (left) and instances (right).
pub fn render_extensions(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    app: &AppState,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    crds::render_crd_picker(
        frame,
        chunks[0],
        &snapshot.custom_resource_definitions,
        app.selected_idx(),
        app.search_query(),
    );

    custom_resources::render_custom_resources(
        frame,
        chunks[1],
        &app.extension_instances,
        app.extension_error.as_deref(),
    );
}
