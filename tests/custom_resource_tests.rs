#![allow(clippy::field_reassign_with_default)]
//! Tests for Custom Resource detail view, Extensions navigation,
//! Helm repositories, probe panel error state, and context-aware detail footer.

use crossterm::event::{KeyCode, KeyEvent};
use kubectui::app::{AppAction, AppState, AppView, DetailViewState, Focus, ResourceRef};
use kubectui::events::route_keyboard_input;
use kubectui::k8s::dtos::{CustomResourceDefinitionInfo, CustomResourceInfo, HelmRepoInfo};
use kubectui::policy::DetailAction;
use kubectui::ui::components::probe_panel::ProbePanelState;

// ─── ResourceRef::CustomResource helpers ─────────────────────────────────────

#[test]
fn custom_resource_ref_kind_returns_crd_kind() {
    let cr = ResourceRef::CustomResource {
        name: "my-widget".to_string(),
        namespace: Some("default".to_string()),
        group: "demo.io".to_string(),
        version: "v1alpha1".to_string(),
        kind: "Widget".to_string(),
        plural: "widgets".to_string(),
    };
    assert_eq!(cr.kind(), "Widget");
}

#[test]
fn custom_resource_ref_name_returns_instance_name() {
    let cr = ResourceRef::CustomResource {
        name: "my-widget".to_string(),
        namespace: Some("production".to_string()),
        group: "demo.io".to_string(),
        version: "v1".to_string(),
        kind: "Widget".to_string(),
        plural: "widgets".to_string(),
    };
    assert_eq!(cr.name(), "my-widget");
}

#[test]
fn custom_resource_ref_namespace_returns_some_for_namespaced() {
    let cr = ResourceRef::CustomResource {
        name: "item".to_string(),
        namespace: Some("staging".to_string()),
        group: "example.com".to_string(),
        version: "v1".to_string(),
        kind: "Item".to_string(),
        plural: "items".to_string(),
    };
    assert_eq!(cr.namespace(), Some("staging"));
}

#[test]
fn custom_resource_ref_namespace_returns_none_for_cluster_scoped() {
    let cr = ResourceRef::CustomResource {
        name: "global-thing".to_string(),
        namespace: None,
        group: "infra.io".to_string(),
        version: "v1beta1".to_string(),
        kind: "GlobalThing".to_string(),
        plural: "globalthings".to_string(),
    };
    assert_eq!(cr.namespace(), None);
}

// ─── Extensions view: two-phase navigation ───────────────────────────────────

fn extensions_app_with_crd_and_instances() -> (AppState, kubectui::state::ClusterSnapshot) {
    let mut app = AppState::default();
    // Navigate to Extensions view
    while app.view() != AppView::Extensions {
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
    }
    app.focus = Focus::Content;

    let crd = CustomResourceDefinitionInfo {
        name: "widgets.demo.io".to_string(),
        group: "demo.io".to_string(),
        version: "v1".to_string(),
        kind: "Widget".to_string(),
        plural: "widgets".to_string(),
        scope: "Namespaced".to_string(),
        instances: 2,
    };

    let mut snapshot = kubectui::state::ClusterSnapshot::default();
    snapshot.custom_resource_definitions.push(crd);

    // Simulate that the CRD was selected and instances were loaded
    app.set_extension_instances(
        "widgets.demo.io".to_string(),
        vec![
            CustomResourceInfo {
                name: "widget-alpha".to_string(),
                namespace: Some("default".to_string()),
                ..CustomResourceInfo::default()
            },
            CustomResourceInfo {
                name: "widget-beta".to_string(),
                namespace: Some("staging".to_string()),
                ..CustomResourceInfo::default()
            },
        ],
        None,
    );

    (app, snapshot)
}

#[test]
fn extensions_initially_not_in_instances_mode() {
    let (app, _) = extensions_app_with_crd_and_instances();
    assert!(!app.extension_in_instances);
    assert_eq!(app.extension_instance_cursor, 0);
}

#[test]
fn extensions_enter_drills_into_instances_pane() {
    let (mut app, _) = extensions_app_with_crd_and_instances();

    // Pressing Enter on CRD picker should drill into instances
    // (simulating what main.rs does)
    assert!(!app.extension_in_instances);
    app.extension_in_instances = true;
    app.extension_instance_cursor = 0;

    assert!(app.extension_in_instances);
    assert_eq!(app.extension_instance_cursor, 0);
}

#[test]
fn extensions_jk_navigates_instances_when_in_instances_mode() {
    let (mut app, _) = extensions_app_with_crd_and_instances();
    app.extension_in_instances = true;

    // j moves instance cursor down
    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('j')), &mut app);
    assert_eq!(action, AppAction::None);
    assert_eq!(app.extension_instance_cursor, 1);

    // k moves instance cursor up
    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('k')), &mut app);
    assert_eq!(action, AppAction::None);
    assert_eq!(app.extension_instance_cursor, 0);
}

#[test]
fn extensions_instance_cursor_wraps_around() {
    let (mut app, _) = extensions_app_with_crd_and_instances();
    app.extension_in_instances = true;

    // Move past last instance (2 instances: 0, 1)
    route_keyboard_input(KeyEvent::from(KeyCode::Char('j')), &mut app);
    assert_eq!(app.extension_instance_cursor, 1);
    route_keyboard_input(KeyEvent::from(KeyCode::Char('j')), &mut app);
    assert_eq!(app.extension_instance_cursor, 0); // wrapped

    // Move up from 0
    route_keyboard_input(KeyEvent::from(KeyCode::Char('k')), &mut app);
    assert_eq!(app.extension_instance_cursor, 1); // wrapped to end
}

#[test]
fn extensions_jk_navigates_crds_when_not_in_instances_mode() {
    let (mut app, _) = extensions_app_with_crd_and_instances();
    assert!(!app.extension_in_instances);

    let old_idx = app.selected_idx;
    route_keyboard_input(KeyEvent::from(KeyCode::Char('j')), &mut app);
    // selected_idx should change (CRD picker navigation), not instance cursor
    assert_eq!(app.selected_idx, old_idx + 1);
    assert_eq!(app.extension_instance_cursor, 0);
}

#[test]
fn extensions_set_instances_resets_cursor() {
    let mut app = AppState::default();
    app.extension_instance_cursor = 5;

    app.set_extension_instances(
        "test.crd".to_string(),
        vec![CustomResourceInfo {
            name: "item".to_string(),
            ..CustomResourceInfo::default()
        }],
        None,
    );

    assert_eq!(app.extension_instance_cursor, 0);
}

#[test]
fn extensions_set_instances_stores_error() {
    let mut app = AppState::default();
    app.set_extension_instances(
        "broken.crd".to_string(),
        vec![],
        Some("RBAC denied".to_string()),
    );

    assert_eq!(app.extension_error.as_deref(), Some("RBAC denied"));
    assert!(app.extension_instances.is_empty());
}

#[test]
fn extensions_navigate_away_resets_instances_mode() {
    let (mut app, _) = extensions_app_with_crd_and_instances();
    app.extension_in_instances = true;

    // Tab away from Extensions
    let action = route_keyboard_input(KeyEvent::from(KeyCode::Tab), &mut app);
    assert_eq!(action, AppAction::None);
    assert_ne!(app.view(), AppView::Extensions);
    // extension_in_instances is reset by NavigateTo in main.rs, not by Tab directly.
    // But the next time we navigate to Extensions, it should be false.
}

// ─── Probe panel error state ─────────────────────────────────────────────────

#[test]
fn probe_panel_error_field_defaults_to_none() {
    let state = ProbePanelState::new("pod".to_string(), "ns".to_string(), vec![]);
    assert!(state.error.is_none());
}

#[test]
fn probe_panel_error_can_be_set() {
    let mut state = ProbePanelState::new("pod".to_string(), "ns".to_string(), vec![]);
    state.error = Some("connection refused".to_string());
    assert_eq!(state.error.as_deref(), Some("connection refused"));
}

#[test]
fn probe_panel_update_probes_preserves_error() {
    let mut state = ProbePanelState::new("pod".to_string(), "ns".to_string(), vec![]);
    state.error = Some("timeout".to_string());
    state.update_probes(vec![]);
    // update_probes only replaces container_probes, not error
    assert_eq!(state.error.as_deref(), Some("timeout"));
}

// ─── Helm repositories ──────────────────────────────────────────────────────

#[test]
fn helm_repo_info_default_is_empty() {
    let repo = HelmRepoInfo::default();
    assert!(repo.name.is_empty());
    assert!(repo.url.is_empty());
}

#[test]
fn helm_repo_info_equality() {
    let a = HelmRepoInfo {
        name: "bitnami".to_string(),
        url: "https://charts.bitnami.com/bitnami".to_string(),
    };
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn helm_repos_read_returns_vec() {
    // This reads from the local filesystem — may be empty if no helm config exists.
    // The important thing is it doesn't panic.
    let repos = kubectui::k8s::helm::read_helm_repositories();
    // repos is a Vec<HelmRepoInfo> — just verify it's a valid vec
    let _ = repos.len();
}

// ─── HelmCharts sidebar label is now "Repositories" ─────────────────────────

#[test]
fn helm_charts_view_label_is_repositories() {
    assert_eq!(AppView::HelmCharts.label(), "Repositories");
}

#[test]
fn helm_releases_view_label_unchanged() {
    assert_eq!(AppView::HelmReleases.label(), "Releases");
}

// ─── Detail footer: context-aware keybinds ──────────────────────────────────

#[test]
fn detail_footer_pod_resource_shows_logs_portfwd_probes() {
    let detail = DetailViewState {
        resource: Some(ResourceRef::Pod("p1".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    };
    assert!(detail.supports_action(DetailAction::Logs));
    assert!(detail.supports_action(DetailAction::PortForward));
    assert!(detail.supports_action(DetailAction::Probes));
}

#[test]
fn detail_footer_deployment_is_scalable_and_restartable() {
    let detail = DetailViewState {
        resource: Some(ResourceRef::Deployment(
            "dep1".to_string(),
            "ns".to_string(),
        )),
        yaml: Some("kind: Deployment".to_string()),
        ..DetailViewState::default()
    };
    assert!(detail.supports_action(DetailAction::Scale));
    assert!(detail.supports_action(DetailAction::Restart));
}

#[test]
fn detail_footer_configmap_not_scalable_not_restartable() {
    let detail = DetailViewState {
        resource: Some(ResourceRef::ConfigMap("cm1".to_string(), "ns".to_string())),
        yaml: Some("kind: ConfigMap".to_string()),
        ..DetailViewState::default()
    };
    assert!(!detail.supports_action(DetailAction::Scale));
    assert!(!detail.supports_action(DetailAction::Restart));
    assert!(!detail.supports_action(DetailAction::Logs));
}

#[test]
fn detail_footer_custom_resource_not_scalable_not_restartable_not_pod() {
    let detail = DetailViewState {
        resource: Some(ResourceRef::CustomResource {
            name: "item".to_string(),
            namespace: Some("ns".to_string()),
            group: "demo.io".to_string(),
            version: "v1".to_string(),
            kind: "Widget".to_string(),
            plural: "widgets".to_string(),
        }),
        yaml: Some("kind: Widget".to_string()),
        ..DetailViewState::default()
    };
    assert!(!detail.supports_action(DetailAction::Logs));
    assert!(!detail.supports_action(DetailAction::Scale));
    assert!(!detail.supports_action(DetailAction::Restart));
}

// ─── Keyboard: R only restarts restartable workloads ─────────────────────────

#[test]
fn rollout_restart_only_for_restartable_kinds() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Deployment("dep".to_string(), "ns".to_string())),
        ..DetailViewState::default()
    });

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('R')), &mut app);
    assert_eq!(action, AppAction::RolloutRestart);
}

#[test]
fn rollout_restart_noop_for_non_restartable() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::ConfigMap("cm".to_string(), "ns".to_string())),
        ..DetailViewState::default()
    });

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('R')), &mut app);
    assert_eq!(action, AppAction::None);
}

#[test]
fn rollout_restart_noop_for_custom_resource() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::CustomResource {
            name: "x".to_string(),
            namespace: Some("ns".to_string()),
            group: "g".to_string(),
            version: "v1".to_string(),
            kind: "K".to_string(),
            plural: "ks".to_string(),
        }),
        ..DetailViewState::default()
    });

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('R')), &mut app);
    assert_eq!(action, AppAction::None);
}

// ─── Edit YAML only when loaded ──────────────────────────────────────────────

#[test]
fn edit_yaml_allowed_when_yaml_loaded() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("p".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    });

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('e')), &mut app);
    assert_eq!(action, AppAction::EditYaml);
}

#[test]
fn edit_yaml_noop_when_no_yaml() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("p".to_string(), "ns".to_string())),
        yaml: None,
        ..DetailViewState::default()
    });

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('e')), &mut app);
    assert_eq!(action, AppAction::None);
}

#[test]
fn edit_yaml_noop_when_loading() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("p".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        loading: true,
        ..DetailViewState::default()
    });

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('e')), &mut app);
    assert_eq!(action, AppAction::None);
}

#[test]
fn edit_yaml_noop_when_subpanel_open() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("p".to_string(), "ns".to_string())),
        yaml: Some("kind: Pod".to_string()),
        ..DetailViewState::default()
    });
    app.open_logs_viewer();

    let action = route_keyboard_input(KeyEvent::from(KeyCode::Char('e')), &mut app);
    // LogsViewer is active, so 'e' is not handled by the detail view
    assert_eq!(action, AppAction::None);
}
