use crossterm::event::{KeyCode, KeyEvent};
use jiff::ToSpan;

use super::flux_reconcile::{
    flux_observed_state, flux_reconcile_progress_observed, process_flux_reconcile_verifications,
};
use super::{
    ExtensionFetchResult, MAX_RECENT_EVENTS_CACHE_ITEMS, PendingFluxReconcileVerification,
    apply_extension_fetch_result, detail_debug_launch_owned, detail_node_debug_launch_owned,
    fail_context_switch, map_palette_detail_action, mutation_refresh_options,
    normalize_recent_events, palette_action_requires_loaded_detail, parse_editor_command,
    prepare_bookmark_target, prepare_resource_target, preserve_detail_selection_identity,
    queued_refresh_requires_two_phase, refresh_options_for_view, refresh_palette_resources,
    selected_extension_crd, selected_flux_reconcile_resource, selected_resource,
    should_request_periodic_redraw, ui_staleness_visible, workbench_follow_streams_to_stop,
};
use kubectui::ui::components::command_palette::PaletteEntry;
use kubectui::{
    action_history::ActionKind,
    app::{AppAction, AppState, AppView, DetailViewState, ResourceRef, SidebarItem},
    bookmarks::{BookmarkEntry, resource_exists},
    cronjob::CronJobHistoryEntry,
    extensions::AiWorkflowKind,
    k8s::dtos::{
        ConfigMapInfo, CustomResourceDefinitionInfo, CustomResourceInfo, FluxResourceInfo,
        K8sEventInfo, NodeInfo, PodInfo, VulnerabilityReportInfo, VulnerabilitySummaryCounts,
    },
    policy::DetailAction,
    state::{
        ClusterSnapshot, DataPhase, GlobalState, RefreshOptions, RefreshScope,
        watch::{WatchPayload, WatchUpdate, WatchedResource},
    },
    time::{AppTimestamp, now},
    ui::components::{
        debug_container_dialog::DebugContainerDialogState, node_debug_dialog::NodeDebugDialogState,
    },
    workbench::{PodLogsTabState, RolloutTabState, WorkbenchTabState},
};
use std::time::{Duration, Instant};

#[test]
fn truncate_ai_block_respects_character_limit() {
    let truncated = super::truncate_ai_block("abcdef", 1);
    assert_eq!(truncated, "a…");

    let multibyte = super::truncate_ai_block("éclair", 2);
    assert_eq!(multibyte, "éc…");
}

#[test]
fn sanitize_ai_annotation_redacts_sensitive_keys() {
    assert_eq!(
        super::sanitize_ai_annotation("authorization", "Bearer top-secret"),
        "[redacted]"
    );
    assert_eq!(
        super::sanitize_ai_annotation("team", "payments"),
        "payments"
    );
}

#[test]
fn sanitize_ai_yaml_excerpt_redacts_secret_values() {
    let excerpt = super::sanitize_ai_yaml_excerpt(
        &ResourceRef::Deployment("api".to_string(), "prod".to_string()),
        r#"
apiVersion: v1
kind: Pod
metadata:
  name: api
spec:
  token: super-secret
  containers:
    - name: api
      env:
        - name: API_TOKEN
          value: literal-secret
  stringData:
    password: hidden
"#,
    )
    .expect("sanitized excerpt");

    assert!(excerpt.contains("token: <redacted>"));
    assert!(excerpt.contains("stringData: <redacted>"));
    assert!(!excerpt.contains("super-secret"));
    assert!(!excerpt.contains("literal-secret"));
}

#[test]
fn sanitize_ai_yaml_excerpt_omits_secret_manifests() {
    let excerpt = super::sanitize_ai_yaml_excerpt(
        &ResourceRef::Secret("app".to_string(), "prod".to_string()),
        "kind: Secret\ndata:\n  token: aGVsbG8=\n",
    )
    .expect("redacted secret excerpt");

    assert!(excerpt.contains("Secret manifests are not sent to AI"));
}

#[test]
fn rollout_ai_context_uses_rollout_summary_when_available() {
    let resource = ResourceRef::Deployment("api".to_string(), "prod".to_string());
    let mut app = AppState::default();
    let mut rollout = RolloutTabState::new(resource.clone());
    rollout.summary_lines = vec!["Progressing: 1 unavailable replica remains".to_string()];
    rollout.conditions = vec![kubectui::k8s::rollout::RolloutConditionInfo {
        type_: "Progressing".to_string(),
        status: "True".to_string(),
        reason: Some("ReplicaSetUpdated".to_string()),
        message: Some("updating".to_string()),
    }];
    app.workbench.open_tab(WorkbenchTabState::Rollout(rollout));

    let context = super::build_ai_analysis_context(
        &app,
        &ClusterSnapshot::default(),
        &resource,
        AiWorkflowKind::RolloutRisk,
    );

    assert_eq!(context.workflow_title.as_deref(), Some("Rollout Context"));
    assert!(
        context
            .workflow_lines
            .iter()
            .any(|line| line.contains("1 unavailable replica"))
    );
    assert!(
        context
            .workflow_lines
            .iter()
            .any(|line| line.contains("Progressing=True"))
    );
}

#[test]
fn triage_ai_context_includes_vulnerability_findings() {
    let resource = ResourceRef::Deployment("api".to_string(), "prod".to_string());
    let context = super::build_ai_analysis_context(
        &AppState::default(),
        &ClusterSnapshot {
            snapshot_version: 9_001,
            vulnerability_reports: vec![VulnerabilityReportInfo {
                resource_kind: "Deployment".to_string(),
                resource_name: "api".to_string(),
                resource_namespace: "prod".to_string(),
                namespace: "prod".to_string(),
                fixable_count: 2,
                counts: VulnerabilitySummaryCounts {
                    critical: 1,
                    high: 2,
                    medium: 0,
                    low: 0,
                    unknown: 0,
                },
                ..VulnerabilityReportInfo::default()
            }],
            ..ClusterSnapshot::default()
        },
        &resource,
        AiWorkflowKind::TriageFindings,
    );

    assert_eq!(context.workflow_title.as_deref(), Some("Triage Context"));
    assert!(
        context
            .workflow_lines
            .iter()
            .any(|line| line.contains("Vulnerabilities [Deployment]: 3 total, 2 fixable")),
        "{:?}",
        context.workflow_lines
    );
}

#[test]
fn selected_flux_reconcile_resource_rejects_suspended_flux_objects() {
    let mut app = AppState::default();
    app.view = AppView::FluxCDKustomizations;

    let mut snapshot = ClusterSnapshot::default();
    snapshot.flux_resources.push(FluxResourceInfo {
        name: "apps".to_string(),
        namespace: Some("flux-system".to_string()),
        group: "kustomize.toolkit.fluxcd.io".to_string(),
        version: "v1".to_string(),
        kind: "Kustomization".to_string(),
        plural: "kustomizations".to_string(),
        suspended: true,
        ..FluxResourceInfo::default()
    });

    let err = selected_flux_reconcile_resource(&app, &snapshot).expect_err("must reject");
    assert_eq!(
        err,
        "Flux reconcile is unavailable because Kustomization 'apps' is suspended."
    );
}

#[test]
fn selected_flux_reconcile_resource_uses_detail_resource_when_present() {
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::CustomResource {
            name: "backend".to_string(),
            namespace: Some("flux-system".to_string()),
            group: "helm.toolkit.fluxcd.io".to_string(),
            version: "v2".to_string(),
            kind: "HelmRelease".to_string(),
            plural: "helmreleases".to_string(),
        }),
        ..DetailViewState::default()
    });

    let resource = selected_flux_reconcile_resource(&app, &ClusterSnapshot::default())
        .expect("detail flux resource is selected");
    assert_eq!(resource.kind(), "HelmRelease");
    assert_eq!(resource.name(), "backend");
    assert_eq!(resource.namespace(), Some("flux-system"));
}

#[test]
fn watch_update_needs_flux_refresh_for_flux_change_payload() {
    let update = WatchUpdate {
        resource: WatchedResource::Flux,
        context_generation: 1,
        data: WatchPayload::FluxChanged,
    };
    assert!(super::watch_update_needs_flux_refresh(&update));
}

#[test]
fn watch_update_needs_flux_refresh_ignores_non_flux_payloads() {
    let update = WatchUpdate {
        resource: WatchedResource::Pods,
        context_generation: 1,
        data: WatchPayload::Pods(vec![]),
    };
    assert!(!super::watch_update_needs_flux_refresh(&update));
}

#[test]
fn should_refresh_from_flux_watch_for_flux_views() {
    assert!(super::should_refresh_from_flux_watch(
        AppView::FluxCDAll,
        &[]
    ));
}

#[test]
fn should_refresh_from_flux_watch_for_issues_views() {
    assert!(super::should_refresh_from_flux_watch(AppView::Issues, &[]));
    assert!(super::should_refresh_from_flux_watch(
        AppView::HealthReport,
        &[]
    ));
}

#[test]
fn should_refresh_from_flux_watch_when_reconcile_verification_pending() {
    let pending = vec![PendingFluxReconcileVerification {
        action_history_id: 1,
        resource: ResourceRef::CustomResource {
            name: "apps".to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
        },
        resource_label: "Kustomization/apps".to_string(),
        baseline: None,
        deadline: Instant::now() + Duration::from_secs(30),
    }];

    assert!(super::should_refresh_from_flux_watch(
        AppView::Pods,
        &pending
    ));
}

#[test]
fn should_refresh_from_flux_watch_ignores_non_flux_views_without_pending_verification() {
    assert!(!super::should_refresh_from_flux_watch(AppView::Pods, &[]));
}

#[test]
fn should_mark_snapshot_dirty_after_watch_when_non_flux_update() {
    assert!(super::should_mark_snapshot_dirty_after_watch(false, false));
    assert!(super::should_mark_snapshot_dirty_after_watch(false, true));
}

#[test]
fn should_mark_snapshot_dirty_after_watch_when_flux_refresh_requested() {
    assert!(super::should_mark_snapshot_dirty_after_watch(true, true));
}

#[test]
fn should_mark_snapshot_dirty_after_watch_skips_flux_no_refresh_case() {
    assert!(!super::should_mark_snapshot_dirty_after_watch(true, false));
}

#[test]
fn periodic_redraw_triggers_on_minute_bucket_change() {
    let app = AppState::default();
    let snapshot = ClusterSnapshot::default();
    let mut last_age_bucket = 10;
    let mut last_staleness_second = 600;

    assert!(should_request_periodic_redraw(
        &app,
        &snapshot,
        660,
        &mut last_age_bucket,
        &mut last_staleness_second,
    ));
    assert_eq!(last_age_bucket, 11);
}

#[test]
fn staleness_visible_only_without_other_status_surfaces() {
    let now_unix = 1_000;
    let stale_snapshot = ClusterSnapshot {
        last_updated: Some(AppTimestamp::from_second(now_unix - 46).expect("valid timestamp")),
        ..ClusterSnapshot::default()
    };
    let mut app = AppState::default();
    assert!(ui_staleness_visible(&app, &stale_snapshot, now_unix));

    app.set_status("working".to_string());
    assert!(!ui_staleness_visible(&app, &stale_snapshot, now_unix));

    app.clear_status();
    app.push_toast("toast".to_string(), false);
    assert!(!ui_staleness_visible(&app, &stale_snapshot, now_unix));
}

#[test]
fn closing_active_logs_tab_collects_follow_stream_to_stop() {
    let mut app = AppState::default();
    app.open_pod_logs_tab(ResourceRef::Pod("pod-0".to_string(), "ns".to_string()));
    if let Some(tab) = app.workbench_mut().active_tab_mut()
        && let WorkbenchTabState::PodLogs(PodLogsTabState { viewer, .. }) = &mut tab.state
    {
        viewer.pod_name = "pod-0".to_string();
        viewer.pod_namespace = "ns".to_string();
        viewer.container_name = "main".to_string();
        viewer.follow_mode = true;
    }

    let streams = workbench_follow_streams_to_stop(&app, AppAction::WorkbenchCloseActiveTab);
    assert_eq!(
        streams,
        vec![("pod-0".to_string(), "ns".to_string(), "main".to_string())]
    );
}

fn cronjob_history_entry(job_name: &str, namespace: &str) -> CronJobHistoryEntry {
    CronJobHistoryEntry {
        job_name: job_name.to_string(),
        namespace: namespace.to_string(),
        status: "Complete".to_string(),
        completions: "1/1".to_string(),
        duration: Some("10s".to_string()),
        pod_count: 1,
        live_pod_count: 0,
        completion_pct: Some(100),
        active_pods: 0,
        failed_pods: 0,
        age: None,
        created_at: None,
        logs_authorized: Some(true),
    }
}

#[test]
fn preserve_detail_selection_identity_keeps_selected_cronjob_job() {
    let current = DetailViewState {
        resource: Some(ResourceRef::CronJob("batch".to_string(), "ops".to_string())),
        cronjob_history: vec![
            cronjob_history_entry("batch-100", "ops"),
            cronjob_history_entry("batch-101", "ops"),
        ],
        cronjob_history_selected: 1,
        ..DetailViewState::default()
    };
    let mut next = DetailViewState {
        resource: Some(ResourceRef::CronJob("batch".to_string(), "ops".to_string())),
        cronjob_history: vec![
            cronjob_history_entry("batch-101", "ops"),
            cronjob_history_entry("batch-102", "ops"),
        ],
        cronjob_history_selected: 1,
        ..DetailViewState::default()
    };

    preserve_detail_selection_identity(Some(&current), &mut next);

    assert_eq!(next.cronjob_history_selected, 0);
    assert_eq!(next.cronjob_history[0].job_name, "batch-101");
}

#[test]
fn detail_debug_launch_owned_requires_matching_resource_and_action_id() {
    let mut app = AppState::default();
    let resource = ResourceRef::Pod("api-0".to_string(), "default".to_string());
    let mut dialog = DebugContainerDialogState::new("api-0", "default");
    dialog.begin_launch(42);
    app.detail_view = Some(DetailViewState {
        resource: Some(resource.clone()),
        debug_dialog: Some(dialog),
        ..DetailViewState::default()
    });

    assert!(detail_debug_launch_owned(&app, &resource, 42));
    assert!(!detail_debug_launch_owned(&app, &resource, 77));
    assert!(!detail_debug_launch_owned(
        &app,
        &ResourceRef::Pod("other".to_string(), "default".to_string()),
        42
    ));
}

#[test]
fn detail_node_debug_launch_owned_requires_matching_resource_and_action_id() {
    let mut app = AppState::default();
    let resource = ResourceRef::Node("node-a".to_string());
    let mut dialog =
        NodeDebugDialogState::new("node-a", "default".to_string(), vec!["default".to_string()]);
    dialog.begin_launch(77);
    app.detail_view = Some(DetailViewState {
        resource: Some(resource.clone()),
        node_debug_dialog: Some(dialog),
        ..DetailViewState::default()
    });

    assert!(detail_node_debug_launch_owned(&app, &resource, 77));
    assert!(!detail_node_debug_launch_owned(&app, &resource, 42));
    assert!(!detail_node_debug_launch_owned(
        &app,
        &ResourceRef::Node("node-b".to_string()),
        77
    ));
}

#[test]
fn closing_non_logs_workbench_does_not_collect_streams() {
    let app = AppState::default();
    let streams = workbench_follow_streams_to_stop(&app, AppAction::WorkbenchCloseActiveTab);
    assert!(streams.is_empty());
}

#[test]
fn failed_context_switch_clears_pending_workspace_restore() {
    let mut app = AppState::default();
    app.pending_workspace_restore = Some(kubectui::workspaces::WorkspaceSnapshot {
        context: Some("prod".into()),
        namespace: "payments".into(),
        view: AppView::Pods,
        search_query: Some("checkout".into()),
        collapsed_groups: Vec::new(),
        workbench_open: false,
        workbench_height: kubectui::workbench::DEFAULT_WORKBENCH_HEIGHT,
        workbench_maximized: false,
        action_history_tab: false,
    });
    let mut global_state = GlobalState::default();
    let mut snapshot_dirty = false;
    let mut needs_redraw = false;
    let mut pending_runbook_restore = Some(kubectui::workbench::RunbookTabState::new(
        kubectui::runbooks::LoadedRunbook {
            id: "pod_failure".into(),
            title: "Pod Failure Triage".into(),
            description: None,
            aliases: vec!["incident".into()],
            resource_kinds: vec!["Pod".into()],
            shortcut: None,
            steps: Vec::new(),
        },
        Some(ResourceRef::Pod("api".into(), "prod".into())),
    ));

    fail_context_switch(
        &mut app,
        &mut global_state,
        "context failed".into(),
        &mut pending_runbook_restore,
        &mut snapshot_dirty,
        &mut needs_redraw,
    );

    assert!(app.pending_workspace_restore.is_none());
    assert!(pending_runbook_restore.is_none());
    assert_eq!(global_state.snapshot().phase, DataPhase::Error);
    assert!(snapshot_dirty);
    assert!(needs_redraw);
    assert_eq!(app.error_message(), Some("context failed"));
}

#[test]
fn parse_editor_command_supports_args_and_quotes() {
    let args =
        parse_editor_command("code -w --reuse-window \"My Editor\"").expect("quoted editor args");
    assert_eq!(
        args,
        vec![
            "code".to_string(),
            "-w".to_string(),
            "--reuse-window".to_string(),
            "My Editor".to_string()
        ]
    );
}

#[test]
fn parse_editor_command_rejects_unmatched_quotes() {
    let err = parse_editor_command("code \"unterminated").expect_err("must reject");
    assert!(err.to_string().contains("unmatched quote"));
}

#[test]
fn selected_resource_clamps_to_last_visible_filtered_row() {
    let mut app = AppState::default();
    app.view = AppView::Nodes;
    app.focus = kubectui::app::Focus::Content;
    app.selected_idx = 8;
    app.search_query = "worker".to_string();

    let mut snapshot = ClusterSnapshot::default();
    snapshot.nodes = vec![
        NodeInfo {
            name: "control-plane".to_string(),
            created_at: Some(now()),
            ..NodeInfo::default()
        },
        NodeInfo {
            name: "worker-a".to_string(),
            created_at: Some(now()),
            ..NodeInfo::default()
        },
    ];

    let selected = selected_resource(&app, &snapshot).expect("selected resource");
    assert_eq!(selected, ResourceRef::Node("worker-a".to_string()));
}

#[test]
fn selected_resource_uses_bookmark_view_selection() {
    let mut app = AppState::default();
    app.view = AppView::Bookmarks;
    app.current_context_name = Some("prod".to_string());
    app.cluster_preferences = Some(std::collections::HashMap::from([(
        "prod".to_string(),
        kubectui::preferences::ClusterPreferences {
            views: std::collections::HashMap::new(),
            bookmarks: vec![BookmarkEntry {
                resource: ResourceRef::Secret("app-secret".to_string(), "default".to_string()),
                bookmarked_at_unix: 1,
            }],
        },
    )]));

    let selected = selected_resource(&app, &ClusterSnapshot::default()).expect("bookmark");
    assert_eq!(
        selected,
        ResourceRef::Secret("app-secret".to_string(), "default".to_string())
    );
}

#[test]
fn prepare_bookmark_target_navigates_to_resource_view() {
    let mut app = AppState::default();
    app.view = AppView::Bookmarks;
    app.focus = kubectui::app::Focus::Content;
    app.current_context_name = Some("prod".to_string());
    app.cluster_preferences = Some(std::collections::HashMap::from([(
        "prod".to_string(),
        kubectui::preferences::ClusterPreferences {
            views: std::collections::HashMap::new(),
            bookmarks: vec![BookmarkEntry {
                resource: ResourceRef::Secret("app-secret".to_string(), "default".to_string()),
                bookmarked_at_unix: 1,
            }],
        },
    )]));
    app.search_query = "app".to_string();

    let mut snapshot = ClusterSnapshot::default();
    snapshot.secrets.push(kubectui::k8s::dtos::SecretInfo {
        name: "app-secret".to_string(),
        namespace: "default".to_string(),
        ..Default::default()
    });

    let target = prepare_bookmark_target(&mut app, &snapshot).expect("bookmark target");
    assert_eq!(target.kind(), "Secret");
    assert_eq!(app.view, AppView::Secrets);
    assert_eq!(app.selected_idx, 0);
    assert!(app.search_query.is_empty());
}

#[test]
fn prepare_resource_target_syncs_sidebar_to_target_view() {
    let mut app = AppState::default();
    app.view = AppView::Governance;
    app.focus = kubectui::app::Focus::Sidebar;
    app.collapsed_groups = kubectui::app::sidebar::all_groups().collect();
    app.sidebar_cursor = 0;
    app.search_query = "stale".to_string();

    let resource = ResourceRef::Pod("api-0".to_string(), "prod".to_string());
    let mut snapshot = ClusterSnapshot::default();
    snapshot.pods.push(PodInfo {
        name: "api-0".to_string(),
        namespace: "prod".to_string(),
        ..PodInfo::default()
    });

    prepare_resource_target(&mut app, &snapshot, &resource).expect("resource target");

    assert_eq!(app.view, AppView::Pods);
    assert_eq!(app.focus, kubectui::app::Focus::Content);
    assert!(app.search_query.is_empty());
    assert!(
        !app.collapsed_groups
            .contains(&kubectui::app::NavGroup::Workloads)
    );
    let rows = kubectui::app::sidebar_rows(&app.collapsed_groups);
    assert_eq!(
        rows.get(app.sidebar_cursor),
        Some(&SidebarItem::View(AppView::Pods))
    );
}

#[test]
fn prepare_resource_target_routes_flux_resources_to_specific_view() {
    let mut app = AppState::default();
    let resource = ResourceRef::CustomResource {
        name: "apps".to_string(),
        namespace: Some("flux-system".to_string()),
        group: "kustomize.toolkit.fluxcd.io".to_string(),
        version: "v1".to_string(),
        kind: "Kustomization".to_string(),
        plural: "kustomizations".to_string(),
    };
    let mut snapshot = ClusterSnapshot::default();
    snapshot.flux_resources.push(FluxResourceInfo {
        name: "apps".to_string(),
        namespace: Some("flux-system".to_string()),
        group: "kustomize.toolkit.fluxcd.io".to_string(),
        version: "v1".to_string(),
        kind: "Kustomization".to_string(),
        plural: "kustomizations".to_string(),
        ..FluxResourceInfo::default()
    });

    prepare_resource_target(&mut app, &snapshot, &resource).expect("flux resource target");

    assert_eq!(app.view, AppView::FluxCDKustomizations);
}

#[test]
fn prepare_resource_target_routes_generic_custom_resources_to_extensions() {
    let mut app = AppState::default();
    let resource = ResourceRef::CustomResource {
        name: "redis".to_string(),
        namespace: Some("prod".to_string()),
        group: "demo.io".to_string(),
        version: "v1".to_string(),
        kind: "Widget".to_string(),
        plural: "widgets".to_string(),
    };
    let mut snapshot = ClusterSnapshot::default();
    snapshot
        .custom_resource_definitions
        .push(CustomResourceDefinitionInfo {
            name: "widgets.demo.io".to_string(),
            group: "demo.io".to_string(),
            version: "v1".to_string(),
            kind: "Widget".to_string(),
            plural: "widgets".to_string(),
            scope: "Namespaced".to_string(),
            instances: 1,
        });

    prepare_resource_target(&mut app, &snapshot, &resource).expect("extension resource target");

    assert_eq!(app.view, AppView::Extensions);
    assert_eq!(app.selected_idx, 0);
    assert_eq!(
        app.extension_selected_crd.as_deref(),
        Some("widgets.demo.io")
    );
    assert!(!app.extension_in_instances);
}

#[test]
fn prepare_resource_target_selects_loaded_extension_instance_when_available() {
    let mut app = AppState::default();
    app.extension_selected_crd = Some("widgets.demo.io".to_string());
    app.extension_instances = vec![
        CustomResourceInfo {
            name: "api".to_string(),
            namespace: Some("prod".to_string()),
            ..CustomResourceInfo::default()
        },
        CustomResourceInfo {
            name: "redis".to_string(),
            namespace: Some("prod".to_string()),
            ..CustomResourceInfo::default()
        },
    ];
    let resource = ResourceRef::CustomResource {
        name: "redis".to_string(),
        namespace: Some("prod".to_string()),
        group: "demo.io".to_string(),
        version: "v1".to_string(),
        kind: "Widget".to_string(),
        plural: "widgets".to_string(),
    };
    let mut snapshot = ClusterSnapshot::default();
    snapshot
        .custom_resource_definitions
        .push(CustomResourceDefinitionInfo {
            name: "widgets.demo.io".to_string(),
            group: "demo.io".to_string(),
            version: "v1".to_string(),
            kind: "Widget".to_string(),
            plural: "widgets".to_string(),
            scope: "Namespaced".to_string(),
            instances: 2,
        });

    prepare_resource_target(&mut app, &snapshot, &resource).expect("extension resource target");

    assert_eq!(app.view, AppView::Extensions);
    assert!(app.extension_in_instances);
    assert_eq!(app.extension_instance_cursor, 1);
}

#[test]
fn prepare_resource_target_resets_extension_instances_mode_when_switching_crd() {
    let mut app = AppState::default();
    app.extension_selected_crd = Some("gadgets.demo.io".to_string());
    app.extension_instances = vec![CustomResourceInfo {
        name: "legacy".to_string(),
        namespace: Some("prod".to_string()),
        ..CustomResourceInfo::default()
    }];
    app.extension_in_instances = true;
    app.extension_instance_cursor = 0;

    let resource = ResourceRef::CustomResource {
        name: "redis".to_string(),
        namespace: Some("prod".to_string()),
        group: "demo.io".to_string(),
        version: "v1".to_string(),
        kind: "Widget".to_string(),
        plural: "widgets".to_string(),
    };
    let mut snapshot = ClusterSnapshot::default();
    snapshot
        .custom_resource_definitions
        .push(CustomResourceDefinitionInfo {
            name: "gadgets.demo.io".to_string(),
            group: "demo.io".to_string(),
            version: "v1".to_string(),
            kind: "Gadget".to_string(),
            plural: "gadgets".to_string(),
            scope: "Namespaced".to_string(),
            instances: 1,
        });
    snapshot
        .custom_resource_definitions
        .push(CustomResourceDefinitionInfo {
            name: "widgets.demo.io".to_string(),
            group: "demo.io".to_string(),
            version: "v1".to_string(),
            kind: "Widget".to_string(),
            plural: "widgets".to_string(),
            scope: "Namespaced".to_string(),
            instances: 1,
        });

    prepare_resource_target(&mut app, &snapshot, &resource).expect("extension resource target");

    assert_eq!(app.view, AppView::Extensions);
    assert_eq!(app.selected_idx, 1);
    assert_eq!(
        app.extension_selected_crd.as_deref(),
        Some("widgets.demo.io")
    );
    assert!(app.extension_instances.is_empty());
    assert!(!app.extension_in_instances);
    assert_eq!(app.extension_instance_cursor, 0);
}

#[test]
fn selected_extension_crd_uses_filtered_query_selection() {
    let mut app = AppState::default();
    app.view = AppView::Extensions;
    app.search_query = "gadget".to_string();

    let mut snapshot = ClusterSnapshot::default();
    snapshot.custom_resource_definitions = vec![
        CustomResourceDefinitionInfo {
            name: "widgets.demo.io".to_string(),
            group: "demo.io".to_string(),
            version: "v1".to_string(),
            kind: "Widget".to_string(),
            plural: "widgets".to_string(),
            scope: "Namespaced".to_string(),
            instances: 1,
        },
        CustomResourceDefinitionInfo {
            name: "gadgets.demo.io".to_string(),
            group: "demo.io".to_string(),
            version: "v1".to_string(),
            kind: "Gadget".to_string(),
            plural: "gadgets".to_string(),
            scope: "Namespaced".to_string(),
            instances: 2,
        },
    ];

    let selected = selected_extension_crd(&app, &snapshot).expect("selected CRD");
    assert_eq!(selected.name, "gadgets.demo.io");
}

#[test]
fn stale_extension_fetch_results_are_ignored() {
    let mut app = AppState::default();
    app.begin_extension_instances_load("widgets.demo.io".to_string());

    apply_extension_fetch_result(
        &mut app,
        ExtensionFetchResult {
            crd_name: "gadgets.demo.io".to_string(),
            result: Ok(Vec::new()),
        },
    );

    assert_eq!(
        app.extension_selected_crd.as_deref(),
        Some("widgets.demo.io")
    );
    assert!(app.extension_instances.is_empty());
    assert!(app.extension_error.is_none());
}

#[test]
fn refresh_palette_resources_merges_extension_instances_without_duplicates() {
    let mut app = AppState::default();
    app.extension_selected_crd = Some("helmreleases.helm.toolkit.fluxcd.io".to_string());
    app.extension_instances = vec![CustomResourceInfo {
        name: "redis".to_string(),
        namespace: Some("prod".to_string()),
        ..CustomResourceInfo::default()
    }];

    let mut snapshot = ClusterSnapshot::default();
    snapshot.snapshot_version = 1;
    snapshot
        .custom_resource_definitions
        .push(CustomResourceDefinitionInfo {
            name: "helmreleases.helm.toolkit.fluxcd.io".to_string(),
            group: "helm.toolkit.fluxcd.io".to_string(),
            version: "v2".to_string(),
            kind: "HelmRelease".to_string(),
            plural: "helmreleases".to_string(),
            scope: "Namespaced".to_string(),
            instances: 1,
        });
    snapshot.flux_resources.push(FluxResourceInfo {
        name: "redis".to_string(),
        namespace: Some("prod".to_string()),
        group: "helm.toolkit.fluxcd.io".to_string(),
        version: "v2".to_string(),
        kind: "HelmRelease".to_string(),
        plural: "helmreleases".to_string(),
        ..FluxResourceInfo::default()
    });

    refresh_palette_resources(&mut app, &snapshot);
    app.command_palette.open();
    for c in "redis".chars() {
        let _ = app
            .command_palette
            .handle_key(KeyEvent::from(KeyCode::Char(c)));
    }

    let redis_entries = app
        .command_palette
        .filtered()
        .into_iter()
        .filter_map(|entry| match entry {
            PaletteEntry::Resource(resource) => Some(resource),
            _ => None,
        })
        .filter(|entry| {
            entry.resource
                == ResourceRef::CustomResource {
                    name: "redis".to_string(),
                    namespace: Some("prod".to_string()),
                    group: "helm.toolkit.fluxcd.io".to_string(),
                    version: "v2".to_string(),
                    kind: "HelmRelease".to_string(),
                    plural: "helmreleases".to_string(),
                }
        })
        .count();

    assert_eq!(redis_entries, 1);
}

#[test]
fn palette_redis_resource_results_are_routable_and_existing() {
    let mut app = AppState::default();
    app.extension_selected_crd = Some("widgets.demo.io".to_string());
    app.extension_instances = vec![CustomResourceInfo {
        name: "redis".to_string(),
        namespace: Some("prod".to_string()),
        ..CustomResourceInfo::default()
    }];

    let mut snapshot = ClusterSnapshot::default();
    snapshot.snapshot_version = 2;
    snapshot.pods.push(PodInfo {
        name: "redis-1".to_string(),
        namespace: "prod".to_string(),
        ..PodInfo::default()
    });
    snapshot.services.push(kubectui::k8s::dtos::ServiceInfo {
        name: "redis".to_string(),
        namespace: "prod".to_string(),
        ..kubectui::k8s::dtos::ServiceInfo::default()
    });
    snapshot
        .deployments
        .push(kubectui::k8s::dtos::DeploymentInfo {
            name: "redis".to_string(),
            namespace: "prod".to_string(),
            ..kubectui::k8s::dtos::DeploymentInfo::default()
        });
    snapshot
        .pod_disruption_budgets
        .push(kubectui::k8s::dtos::PodDisruptionBudgetInfo {
            name: "redis".to_string(),
            namespace: "prod".to_string(),
            ..kubectui::k8s::dtos::PodDisruptionBudgetInfo::default()
        });
    snapshot.gateways.push(kubectui::k8s::dtos::GatewayInfo {
        name: "redis".to_string(),
        namespace: "prod".to_string(),
        version: "v1".to_string(),
        ..kubectui::k8s::dtos::GatewayInfo::default()
    });
    snapshot.flux_resources.push(FluxResourceInfo {
        name: "redis".to_string(),
        namespace: Some("prod".to_string()),
        group: "helm.toolkit.fluxcd.io".to_string(),
        version: "v2".to_string(),
        kind: "HelmRelease".to_string(),
        plural: "helmreleases".to_string(),
        ..FluxResourceInfo::default()
    });
    snapshot
        .custom_resource_definitions
        .push(CustomResourceDefinitionInfo {
            name: "widgets.demo.io".to_string(),
            group: "demo.io".to_string(),
            version: "v1".to_string(),
            kind: "Widget".to_string(),
            plural: "widgets".to_string(),
            scope: "Namespaced".to_string(),
            instances: 1,
        });

    refresh_palette_resources(&mut app, &snapshot);
    app.command_palette.open();
    for c in "redis".chars() {
        let _ = app
            .command_palette
            .handle_key(KeyEvent::from(KeyCode::Char(c)));
    }

    let resources = app
        .command_palette
        .filtered()
        .into_iter()
        .filter_map(|entry| match entry {
            PaletteEntry::Resource(resource) => Some(resource.resource),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(
        resources.len() >= 6,
        "expected mixed redis matches across built-in, gateway, flux, and extension resources"
    );

    for resource in resources {
        assert!(
            resource_exists(&snapshot, &resource),
            "palette produced stale resource: {resource:?}"
        );
        let mut routed_app = app.clone();
        assert!(
            prepare_resource_target(&mut routed_app, &snapshot, &resource).is_ok(),
            "palette resource was not routable: {resource:?}"
        );
    }
}

#[test]
fn normalize_recent_events_sorts_and_truncates() {
    let now = now();
    let events = (0..300)
        .map(|idx| K8sEventInfo {
            name: format!("event-{idx}"),
            last_seen: Some(
                now.checked_sub(i64::from(idx).seconds())
                    .expect("timestamp in range"),
            ),
            ..K8sEventInfo::default()
        })
        .collect::<Vec<_>>();

    let normalized = normalize_recent_events(events);
    assert_eq!(normalized.len(), MAX_RECENT_EVENTS_CACHE_ITEMS);
    assert_eq!(normalized[0].name, "event-0");
    assert_eq!(
        normalized.last().map(|event| event.name.as_str()),
        Some("event-249")
    );
}

#[test]
fn events_view_uses_fast_refresh_profile() {
    let options = refresh_options_for_view(AppView::Events, false, false);
    assert_eq!(options.primary_scope, RefreshScope::NONE);
    assert_eq!(options.options.scope, RefreshScope::NONE);
}

#[test]
fn dashboard_refresh_profile_runs_metrics_in_background() {
    let dispatch = refresh_options_for_view(AppView::Dashboard, false, false);
    assert_eq!(dispatch.primary_scope, RefreshScope::CORE_OVERVIEW);
    assert!(dispatch.options.scope.contains(RefreshScope::METRICS));
    assert!(dispatch.options.include_cluster_info);
}

#[test]
fn pods_and_nodes_refresh_profiles_backfill_metrics() {
    let pods = refresh_options_for_view(AppView::Pods, false, false);
    let nodes = refresh_options_for_view(AppView::Nodes, false, false);

    assert_eq!(pods.primary_scope, RefreshScope::PODS);
    assert!(pods.options.scope.contains(RefreshScope::METRICS));
    assert!(!pods.options.include_cluster_info);
    assert_eq!(nodes.primary_scope, RefreshScope::NODES);
    assert!(nodes.options.scope.contains(RefreshScope::METRICS));
    assert!(!nodes.options.include_cluster_info);
}

#[test]
fn services_and_issues_refresh_profiles_keep_services_scope_lightweight() {
    let services = refresh_options_for_view(AppView::Services, false, false);
    let issues = refresh_options_for_view(AppView::Issues, false, false);
    let health_report = refresh_options_for_view(AppView::HealthReport, false, false);

    assert_eq!(services.primary_scope, RefreshScope::SERVICES);
    assert_eq!(services.options.scope, RefreshScope::SERVICES);
    assert_eq!(issues.primary_scope, RefreshScope::CORE_OVERVIEW);
    assert!(
        issues
            .options
            .scope
            .contains(RefreshScope::LEGACY_SECONDARY)
    );
    assert!(issues.options.scope.contains(RefreshScope::FLUX));
    assert_eq!(health_report.primary_scope, issues.primary_scope);
    assert_eq!(health_report.options.scope, issues.options.scope);
}

#[test]
fn health_report_selected_resource_uses_sanitizer_only_rows() {
    let mut app = AppState::default();
    app.view = AppView::HealthReport;

    let mut snapshot = ClusterSnapshot::default();
    snapshot.snapshot_version = 41;
    snapshot.nodes.push(NodeInfo {
        name: "node-a".to_string(),
        ready: false,
        ..Default::default()
    });
    snapshot.config_maps.push(ConfigMapInfo {
        name: "unused-config".to_string(),
        namespace: "default".to_string(),
        ..Default::default()
    });

    let selected = selected_resource(&app, &snapshot).expect("selected resource");
    assert_eq!(
        selected,
        ResourceRef::ConfigMap("unused-config".to_string(), "default".to_string())
    );
}

#[test]
fn bucket_views_refresh_only_their_own_scope() {
    let network = refresh_options_for_view(AppView::Endpoints, false, false);
    let config = refresh_options_for_view(AppView::ConfigMaps, false, false);
    let storage = refresh_options_for_view(AppView::PersistentVolumes, false, false);
    let helm_charts = refresh_options_for_view(AppView::HelmCharts, false, false);
    let helm_releases = refresh_options_for_view(AppView::HelmReleases, false, false);
    let security = refresh_options_for_view(AppView::ServiceAccounts, false, false);
    let flux = refresh_options_for_view(AppView::FluxCDAll, false, false);
    let events = refresh_options_for_view(AppView::Events, false, false);
    let extensions = refresh_options_for_view(AppView::Extensions, false, false);

    assert_eq!(network.primary_scope, RefreshScope::NETWORK);
    assert_eq!(network.options.scope, RefreshScope::NETWORK);
    assert_eq!(config.primary_scope, RefreshScope::CONFIG);
    assert_eq!(config.options.scope, RefreshScope::CONFIG);
    assert_eq!(storage.primary_scope, RefreshScope::STORAGE);
    assert_eq!(storage.options.scope, RefreshScope::STORAGE);
    assert_eq!(
        helm_charts.primary_scope,
        RefreshScope::LOCAL_HELM_REPOSITORIES
    );
    assert_eq!(
        helm_charts.options.scope,
        RefreshScope::LOCAL_HELM_REPOSITORIES
    );
    assert_eq!(helm_releases.primary_scope, RefreshScope::HELM);
    assert_eq!(helm_releases.options.scope, RefreshScope::HELM);
    assert_eq!(security.primary_scope, RefreshScope::SECURITY);
    assert_eq!(security.options.scope, RefreshScope::SECURITY);
    assert_eq!(flux.primary_scope, RefreshScope::FLUX);
    assert_eq!(flux.options.scope, RefreshScope::FLUX);
    assert_eq!(events.primary_scope, RefreshScope::NONE);
    assert_eq!(events.options.scope, RefreshScope::NONE);
    assert_eq!(extensions.primary_scope, RefreshScope::EXTENSIONS);
    assert_eq!(extensions.options.scope, RefreshScope::EXTENSIONS);
}

#[test]
fn workload_views_refresh_only_their_specific_bucket() {
    let deployments = refresh_options_for_view(AppView::Deployments, false, false);
    let statefulsets = refresh_options_for_view(AppView::StatefulSets, false, false);
    let daemonsets = refresh_options_for_view(AppView::DaemonSets, false, false);
    let replicasets = refresh_options_for_view(AppView::ReplicaSets, false, false);
    let controllers = refresh_options_for_view(AppView::ReplicationControllers, false, false);
    let jobs = refresh_options_for_view(AppView::Jobs, false, false);
    let cronjobs = refresh_options_for_view(AppView::CronJobs, false, false);
    let namespaces = refresh_options_for_view(AppView::Namespaces, false, false);

    assert_eq!(deployments.primary_scope, RefreshScope::DEPLOYMENTS);
    assert_eq!(deployments.options.scope, RefreshScope::DEPLOYMENTS);
    assert_eq!(statefulsets.primary_scope, RefreshScope::STATEFULSETS);
    assert_eq!(statefulsets.options.scope, RefreshScope::STATEFULSETS);
    assert_eq!(daemonsets.primary_scope, RefreshScope::DAEMONSETS);
    assert_eq!(daemonsets.options.scope, RefreshScope::DAEMONSETS);
    assert_eq!(replicasets.primary_scope, RefreshScope::REPLICASETS);
    assert_eq!(replicasets.options.scope, RefreshScope::REPLICASETS);
    assert_eq!(
        controllers.primary_scope,
        RefreshScope::REPLICATION_CONTROLLERS
    );
    assert_eq!(
        controllers.options.scope,
        RefreshScope::REPLICATION_CONTROLLERS
    );
    assert_eq!(jobs.primary_scope, RefreshScope::JOBS);
    assert_eq!(jobs.options.scope, RefreshScope::JOBS);
    assert_eq!(cronjobs.primary_scope, RefreshScope::CRONJOBS);
    assert_eq!(cronjobs.options.scope, RefreshScope::CRONJOBS);
    assert_eq!(namespaces.primary_scope, RefreshScope::NAMESPACES);
    assert_eq!(namespaces.options.scope, RefreshScope::NAMESPACES);
}

#[test]
fn mutation_refresh_profiles_prioritize_active_scope() {
    let deployments = mutation_refresh_options(AppView::Deployments, false);
    let cronjobs = mutation_refresh_options(AppView::CronJobs, false);
    let config = mutation_refresh_options(AppView::ConfigMaps, false);
    let network = mutation_refresh_options(AppView::Endpoints, false);
    let helm = mutation_refresh_options(AppView::HelmReleases, false);

    assert_eq!(deployments.primary_scope, RefreshScope::CORE_OVERVIEW);
    assert_eq!(deployments.options.scope, RefreshScope::CORE_OVERVIEW);
    assert_eq!(cronjobs.primary_scope, RefreshScope::CORE_OVERVIEW);
    assert_eq!(cronjobs.options.scope, RefreshScope::CORE_OVERVIEW);
    assert_eq!(config.primary_scope, RefreshScope::CONFIG);
    assert_eq!(config.options.scope, RefreshScope::CONFIG);
    assert_eq!(network.primary_scope, RefreshScope::NETWORK);
    assert_eq!(network.options.scope, RefreshScope::NETWORK);
    assert_eq!(helm.primary_scope, RefreshScope::HELM);
    assert_eq!(helm.options.scope, RefreshScope::HELM);
}

#[test]
fn queued_refresh_only_reruns_two_phase_for_full_refreshes() {
    assert!(queued_refresh_requires_two_phase(
        RefreshScope::CORE_OVERVIEW,
        RefreshOptions {
            scope: RefreshScope::CORE_OVERVIEW.union(RefreshScope::LEGACY_SECONDARY),
            include_cluster_info: false,
            skip_core: false,
        },
    ));
    assert!(!queued_refresh_requires_two_phase(
        RefreshScope::NONE,
        RefreshOptions {
            scope: RefreshScope::LEGACY_SECONDARY,
            include_cluster_info: false,
            skip_core: true,
        },
    ));
    assert!(!queued_refresh_requires_two_phase(
        RefreshScope::PODS,
        RefreshOptions {
            scope: RefreshScope::PODS,
            include_cluster_info: false,
            skip_core: false,
        },
    ));
}

#[test]
fn palette_node_actions_require_loaded_detail() {
    assert!(palette_action_requires_loaded_detail(
        &AppAction::CordonNode
    ));
    assert!(palette_action_requires_loaded_detail(
        &AppAction::UncordonNode
    ));
    assert!(palette_action_requires_loaded_detail(
        &AppAction::ConfirmDrainNode
    ));
}

#[test]
fn palette_debug_container_requires_loaded_detail() {
    assert!(palette_action_requires_loaded_detail(
        &AppAction::DebugContainerDialogOpen
    ));
}

#[test]
fn palette_helm_history_does_not_require_loaded_detail() {
    assert!(!palette_action_requires_loaded_detail(
        &AppAction::OpenHelmHistory
    ));
}

#[test]
fn palette_rollout_does_not_require_loaded_detail() {
    assert!(!palette_action_requires_loaded_detail(
        &AppAction::OpenRollout
    ));
}

#[test]
fn palette_drain_maps_to_confirmation_action() {
    assert_eq!(
        map_palette_detail_action(DetailAction::Drain),
        AppAction::ConfirmDrainNode
    );
}

#[test]
fn palette_debug_container_maps_to_dialog_open() {
    assert_eq!(
        map_palette_detail_action(DetailAction::DebugContainer),
        AppAction::DebugContainerDialogOpen
    );
}

#[test]
fn palette_helm_history_maps_to_open_history() {
    assert_eq!(
        map_palette_detail_action(DetailAction::ViewHelmHistory),
        AppAction::OpenHelmHistory
    );
}

#[test]
fn palette_rollout_maps_to_open_rollout() {
    assert_eq!(
        map_palette_detail_action(DetailAction::ViewRollout),
        AppAction::OpenRollout
    );
}

#[test]
fn palette_network_policy_maps_to_analysis_open() {
    assert_eq!(
        map_palette_detail_action(DetailAction::ViewNetworkPolicies),
        AppAction::OpenNetworkPolicyView
    );
    assert!(!palette_action_requires_loaded_detail(
        &AppAction::OpenNetworkPolicyView
    ));
}

#[test]
fn palette_access_review_maps_to_open_action() {
    assert_eq!(
        map_palette_detail_action(DetailAction::ViewAccessReview),
        AppAction::OpenAccessReview
    );
    assert!(!palette_action_requires_loaded_detail(
        &AppAction::OpenAccessReview
    ));
}

#[test]
fn palette_connectivity_maps_to_query_open() {
    assert_eq!(
        map_palette_detail_action(DetailAction::CheckNetworkConnectivity),
        AppAction::OpenNetworkConnectivity
    );
    assert!(!palette_action_requires_loaded_detail(
        &AppAction::OpenNetworkConnectivity
    ));
}

#[test]
fn palette_traffic_debug_maps_to_open_action() {
    assert_eq!(
        map_palette_detail_action(DetailAction::ViewTrafficDebug),
        AppAction::OpenTrafficDebug
    );
    assert!(!palette_action_requires_loaded_detail(
        &AppAction::OpenTrafficDebug
    ));
}

#[test]
fn palette_cronjob_suspend_maps_to_confirmation_action() {
    assert_eq!(
        map_palette_detail_action(DetailAction::SuspendCronJob),
        AppAction::ConfirmCronJobSuspend(true)
    );
    assert_eq!(
        map_palette_detail_action(DetailAction::ResumeCronJob),
        AppAction::ConfirmCronJobSuspend(false)
    );
    assert!(palette_action_requires_loaded_detail(
        &AppAction::ConfirmCronJobSuspend(true)
    ));
}

fn test_flux_resource(status: &str, last_reconcile_time: Option<AppTimestamp>) -> FluxResourceInfo {
    FluxResourceInfo {
        name: "apps".to_string(),
        namespace: Some("flux-system".to_string()),
        kind: "Kustomization".to_string(),
        group: "kustomize.toolkit.fluxcd.io".to_string(),
        version: "v1".to_string(),
        plural: "kustomizations".to_string(),
        status: status.to_string(),
        message: Some(String::new()),
        suspended: false,
        last_reconcile_time,
        ..FluxResourceInfo::default()
    }
}

fn test_flux_resource_ref() -> ResourceRef {
    ResourceRef::CustomResource {
        name: "apps".to_string(),
        namespace: Some("flux-system".to_string()),
        group: "kustomize.toolkit.fluxcd.io".to_string(),
        version: "v1".to_string(),
        kind: "Kustomization".to_string(),
        plural: "kustomizations".to_string(),
    }
}

#[test]
fn flux_reconcile_progress_detects_last_reconcile_change() {
    let baseline_time = now();
    let baseline = flux_observed_state(&test_flux_resource("Ready", Some(baseline_time)));
    let current = test_flux_resource(
        "Ready",
        Some(
            baseline_time
                .checked_add(30.seconds())
                .expect("timestamp in range"),
        ),
    );

    assert!(flux_reconcile_progress_observed(Some(&baseline), &current));
}

#[test]
fn process_flux_reconcile_verifications_marks_success_when_status_changes() {
    let mut app = AppState::default();
    let resource = test_flux_resource_ref();
    let entry_id = app.record_action_pending(
        ActionKind::FluxReconcile,
        AppView::FluxCDKustomizations,
        Some(resource.clone()),
        "Kustomization 'apps'".to_string(),
        "Requesting reconcile".to_string(),
    );
    let baseline_time = now();
    let baseline = flux_observed_state(&test_flux_resource("Ready", Some(baseline_time)));
    let snapshot = ClusterSnapshot {
        flux_resources: vec![test_flux_resource(
            "Ready",
            Some(
                baseline_time
                    .checked_add(30.seconds())
                    .expect("timestamp in range"),
            ),
        )],
        ..ClusterSnapshot::default()
    };
    let mut pending = vec![PendingFluxReconcileVerification {
        action_history_id: entry_id,
        resource,
        resource_label: "Kustomization 'apps'".to_string(),
        baseline: Some(baseline),
        deadline: Instant::now() + Duration::from_secs(5),
    }];
    assert!(process_flux_reconcile_verifications(
        &mut app,
        &snapshot,
        &mut pending,
        &mut |a, msg| a.set_status(msg),
    ));
    assert!(pending.is_empty());
    assert!(
        app.action_history()
            .find_by_id(entry_id)
            .expect("history entry")
            .message
            .contains("Flux reconcile observed")
    );
    assert!(
        app.status_message()
            .expect("status message")
            .contains("Flux reconcile observed")
    );
}

#[test]
fn process_flux_reconcile_verifications_reports_waiting_when_deadline_expires() {
    let mut app = AppState::default();
    let resource = test_flux_resource_ref();
    let entry_id = app.record_action_pending(
        ActionKind::FluxReconcile,
        AppView::FluxCDKustomizations,
        Some(resource.clone()),
        "Kustomization 'apps'".to_string(),
        "Requesting reconcile".to_string(),
    );
    let snapshot = ClusterSnapshot {
        flux_resources: vec![test_flux_resource("Ready", None)],
        ..ClusterSnapshot::default()
    };
    let mut pending = vec![PendingFluxReconcileVerification {
        action_history_id: entry_id,
        resource,
        resource_label: "Kustomization 'apps'".to_string(),
        baseline: Some(flux_observed_state(&test_flux_resource("Ready", None))),
        deadline: Instant::now() - Duration::from_secs(1),
    }];
    assert!(process_flux_reconcile_verifications(
        &mut app,
        &snapshot,
        &mut pending,
        &mut |a, msg| a.set_status(msg),
    ));
    assert!(pending.is_empty());
    assert!(
        app.action_history()
            .find_by_id(entry_id)
            .expect("history entry")
            .message
            .contains("Waiting for controller status update")
    );
}
