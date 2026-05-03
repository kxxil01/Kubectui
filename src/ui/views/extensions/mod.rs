//! Extensions view: CRD picker + custom resources list.

pub mod crds;
pub mod custom_resources;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::Frame,
};

use crate::{
    app::{AppState, AppView},
    state::{ClusterSnapshot, DataPhase, RefreshScope, ViewLoadState},
    ui::active_view_fetch_error,
};

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

    let crds_loading = matches!(snapshot.phase, DataPhase::Loading | DataPhase::Idle)
        || (!snapshot.scope_loaded(RefreshScope::EXTENSIONS)
            && matches!(
                snapshot.view_load_state(AppView::Extensions),
                ViewLoadState::Idle | ViewLoadState::Loading | ViewLoadState::Refreshing
            ));

    crds::render_crd_picker(
        frame,
        chunks[0],
        crds::CrdPickerPane {
            crds: &snapshot.custom_resource_definitions,
            is_loading: crds_loading,
            fetch_error: active_view_fetch_error(snapshot, AppView::Extensions),
            selected_idx: app.selected_idx(),
            query: app.search_query(),
            is_focused: !app.extension_in_instances,
        },
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
