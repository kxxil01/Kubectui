use super::*;
use crate::cronjob::CronJobHistoryEntry;
use crate::k8s::dtos::PodInfo;
use crate::k8s::rollout::{RolloutInspection, RolloutRevisionInfo, RolloutWorkloadKind};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Verifies full forward tab cycle across all views and wraps to Dashboard.
#[test]
fn tab_cycles_all_views_forward() {
    let mut app = AppState::default();
    let expected = [
        // Overview
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
        app.handle_key_event(KeyEvent::from(KeyCode::Char(']'))),
        AppAction::WorkbenchNextTab
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('['))),
        AppAction::WorkbenchPreviousTab
    );
    assert_eq!(
        app.handle_key_event(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)),
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
fn ai_workbench_tab_supports_scrolling_shortcuts() {
    use crate::workbench::{AiAnalysisTabState, WorkbenchTabState};

    let mut app = AppState::default();
    let mut tab = AiAnalysisTabState::new(
        9,
        "Ask AI",
        ResourceRef::Pod("api-0".into(), "default".into()),
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
    assert!(loaded.workbench.open);
    assert_eq!(loaded.workbench.height, 15);

    let _ = std::fs::remove_file(path);
}

/// Verifies quit requires confirmation: first q sets confirm_quit, second q quits.
#[test]
fn quit_action_sets_should_quit() {
    let mut app = AppState::default();

    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('q')));
    assert_eq!(action, AppAction::None);
    assert!(app.confirm_quit);
    assert!(!app.should_quit());

    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('q')));
    assert_eq!(action, AppAction::Quit);
    assert!(app.should_quit());
}

/// Verifies any other key cancels the quit confirmation.
#[test]
fn quit_confirm_cancelled_by_other_key() {
    let mut app = AppState::default();
    app.handle_key_event(KeyEvent::from(KeyCode::Char('q')));
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

/// Verifies rapid tab switching remains stable.
#[test]
fn rapid_tab_switching_is_stable() {
    let mut app = AppState::default();

    for _ in 0..(AppView::tabs().len() * 3) {
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
    }

    assert_eq!(app.view(), AppView::Dashboard);
}

/// Verifies search input ignores Ctrl-modified characters except supported shortcuts.
#[test]
fn search_input_ignores_ctrl_characters() {
    let mut app = AppState::default();
    app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));

    app.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));

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
}

#[test]
fn ctrl_y_returns_copy_resource_name() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
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
    assert_eq!(prefs.workspaces.banks.len(), 1);
    assert_eq!(prefs.workspaces.banks[0].name, "ops services");
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
    // All groups collapsed except Overview (default view's group).
    assert!(!loaded.collapsed_groups.contains(&NavGroup::Overview));
    assert!(loaded.collapsed_groups.contains(&NavGroup::Workloads));
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
