#![allow(clippy::field_reassign_with_default)]
//! Event Loop Integration & Keyboard Input Tests
//!
//! Tests for:
//! - Input routing based on active component
//! - Component state transitions (open/close)
//! - Keyboard bindings for each component
//! - Priority ordering (LogsViewer > PortForward > Scale > ProbePanel > DetailView > MainView)

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kubectui::events::{apply_action, route_keyboard_input};
use kubectui::secret::{DecodedSecretEntry, DecodedSecretValue};
use kubectui::ui::components::{ScaleField, port_forward_dialog::PortForwardMode};
use kubectui::workbench::WorkbenchTabState;
use kubectui::{
    action_history::{ActionKind, ActionStatus},
    app::{ActiveComponent, AppAction, AppState, AppView, DetailViewState, Focus, ResourceRef},
};

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

fn secret_detail() -> DetailViewState {
    DetailViewState {
        resource: Some(ResourceRef::Secret(
            "app-secret".to_string(),
            "default".to_string(),
        )),
        yaml: Some("apiVersion: v1\nkind: Secret\ndata:\n  token: aGVsbG8=\n".to_string()),
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
fn test_logs_viewer_ctrl_l_does_not_open() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());

    let action = route_keyboard_input(
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
        &mut app,
    );
    assert_eq!(action, AppAction::None);
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
fn test_logs_viewer_ctrl_f_does_not_toggle_follow_mode() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());
    app.open_logs_viewer();

    let action = route_keyboard_input(
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        &mut app,
    );
    assert_eq!(action, AppAction::None);
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
fn test_port_forward_ctrl_f_does_not_open_from_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());

    let action = route_keyboard_input(
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        &mut app,
    );
    assert_eq!(action, AppAction::None);
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
fn test_workbench_yaml_tab_refresh_uses_global_refresh_action() {
    let mut app = AppState::default();
    app.open_resource_yaml_tab(
        ResourceRef::Pod("test-pod".to_string(), "default".to_string()),
        Some("kind: Pod".to_string()),
        None,
        None,
    );

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('r')), &mut app);
    assert_eq!(action, AppAction::RefreshData);
}

#[test]
fn test_secret_detail_o_opens_decoded_secret() {
    let mut app = AppState::default();
    app.detail_view = Some(secret_detail());

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('o')), &mut app);
    assert_eq!(action, AppAction::OpenDecodedSecret);
}

#[test]
fn test_secret_detail_uppercase_b_toggles_bookmark() {
    let mut app = AppState::default();
    app.detail_view = Some(secret_detail());

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('B')), &mut app);
    assert_eq!(action, AppAction::ToggleBookmark);
}

#[test]
fn test_secret_detail_ctrl_shift_b_does_not_toggle_bookmark() {
    let mut app = AppState::default();
    app.detail_view = Some(secret_detail());

    let action = route_keyboard_input(
        KeyEvent::new(
            KeyCode::Char('B'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        &mut app,
    );
    assert_eq!(action, AppAction::None);
}

#[test]
fn test_secrets_list_o_opens_decoded_secret() {
    let mut app = AppState::default();
    app.view = AppView::Secrets;
    app.focus = Focus::Content;

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('o')), &mut app);
    assert_eq!(action, AppAction::OpenDecodedSecret);
}

#[test]
fn test_decoded_secret_tab_refresh_uses_global_refresh_action() {
    let mut app = AppState::default();
    app.open_decoded_secret_tab(
        ResourceRef::Secret("app-secret".to_string(), "default".to_string()),
        Some("apiVersion: v1\nkind: Secret\ndata:\n  token: aGVsbG8=\n".to_string()),
        None,
        None,
    );

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('r')), &mut app);
    assert_eq!(action, AppAction::RefreshData);
}

#[test]
fn test_decoded_secret_editor_keeps_r_as_text() {
    let mut app = AppState::default();
    app.open_decoded_secret_tab(
        ResourceRef::Secret("app-secret".to_string(), "default".to_string()),
        Some("apiVersion: v1\nkind: Secret\ndata:\n  token: aGVsbG8=\n".to_string()),
        None,
        None,
    );

    if let Some(tab) = app.workbench_mut().active_tab_mut()
        && let WorkbenchTabState::DecodedSecret(secret_tab) = &mut tab.state
    {
        secret_tab.entries = vec![DecodedSecretEntry {
            key: "token".to_string(),
            value: DecodedSecretValue::Text {
                original: "hello".to_string(),
                current: "hello".to_string(),
            },
        }];
        secret_tab.masked = false;
        secret_tab.editing = true;
        secret_tab.edit_input.clear();
    }

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('r')), &mut app);
    assert_eq!(action, AppAction::None);

    if let Some(tab) = app.workbench().active_tab()
        && let WorkbenchTabState::DecodedSecret(secret_tab) = &tab.state
    {
        assert_eq!(secret_tab.edit_input, "r");
        assert!(secret_tab.editing);
    } else {
        panic!("expected active decoded secret tab");
    }
}

#[test]
fn test_decoded_secret_editor_requires_reveal_before_editing() {
    let mut app = AppState::default();
    app.open_decoded_secret_tab(
        ResourceRef::Secret("app-secret".to_string(), "default".to_string()),
        Some("apiVersion: v1\nkind: Secret\ndata:\n  token: aGVsbG8=\n".to_string()),
        None,
        None,
    );

    if let Some(tab) = app.workbench_mut().active_tab_mut()
        && let WorkbenchTabState::DecodedSecret(secret_tab) = &mut tab.state
    {
        secret_tab.entries = vec![DecodedSecretEntry {
            key: "token".to_string(),
            value: DecodedSecretValue::Text {
                original: "hello".to_string(),
                current: "hello".to_string(),
            },
        }];
    }

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('e')), &mut app);
    assert_eq!(action, AppAction::None);

    if let Some(tab) = app.workbench().active_tab()
        && let WorkbenchTabState::DecodedSecret(secret_tab) = &tab.state
    {
        assert!(secret_tab.masked);
        assert!(!secret_tab.editing);
        assert!(secret_tab.edit_input.is_empty());
    } else {
        panic!("expected active decoded secret tab");
    }

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('m')), &mut app);
    assert_eq!(action, AppAction::None);
    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('e')), &mut app);
    assert_eq!(action, AppAction::None);

    if let Some(tab) = app.workbench().active_tab()
        && let WorkbenchTabState::DecodedSecret(secret_tab) = &tab.state
    {
        assert!(!secret_tab.masked);
        assert!(secret_tab.editing);
        assert_eq!(secret_tab.edit_input, "hello");
    } else {
        panic!("expected active decoded secret tab");
    }
}

#[test]
fn test_decoded_secret_tab_save_emits_action_when_dirty() {
    let mut app = AppState::default();
    app.open_decoded_secret_tab(
        ResourceRef::Secret("app-secret".to_string(), "default".to_string()),
        Some("apiVersion: v1\nkind: Secret\ndata:\n  token: aGVsbG8=\n".to_string()),
        None,
        None,
    );

    if let Some(tab) = app.workbench_mut().active_tab_mut()
        && let WorkbenchTabState::DecodedSecret(secret_tab) = &mut tab.state
    {
        secret_tab.entries = vec![DecodedSecretEntry {
            key: "token".to_string(),
            value: DecodedSecretValue::Text {
                original: "hello".to_string(),
                current: "updated".to_string(),
            },
        }];
    }

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('s')), &mut app);
    assert_eq!(action, AppAction::SaveDecodedSecret);
}

#[test]
fn test_logs_viewer_refresh_emits_global_action_when_not_searching() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());
    app.open_logs_viewer();

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('r')), &mut app);
    assert_eq!(action, AppAction::RefreshData);
}

#[test]
fn test_logs_viewer_search_keeps_r_as_text() {
    let mut app = AppState::default();
    app.detail_view = Some(pod_detail());
    app.open_logs_viewer();

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('/')), &mut app);
    apply_action(action, &mut app);

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('r')), &mut app);
    assert_eq!(action, AppAction::None);

    if let Some(tab) = app.workbench().active_tab()
        && let WorkbenchTabState::PodLogs(logs_tab) = &tab.state
    {
        assert_eq!(logs_tab.viewer.search_input, "r");
        assert!(logs_tab.viewer.searching);
    } else {
        panic!("expected active pod logs tab");
    }
}

#[test]
fn test_history_shortcut_opens_action_history_tab() {
    let mut app = AppState::default();

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('H')), &mut app);
    apply_action(action, &mut app);

    assert!(app.workbench().open);
    assert!(matches!(
        app.workbench().active_tab().map(|tab| &tab.state),
        Some(WorkbenchTabState::ActionHistory(_))
    ));
}

#[test]
fn test_action_history_enter_opens_selected_entry() {
    let mut app = AppState::default();
    let entry_id = app.record_action_pending(
        ActionKind::Restart,
        kubectui::app::AppView::Deployments,
        Some(ResourceRef::Deployment(
            "api".to_string(),
            "default".to_string(),
        )),
        "deployment 'api'".to_string(),
        "Requesting restart".to_string(),
    );
    app.complete_action_history(entry_id, ActionStatus::Succeeded, "Restart requested", true);
    app.open_action_history_tab(true);

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Enter), &mut app);
    assert_eq!(action, AppAction::ActionHistoryOpenSelected);
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
fn test_scale_dialog_modified_backspace_does_not_edit() {
    let mut app = AppState::default();
    app.detail_view = Some(deployment_detail());
    app.open_scale_dialog();

    for digit in "42".chars() {
        let action = route_keyboard_input(KeyEvent::from(KeyCode::Char(digit)), &mut app);
        apply_action(action, &mut app);
    }

    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::META,
        KeyModifiers::SUPER,
    ] {
        let action = route_keyboard_input(KeyEvent::new(KeyCode::Backspace, modifiers), &mut app);
        apply_action(action, &mut app);

        if let Some(detail) = &app.detail_view
            && let Some(scale) = &detail.scale_dialog
        {
            assert_eq!(scale.desired_replicas, "42", "{modifiers:?}");
        }
    }
}

#[test]
fn test_scale_dialog_tab_navigation_routes_to_focus_fields() {
    let mut app = AppState::default();
    app.detail_view = Some(deployment_detail());
    app.open_scale_dialog();

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Tab), &mut app);
    apply_action(action, &mut app);
    assert_eq!(
        app.detail_view
            .as_ref()
            .and_then(|detail| detail.scale_dialog.as_ref())
            .map(|scale| scale.focus_field),
        Some(ScaleField::ApplyBtn)
    );

    let action = route_keyboard_input(
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
        &mut app,
    );
    apply_action(action, &mut app);
    assert_eq!(
        app.detail_view
            .as_ref()
            .and_then(|detail| detail.scale_dialog.as_ref())
            .map(|scale| scale.focus_field),
        Some(ScaleField::InputField)
    );
}

#[test]
fn test_scale_dialog_enter_on_cancel_closes_dialog() {
    let mut app = AppState::default();
    app.detail_view = Some(deployment_detail());
    app.open_scale_dialog();

    for _ in 0..2 {
        let action = route_keyboard_input(KeyEvent::from(KeyCode::Tab), &mut app);
        apply_action(action, &mut app);
    }

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Enter), &mut app);
    apply_action(action, &mut app);

    assert_eq!(app.active_component(), ActiveComponent::None);
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
fn test_main_view_escape_starts_quit_confirmation() {
    let mut app = AppState::default();
    let action = route_keyboard_input(KeyEvent::from(KeyCode::Esc), &mut app);
    assert_eq!(action, AppAction::None);
    assert!(app.confirm_quit);
    assert!(!app.should_quit());
}

#[test]
fn test_main_view_quit_requires_escape_then_enter_only() {
    let mut app = AppState::default();
    route_keyboard_input(KeyEvent::from(KeyCode::Char('q')), &mut app);
    assert!(!app.confirm_quit);
    assert!(!app.should_quit());

    route_keyboard_input(KeyEvent::from(KeyCode::Esc), &mut app);
    assert!(app.confirm_quit);

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('y')), &mut app);
    assert_eq!(action, AppAction::None);
    assert!(!app.confirm_quit);
    assert!(!app.should_quit());

    route_keyboard_input(KeyEvent::from(KeyCode::Esc), &mut app);
    let action = route_keyboard_input(KeyEvent::from(KeyCode::Enter), &mut app);
    assert_eq!(action, AppAction::Quit);
}

#[test]
fn test_main_view_modified_escape_does_not_start_quit_confirmation() {
    for modifiers in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
        let mut app = AppState::default();
        let action = route_keyboard_input(KeyEvent::new(KeyCode::Esc, modifiers), &mut app);
        assert_eq!(action, AppAction::None);
        assert!(!app.confirm_quit);
        assert!(!app.should_quit());

        let action = route_keyboard_input(KeyEvent::from(KeyCode::Enter), &mut app);
        assert_ne!(action, AppAction::Quit);
        assert!(!app.should_quit());
    }
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
