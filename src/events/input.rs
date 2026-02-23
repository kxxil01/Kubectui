//! Input event routing and dispatching.

use crossterm::event::KeyEvent;

use crate::app::{AppAction, AppState};

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
        AppAction::EscapePressed => {
            if app_state
                .detail_view
                .as_ref()
                .and_then(|d| d.logs_viewer.as_ref())
                .is_some()
            {
                app_state.close_logs_viewer();
            } else if app_state
                .detail_view
                .as_ref()
                .and_then(|d| d.port_forward_dialog.as_ref())
                .is_some()
            {
                app_state.close_port_forward();
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
        AppAction::LogsViewerClose => {
            app_state.close_logs_viewer();
            true
        }
        AppAction::LogsViewerScrollUp => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(logs) = &mut detail.logs_viewer
            {
                logs.scroll_offset = logs.scroll_offset.saturating_sub(1);
                return true;
            }
            false
        }
        AppAction::LogsViewerScrollDown => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(logs) = &mut detail.logs_viewer
            {
                logs.scroll_offset = logs.scroll_offset.saturating_add(1);
                return true;
            }
            false
        }
        AppAction::LogsViewerToggleFollow => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(logs) = &mut detail.logs_viewer
            {
                logs.follow_mode = !logs.follow_mode;
                return true;
            }
            false
        }
        AppAction::PortForwardOpen => {
            app_state.open_port_forward();
            true
        }
        AppAction::PortForwardClose => {
            app_state.close_port_forward();
            true
        }
        AppAction::PortForwardNextField => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(dialog) = &mut detail.port_forward_dialog
            {
                dialog.active_field = match dialog.active_field {
                    crate::app::PortForwardField::LocalPort => {
                        crate::app::PortForwardField::RemotePort
                    }
                    crate::app::PortForwardField::RemotePort => {
                        crate::app::PortForwardField::TunnelList
                    }
                    crate::app::PortForwardField::TunnelList => {
                        crate::app::PortForwardField::LocalPort
                    }
                };
                return true;
            }
            false
        }
        AppAction::PortForwardPrevField => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(dialog) = &mut detail.port_forward_dialog
            {
                dialog.active_field = match dialog.active_field {
                    crate::app::PortForwardField::LocalPort => {
                        crate::app::PortForwardField::TunnelList
                    }
                    crate::app::PortForwardField::RemotePort => {
                        crate::app::PortForwardField::LocalPort
                    }
                    crate::app::PortForwardField::TunnelList => {
                        crate::app::PortForwardField::RemotePort
                    }
                };
                return true;
            }
            false
        }
        AppAction::PortForwardUpdateLocalPort(text) => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(dialog) = &mut detail.port_forward_dialog
            {
                dialog.local_port.push_str(&text);
                return true;
            }
            false
        }
        AppAction::PortForwardUpdateRemotePort(text) => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(dialog) = &mut detail.port_forward_dialog
            {
                dialog.remote_port.push_str(&text);
                return true;
            }
            false
        }
        AppAction::PortForwardBackspace => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(dialog) = &mut detail.port_forward_dialog
            {
                match dialog.active_field {
                    crate::app::PortForwardField::LocalPort => {
                        dialog.local_port.pop();
                    }
                    crate::app::PortForwardField::RemotePort => {
                        dialog.remote_port.pop();
                    }
                    crate::app::PortForwardField::TunnelList => {}
                }
                return true;
            }
            false
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
                scale.replica_input.push(c);
                scale.target_replicas = scale.replica_input.parse::<i32>().unwrap_or(0);
                return true;
            }
            false
        }
        AppAction::ScaleDialogBackspace => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(scale) = &mut detail.scale_dialog
            {
                scale.replica_input.pop();
                scale.target_replicas = scale.replica_input.parse::<i32>().unwrap_or(0);
                return true;
            }
            false
        }
        AppAction::ProbePanelOpen => {
            app_state.open_probe_panel();
            true
        }
        AppAction::ProbePanelClose => {
            app_state.close_probe_panel();
            true
        }
        AppAction::ProbeToggleExpand(index) => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(panel) = &mut detail.probe_panel
            {
                if index < panel.expanded.len() {
                    panel.expanded[index] = !panel.expanded[index];
                }
                return true;
            }
            false
        }
        AppAction::ProbeSelectNext => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(panel) = &mut detail.probe_panel
            {
                if !panel.probes.is_empty() {
                    panel.selected_idx = (panel.selected_idx + 1) % panel.probes.len();
                }
                return true;
            }
            false
        }
        AppAction::ProbeSelectPrev => {
            if let Some(detail) = &mut app_state.detail_view
                && let Some(panel) = &mut detail.probe_panel
            {
                if !panel.probes.is_empty() {
                    panel.selected_idx = panel.selected_idx.saturating_sub(1);
                }
                return true;
            }
            false
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
    fn test_apply_action_close_detail() {
        let mut app = AppState::default();
        app.detail_view = Some(Default::default());
        assert!(app.detail_view.is_some());
        apply_action(AppAction::CloseDetail, &mut app);
        assert!(app.detail_view.is_none());
    }
}
