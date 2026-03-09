//! Input event routing and dispatching.

use crossterm::event::KeyEvent;

use crate::{
    app::{AppAction, AppState, Focus},
    workbench::WorkbenchTabState,
};

/// Routes a keyboard event to the appropriate handler based on application state.
pub fn route_keyboard_input(key: KeyEvent, app_state: &mut AppState) -> AppAction {
    app_state.handle_key_event(key)
}

/// Applies an action to the application state and returns whether state changed.
pub fn apply_action(action: AppAction, app_state: &mut AppState) -> bool {
    match action {
        AppAction::None => false,
        AppAction::Quit => {
            app_state.should_quit = true;
            true
        }
        AppAction::RefreshData => true,
        AppAction::FluxReconcile => true,
        AppAction::OpenDetail(_) => true,
        AppAction::CloseDetail => {
            app_state.detail_view = None;
            true
        }
        AppAction::OpenNamespacePicker => {
            app_state.open_namespace_picker();
            true
        }
        AppAction::CloseNamespacePicker => {
            app_state.close_namespace_picker();
            true
        }
        AppAction::SelectNamespace(ns) => {
            app_state.set_namespace(ns);
            app_state.close_namespace_picker();
            true
        }
        AppAction::OpenContextPicker => {
            app_state.context_picker.open();
            true
        }
        AppAction::CloseContextPicker => {
            app_state.close_context_picker();
            true
        }
        AppAction::SelectContext(_) => {
            app_state.close_context_picker();
            true
        }
        AppAction::ToggleNavGroup(group) => {
            app_state.toggle_nav_group(group);
            true
        }
        AppAction::OpenCommandPalette => {
            app_state.command_palette.open();
            true
        }
        AppAction::CloseCommandPalette => {
            app_state.command_palette.close();
            true
        }
        AppAction::NavigateTo(view) => {
            app_state.command_palette.close();
            app_state.view = view;
            app_state.selected_idx = 0;
            true
        }
        AppAction::EscapePressed => {
            if app_state.focus == Focus::Workbench && app_state.workbench().maximized {
                app_state.workbench_toggle_maximize();
            } else if app_state.focus == Focus::Workbench {
                app_state.blur_workbench();
            } else if app_state
                .detail_view
                .as_ref()
                .and_then(|d| d.scale_dialog.as_ref())
                .is_some()
            {
                app_state.close_scale_dialog();
            } else if app_state
                .detail_view
                .as_ref()
                .and_then(|d| d.probe_panel.as_ref())
                .is_some()
            {
                app_state.close_probe_panel();
            } else if app_state.detail_view.is_some() {
                app_state.detail_view = None;
            } else {
                app_state.should_quit = true;
            }
            true
        }
        AppAction::LogsViewerOpen => {
            app_state.open_logs_viewer();
            true
        }
        AppAction::LogsViewerClose => false,
        AppAction::LogsViewerScrollUp => {
            if let Some(tab) = app_state.workbench_mut().active_tab_mut()
                && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
            {
                logs_tab.viewer.scroll_offset = logs_tab.viewer.scroll_offset.saturating_sub(1);
                return true;
            }
            false
        }
        AppAction::LogsViewerScrollDown => {
            if let Some(tab) = app_state.workbench_mut().active_tab_mut()
                && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
            {
                let max = logs_tab.viewer.lines.len().saturating_sub(1);
                logs_tab.viewer.scroll_offset = (logs_tab.viewer.scroll_offset + 1).min(max);
                return true;
            }
            false
        }
        AppAction::LogsViewerScrollTop => {
            if let Some(tab) = app_state.workbench_mut().active_tab_mut()
                && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
            {
                logs_tab.viewer.scroll_offset = 0;
                return true;
            }
            false
        }
        AppAction::LogsViewerScrollBottom => {
            if let Some(tab) = app_state.workbench_mut().active_tab_mut()
                && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
            {
                logs_tab.viewer.scroll_offset = logs_tab.viewer.lines.len().saturating_sub(1);
                return true;
            }
            false
        }
        AppAction::LogsViewerToggleFollow => {
            if let Some(tab) = app_state.workbench_mut().active_tab_mut()
                && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
            {
                logs_tab.viewer.follow_mode = !logs_tab.viewer.follow_mode;
                return true;
            }
            false
        }
        AppAction::LogsViewerPickerUp => {
            if let Some(tab) = app_state.workbench_mut().active_tab_mut()
                && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
                && logs_tab.viewer.picking_container
            {
                logs_tab.viewer.container_cursor =
                    logs_tab.viewer.container_cursor.saturating_sub(1);
                return true;
            }
            false
        }
        AppAction::LogsViewerPickerDown => {
            if let Some(tab) = app_state.workbench_mut().active_tab_mut()
                && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
                && logs_tab.viewer.picking_container
            {
                // Extra "All Containers" entry when 2+ containers
                let extra = if logs_tab.viewer.containers.len() > 1 {
                    1
                } else {
                    0
                };
                let max = (logs_tab.viewer.containers.len() + extra).saturating_sub(1);
                logs_tab.viewer.container_cursor = (logs_tab.viewer.container_cursor + 1).min(max);
                return true;
            }
            false
        }
        AppAction::LogsViewerSearchOpen => {
            if let Some(tab) = app_state.workbench_mut().active_tab_mut()
                && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
            {
                logs_tab.viewer.searching = true;
                logs_tab.viewer.search_input = logs_tab.viewer.search_query.clone();
                return true;
            }
            false
        }
        AppAction::LogsViewerSearchClose => {
            if let Some(tab) = app_state.workbench_mut().active_tab_mut()
                && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
            {
                logs_tab.viewer.search_query = logs_tab.viewer.search_input.clone();
                logs_tab.viewer.searching = false;
                return true;
            }
            false
        }
        AppAction::LogsViewerSearchNext => {
            if let Some(tab) = app_state.workbench_mut().active_tab_mut()
                && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
                && !logs_tab.viewer.search_query.is_empty()
            {
                let query = logs_tab.viewer.search_query.to_ascii_lowercase();
                let start = logs_tab.viewer.scroll_offset + 1;
                if let Some(pos) = logs_tab
                    .viewer
                    .lines
                    .iter()
                    .skip(start)
                    .position(|l| l.to_ascii_lowercase().contains(&query))
                {
                    logs_tab.viewer.scroll_offset = start + pos;
                    logs_tab.viewer.follow_mode = false;
                }
                return true;
            }
            false
        }
        AppAction::LogsViewerSearchPrev => {
            if let Some(tab) = app_state.workbench_mut().active_tab_mut()
                && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
                && !logs_tab.viewer.search_query.is_empty()
            {
                let query = logs_tab.viewer.search_query.to_ascii_lowercase();
                let end = logs_tab.viewer.scroll_offset;
                if let Some(pos) = logs_tab.viewer.lines[..end]
                    .iter()
                    .rev()
                    .position(|l| l.to_ascii_lowercase().contains(&query))
                {
                    logs_tab.viewer.scroll_offset = end - 1 - pos;
                    logs_tab.viewer.follow_mode = false;
                }
                return true;
            }
            false
        }
        // LogsViewerSelectContainer / SelectAll / TogglePrevious / ToggleTimestamps handled in main.rs (needs async log fetch)
        AppAction::LogsViewerTogglePrevious => true,
        AppAction::LogsViewerSelectContainer(_) => true,
        AppAction::LogsViewerSelectAllContainers => true,
        AppAction::LogsViewerToggleTimestamps => true,
        AppAction::OpenResourceYaml => true,
        AppAction::OpenResourceEvents => true,
        AppAction::OpenActionHistory => {
            app_state.open_action_history_tab(true);
            true
        }
        AppAction::OpenExec => true,
        AppAction::PortForwardOpen => {
            app_state.open_port_forward();
            true
        }
        AppAction::PortForwardCreate(_)
        | AppAction::PortForwardRefresh
        | AppAction::PortForwardStop(_) => {
            // Handled in main.rs event loop
            true
        }
        AppAction::ScaleDialogOpen => {
            app_state.open_scale_dialog();
            true
        }
        AppAction::ScaleDialogClose => {
            app_state.close_scale_dialog();
            true
        }
        AppAction::ScaleDialogUpdateInput(c) => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(scale) = &mut detail.scale_dialog
            {
                scale.handle_action(crate::ui::components::scale_dialog::ScaleAction::AddChar(c));
                return true;
            }
            false
        }
        AppAction::ScaleDialogBackspace => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(scale) = &mut detail.scale_dialog
            {
                scale.handle_action(crate::ui::components::scale_dialog::ScaleAction::DeleteChar);
                return true;
            }
            false
        }
        AppAction::ScaleDialogIncrement => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(scale) = &mut detail.scale_dialog
            {
                scale.handle_action(crate::ui::components::scale_dialog::ScaleAction::Increment);
                return true;
            }
            false
        }
        AppAction::ScaleDialogDecrement => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(scale) = &mut detail.scale_dialog
            {
                scale.handle_action(crate::ui::components::scale_dialog::ScaleAction::Decrement);
                return true;
            }
            false
        }
        AppAction::ScaleDialogSubmit => {
            // Handled in main.rs event loop (needs async K8s call)
            true
        }
        AppAction::RolloutRestart => {
            // Handled in main.rs event loop (needs async K8s call)
            true
        }
        AppAction::EditYaml => {
            // Handled in main.rs event loop (needs terminal handoff + async apply)
            true
        }
        AppAction::DeleteResource => {
            // Handled in main.rs event loop (needs async K8s call)
            true
        }
        AppAction::ForceDeleteResource => {
            // Handled in main.rs (needs async K8s call)
            true
        }
        AppAction::TriggerCronJob => {
            // Handled in main.rs (needs async K8s call)
            true
        }
        AppAction::CycleTheme => {
            crate::ui::theme::cycle_theme();
            true
        }
        AppAction::ProbePanelOpen => {
            app_state.open_probe_panel();
            true
        }
        AppAction::ProbePanelClose => {
            app_state.close_probe_panel();
            true
        }
        AppAction::ProbeToggleExpand => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(panel) = &mut detail.probe_panel
            {
                panel.toggle_expand();
                return true;
            }
            false
        }
        AppAction::ProbeSelectNext => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(panel) = &mut detail.probe_panel
            {
                panel.select_next();
                return true;
            }
            false
        }
        AppAction::ProbeSelectPrev => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(panel) = &mut detail.probe_panel
            {
                panel.select_prev();
                return true;
            }
            false
        }
        AppAction::ToggleWorkbench => {
            app_state.toggle_workbench();
            true
        }
        AppAction::WorkbenchNextTab => {
            app_state.workbench_next_tab();
            true
        }
        AppAction::WorkbenchPreviousTab => {
            app_state.workbench_previous_tab();
            true
        }
        AppAction::WorkbenchCloseActiveTab => {
            app_state.workbench_close_active_tab();
            true
        }
        AppAction::WorkbenchIncreaseHeight => {
            app_state.workbench_increase_height();
            true
        }
        AppAction::WorkbenchDecreaseHeight => {
            app_state.workbench_decrease_height();
            true
        }
        AppAction::WorkbenchToggleMaximize => {
            app_state.workbench_toggle_maximize();
            true
        }
        AppAction::ActionHistoryOpenSelected => {
            // Handled in main.rs event loop (needs resource jump / detail fetch)
            true
        }
        AppAction::ExecSelectContainer(_) | AppAction::ExecSendInput => {
            // Handled in main.rs event loop (needs async exec session runtime)
            true
        }
        AppAction::OpenHelp => {
            app_state.help_overlay.toggle();
            true
        }
        AppAction::CloseHelp => {
            app_state.help_overlay.close();
            true
        }
        AppAction::CopyResourceName | AppAction::CopyResourceFullName => {
            // Handled in main.rs (needs cluster snapshot to resolve selected resource)
            true
        }
        AppAction::CopyLogContent => {
            // Handled in main.rs (needs log buffer access)
            true
        }
        AppAction::ExportLogs => {
            // Handled in main.rs (needs log buffer access)
            true
        }
        AppAction::PaletteAction { .. } => {
            app_state.command_palette.close();
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_action_none() {
        let mut app = AppState::default();
        assert!(!apply_action(AppAction::None, &mut app));
    }

    #[test]
    fn test_apply_action_quit() {
        let mut app = AppState::default();
        assert!(!app.should_quit);
        apply_action(AppAction::Quit, &mut app);
        assert!(app.should_quit);
    }

    #[test]
    fn test_apply_action_palette_action_closes_palette() {
        use crate::app::ResourceRef;
        use crate::policy::DetailAction;
        let mut app = AppState::default();
        app.command_palette
            .open_with_context(Some(ResourceRef::Pod("test".into(), "default".into())));
        let changed = apply_action(
            AppAction::PaletteAction {
                action: DetailAction::ViewYaml,
                resource: ResourceRef::Pod("test".into(), "default".into()),
            },
            &mut app,
        );
        assert!(changed);
        assert!(!app.command_palette.is_open());
    }

    #[test]
    fn test_apply_action_close_detail() {
        let mut app = AppState::default();
        app.detail_view = Some(Default::default());
        assert!(app.detail_view.is_some());
        apply_action(AppAction::CloseDetail, &mut app);
        assert!(app.detail_view.is_none());
    }
}
