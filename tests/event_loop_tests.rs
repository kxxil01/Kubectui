#![allow(clippy::field_reassign_with_default)]
//! Event Loop Integration & Keyboard Input Tests
//!
//! Tests for:
//! - Input routing based on active component
//! - Component state transitions (open/close)
//! - Keyboard bindings for each component
//! - Priority ordering (LogsViewer > PortForward > Scale > ProbePanel > DetailView > MainView)

use crossterm::event::{KeyCode, KeyEvent};
use kubectui::app::{ActiveComponent, AppAction, AppState, DetailViewState, ResourceRef};
use kubectui::events::{apply_action, route_keyboard_input};
use kubectui::ui::components::port_forward_dialog::PortForwardMode;
use kubectui::workbench::WorkbenchTabState;

fn pod_detail() -> DetailViewState {
    DetailViewState {
        resource: Some(ResourceRef::Pod(
            "test-pod".to_string(),
            "default".to_string(),
        )),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    }
}

fn deployment_detail() -> DetailViewState {
    DetailViewState {
        resource: Some(ResourceRef::Deployment(
            "test-deployment".to_string(),
            "default".to_string(),
        )),
        yaml: Some("kind: Deployment".to_string()),
        ..DetailViewState::default()
    }
}

#[test]
fn test_logs_viewer_open_close() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());

    let key = KeyEvent::from(KeyCode::Char('l'));
    let action = route_keyboard_input(key, &mut app);
    apply_action(action, &mut app);
    assert_eq!(app.active_component(), ActiveComponent::LogsViewer);

    let key = KeyEvent::from(KeyCode::Esc);
    let action = route_keyboard_input(key, &mut app);
    apply_action(action, &mut app);
    assert_eq!(app.active_component(), ActiveComponent::None);
}

#[test]
fn test_logs_viewer_scroll_controls() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());
    app.open_logs_viewer();

    assert_eq!(
        route_keyboard_input(KeyEvent::from(KeyCode::Char('k')), &mut app),
        AppAction::LogsViewerScrollUp
    );
    assert_eq!(
        route_keyboard_input(KeyEvent::from(KeyCode::Char('j')), &mut app),
        AppAction::LogsViewerScrollDown
    );
    assert_eq!(
        route_keyboard_input(KeyEvent::from(KeyCode::Up), &mut app),
        AppAction::LogsViewerScrollUp
    );
    assert_eq!(
        route_keyboard_input(KeyEvent::from(KeyCode::Down), &mut app),
        AppAction::LogsViewerScrollDown
    );
}

#[test]
fn test_logs_viewer_follow_mode_toggle() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());
    app.open_logs_viewer();

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('f')), &mut app);
    assert_eq!(action, AppAction::LogsViewerToggleFollow);
    apply_action(action, &mut app);

    if let Some(tab) = app.workbench().active_tab()
        && let WorkbenchTabState::PodLogs(logs_tab) = &tab.state
    {
        assert!(logs_tab.viewer.follow_mode);
    }
}

#[test]
fn test_port_forward_open_close() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('f')), &mut app);
    apply_action(action, &mut app);
    assert_eq!(app.active_component(), ActiveComponent::PortForward);

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Esc), &mut app);
    apply_action(action, &mut app);
    assert_eq!(app.active_component(), ActiveComponent::None);
}

#[test]
fn test_port_forward_list_refresh_emits_refresh_action() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());
    app.open_port_forward();

    if let Some(tab) = app.workbench_mut().active_tab_mut()
        && let WorkbenchTabState::PortForward(port_tab) = &mut tab.state
    {
        port_tab.dialog.mode = PortForwardMode::List;
    }

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('r')), &mut app);
    assert_eq!(action, AppAction::PortForwardRefresh);
}

#[test]
fn test_scale_dialog_open_close() {
    let mut app = AppState::default();
    app.detail_view = Some(deployment_detail());

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('s')), &mut app);
    apply_action(action, &mut app);
    assert_eq!(app.active_component(), ActiveComponent::Scale);

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Esc), &mut app);
    apply_action(action, &mut app);
    assert_eq!(app.active_component(), ActiveComponent::None);
}

#[test]
fn test_scale_dialog_numeric_input() {
    let mut app = AppState::default();
    app.detail_view = Some(deployment_detail());
    app.open_scale_dialog();

    for digit in "35".chars() {
        let action = route_keyboard_input(KeyEvent::from(KeyCode::Char(digit)), &mut app);
        apply_action(action, &mut app);
    }

    if let Some(detail) = &app.detail_view
        && let Some(scale) = &detail.scale_dialog
    {
        assert_eq!(scale.desired_replicas, "35");
    }
}

#[test]
fn test_scale_dialog_backspace() {
    let mut app = AppState::default();
    app.detail_view = Some(deployment_detail());
    app.open_scale_dialog();

    for digit in "42".chars() {
        let action = route_keyboard_input(KeyEvent::from(KeyCode::Char(digit)), &mut app);
        apply_action(action, &mut app);
    }

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Backspace), &mut app);
    apply_action(action, &mut app);

    if let Some(detail) = &app.detail_view
        && let Some(scale) = &detail.scale_dialog
    {
        assert_eq!(scale.desired_replicas, "4");
    }
}

#[test]
fn test_probe_panel_open_close() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());

    app.open_probe_panel();
    assert_eq!(app.active_component(), ActiveComponent::ProbePanel);

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Esc), &mut app);
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

    let _action = route_keyboard_input(KeyEvent::from(KeyCode::Down), &mut app);
    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('l')), &mut app);
    assert_eq!(action, AppAction::LogsViewerOpen);
}

#[test]
fn test_component_priority_escape_closes_logs_first() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());
    app.open_logs_viewer();
    assert_eq!(app.active_component(), ActiveComponent::LogsViewer);

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Esc), &mut app);
    assert_eq!(action, AppAction::EscapePressed);
    apply_action(action, &mut app);

    assert_eq!(app.active_component(), ActiveComponent::None);
    assert!(app.detail_view.is_some());
}

#[test]
fn test_main_view_quit_on_escape() {
    let mut app = AppState::default();
    let action = route_keyboard_input(KeyEvent::from(KeyCode::Esc), &mut app);
    assert_eq!(action, AppAction::None);
    assert!(app.confirm_quit);

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Esc), &mut app);
    assert_eq!(action, AppAction::None);
    assert!(!app.confirm_quit);

    route_keyboard_input(KeyEvent::from(KeyCode::Char('q')), &mut app);
    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('y')), &mut app);
    assert_eq!(action, AppAction::Quit);
}

#[test]
fn test_main_view_quit_on_q() {
    let mut app = AppState::default();
    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('q')), &mut app);
    assert_eq!(action, AppAction::None);
    assert!(app.confirm_quit);

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('q')), &mut app);
    assert_eq!(action, AppAction::Quit);
}

#[test]
fn test_logs_viewer_with_capital_l() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('L')), &mut app);
    assert_eq!(action, AppAction::LogsViewerOpen);
}

#[test]
fn test_all_components_can_be_opened_independently() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());

    app.open_logs_viewer();
    assert_eq!(app.active_component(), ActiveComponent::LogsViewer);

    app.close_logs_viewer();
    app.open_port_forward();
    assert_eq!(app.active_component(), ActiveComponent::PortForward);

    app.close_port_forward();
    app.detail_view = Some(deployment_detail());
    app.open_scale_dialog();
    assert_eq!(app.active_component(), ActiveComponent::Scale);

    app.close_scale_dialog();
    app.detail_view = Some(pod_detail());
    app.open_probe_panel();
    assert_eq!(app.active_component(), ActiveComponent::ProbePanel);

    app.close_probe_panel();
    assert_eq!(app.active_component(), ActiveComponent::None);
}

#[test]
fn test_component_state_persistence() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());
    app.open_logs_viewer();

    if let Some(tab) = app.workbench_mut().active_tab_mut()
        && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
    {
        logs_tab.viewer.scroll_offset = 42;
        logs_tab.viewer.follow_mode = true;
    }

    if let Some(tab) = app.workbench().active_tab()
        && let WorkbenchTabState::PodLogs(logs_tab) = &tab.state
    {
        assert_eq!(logs_tab.viewer.scroll_offset, 42);
        assert!(logs_tab.viewer.follow_mode);
    }

    app.close_logs_viewer();
    app.open_logs_viewer();

    if let Some(tab) = app.workbench().active_tab()
        && let WorkbenchTabState::PodLogs(logs_tab) = &tab.state
    {
        assert_eq!(logs_tab.viewer.scroll_offset, 0);
        assert!(!logs_tab.viewer.follow_mode);
    }
}

#[test]
fn test_probe_panel_navigation() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());
    app.open_probe_panel();

    assert_eq!(
        route_keyboard_input(KeyEvent::from(KeyCode::Char(' ')), &mut app),
        AppAction::ProbeToggleExpand
    );
    assert_eq!(
        route_keyboard_input(KeyEvent::from(KeyCode::Char('j')), &mut app),
        AppAction::ProbeSelectNext
    );
    assert_eq!(
        route_keyboard_input(KeyEvent::from(KeyCode::Char('k')), &mut app),
        AppAction::ProbeSelectPrev
    );
}
