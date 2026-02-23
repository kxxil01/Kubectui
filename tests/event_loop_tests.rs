//! Phase 3 Stream A: Event Loop Integration & Keyboard Input Tests
//!
//! Tests for:
//! - Input routing based on active component
//! - Component state transitions (open/close)
//! - Keyboard bindings for each component
//! - Priority ordering (LogsViewer > PortForward > Scale > ProbePanel > DetailView > MainView)

use crossterm::event::{KeyCode, KeyEvent};
use kubectui::app::{
    ActiveComponent, AppAction, AppState, DetailViewState, PortForwardField, ResourceRef,
};
use kubectui::events::{apply_action, route_keyboard_input};

#[test]
fn test_logs_viewer_open_close() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());

    // Open logs viewer
    let key = KeyEvent::from(KeyCode::Char('l'));
    let action = route_keyboard_input(key, &mut app);
    apply_action(action, &mut app);

    assert_eq!(app.active_component(), ActiveComponent::LogsViewer);

    // Close logs viewer with Escape
    let key = KeyEvent::from(KeyCode::Esc);
    let action = route_keyboard_input(key, &mut app);
    apply_action(action, &mut app);

    assert_eq!(app.active_component(), ActiveComponent::None);
}

#[test]
fn test_logs_viewer_scroll_controls() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());
    app.open_logs_viewer();

    // Test 'k' for scroll up
    let key = KeyEvent::from(KeyCode::Char('k'));
    let action = route_keyboard_input(key, &mut app);
    assert_eq!(action, AppAction::LogsViewerScrollUp);

    // Test 'j' for scroll down
    let key = KeyEvent::from(KeyCode::Char('j'));
    let action = route_keyboard_input(key, &mut app);
    assert_eq!(action, AppAction::LogsViewerScrollDown);

    // Test arrow keys
    let key = KeyEvent::from(KeyCode::Up);
    let action = route_keyboard_input(key, &mut app);
    assert_eq!(action, AppAction::LogsViewerScrollUp);

    let key = KeyEvent::from(KeyCode::Down);
    let action = route_keyboard_input(key, &mut app);
    assert_eq!(action, AppAction::LogsViewerScrollDown);
}

#[test]
fn test_logs_viewer_follow_mode_toggle() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());
    app.open_logs_viewer();

    // Toggle follow mode
    let key = KeyEvent::from(KeyCode::Char('f'));
    let action = route_keyboard_input(key, &mut app);
    assert_eq!(action, AppAction::LogsViewerToggleFollow);

    // Apply the action
    apply_action(action, &mut app);

    // Verify follow mode is toggled
    if let Some(detail) = &app.detail_view {
        if let Some(logs) = &detail.logs_viewer {
            assert!(logs.follow_mode);
        }
    }
}

#[test]
fn test_port_forward_open_close() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());

    // Open port forward dialog
    let key = KeyEvent::from(KeyCode::Char('f'));
    let action = route_keyboard_input(key, &mut app);
    apply_action(action, &mut app);

    assert_eq!(app.active_component(), ActiveComponent::PortForward);

    // Close with Escape
    let key = KeyEvent::from(KeyCode::Esc);
    let action = route_keyboard_input(key, &mut app);
    apply_action(action, &mut app);

    assert_eq!(app.active_component(), ActiveComponent::None);
}

#[test]
fn test_port_forward_field_navigation_tab() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());
    app.open_port_forward();

    // Should start at LocalPort field
    if let Some(detail) = &app.detail_view {
        if let Some(pf) = &detail.port_forward_dialog {
            assert_eq!(pf.active_field, PortForwardField::LocalPort);
        }
    }

    // Navigate to RemotePort with Tab
    let key = KeyEvent::from(KeyCode::Tab);
    let action = route_keyboard_input(key, &mut app);
    apply_action(action, &mut app);

    if let Some(detail) = &app.detail_view {
        if let Some(pf) = &detail.port_forward_dialog {
            assert_eq!(pf.active_field, PortForwardField::RemotePort);
        }
    }

    // Navigate to TunnelList with Tab
    let key = KeyEvent::from(KeyCode::Tab);
    let action = route_keyboard_input(key, &mut app);
    apply_action(action, &mut app);

    if let Some(detail) = &app.detail_view {
        if let Some(pf) = &detail.port_forward_dialog {
            assert_eq!(pf.active_field, PortForwardField::TunnelList);
        }
    }

    // Wrap around to LocalPort with Tab
    let key = KeyEvent::from(KeyCode::Tab);
    let action = route_keyboard_input(key, &mut app);
    apply_action(action, &mut app);

    if let Some(detail) = &app.detail_view {
        if let Some(pf) = &detail.port_forward_dialog {
            assert_eq!(pf.active_field, PortForwardField::LocalPort);
        }
    }
}

#[test]
fn test_port_forward_field_navigation_backtab() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());
    app.open_port_forward();

    // Navigate backwards with Shift+Tab
    let key = KeyEvent::from(KeyCode::BackTab);
    let action = route_keyboard_input(key, &mut app);
    apply_action(action, &mut app);

    if let Some(detail) = &app.detail_view {
        if let Some(pf) = &detail.port_forward_dialog {
            assert_eq!(pf.active_field, PortForwardField::TunnelList);
        }
    }
}

#[test]
fn test_port_forward_digit_input() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());
    app.open_port_forward();

    // Type '8' in LocalPort field
    let key = KeyEvent::from(KeyCode::Char('8'));
    let action = route_keyboard_input(key, &mut app);
    assert_eq!(
        action,
        AppAction::PortForwardUpdateLocalPort("8".to_string())
    );

    apply_action(action, &mut app);

    if let Some(detail) = &app.detail_view {
        if let Some(pf) = &detail.port_forward_dialog {
            assert_eq!(pf.local_port, "8");
        }
    }

    // Type '0' to make "80"
    let key = KeyEvent::from(KeyCode::Char('0'));
    let action = route_keyboard_input(key, &mut app);
    apply_action(action, &mut app);

    if let Some(detail) = &app.detail_view {
        if let Some(pf) = &detail.port_forward_dialog {
            assert_eq!(pf.local_port, "80");
        }
    }
}

#[test]
fn test_scale_dialog_open_close() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());

    // Open scale dialog
    let key = KeyEvent::from(KeyCode::Char('s'));
    let action = route_keyboard_input(key, &mut app);
    apply_action(action, &mut app);

    assert_eq!(app.active_component(), ActiveComponent::Scale);

    // Close with Escape
    let key = KeyEvent::from(KeyCode::Esc);
    let action = route_keyboard_input(key, &mut app);
    apply_action(action, &mut app);

    assert_eq!(app.active_component(), ActiveComponent::None);
}

#[test]
fn test_scale_dialog_numeric_input() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());
    app.open_scale_dialog();

    // Type digits
    for digit in "35".chars() {
        let key = KeyEvent::from(KeyCode::Char(digit));
        let action = route_keyboard_input(key, &mut app);
        apply_action(action, &mut app);
    }

    if let Some(detail) = &app.detail_view {
        if let Some(scale) = &detail.scale_dialog {
            assert_eq!(scale.replica_input, "35");
            assert_eq!(scale.target_replicas, 35);
        }
    }
}

#[test]
fn test_scale_dialog_backspace() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());
    app.open_scale_dialog();

    // Type "42"
    for digit in "42".chars() {
        let key = KeyEvent::from(KeyCode::Char(digit));
        let action = route_keyboard_input(key, &mut app);
        apply_action(action, &mut app);
    }

    // Backspace to remove the '2'
    let key = KeyEvent::from(KeyCode::Backspace);
    let action = route_keyboard_input(key, &mut app);
    apply_action(action, &mut app);

    if let Some(detail) = &app.detail_view {
        if let Some(scale) = &detail.scale_dialog {
            assert_eq!(scale.replica_input, "4");
        }
    }
}

#[test]
fn test_probe_panel_open_close() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());

    // Open probe panel (not tested with a specific key, just open manually)
    app.open_probe_panel();
    assert_eq!(app.active_component(), ActiveComponent::ProbePanel);

    // Close with Escape
    let key = KeyEvent::from(KeyCode::Esc);
    let action = route_keyboard_input(key, &mut app);
    apply_action(action, &mut app);

    assert_eq!(app.active_component(), ActiveComponent::None);
}

#[test]
fn test_detail_view_navigation_keys() {
    let mut app = AppState::default();
    let resource = ResourceRef::Pod("test-pod".to_string(), "default".to_string());
    app.detail_view = Some(DetailViewState {
        resource: Some(resource),
        ..DetailViewState::default()
    });

    // Arrow keys should work in detail view
    let key = KeyEvent::from(KeyCode::Down);
    let _action = route_keyboard_input(key, &mut app);

    // L key should open logs viewer
    let key = KeyEvent::from(KeyCode::Char('l'));
    let action = route_keyboard_input(key, &mut app);
    assert_eq!(action, AppAction::LogsViewerOpen);
}

#[test]
fn test_component_priority_escape_closes_logs_first() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());

    // Open logs viewer
    app.open_logs_viewer();
    assert_eq!(app.active_component(), ActiveComponent::LogsViewer);

    // Press Escape
    let key = KeyEvent::from(KeyCode::Esc);
    let action = route_keyboard_input(key, &mut app);

    // Should get EscapePressed action
    assert_eq!(action, AppAction::EscapePressed);

    // Apply it
    apply_action(action, &mut app);

    // Logs should close, not detail view
    assert_eq!(app.active_component(), ActiveComponent::None);
    assert!(app.detail_view.is_some());
}

#[test]
fn test_main_view_quit_on_escape() {
    let mut app = AppState::default();
    // No detail view open — first Esc sets confirm_quit
    let action = route_keyboard_input(KeyEvent::from(KeyCode::Esc), &mut app);
    assert_eq!(action, AppAction::None);
    assert!(app.confirm_quit);

    // Second Esc cancels the dialog
    let action = route_keyboard_input(KeyEvent::from(KeyCode::Esc), &mut app);
    assert_eq!(action, AppAction::None);
    assert!(!app.confirm_quit);

    // q then y confirms quit
    route_keyboard_input(KeyEvent::from(KeyCode::Char('q')), &mut app);
    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('y')), &mut app);
    assert_eq!(action, AppAction::Quit);
}

#[test]
fn test_main_view_quit_on_q() {
    let mut app = AppState::default();

    // First q sets confirm_quit
    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('q')), &mut app);
    assert_eq!(action, AppAction::None);
    assert!(app.confirm_quit);

    // Second q confirms quit
    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('q')), &mut app);
    assert_eq!(action, AppAction::Quit);
}

#[test]
fn test_logs_viewer_with_capital_l() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());

    // Try capital L
    let key = KeyEvent::from(KeyCode::Char('L'));
    let action = route_keyboard_input(key, &mut app);

    // Should open logs viewer
    assert_eq!(action, AppAction::LogsViewerOpen);
}

#[test]
fn test_all_components_can_be_opened_independently() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());

    // Open each component and verify state
    app.open_logs_viewer();
    assert_eq!(app.active_component(), ActiveComponent::LogsViewer);

    app.close_logs_viewer();
    app.open_port_forward();
    assert_eq!(app.active_component(), ActiveComponent::PortForward);

    app.close_port_forward();
    app.open_scale_dialog();
    assert_eq!(app.active_component(), ActiveComponent::Scale);

    app.close_scale_dialog();
    app.open_probe_panel();
    assert_eq!(app.active_component(), ActiveComponent::ProbePanel);

    app.close_probe_panel();
    assert_eq!(app.active_component(), ActiveComponent::None);
}

#[test]
fn test_component_state_persistence() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());
    app.open_logs_viewer();

    // Modify logs viewer state
    if let Some(detail) = &mut app.detail_view {
        if let Some(logs) = &mut detail.logs_viewer {
            logs.scroll_offset = 42;
            logs.follow_mode = true;
        }
    }

    // State should persist
    if let Some(detail) = &app.detail_view {
        if let Some(logs) = &detail.logs_viewer {
            assert_eq!(logs.scroll_offset, 42);
            assert!(logs.follow_mode);
        }
    }

    // Close and reopen
    app.close_logs_viewer();
    app.open_logs_viewer();

    // State should be reset (new state created)
    if let Some(detail) = &app.detail_view {
        if let Some(logs) = &detail.logs_viewer {
            assert_eq!(logs.scroll_offset, 0);
            assert!(!logs.follow_mode);
        }
    }
}

#[test]
fn test_probe_panel_navigation() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState::default());
    app.open_probe_panel();

    // Add some probes to the state
    if let Some(detail) = &mut app.detail_view {
        if let Some(probe) = &mut detail.probe_panel {
            probe.probes = vec![
                "Probe1".to_string(),
                "Probe2".to_string(),
                "Probe3".to_string(),
            ];
            probe.expanded = vec![false, false, false];
        }
    }

    // Test space to toggle expand
    let key = KeyEvent::from(KeyCode::Char(' '));
    let action = route_keyboard_input(key, &mut app);
    assert_eq!(action, AppAction::ProbeToggleExpand(0));

    // Test 'j' for next
    let key = KeyEvent::from(KeyCode::Char('j'));
    let action = route_keyboard_input(key, &mut app);
    assert_eq!(action, AppAction::ProbeSelectNext);

    // Test 'k' for previous
    let key = KeyEvent::from(KeyCode::Char('k'));
    let action = route_keyboard_input(key, &mut app);
    assert_eq!(action, AppAction::ProbeSelectPrev);
}
