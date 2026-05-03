//! Extensions view: CRD picker + custom resources list.

pub mod crds;
pub mod custom_resources;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::Frame,
};

use crate::{app::AppState, state::ClusterSnapshot};

const STACKED_EXTENSIONS_WIDTH: u16 = 72;

fn use_stacked_extensions_layout(area: Rect) -> bool {
    area.width < STACKED_EXTENSIONS_WIDTH
}

/// Renders extensions split-pane with CRDs (left) and instances (right).
pub fn render_extensions(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    app: &AppState,
    _focused: bool,
) {
    let stacked = use_stacked_extensions_layout(area);
    let chunks = Layout::default()
        .direction(if stacked {
            Direction::Vertical
        } else {
            Direction::Horizontal
        })
        .constraints(if stacked {
            [Constraint::Percentage(40), Constraint::Percentage(60)]
        } else {
            [Constraint::Percentage(45), Constraint::Percentage(55)]
        })
        .split(area);

    crds::render_crd_picker(
        frame,
        chunks[0],
        &snapshot.custom_resource_definitions,
        matches!(
            snapshot.phase,
            crate::state::DataPhase::Loading | crate::state::DataPhase::Idle
        ),
        app.selected_idx(),
        app.search_query(),
        !app.extension_in_instances,
    );

    custom_resources::render_custom_resources(
        frame,
        chunks[1],
        custom_resources::CustomResourcesPane {
            resources: &app.extension_instances,
            error: app.extension_error.as_deref(),
            is_loading: app.extension_instances_loading,
            selected_crd: app.extension_selected_crd.as_deref(),
            selected_idx: app.extension_instance_cursor,
            is_focused: app.extension_in_instances,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacked_extensions_layout_activates_on_narrow_width() {
        assert!(use_stacked_extensions_layout(Rect::new(0, 0, 60, 20)));
        assert!(!use_stacked_extensions_layout(Rect::new(0, 0, 90, 20)));
    }
}
