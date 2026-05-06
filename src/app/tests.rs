use super::*;
use crate::cronjob::CronJobHistoryEntry;
use crate::k8s::dtos::PodInfo;
use crate::k8s::rollout::{RolloutInspection, RolloutRevisionInfo, RolloutWorkloadKind};
use crate::resource_templates::ResourceTemplateKind;
use crate::runbooks::{
    LoadedRunbook, LoadedRunbookStep, LoadedRunbookStepKind, RunbookDetailAction,
};
use crate::workbench::WorkbenchTabState;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn removed_workbench_tab_keys() -> [KeyEvent; 5] {
    [
        KeyEvent::from(KeyCode::Char('[')),
        KeyEvent::from(KeyCode::Char(']')),
        KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::CONTROL),
    ]
}

fn modified_edit_key_events() -> Vec<KeyEvent> {
    let codes = [
        KeyCode::Backspace,
        KeyCode::Delete,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Home,
        KeyCode::End,
    ];
    let modifiers = [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::META,
        KeyModifiers::SUPER,
    ];
    modifiers
        .into_iter()
        .flat_map(|modifier| {
            codes
                .into_iter()
                .map(move |code| KeyEvent::new(code, modifier))
        })
        .collect()
}

/// Verifies full forward tab cycle across all views and wraps to Dashboard.
#[test]
fn tab_cycles_all_views_forward() {
    let mut app = AppState::default();
    let expected = [
        // Overview
        AppView::Projects,
        AppView::Governance,
        AppView::Bookmarks,
        AppView::Issues,
        AppView::HealthReport,
        AppView::Vulnerabilities,
        AppView::Nodes,
        AppView::Namespaces,
        AppView::Events,
        // Workloads
        AppView::Pods,
        AppView::Deployments,
        AppView::StatefulSets,
        AppView::DaemonSets,
        AppView::ReplicaSets,
        AppView::ReplicationControllers,
        AppView::Jobs,
        AppView::CronJobs,
        // Network
        AppView::Services,
        AppView::Endpoints,
        AppView::Ingresses,
        AppView::IngressClasses,
        AppView::GatewayClasses,
        AppView::Gateways,
        AppView::HttpRoutes,
        AppView::GrpcRoutes,
        AppView::ReferenceGrants,
        AppView::NetworkPolicies,
        AppView::PortForwarding,
        // Config
        AppView::ConfigMaps,
        AppView::Secrets,
        AppView::ResourceQuotas,
        AppView::LimitRanges,
        AppView::HPAs,
        AppView::PodDisruptionBudgets,
        AppView::PriorityClasses,
        // Storage
        AppView::PersistentVolumeClaims,
        AppView::PersistentVolumes,
        AppView::StorageClasses,
        // Helm
        AppView::HelmCharts,
        AppView::HelmReleases,
        // FluxCD
        AppView::FluxCDAlertProviders,
        AppView::FluxCDAlerts,
        AppView::FluxCDAll,
        AppView::FluxCDArtifacts,
        AppView::FluxCDHelmReleases,
        AppView::FluxCDHelmRepositories,
        AppView::FluxCDImages,
        AppView::FluxCDKustomizations,
        AppView::FluxCDReceivers,
        AppView::FluxCDSources,
        // Access Control
        AppView::ServiceAccounts,
        AppView::ClusterRoles,
        AppView::Roles,
        AppView::ClusterRoleBindings,
        AppView::RoleBindings,
        // Custom Resources
        AppView::Extensions,
        // Wraps back to start
        AppView::Dashboard,
    ];
    for view in expected {
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), view);
    }
}

/// Verifies reverse tab cycle wraps from Dashboard to Extensions.
#[test]
fn shift_tab_cycles_reverse() {
    let mut app = AppState::default();
    app.handle_key_event(KeyEvent::from(KeyCode::BackTab));
    assert_eq!(app.view(), AppView::Extensions);
}

/// Verifies entering search mode and adding/removing characters.
#[test]
fn search_query_add_backspace_and_clear() {
    let mut app = AppState::default();

    app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
    app.handle_key_event(KeyEvent::from(KeyCode::Char('a')));
    app.handle_key_event(KeyEvent::from(KeyCode::Char('b')));
    app.handle_key_event(KeyEvent::from(KeyCode::Backspace));

    assert_eq!(app.search_query(), "a");

    app.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(app.search_query(), "");
}

#[test]
fn search_query_edit_clears_selection_search_status() {
    let mut app = AppState::default();
    app.set_status(SELECTION_SEARCH_FALLBACK_STATUS.to_string());
    app.search_query = "Running".to_string();
    app.search_cursor = app.search_query.chars().count();
    app.is_search_mode = true;

    app.handle_key_event(KeyEvent::from(KeyCode::Backspace));

    assert_eq!(app.search_query(), "Runnin");
    assert_eq!(app.status_message(), None);
}

#[test]
fn search_query_edit_preserves_unrelated_status() {
    let mut app = AppState::default();
    app.set_status("Saved workspace: ops".to_string());
    app.search_query = "Running".to_string();
    app.search_cursor = app.search_query.chars().count();
    app.is_search_mode = true;

    app.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));

    assert_eq!(app.search_query(), "");
    assert_eq!(app.status_message(), Some("Saved workspace: ops"));
}

#[test]
fn navigation_clears_selection_search_status() {
    let mut app = AppState {
        view: AppView::Pods,
        search_query: "Running".to_string(),
        search_cursor: "Running".chars().count(),
        is_search_mode: true,
        ..AppState::default()
    };
    app.set_status(SELECTION_SEARCH_NO_VISIBLE_RESULTS_STATUS.to_string());

    app.navigate_to_view(AppView::Services);

    assert_eq!(app.view(), AppView::Services);
    assert!(app.search_query().is_empty());
    assert!(!app.is_search_mode());
    assert_eq!(app.status_message(), None);
}

#[test]
fn navigation_preserves_unrelated_status() {
    let mut app = AppState {
        view: AppView::Pods,
        search_query: "api".to_string(),
        search_cursor: "api".chars().count(),
        is_search_mode: true,
        ..AppState::default()
    };
    app.set_status("Saved workspace: ops".to_string());

    app.navigate_to_view(AppView::Services);

    assert!(app.search_query().is_empty());
    assert!(!app.is_search_mode());
    assert_eq!(app.status_message(), Some("Saved workspace: ops"));
}

#[test]
fn navigation_closes_stale_detail() {
    let mut app = AppState {
        view: AppView::Pods,
        selected_idx: 3,
        detail_view: Some(DetailViewState {
            resource: Some(ResourceRef::Pod("api-0".to_string(), "default".to_string())),
            ..DetailViewState::default()
        }),
        ..AppState::default()
    };

    app.navigate_to_view(AppView::Services);

    assert_eq!(app.view(), AppView::Services);
    assert_eq!(app.selected_idx(), 0);
    assert!(app.detail_view.is_none());
}

#[test]
fn namespace_switch_clears_selection_search_status() {
    let mut app = AppState {
        search_query: "Running".to_string(),
        search_cursor: "Running".chars().count(),
        is_search_mode: true,
        ..AppState::default()
    };
    app.set_status(SELECTION_SEARCH_FALLBACK_STATUS.to_string());

    app.set_namespace("prod".to_string());

    assert_eq!(app.get_namespace(), "prod");
    assert!(app.search_query().is_empty());
    assert!(!app.is_search_mode());
    assert_eq!(app.status_message(), None);
}

#[test]
fn namespace_switch_closes_stale_detail() {
    let mut app = AppState {
        view: AppView::Pods,
        detail_view: Some(DetailViewState {
            resource: Some(ResourceRef::Pod("api-0".to_string(), "default".to_string())),
            ..DetailViewState::default()
        }),
        ..AppState::default()
    };

    app.set_namespace("prod".to_string());

    assert_eq!(app.get_namespace(), "prod");
    assert_eq!(app.selected_idx(), 0);
    assert!(app.detail_view.is_none());
}

#[test]
fn search_query_edit_resets_selected_idx_to_first_result() {
    let mut app = AppState {
        selected_idx: 9,
        ..AppState::default()
    };
    app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));

    app.handle_key_event(KeyEvent::from(KeyCode::Char('a')));

    assert_eq!(app.search_query(), "a");
    assert_eq!(app.selected_idx, 0);
}

#[test]
fn search_cursor_move_does_not_reset_selected_idx() {
    let mut app = AppState {
        selected_idx: 9,
        search_query: "api".to_string(),
        search_cursor: 3,
        is_search_mode: true,
        ..AppState::default()
    };

    app.handle_key_event(KeyEvent::from(KeyCode::Left));

    assert_eq!(app.search_query(), "api");
    assert_eq!(app.search_cursor, 2);
    assert_eq!(app.selected_idx, 9);
}

#[test]
fn search_query_edit_closes_stale_detail() {
    let mut app = AppState {
        search_query: "api".to_string(),
        search_cursor: 3,
        is_search_mode: true,
        detail_view: Some(DetailViewState {
            resource: Some(ResourceRef::Pod("api-0".to_string(), "default".to_string())),
            ..DetailViewState::default()
        }),
        ..AppState::default()
    };

    app.handle_key_event(KeyEvent::from(KeyCode::Char('x')));

    assert_eq!(app.search_query(), "apix");
    assert!(app.detail_view.is_none());
}

#[test]
fn search_esc_with_empty_query_closes_stale_detail() {
    let mut app = AppState {
        selected_idx: 5,
        is_search_mode: true,
        detail_view: Some(DetailViewState {
            resource: Some(ResourceRef::Pod("api-5".to_string(), "default".to_string())),
            ..DetailViewState::default()
        }),
        ..AppState::default()
    };

    app.handle_key_event(KeyEvent::from(KeyCode::Esc));

    assert_eq!(app.selected_idx, 0);
    assert!(!app.is_search_mode());
    assert!(app.search_query().is_empty());
    assert!(app.detail_view.is_none());
}

#[test]
fn search_esc_with_empty_query_clears_selection_search_status() {
    let mut app = AppState {
        is_search_mode: true,
        ..AppState::default()
    };
    app.set_status(SELECTION_SEARCH_FALLBACK_STATUS.to_string());

    app.handle_key_event(KeyEvent::from(KeyCode::Esc));

    assert_eq!(app.status_message(), None);
}

#[test]
fn search_cursor_move_keeps_detail_open() {
    let resource = ResourceRef::Pod("api-0".to_string(), "default".to_string());
    let mut app = AppState {
        search_query: "api".to_string(),
        search_cursor: 3,
        is_search_mode: true,
        detail_view: Some(DetailViewState {
            resource: Some(resource.clone()),
            ..DetailViewState::default()
        }),
        ..AppState::default()
    };

    app.handle_key_event(KeyEvent::from(KeyCode::Left));

    assert_eq!(
        app.detail_view
            .as_ref()
            .and_then(|detail| detail.resource.as_ref()),
        Some(&resource)
    );
}

#[test]
fn search_query_supports_cursor_editing() {
    let mut app = AppState::default();

    app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
    app.handle_key_event(KeyEvent::from(KeyCode::Char('a')));
    app.handle_key_event(KeyEvent::from(KeyCode::Char('c')));
    app.handle_key_event(KeyEvent::from(KeyCode::Left));
    app.handle_key_event(KeyEvent::from(KeyCode::Char('b')));

    assert_eq!(app.search_query(), "abc");
}

#[test]
fn search_query_supports_unicode_cursor_editing() {
    let mut app = AppState::default();

    app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
    app.handle_key_event(KeyEvent::from(KeyCode::Char('a')));
    app.handle_key_event(KeyEvent::from(KeyCode::Char('å')));
    app.handle_key_event(KeyEvent::from(KeyCode::Char('b')));
    app.handle_key_event(KeyEvent::from(KeyCode::Left));
    app.handle_key_event(KeyEvent::from(KeyCode::Left));
    app.handle_key_event(KeyEvent::from(KeyCode::Char('β')));
    app.handle_key_event(KeyEvent::from(KeyCode::Delete));
    app.handle_key_event(KeyEvent::from(KeyCode::Backspace));

    assert_eq!(app.search_query(), "ab");
}

#[test]
fn search_query_caps_at_input_limit() {
    let mut app = AppState::default();
    app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));

    for _ in 0..(crate::app::input::MAX_SEARCH_QUERY_CHARS + 10) {
        app.handle_key_event(KeyEvent::from(KeyCode::Char('x')));
    }

    assert_eq!(
        app.search_query().chars().count(),
        crate::app::input::MAX_SEARCH_QUERY_CHARS
    );
}

/// Verifies pressing Esc in search mode exits mode and clears query.
#[test]
fn search_mode_esc_clears_and_exits() {
    let mut app = AppState::default();
    app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
    app.handle_key_event(KeyEvent::from(KeyCode::Char('x')));

    app.handle_key_event(KeyEvent::from(KeyCode::Esc));

    assert_eq!(app.search_query(), "");
    assert!(!app.is_search_mode());
}

#[test]
fn search_mode_modified_escape_does_not_clear_or_exit() {
    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::META,
        KeyModifiers::SUPER,
        KeyModifiers::CONTROL | KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::SUPER,
    ] {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
        app.handle_key_event(KeyEvent::from(KeyCode::Char('x')));

        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );
        assert_eq!(app.search_query(), "x", "{modifiers:?}");
        assert!(app.is_search_mode(), "{modifiers:?}");
    }
}

#[test]
fn search_mode_modified_edit_keys_do_not_mutate_query_or_cursor() {
    for key in modified_edit_key_events() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
        app.handle_key_event(KeyEvent::from(KeyCode::Char('a')));
        app.handle_key_event(KeyEvent::from(KeyCode::Char('c')));
        app.handle_key_event(KeyEvent::from(KeyCode::Left));

        assert_eq!(app.handle_key_event(key), AppAction::None, "{key:?}");
        assert_eq!(app.search_query(), "ac", "{key:?}");
        assert_eq!(app.search_cursor, 1, "{key:?}");
    }
}

/// Verifies refresh actions are emitted for `r` and Ctrl+R.
#[test]
fn refresh_action_transitions() {
    let mut app = AppState::default();
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('r'))),
        AppAction::RefreshData
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::CONTROL)),
        AppAction::RefreshData
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
        AppAction::RefreshData
    );
}

#[test]
fn flux_view_uppercase_r_triggers_reconcile_without_overriding_ctrl_r() {
    let mut app = AppState::default();
    app.view = AppView::FluxCDKustomizations;

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('R'))),
        AppAction::FluxReconcile
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::CONTROL)),
        AppAction::RefreshData
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
        AppAction::RefreshData
    );
}

#[test]
fn flux_alerts_view_uppercase_r_is_noop_but_ctrl_r_still_refreshes() {
    let mut app = AppState::default();
    app.view = AppView::FluxCDAlerts;

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('R'))),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::CONTROL)),
        AppAction::RefreshData
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
        AppAction::RefreshData
    );
}

#[test]
fn flux_detail_uppercase_r_triggers_reconcile_for_supported_resource() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::CustomResource {
            name: "apps".to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
        }),
        ..DetailViewState::default()
    });

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('R'))),
        AppAction::FluxReconcile
    );
}

#[test]
fn unsupported_flux_detail_uppercase_r_is_noop() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::CustomResource {
            name: "webhook".to_string(),
            namespace: Some("flux-system".to_string()),
            group: "notification.toolkit.fluxcd.io".to_string(),
            version: "v1beta3".to_string(),
            kind: "Alert".to_string(),
            plural: "alerts".to_string(),
        }),
        ..DetailViewState::default()
    });

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('R'))),
        AppAction::None
    );
}

#[test]
fn flux_detail_uppercase_r_is_blocked_when_confirmation_dialog_open() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::CustomResource {
            name: "apps".to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
        }),
        confirm_delete: true,
        ..DetailViewState::default()
    });

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('R'))),
        AppAction::None
    );
}

/// Verifies namespace can be switched through dedicated mutators.
#[test]
fn test_appstate_namespace_switching() {
    let mut app = AppState::default();
    assert_eq!(app.get_namespace(), "all");

    app.set_namespace("kube-system".to_string());
    assert_eq!(app.get_namespace(), "kube-system");
}

/// Verifies namespace picker shortcut emits open action.
#[test]
fn tilde_opens_namespace_picker_action() {
    let mut app = AppState::default();
    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('~')));
    assert_eq!(action, AppAction::OpenNamespacePicker);
}

#[test]
fn resource_template_dialog_ctrl_u_clears_active_field() {
    let mut app = AppState::default();
    app.resource_template_dialog = Some(crate::ui::components::ResourceTemplateDialogState::new(
        ResourceTemplateKind::Deployment,
        "default",
    ));

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));

    assert_eq!(action, AppAction::None);
    assert_eq!(
        app.resource_template_dialog
            .as_ref()
            .expect("dialog should remain open")
            .values
            .name,
        ""
    );
}

#[test]
fn resource_template_dialog_shifted_ctrl_u_clears_active_field() {
    let mut app = AppState::default();
    app.resource_template_dialog = Some(crate::ui::components::ResourceTemplateDialogState::new(
        ResourceTemplateKind::Deployment,
        "default",
    ));

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('U'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));

    assert_eq!(action, AppAction::None);
    assert_eq!(
        app.resource_template_dialog
            .as_ref()
            .expect("dialog should remain open")
            .values
            .name,
        ""
    );
}

#[test]
fn resource_template_dialog_modified_ctrl_u_does_not_clear_active_field() {
    let mut app = AppState::default();
    app.resource_template_dialog = Some(crate::ui::components::ResourceTemplateDialogState::new(
        ResourceTemplateKind::Deployment,
        "default",
    ));

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    ));

    assert_eq!(action, AppAction::None);
    assert_eq!(
        app.resource_template_dialog
            .as_ref()
            .expect("dialog should remain open")
            .values
            .name,
        "sample-app"
    );
}

#[test]
fn resource_template_dialog_modified_navigation_keys_do_not_move_focus() {
    let mut app = AppState::default();
    app.resource_template_dialog = Some(crate::ui::components::ResourceTemplateDialogState::new(
        ResourceTemplateKind::Deployment,
        "default",
    ));

    for (code, modifiers) in [
        (KeyCode::Down, KeyModifiers::CONTROL),
        (KeyCode::Up, KeyModifiers::CONTROL),
        (KeyCode::Tab, KeyModifiers::ALT),
        (KeyCode::BackTab, KeyModifiers::ALT),
    ] {
        assert_eq!(
            app.handle_key_event(KeyEvent::new(code, modifiers)),
            AppAction::None,
            "{code:?} {modifiers:?}"
        );
        assert_eq!(
            app.resource_template_dialog
                .as_ref()
                .expect("dialog should remain open")
                .focus_field,
            crate::ui::components::ResourceTemplateField::Name
        );
    }
}

#[test]
fn resource_template_dialog_alt_modified_chars_do_not_edit_fields() {
    let mut app = AppState::default();
    app.resource_template_dialog = Some(crate::ui::components::ResourceTemplateDialogState::new(
        ResourceTemplateKind::Deployment,
        "default",
    ));

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
        AppAction::None
    );

    assert_eq!(
        app.resource_template_dialog
            .as_ref()
            .expect("dialog should remain open")
            .values
            .name,
        "sample-app"
    );
}

#[test]
fn resource_template_dialog_modified_edit_keys_do_not_mutate_fields_or_cursor() {
    for key in modified_edit_key_events() {
        let mut app = AppState::default();
        app.resource_template_dialog =
            Some(crate::ui::components::ResourceTemplateDialogState::new(
                ResourceTemplateKind::Deployment,
                "default",
            ));
        let dialog = app.resource_template_dialog.as_mut().unwrap();
        dialog.values.name = "sample-app".into();

        assert_eq!(app.handle_key_event(key), AppAction::None, "{key:?}");

        let dialog = app.resource_template_dialog.as_ref().unwrap();
        assert_eq!(dialog.values.name, "sample-app", "{key:?}");
    }
}

#[test]
fn resource_template_dialog_modified_enter_does_not_submit_or_cancel() {
    let mut submit = AppState::default();
    submit.resource_template_dialog =
        Some(crate::ui::components::ResourceTemplateDialogState::new(
            ResourceTemplateKind::Deployment,
            "default",
        ));
    submit
        .resource_template_dialog
        .as_mut()
        .expect("dialog should exist")
        .focus_field = crate::ui::components::ResourceTemplateField::CreateBtn;

    assert_eq!(
        submit.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
        AppAction::None
    );
    assert!(submit.resource_template_dialog.is_some());

    let mut cancel = AppState::default();
    cancel.resource_template_dialog =
        Some(crate::ui::components::ResourceTemplateDialogState::new(
            ResourceTemplateKind::Deployment,
            "default",
        ));
    cancel
        .resource_template_dialog
        .as_mut()
        .expect("dialog should exist")
        .focus_field = crate::ui::components::ResourceTemplateField::CancelBtn;

    assert_eq!(
        cancel.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
        AppAction::None
    );
    assert!(cancel.resource_template_dialog.is_some());
}

#[test]
fn resource_template_dialog_modified_escape_does_not_close() {
    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::META,
        KeyModifiers::SUPER,
        KeyModifiers::CONTROL | KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::SUPER,
    ] {
        let mut app = AppState::default();
        app.resource_template_dialog =
            Some(crate::ui::components::ResourceTemplateDialogState::new(
                ResourceTemplateKind::Deployment,
                "default",
            ));

        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );
        assert!(app.resource_template_dialog.is_some(), "{modifiers:?}");
    }
}

#[test]
fn workspace_shortcuts_emit_actions() {
    let mut app = AppState::default();

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('W'))),
        AppAction::SaveWorkspace
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('{'))),
        AppAction::ApplyPreviousWorkspace
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('}'))),
        AppAction::ApplyNextWorkspace
    );
}

#[test]
fn ctrl_shift_w_does_not_save_workspace() {
    let mut app = AppState::default();

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('W'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));

    assert_eq!(action, AppAction::None);
}

#[test]
fn ctrl_shift_t_and_i_do_not_cycle_theme_or_icons() {
    let mut app = AppState::default();

    assert_eq!(
        app.handle_key_event(KeyEvent::new(
            KeyCode::Char('T'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        )),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(
            KeyCode::Char('I'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        )),
        AppAction::None
    );
}

#[test]
fn modified_plain_main_shortcuts_do_not_fire_without_configured_hotkey() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
    app.focus = Focus::Content;
    app.selected_idx = 3;

    for (code, modifiers) in [
        (KeyCode::Char('n'), KeyModifiers::CONTROL),
        (KeyCode::Char('a'), KeyModifiers::CONTROL),
        (KeyCode::Char('1'), KeyModifiers::ALT),
        (KeyCode::Char('2'), KeyModifiers::CONTROL),
        (KeyCode::Char('3'), KeyModifiers::CONTROL),
        (KeyCode::Char('0'), KeyModifiers::CONTROL),
        (KeyCode::Char('/'), KeyModifiers::CONTROL),
        (
            KeyCode::Char('~'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        (
            KeyCode::Char('{'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        (
            KeyCode::Char('}'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        (KeyCode::Char('b'), KeyModifiers::CONTROL),
        (KeyCode::Char('c'), KeyModifiers::CONTROL),
        (
            KeyCode::Char(':'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        (
            KeyCode::Char('T'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        (
            KeyCode::Char('I'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        (
            KeyCode::Char('?'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
    ] {
        assert_eq!(
            app.handle_key_event(KeyEvent::new(code, modifiers)),
            AppAction::None,
            "{code:?} {modifiers:?}"
        );
    }

    assert_eq!(app.pod_sort(), None);
    assert_eq!(app.selected_idx, 3);
    assert!(!app.is_search_mode());
    assert!(!app.namespace_picker.is_open());
    assert!(!app.command_palette.is_open());
    assert!(!app.help_overlay.is_open());
    assert!(!app.workbench.open);
}

#[test]
fn configured_workspace_hotkey_routes_before_main_navigation() {
    let mut app = AppState::default();
    let prefs = app.preferences.get_or_insert_with(Default::default);
    prefs
        .workspaces
        .hotkeys
        .push(crate::workspaces::HotkeyBinding {
            key: "alt+1".into(),
            target: crate::workspaces::HotkeyTarget::View {
                view: AppView::Pods,
            },
        });

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT)),
        AppAction::NavigateTo(AppView::Pods)
    );
}

#[test]
fn configured_workspace_action_hotkey_routes_to_global_action() {
    let mut app = AppState::default();
    let prefs = app.preferences.get_or_insert_with(Default::default);
    prefs
        .workspaces
        .hotkeys
        .push(crate::workspaces::HotkeyBinding {
            key: "alt+r".into(),
            target: crate::workspaces::HotkeyTarget::Action {
                action: crate::workspaces::HotkeyAction::RefreshData,
            },
        });

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT)),
        AppAction::RefreshData
    );
}

#[test]
fn pods_sort_keybindings_toggle_and_clear() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
    app.focus = Focus::Content;

    assert_eq!(app.pod_sort(), None);

    app.handle_key_event(KeyEvent::from(KeyCode::Char('1')));
    assert_eq!(
        app.pod_sort(),
        Some(PodSortState::new(PodSortColumn::Age, true))
    );

    app.handle_key_event(KeyEvent::from(KeyCode::Char('1')));
    assert_eq!(
        app.pod_sort(),
        Some(PodSortState::new(PodSortColumn::Age, false))
    );

    app.handle_key_event(KeyEvent::from(KeyCode::Char('3')));
    assert_eq!(
        app.pod_sort(),
        Some(PodSortState::new(PodSortColumn::Restarts, true))
    );

    app.handle_key_event(KeyEvent::from(KeyCode::Char('0')));
    assert_eq!(app.pod_sort(), None);
}

#[test]
fn pods_name_sort_shortcut_toggles() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
    app.focus = Focus::Content;

    app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
    assert_eq!(
        app.pod_sort(),
        Some(PodSortState::new(PodSortColumn::Name, false))
    );

    app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
    assert_eq!(
        app.pod_sort(),
        Some(PodSortState::new(PodSortColumn::Name, true))
    );
}

#[test]
fn pods_sort_keybindings_are_scoped_to_pods_view() {
    let mut app = AppState::default();
    app.view = AppView::Services;
    app.focus = Focus::Content;

    app.handle_key_event(KeyEvent::from(KeyCode::Char('1')));
    assert_eq!(app.pod_sort(), None);
}

#[test]
fn pods_sort_keybindings_require_content_focus() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
    app.focus = Focus::Sidebar;

    for key in ['n', 'a', '1', '2', '3', '0'] {
        app.handle_key_event(KeyEvent::from(KeyCode::Char(key)));
    }

    assert_eq!(app.pod_sort(), None);
}

#[test]
fn workload_sort_keybindings_toggle_and_clear() {
    let mut app = AppState::default();
    app.view = AppView::Deployments;
    app.focus = Focus::Content;

    assert_eq!(app.workload_sort(), None);

    app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
    assert_eq!(
        app.workload_sort(),
        Some(WorkloadSortState::new(WorkloadSortColumn::Name, false))
    );

    app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
    assert_eq!(
        app.workload_sort(),
        Some(WorkloadSortState::new(WorkloadSortColumn::Name, true))
    );

    app.handle_key_event(KeyEvent::from(KeyCode::Char('a')));
    assert_eq!(
        app.workload_sort(),
        Some(WorkloadSortState::new(WorkloadSortColumn::Age, true))
    );

    app.handle_key_event(KeyEvent::from(KeyCode::Char('0')));
    assert_eq!(app.workload_sort(), None);
}

#[test]
fn workload_sort_keybindings_are_scoped_to_workload_views() {
    let mut app = AppState::default();
    app.view = AppView::ConfigMaps;
    app.focus = Focus::Content;

    app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
    assert_eq!(app.workload_sort(), None);
}

#[test]
fn workload_sort_keybindings_require_content_focus() {
    let mut app = AppState::default();
    app.view = AppView::Deployments;
    app.focus = Focus::Sidebar;

    for key in ['n', 'a', '1', '0'] {
        app.handle_key_event(KeyEvent::from(KeyCode::Char(key)));
    }

    assert_eq!(app.workload_sort(), None);
}

#[test]
fn workbench_keybindings_emit_expected_actions() {
    use crate::workbench::{ActionHistoryTabState, WorkbenchTabState};

    let mut app = AppState::default();

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('b'))),
        AppAction::ToggleWorkbench
    );

    // Add a tab (background so open stays false), then toggle open
    app.workbench
        .ensure_background_tab(WorkbenchTabState::ActionHistory(
            ActionHistoryTabState::default(),
        ));
    app.toggle_workbench();
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('.'))),
        AppAction::WorkbenchNextTab
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char(','))),
        AppAction::WorkbenchPreviousTab
    );
    for key in removed_workbench_tab_keys() {
        assert_eq!(app.handle_key_event(key), AppAction::None, "{key:?}");
    }
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)),
        AppAction::WorkbenchCloseActiveTab
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(
            KeyCode::Char('W'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )),
        AppAction::WorkbenchCloseActiveTab
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL)),
        AppAction::WorkbenchIncreaseHeight
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL)),
        AppAction::WorkbenchDecreaseHeight
    );
}

#[test]
fn content_events_shortcut_only_routes_supported_views() {
    let mut app = AppState {
        focus: Focus::Content,
        view: AppView::Pods,
        ..AppState::default()
    };

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('v'))),
        AppAction::OpenResourceEvents
    );

    app.view = AppView::Events;
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('v'))),
        AppAction::None
    );

    app.view = AppView::Nodes;
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('v'))),
        AppAction::None
    );
}

#[test]
fn content_logs_shortcut_only_routes_supported_views() {
    let mut app = AppState {
        focus: Focus::Content,
        view: AppView::Pods,
        ..AppState::default()
    };

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('l'))),
        AppAction::LogsViewerOpen
    );

    app.view = AppView::Deployments;
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('L'))),
        AppAction::LogsViewerOpen
    );

    app.view = AppView::CronJobs;
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('l'))),
        AppAction::None
    );

    app.view = AppView::ConfigMaps;
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('l'))),
        AppAction::None
    );
}

#[test]
fn content_selected_resource_shortcuts_ignore_non_resource_views() {
    let mut app = AppState {
        focus: Focus::Content,
        view: AppView::Pods,
        ..AppState::default()
    };

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('y'))),
        AppAction::OpenResourceYaml
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('A'))),
        AppAction::OpenAccessReview
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('Y'))),
        AppAction::CopyResourceFullName
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('B'))),
        AppAction::ToggleBookmark
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)),
        AppAction::CopyResourceName
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(
            KeyCode::Char('Y'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )),
        AppAction::CopyResourceName
    );

    for view in [
        AppView::Dashboard,
        AppView::HelmCharts,
        AppView::PortForwarding,
        AppView::Extensions,
    ] {
        app.view = view;
        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('y'))),
            AppAction::None
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('A'))),
            AppAction::None
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('Y'))),
            AppAction::None
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('B'))),
            AppAction::None
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)),
            AppAction::None
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::new(
                KeyCode::Char('Y'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            AppAction::None
        );
    }

    app.view = AppView::Extensions;
    app.extension_in_instances = true;
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('y'))),
        AppAction::OpenResourceYaml
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('A'))),
        AppAction::OpenAccessReview
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('Y'))),
        AppAction::CopyResourceFullName
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('B'))),
        AppAction::ToggleBookmark
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)),
        AppAction::CopyResourceName
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(
            KeyCode::Char('Y'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )),
        AppAction::CopyResourceName
    );
}

#[test]
fn content_pod_only_shortcuts_ignore_non_pod_views() {
    let mut app = AppState {
        focus: Focus::Content,
        view: AppView::Pods,
        ..AppState::default()
    };

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('x'))),
        AppAction::OpenExec
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('f'))),
        AppAction::PortForwardOpen
    );

    app.view = AppView::Deployments;
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('x'))),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('f'))),
        AppAction::None
    );
}

#[test]
fn workbench_b_key_toggles_from_workbench_focus() {
    use crate::workbench::{ActionHistoryTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.workbench
        .ensure_background_tab(WorkbenchTabState::ActionHistory(
            ActionHistoryTabState::default(),
        ));
    app.toggle_workbench();
    app.focus = Focus::Workbench;
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('b'))),
        AppAction::ToggleWorkbench
    );
}

#[test]
fn ctrl_b_does_not_toggle_workbench_from_workbench_focus() {
    use crate::workbench::{ActionHistoryTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.workbench
        .ensure_background_tab(WorkbenchTabState::ActionHistory(
            ActionHistoryTabState::default(),
        ));
    app.toggle_workbench();
    app.focus = Focus::Workbench;

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)),
        AppAction::None
    );
}

#[test]
fn workbench_focus_supports_tab_resize_and_close_shortcuts() {
    use crate::workbench::{ActionHistoryTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::ActionHistory(
        ActionHistoryTabState::default(),
    ));
    app.focus = Focus::Workbench;

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('.'))),
        AppAction::WorkbenchNextTab
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char(','))),
        AppAction::WorkbenchPreviousTab
    );
    for key in removed_workbench_tab_keys() {
        assert_eq!(app.handle_key_event(key), AppAction::None, "{key:?}");
    }
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)),
        AppAction::WorkbenchCloseActiveTab
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(
            KeyCode::Char('W'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )),
        AppAction::WorkbenchCloseActiveTab
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL)),
        AppAction::WorkbenchIncreaseHeight
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL)),
        AppAction::WorkbenchDecreaseHeight
    );
}

#[test]
fn relations_workbench_tab_escape_returns_from_workbench() {
    use crate::workbench::{RelationsTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::Relations(RelationsTabState::new(
            ResourceRef::Pod("api".into(), "prod".into()),
        )));
    app.focus = Focus::Workbench;

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Esc)),
        AppAction::EscapePressed
    );
}

#[test]
fn ctrl_alt_workbench_control_shortcuts_do_not_fire() {
    use crate::workbench::{ActionHistoryTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::ActionHistory(
        ActionHistoryTabState::default(),
    ));
    app.workbench
        .ensure_background_tab(WorkbenchTabState::ActionHistory(
            ActionHistoryTabState::default(),
        ));
    app.focus = Focus::Workbench;
    let active_before = app.workbench.active_tab;
    let height_before = app.workbench.height;

    for code in [
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Char('w'),
        KeyCode::Up,
        KeyCode::Down,
    ] {
        assert_eq!(
            app.handle_key_event(KeyEvent::new(
                code,
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )),
            AppAction::None,
            "{code:?}"
        );
    }

    assert_eq!(app.workbench.active_tab, active_before);
    assert_eq!(app.workbench.height, height_before);
    assert!(app.workbench.open);
}

#[test]
fn workbench_local_editor_blocks_global_tab_shortcuts() {
    use crate::workbench::{PodLogsTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
        )));
    app.focus = Focus::Workbench;

    {
        let Some(tab) = app.workbench.active_tab_mut() else {
            panic!("expected active workbench tab");
        };
        let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state else {
            panic!("expected pod logs tab");
        };
        logs_tab.viewer.searching = true;
    }

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('['))),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL)),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::PodLogs(logs_tab) = &tab.state else {
        panic!("expected pod logs tab");
    };
    assert_eq!(logs_tab.viewer.search_input, "[");
}

#[test]
fn ai_workbench_tab_supports_scrolling_shortcuts() {
    use crate::workbench::{AiAnalysisTabState, WorkbenchTabState};

    let mut app = AppState::default();
    let mut tab = AiAnalysisTabState::new(
        9,
        "Ask AI",
        ResourceRef::Pod("api-0".into(), "default".into()),
        "Codex CLI",
        "codex-cli",
        Vec::new(),
    );
    tab.apply_result(
        "AI",
        "gpt-test",
        "summary".into(),
        vec!["cause-1".into(), "cause-2".into()],
        vec!["step-1".into(), "step-2".into()],
        vec!["uncertain-1".into()],
    );
    app.workbench
        .open_tab(WorkbenchTabState::AiAnalysis(Box::new(tab)));
    app.focus = Focus::Workbench;

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('j'))),
        AppAction::None
    );
    let scroll_after_down = match &app.workbench.active_tab().expect("tab").state {
        WorkbenchTabState::AiAnalysis(tab) => tab.scroll,
        _ => panic!("expected ai analysis tab"),
    };
    assert_eq!(scroll_after_down, 1);

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('G'))),
        AppAction::None
    );
    let max_scroll = match &app.workbench.active_tab().expect("tab").state {
        WorkbenchTabState::AiAnalysis(tab) => tab.scroll,
        _ => panic!("expected ai analysis tab"),
    };
    assert!(max_scroll > 1);

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('g'))),
        AppAction::None
    );
    let scroll_after_top = match &app.workbench.active_tab().expect("tab").state {
        WorkbenchTabState::AiAnalysis(tab) => tab.scroll,
        _ => panic!("expected ai analysis tab"),
    };
    assert_eq!(scroll_after_top, 0);
}

#[test]
fn extension_output_shortcuts_do_not_clamp_to_logical_line_count() {
    use crate::workbench::{ExtensionOutputTabState, WorkbenchTabState};

    let mut app = AppState::default();
    let mut tab = ExtensionOutputTabState::new(7, "Ext", None, "mode", "cmd");
    tab.apply_output(
        vec![
            "very long wrapped extension output line".into(),
            "second long wrapped extension output line".into(),
        ],
        true,
        Some(0),
        None,
    );
    let logical_max = tab.lines.len().saturating_sub(1);
    app.workbench
        .open_tab(WorkbenchTabState::ExtensionOutput(tab));
    app.focus = Focus::Workbench;

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('G'))),
        AppAction::None
    );

    let scroll = match &app.workbench.active_tab().expect("tab").state {
        WorkbenchTabState::ExtensionOutput(tab) => tab.scroll,
        _ => panic!("expected extension output tab"),
    };
    assert!(scroll > logical_max);
}

#[test]
fn resource_events_shortcuts_do_not_clamp_to_logical_line_count() {
    use crate::k8s::events::EventInfo;
    use crate::timeline::TimelineEntry;
    use crate::workbench::{ResourceEventsTabState, WorkbenchTabState};

    let mut app = AppState::default();
    let mut tab = ResourceEventsTabState::new(ResourceRef::Pod("api-0".into(), "default".into()));
    tab.timeline = vec![
        TimelineEntry::Event {
            event: EventInfo {
                event_type: "Warning".into(),
                reason: "BackOff".into(),
                message: "very long wrapped event message".into(),
                first_timestamp: crate::time::now(),
                last_timestamp: crate::time::now(),
                count: 1,
            },
            correlated_action_idx: None,
        },
        TimelineEntry::Event {
            event: EventInfo {
                event_type: "Normal".into(),
                reason: "Pulled".into(),
                message: "another very long wrapped event message".into(),
                first_timestamp: crate::time::now(),
                last_timestamp: crate::time::now(),
                count: 1,
            },
            correlated_action_idx: None,
        },
        TimelineEntry::Event {
            event: EventInfo {
                event_type: "Normal".into(),
                reason: "Started".into(),
                message: "third wrapped event message".into(),
                first_timestamp: crate::time::now(),
                last_timestamp: crate::time::now(),
                count: 1,
            },
            correlated_action_idx: None,
        },
    ];
    let logical_max = tab.timeline.len().saturating_sub(1);
    app.workbench
        .open_tab(WorkbenchTabState::ResourceEvents(tab));
    app.focus = Focus::Workbench;

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('G'))),
        AppAction::None
    );

    let scroll = match &app.workbench.active_tab().expect("tab").state {
        WorkbenchTabState::ResourceEvents(tab) => tab.scroll,
        _ => panic!("expected resource events tab"),
    };
    assert!(scroll > logical_max);
}

#[test]
fn search_esc_resets_selected_idx() {
    let mut app = AppState::default();
    app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
    app.handle_key_event(KeyEvent::from(KeyCode::Char('x')));
    app.selected_idx = 5;
    app.handle_key_event(KeyEvent::from(KeyCode::Esc));
    assert_eq!(app.selected_idx, 0);
    assert!(app.search_query().is_empty());
}

#[test]
fn delete_confirm_accepts_lowercase_d() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".into(), "default".into())),
        yaml: Some("kind: Pod".into()),
        confirm_delete: true,
        ..DetailViewState::default()
    });
    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('d')));
    assert_eq!(action, AppAction::DeleteResource);
}

#[test]
fn ctrl_d_does_not_open_delete_confirmation_for_pod_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".into(), "default".into())),
        yaml: Some("kind: Pod".into()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    assert_eq!(action, AppAction::None);
    assert!(
        !app.detail_view
            .as_ref()
            .is_some_and(|detail| detail.confirm_delete),
        "ctrl+d should not arm delete"
    );
}

#[test]
fn ctrl_w_does_not_open_relationships_for_pod_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".into(), "default".into())),
        yaml: Some("kind: Pod".into()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    assert_eq!(action, AppAction::None);
}

#[test]
fn sync_workbench_focus_resets_when_tabs_empty() {
    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    app.sync_workbench_focus();
    assert_eq!(app.focus, Focus::Content);
}

#[test]
fn sync_workbench_focus_preserves_when_tabs_exist() {
    use crate::workbench::{ActionHistoryTabState, WorkbenchTabState};
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::ActionHistory(
        ActionHistoryTabState::default(),
    ));
    app.focus = Focus::Workbench;
    app.sync_workbench_focus();
    assert_eq!(app.focus, Focus::Workbench);
}

#[test]
fn pod_logs_search_mode_accepts_shortcut_characters_as_text() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-1".into(), "default".into())),
        ..DetailViewState::default()
    });
    app.open_logs_viewer();

    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state else {
        panic!("expected pod logs tab");
    };
    logs_tab.viewer.searching = true;
    logs_tab.viewer.search_input.clear();

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('g'))),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('f'))),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('t'))),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::PodLogs(logs_tab) = &tab.state else {
        panic!("expected pod logs tab");
    };
    assert_eq!(logs_tab.viewer.search_input, "gft");
}

#[test]
fn pod_logs_search_caps_at_input_limit() {
    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
        )));
    app.focus_workbench();

    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state else {
        panic!("expected pod logs tab");
    };
    logs_tab.viewer.searching = true;

    for _ in 0..(crate::app::input::MAX_LOG_SEARCH_INPUT_CHARS + 10) {
        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('x'))),
            AppAction::None
        );
    }

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::PodLogs(logs_tab) = &tab.state else {
        panic!("expected pod logs tab");
    };
    assert_eq!(
        logs_tab.viewer.search_input.chars().count(),
        crate::app::input::MAX_LOG_SEARCH_INPUT_CHARS
    );
    assert_eq!(
        logs_tab.viewer.search_cursor,
        crate::app::input::MAX_LOG_SEARCH_INPUT_CHARS
    );
}

#[test]
fn workbench_common_shortcuts_do_not_leak_into_pod_logs_search() {
    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
        )));
    app.focus_workbench();

    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state else {
        panic!("expected pod logs tab");
    };
    logs_tab.viewer.searching = true;

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('b'))),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('z'))),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::PodLogs(logs_tab) = &tab.state else {
        panic!("expected pod logs tab");
    };
    assert_eq!(logs_tab.viewer.search_input, "bz");
    assert!(app.workbench.open);
}

#[test]
fn pod_logs_search_ignores_alt_modified_chars() {
    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
        )));
    app.focus_workbench();

    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state else {
        panic!("expected pod logs tab");
    };
    logs_tab.viewer.searching = true;
    logs_tab.viewer.search_input = "api".into();

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT)),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::PodLogs(logs_tab) = &tab.state else {
        panic!("expected pod logs tab");
    };
    assert_eq!(logs_tab.viewer.search_input, "api");
}

#[test]
fn workbench_common_shortcuts_do_not_leak_into_exec_input() {
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::Exec(
        crate::workbench::ExecTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
            1,
            "pod-1".into(),
            "default".into(),
        ),
    ));
    app.focus_workbench();

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('b'))),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('z'))),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::Exec(exec_tab) = &tab.state else {
        panic!("expected exec tab");
    };
    assert_eq!(exec_tab.input, "bz");
    assert!(app.workbench.open);
}

#[test]
fn exec_escape_enters_command_mode_for_workbench_controls() {
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::Exec(
        crate::workbench::ExecTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
            1,
            "pod-1".into(),
            "default".into(),
        ),
    ));
    app.focus_workbench();

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('z'))),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Esc)),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('z'))),
        AppAction::WorkbenchToggleMaximize
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::Exec(exec_tab) = &tab.state else {
        panic!("expected exec tab");
    };
    assert_eq!(exec_tab.input, "z");
    assert!(exec_tab.command_mode);
}

#[test]
fn exec_command_mode_can_return_to_input_or_back_out() {
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::Exec(
        crate::workbench::ExecTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
            1,
            "pod-1".into(),
            "default".into(),
        ),
    ));
    app.focus_workbench();

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Esc)),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('i'))),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('z'))),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::Exec(exec_tab) = &tab.state else {
        panic!("expected exec tab");
    };
    assert_eq!(exec_tab.input, "z");
    assert!(!exec_tab.command_mode);

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Esc)),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Esc)),
        AppAction::EscapePressed
    );
}

#[test]
fn exec_page_down_scrolls_wrapped_visual_output() {
    let mut app = AppState::default();
    let mut tab = crate::workbench::ExecTabState::new(
        ResourceRef::Pod("pod-1".into(), "default".into()),
        1,
        "pod-1".into(),
        "default".into(),
    );
    tab.loading = false;
    tab.container_name = "main".into();
    tab.lines.push("one very long line".into());
    app.workbench.open_tab(WorkbenchTabState::Exec(tab));
    app.focus_workbench();

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::PageDown)),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::Exec(exec_tab) = &tab.state else {
        panic!("expected exec tab");
    };
    assert_eq!(exec_tab.scroll, 10);
}

#[test]
fn exec_input_ignores_alt_modified_chars() {
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::Exec(
        crate::workbench::ExecTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
            1,
            "pod-1".into(),
            "default".into(),
        ),
    ));
    app.focus_workbench();

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT)),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::Exec(exec_tab) = &tab.state else {
        panic!("expected exec tab");
    };
    assert!(exec_tab.input.is_empty());
}

#[test]
fn workbench_common_shortcuts_do_not_leak_into_connectivity_filter() {
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::Connectivity(
        crate::workbench::ConnectivityTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
            Vec::new(),
        ),
    ));
    app.focus_workbench();

    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::Connectivity(connectivity_tab) = &mut tab.state else {
        panic!("expected connectivity tab");
    };
    connectivity_tab.focus = crate::workbench::ConnectivityTabFocus::Filter;

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('b'))),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('z'))),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::Connectivity(connectivity_tab) = &tab.state else {
        panic!("expected connectivity tab");
    };
    assert_eq!(connectivity_tab.filter.value, "bz");
    assert!(app.workbench.open);
}

#[test]
fn connectivity_filter_ignores_alt_modified_chars() {
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::Connectivity(
        crate::workbench::ConnectivityTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
            Vec::new(),
        ),
    ));
    app.focus_workbench();

    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::Connectivity(connectivity_tab) = &mut tab.state else {
        panic!("expected connectivity tab");
    };
    connectivity_tab.focus = crate::workbench::ConnectivityTabFocus::Filter;
    connectivity_tab.filter.value = "api".into();
    connectivity_tab.filter.cursor_pos = 3;

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT)),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::Connectivity(connectivity_tab) = &tab.state else {
        panic!("expected connectivity tab");
    };
    assert_eq!(connectivity_tab.filter.value, "api");
    assert_eq!(connectivity_tab.filter.cursor_pos, 3);
}

#[test]
fn connectivity_filter_modified_edit_keys_do_not_mutate_filter_or_cursor() {
    for key in modified_edit_key_events() {
        let mut app = AppState::default();
        app.workbench.open_tab(WorkbenchTabState::Connectivity(
            crate::workbench::ConnectivityTabState::new(
                ResourceRef::Pod("pod-1".into(), "default".into()),
                Vec::new(),
            ),
        ));
        app.focus_workbench();

        let Some(tab) = app.workbench.active_tab_mut() else {
            panic!("expected active workbench tab");
        };
        let WorkbenchTabState::Connectivity(connectivity_tab) = &mut tab.state else {
            panic!("expected connectivity tab");
        };
        connectivity_tab.focus = crate::workbench::ConnectivityTabFocus::Filter;
        connectivity_tab.filter.value = "api".into();
        connectivity_tab.filter.cursor_pos = 2;

        assert_eq!(app.handle_key_event(key), AppAction::None, "{key:?}");

        let WorkbenchTabState::Connectivity(connectivity_tab) =
            &app.workbench.active_tab().unwrap().state
        else {
            panic!("expected connectivity tab");
        };
        assert_eq!(connectivity_tab.filter.value, "api", "{key:?}");
        assert_eq!(connectivity_tab.filter.cursor_pos, 2, "{key:?}");
    }
}

#[test]
fn connectivity_filter_delete_removes_character_at_cursor() {
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::Connectivity(
        crate::workbench::ConnectivityTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
            Vec::new(),
        ),
    ));
    app.focus_workbench();

    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::Connectivity(connectivity_tab) = &mut tab.state else {
        panic!("expected connectivity tab");
    };
    connectivity_tab.focus = crate::workbench::ConnectivityTabFocus::Filter;
    connectivity_tab.filter.value = "abcd".into();
    connectivity_tab.filter.cursor_pos = 1;

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Delete)),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::Connectivity(connectivity_tab) = &tab.state else {
        panic!("expected connectivity tab");
    };
    assert_eq!(connectivity_tab.filter.value, "acd");
    assert_eq!(connectivity_tab.filter.cursor_pos, 1);
}

#[test]
fn workload_logs_filter_mode_supports_ctrl_u_clear() {
    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::WorkloadLogs(WorkloadLogsTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
            1,
        )));
    app.focus_workbench();

    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::WorkloadLogs(logs_tab) = &mut tab.state else {
        panic!("expected workload logs tab");
    };
    logs_tab.editing_text_filter = true;
    logs_tab.filter_input = "error".into();

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::WorkloadLogs(logs_tab) = &tab.state else {
        panic!("expected workload logs tab");
    };
    assert!(logs_tab.filter_input.is_empty());
}

#[test]
fn workload_logs_filter_ignores_alt_modified_chars() {
    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::WorkloadLogs(WorkloadLogsTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
            1,
        )));
    app.focus_workbench();

    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::WorkloadLogs(logs_tab) = &mut tab.state else {
        panic!("expected workload logs tab");
    };
    logs_tab.editing_text_filter = true;
    logs_tab.filter_input = "error".into();
    logs_tab.filter_input_cursor = logs_tab.filter_input.chars().count();

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT)),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::WorkloadLogs(logs_tab) = &tab.state else {
        panic!("expected workload logs tab");
    };
    assert_eq!(logs_tab.filter_input, "error");
    assert_eq!(logs_tab.filter_input_cursor, "error".len());
}

#[test]
fn pod_logs_search_supports_cursor_editing() {
    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
        )));
    app.focus_workbench();

    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state else {
        panic!("expected pod logs tab");
    };
    logs_tab.viewer.searching = true;
    logs_tab.viewer.search_input = "abcd".into();
    logs_tab.viewer.search_cursor = 2;

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Left)),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('x'))),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Delete)),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::PodLogs(logs_tab) = &tab.state else {
        panic!("expected pod logs tab");
    };
    assert_eq!(logs_tab.viewer.search_input, "axcd");
    assert_eq!(logs_tab.viewer.search_cursor, 2);
}

#[test]
fn workload_logs_filter_supports_cursor_editing() {
    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::WorkloadLogs(WorkloadLogsTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
            1,
        )));
    app.focus_workbench();

    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::WorkloadLogs(logs_tab) = &mut tab.state else {
        panic!("expected workload logs tab");
    };
    logs_tab.editing_text_filter = true;
    logs_tab.filter_input = "abcd".into();
    logs_tab.filter_input_cursor = 2;

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Left)),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('x'))),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Delete)),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::WorkloadLogs(logs_tab) = &tab.state else {
        panic!("expected workload logs tab");
    };
    assert_eq!(logs_tab.filter_input, "axcd");
    assert_eq!(logs_tab.filter_input_cursor, 2);
}

#[test]
fn exec_input_supports_cursor_editing() {
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::Exec(
        crate::workbench::ExecTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
            1,
            "pod-1".into(),
            "default".into(),
        ),
    ));
    app.focus_workbench();

    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::Exec(exec_tab) = &mut tab.state else {
        panic!("expected exec tab");
    };
    exec_tab.input = "abcd".into();
    exec_tab.input_cursor = 2;

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Left)),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('x'))),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Delete)),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::Exec(exec_tab) = &tab.state else {
        panic!("expected exec tab");
    };
    assert_eq!(exec_tab.input, "axcd");
    assert_eq!(exec_tab.input_cursor, 2);
}

#[test]
fn exec_input_routes_history_and_clear_output_shortcuts() {
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::Exec(
        crate::workbench::ExecTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
            1,
            "pod-1".into(),
            "default".into(),
        ),
    ));
    app.focus_workbench();

    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::Exec(exec_tab) = &mut tab.state else {
        panic!("expected exec tab");
    };
    exec_tab.record_command_history("echo ready");
    exec_tab.append_output("old output\n");

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Up)),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL)),
        AppAction::ExecClearOutput
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::Exec(exec_tab) = &tab.state else {
        panic!("expected exec tab");
    };
    assert_eq!(exec_tab.input, "echo ready");
}

#[test]
fn context_picker_takes_precedence_over_global_context_shortcut() {
    let mut app = AppState::default();
    app.context_picker.open();

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('c'))),
        AppAction::None
    );

    assert!(app.context_picker.is_open());
    assert_eq!(app.context_picker.search_query(), "c");
}

#[test]
fn namespace_picker_takes_precedence_over_global_context_shortcut() {
    let mut app = AppState::default();
    app.namespace_picker.open();

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('c'))),
        AppAction::None
    );

    assert!(app.namespace_picker.is_open());
    assert_eq!(app.namespace_picker.search_query(), "c");
}

#[test]
fn ctrl_c_does_not_open_context_picker() {
    let mut app = AppState::default();

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        AppAction::None
    );
    assert!(!app.context_picker.is_open());
}

#[test]
fn command_palette_takes_precedence_over_help_shortcut() {
    let mut app = AppState::default();
    app.command_palette.open();

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT)),
        AppAction::None
    );

    assert!(app.command_palette.is_open());
    assert!(!app.help_overlay.is_open());
}

#[test]
fn workbench_focus_supports_help_overlay_shortcut() {
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::ActionHistory(
        crate::workbench::ActionHistoryTabState::default(),
    ));
    app.focus_workbench();

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT)),
        AppAction::OpenHelp
    );
}

#[test]
fn help_overlay_page_keys_scroll_overlay() {
    let mut app = AppState::default();
    app.help_overlay.open();
    app.help_overlay.scroll_down();
    app.help_overlay.scroll_down();

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::PageDown)),
        AppAction::None
    );
    assert!(app.help_overlay.is_open());
    assert!(app.help_overlay.scroll() > 2);

    let scrolled = app.help_overlay.scroll();
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::PageUp)),
        AppAction::None
    );
    assert!(app.help_overlay.scroll() < scrolled);
}

#[test]
fn help_overlay_modified_keys_do_not_close_or_scroll() {
    for (code, modifiers) in [
        (KeyCode::Esc, KeyModifiers::CONTROL),
        (KeyCode::Esc, KeyModifiers::ALT),
        (
            KeyCode::Char('?'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        (KeyCode::PageDown, KeyModifiers::CONTROL),
        (KeyCode::PageUp, KeyModifiers::ALT),
        (KeyCode::Char('j'), KeyModifiers::CONTROL),
        (KeyCode::Char('k'), KeyModifiers::CONTROL),
        (KeyCode::Down, KeyModifiers::CONTROL),
        (KeyCode::Up, KeyModifiers::ALT),
    ] {
        let mut app = AppState::default();
        app.help_overlay.open();
        app.help_overlay.scroll_down();
        let scroll = app.help_overlay.scroll();

        assert_eq!(
            app.handle_key_event(KeyEvent::new(code, modifiers)),
            AppAction::None,
            "{code:?} {modifiers:?}"
        );
        assert!(app.help_overlay.is_open(), "{code:?} {modifiers:?}");
        assert_eq!(app.help_overlay.scroll(), scroll, "{code:?} {modifiers:?}");
    }
}

#[test]
fn content_detail_page_keys_scroll_secondary_panes_without_moving_selection() {
    for view in [
        AppView::Dashboard,
        AppView::Projects,
        AppView::Governance,
        AppView::Roles,
        AppView::RoleBindings,
        AppView::ClusterRoles,
        AppView::ClusterRoleBindings,
        AppView::FluxCDKustomizations,
    ] {
        let mut app = AppState::default();
        app.view = view;
        app.focus = Focus::Content;
        app.selected_idx = 3;

        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::PageDown)),
            AppAction::None,
            "{view:?}"
        );
        assert_eq!(app.content_detail_scroll, 10, "{view:?}");
        assert_eq!(app.selected_idx, 3, "{view:?}");

        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::PageUp)),
            AppAction::None,
            "{view:?}"
        );
        assert_eq!(app.content_detail_scroll, 0, "{view:?}");
        assert_eq!(app.selected_idx, 3, "{view:?}");

        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL)),
            AppAction::None,
            "{view:?}"
        );
        assert_eq!(app.content_detail_scroll, 10, "{view:?}");
        assert_eq!(app.selected_idx, 3, "{view:?}");

        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)),
            AppAction::None,
            "{view:?}"
        );
        assert_eq!(app.content_detail_scroll, 0, "{view:?}");
        assert_eq!(app.selected_idx, 3, "{view:?}");

        assert_eq!(
            app.handle_key_event(KeyEvent::new(
                KeyCode::Char('F'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            AppAction::None,
            "{view:?}"
        );
        assert_eq!(app.content_detail_scroll, 10, "{view:?}");
        assert_eq!(app.selected_idx, 3, "{view:?}");

        assert_eq!(
            app.handle_key_event(KeyEvent::new(
                KeyCode::Char('B'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            AppAction::None,
            "{view:?}"
        );
        assert_eq!(app.content_detail_scroll, 0, "{view:?}");
        assert_eq!(app.selected_idx, 3, "{view:?}");

        assert_eq!(
            app.handle_key_event(KeyEvent::new(
                KeyCode::Char('D'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            AppAction::None,
            "{view:?}"
        );
        assert_eq!(app.content_detail_scroll, 10, "{view:?}");
        assert_eq!(app.selected_idx, 3, "{view:?}");

        assert_eq!(
            app.handle_key_event(KeyEvent::new(
                KeyCode::Char('U'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            AppAction::None,
            "{view:?}"
        );
        assert_eq!(app.content_detail_scroll, 0, "{view:?}");
        assert_eq!(app.selected_idx, 3, "{view:?}");
    }
}

#[test]
fn ctrl_alt_content_detail_scroll_shortcuts_do_not_scroll() {
    let mut app = AppState::default();
    app.view = AppView::Governance;
    app.focus = Focus::Content;
    app.selected_idx = 3;

    for code in [
        KeyCode::Char('f'),
        KeyCode::Char('b'),
        KeyCode::Char('d'),
        KeyCode::Char('u'),
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::PageDown,
        KeyCode::PageUp,
    ] {
        assert_eq!(
            app.handle_key_event(KeyEvent::new(
                code,
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )),
            AppAction::None,
            "{code:?}"
        );
        assert_eq!(app.content_detail_scroll, 0, "{code:?}");
        assert_eq!(app.selected_idx, 3, "{code:?}");
    }
}

#[test]
fn secondary_pane_focus_routes_plain_navigation_to_scrollable_detail_pane() {
    let mut app = AppState::default();
    app.view = AppView::Governance;
    app.focus = Focus::Content;
    app.selected_idx = 3;

    assert_eq!(app.content_pane_focus(), ContentPaneFocus::List);
    assert!(!app.content_secondary_pane_active());

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char(';'))),
        AppAction::None
    );
    assert_eq!(app.content_pane_focus(), ContentPaneFocus::Secondary);
    assert!(app.content_secondary_pane_active());

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('j'))),
        AppAction::None
    );
    assert_eq!(app.content_detail_scroll, 1);
    assert_eq!(app.selected_idx, 3);

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('d'))),
        AppAction::None
    );
    assert_eq!(app.content_detail_scroll, 11);
    assert_eq!(app.selected_idx, 3);

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('u'))),
        AppAction::None
    );
    assert_eq!(app.content_detail_scroll, 1);

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('k'))),
        AppAction::None
    );
    assert_eq!(app.content_detail_scroll, 0);
    assert_eq!(app.selected_idx, 3);

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char(';'))),
        AppAction::None
    );
    assert_eq!(app.content_pane_focus(), ContentPaneFocus::List);

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('j'))),
        AppAction::None
    );
    assert_eq!(app.selected_idx, 4);
}

#[test]
fn secondary_pane_focus_is_scoped_to_supported_split_views() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
    app.focus = Focus::Content;

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char(';'))),
        AppAction::None
    );
    assert_eq!(app.content_pane_focus(), ContentPaneFocus::List);
    assert!(!app.content_secondary_pane_active());

    app.view = AppView::FluxCDKustomizations;
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char(';'))),
        AppAction::None
    );
    assert_eq!(app.content_pane_focus(), ContentPaneFocus::Secondary);

    app.navigate_to_view(AppView::Projects);
    assert_eq!(app.content_pane_focus(), ContentPaneFocus::List);
    assert!(!app.content_secondary_pane_active());
}

#[test]
fn ctrl_b_and_ctrl_f_do_not_trigger_unrelated_content_actions() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
    app.focus = Focus::Content;

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL)),
        AppAction::None
    );

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)),
        AppAction::None
    );
    assert!(!app.workbench.open);
}

#[test]
fn workbench_focus_supports_command_palette_shortcut() {
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::ActionHistory(
        crate::workbench::ActionHistoryTabState::default(),
    ));
    app.focus_workbench();

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char(':'))),
        AppAction::OpenCommandPalette
    );
}

#[test]
fn ctrl_z_does_not_toggle_workbench_maximize() {
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::ActionHistory(
        crate::workbench::ActionHistoryTabState::default(),
    ));
    app.focus_workbench();

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)),
        AppAction::None
    );
    assert!(!app.workbench.maximized);

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('z'))),
        AppAction::WorkbenchToggleMaximize
    );
}

#[test]
fn modified_plain_workbench_shortcuts_do_not_fire() {
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::ActionHistory(
        crate::workbench::ActionHistoryTabState::default(),
    ));
    app.focus_workbench();
    if let Some(tab) = app.workbench.active_tab_mut()
        && let WorkbenchTabState::ActionHistory(tab) = &mut tab.state
    {
        tab.selected = 5;
    }

    for (code, modifiers) in [
        (KeyCode::Char('j'), KeyModifiers::CONTROL),
        (KeyCode::Char('k'), KeyModifiers::CONTROL),
        (KeyCode::Char('g'), KeyModifiers::CONTROL),
        (
            KeyCode::Char('G'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        (KeyCode::Char('z'), KeyModifiers::CONTROL),
        (KeyCode::Char('b'), KeyModifiers::CONTROL),
        (KeyCode::Char('['), KeyModifiers::CONTROL),
        (KeyCode::Char(']'), KeyModifiers::CONTROL),
        (
            KeyCode::Char(':'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        (
            KeyCode::Char('?'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        (
            KeyCode::Char('~'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
    ] {
        assert_eq!(
            app.handle_key_event(KeyEvent::new(code, modifiers)),
            AppAction::None,
            "{code:?} {modifiers:?}"
        );
    }

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::ActionHistory(tab) = &tab.state else {
        panic!("expected action history tab");
    };
    assert_eq!(tab.selected, 5);
    assert!(app.workbench.open);
    assert!(!app.workbench.maximized);
    assert!(!app.command_palette.is_open());
    assert!(!app.help_overlay.is_open());
    assert!(!app.namespace_picker.is_open());
}

#[test]
fn ctrl_brackets_do_not_switch_workbench_tabs_from_content_focus() {
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::ActionHistory(
        crate::workbench::ActionHistoryTabState::default(),
    ));
    app.workbench
        .ensure_background_tab(WorkbenchTabState::ActionHistory(
            crate::workbench::ActionHistoryTabState::default(),
        ));
    app.focus = Focus::Content;
    let active_before = app.workbench.active_tab;

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('['), KeyModifiers::CONTROL)),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL)),
        AppAction::None
    );
    assert_eq!(app.workbench.active_tab, active_before);
}

#[test]
fn workbench_local_editor_keeps_help_and_palette_shortcuts_as_text() {
    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
        )));
    app.focus_workbench();

    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state else {
        panic!("expected pod logs tab");
    };
    logs_tab.viewer.searching = true;

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char(':'))),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT)),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::PodLogs(logs_tab) = &tab.state else {
        panic!("expected pod logs tab");
    };
    assert_eq!(logs_tab.viewer.search_input, ":?");
    assert!(!app.help_overlay.is_open());
}

#[test]
fn pod_logs_shortcuts_toggle_regex_and_structured_view() {
    use crate::events::input::apply_action;
    use crate::log_investigation::LogQueryMode;

    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
        )));
    app.focus_workbench();

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('r'))),
        AppAction::RefreshData
    );
    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('R')));
    assert_eq!(action, AppAction::ToggleLogRegexMode);
    assert!(apply_action(action, &mut app));
    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('W')));
    assert_eq!(action, AppAction::ToggleLogTimeWindow);
    assert!(apply_action(action, &mut app));
    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('T')));
    assert_eq!(action, AppAction::OpenLogTimeJump);
    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('C')));
    assert_eq!(action, AppAction::ToggleLogCorrelation);
    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('J')));
    assert_eq!(action, AppAction::ToggleStructuredLogView);
    assert!(apply_action(action, &mut app));

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::PodLogs(logs_tab) = &tab.state else {
        panic!("expected pod logs tab");
    };
    assert_eq!(logs_tab.viewer.search_mode, LogQueryMode::Regex);
    assert_eq!(
        logs_tab.viewer.time_window,
        crate::log_investigation::LogTimeWindow::Last5Minutes
    );
    assert!(!logs_tab.viewer.structured_view);
}

#[test]
fn pod_logs_control_modified_plain_shortcuts_do_not_fire() {
    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
        )));
    app.focus_workbench();

    for key in ['T', 'C', 'J', 'S', 'M', 'y', 'g', 'G', '[', ']'] {
        assert_eq!(
            app.handle_key_event(KeyEvent::new(
                KeyCode::Char(key),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            )),
            AppAction::None,
            "{key}"
        );
    }
    assert_eq!(
        app.handle_key_event(KeyEvent::new(
            KeyCode::Char('R'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        )),
        AppAction::RefreshData
    );
}

#[test]
fn workload_logs_shortcuts_toggle_regex_and_structured_view() {
    use crate::events::input::apply_action;
    use crate::log_investigation::LogQueryMode;

    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::WorkloadLogs(WorkloadLogsTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
            1,
        )));
    app.focus_workbench();

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('r'))),
        AppAction::RefreshData
    );
    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('R')));
    assert_eq!(action, AppAction::ToggleLogRegexMode);
    assert!(apply_action(action, &mut app));
    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('W')));
    assert_eq!(action, AppAction::ToggleLogTimeWindow);
    assert!(apply_action(action, &mut app));
    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('T')));
    assert_eq!(action, AppAction::OpenLogTimeJump);
    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('L')));
    assert_eq!(action, AppAction::CycleWorkloadLogLabelFilter);
    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('C')));
    assert_eq!(action, AppAction::ToggleLogCorrelation);
    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('J')));
    assert_eq!(action, AppAction::ToggleStructuredLogView);
    assert!(apply_action(action, &mut app));

    let Some(tab) = app.workbench.active_tab() else {
        panic!("expected active workbench tab");
    };
    let WorkbenchTabState::WorkloadLogs(logs_tab) = &tab.state else {
        panic!("expected workload logs tab");
    };
    assert_eq!(logs_tab.text_filter_mode, LogQueryMode::Regex);
    assert_eq!(
        logs_tab.time_window,
        crate::log_investigation::LogTimeWindow::Last5Minutes
    );
    assert!(!logs_tab.structured_view);
}

#[test]
fn workload_logs_control_modified_plain_shortcuts_do_not_fire() {
    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::WorkloadLogs(WorkloadLogsTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
            1,
        )));
    app.focus_workbench();

    for key in [
        'T', 'L', 'C', 'J', 'S', 'M', 'y', 'p', 'c', 'g', 'G', '[', ']',
    ] {
        assert_eq!(
            app.handle_key_event(KeyEvent::new(
                KeyCode::Char(key),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            )),
            AppAction::None,
            "{key}"
        );
    }
    assert_eq!(
        app.handle_key_event(KeyEvent::new(
            KeyCode::Char('R'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        )),
        AppAction::RefreshData
    );
}

#[test]
fn pod_logs_shortcuts_route_saved_preset_actions() {
    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
        )));
    app.focus_workbench();

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('M'))),
        AppAction::SaveLogPreset
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('m'))),
        AppAction::SaveLogPreset
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('S'))),
        AppAction::ExportLogs
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('s'))),
        AppAction::ExportLogs
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('['))),
        AppAction::ApplyPreviousLogPreset
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char(']'))),
        AppAction::ApplyNextLogPreset
    );
}

#[test]
fn workload_logs_shortcuts_route_saved_preset_actions() {
    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::WorkloadLogs(WorkloadLogsTabState::new(
            ResourceRef::Pod("pod-1".into(), "default".into()),
            1,
        )));
    app.focus_workbench();

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('M'))),
        AppAction::SaveLogPreset
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('m'))),
        AppAction::SaveLogPreset
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('S'))),
        AppAction::ExportLogs
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('s'))),
        AppAction::ExportLogs
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('['))),
        AppAction::ApplyPreviousLogPreset
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char(']'))),
        AppAction::ApplyNextLogPreset
    );
}

#[test]
fn filtered_pod_indices_apply_restarts_sort_with_stable_tie_breakers() {
    let mut pods = vec![
        PodInfo {
            name: "zeta".to_string(),
            namespace: "prod".to_string(),
            status: "Running".to_string(),
            restarts: 2,
            ..PodInfo::default()
        },
        PodInfo {
            name: "alpha".to_string(),
            namespace: "dev".to_string(),
            status: "Pending".to_string(),
            restarts: 2,
            ..PodInfo::default()
        },
        PodInfo {
            name: "beta".to_string(),
            namespace: "prod".to_string(),
            status: "Running".to_string(),
            restarts: 5,
            ..PodInfo::default()
        },
    ];
    // Ensure deterministic age field ordering is not involved in this test.
    for pod in &mut pods {
        pod.created_at = None;
    }

    let sorted = filtered_pod_indices(
        &pods,
        "",
        Some(PodSortState::new(PodSortColumn::Restarts, true)),
    );

    // Highest restarts first, then namespace/name tie-breakers for equal restart count.
    assert_eq!(sorted, vec![2, 1, 0]);
}

#[test]
fn filtered_workload_indices_apply_age_sort_with_name_tie_breaker() {
    #[derive(Clone)]
    struct Item {
        name: String,
        namespace: String,
        age: Option<std::time::Duration>,
    }

    let items = vec![
        Item {
            name: "zeta".to_string(),
            namespace: "prod".to_string(),
            age: Some(std::time::Duration::from_secs(60)),
        },
        Item {
            name: "alpha".to_string(),
            namespace: "dev".to_string(),
            age: Some(std::time::Duration::from_secs(60)),
        },
        Item {
            name: "beta".to_string(),
            namespace: "prod".to_string(),
            age: Some(std::time::Duration::from_secs(120)),
        },
    ];

    let sorted = filtered_workload_indices(
        &items,
        "",
        Some(WorkloadSortState::new(WorkloadSortColumn::Age, true)),
        |item, _| !item.name.is_empty(),
        |item| item.name.as_str(),
        |item| item.namespace.as_str(),
        |item| item.age,
    );

    assert_eq!(sorted, vec![2, 1, 0]);
}

/// Verifies namespace persistence round-trip via config helpers.
#[test]
fn test_namespace_persistence() {
    use crate::workbench::{ActionHistoryTabState, WorkbenchTabState};

    let _icon_mode_lock = crate::icons::icon_mode_test_lock();
    let path =
        std::env::temp_dir().join(format!("kubectui-config-test-{}.json", std::process::id()));

    let mut app = AppState::default();
    app.set_namespace("demo".to_string());
    app.workbench
        .ensure_background_tab(WorkbenchTabState::ActionHistory(
            ActionHistoryTabState::default(),
        ));
    app.toggle_workbench();
    app.workbench.height = 15;
    save_config_to_path(&app, &path);

    let loaded = load_config_from_path(&path);
    assert_eq!(loaded.get_namespace(), "demo");
    assert!(!loaded.workbench.open);
    assert_eq!(loaded.workbench.height, 15);

    let _ = std::fs::remove_file(path);
}

#[test]
fn save_config_skips_write_when_parent_is_not_directory() {
    let marker = std::env::temp_dir().join(format!(
        "kubectui-config-parent-file-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&marker);
    std::fs::write(&marker, "sentinel").expect("marker file");
    let path = marker.join("kubectui-config.json");

    let mut app = AppState::default();
    app.set_namespace("demo".to_string());
    save_config_to_path(&app, &path);

    assert!(!path.exists());
    assert_eq!(
        std::fs::read_to_string(&marker).expect("marker contents"),
        "sentinel"
    );

    let _ = std::fs::remove_file(marker);
}

/// Verifies quit requires confirmation: first Esc sets confirm_quit, Enter quits.
#[test]
fn quit_action_sets_should_quit() {
    let mut app = AppState::default();

    let action = app.handle_key_event(KeyEvent::from(KeyCode::Esc));
    assert_eq!(action, AppAction::None);
    assert!(app.confirm_quit);
    assert!(!app.should_quit());

    let action = app.handle_key_event(KeyEvent::from(KeyCode::Enter));
    assert_eq!(action, AppAction::Quit);
    assert!(app.should_quit());
}

#[test]
fn quit_requires_esc_then_enter_only() {
    for key in [KeyCode::Char('q'), KeyCode::Char('y'), KeyCode::Esc] {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Esc));
        assert!(app.confirm_quit);

        let action = app.handle_key_event(KeyEvent::from(key));
        assert_eq!(action, AppAction::None);
        assert!(!app.should_quit());
        assert!(!app.confirm_quit);
    }
}

#[test]
fn modified_enter_does_not_confirm_quit() {
    for modifiers in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Esc));
        assert!(app.confirm_quit);

        let action = app.handle_key_event(KeyEvent::new(KeyCode::Enter, modifiers));
        assert_eq!(action, AppAction::None);
        assert!(!app.should_quit());
        assert!(!app.confirm_quit);
    }
}

#[test]
fn modified_enter_does_not_open_detail_child_resource() {
    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::META,
        KeyModifiers::SUPER,
    ] {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::CronJob("cron-0".to_string(), "ns".to_string())),
            cronjob_history: vec![CronJobHistoryEntry {
                job_name: "cron-0-123".to_string(),
                namespace: "ns".to_string(),
                status: "Succeeded".to_string(),
                completions: "1/1".to_string(),
                duration: Some("1s".to_string()),
                pod_count: 1,
                live_pod_count: 0,
                completion_pct: Some(100),
                active_pods: 0,
                failed_pods: 0,
                age: None,
                created_at: None,
                logs_authorized: None,
            }],
            ..DetailViewState::default()
        });

        let action = app.handle_key_event(KeyEvent::new(KeyCode::Enter, modifiers));

        assert_eq!(action, AppAction::None, "{modifiers:?}");
    }
}

#[test]
fn modified_esc_does_not_start_quit_confirmation() {
    for modifiers in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
        let mut app = AppState::default();
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers));
        assert_eq!(action, AppAction::None);
        assert!(!app.confirm_quit);
        assert!(!app.should_quit());

        let action = app.handle_key_event(KeyEvent::from(KeyCode::Enter));
        assert_ne!(action, AppAction::Quit);
        assert!(!app.should_quit());
    }
}

#[test]
fn modified_esc_does_not_close_detail_or_move_focus() {
    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::META,
        KeyModifiers::SUPER,
        KeyModifiers::CONTROL | KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::SUPER,
    ] {
        let mut detail_app = AppState {
            focus: Focus::Content,
            detail_view: Some(DetailViewState {
                resource: Some(ResourceRef::Pod("pod-a".to_string(), "default".to_string())),
                ..DetailViewState::default()
            }),
            ..AppState::default()
        };
        assert_eq!(
            detail_app.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );
        assert!(detail_app.detail_view.is_some(), "{modifiers:?}");

        let mut content_app = AppState {
            focus: Focus::Content,
            ..AppState::default()
        };
        assert_eq!(
            content_app.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );
        assert_eq!(content_app.focus, Focus::Content, "{modifiers:?}");

        let mut workbench_app = AppState::default();
        workbench_app
            .workbench
            .open_tab(WorkbenchTabState::ActionHistory(
                crate::workbench::ActionHistoryTabState::default(),
            ));
        workbench_app.focus_workbench();
        assert_eq!(
            workbench_app.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );
        assert_eq!(workbench_app.focus, Focus::Workbench, "{modifiers:?}");
    }
}

#[test]
fn q_does_not_start_quit_confirmation() {
    let mut app = AppState::default();

    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('q')));
    assert_eq!(action, AppAction::None);
    assert!(!app.confirm_quit);
    assert!(!app.should_quit());
}

/// Verifies any other key cancels the quit confirmation.
#[test]
fn quit_confirm_cancelled_by_other_key() {
    let mut app = AppState::default();
    app.handle_key_event(KeyEvent::from(KeyCode::Esc));
    assert!(app.confirm_quit);

    app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
    assert!(!app.confirm_quit);
    assert!(!app.should_quit());
}

/// Verifies Esc closes detail view before requesting app quit.
#[test]
fn esc_closes_detail_before_quit() {
    let mut app = AppState {
        detail_view: Some(DetailViewState::default()),
        ..AppState::default()
    };

    let action = app.handle_key_event(KeyEvent::from(KeyCode::Esc));

    assert_eq!(action, AppAction::CloseDetail);
    assert!(!app.should_quit());
}

/// Verifies selection index saturates at zero when moving up.
#[test]
fn selected_index_never_underflows() {
    let mut app = AppState::default();
    app.handle_key_event(KeyEvent::from(KeyCode::Up));
    assert_eq!(app.selected_idx(), 0);
}

/// Verifies j/k move the sidebar cursor (not selected_idx) when no detail view.
#[test]
fn selected_index_grows_with_down_events() {
    let mut app = AppState::default();
    for _ in 0..5 {
        app.handle_key_event(KeyEvent::from(KeyCode::Down));
    }
    assert_eq!(app.sidebar_cursor, 5);
}

/// Verifies selection resets to zero when switching tabs.
#[test]
fn view_switch_resets_selection_index() {
    let mut app = AppState::default();
    app.selected_idx = 2;
    assert_eq!(app.selected_idx(), 2);

    app.handle_key_event(KeyEvent::from(KeyCode::Tab));

    assert_eq!(app.selected_idx(), 0);
}

#[test]
fn sidebar_cursor_expands_only_highlighted_group() {
    let mut app = AppState::default();

    while sidebar_rows(&app.collapsed_groups)[app.sidebar_cursor]
        != SidebarItem::Group(NavGroup::Workloads)
    {
        app.sidebar_cursor_down();
    }

    assert!(!app.collapsed_groups.contains(&NavGroup::Workloads));
    assert!(app.collapsed_groups.contains(&NavGroup::Network));
    assert!(sidebar_rows(&app.collapsed_groups).contains(&SidebarItem::View(AppView::Pods)));
    assert!(!sidebar_rows(&app.collapsed_groups).contains(&SidebarItem::View(AppView::Services)));

    while sidebar_rows(&app.collapsed_groups)[app.sidebar_cursor]
        != SidebarItem::Group(NavGroup::Network)
    {
        app.sidebar_cursor_down();
    }

    assert!(app.collapsed_groups.contains(&NavGroup::Workloads));
    assert!(!app.collapsed_groups.contains(&NavGroup::Network));
    assert!(!sidebar_rows(&app.collapsed_groups).contains(&SidebarItem::View(AppView::Pods)));
    assert!(sidebar_rows(&app.collapsed_groups).contains(&SidebarItem::View(AppView::Services)));
}

#[test]
fn tab_navigation_expands_only_active_view_group() {
    let mut app = AppState::default();

    for _ in 0..10 {
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
    }

    assert_eq!(app.view(), AppView::Pods);
    assert!(!app.collapsed_groups.contains(&NavGroup::Workloads));
    assert!(app.collapsed_groups.contains(&NavGroup::Network));

    for _ in 0..8 {
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
    }

    assert_eq!(app.view(), AppView::Services);
    assert!(app.collapsed_groups.contains(&NavGroup::Workloads));
    assert!(!app.collapsed_groups.contains(&NavGroup::Network));
}

#[test]
fn overview_views_are_top_level_not_grouped() {
    let rows = sidebar_rows(&AppState::default().collapsed_groups);

    assert_eq!(rows[0], SidebarItem::View(AppView::Dashboard));
    assert_eq!(rows[1], SidebarItem::View(AppView::Projects));
    assert_eq!(rows[9], SidebarItem::View(AppView::Events));
    assert_eq!(rows[10], SidebarItem::Group(NavGroup::Workloads));
    assert!(!rows.contains(&SidebarItem::Group(NavGroup::Overview)));
}

/// Verifies rapid tab switching remains stable.
#[test]
fn rapid_tab_switching_is_stable() {
    let mut app = AppState::default();

    for _ in 0..(AppView::tabs().len() * 3) {
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
    }

    assert_eq!(app.view(), AppView::Dashboard);
}

/// Verifies search input ignores modified characters except supported shortcuts.
#[test]
fn search_input_ignores_modified_characters() {
    let mut app = AppState::default();
    app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));

    app.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
    app.handle_key_event(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT));

    assert_eq!(app.search_query(), "");
}

/// Verifies error message can be set and cleared.
#[test]
fn error_message_set_and_clear() {
    let mut app = AppState::default();
    app.set_error("boom".to_string());
    assert_eq!(app.error_message(), Some("boom"));

    app.clear_error();
    assert_eq!(app.error_message(), None);
}

#[test]
fn status_message_set_and_clear() {
    let mut app = AppState::default();
    app.set_status("working".to_string());
    assert_eq!(app.status_message(), Some("working"));
    assert_eq!(app.error_message(), None);

    app.clear_status();
    assert_eq!(app.status_message(), None);
}

/// Verifies resource reference helper methods return expected kind/name/namespace.
#[test]
fn resource_ref_helpers_work_for_each_variant() {
    let node = ResourceRef::Node("n1".to_string());
    let pod = ResourceRef::Pod("p1".to_string(), "ns1".to_string());
    let statefulset = ResourceRef::StatefulSet("ss1".to_string(), "ns1".to_string());
    let quota = ResourceRef::ResourceQuota("rq1".to_string(), "ns1".to_string());
    let daemonset = ResourceRef::DaemonSet("ds1".to_string(), "ns1".to_string());
    let pv = ResourceRef::Pv("pv1".to_string());
    let cluster_role = ResourceRef::ClusterRole("cr1".to_string());

    assert_eq!(node.kind(), "Node");
    assert_eq!(node.name(), "n1");
    assert_eq!(node.namespace(), None);

    assert_eq!(pod.kind(), "Pod");
    assert_eq!(pod.name(), "p1");
    assert_eq!(pod.namespace(), Some("ns1"));

    assert_eq!(statefulset.kind(), "StatefulSet");
    assert_eq!(statefulset.name(), "ss1");
    assert_eq!(statefulset.namespace(), Some("ns1"));

    assert_eq!(quota.kind(), "ResourceQuota");
    assert_eq!(quota.name(), "rq1");
    assert_eq!(quota.namespace(), Some("ns1"));

    assert_eq!(daemonset.kind(), "DaemonSet");
    assert_eq!(daemonset.name(), "ds1");
    assert_eq!(daemonset.namespace(), Some("ns1"));

    assert_eq!(pv.kind(), "PersistentVolume");
    assert_eq!(pv.name(), "pv1");
    assert_eq!(pv.namespace(), None);

    assert_eq!(cluster_role.kind(), "ClusterRole");
    assert_eq!(cluster_role.name(), "cr1");
    assert_eq!(cluster_role.namespace(), None);

    let helm = ResourceRef::HelmRelease("my-release".to_string(), "default".to_string());
    assert_eq!(helm.kind(), "HelmRelease");
    assert_eq!(helm.name(), "my-release");
    assert_eq!(helm.namespace(), Some("default"));

    let cr = ResourceRef::CustomResource {
        name: "my-widget".to_string(),
        namespace: Some("prod".to_string()),
        group: "demo.io".to_string(),
        version: "v1".to_string(),
        kind: "Widget".to_string(),
        plural: "widgets".to_string(),
    };
    assert_eq!(cr.kind(), "Widget");
    assert_eq!(cr.name(), "my-widget");
    assert_eq!(cr.namespace(), Some("prod"));
    assert_eq!(cr.primary_view(), Some(AppView::Extensions));

    let cr_cluster = ResourceRef::CustomResource {
        name: "global".to_string(),
        namespace: None,
        group: "infra.io".to_string(),
        version: "v1beta1".to_string(),
        kind: "ClusterWidget".to_string(),
        plural: "clusterwidgets".to_string(),
    };
    assert_eq!(cr_cluster.kind(), "ClusterWidget");
    assert_eq!(cr_cluster.name(), "global");
    assert_eq!(cr_cluster.namespace(), None);
    assert_eq!(cr_cluster.primary_view(), Some(AppView::Extensions));

    let flux_helm_release = ResourceRef::CustomResource {
        name: "backend".to_string(),
        namespace: Some("flux-system".to_string()),
        group: "helm.toolkit.fluxcd.io".to_string(),
        version: "v2".to_string(),
        kind: "HelmRelease".to_string(),
        plural: "helmreleases".to_string(),
    };
    assert_eq!(
        flux_helm_release.primary_view(),
        Some(AppView::FluxCDHelmReleases)
    );

    let flux_kustomization = ResourceRef::CustomResource {
        name: "apps".to_string(),
        namespace: Some("flux-system".to_string()),
        group: "kustomize.toolkit.fluxcd.io".to_string(),
        version: "v1".to_string(),
        kind: "Kustomization".to_string(),
        plural: "kustomizations".to_string(),
    };
    assert_eq!(
        flux_kustomization.primary_view(),
        Some(AppView::FluxCDKustomizations)
    );

    let flux_helm_chart = ResourceRef::CustomResource {
        name: "podinfo".to_string(),
        namespace: Some("flux-system".to_string()),
        group: "source.toolkit.fluxcd.io".to_string(),
        version: "v1".to_string(),
        kind: "HelmChart".to_string(),
        plural: "helmcharts".to_string(),
    };
    assert_eq!(flux_helm_chart.primary_view(), Some(AppView::FluxCDSources));
}

#[test]
fn ctrl_y_returns_copy_resource_name() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
    app.focus = Focus::Content;
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
    assert_eq!(action, AppAction::CopyResourceName);
}

#[test]
fn ctrl_y_returns_copy_resource_name_from_detail() {
    let mut app = AppState {
        detail_view: Some(DetailViewState {
            resource: Some(ResourceRef::Pod("api".to_string(), "default".to_string())),
            ..DetailViewState::default()
        }),
        ..AppState::default()
    };

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
    assert_eq!(action, AppAction::CopyResourceName);
}

#[test]
fn shift_y_returns_copy_full_name() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
    app.focus = Focus::Content;
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::CopyResourceFullName);
}

#[test]
fn shift_y_returns_copy_full_name_from_detail() {
    let mut app = AppState {
        detail_view: Some(DetailViewState {
            resource: Some(ResourceRef::Pod("api".to_string(), "default".to_string())),
            ..DetailViewState::default()
        }),
        ..AppState::default()
    };

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::CopyResourceFullName);
}

#[test]
fn ctrl_shift_y_uses_copy_resource_name_not_full_name() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
    app.focus = Focus::Content;

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('Y'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));

    assert_eq!(action, AppAction::CopyResourceName);
}

#[test]
fn ctrl_with_extra_modifier_does_not_copy_resource_name() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
    app.focus = Focus::Content;

    for modifiers in [
        KeyModifiers::CONTROL | KeyModifiers::ALT,
        KeyModifiers::CONTROL | KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::SUPER,
    ] {
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), modifiers));
        assert_eq!(action, AppAction::None, "{modifiers:?}");
    }
}

#[test]
fn ctrl_with_system_modifier_does_not_route_workbench_controls() {
    let mut app = AppState::default();
    app.workbench.open_tab(WorkbenchTabState::ActionHistory(
        crate::workbench::ActionHistoryTabState::default(),
    ));
    app.focus_workbench();

    for (code, modifiers) in [
        (
            KeyCode::Char('w'),
            KeyModifiers::CONTROL | KeyModifiers::META,
        ),
        (KeyCode::Up, KeyModifiers::CONTROL | KeyModifiers::SUPER),
        (KeyCode::Down, KeyModifiers::CONTROL | KeyModifiers::SUPER),
    ] {
        assert_eq!(
            app.handle_key_event(KeyEvent::new(code, modifiers)),
            AppAction::None,
            "{code:?} {modifiers:?}"
        );
    }

    assert_eq!(app.workbench.tabs.len(), 1);
}

#[test]
fn c_key_returns_cordon_in_node_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        ..DetailViewState::default()
    });
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::CordonNode);
}

#[test]
fn ctrl_c_does_not_cordon_in_node_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert_eq!(action, AppAction::None);
}

#[test]
fn u_key_returns_uncordon_in_node_detail() {
    let mut app = AppState::default();
    let mut detail = DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        ..DetailViewState::default()
    };
    detail.metadata.node_unschedulable = Some(true);
    app.detail_view = Some(detail);
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::UncordonNode);
}

#[test]
fn d_key_opens_drain_confirmation_in_node_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        ..DetailViewState::default()
    });
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::None);
    assert!(app.detail_view.as_ref().unwrap().confirm_drain);
}

#[test]
fn ctrl_shift_d_does_not_open_drain_confirmation_in_node_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('D'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(action, AppAction::None);
    assert!(!app.detail_view.as_ref().unwrap().confirm_drain);
}

#[test]
fn drain_confirm_d_returns_drain_node() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        confirm_drain: true,
        ..DetailViewState::default()
    });
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::DrainNode);
}

#[test]
fn ctrl_shift_d_does_not_confirm_drain_dialog() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        confirm_drain: true,
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('D'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(action, AppAction::None);
    assert!(app.detail_view.as_ref().unwrap().confirm_drain);
}

#[test]
fn drain_confirm_f_returns_force_drain() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        confirm_drain: true,
        ..DetailViewState::default()
    });
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::ForceDrainNode);
}

#[test]
fn drain_confirm_esc_cancels() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        confirm_drain: true,
        ..DetailViewState::default()
    });
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(action, AppAction::None);
    assert!(!app.detail_view.as_ref().unwrap().confirm_drain);
}

#[test]
fn cronjob_detail_jk_and_enter_follow_selected_job() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::CronJob(
            "nightly".to_string(),
            "ops".to_string(),
        )),
        yaml: Some("kind: CronJob".to_string()),
        cronjob_history: vec![
            CronJobHistoryEntry {
                job_name: "nightly-001".to_string(),
                namespace: "ops".to_string(),
                status: "Succeeded".to_string(),
                completions: "1/1".to_string(),
                duration: Some("8s".to_string()),
                pod_count: 1,
                live_pod_count: 0,
                completion_pct: Some(100),
                active_pods: 0,
                failed_pods: 0,
                age: None,
                created_at: None,
                logs_authorized: None,
            },
            CronJobHistoryEntry {
                job_name: "nightly-002".to_string(),
                namespace: "ops".to_string(),
                status: "Failed".to_string(),
                completions: "0/1".to_string(),
                duration: Some("3s".to_string()),
                pod_count: 1,
                live_pod_count: 1,
                completion_pct: Some(0),
                active_pods: 0,
                failed_pods: 1,
                age: None,
                created_at: None,
                logs_authorized: None,
            },
        ],
        ..DetailViewState::default()
    });

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
        AppAction::None
    );
    assert_eq!(
        app.detail_view.as_ref().unwrap().cronjob_history_selected,
        1
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::OpenDetail(ResourceRef::Job(
            "nightly-002".to_string(),
            "ops".to_string(),
        ))
    );
}

#[test]
fn cronjob_detail_shift_s_opens_suspend_confirmation() {
    let mut app = AppState::default();
    let mut detail = DetailViewState {
        resource: Some(ResourceRef::CronJob(
            "nightly".to_string(),
            "ops".to_string(),
        )),
        yaml: Some("kind: CronJob".to_string()),
        ..DetailViewState::default()
    };
    detail.metadata.cronjob_suspended = Some(false);
    app.detail_view = Some(detail);

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT)),
        AppAction::ConfirmCronJobSuspend(true)
    );
}

#[test]
fn cronjob_suspend_confirm_enter_dispatches_target_state() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::CronJob(
            "nightly".to_string(),
            "ops".to_string(),
        )),
        yaml: Some("kind: CronJob".to_string()),
        confirm_cronjob_suspend: Some(false),
        ..DetailViewState::default()
    });

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::SetCronJobSuspend(false)
    );
}

#[test]
fn c_key_does_not_cordon_for_pod_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    });
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert_ne!(action, AppAction::CordonNode);
}

#[test]
fn d_key_does_not_drain_for_pod_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    });
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT));
    assert_ne!(action, AppAction::DrainNode);
    assert!(!app.detail_view.as_ref().unwrap().confirm_drain);
}

#[test]
fn uppercase_d_opens_resource_diff_for_pod_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::OpenResourceDiff);
}

#[test]
fn ctrl_shift_d_scrolls_detail_panels_instead_of_opening_diff_for_pod_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('D'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(action, AppAction::None);
    assert_eq!(app.detail_view.as_ref().unwrap().top_panel_scroll, 10);
}

#[test]
fn ctrl_alt_detail_panel_scroll_shortcuts_do_not_scroll() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    });

    for code in [
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char('d'),
        KeyCode::Char('u'),
    ] {
        assert_eq!(
            app.handle_key_event(KeyEvent::new(
                code,
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )),
            AppAction::None,
            "{code:?}"
        );
        assert_eq!(app.detail_view.as_ref().unwrap().top_panel_scroll, 0);
    }
}

#[test]
fn g_key_opens_debug_dialog_for_pod_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::DebugContainerDialogOpen);
}

#[test]
fn uppercase_c_opens_connectivity_for_pod_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::OpenNetworkConnectivity);
}

#[test]
fn ctrl_shift_c_does_not_open_connectivity_for_pod_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('C'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(action, AppAction::None);
}

#[test]
fn uppercase_a_opens_access_review_for_pod_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::OpenAccessReview);
}

#[test]
fn uppercase_a_opens_access_review_from_content_focus() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
    app.focus = Focus::Content;

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::OpenAccessReview);
}

#[test]
fn ctrl_shift_a_does_not_open_access_review() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('A'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(action, AppAction::None);
}

#[test]
fn access_review_s_key_focuses_subject_input() {
    use crate::workbench::{AccessReviewFocus, AccessReviewTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    app.workbench
        .open_tab(WorkbenchTabState::AccessReview(AccessReviewTabState::new(
            ResourceRef::Pod("pod-0".to_string(), "ns".to_string()),
            Some("prod".to_string()),
            "ns".to_string(),
            Vec::new(),
            None,
            None,
        )));

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::None);
    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing access review tab");
    };
    let WorkbenchTabState::AccessReview(tab) = &tab.state else {
        panic!("expected access review tab");
    };
    assert_eq!(tab.focus, AccessReviewFocus::SubjectInput);
}

#[test]
fn access_review_s_key_scrolls_subject_input_into_view() {
    use crate::workbench::{AccessReviewFocus, AccessReviewTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let mut tab = AccessReviewTabState::new(
        ResourceRef::Pod("pod-0".to_string(), "ns".to_string()),
        Some("prod".to_string()),
        "ns".to_string(),
        Vec::new(),
        None,
        None,
    );
    tab.scroll = 99;
    app.workbench.open_tab(WorkbenchTabState::AccessReview(tab));

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::None);
    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing access review tab");
    };
    let WorkbenchTabState::AccessReview(tab) = &tab.state else {
        panic!("expected access review tab");
    };
    assert_eq!(tab.focus, AccessReviewFocus::SubjectInput);
    assert_eq!(tab.scroll, tab.subject_input_offset());
}

#[test]
fn access_review_enter_submits_subject_input() {
    use crate::workbench::{AccessReviewFocus, AccessReviewTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let mut tab = AccessReviewTabState::new(
        ResourceRef::Pod("pod-0".to_string(), "ns".to_string()),
        Some("prod".to_string()),
        "ns".to_string(),
        Vec::new(),
        None,
        None,
    );
    tab.start_subject_input();
    tab.subject_input.value = "User/alice@example.com".to_string();
    app.workbench.open_tab(WorkbenchTabState::AccessReview(tab));

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(action, AppAction::ApplyAccessReviewSubject);
    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing access review tab");
    };
    let WorkbenchTabState::AccessReview(tab) = &tab.state else {
        panic!("expected access review tab");
    };
    assert_eq!(tab.focus, AccessReviewFocus::SubjectInput);
}

#[test]
fn access_review_subject_input_treats_r_and_b_as_text() {
    use crate::workbench::{AccessReviewTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let mut tab = AccessReviewTabState::new(
        ResourceRef::Pod("pod-0".to_string(), "ns".to_string()),
        Some("prod".to_string()),
        "ns".to_string(),
        Vec::new(),
        None,
        None,
    );
    tab.start_subject_input();
    app.workbench.open_tab(WorkbenchTabState::AccessReview(tab));

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
        AppAction::None
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE)),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing access review tab");
    };
    let WorkbenchTabState::AccessReview(tab) = &tab.state else {
        panic!("expected access review tab");
    };
    assert_eq!(tab.subject_input.value, "rb");
}

#[test]
fn access_review_subject_input_ignores_alt_modified_chars() {
    use crate::workbench::{AccessReviewTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let mut tab = AccessReviewTabState::new(
        ResourceRef::Pod("pod-0".to_string(), "ns".to_string()),
        Some("prod".to_string()),
        "ns".to_string(),
        Vec::new(),
        None,
        None,
    );
    tab.start_subject_input();
    tab.subject_input.value = "User/alice".to_string();
    tab.subject_input.cursor_pos = tab.subject_input.value.chars().count();
    app.workbench.open_tab(WorkbenchTabState::AccessReview(tab));

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT)),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing access review tab");
    };
    let WorkbenchTabState::AccessReview(tab) = &tab.state else {
        panic!("expected access review tab");
    };
    assert_eq!(tab.subject_input.value, "User/alice");
}

#[test]
fn access_review_subject_modified_escape_does_not_stop_input() {
    use crate::workbench::{AccessReviewFocus, AccessReviewTabState, WorkbenchTabState};

    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::META,
        KeyModifiers::SUPER,
        KeyModifiers::CONTROL | KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::SUPER,
    ] {
        let mut app = AppState::default();
        app.focus = Focus::Workbench;
        let mut tab = AccessReviewTabState::new(
            ResourceRef::Pod("pod-0".to_string(), "ns".to_string()),
            Some("prod".to_string()),
            "ns".to_string(),
            Vec::new(),
            None,
            None,
        );
        tab.start_subject_input();
        tab.subject_input.value = "User/alice".to_string();
        app.workbench.open_tab(WorkbenchTabState::AccessReview(tab));

        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );

        let Some(tab) = app.workbench.active_tab() else {
            panic!("missing access review tab");
        };
        let WorkbenchTabState::AccessReview(tab) = &tab.state else {
            panic!("expected access review tab");
        };
        assert_eq!(tab.focus, AccessReviewFocus::SubjectInput, "{modifiers:?}");
        assert_eq!(tab.subject_input.value, "User/alice", "{modifiers:?}");
    }
}

#[test]
fn access_review_subject_modified_edit_keys_do_not_mutate_input_or_cursor() {
    use crate::workbench::{AccessReviewTabState, WorkbenchTabState};

    for key in modified_edit_key_events() {
        let mut app = AppState::default();
        app.focus = Focus::Workbench;
        let mut tab = AccessReviewTabState::new(
            ResourceRef::Pod("pod-0".to_string(), "ns".to_string()),
            Some("prod".to_string()),
            "ns".to_string(),
            Vec::new(),
            None,
            None,
        );
        tab.start_subject_input();
        tab.subject_input.value = "User/alice".to_string();
        tab.subject_input.cursor_pos = 4;
        app.workbench.open_tab(WorkbenchTabState::AccessReview(tab));

        assert_eq!(app.handle_key_event(key), AppAction::None, "{key:?}");

        let WorkbenchTabState::AccessReview(tab) = &app.workbench.active_tab().unwrap().state
        else {
            panic!("expected access review tab");
        };
        assert_eq!(tab.subject_input.value, "User/alice", "{key:?}");
        assert_eq!(tab.subject_input.cursor_pos, 4, "{key:?}");
    }
}

#[test]
fn access_review_subject_delete_removes_character_at_cursor() {
    use crate::workbench::{AccessReviewTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let mut tab = AccessReviewTabState::new(
        ResourceRef::Pod("pod-0".to_string(), "ns".to_string()),
        Some("prod".to_string()),
        "ns".to_string(),
        Vec::new(),
        None,
        None,
    );
    tab.start_subject_input();
    tab.subject_input.value = "abcd".to_string();
    tab.subject_input.cursor_pos = 1;
    app.workbench.open_tab(WorkbenchTabState::AccessReview(tab));

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
        AppAction::None
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing access review tab");
    };
    let WorkbenchTabState::AccessReview(tab) = &tab.state else {
        panic!("expected access review tab");
    };
    assert_eq!(tab.subject_input.value, "acd");
    assert_eq!(tab.subject_input.cursor_pos, 1);
}

#[test]
fn reopen_access_review_tab_preserves_existing_subject_input_state() {
    use crate::policy::DetailAction;
    use crate::workbench::{AccessReviewFocus, AccessReviewTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::Pod("pod-0".to_string(), "ns".to_string());
    let mut tab = AccessReviewTabState::new(
        resource.clone(),
        Some("prod".to_string()),
        "ns".to_string(),
        Vec::new(),
        None,
        None,
    );
    tab.start_subject_input();
    tab.subject_input.value = "User/alice@example.com".to_string();
    tab.subject_input.cursor_pos = tab.subject_input.value.chars().count();
    tab.focus = AccessReviewFocus::SubjectInput;
    tab.scroll = 7;
    tab.entries = vec![crate::authorization::ActionAccessReview {
        action: DetailAction::Logs,
        authorization: None,
        strict: false,
        checks: Vec::new(),
    }];
    app.workbench.open_tab(WorkbenchTabState::AccessReview(tab));

    app.open_access_review_tab(
        resource,
        Some("staging".to_string()),
        "payments".to_string(),
        vec![crate::authorization::ActionAccessReview {
            action: DetailAction::Delete,
            authorization: None,
            strict: true,
            checks: Vec::new(),
        }],
        Some(crate::rbac_subjects::SubjectAccessReview {
            subject: crate::rbac_subjects::AccessReviewSubject::ServiceAccount {
                name: "builder".into(),
                namespace: "payments".into(),
            },
            bindings: vec![crate::rbac_subjects::SubjectBindingResolution {
                binding: ResourceRef::RoleBinding("payments-read".into(), "payments".into()),
                role: crate::rbac_subjects::SubjectRoleResolution {
                    resource: Some(ResourceRef::Role(
                        "payments-reader".into(),
                        "payments".into(),
                    )),
                    kind: "Role".into(),
                    name: "payments-reader".into(),
                    namespace: Some("payments".into()),
                    rules: vec![crate::k8s::dtos::RbacRule {
                        verbs: vec!["get".into()],
                        resources: vec!["pods".into()],
                        ..crate::k8s::dtos::RbacRule::default()
                    }],
                    missing: false,
                },
            }],
        }),
        Some(crate::workbench::AttemptedActionReview {
            action: DetailAction::Delete,
            authorization: None,
            strict: true,
            checks: Vec::new(),
            note: Some("fresh".into()),
        }),
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing access review tab");
    };
    let WorkbenchTabState::AccessReview(tab) = &tab.state else {
        panic!("expected access review tab");
    };
    assert_eq!(tab.focus, AccessReviewFocus::SubjectInput);
    assert_eq!(tab.subject_input.value, "User/alice@example.com");
    assert_eq!(tab.scroll, 7);
    assert_eq!(tab.context_name.as_deref(), Some("staging"));
    assert_eq!(tab.namespace_scope, "payments");
    assert_eq!(tab.entries.len(), 1);
    assert_eq!(tab.entries[0].action, DetailAction::Delete);
    assert_eq!(
        tab.attempted_review.as_ref().map(|review| review.action),
        Some(DetailAction::Delete)
    );
    assert_eq!(
        tab.subject_review
            .as_ref()
            .map(|review| review.subject.spec())
            .as_deref(),
        Some("ServiceAccount/payments/builder")
    );
}

#[test]
fn t_key_opens_traffic_debug_for_service_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Service("api".to_string(), "ns".to_string())),
        yaml: Some("kind: Service".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::OpenTrafficDebug);
}

#[test]
fn t_key_opens_traffic_debug_from_services_view() {
    let mut app = AppState::default();
    app.view = AppView::Services;
    app.focus = Focus::Content;

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::OpenTrafficDebug);
}

#[test]
fn h_key_opens_helm_history_for_helm_release_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::HelmRelease(
            "web".to_string(),
            "default".to_string(),
        )),
        yaml: Some("kind: Secret".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::OpenHelmHistory);
}

#[test]
fn h_key_opens_helm_history_from_helm_releases_view() {
    let mut app = AppState::default();
    app.view = AppView::HelmReleases;
    app.focus = Focus::Content;

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::OpenHelmHistory);
}

#[test]
fn h_key_is_noop_for_pod_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::None);
}

fn app_with_helm_history_workbench_tab() -> AppState {
    use crate::k8s::dtos::HelmReleaseRevisionInfo;
    use crate::workbench::{HelmHistoryTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;

    let resource = ResourceRef::HelmRelease("web".to_string(), "default".to_string());
    let mut tab = HelmHistoryTabState::new(resource);
    tab.loading = false;
    tab.revisions = vec![
        HelmReleaseRevisionInfo {
            revision: 5,
            ..HelmReleaseRevisionInfo::default()
        },
        HelmReleaseRevisionInfo {
            revision: 4,
            ..HelmReleaseRevisionInfo::default()
        },
    ];
    tab.current_revision = Some(5);
    tab.selected = 1;

    app.workbench.open_tab(WorkbenchTabState::HelmHistory(tab));
    app
}

#[test]
fn helm_history_enter_opens_values_diff_for_selected_target_revision() {
    let mut app = app_with_helm_history_workbench_tab();

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(action, AppAction::OpenHelmValuesDiff);
}

#[test]
fn helm_history_uppercase_r_opens_rollback_confirmation() {
    let mut app = app_with_helm_history_workbench_tab();

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::ConfirmHelmRollback);
}

#[test]
fn helm_history_confirm_mode_executes_rollback_on_enter_y_or_r() {
    let mut app = app_with_helm_history_workbench_tab();
    if let Some(tab) = app.workbench.active_tab_mut()
        && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &mut tab.state
    {
        helm_tab.confirm_rollback_revision = Some(4);
    }

    for key in [
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT),
    ] {
        let mut app = app.clone();
        let action = app.handle_key_event(key);
        assert_eq!(action, AppAction::ExecuteHelmRollback);
    }
}

#[test]
fn helm_history_confirm_mode_ctrl_shift_r_does_not_execute_rollback() {
    let mut app = app_with_helm_history_workbench_tab();
    if let Some(tab) = app.workbench.active_tab_mut()
        && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &mut tab.state
    {
        helm_tab.confirm_rollback_revision = Some(4);
    }

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('R'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(action, AppAction::None);

    if let Some(tab) = app.workbench.active_tab()
        && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &tab.state
    {
        assert_eq!(helm_tab.confirm_rollback_revision, Some(4));
    } else {
        panic!("expected helm history tab");
    }
}

#[test]
fn helm_history_confirm_mode_ctrl_shift_d_scrolls_instead_of_confirming() {
    let mut app = app_with_helm_history_workbench_tab();
    if let Some(tab) = app.workbench.active_tab_mut()
        && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &mut tab.state
    {
        helm_tab.confirm_rollback_revision = Some(4);
    }

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('D'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(action, AppAction::None);

    if let Some(tab) = app.workbench.active_tab()
        && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &tab.state
    {
        assert_eq!(helm_tab.confirm_rollback_revision, Some(4));
        assert_eq!(helm_tab.scroll, 10);
    } else {
        panic!("expected helm history tab");
    }
}

#[test]
fn helm_history_confirm_mode_ctrl_shift_j_scrolls() {
    let mut app = app_with_helm_history_workbench_tab();
    if let Some(tab) = app.workbench.active_tab_mut()
        && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &mut tab.state
    {
        helm_tab.confirm_rollback_revision = Some(4);
    }

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('J'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(action, AppAction::None);

    if let Some(tab) = app.workbench.active_tab()
        && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &tab.state
    {
        assert_eq!(helm_tab.confirm_rollback_revision, Some(4));
        assert_eq!(helm_tab.scroll, 1);
    } else {
        panic!("expected helm history tab");
    }
}

#[test]
fn helm_history_confirm_page_keys_require_plain_modifiers() {
    let mut plain = app_with_helm_history_workbench_tab();
    if let Some(tab) = plain.workbench.active_tab_mut()
        && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &mut tab.state
    {
        helm_tab.confirm_rollback_revision = Some(4);
    }
    assert_eq!(
        plain.handle_key_event(KeyEvent::from(KeyCode::PageDown)),
        AppAction::None
    );
    if let Some(tab) = plain.workbench.active_tab()
        && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &tab.state
    {
        assert_eq!(helm_tab.scroll, 10);
    } else {
        panic!("expected helm history tab");
    }

    let mut modified = app_with_helm_history_workbench_tab();
    if let Some(tab) = modified.workbench.active_tab_mut()
        && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &mut tab.state
    {
        helm_tab.confirm_rollback_revision = Some(4);
        helm_tab.scroll = 10;
    }
    assert_eq!(
        modified.handle_key_event(KeyEvent::new(KeyCode::PageUp, KeyModifiers::CONTROL)),
        AppAction::None
    );
    if let Some(tab) = modified.workbench.active_tab()
        && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &tab.state
    {
        assert_eq!(helm_tab.scroll, 10);
    } else {
        panic!("expected helm history tab");
    }
}

#[test]
fn helm_history_escape_cancels_rollback_confirmation() {
    let mut app = app_with_helm_history_workbench_tab();
    if let Some(tab) = app.workbench.active_tab_mut()
        && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &mut tab.state
    {
        helm_tab.confirm_rollback_revision = Some(4);
    }

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(action, AppAction::None);

    if let Some(tab) = app.workbench.active_tab()
        && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &tab.state
    {
        assert!(helm_tab.confirm_rollback_revision.is_none());
    } else {
        panic!("expected helm history tab");
    }
}

#[test]
fn helm_history_modified_escape_does_not_cancel_submodes() {
    use crate::workbench::HelmValuesDiffState;

    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::META,
        KeyModifiers::SUPER,
        KeyModifiers::CONTROL | KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::SUPER,
    ] {
        let mut confirm = app_with_helm_history_workbench_tab();
        if let Some(tab) = confirm.workbench.active_tab_mut()
            && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &mut tab.state
        {
            helm_tab.confirm_rollback_revision = Some(4);
        }
        assert_eq!(
            confirm.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );
        if let Some(tab) = confirm.workbench.active_tab()
            && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &tab.state
        {
            assert_eq!(helm_tab.confirm_rollback_revision, Some(4), "{modifiers:?}");
        } else {
            panic!("expected helm history tab");
        }

        let mut diff = app_with_helm_history_workbench_tab();
        if let Some(tab) = diff.workbench.active_tab_mut()
            && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &mut tab.state
        {
            helm_tab.diff = Some(HelmValuesDiffState::new(5, 4, 9));
        }
        assert_eq!(
            diff.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );
        if let Some(tab) = diff.workbench.active_tab()
            && let crate::workbench::WorkbenchTabState::HelmHistory(helm_tab) = &tab.state
        {
            assert!(helm_tab.diff.is_some(), "{modifiers:?}");
        } else {
            panic!("expected helm history tab");
        }
    }
}

#[test]
fn uppercase_o_opens_rollout_for_deployment_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Deployment(
            "api".to_string(),
            "default".to_string(),
        )),
        yaml: Some("kind: Deployment".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::OpenRollout);
}

#[test]
fn ctrl_shift_o_does_not_open_rollout_for_deployment_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Deployment(
            "api".to_string(),
            "default".to_string(),
        )),
        yaml: Some("kind: Deployment".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('O'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(action, AppAction::None);
}

#[test]
fn uppercase_o_is_noop_for_pod_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::None);
}

#[test]
fn g_key_opens_node_debug_dialog_for_node_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::NodeDebugDialogOpen);
}

#[test]
fn uppercase_c_is_noop_for_node_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::None);
}

#[test]
fn escape_closes_debug_dialog_before_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        debug_dialog: Some(crate::ui::components::DebugContainerDialogState::new(
            "pod-0", "ns",
        )),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(action, AppAction::None);
    assert!(
        app.detail_view
            .as_ref()
            .is_some_and(|detail| detail.debug_dialog.is_none())
    );
}

#[test]
fn uppercase_d_is_noop_from_nodes_content_view() {
    let mut app = AppState {
        focus: Focus::Content,
        view: AppView::Nodes,
        ..AppState::default()
    };

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::None);
}

#[test]
fn rollout_workbench_shortcuts_dispatch_expected_actions() {
    let mut app = AppState::default();
    app.open_rollout_tab(
        ResourceRef::Deployment("api".to_string(), "default".to_string()),
        Some(RolloutInspection {
            kind: RolloutWorkloadKind::Deployment,
            strategy: "RollingUpdate".to_string(),
            paused: false,
            current_revision: Some(5),
            update_target_revision: Some(5),
            summary_lines: vec!["Desired 3".to_string()],
            conditions: Vec::new(),
            revisions: vec![
                RolloutRevisionInfo {
                    revision: 5,
                    name: "api-5".to_string(),
                    created: None,
                    summary: "3/3 ready".to_string(),
                    change_cause: None,
                    is_current: true,
                    is_update_target: true,
                },
                RolloutRevisionInfo {
                    revision: 4,
                    name: "api-4".to_string(),
                    created: None,
                    summary: "3/3 ready".to_string(),
                    change_cause: None,
                    is_current: false,
                    is_update_target: false,
                },
            ],
        }),
        None,
        None,
    );
    app.focus_workbench();
    if let Some(tab) = app.workbench.active_tab_mut()
        && let WorkbenchTabState::Rollout(rollout_tab) = &mut tab.state
    {
        rollout_tab.selected = 1;
    }

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('U'), KeyModifiers::SHIFT)),
        AppAction::ConfirmRolloutUndo
    );
    if let Some(tab) = app.workbench.active_tab_mut()
        && let WorkbenchTabState::Rollout(rollout_tab) = &mut tab.state
    {
        rollout_tab.confirm_undo_revision = Some(4);
    }
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::ExecuteRolloutUndo
    );

    if let Some(tab) = app.workbench.active_tab_mut()
        && let WorkbenchTabState::Rollout(rollout_tab) = &mut tab.state
    {
        rollout_tab.confirm_undo_revision = None;
    }

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT)),
        AppAction::ToggleRolloutPauseResume
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT)),
        AppAction::RolloutRestart
    );
}

#[test]
fn rollout_confirm_mode_ctrl_shift_u_scrolls_instead_of_execute_undo() {
    let mut app = AppState::default();
    app.open_rollout_tab(
        ResourceRef::Deployment("api".to_string(), "default".to_string()),
        Some(RolloutInspection {
            kind: RolloutWorkloadKind::Deployment,
            strategy: "RollingUpdate".to_string(),
            paused: false,
            current_revision: Some(5),
            update_target_revision: Some(5),
            summary_lines: vec!["Desired 3".to_string()],
            conditions: Vec::new(),
            revisions: vec![RolloutRevisionInfo {
                revision: 4,
                name: "api-4".to_string(),
                created: None,
                summary: "3/3 ready".to_string(),
                change_cause: None,
                is_current: false,
                is_update_target: false,
            }],
        }),
        None,
        None,
    );
    app.focus_workbench();
    if let Some(tab) = app.workbench.active_tab_mut()
        && let WorkbenchTabState::Rollout(rollout_tab) = &mut tab.state
    {
        rollout_tab.confirm_undo_revision = Some(4);
        rollout_tab.detail_scroll = 10;
    }

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('U'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(action, AppAction::None);

    if let Some(tab) = app.workbench.active_tab()
        && let WorkbenchTabState::Rollout(rollout_tab) = &tab.state
    {
        assert_eq!(rollout_tab.confirm_undo_revision, Some(4));
        assert_eq!(rollout_tab.detail_scroll, 0);
    } else {
        panic!("expected rollout tab");
    }
}

#[test]
fn rollout_confirm_page_keys_require_plain_modifiers() {
    let mut app = AppState::default();
    app.open_rollout_tab(
        ResourceRef::Deployment("api".to_string(), "default".to_string()),
        Some(RolloutInspection {
            kind: RolloutWorkloadKind::Deployment,
            strategy: "RollingUpdate".to_string(),
            paused: false,
            current_revision: Some(5),
            update_target_revision: Some(5),
            summary_lines: vec!["Desired 3".to_string()],
            conditions: Vec::new(),
            revisions: vec![RolloutRevisionInfo {
                revision: 4,
                name: "api-4".to_string(),
                created: None,
                summary: "3/3 ready".to_string(),
                change_cause: None,
                is_current: false,
                is_update_target: false,
            }],
        }),
        None,
        None,
    );
    app.focus_workbench();
    if let Some(tab) = app.workbench.active_tab_mut()
        && let WorkbenchTabState::Rollout(rollout_tab) = &mut tab.state
    {
        rollout_tab.confirm_undo_revision = Some(4);
    }
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::PageDown)),
        AppAction::None
    );
    if let Some(tab) = app.workbench.active_tab()
        && let WorkbenchTabState::Rollout(rollout_tab) = &tab.state
    {
        assert_eq!(rollout_tab.detail_scroll, 10);
    } else {
        panic!("expected rollout tab");
    }

    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::PageUp, KeyModifiers::ALT)),
        AppAction::None
    );
    if let Some(tab) = app.workbench.active_tab()
        && let WorkbenchTabState::Rollout(rollout_tab) = &tab.state
    {
        assert_eq!(rollout_tab.detail_scroll, 10);
    } else {
        panic!("expected rollout tab");
    }
}

#[test]
fn rollout_confirm_mode_ctrl_shift_j_scrolls() {
    let mut app = AppState::default();
    app.open_rollout_tab(
        ResourceRef::Deployment("api".to_string(), "default".to_string()),
        Some(RolloutInspection {
            kind: RolloutWorkloadKind::Deployment,
            strategy: "RollingUpdate".to_string(),
            paused: false,
            current_revision: Some(5),
            update_target_revision: Some(5),
            summary_lines: vec!["Desired 3".to_string()],
            conditions: Vec::new(),
            revisions: vec![RolloutRevisionInfo {
                revision: 4,
                name: "api-4".to_string(),
                created: None,
                summary: "3/3 ready".to_string(),
                change_cause: None,
                is_current: false,
                is_update_target: false,
            }],
        }),
        None,
        None,
    );
    app.focus = Focus::Workbench;
    if let Some(tab) = app.workbench.active_tab_mut()
        && let WorkbenchTabState::Rollout(rollout_tab) = &mut tab.state
    {
        rollout_tab.confirm_undo_revision = Some(4);
    }

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('J'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(action, AppAction::None);

    if let Some(tab) = app.workbench.active_tab()
        && let WorkbenchTabState::Rollout(rollout_tab) = &tab.state
    {
        assert_eq!(rollout_tab.confirm_undo_revision, Some(4));
        assert_eq!(rollout_tab.detail_scroll, 1);
    } else {
        panic!("expected rollout tab");
    }
}

#[test]
fn rollout_confirm_modified_escape_does_not_cancel_undo() {
    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::META,
        KeyModifiers::SUPER,
        KeyModifiers::CONTROL | KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::SUPER,
    ] {
        let mut app = AppState::default();
        app.open_rollout_tab(
            ResourceRef::Deployment("api".to_string(), "default".to_string()),
            Some(RolloutInspection {
                kind: RolloutWorkloadKind::Deployment,
                strategy: "RollingUpdate".to_string(),
                paused: false,
                current_revision: Some(5),
                update_target_revision: Some(5),
                summary_lines: vec!["Desired 3".to_string()],
                conditions: Vec::new(),
                revisions: vec![RolloutRevisionInfo {
                    revision: 4,
                    name: "api-4".to_string(),
                    created: None,
                    summary: "3/3 ready".to_string(),
                    change_cause: None,
                    is_current: false,
                    is_update_target: false,
                }],
            }),
            None,
            None,
        );
        app.focus_workbench();
        if let Some(tab) = app.workbench.active_tab_mut()
            && let WorkbenchTabState::Rollout(rollout_tab) = &mut tab.state
        {
            rollout_tab.confirm_undo_revision = Some(4);
        }

        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );
        if let Some(tab) = app.workbench.active_tab()
            && let WorkbenchTabState::Rollout(rollout_tab) = &tab.state
        {
            assert_eq!(rollout_tab.confirm_undo_revision, Some(4), "{modifiers:?}");
        } else {
            panic!("expected rollout tab");
        }
    }
}

#[test]
fn runbook_workbench_shortcuts_dispatch_expected_actions() {
    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    app.open_runbook_tab(
        LoadedRunbook {
            id: "pod_failure".into(),
            title: "Pod Failure Triage".into(),
            description: None,
            aliases: vec!["incident".into()],
            resource_kinds: vec!["Pod".into()],
            shortcut: None,
            steps: vec![
                LoadedRunbookStep {
                    title: "Checklist".into(),
                    description: None,
                    kind: LoadedRunbookStepKind::Checklist {
                        items: vec!["Inspect events".into()],
                    },
                },
                LoadedRunbookStep {
                    title: "Open logs".into(),
                    description: None,
                    kind: LoadedRunbookStepKind::DetailAction {
                        action: RunbookDetailAction::Logs,
                    },
                },
            ],
        },
        Some(ResourceRef::Pod("api".into(), "prod".into())),
    );

    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Enter)),
        AppAction::RunbookExecuteSelectedStep
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('d'))),
        AppAction::RunbookToggleStepDone
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('s'))),
        AppAction::RunbookToggleStepSkipped
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(
            KeyCode::Char('D'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        )),
        AppAction::None
    );
    if let Some(tab) = app.workbench.active_tab()
        && let WorkbenchTabState::Runbook(tab) = &tab.state
    {
        assert_eq!(tab.detail_scroll, 10);
    } else {
        panic!("expected active runbook tab");
    }
    assert_eq!(
        app.handle_key_event(KeyEvent::new(
            KeyCode::Char('J'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        )),
        AppAction::None
    );
    if let Some(tab) = app.workbench.active_tab()
        && let WorkbenchTabState::Runbook(tab) = &tab.state
    {
        assert_eq!(tab.detail_scroll, 11);
    } else {
        panic!("expected active runbook tab");
    }

    app.handle_key_event(KeyEvent::from(KeyCode::Down));
    if let Some(tab) = app.workbench.active_tab()
        && let WorkbenchTabState::Runbook(tab) = &tab.state
    {
        assert_eq!(tab.selected, 1);
    } else {
        panic!("expected active runbook tab");
    }
}

#[test]
fn reopen_runbook_tab_preserves_existing_progress_and_selection() {
    let mut app = AppState::default();
    let resource = Some(ResourceRef::Pod("api".into(), "prod".into()));
    let runbook = LoadedRunbook {
        id: "pod_failure".into(),
        title: "Pod Failure Triage".into(),
        description: None,
        aliases: vec!["incident".into()],
        resource_kinds: vec!["Pod".into()],
        shortcut: None,
        steps: vec![
            LoadedRunbookStep {
                title: "Checklist".into(),
                description: None,
                kind: LoadedRunbookStepKind::Checklist {
                    items: vec!["Inspect events".into()],
                },
            },
            LoadedRunbookStep {
                title: "Open logs".into(),
                description: None,
                kind: LoadedRunbookStepKind::DetailAction {
                    action: RunbookDetailAction::Logs,
                },
            },
        ],
    };

    app.focus = Focus::Workbench;
    app.open_runbook_tab(runbook.clone(), resource.clone());
    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("missing runbook tab");
    };
    let WorkbenchTabState::Runbook(tab) = &mut tab.state else {
        panic!("expected runbook tab");
    };
    tab.selected = 1;
    tab.detail_scroll = 4;
    tab.toggle_done();

    let refreshed_runbook = LoadedRunbook {
        title: "Pod Failure Triage v2".into(),
        steps: vec![
            LoadedRunbookStep {
                title: "Checklist".into(),
                description: Some("fresh".into()),
                kind: LoadedRunbookStepKind::Checklist {
                    items: vec!["Inspect events".into(), "Inspect probes".into()],
                },
            },
            LoadedRunbookStep {
                title: "Open logs".into(),
                description: Some("updated".into()),
                kind: LoadedRunbookStepKind::DetailAction {
                    action: RunbookDetailAction::Logs,
                },
            },
        ],
        ..runbook
    };

    app.open_runbook_tab(refreshed_runbook, resource);

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing runbook tab");
    };
    let WorkbenchTabState::Runbook(tab) = &tab.state else {
        panic!("expected runbook tab");
    };
    assert_eq!(tab.selected, 1);
    assert_eq!(tab.detail_scroll, 4);
    assert_eq!(tab.steps[1].state, crate::workbench::RunbookStepState::Done);
    assert_eq!(tab.runbook.title, "Pod Failure Triage v2");
    assert_eq!(tab.steps[0].step.description.as_deref(), Some("fresh"));
    let crate::runbooks::LoadedRunbookStepKind::Checklist { items } = &tab.steps[0].step.kind
    else {
        panic!("expected checklist step");
    };
    assert_eq!(items.len(), 2);
}

#[test]
fn reopen_same_target_port_forward_tab_preserves_dialog_state() {
    use crate::ui::components::port_forward_dialog::{PortForwardDialog, PortForwardMode};
    use crate::workbench::WorkbenchTabState;

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = Some(ResourceRef::Pod("api".into(), "prod".into()));
    let mut dialog = PortForwardDialog::with_target("prod", "api", 8080);
    dialog.mode = PortForwardMode::List;
    dialog.selected_tunnel = 2;
    app.open_port_forward_tab(resource.clone(), dialog);

    let Some(tab) = app.workbench.active_tab_mut() else {
        panic!("missing port-forward tab");
    };
    let WorkbenchTabState::PortForward(tab) = &mut tab.state else {
        panic!("expected port-forward tab");
    };
    tab.dialog.selected_tunnel = 1;
    tab.dialog.success = Some("keep".into());

    app.open_port_forward_tab(resource, PortForwardDialog::new());

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing port-forward tab");
    };
    let WorkbenchTabState::PortForward(tab) = &tab.state else {
        panic!("expected port-forward tab");
    };
    assert_eq!(tab.dialog.mode, PortForwardMode::List);
    assert_eq!(tab.dialog.selected_tunnel, 1);
    assert_eq!(tab.dialog.success.as_deref(), Some("keep"));
}

#[test]
fn reopen_decoded_secret_tab_preserves_unsaved_edit_state() {
    use crate::secret::{DecodedSecretEntry, DecodedSecretValue};
    use crate::workbench::{DecodedSecretTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::Secret("app-secret".into(), "prod".into());
    let mut tab = DecodedSecretTabState::new(resource.clone());
    tab.entries = vec![DecodedSecretEntry {
        key: "TOKEN".into(),
        value: DecodedSecretValue::Text {
            current: "new-value".into(),
            original: "old-value".into(),
        },
    }];
    tab.selected = 0;
    tab.masked = false;
    tab.editing = true;
    tab.edit_input = "new-value".into();
    tab.edit_cursor = tab.edit_input.len();
    app.workbench
        .open_tab(WorkbenchTabState::DecodedSecret(tab));

    app.open_decoded_secret_tab(resource, Some("kind: Secret".into()), None, Some(42));

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing decoded secret tab");
    };
    let WorkbenchTabState::DecodedSecret(tab) = &tab.state else {
        panic!("expected decoded secret tab");
    };
    assert!(tab.editing);
    assert_eq!(tab.edit_input, "new-value");
    assert!(tab.has_unsaved_changes());
    assert_eq!(
        tab.selected_entry().map(|entry| entry.key.as_str()),
        Some("TOKEN")
    );
}

#[test]
fn decoded_secret_modified_escape_does_not_cancel_unmasked_edit() {
    use crate::secret::{DecodedSecretEntry, DecodedSecretValue};
    use crate::workbench::{DecodedSecretTabState, WorkbenchTabState};

    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::META,
        KeyModifiers::SUPER,
        KeyModifiers::CONTROL | KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::SUPER,
    ] {
        let mut app = AppState::default();
        app.focus = Focus::Workbench;
        let mut tab =
            DecodedSecretTabState::new(ResourceRef::Secret("app-secret".into(), "prod".into()));
        tab.entries = vec![DecodedSecretEntry {
            key: "TOKEN".into(),
            value: DecodedSecretValue::Text {
                current: "new-value".into(),
                original: "old-value".into(),
            },
        }];
        tab.masked = false;
        tab.editing = true;
        tab.edit_input = "new-value".into();
        tab.edit_cursor = tab.edit_input.len();
        app.workbench
            .open_tab(WorkbenchTabState::DecodedSecret(tab));

        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );

        let Some(tab) = app.workbench.active_tab() else {
            panic!("missing decoded secret tab");
        };
        let WorkbenchTabState::DecodedSecret(tab) = &tab.state else {
            panic!("expected decoded secret tab");
        };
        assert!(tab.editing, "{modifiers:?}");
        assert_eq!(tab.edit_input, "new-value", "{modifiers:?}");
    }
}

#[test]
fn decoded_secret_modified_edit_keys_do_not_mutate_input_or_cursor() {
    use crate::secret::{DecodedSecretEntry, DecodedSecretValue};
    use crate::workbench::{DecodedSecretTabState, WorkbenchTabState};

    for key in modified_edit_key_events() {
        let mut app = AppState::default();
        app.focus = Focus::Workbench;
        let mut tab =
            DecodedSecretTabState::new(ResourceRef::Secret("app-secret".into(), "prod".into()));
        tab.entries = vec![DecodedSecretEntry {
            key: "TOKEN".into(),
            value: DecodedSecretValue::Text {
                current: "new-value".into(),
                original: "old-value".into(),
            },
        }];
        tab.masked = false;
        tab.editing = true;
        tab.edit_input = "new-value".into();
        tab.edit_cursor = 3;
        app.workbench
            .open_tab(WorkbenchTabState::DecodedSecret(tab));

        assert_eq!(app.handle_key_event(key), AppAction::None, "{key:?}");

        let WorkbenchTabState::DecodedSecret(tab) = &app.workbench.active_tab().unwrap().state
        else {
            panic!("expected decoded secret tab");
        };
        assert_eq!(tab.edit_input, "new-value", "{key:?}");
        assert_eq!(tab.edit_cursor, 3, "{key:?}");
    }
}

#[test]
fn reopen_clean_decoded_secret_tab_tracks_new_pending_fetch() {
    use crate::secret::{DecodedSecretEntry, DecodedSecretValue};
    use crate::workbench::{DecodedSecretTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::Secret("app-secret".into(), "prod".into());
    let mut tab = DecodedSecretTabState::new(resource.clone());
    tab.loading = false;
    tab.error = Some("previous load failed".into());
    tab.entries = vec![DecodedSecretEntry {
        key: "TOKEN".into(),
        value: DecodedSecretValue::Text {
            current: "old-value".into(),
            original: "old-value".into(),
        },
    }];
    app.workbench
        .open_tab(WorkbenchTabState::DecodedSecret(tab));

    app.open_decoded_secret_tab(resource, None, None, Some(99));

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing decoded secret tab");
    };
    let WorkbenchTabState::DecodedSecret(tab) = &tab.state else {
        panic!("expected decoded secret tab");
    };
    assert!(tab.loading);
    assert_eq!(tab.pending_request_id, Some(99));
    assert!(tab.error.is_none());
}

#[test]
fn reopen_resource_yaml_tab_preserves_scroll_while_refreshing_payload() {
    use crate::workbench::{ResourceYamlTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::Pod("api".into(), "prod".into());
    let mut tab = ResourceYamlTabState::new(resource.clone());
    tab.yaml = Some("kind: Pod\nmetadata:\n  name: api".into());
    tab.loading = false;
    tab.scroll = 9;
    app.workbench.open_tab(WorkbenchTabState::ResourceYaml(tab));

    app.open_resource_yaml_tab(resource, None, None, Some(77));

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing yaml tab");
    };
    let WorkbenchTabState::ResourceYaml(tab) = &tab.state else {
        panic!("expected yaml tab");
    };
    assert_eq!(tab.scroll, 9);
    assert!(tab.loading);
    assert_eq!(tab.pending_request_id, Some(77));
}

#[test]
fn reopen_rollout_tab_preserves_revision_selection_and_detail_scroll() {
    use crate::k8s::rollout::{RolloutInspection, RolloutRevisionInfo, RolloutWorkloadKind};
    use crate::workbench::{RolloutTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::Deployment("api".into(), "prod".into());
    let mut tab = RolloutTabState::new(resource.clone());
    tab.revisions = vec![
        RolloutRevisionInfo {
            revision: 1,
            name: "rev-1".into(),
            created: None,
            summary: "first".into(),
            change_cause: None,
            is_current: false,
            is_update_target: false,
        },
        RolloutRevisionInfo {
            revision: 2,
            name: "rev-2".into(),
            created: None,
            summary: "second".into(),
            change_cause: None,
            is_current: true,
            is_update_target: true,
        },
    ];
    tab.selected = 1;
    tab.detail_scroll = 6;
    tab.loading = false;
    app.workbench.open_tab(WorkbenchTabState::Rollout(tab));

    app.open_rollout_tab(
        resource,
        Some(RolloutInspection {
            kind: RolloutWorkloadKind::Deployment,
            strategy: "RollingUpdate".into(),
            paused: false,
            current_revision: Some(2),
            update_target_revision: Some(2),
            summary_lines: vec!["healthy".into()],
            conditions: Vec::new(),
            revisions: vec![
                RolloutRevisionInfo {
                    revision: 2,
                    name: "rev-2".into(),
                    created: None,
                    summary: "second".into(),
                    change_cause: None,
                    is_current: true,
                    is_update_target: true,
                },
                RolloutRevisionInfo {
                    revision: 3,
                    name: "rev-3".into(),
                    created: None,
                    summary: "third".into(),
                    change_cause: None,
                    is_current: false,
                    is_update_target: false,
                },
            ],
        }),
        None,
        None,
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing rollout tab");
    };
    let WorkbenchTabState::Rollout(tab) = &tab.state else {
        panic!("expected rollout tab");
    };
    assert_eq!(tab.selected_revision().map(|entry| entry.revision), Some(2));
    assert_eq!(tab.detail_scroll, 6);
}

#[test]
fn reopen_rollout_tab_refresh_clears_stale_undo_confirmation() {
    use crate::workbench::{RolloutTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::Deployment("api".into(), "prod".into());
    let mut tab = RolloutTabState::new(resource.clone());
    tab.confirm_undo_revision = Some(3);
    tab.detail_scroll = 4;
    tab.loading = false;
    app.workbench.open_tab(WorkbenchTabState::Rollout(tab));

    app.open_rollout_tab(resource, None, None, Some(91));

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing rollout tab");
    };
    let WorkbenchTabState::Rollout(tab) = &tab.state else {
        panic!("expected rollout tab");
    };
    assert!(tab.loading);
    assert_eq!(tab.pending_request_id, Some(91));
    assert!(tab.confirm_undo_revision.is_none());
    assert_eq!(tab.detail_scroll, 0);
}

#[test]
fn reopen_rollout_tab_error_clears_stale_payload() {
    use crate::workbench::{RolloutTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::Deployment("api".into(), "prod".into());
    let mut tab = RolloutTabState::new(resource.clone());
    tab.kind = Some(RolloutWorkloadKind::Deployment);
    tab.strategy = Some("RollingUpdate".into());
    tab.summary_lines = vec!["healthy".into()];
    tab.revisions = vec![RolloutRevisionInfo {
        revision: 2,
        name: "rev-2".into(),
        created: None,
        summary: "second".into(),
        change_cause: None,
        is_current: true,
        is_update_target: true,
    }];
    tab.loading = false;
    app.workbench.open_tab(WorkbenchTabState::Rollout(tab));

    app.open_rollout_tab(resource, None, Some("boom".into()), None);

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing rollout tab");
    };
    let WorkbenchTabState::Rollout(tab) = &tab.state else {
        panic!("expected rollout tab");
    };
    assert!(tab.kind.is_none());
    assert!(tab.strategy.is_none());
    assert!(tab.summary_lines.is_empty());
    assert!(tab.revisions.is_empty());
    assert_eq!(tab.error.as_deref(), Some("boom"));
}

#[test]
fn reopen_resource_diff_tab_error_clears_stale_payload() {
    use crate::resource_diff::{ResourceDiffBaselineKind, ResourceDiffLine, ResourceDiffLineKind};
    use crate::workbench::{ResourceDiffTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::Pod("api".into(), "prod".into());
    let mut tab = ResourceDiffTabState::new(resource.clone());
    tab.baseline_kind = Some(ResourceDiffBaselineKind::LastAppliedAnnotation);
    tab.summary = Some("drift".into());
    tab.lines = vec![ResourceDiffLine {
        kind: ResourceDiffLineKind::Context,
        content: "a".into(),
    }];
    tab.loading = false;
    app.workbench.open_tab(WorkbenchTabState::ResourceDiff(tab));

    app.open_resource_diff_tab(resource, None, Some("boom".into()), None);

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing diff tab");
    };
    let WorkbenchTabState::ResourceDiff(tab) = &tab.state else {
        panic!("expected diff tab");
    };
    assert!(tab.baseline_kind.is_none());
    assert!(tab.summary.is_none());
    assert!(tab.lines.is_empty());
    assert_eq!(tab.error.as_deref(), Some("boom"));
}

#[test]
fn reopen_resource_diff_tab_refresh_clears_stale_payload() {
    use crate::resource_diff::{ResourceDiffBaselineKind, ResourceDiffLine, ResourceDiffLineKind};
    use crate::workbench::{ResourceDiffTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::Pod("api".into(), "prod".into());
    let mut tab = ResourceDiffTabState::new(resource.clone());
    tab.baseline_kind = Some(ResourceDiffBaselineKind::LastAppliedAnnotation);
    tab.summary = Some("stale drift".into());
    tab.lines = vec![ResourceDiffLine {
        kind: ResourceDiffLineKind::Context,
        content: "old".into(),
    }];
    tab.scroll = 7;
    tab.loading = false;
    app.workbench.open_tab(WorkbenchTabState::ResourceDiff(tab));

    app.open_resource_diff_tab(resource, None, None, Some(42));

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing diff tab");
    };
    let WorkbenchTabState::ResourceDiff(tab) = &tab.state else {
        panic!("expected diff tab");
    };
    assert!(tab.baseline_kind.is_none());
    assert!(tab.summary.is_none());
    assert!(tab.lines.is_empty());
    assert_eq!(tab.scroll, 0);
    assert!(tab.loading);
    assert_eq!(tab.pending_request_id, Some(42));
    assert!(tab.error.is_none());
}

#[test]
fn reopen_helm_history_tab_error_clears_stale_payload() {
    use crate::k8s::dtos::HelmReleaseRevisionInfo;
    use crate::workbench::{HelmHistoryTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::HelmRelease("release".into(), "prod".into());
    let mut tab = HelmHistoryTabState::new(resource.clone());
    tab.revisions = vec![HelmReleaseRevisionInfo {
        revision: 3,
        ..HelmReleaseRevisionInfo::default()
    }];
    tab.current_revision = Some(3);
    tab.loading = false;
    app.workbench.open_tab(WorkbenchTabState::HelmHistory(tab));

    app.open_helm_history_tab(resource, None, Some("boom".into()), None);

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing helm tab");
    };
    let WorkbenchTabState::HelmHistory(tab) = &tab.state else {
        panic!("expected helm tab");
    };
    assert!(tab.revisions.is_empty());
    assert!(tab.current_revision.is_none());
    assert_eq!(tab.error.as_deref(), Some("boom"));
}

#[test]
fn reopen_helm_history_tab_refresh_clears_stale_confirm_and_diff() {
    use crate::k8s::dtos::HelmReleaseRevisionInfo;
    use crate::workbench::{HelmHistoryTabState, HelmValuesDiffState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::HelmRelease("release".into(), "prod".into());
    let mut tab = HelmHistoryTabState::new(resource.clone());
    tab.revisions = vec![HelmReleaseRevisionInfo {
        revision: 3,
        ..HelmReleaseRevisionInfo::default()
    }];
    tab.current_revision = Some(3);
    tab.loading = false;
    tab.scroll = 7;
    tab.confirm_rollback_revision = Some(2);
    tab.diff = Some(HelmValuesDiffState::new(3, 2, 40));
    app.workbench.open_tab(WorkbenchTabState::HelmHistory(tab));

    app.open_helm_history_tab(resource, None, None, Some(88));

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing helm tab");
    };
    let WorkbenchTabState::HelmHistory(tab) = &tab.state else {
        panic!("expected helm tab");
    };
    assert!(tab.loading);
    assert_eq!(tab.pending_history_request_id, Some(88));
    assert!(tab.confirm_rollback_revision.is_none());
    assert!(tab.diff.is_none());
    assert_eq!(tab.scroll, 0);
}

#[test]
fn reopen_network_policy_tab_error_clears_stale_payload() {
    use crate::workbench::{NetworkPolicyTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::Pod("api".into(), "prod".into());
    let mut tab = NetworkPolicyTabState::new(resource.clone());
    tab.summary_lines = vec!["reachable".into()];
    tab.tree = vec![crate::k8s::relationships::RelationNode {
        resource: None,
        label: "Policy Summary".into(),
        status: None,
        namespace: None,
        relation: crate::k8s::relationships::RelationKind::SectionHeader,
        not_found: false,
        children: Vec::new(),
    }];
    app.workbench
        .open_tab(WorkbenchTabState::NetworkPolicy(tab));

    app.open_network_policy_tab(resource, None, Some("boom".into()));

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing network policy tab");
    };
    let WorkbenchTabState::NetworkPolicy(tab) = &tab.state else {
        panic!("expected network policy tab");
    };
    assert!(tab.summary_lines.is_empty());
    assert!(tab.tree.is_empty());
    assert_eq!(tab.error.as_deref(), Some("boom"));
}

#[test]
fn reopen_traffic_debug_tab_error_clears_stale_payload() {
    use crate::workbench::{TrafficDebugTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::Pod("api".into(), "prod".into());
    let mut tab = TrafficDebugTabState::new(resource.clone());
    tab.summary_lines = vec!["reachable".into()];
    tab.tree = vec![crate::k8s::relationships::RelationNode {
        resource: None,
        label: "Traffic".into(),
        status: None,
        namespace: None,
        relation: crate::k8s::relationships::RelationKind::SectionHeader,
        not_found: false,
        children: Vec::new(),
    }];
    app.workbench.open_tab(WorkbenchTabState::TrafficDebug(tab));

    app.open_traffic_debug_tab(resource, None, Some("boom".into()));

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing traffic tab");
    };
    let WorkbenchTabState::TrafficDebug(tab) = &tab.state else {
        panic!("expected traffic tab");
    };
    assert!(tab.summary_lines.is_empty());
    assert!(tab.tree.is_empty());
    assert_eq!(tab.error.as_deref(), Some("boom"));
}

#[test]
fn reopen_resource_events_tab_preserves_scroll_during_refresh() {
    use crate::k8s::events::EventInfo;
    use crate::time::now;
    use crate::workbench::{ResourceEventsTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::Pod("api".into(), "prod".into());
    let mut tab = ResourceEventsTabState::new(resource.clone());
    tab.events = vec![EventInfo {
        event_type: "Normal".into(),
        reason: "Pulled".into(),
        message: "image".into(),
        first_timestamp: now(),
        last_timestamp: now(),
        count: 1,
    }];
    tab.rebuild_timeline(&app.action_history);
    tab.scroll = 5;
    tab.loading = false;
    app.workbench
        .open_tab(WorkbenchTabState::ResourceEvents(tab));

    app.open_resource_events_tab(resource, Vec::new(), true, None, Some(33));

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing events tab");
    };
    let WorkbenchTabState::ResourceEvents(tab) = &tab.state else {
        panic!("expected events tab");
    };
    assert_eq!(tab.scroll, 5);
    assert!(tab.loading);
    assert_eq!(tab.pending_request_id, Some(33));
}

#[test]
fn reopen_pod_logs_tab_preserves_investigation_settings() {
    use crate::log_investigation::{LogQueryMode, LogTimeWindow};
    use crate::workbench::{PodLogsTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::Pod("api".into(), "prod".into());
    let mut tab = PodLogsTabState::new(resource.clone());
    tab.viewer.container_name = "app".into();
    tab.viewer.previous_logs = true;
    tab.viewer.show_timestamps = true;
    tab.viewer.search_query = "error".into();
    tab.viewer.search_mode = LogQueryMode::Regex;
    tab.viewer.time_window = LogTimeWindow::Last1Hour;
    tab.viewer.structured_view = false;
    app.workbench.open_tab(WorkbenchTabState::PodLogs(tab));

    app.open_pod_logs_tab(resource);

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing pod logs tab");
    };
    let WorkbenchTabState::PodLogs(tab) = &tab.state else {
        panic!("expected pod logs tab");
    };
    assert_eq!(tab.viewer.container_name, "app");
    assert!(tab.viewer.previous_logs);
    assert!(tab.viewer.show_timestamps);
    assert_eq!(tab.viewer.search_query, "error");
    assert_eq!(tab.viewer.search_mode, LogQueryMode::Regex);
    assert_eq!(tab.viewer.time_window, LogTimeWindow::Last1Hour);
    assert!(!tab.viewer.structured_view);
}

#[test]
fn reopen_workload_logs_tab_preserves_filters_while_resetting_session() {
    use crate::log_investigation::{LogQueryMode, LogTimeWindow};
    use crate::workbench::{WorkbenchTabState, WorkloadLogsTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::Deployment("api".into(), "prod".into());
    let mut tab = WorkloadLogsTabState::new(resource.clone(), 7);
    tab.text_filter = "timeout".into();
    tab.filter_input = "timeout".into();
    tab.text_filter_mode = LogQueryMode::Regex;
    tab.time_window = LogTimeWindow::Last15Minutes;
    tab.structured_view = false;
    tab.label_filter = Some("app=api".into());
    tab.pod_filter = Some("api-0".into());
    tab.container_filter = Some("main".into());
    tab.follow_mode = false;
    tab.lines.push(crate::workbench::WorkloadLogLine {
        pod_name: "api-0".into(),
        container_name: "main".into(),
        entry: crate::log_investigation::LogEntry::from_raw("hello"),
        is_stderr: false,
    });
    app.workbench.open_tab(WorkbenchTabState::WorkloadLogs(tab));

    app.open_workload_logs_tab(resource, 99);

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing workload logs tab");
    };
    let WorkbenchTabState::WorkloadLogs(tab) = &tab.state else {
        panic!("expected workload logs tab");
    };
    assert_eq!(tab.session_id, 99);
    assert!(tab.lines.is_empty());
    assert_eq!(tab.text_filter, "timeout");
    assert_eq!(tab.filter_input, "timeout");
    assert_eq!(tab.text_filter_mode, LogQueryMode::Regex);
    assert_eq!(tab.time_window, LogTimeWindow::Last15Minutes);
    assert!(!tab.structured_view);
    assert_eq!(tab.label_filter.as_deref(), Some("app=api"));
    assert_eq!(tab.pod_filter.as_deref(), Some("api-0"));
    assert_eq!(tab.container_filter.as_deref(), Some("main"));
    assert!(!tab.follow_mode);
    assert!(tab.loading);
}

#[test]
fn pod_logs_modified_escape_does_not_cancel_search_or_time_jump() {
    use crate::workbench::{PodLogsTabState, WorkbenchTabState};

    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::META,
        KeyModifiers::SUPER,
        KeyModifiers::CONTROL | KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::SUPER,
    ] {
        let mut search = AppState::default();
        search.focus = Focus::Workbench;
        let mut tab = PodLogsTabState::new(ResourceRef::Pod("api".into(), "prod".into()));
        tab.viewer.searching = true;
        tab.viewer.search_input = "error".into();
        tab.viewer.search_cursor = tab.viewer.search_input.len();
        search.workbench.open_tab(WorkbenchTabState::PodLogs(tab));
        assert_eq!(
            search.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );
        let WorkbenchTabState::PodLogs(tab) = &search.workbench.active_tab().unwrap().state else {
            panic!("expected pod logs tab");
        };
        assert!(tab.viewer.searching, "{modifiers:?}");
        assert_eq!(tab.viewer.search_input, "error", "{modifiers:?}");

        let mut jump = AppState::default();
        jump.focus = Focus::Workbench;
        let mut tab = PodLogsTabState::new(ResourceRef::Pod("api".into(), "prod".into()));
        tab.viewer.jumping_to_time = true;
        tab.viewer.time_jump_input = "10:30".into();
        tab.viewer.time_jump_cursor = tab.viewer.time_jump_input.len();
        jump.workbench.open_tab(WorkbenchTabState::PodLogs(tab));
        assert_eq!(
            jump.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );
        let WorkbenchTabState::PodLogs(tab) = &jump.workbench.active_tab().unwrap().state else {
            panic!("expected pod logs tab");
        };
        assert!(tab.viewer.jumping_to_time, "{modifiers:?}");
        assert_eq!(tab.viewer.time_jump_input, "10:30", "{modifiers:?}");
    }
}

#[test]
fn pod_logs_modified_edit_keys_do_not_mutate_search_or_time_jump() {
    use crate::workbench::{PodLogsTabState, WorkbenchTabState};

    for key in modified_edit_key_events() {
        let mut search = AppState::default();
        search.focus = Focus::Workbench;
        let mut tab = PodLogsTabState::new(ResourceRef::Pod("api".into(), "prod".into()));
        tab.viewer.searching = true;
        tab.viewer.search_input = "error".into();
        tab.viewer.search_cursor = 2;
        search.workbench.open_tab(WorkbenchTabState::PodLogs(tab));
        assert_eq!(search.handle_key_event(key), AppAction::None, "{key:?}");
        let WorkbenchTabState::PodLogs(tab) = &search.workbench.active_tab().unwrap().state else {
            panic!("expected pod logs tab");
        };
        assert_eq!(tab.viewer.search_input, "error", "{key:?}");
        assert_eq!(tab.viewer.search_cursor, 2, "{key:?}");

        let mut jump = AppState::default();
        jump.focus = Focus::Workbench;
        let mut tab = PodLogsTabState::new(ResourceRef::Pod("api".into(), "prod".into()));
        tab.viewer.jumping_to_time = true;
        tab.viewer.time_jump_input = "10:30".into();
        tab.viewer.time_jump_cursor = 2;
        jump.workbench.open_tab(WorkbenchTabState::PodLogs(tab));
        assert_eq!(jump.handle_key_event(key), AppAction::None, "{key:?}");
        let WorkbenchTabState::PodLogs(tab) = &jump.workbench.active_tab().unwrap().state else {
            panic!("expected pod logs tab");
        };
        assert_eq!(tab.viewer.time_jump_input, "10:30", "{key:?}");
        assert_eq!(tab.viewer.time_jump_cursor, 2, "{key:?}");
    }
}

#[test]
fn workload_logs_modified_escape_does_not_cancel_filter_or_time_jump() {
    use crate::workbench::{WorkbenchTabState, WorkloadLogsTabState};

    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::META,
        KeyModifiers::SUPER,
        KeyModifiers::CONTROL | KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::SUPER,
    ] {
        let mut filter = AppState::default();
        filter.focus = Focus::Workbench;
        let mut tab =
            WorkloadLogsTabState::new(ResourceRef::Deployment("api".into(), "prod".into()), 7);
        tab.editing_text_filter = true;
        tab.filter_input = "timeout".into();
        tab.filter_input_cursor = tab.filter_input.len();
        filter
            .workbench
            .open_tab(WorkbenchTabState::WorkloadLogs(tab));
        assert_eq!(
            filter.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );
        let WorkbenchTabState::WorkloadLogs(tab) = &filter.workbench.active_tab().unwrap().state
        else {
            panic!("expected workload logs tab");
        };
        assert!(tab.editing_text_filter, "{modifiers:?}");
        assert_eq!(tab.filter_input, "timeout", "{modifiers:?}");

        let mut jump = AppState::default();
        jump.focus = Focus::Workbench;
        let mut tab =
            WorkloadLogsTabState::new(ResourceRef::Deployment("api".into(), "prod".into()), 7);
        tab.jumping_to_time = true;
        tab.time_jump_input = "10:30".into();
        tab.time_jump_cursor = tab.time_jump_input.len();
        jump.workbench
            .open_tab(WorkbenchTabState::WorkloadLogs(tab));
        assert_eq!(
            jump.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );
        let WorkbenchTabState::WorkloadLogs(tab) = &jump.workbench.active_tab().unwrap().state
        else {
            panic!("expected workload logs tab");
        };
        assert!(tab.jumping_to_time, "{modifiers:?}");
        assert_eq!(tab.time_jump_input, "10:30", "{modifiers:?}");
    }
}

#[test]
fn workload_logs_modified_edit_keys_do_not_mutate_filter_or_time_jump() {
    use crate::workbench::{WorkbenchTabState, WorkloadLogsTabState};

    for key in modified_edit_key_events() {
        let mut filter = AppState::default();
        filter.focus = Focus::Workbench;
        let mut tab =
            WorkloadLogsTabState::new(ResourceRef::Deployment("api".into(), "prod".into()), 7);
        tab.editing_text_filter = true;
        tab.filter_input = "timeout".into();
        tab.filter_input_cursor = 3;
        filter
            .workbench
            .open_tab(WorkbenchTabState::WorkloadLogs(tab));
        assert_eq!(filter.handle_key_event(key), AppAction::None, "{key:?}");
        let WorkbenchTabState::WorkloadLogs(tab) = &filter.workbench.active_tab().unwrap().state
        else {
            panic!("expected workload logs tab");
        };
        assert_eq!(tab.filter_input, "timeout", "{key:?}");
        assert_eq!(tab.filter_input_cursor, 3, "{key:?}");

        let mut jump = AppState::default();
        jump.focus = Focus::Workbench;
        let mut tab =
            WorkloadLogsTabState::new(ResourceRef::Deployment("api".into(), "prod".into()), 7);
        tab.jumping_to_time = true;
        tab.time_jump_input = "10:30".into();
        tab.time_jump_cursor = 2;
        jump.workbench
            .open_tab(WorkbenchTabState::WorkloadLogs(tab));
        assert_eq!(jump.handle_key_event(key), AppAction::None, "{key:?}");
        let WorkbenchTabState::WorkloadLogs(tab) = &jump.workbench.active_tab().unwrap().state
        else {
            panic!("expected workload logs tab");
        };
        assert_eq!(tab.time_jump_input, "10:30", "{key:?}");
        assert_eq!(tab.time_jump_cursor, 2, "{key:?}");
    }
}

#[test]
fn workload_logs_single_filtered_line_allows_row_scroll_offsets() {
    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let mut tab = crate::workbench::WorkloadLogsTabState::new(
        ResourceRef::Deployment("api".into(), "prod".into()),
        7,
    );
    tab.loading = false;
    tab.follow_mode = false;
    tab.lines.push(crate::workbench::WorkloadLogLine {
        pod_name: "api-0".into(),
        container_name: "main".into(),
        entry: crate::log_investigation::LogEntry::from_raw("single visible line"),
        is_stderr: false,
    });
    app.workbench.open_tab(WorkbenchTabState::WorkloadLogs(tab));

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::None);
    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing workload logs tab");
    };
    let WorkbenchTabState::WorkloadLogs(tab) = &tab.state else {
        panic!("expected workload logs tab");
    };
    assert_eq!(tab.scroll, 1);

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::None);
    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing workload logs tab");
    };
    let WorkbenchTabState::WorkloadLogs(tab) = &tab.state else {
        panic!("expected workload logs tab");
    };
    assert_eq!(tab.scroll, 0);
}

#[test]
fn reopen_exec_tab_preserves_selected_container_while_resetting_session() {
    use crate::workbench::{ExecTabState, WorkbenchTabState};

    let mut app = AppState::default();
    app.focus = Focus::Workbench;
    let resource = ResourceRef::Pod("api".into(), "prod".into());
    let mut tab = ExecTabState::new(resource.clone(), 7, "api".into(), "prod".into());
    tab.container_name = "sidecar".into();
    tab.containers = vec!["main".into(), "sidecar".into()];
    tab.lines = vec!["old output".into()];
    tab.scroll = 4;
    tab.loading = false;
    app.workbench.open_tab(WorkbenchTabState::Exec(tab));

    app.open_exec_tab(resource, 19, "api".into(), "prod".into());

    let Some(tab) = app.workbench.active_tab() else {
        panic!("missing exec tab");
    };
    let WorkbenchTabState::Exec(tab) = &tab.state else {
        panic!("expected exec tab");
    };
    assert_eq!(tab.session_id, 19);
    assert_eq!(tab.container_name, "sidecar");
    assert!(tab.containers.is_empty());
    assert!(tab.lines.is_empty());
    assert_eq!(tab.scroll, 0);
    assert!(tab.loading);
}

#[test]
fn exec_picker_modified_escape_does_not_cancel_container_picker() {
    use crate::workbench::{ExecTabState, WorkbenchTabState};

    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::META,
        KeyModifiers::SUPER,
        KeyModifiers::CONTROL | KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::SUPER,
    ] {
        let mut app = AppState::default();
        app.focus = Focus::Workbench;
        let resource = ResourceRef::Pod("api".into(), "prod".into());
        let mut tab = ExecTabState::new(resource.clone(), 7, "api".into(), "prod".into());
        tab.containers = vec!["main".into(), "sidecar".into()];
        tab.picking_container = true;
        tab.container_cursor = 1;
        app.workbench.open_tab(WorkbenchTabState::Exec(tab));

        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );

        let Some(tab) = app.workbench.active_tab() else {
            panic!("missing exec tab");
        };
        let WorkbenchTabState::Exec(tab) = &tab.state else {
            panic!("expected exec tab");
        };
        assert!(tab.picking_container, "{modifiers:?}");
        assert_eq!(tab.container_cursor, 1, "{modifiers:?}");
    }
}

#[test]
fn exec_input_modified_edit_keys_do_not_mutate_input_cursor_or_scroll() {
    use crate::workbench::{ExecTabState, WorkbenchTabState};

    let keys = modified_edit_key_events().into_iter().chain([
        KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Down, KeyModifiers::ALT),
    ]);

    for key in keys {
        let mut app = AppState::default();
        app.focus = Focus::Workbench;
        let resource = ResourceRef::Pod("api".into(), "prod".into());
        let mut tab = ExecTabState::new(resource.clone(), 7, "api".into(), "prod".into());
        tab.input = "kubectl".into();
        tab.input_cursor = 3;
        tab.lines = vec!["one".into(), "two".into(), "three".into()];
        tab.scroll = 1;
        app.workbench.open_tab(WorkbenchTabState::Exec(tab));

        assert_eq!(app.handle_key_event(key), AppAction::None, "{key:?}");

        let WorkbenchTabState::Exec(tab) = &app.workbench.active_tab().unwrap().state else {
            panic!("expected exec tab");
        };
        assert_eq!(tab.input, "kubectl", "{key:?}");
        assert_eq!(tab.input_cursor, 3, "{key:?}");
        assert_eq!(tab.scroll, 1, "{key:?}");
    }
}

#[test]
fn u_key_does_not_uncordon_for_pod_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    });
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
    assert_ne!(action, AppAction::UncordonNode);
}

#[test]
fn y_key_in_drain_confirm_dispatches_drain_not_yaml() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        confirm_drain: true,
        ..DetailViewState::default()
    });
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::DrainNode);
}

#[test]
fn ctrl_y_does_not_confirm_drain_dialog() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        confirm_drain: true,
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
    assert_eq!(action, AppAction::None);
    assert!(app.detail_view.as_ref().unwrap().confirm_drain);
}

#[test]
fn modified_confirmation_keys_do_not_execute_dialog_actions() {
    for code in [
        KeyCode::Char('y'),
        KeyCode::Char('D'),
        KeyCode::Char('F'),
        KeyCode::Enter,
    ] {
        let mut drain = AppState::default();
        drain.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            yaml: Some("kind: Node".to_string()),
            confirm_drain: true,
            ..DetailViewState::default()
        });
        assert_eq!(
            drain.handle_key_event(KeyEvent::new(code, KeyModifiers::ALT)),
            AppAction::None,
            "{code:?}"
        );
        assert!(
            drain.detail_view.as_ref().unwrap().confirm_drain,
            "{code:?}"
        );
    }

    for code in [
        KeyCode::Char('d'),
        KeyCode::Char('D'),
        KeyCode::Char('F'),
        KeyCode::Char('y'),
        KeyCode::Enter,
    ] {
        let mut delete = AppState::default();
        delete.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
            yaml: Some("kind: Pod".to_string()),
            confirm_delete: true,
            ..DetailViewState::default()
        });
        assert_eq!(
            delete.handle_key_event(KeyEvent::new(code, KeyModifiers::ALT)),
            AppAction::None,
            "{code:?}"
        );
        assert!(
            delete.detail_view.as_ref().unwrap().confirm_delete,
            "{code:?}"
        );
    }

    for code in [KeyCode::Char('S'), KeyCode::Char('y'), KeyCode::Enter] {
        let mut cron = AppState::default();
        cron.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::CronJob("job-0".to_string(), "ns".to_string())),
            yaml: Some("kind: CronJob".to_string()),
            confirm_cronjob_suspend: Some(true),
            ..DetailViewState::default()
        });
        assert_eq!(
            cron.handle_key_event(KeyEvent::new(code, KeyModifiers::ALT | KeyModifiers::SHIFT)),
            AppAction::None,
            "{code:?}"
        );
        assert_eq!(
            cron.detail_view.as_ref().unwrap().confirm_cronjob_suspend,
            Some(true),
            "{code:?}"
        );
    }
}

#[test]
fn modified_confirmation_escape_does_not_cancel_dialog_actions() {
    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::META,
        KeyModifiers::SUPER,
        KeyModifiers::CONTROL | KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::SUPER,
    ] {
        let mut drain = AppState::default();
        drain.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            yaml: Some("kind: Node".to_string()),
            confirm_drain: true,
            ..DetailViewState::default()
        });
        assert_eq!(
            drain.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );
        assert!(
            drain.detail_view.as_ref().unwrap().confirm_drain,
            "{modifiers:?}"
        );

        let mut delete = AppState::default();
        delete.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
            yaml: Some("kind: Pod".to_string()),
            confirm_delete: true,
            ..DetailViewState::default()
        });
        assert_eq!(
            delete.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );
        assert!(
            delete.detail_view.as_ref().unwrap().confirm_delete,
            "{modifiers:?}"
        );

        let mut cron = AppState::default();
        cron.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::CronJob("job-0".to_string(), "ns".to_string())),
            yaml: Some("kind: CronJob".to_string()),
            confirm_cronjob_suspend: Some(true),
            ..DetailViewState::default()
        });
        assert_eq!(
            cron.handle_key_event(KeyEvent::new(KeyCode::Esc, modifiers)),
            AppAction::None,
            "{modifiers:?}"
        );
        assert_eq!(
            cron.detail_view.as_ref().unwrap().confirm_cronjob_suspend,
            Some(true),
            "{modifiers:?}"
        );
    }
}

#[test]
fn palette_blocked_during_drain_confirm() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        confirm_drain: true,
        ..DetailViewState::default()
    });
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE));
    assert_ne!(action, AppAction::OpenCommandPalette);
}

#[test]
fn y_key_blocked_during_drain_confirm_does_not_open_yaml() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        confirm_drain: true,
        ..DetailViewState::default()
    });
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert_ne!(action, AppAction::OpenResourceYaml);
}

#[test]
fn metadata_toggle_blocked_during_drain_confirm() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Node("node-0".to_string())),
        yaml: Some("kind: Node".to_string()),
        confirm_drain: true,
        ..DetailViewState::default()
    });
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::None);
}

#[test]
fn o_key_opens_decoded_secret_in_secret_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Secret(
            "app-secret".to_string(),
            "default".to_string(),
        )),
        yaml: Some("kind: Secret".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::OpenDecodedSecret);
}

#[test]
fn o_key_opens_decoded_secret_from_secrets_list() {
    let mut app = AppState::default();
    app.view = AppView::Secrets;
    app.focus = Focus::Content;

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::OpenDecodedSecret);
}

#[test]
fn o_key_does_not_open_decoded_secret_for_non_secret_detail() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::ConfigMap(
            "app-config".to_string(),
            "default".to_string(),
        )),
        yaml: Some("kind: ConfigMap".to_string()),
        ..DetailViewState::default()
    });

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::None);
}

#[test]
fn uppercase_b_toggles_bookmark_for_selected_resource() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
    app.focus = Focus::Content;

    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('B'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::ToggleBookmark);
}

#[test]
fn ctrl_shift_b_does_not_toggle_bookmark_for_selected_resource() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
    app.focus = Focus::Content;

    let action = app.handle_key_event(KeyEvent::new(
        KeyCode::Char('B'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(action, AppAction::None);
}

#[test]
fn toggle_bookmark_persists_per_current_context() {
    let mut app = AppState::default();
    app.current_context_name = Some("prod".to_string());

    let result = app
        .toggle_bookmark(ResourceRef::Secret(
            "app-secret".to_string(),
            "default".to_string(),
        ))
        .expect("bookmark added");
    assert_eq!(result, BookmarkToggleResult::Added);
    assert_eq!(app.bookmark_count(), 1);
    assert!(app.is_bookmarked(&ResourceRef::Secret(
        "app-secret".to_string(),
        "default".to_string(),
    )));
    assert!(app.needs_config_save);
}

#[test]
fn apply_sort_from_preferences_pods() {
    use crate::preferences::{UserPreferences, ViewPreferences};
    let mut app = AppState::default();
    let mut global = UserPreferences::default();
    global.views.insert(
        "pods".into(),
        ViewPreferences {
            sort_column: Some("restarts".into()),
            sort_ascending: false,
            ..Default::default()
        },
    );
    app.preferences = Some(global);
    app.apply_sort_from_preferences("pods");
    let sort = app.pod_sort.unwrap();
    assert_eq!(sort.column, PodSortColumn::Restarts);
    assert!(sort.descending);
}

#[test]
fn apply_sort_from_preferences_workload() {
    use crate::preferences::{UserPreferences, ViewPreferences};
    let mut app = AppState::default();
    let mut global = UserPreferences::default();
    global.views.insert(
        "deployments".into(),
        ViewPreferences {
            sort_column: Some("age".into()),
            sort_ascending: true,
            ..Default::default()
        },
    );
    app.preferences = Some(global);
    app.apply_sort_from_preferences("deployments");
    let sort = app.workload_sort.unwrap();
    assert_eq!(sort.column, WorkloadSortColumn::Age);
    assert!(!sort.descending);
}

#[test]
fn apply_sort_invalid_column_ignored() {
    use crate::preferences::{UserPreferences, ViewPreferences};
    let mut app = AppState::default();
    let mut global = UserPreferences::default();
    global.views.insert(
        "pods".into(),
        ViewPreferences {
            sort_column: Some("nonexistent".into()),
            ..Default::default()
        },
    );
    app.preferences = Some(global);
    app.apply_sort_from_preferences("pods");
    assert!(app.pod_sort.is_none());
}

#[test]
fn save_sort_to_preferences_round_trip() {
    let mut app = AppState::default();
    app.pod_sort = Some(PodSortState::new(PodSortColumn::Status, false));
    app.save_sort_to_preferences("pods");
    let prefs = app.preferences.as_ref().unwrap();
    let vp = prefs.views.get("pods").unwrap();
    assert_eq!(vp.sort_column.as_deref(), Some("status"));
    assert!(vp.sort_ascending); // descending=false → ascending=true
    assert!(app.needs_config_save);
}

#[test]
fn clear_sort_removes_from_preferences() {
    use crate::preferences::{UserPreferences, ViewPreferences};
    let mut app = AppState::default();
    let mut global = UserPreferences::default();
    global.views.insert(
        "pods".into(),
        ViewPreferences {
            sort_column: Some("age".into()),
            ..Default::default()
        },
    );
    app.preferences = Some(global);
    app.pod_sort = None;
    app.save_sort_to_preferences("pods");
    let vp = app.preferences.as_ref().unwrap().views.get("pods").unwrap();
    assert!(vp.sort_column.is_none());
}

#[test]
fn save_sort_creates_cluster_preferences_for_active_context() {
    let mut app = AppState::default();
    app.current_context_name = Some("prod".to_string());
    app.preferences = Some(UserPreferences::default());
    app.pod_sort = Some(PodSortState::new(PodSortColumn::Status, false));

    app.save_sort_to_preferences("pods");

    let global = app.preferences.as_ref().unwrap();
    assert!(!global.views.contains_key("pods"));

    let clusters = app.cluster_preferences.as_ref().unwrap();
    let prod = clusters.get("prod").unwrap();
    let pod_prefs = prod.views.get("pods").unwrap();
    assert_eq!(pod_prefs.sort_column.as_deref(), Some("status"));
    assert!(pod_prefs.sort_ascending);
}

#[test]
fn toggle_default_hidden_column_uses_shown_columns() {
    let mut app = AppState::default();
    app.navigate_to_view(AppView::Pods);

    app.toggle_column_visibility("cpu_usage");

    let prefs = app.preferences.as_ref().unwrap();
    let pod_prefs = prefs.views.get("pods").unwrap();
    assert_eq!(pod_prefs.shown_columns, vec!["cpu_usage"]);
    assert!(pod_prefs.hidden_columns.is_empty());
}

#[test]
fn config_round_trip_with_preferences() {
    use crate::{
        icons::IconMode,
        log_investigation::{LogQueryMode, PodLogPreset},
        preferences::{ClusterPreferences, LogPresetPreferences, UserPreferences, ViewPreferences},
        workspaces::{
            HotkeyAction, HotkeyBinding, HotkeyTarget, SavedWorkspace, WorkspaceBank,
            WorkspacePreferences, WorkspaceSnapshot,
        },
    };
    let _icon_mode_lock = crate::icons::icon_mode_test_lock();
    let path = std::env::temp_dir().join("kubectui_test_config_prefs.json");

    let mut app = AppState::default();
    let mut global = UserPreferences::default();
    global.views.insert(
        "pods".into(),
        ViewPreferences {
            sort_column: Some("restarts".into()),
            sort_ascending: false,
            hidden_columns: vec!["namespace".into()],
            ..Default::default()
        },
    );
    app.preferences = Some(global);
    app.preferences.as_mut().unwrap().log_presets = LogPresetPreferences {
        pod_logs: vec![PodLogPreset {
            name: "errors".into(),
            query: "error".into(),
            mode: LogQueryMode::Regex,
            time_window: crate::log_investigation::LogTimeWindow::Last15Minutes,
            structured_view: false,
        }],
        workload_logs: Vec::new(),
    };
    app.preferences.as_mut().unwrap().workspaces = WorkspacePreferences {
        saved: vec![SavedWorkspace {
            name: "prod pods".into(),
            snapshot: WorkspaceSnapshot {
                context: Some("prod".into()),
                namespace: "payments".into(),
                view: AppView::Pods,
                search_query: Some("checkout".into()),
                collapsed_groups: vec![NavGroup::FluxCD],
                workbench_open: true,
                workbench_height: 15,
                workbench_maximized: false,
                action_history_tab: true,
            },
        }],
        banks: vec![WorkspaceBank {
            name: "ops services".into(),
            context: Some("staging".into()),
            namespace: "ops".into(),
            view: AppView::Services,
            search_query: Some("payments".into()),
            hotkey: Some("alt+2".into()),
        }],
        hotkeys: vec![HotkeyBinding {
            key: "alt+r".into(),
            target: HotkeyTarget::Action {
                action: HotkeyAction::RefreshData,
            },
        }],
    };

    let mut cluster_prefs = ClusterPreferences::default();
    cluster_prefs.views.insert(
        "pods".into(),
        ViewPreferences {
            sort_column: Some("status".into()),
            ..Default::default()
        },
    );
    cluster_prefs.bookmarks.push(BookmarkEntry {
        resource: ResourceRef::Secret("app-secret".to_string(), "default".to_string()),
        bookmarked_at_unix: 42,
    });
    let mut clusters = HashMap::new();
    clusters.insert("prod".into(), cluster_prefs);
    app.cluster_preferences = Some(clusters);

    app.collapsed_groups.remove(&NavGroup::Workloads);
    app.collapsed_groups.insert(NavGroup::FluxCD);
    app.collapsed_groups.insert(NavGroup::AccessControl);
    crate::icons::set_icon_mode(IconMode::Plain);

    save_config_to_path(&app, &path);
    crate::icons::set_icon_mode(IconMode::Nerd);
    let loaded = load_config_from_path(&path);

    let prefs = loaded.preferences.as_ref().unwrap();
    let pod_prefs = prefs.views.get("pods").unwrap();
    assert_eq!(pod_prefs.sort_column.as_deref(), Some("restarts"));
    assert!(!pod_prefs.sort_ascending);
    assert_eq!(pod_prefs.hidden_columns, vec!["namespace"]);
    assert_eq!(prefs.log_presets.pod_logs.len(), 1);
    assert_eq!(prefs.log_presets.pod_logs[0].query, "error");
    assert_eq!(prefs.workspaces.saved.len(), 1);
    assert_eq!(prefs.workspaces.saved[0].name, "prod pods");
    assert_eq!(
        prefs.workspaces.saved[0].snapshot.context.as_deref(),
        Some("prod")
    );
    assert_eq!(prefs.workspaces.saved[0].snapshot.namespace, "payments");
    assert_eq!(prefs.workspaces.saved[0].snapshot.view, AppView::Pods);
    assert_eq!(
        prefs.workspaces.saved[0].snapshot.search_query.as_deref(),
        Some("checkout")
    );
    assert_eq!(prefs.workspaces.banks.len(), 1);
    assert_eq!(prefs.workspaces.banks[0].name, "ops services");
    assert_eq!(
        prefs.workspaces.banks[0].search_query.as_deref(),
        Some("payments")
    );
    assert_eq!(prefs.workspaces.banks[0].hotkey.as_deref(), Some("alt+2"));
    assert_eq!(prefs.workspaces.hotkeys.len(), 1);
    assert_eq!(prefs.workspaces.hotkeys[0].key, "alt+r");
    assert!(matches!(
        prefs.workspaces.hotkeys[0].target,
        HotkeyTarget::Action {
            action: HotkeyAction::RefreshData
        }
    ));

    let clusters = loaded.cluster_preferences.as_ref().unwrap();
    let prod = clusters.get("prod").unwrap();
    let prod_pods = prod.views.get("pods").unwrap();
    assert_eq!(prod_pods.sort_column.as_deref(), Some("status"));
    assert_eq!(prod.bookmarks.len(), 1);
    assert_eq!(prod.bookmarks[0].bookmarked_at_unix, 42);

    assert_eq!(crate::icons::active_icon_mode(), IconMode::Plain);
    assert!(!loaded.collapsed_groups.contains(&NavGroup::Overview));
    assert!(!loaded.collapsed_groups.contains(&NavGroup::Workloads));
    assert!(loaded.collapsed_groups.contains(&NavGroup::FluxCD));
    assert!(loaded.collapsed_groups.contains(&NavGroup::AccessControl));

    crate::icons::set_icon_mode(IconMode::Nerd);
}

#[test]
fn config_backward_compat_no_prefs() {
    let path = std::env::temp_dir().join("kubectui_test_config_compat.json");
    std::fs::write(
        &path,
        r#"{"namespace":"default","workbench_open":true,"workbench_height":14}"#,
    )
    .unwrap();
    let loaded = load_config_from_path(&path);
    assert!(loaded.preferences.is_none());
    assert!(loaded.cluster_preferences.is_none());
    assert!(!loaded.workbench.open);
    assert_eq!(loaded.workbench.height, 14);
    // All groups collapsed except Overview (default view's group).
    assert!(!loaded.collapsed_groups.contains(&NavGroup::Overview));
    assert!(loaded.collapsed_groups.contains(&NavGroup::Workloads));
}

#[test]
fn apply_workspace_snapshot_reopens_active_group() {
    let mut app = AppState::default();
    let snapshot = WorkspaceSnapshot {
        context: Some("prod".into()),
        namespace: "payments".into(),
        view: AppView::Pods,
        search_query: Some("checkout".into()),
        collapsed_groups: sidebar::all_groups()
            .filter(|group| *group != NavGroup::Workloads)
            .collect(),
        workbench_open: false,
        workbench_height: 15,
        workbench_maximized: false,
        action_history_tab: false,
    };

    app.apply_workspace_snapshot(&snapshot);

    assert_eq!(app.view(), AppView::Pods);
    assert_eq!(app.search_query(), "checkout");
    assert!(!app.collapsed_groups.contains(&NavGroup::Workloads));
    assert!(app.collapsed_groups.contains(&NavGroup::Network));
    assert_eq!(
        sidebar_rows(&app.collapsed_groups)[app.sidebar_cursor],
        SidebarItem::View(AppView::Pods)
    );
}

#[test]
fn apply_workspace_snapshot_normalizes_text_fields() {
    let mut app = AppState::default();
    let snapshot = WorkspaceSnapshot {
        context: Some(" prod ".into()),
        namespace: " payments ".into(),
        view: AppView::Pods,
        search_query: Some(" checkout ".into()),
        collapsed_groups: Vec::new(),
        workbench_open: false,
        workbench_height: 15,
        workbench_maximized: false,
        action_history_tab: false,
    };

    app.apply_workspace_snapshot(&snapshot);

    assert_eq!(app.get_namespace(), "payments");
    assert_eq!(app.search_query(), "checkout");
}

#[test]
fn apply_workspace_snapshot_falls_back_for_blank_text_fields() {
    let mut app = AppState::default();
    let snapshot = WorkspaceSnapshot {
        context: Some("  ".into()),
        namespace: "  ".into(),
        view: AppView::Pods,
        search_query: Some("  ".into()),
        collapsed_groups: Vec::new(),
        workbench_open: false,
        workbench_height: 15,
        workbench_maximized: false,
        action_history_tab: false,
    };

    app.apply_workspace_snapshot(&snapshot);

    assert_eq!(app.get_namespace(), "all");
    assert!(app.search_query().is_empty());
}

#[test]
fn apply_workspace_snapshot_clears_selection_search_status() {
    let mut app = AppState {
        search_query: "stale".to_string(),
        search_cursor: "stale".chars().count(),
        is_search_mode: true,
        ..AppState::default()
    };
    app.set_status(SELECTION_SEARCH_FALLBACK_STATUS.to_string());
    let snapshot = WorkspaceSnapshot {
        context: Some("prod".into()),
        namespace: "payments".into(),
        view: AppView::Pods,
        search_query: Some("checkout".into()),
        collapsed_groups: Vec::new(),
        workbench_open: false,
        workbench_height: 15,
        workbench_maximized: false,
        action_history_tab: false,
    };

    app.apply_workspace_snapshot(&snapshot);

    assert_eq!(app.search_query(), "checkout");
    assert!(!app.is_search_mode());
    assert_eq!(app.status_message(), None);
}

#[test]
fn apply_workspace_snapshot_preserves_unrelated_status() {
    let mut app = AppState::default();
    app.set_status("Saved workspace: ops".to_string());
    let snapshot = WorkspaceSnapshot {
        context: Some("prod".into()),
        namespace: "payments".into(),
        view: AppView::Pods,
        search_query: None,
        collapsed_groups: Vec::new(),
        workbench_open: false,
        workbench_height: 15,
        workbench_maximized: false,
        action_history_tab: false,
    };

    app.apply_workspace_snapshot(&snapshot);

    assert!(app.search_query().is_empty());
    assert_eq!(app.status_message(), Some("Saved workspace: ops"));
}

#[test]
fn apply_workspace_snapshot_keeps_workbench_closed_without_restored_tabs() {
    let mut app = AppState::default();
    app.workbench
        .open_tab(WorkbenchTabState::ActionHistory(Default::default()));
    app.workbench.open = true;
    app.workbench.maximized = true;

    let snapshot = WorkspaceSnapshot {
        context: Some("prod".into()),
        namespace: "payments".into(),
        view: AppView::Pods,
        search_query: None,
        collapsed_groups: Vec::new(),
        workbench_open: true,
        workbench_height: 15,
        workbench_maximized: true,
        action_history_tab: false,
    };

    app.apply_workspace_snapshot(&snapshot);

    assert!(!app.workbench.open);
    assert!(!app.workbench.maximized);
    assert!(app.workbench.tabs.is_empty());
    assert_eq!(app.workbench.height, 15);
}

#[test]
fn apply_workspace_snapshot_records_recent_view_jump_in_restored_scope() {
    let mut app = AppState::default();
    app.current_context_name = Some("prod".into());
    app.current_namespace = "default".into();
    app.view = AppView::Dashboard;

    let snapshot = WorkspaceSnapshot {
        context: Some("prod".into()),
        namespace: "payments".into(),
        view: AppView::Pods,
        search_query: None,
        collapsed_groups: Vec::new(),
        workbench_open: false,
        workbench_height: 15,
        workbench_maximized: false,
        action_history_tab: false,
    };

    app.apply_workspace_snapshot(&snapshot);

    let recent = app.recent_jumps().front().expect("recent jump");
    assert_eq!(recent.target, RecentJumpTarget::View(AppView::Pods));
    assert_eq!(
        recent.scope,
        ActivityScope {
            context: Some("prod".into()),
            namespace: "payments".into(),
        }
    );
}

#[test]
fn apply_workspace_snapshot_resets_secondary_pane_focus_and_scroll() {
    let mut app = AppState::default();
    app.view = AppView::Governance;
    app.focus = Focus::Content;
    app.content_detail_scroll = 17;
    app.content_pane_focus = ContentPaneFocus::Secondary;

    let snapshot = WorkspaceSnapshot {
        context: Some("prod".into()),
        namespace: "payments".into(),
        view: AppView::Projects,
        search_query: Some("checkout".into()),
        collapsed_groups: Vec::new(),
        workbench_open: false,
        workbench_height: 15,
        workbench_maximized: false,
        action_history_tab: false,
    };

    app.apply_workspace_snapshot(&snapshot);

    assert_eq!(app.content_detail_scroll, 0);
    assert_eq!(app.content_pane_focus(), ContentPaneFocus::List);
    assert!(!app.content_secondary_pane_active());
}

#[test]
fn save_and_cycle_pod_log_presets_round_trip() {
    use crate::events::input::apply_action;
    use crate::log_investigation::{LogEntry, LogQueryMode};

    let mut app = AppState::default();
    let mut tab = PodLogsTabState::new(ResourceRef::Pod("pod-0".into(), "default".into()));
    tab.viewer.lines = vec![
        LogEntry::from_raw("boot"),
        LogEntry::from_raw(r#"{"level":"error","message":"request failed"}"#),
    ];
    tab.viewer.search_query = "request".into();
    tab.viewer.search_mode = LogQueryMode::Regex;
    tab.viewer.compiled_search =
        crate::log_investigation::compile_query("request", LogQueryMode::Regex)
            .expect("compile query");
    tab.viewer.structured_view = false;
    app.workbench_mut()
        .open_tab(WorkbenchTabState::PodLogs(tab));

    assert!(apply_action(AppAction::SaveLogPreset, &mut app));
    assert!(
        app.status_message()
            .is_some_and(|msg| msg.starts_with("Saved log preset: "))
    );
    let saved = &app
        .preferences
        .as_ref()
        .expect("preferences")
        .log_presets
        .pod_logs;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].query, "request");

    let Some(active_tab) = app.workbench_mut().active_tab_mut() else {
        panic!("expected active tab");
    };
    let WorkbenchTabState::PodLogs(tab) = &mut active_tab.state else {
        panic!("expected pod logs tab");
    };
    tab.viewer.search_query.clear();
    tab.viewer.search_input.clear();
    tab.viewer.search_mode = LogQueryMode::Substring;
    tab.viewer.compiled_search = None;
    tab.viewer.structured_view = true;

    assert!(apply_action(AppAction::ApplyNextLogPreset, &mut app));
    assert!(
        app.status_message()
            .is_some_and(|msg| msg.starts_with("Applied pod log preset: "))
    );

    let WorkbenchTabState::PodLogs(tab) = &app.workbench().active_tab().unwrap().state else {
        panic!("expected pod logs tab");
    };
    assert_eq!(tab.viewer.search_query, "request");
    assert_eq!(tab.viewer.search_mode, LogQueryMode::Regex);
    assert!(!tab.viewer.structured_view);
}

#[test]
fn save_and_cycle_workload_log_presets_preserve_filters() {
    use crate::events::input::apply_action;
    use crate::log_investigation::LogQueryMode;

    let mut app = AppState::default();
    let mut tab =
        WorkloadLogsTabState::new(ResourceRef::Deployment("api".into(), "default".into()), 7);
    tab.available_pods = vec!["api-0".into(), "api-1".into()];
    tab.available_containers = vec!["main".into(), "sidecar".into()];
    tab.text_filter = "warn".into();
    tab.text_filter_mode = LogQueryMode::Regex;
    tab.compiled_text_filter = crate::log_investigation::compile_query("warn", LogQueryMode::Regex)
        .expect("compile filter");
    tab.structured_view = false;
    tab.pod_filter = Some("api-1".into());
    tab.container_filter = Some("main".into());
    app.workbench_mut()
        .open_tab(WorkbenchTabState::WorkloadLogs(tab));

    assert!(apply_action(AppAction::SaveLogPreset, &mut app));
    assert!(
        app.status_message()
            .is_some_and(|msg| msg.starts_with("Saved log preset: "))
    );
    let saved = &app
        .preferences
        .as_ref()
        .expect("preferences")
        .log_presets
        .workload_logs;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].pod_filter.as_deref(), Some("api-1"));

    let Some(active_tab) = app.workbench_mut().active_tab_mut() else {
        panic!("expected active tab");
    };
    let WorkbenchTabState::WorkloadLogs(tab) = &mut active_tab.state else {
        panic!("expected workload logs tab");
    };
    tab.text_filter.clear();
    tab.text_filter_mode = LogQueryMode::Substring;
    tab.compiled_text_filter = None;
    tab.structured_view = true;
    tab.pod_filter = None;
    tab.container_filter = None;

    assert!(apply_action(AppAction::ApplyPreviousLogPreset, &mut app));
    assert!(
        app.status_message()
            .is_some_and(|msg| msg.starts_with("Applied workload log preset: "))
    );

    let WorkbenchTabState::WorkloadLogs(tab) = &app.workbench().active_tab().unwrap().state else {
        panic!("expected workload logs tab");
    };
    assert_eq!(tab.text_filter, "warn");
    assert_eq!(tab.text_filter_mode, LogQueryMode::Regex);
    assert_eq!(tab.pod_filter.as_deref(), Some("api-1"));
    assert_eq!(tab.container_filter.as_deref(), Some("main"));
    assert!(!tab.structured_view);
}

#[test]
fn pod_log_preset_cycle_wraps_to_last_saved_entry() {
    use crate::events::input::apply_action;
    use crate::log_investigation::{LogQueryMode, PodLogPreset};
    use crate::preferences::{LogPresetPreferences, UserPreferences};

    let mut app = AppState::default();
    app.preferences = Some(UserPreferences {
        views: HashMap::new(),
        log_presets: LogPresetPreferences {
            pod_logs: vec![
                PodLogPreset {
                    name: "errors".into(),
                    query: "error".into(),
                    mode: LogQueryMode::Substring,
                    time_window: crate::log_investigation::LogTimeWindow::All,
                    structured_view: true,
                },
                PodLogPreset {
                    name: "requests".into(),
                    query: "req=".into(),
                    mode: LogQueryMode::Regex,
                    time_window: crate::log_investigation::LogTimeWindow::Last1Hour,
                    structured_view: false,
                },
            ],
            workload_logs: Vec::new(),
        },
        workspaces: Default::default(),
    });
    app.workbench_mut()
        .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(
            ResourceRef::Pod("pod-0".into(), "default".into()),
        )));

    assert!(apply_action(AppAction::ApplyPreviousLogPreset, &mut app));

    let WorkbenchTabState::PodLogs(tab) = &app.workbench().active_tab().unwrap().state else {
        panic!("expected pod logs tab");
    };
    assert_eq!(tab.viewer.search_query, "req=");
    assert_eq!(tab.viewer.search_mode, LogQueryMode::Regex);
    assert_eq!(
        tab.viewer.time_window,
        crate::log_investigation::LogTimeWindow::Last1Hour
    );
    assert!(!tab.viewer.structured_view);
}

#[test]
fn sidebar_icons_do_not_use_replacement_glyphs() {
    assert!(!NavGroup::Config.icon().contains('\u{fffd}'));
    assert!(!NavGroup::Config.sidebar_text(false).contains('\u{fffd}'));
    assert!(!AppView::Endpoints.icon().contains('\u{fffd}'));
    assert!(!AppView::Endpoints.sidebar_text().contains('\u{fffd}'));
}

#[test]
fn recent_resource_jumps_dedupe_and_move_to_front() {
    let mut app = AppState::default();
    let pod = ResourceRef::Pod("api-0".into(), "prod".into());
    let deployment = ResourceRef::Deployment("api".into(), "prod".into());

    app.record_recent_resource_jump(pod.clone());
    app.record_recent_resource_jump(deployment.clone());
    app.record_recent_resource_jump(pod.clone());

    let targets: Vec<_> = app
        .recent_jumps()
        .iter()
        .map(|entry| &entry.target)
        .collect();
    assert_eq!(
        targets,
        vec![
            &RecentJumpTarget::Resource(pod),
            &RecentJumpTarget::Resource(deployment)
        ]
    );
}

#[test]
fn recent_jumps_stay_bounded() {
    let mut app = AppState::default();
    for idx in 0..(MAX_RECENT_JUMPS + 5) {
        app.record_recent_view_jump(if idx % 2 == 0 {
            AppView::Pods
        } else {
            AppView::Deployments
        });
        app.record_recent_resource_jump(ResourceRef::Pod(format!("api-{idx}"), "prod".into()));
    }

    assert!(app.recent_jumps().len() <= MAX_RECENT_JUMPS);
    assert_eq!(
        app.recent_jumps().front().map(|entry| &entry.target),
        Some(&RecentJumpTarget::Resource(ResourceRef::Pod(
            format!("api-{}", MAX_RECENT_JUMPS + 4),
            "prod".into()
        )))
    );
}

#[test]
fn navigate_to_view_records_recent_view_jump_for_current_scope() {
    let mut app = AppState::default();
    app.current_context_name = Some("prod".into());
    app.current_namespace = "payments".into();

    app.navigate_to_view(AppView::Pods);

    let recent = app.recent_jumps().front().expect("recent jump");
    assert_eq!(recent.target, RecentJumpTarget::View(AppView::Pods));
    assert_eq!(
        recent.scope,
        ActivityScope {
            context: Some("prod".into()),
            namespace: "payments".into(),
        }
    );
}

#[test]
fn visible_action_history_entries_filter_to_active_scope() {
    let mut app = AppState::default();
    app.current_context_name = Some("prod".into());
    app.current_namespace = "payments".into();
    app.record_action_pending(
        ActionKind::Restart,
        AppView::Pods,
        Some(ResourceRef::Pod("api-0".into(), "payments".into())),
        "Pod api-0",
        "Restart requested",
    );

    app.current_context_name = Some("staging".into());
    app.current_namespace = "default".into();
    app.record_action_pending(
        ActionKind::Restart,
        AppView::Pods,
        Some(ResourceRef::Pod("web-0".into(), "default".into())),
        "Pod web-0",
        "Restart requested",
    );

    let entries = app.visible_action_history_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].resource_label, "Pod web-0");
}

#[test]
fn selected_action_history_target_ignores_stale_scope_rows() {
    let mut app = AppState::default();
    app.current_context_name = Some("prod".into());
    app.current_namespace = "payments".into();
    app.record_action_pending(
        ActionKind::Restart,
        AppView::Pods,
        Some(ResourceRef::Pod("api-0".into(), "payments".into())),
        "Pod api-0",
        "Restart requested",
    );

    app.current_context_name = Some("staging".into());
    app.current_namespace = "default".into();
    app.open_action_history_tab(true);

    assert!(app.selected_action_history_target().is_none());
}

#[test]
fn visible_action_history_entries_hide_completed_rows_from_old_scope() {
    let mut app = AppState::default();
    app.current_context_name = Some("prod".into());
    app.current_namespace = "payments".into();
    let entry_id = app.record_action_pending(
        ActionKind::Delete,
        AppView::Pods,
        Some(ResourceRef::Pod("api-0".into(), "payments".into())),
        "Pod api-0",
        "Delete requested",
    );
    app.complete_action_history(
        entry_id,
        ActionStatus::Succeeded,
        "Deleted Pod api-0",
        false,
    );

    app.current_context_name = Some("staging".into());
    app.current_namespace = "default".into();

    assert!(app.visible_action_history_entries().is_empty());
}

#[test]
fn action_history_selection_clamps_when_scope_changes() {
    let mut app = AppState::default();
    app.current_context_name = Some("prod".into());
    app.current_namespace = "payments".into();
    app.record_action_pending(
        ActionKind::Restart,
        AppView::Pods,
        Some(ResourceRef::Pod("api-0".into(), "payments".into())),
        "Pod api-0",
        "Restart requested",
    );

    app.current_context_name = Some("staging".into());
    app.current_namespace = "default".into();
    app.record_action_pending(
        ActionKind::Restart,
        AppView::Pods,
        Some(ResourceRef::Pod("web-0".into(), "default".into())),
        "Pod web-0",
        "Restart requested",
    );
    app.record_action_pending(
        ActionKind::Restart,
        AppView::Pods,
        Some(ResourceRef::Pod("web-1".into(), "default".into())),
        "Pod web-1",
        "Restart requested",
    );
    app.open_action_history_tab(true);
    if let Some(tab) = app.workbench.active_tab_mut()
        && let WorkbenchTabState::ActionHistory(history_tab) = &mut tab.state
    {
        history_tab.selected = 1;
    }

    app.current_context_name = Some("prod".into());
    app.set_namespace("payments".into());

    let Some(tab) = app.workbench.active_tab() else {
        panic!("action history tab should stay open");
    };
    let WorkbenchTabState::ActionHistory(history_tab) = &tab.state else {
        panic!("active tab should be action history");
    };
    assert_eq!(history_tab.selected, 0);
    assert_eq!(
        app.selected_action_history_target()
            .map(|target| target.resource.clone()),
        Some(ResourceRef::Pod("api-0".into(), "payments".into()))
    );
}

#[test]
fn action_history_selection_preserves_selected_entry_when_newer_scope_row_prepends() {
    let mut app = AppState::default();
    app.current_context_name = Some("prod".into());
    app.current_namespace = "payments".into();
    app.record_action_pending(
        ActionKind::Restart,
        AppView::Pods,
        Some(ResourceRef::Pod("api-0".into(), "payments".into())),
        "Pod api-0",
        "Restart requested",
    );
    app.record_action_pending(
        ActionKind::Restart,
        AppView::Pods,
        Some(ResourceRef::Pod("api-1".into(), "payments".into())),
        "Pod api-1",
        "Restart requested",
    );
    app.open_action_history_tab(true);
    let visible_ids = app
        .visible_action_history_entries()
        .into_iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    if let Some(tab) = app.workbench.active_tab_mut()
        && let WorkbenchTabState::ActionHistory(history_tab) = &mut tab.state
    {
        history_tab.select_bottom(&visible_ids);
    }

    app.record_action_pending(
        ActionKind::Restart,
        AppView::Pods,
        Some(ResourceRef::Pod("api-2".into(), "payments".into())),
        "Pod api-2",
        "Restart requested",
    );

    assert_eq!(
        app.selected_action_history_target()
            .map(|target| target.resource.clone()),
        Some(ResourceRef::Pod("api-0".into(), "payments".into()))
    );
}

#[test]
fn reopening_connectivity_tab_preserves_selected_target_identity() {
    let mut app = AppState::default();
    let source = ResourceRef::Pod("source".into(), "default".into());
    app.open_connectivity_tab(
        source.clone(),
        vec![
            crate::workbench::ConnectivityTargetOption {
                resource: ResourceRef::Pod("api-0".into(), "default".into()),
                display: "api-0".into(),
                status: "ready".into(),
                pod_ip: Some("10.0.0.2".into()),
            },
            crate::workbench::ConnectivityTargetOption {
                resource: ResourceRef::Pod("api-1".into(), "default".into()),
                display: "api-1".into(),
                status: "ready".into(),
                pod_ip: Some("10.0.0.3".into()),
            },
        ],
    );

    if let Some(tab) = app.workbench.active_tab_mut()
        && let WorkbenchTabState::Connectivity(connectivity_tab) = &mut tab.state
    {
        connectivity_tab.select_bottom_target();
    }

    app.open_connectivity_tab(
        source,
        vec![
            crate::workbench::ConnectivityTargetOption {
                resource: ResourceRef::Pod("api-00".into(), "default".into()),
                display: "api-00".into(),
                status: "ready".into(),
                pod_ip: Some("10.0.0.1".into()),
            },
            crate::workbench::ConnectivityTargetOption {
                resource: ResourceRef::Pod("api-0".into(), "default".into()),
                display: "api-0".into(),
                status: "ready".into(),
                pod_ip: Some("10.0.0.2".into()),
            },
            crate::workbench::ConnectivityTargetOption {
                resource: ResourceRef::Pod("api-1".into(), "default".into()),
                display: "api-1".into(),
                status: "ready".into(),
                pod_ip: Some("10.0.0.3".into()),
            },
        ],
    );

    let Some(tab) = app.workbench.active_tab() else {
        panic!("connectivity tab should stay open");
    };
    let WorkbenchTabState::Connectivity(connectivity_tab) = &tab.state else {
        panic!("active tab should be connectivity");
    };
    assert_eq!(
        connectivity_tab
            .selected_target_option()
            .map(|target| target.resource.clone()),
        Some(ResourceRef::Pod("api-1".into(), "default".into()))
    );
}
