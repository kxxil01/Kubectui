use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
    preserve_selection_identity_after_snapshot_change, queued_refresh_requires_two_phase,
    refresh_options_for_view, refresh_palette_resources, refresh_scope_pending, request_refresh,
    selected_extension_crd, selected_flux_reconcile_resource, selected_resource,
    should_include_flux_in_auto_refresh, should_preserve_current_flux_after_refresh,
    should_request_navigation_refresh, should_request_periodic_redraw,
    strip_active_watch_scope_from_refresh, ui_staleness_visible, watch_scope_for_view,
    workbench_all_follow_streams_to_stop, workbench_follow_streams_to_stop,
};
use crate::async_types::{QueuedRefresh, RefreshDispatch, RefreshRuntimeState};
use kubectui::ui::components::command_palette::PaletteEntry;
use kubectui::{
    action_history::{ActionKind, ActionStatus},
    ai_actions::AiWorkflowKind,
    app::{
        AppAction, AppState, AppView, ContentPaneFocus, DetailViewState, Focus, ResourceRef,
        SELECTION_SEARCH_FALLBACK_STATUS, SidebarItem, WorkloadSortColumn, WorkloadSortState,
    },
    bookmarks::{BookmarkEntry, resource_exists},
    cronjob::CronJobHistoryEntry,
    k8s::{
        client::FluxWatchTarget,
        dtos::{
            ConfigMapInfo, CustomResourceDefinitionInfo, CustomResourceInfo, DeploymentInfo,
            FluxResourceInfo, JobInfo, K8sEventInfo, NamespaceInfo, NodeInfo, PodInfo, ServiceInfo,
            VulnerabilityReportInfo, VulnerabilitySummaryCounts,
        },
    },
    log_investigation::LogEntry,
    policy::DetailAction,
    state::{
        ClusterSnapshot, DataPhase, FluxResourceTargetKey, FluxTargetFingerprints, GlobalState,
        RefreshOptions, RefreshScope,
        watch::{WatchPayload, WatchUpdate, WatchedResource},
    },
    time::{AppTimestamp, now},
    ui::components::{
        debug_container_dialog::DebugContainerDialogState, node_debug_dialog::NodeDebugDialogState,
    },
    workbench::{
        DecodedSecretTabState, PodLogsTabState, ResourceYamlTabState, RolloutTabState,
        WorkbenchTabState,
    },
};
use std::time::{Duration, Instant};

#[test]
fn root_enter_shortcut_rejects_control_alt_modifiers() {
    let mut app = AppState {
        focus: Focus::Content,
        view: AppView::Pods,
        ..AppState::default()
    };

    assert!(super::should_handle_root_enter(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &app
    ));
    assert!(!super::should_handle_root_enter(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL),
        &app
    ));
    assert!(!super::should_handle_root_enter(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
        &app
    ));

    app.confirm_quit = true;
    assert!(!super::should_handle_root_enter(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &app
    ));
}

#[test]
fn detail_result_gate_includes_decoded_secret_tabs() {
    let resource = ResourceRef::Secret("app-secret".into(), "default".into());
    let mut app = AppState::default();
    let mut tab = DecodedSecretTabState::new(resource.clone());
    tab.loading = true;
    tab.pending_request_id = Some(42);
    app.workbench
        .open_tab(WorkbenchTabState::DecodedSecret(tab));

    assert!(super::workbench_waiting_for_detail_result(
        &app, &resource, 42
    ));
    assert!(!super::workbench_waiting_for_detail_result(
        &app, &resource, 41
    ));
}

#[test]
fn detail_result_gate_still_includes_yaml_and_events_tabs() {
    let resource = ResourceRef::Pod("api".into(), "default".into());
    let mut app = AppState::default();
    app.open_resource_yaml_tab(resource.clone(), None, None, Some(7));
    app.open_resource_events_tab(resource.clone(), Vec::new(), true, None, Some(8));

    assert!(super::workbench_waiting_for_detail_result(
        &app, &resource, 7
    ));
    assert!(super::workbench_waiting_for_detail_result(
        &app, &resource, 8
    ));
}

#[test]
fn prepare_context_switch_ui_resets_secondary_pane_focus_and_scroll() {
    let mut app = AppState::default();
    app.view = AppView::Governance;
    app.focus = Focus::Content;
    app.content_detail_scroll = 13;
    app.content_pane_focus = ContentPaneFocus::Secondary;
    app.search_query = "team-a".into();
    app.search_cursor = 6;
    app.is_search_mode = true;
    app.detail_view = Some(DetailViewState::default());
    app.set_status(SELECTION_SEARCH_FALLBACK_STATUS.to_string());
    app.open_action_history_tab(true);
    app.workbench
        .open_tab(WorkbenchTabState::ResourceYaml(ResourceYamlTabState::new(
            ResourceRef::Pod("api-0".into(), "team-a".into()),
        )));

    super::prepare_context_switch_ui(&mut app);

    assert_eq!(app.selected_idx(), 0);
    assert_eq!(app.content_detail_scroll, 0);
    assert_eq!(app.content_pane_focus(), ContentPaneFocus::List);
    assert_eq!(app.search_query(), "");
    assert_eq!(app.search_cursor(), 0);
    assert!(!app.is_search_mode);
    assert!(app.detail_view.is_none());
    assert_eq!(app.status_message(), None);
    assert_eq!(app.workbench().tabs.len(), 1);
    assert!(matches!(
        &app.workbench().tabs[0].state,
        WorkbenchTabState::ActionHistory(_)
    ));
}

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
fn sanitize_ai_context_lines_redacts_inline_secret_values() {
    let lines = super::cap_ai_lines(
        vec![
            "Authorization: Bearer live-token password=literal-secret".to_string(),
            "dsn=postgres://super:db-pass@db.example:5432/app token: literal-token".to_string(),
        ],
        4,
        1_000,
    );

    let rendered = lines.join("\n");
    assert!(rendered.contains("Authorization: [redacted]"), "{rendered}");
    assert!(rendered.contains("password=<redacted>"), "{rendered}");
    assert!(rendered.contains("[redacted-uri]"), "{rendered}");
    assert!(rendered.contains("token: [redacted]"), "{rendered}");
    assert!(!rendered.contains("live-token"), "{rendered}");
    assert!(!rendered.contains("literal-secret"), "{rendered}");
    assert!(!rendered.contains("db-pass"), "{rendered}");
    assert!(!rendered.contains("literal-token"), "{rendered}");
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
fn ai_context_redacts_sensitive_pod_log_values() {
    let resource = ResourceRef::Pod("api-0".to_string(), "prod".to_string());
    let mut app = AppState::default();
    let mut logs = PodLogsTabState::new(resource.clone());
    logs.viewer.lines = vec![LogEntry::from_raw(
        "INFO Authorization: Bearer live-token password=literal-secret dsn=postgres://super:db-pass@db.example:5432/app",
    )];
    app.workbench.open_tab(WorkbenchTabState::PodLogs(logs));

    let context = super::build_ai_analysis_context(
        &app,
        &ClusterSnapshot::default(),
        &resource,
        AiWorkflowKind::ExplainFailure,
    );
    let rendered = context.log_lines.join("\n");

    assert!(rendered.contains("Authorization: [redacted]"), "{rendered}");
    assert!(rendered.contains("password=<redacted>"), "{rendered}");
    assert!(rendered.contains("[redacted-uri]"), "{rendered}");
    assert!(!rendered.contains("live-token"), "{rendered}");
    assert!(!rendered.contains("literal-secret"), "{rendered}");
    assert!(!rendered.contains("db-pass"), "{rendered}");
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
        data: WatchPayload::Flux {
            target: FluxWatchTarget {
                group: "kustomize.toolkit.fluxcd.io",
                version: "v1",
                kind: "Kustomization",
                plural: "kustomizations",
                namespaced: true,
            },
            items: vec![FluxResourceInfo::default()],
        },
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
    assert!(!super::should_refresh_from_flux_watch(
        AppView::FluxCDAll,
        &[]
    ));
}

#[test]
fn should_refresh_from_flux_watch_for_issues_views() {
    assert!(!super::should_refresh_from_flux_watch(AppView::Issues, &[]));
    assert!(!super::should_refresh_from_flux_watch(
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

    assert!(!super::should_refresh_from_flux_watch(
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
    assert!(super::should_mark_snapshot_dirty_after_watch(true, false));
}

#[test]
fn auto_refresh_includes_flux_only_on_periodic_fallback_ticks() {
    assert!(!should_include_flux_in_auto_refresh(1));
    assert!(!should_include_flux_in_auto_refresh(2));
    assert!(should_include_flux_in_auto_refresh(3));
    assert!(should_include_flux_in_auto_refresh(6));
}

#[test]
fn auto_refresh_strips_only_active_watch_scope() {
    let issues = strip_active_watch_scope_from_refresh(
        refresh_options_for_view(AppView::Issues, false, false),
        RefreshScope::DASHBOARD_WATCHED.union(RefreshScope::NAMESPACES),
    );
    assert!(issues.primary_scope.contains(RefreshScope::JOBS));
    assert!(issues.options.scope.contains(RefreshScope::JOBS));
    assert!(issues.options.scope.contains(RefreshScope::CRONJOBS));
    assert!(issues.options.scope.contains(RefreshScope::REPLICASETS));
    assert!(
        issues
            .options
            .scope
            .contains(RefreshScope::REPLICATION_CONTROLLERS)
    );

    let bookmarks = strip_active_watch_scope_from_refresh(
        refresh_options_for_view(AppView::Bookmarks, false, false),
        RefreshScope::NAMESPACES,
    );
    assert!(bookmarks.primary_scope.contains(RefreshScope::PODS));
    assert!(bookmarks.options.scope.contains(RefreshScope::PODS));
}

#[test]
fn navigation_refresh_includes_empty_local_helm_repositories_view() {
    assert!(should_request_navigation_refresh(AppView::HelmCharts));
    assert!(should_request_navigation_refresh(AppView::HelmReleases));
    assert!(!should_request_navigation_refresh(AppView::PortForwarding));
}

#[test]
fn view_refresh_profiles_have_nonempty_primary_phase() {
    for view in AppView::tabs().iter().copied() {
        let dispatch = refresh_options_for_view(view, false, false);
        if dispatch.options.scope.is_empty() {
            continue;
        }

        assert!(
            dispatch.primary_scope.intersects(dispatch.options.scope),
            "{view:?} primary scope must intersect requested scope"
        );
    }
}

#[test]
fn preserve_current_flux_after_refresh_when_scope_skipped_or_changed_during_flight() {
    let key = FluxResourceTargetKey::new(
        "kustomize.toolkit.fluxcd.io",
        "v1",
        "Kustomization",
        "kustomizations",
    );
    let mut start = FluxTargetFingerprints::new();
    start.insert(key.clone(), 10);
    let unchanged = start.clone();
    let mut changed = FluxTargetFingerprints::new();
    changed.insert(key, 11);
    let non_flux = Some(RefreshOptions {
        scope: RefreshScope::PODS,
        include_cluster_info: false,
        skip_core: false,
    });
    let flux = Some(RefreshOptions {
        scope: RefreshScope::FLUX,
        include_cluster_info: false,
        skip_core: false,
    });

    assert!(should_preserve_current_flux_after_refresh(
        non_flux, &start, &unchanged
    ));
    assert!(should_preserve_current_flux_after_refresh(
        flux, &start, &changed
    ));
    assert!(!should_preserve_current_flux_after_refresh(
        flux, &start, &unchanged
    ));
}

#[test]
fn refresh_scope_pending_detects_in_flight_flux_scope() {
    let mut refresh_state = RefreshRuntimeState {
        in_flight_options: Some(RefreshOptions {
            scope: RefreshScope::FLUX,
            include_cluster_info: false,
            skip_core: false,
        }),
        ..RefreshRuntimeState::default()
    };

    assert!(refresh_scope_pending(&refresh_state, RefreshScope::FLUX));

    refresh_state.in_flight_options = Some(RefreshOptions {
        scope: RefreshScope::METRICS,
        include_cluster_info: false,
        skip_core: false,
    });
    assert!(!refresh_scope_pending(&refresh_state, RefreshScope::FLUX));
}

#[test]
fn refresh_scope_pending_detects_queued_flux_scope() {
    let refresh_state = RefreshRuntimeState {
        queued_refresh: Some(QueuedRefresh {
            request_id: 7,
            namespace: Some("flux-system".to_string()),
            primary_scope: RefreshScope::FLUX,
            options: RefreshOptions {
                scope: RefreshScope::FLUX,
                include_cluster_info: false,
                skip_core: false,
            },
            target_view: Some(AppView::FluxCDKustomizations),
            context_generation: 9,
        }),
        ..RefreshRuntimeState::default()
    };

    assert!(refresh_scope_pending(&refresh_state, RefreshScope::FLUX));
    assert!(!refresh_scope_pending(
        &refresh_state,
        RefreshScope::METRICS
    ));
}

#[tokio::test]
async fn visible_targeted_refresh_preempts_in_flight_secondary_backfill() {
    let (refresh_tx, _refresh_rx) = tokio::sync::mpsc::channel(4);
    let mut global_state = GlobalState::default();
    let client = kubectui::k8s::client::K8sClient::dummy();
    let mut snapshot_dirty = false;
    let mut refresh_state = RefreshRuntimeState {
        request_seq: 5,
        in_flight_id: Some(5),
        in_flight_options: Some(RefreshOptions {
            scope: RefreshScope::CONFIG,
            include_cluster_info: false,
            skip_core: true,
        }),
        in_flight_namespace: Some("default".to_string()),
        in_flight_target_view: None,
        in_flight_task: Some(tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        })),
        ..RefreshRuntimeState::default()
    };

    request_refresh(
        &refresh_tx,
        &mut global_state,
        &client,
        Some("default".to_string()),
        refresh_options_for_view(AppView::Secrets, false, false),
        &mut refresh_state,
        &mut snapshot_dirty,
    );

    assert!(snapshot_dirty);
    assert_eq!(
        global_state.snapshot().view_load_state(AppView::Secrets),
        kubectui::state::ViewLoadState::Loading
    );
    assert_eq!(refresh_state.in_flight_target_view, Some(AppView::Secrets));
    let in_flight = refresh_state
        .in_flight_options
        .expect("visible targeted refresh should be active");
    assert_eq!(in_flight.scope, RefreshScope::CONFIG);
    assert!(!in_flight.include_cluster_info);
    assert!(!in_flight.skip_core);
    assert_eq!(
        refresh_state.in_flight_namespace.as_deref(),
        Some("default")
    );
    assert!(
        refresh_state
            .queued_refresh
            .as_ref()
            .is_some_and(|queued| queued.target_view.is_none() && queued.options.skip_core)
    );
}

#[tokio::test]
async fn aggregate_secondary_backfill_stays_untargeted() {
    let (refresh_tx, _refresh_rx) = tokio::sync::mpsc::channel(4);
    let mut global_state = GlobalState::default();
    let client = kubectui::k8s::client::K8sClient::dummy();
    let mut snapshot_dirty = false;
    let mut refresh_state = RefreshRuntimeState::default();

    request_refresh(
        &refresh_tx,
        &mut global_state,
        &client,
        Some("default".to_string()),
        refresh_options_for_view(AppView::Projects, false, false),
        &mut refresh_state,
        &mut snapshot_dirty,
    );

    assert!(snapshot_dirty);
    assert_eq!(refresh_state.in_flight_target_view, None);
    let in_flight = refresh_state
        .in_flight_options
        .expect("projects core refresh should be active");
    assert_eq!(in_flight.scope, RefreshScope::CORE_OVERVIEW);
    assert!(!in_flight.skip_core);

    let queued = refresh_state
        .queued_refresh
        .as_ref()
        .expect("projects secondary backfill should be queued");
    assert!(queued.options.skip_core);
    assert_eq!(queued.target_view, None);
    assert!(
        queued
            .options
            .scope
            .contains(RefreshScope::LEGACY_SECONDARY)
    );
    assert!(queued.options.scope.contains(RefreshScope::NETWORK));
    assert!(queued.options.scope.contains(RefreshScope::SECURITY));
}

#[tokio::test]
async fn visible_targeted_refresh_preempts_actual_aggregate_secondary_backfill() {
    let (refresh_tx, _refresh_rx) = tokio::sync::mpsc::channel(4);
    let mut global_state = GlobalState::default();
    let client = kubectui::k8s::client::K8sClient::dummy();
    let mut snapshot_dirty = false;
    let mut refresh_state = RefreshRuntimeState::default();

    request_refresh(
        &refresh_tx,
        &mut global_state,
        &client,
        Some("default".to_string()),
        refresh_options_for_view(AppView::Projects, false, false),
        &mut refresh_state,
        &mut snapshot_dirty,
    );

    let queued = refresh_state
        .queued_refresh
        .take()
        .expect("projects secondary backfill should be queued");
    refresh_state.in_flight_id = Some(queued.request_id);
    refresh_state.in_flight_options = Some(queued.options);
    refresh_state.in_flight_namespace = queued.namespace.clone();
    refresh_state.in_flight_target_view = queued.target_view;
    refresh_state.in_flight_task = Some(tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }));

    request_refresh(
        &refresh_tx,
        &mut global_state,
        &client,
        Some("default".to_string()),
        refresh_options_for_view(AppView::Secrets, false, false),
        &mut refresh_state,
        &mut snapshot_dirty,
    );

    assert_eq!(refresh_state.in_flight_target_view, Some(AppView::Secrets));
    let in_flight = refresh_state
        .in_flight_options
        .expect("secrets refresh should preempt aggregate secondary");
    assert_eq!(in_flight.scope, RefreshScope::CONFIG);
    assert!(!in_flight.skip_core);

    let queued = refresh_state
        .queued_refresh
        .as_ref()
        .expect("aggregate secondary should continue in background");
    assert!(queued.options.skip_core);
    assert_eq!(queued.target_view, None);
    assert!(
        queued
            .options
            .scope
            .contains(RefreshScope::LEGACY_SECONDARY)
    );
    assert!(queued.options.scope.contains(RefreshScope::NETWORK));
    assert!(queued.options.scope.contains(RefreshScope::SECURITY));
}

#[tokio::test]
async fn issues_refresh_starts_core_scope_without_empty_noop_phase() {
    let (refresh_tx, _refresh_rx) = tokio::sync::mpsc::channel(4);
    let mut global_state = GlobalState::default();
    let client = kubectui::k8s::client::K8sClient::dummy();
    let mut snapshot_dirty = false;
    let mut refresh_state = RefreshRuntimeState::default();

    request_refresh(
        &refresh_tx,
        &mut global_state,
        &client,
        Some("default".to_string()),
        refresh_options_for_view(AppView::Issues, false, false),
        &mut refresh_state,
        &mut snapshot_dirty,
    );

    assert!(snapshot_dirty);
    assert_eq!(
        global_state.snapshot().view_load_state(AppView::Issues),
        kubectui::state::ViewLoadState::Loading
    );
    let in_flight = refresh_state
        .in_flight_options
        .expect("issues refresh should start immediately");
    assert!(!in_flight.scope.is_empty());
    assert!(in_flight.scope.contains(RefreshScope::CORE_OVERVIEW));
    assert!(!in_flight.skip_core);

    let queued = refresh_state
        .queued_refresh
        .as_ref()
        .expect("issues secondary backfill should be queued");
    assert!(queued.options.skip_core);
    assert!(
        queued
            .options
            .scope
            .contains(RefreshScope::LEGACY_SECONDARY)
    );
    assert!(queued.options.scope.contains(RefreshScope::FLUX));
}

#[tokio::test]
async fn disjoint_refresh_dispatch_starts_requested_scope_without_noop_phase() {
    let (refresh_tx, _refresh_rx) = tokio::sync::mpsc::channel(4);
    let mut global_state = GlobalState::default();
    let client = kubectui::k8s::client::K8sClient::dummy();
    let mut snapshot_dirty = false;
    let mut refresh_state = RefreshRuntimeState::default();

    request_refresh(
        &refresh_tx,
        &mut global_state,
        &client,
        Some("default".to_string()),
        RefreshDispatch::new(RefreshScope::DASHBOARD_WATCHED, RefreshScope::CONFIG)
            .for_view(AppView::Secrets),
        &mut refresh_state,
        &mut snapshot_dirty,
    );

    assert!(snapshot_dirty);
    let in_flight = refresh_state
        .in_flight_options
        .expect("disjoint refresh should still start immediately");
    assert_eq!(in_flight.scope, RefreshScope::CONFIG);
    assert!(!in_flight.skip_core);
    assert!(refresh_state.queued_refresh.is_none());
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

#[test]
fn namespace_switch_collects_all_follow_streams_before_closing_resource_tabs() {
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

    app.open_pod_logs_tab(ResourceRef::Pod("pod-1".to_string(), "ns".to_string()));
    if let Some(tab) = app.workbench_mut().active_tab_mut()
        && let WorkbenchTabState::PodLogs(PodLogsTabState { viewer, .. }) = &mut tab.state
    {
        viewer.pod_name = "pod-1".to_string();
        viewer.pod_namespace = "ns".to_string();
        viewer.container_name = "sidecar".to_string();
        viewer.follow_mode = true;
    }

    let streams = workbench_all_follow_streams_to_stop(&app);
    assert_eq!(
        streams,
        vec![
            ("pod-0".to_string(), "ns".to_string(), "main".to_string()),
            ("pod-1".to_string(), "ns".to_string(), "sidecar".to_string()),
        ]
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
fn flux_selection_identity_survives_watch_reorder_and_delete_updates() {
    fn kustomization(name: &str) -> FluxResourceInfo {
        FluxResourceInfo {
            name: name.to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
            ..FluxResourceInfo::default()
        }
    }

    fn snapshot(version: u64, names: &[&str]) -> ClusterSnapshot {
        ClusterSnapshot {
            snapshot_version: version,
            flux_resources: names.iter().map(|name| kustomization(name)).collect(),
            ..ClusterSnapshot::default()
        }
    }

    let expected = ResourceRef::CustomResource {
        name: "apps".to_string(),
        namespace: Some("flux-system".to_string()),
        group: "kustomize.toolkit.fluxcd.io".to_string(),
        version: "v1".to_string(),
        kind: "Kustomization".to_string(),
        plural: "kustomizations".to_string(),
    };

    let previous = snapshot(1, &["bootstrap", "apps", "platform"]);
    let reordered = snapshot(2, &["apps", "bootstrap", "platform"]);
    let mut app = AppState {
        view: AppView::FluxCDKustomizations,
        selected_idx: 1,
        ..AppState::default()
    };

    assert_eq!(selected_resource(&app, &previous), Some(expected.clone()));
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app, &previous, &reordered
    ));
    assert_eq!(app.selected_idx(), 0);
    assert_eq!(selected_resource(&app, &reordered), Some(expected.clone()));

    let previous = snapshot(3, &["bootstrap", "apps", "platform"]);
    let deleted_before_selection = snapshot(4, &["apps", "platform"]);
    app.selected_idx = 1;

    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app,
        &previous,
        &deleted_before_selection,
    ));
    assert_eq!(app.selected_idx(), 0);
    assert_eq!(
        selected_resource(&app, &deleted_before_selection),
        Some(expected)
    );
}

#[test]
fn command_palette_closes_when_watch_update_changes_selected_resource() {
    fn kustomization(name: &str) -> FluxResourceInfo {
        FluxResourceInfo {
            name: name.to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
            ..FluxResourceInfo::default()
        }
    }

    fn snapshot(version: u64, names: &[&str]) -> ClusterSnapshot {
        ClusterSnapshot {
            snapshot_version: version,
            flux_resources: names.iter().map(|name| kustomization(name)).collect(),
            ..ClusterSnapshot::default()
        }
    }

    let previous = snapshot(1, &["bootstrap", "apps", "platform"]);
    let reordered = snapshot(2, &["apps", "bootstrap", "platform"]);
    let mut app = AppState {
        view: AppView::FluxCDKustomizations,
        selected_idx: 1,
        ..AppState::default()
    };
    app.command_palette.open();

    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app, &previous, &reordered
    ));
    assert!(app.command_palette.is_open());

    let previous = snapshot(3, &["bootstrap", "apps", "platform"]);
    let selected_deleted = snapshot(4, &["bootstrap", "platform"]);
    app.selected_idx = 1;

    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app,
        &previous,
        &selected_deleted,
    ));
    assert!(!app.command_palette.is_open());
    assert_eq!(
        selected_resource(&app, &selected_deleted),
        Some(ResourceRef::CustomResource {
            name: "platform".to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
        })
    );
}

#[test]
fn flux_detail_alignment_survives_watch_reorder_and_selected_delete() {
    fn kustomization(name: &str) -> FluxResourceInfo {
        FluxResourceInfo {
            name: name.to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
            ..FluxResourceInfo::default()
        }
    }

    fn snapshot(version: u64, names: &[&str]) -> ClusterSnapshot {
        ClusterSnapshot {
            snapshot_version: version,
            flux_resources: names.iter().map(|name| kustomization(name)).collect(),
            ..ClusterSnapshot::default()
        }
    }

    fn resource(name: &str) -> ResourceRef {
        ResourceRef::CustomResource {
            name: name.to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
        }
    }

    let previous = snapshot(1, &["bootstrap", "apps", "platform"]);
    let reordered = snapshot(2, &["apps", "bootstrap", "platform"]);
    let selected = resource("apps");
    let mut app = AppState {
        view: AppView::FluxCDKustomizations,
        selected_idx: 1,
        detail_view: Some(DetailViewState {
            resource: Some(selected.clone()),
            ..DetailViewState::default()
        }),
        ..AppState::default()
    };

    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app, &previous, &reordered
    ));
    assert_eq!(selected_resource(&app, &reordered), Some(selected.clone()));
    assert_eq!(
        app.detail_view
            .as_ref()
            .and_then(|detail| detail.resource.as_ref()),
        Some(&selected)
    );

    let deleted_selected = snapshot(3, &["bootstrap", "platform"]);
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app,
        &reordered,
        &deleted_selected
    ));
    assert_eq!(
        selected_resource(&app, &deleted_selected),
        Some(resource("bootstrap"))
    );
    assert!(app.detail_view.is_none());
}

#[test]
fn flux_delete_selected_resource_falls_back_to_nearest_neighbor() {
    fn kustomization(name: &str) -> FluxResourceInfo {
        FluxResourceInfo {
            name: name.to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
            ..FluxResourceInfo::default()
        }
    }

    fn snapshot(version: u64, names: &[&str]) -> ClusterSnapshot {
        ClusterSnapshot {
            snapshot_version: version,
            flux_resources: names.iter().map(|name| kustomization(name)).collect(),
            ..ClusterSnapshot::default()
        }
    }

    fn selected_name(app: &AppState, snapshot: &ClusterSnapshot) -> Option<String> {
        selected_resource(app, snapshot).map(|resource| resource.name().to_string())
    }

    let previous = snapshot(1, &["bootstrap", "apps", "platform"]);

    let mut first = AppState {
        view: AppView::FluxCDKustomizations,
        selected_idx: 0,
        ..AppState::default()
    };
    let deleted_first = snapshot(2, &["apps", "platform"]);
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut first,
        &previous,
        &deleted_first
    ));
    assert_eq!(first.selected_idx(), 0);
    assert_eq!(
        selected_name(&first, &deleted_first).as_deref(),
        Some("apps")
    );

    let mut middle = AppState {
        view: AppView::FluxCDKustomizations,
        selected_idx: 1,
        ..AppState::default()
    };
    let deleted_middle = snapshot(3, &["bootstrap", "platform"]);
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut middle,
        &previous,
        &deleted_middle
    ));
    assert_eq!(middle.selected_idx(), 1);
    assert_eq!(
        selected_name(&middle, &deleted_middle).as_deref(),
        Some("platform")
    );

    let mut last = AppState {
        view: AppView::FluxCDKustomizations,
        selected_idx: 2,
        ..AppState::default()
    };
    let deleted_last = snapshot(4, &["bootstrap", "apps"]);
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut last,
        &previous,
        &deleted_last
    ));
    assert_eq!(last.selected_idx(), 1);
    assert_eq!(selected_name(&last, &deleted_last).as_deref(), Some("apps"));
}

#[test]
fn flux_active_search_selection_fallback_is_predictable() {
    fn kustomization(name: &str, status: &str) -> FluxResourceInfo {
        FluxResourceInfo {
            name: name.to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
            status: status.to_string(),
            ..FluxResourceInfo::default()
        }
    }

    fn snapshot(version: u64, resources: &[(&str, &str)]) -> ClusterSnapshot {
        ClusterSnapshot {
            snapshot_version: version,
            flux_resources: resources
                .iter()
                .map(|(name, status)| kustomization(name, status))
                .collect(),
            ..ClusterSnapshot::default()
        }
    }

    fn selected_name(app: &AppState, snapshot: &ClusterSnapshot) -> Option<String> {
        selected_resource(app, snapshot).map(|resource| resource.name().to_string())
    }

    let previous = snapshot(
        1,
        &[
            ("bootstrap", "Ready"),
            ("apps", "Ready"),
            ("platform", "Ready"),
        ],
    );
    let reordered = snapshot(
        2,
        &[
            ("apps", "Ready"),
            ("bootstrap", "Ready"),
            ("platform", "Ready"),
        ],
    );
    let mut app = AppState {
        view: AppView::FluxCDKustomizations,
        selected_idx: 1,
        search_query: "ready".to_string(),
        ..AppState::default()
    };

    assert_eq!(selected_name(&app, &previous).as_deref(), Some("apps"));
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app, &previous, &reordered
    ));
    assert_eq!(app.selected_idx(), 0);
    assert_eq!(selected_name(&app, &reordered).as_deref(), Some("apps"));
    assert_eq!(app.status_message(), None);

    let hidden_by_search = snapshot(
        3,
        &[
            ("bootstrap", "Ready"),
            ("apps", "Stalled"),
            ("platform", "Ready"),
        ],
    );
    app.selected_idx = 1;

    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app,
        &previous,
        &hidden_by_search
    ));
    assert_eq!(app.selected_idx(), 1);
    assert_eq!(
        selected_name(&app, &hidden_by_search).as_deref(),
        Some("platform")
    );
    assert_eq!(
        app.status_message(),
        Some("Selected resource no longer matches search; moved to nearest visible result.")
    );
}

#[test]
fn flux_sort_reorder_preserves_selected_identity() {
    fn kustomization(name: &str, created_at: AppTimestamp) -> FluxResourceInfo {
        FluxResourceInfo {
            name: name.to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
            created_at: Some(created_at),
            ..FluxResourceInfo::default()
        }
    }

    fn snapshot(version: u64, now: AppTimestamp, resources: &[(&str, i64)]) -> ClusterSnapshot {
        ClusterSnapshot {
            snapshot_version: version,
            flux_resources: resources
                .iter()
                .map(|(name, age_secs)| {
                    kustomization(
                        name,
                        now.checked_sub(age_secs.seconds())
                            .expect("timestamp in range"),
                    )
                })
                .collect(),
            ..ClusterSnapshot::default()
        }
    }

    fn selected_name(app: &AppState, snapshot: &ClusterSnapshot) -> Option<String> {
        selected_resource(app, snapshot).map(|resource| resource.name().to_string())
    }

    let now = now();

    let previous_name = snapshot(1, now, &[("platform", 30), ("apps", 20), ("bootstrap", 10)]);
    let reordered_name = snapshot(2, now, &[("apps", 20), ("bootstrap", 10), ("platform", 30)]);
    let mut name_sorted = AppState {
        view: AppView::FluxCDKustomizations,
        selected_idx: 2,
        workload_sort: Some(WorkloadSortState::new(WorkloadSortColumn::Name, false)),
        ..AppState::default()
    };

    assert_eq!(
        selected_name(&name_sorted, &previous_name).as_deref(),
        Some("platform")
    );
    assert!(!preserve_selection_identity_after_snapshot_change(
        &mut name_sorted,
        &previous_name,
        &reordered_name
    ));
    assert_eq!(name_sorted.selected_idx(), 2);
    assert_eq!(
        selected_name(&name_sorted, &reordered_name).as_deref(),
        Some("platform")
    );

    let previous_age = snapshot(
        3,
        now,
        &[("apps", 300), ("bootstrap", 200), ("platform", 100)],
    );
    let moved_by_age = snapshot(
        4,
        now,
        &[("platform", 200), ("bootstrap", 300), ("apps", 100)],
    );
    let mut age_sorted = AppState {
        view: AppView::FluxCDKustomizations,
        selected_idx: 0,
        workload_sort: Some(WorkloadSortState::new(WorkloadSortColumn::Age, true)),
        ..AppState::default()
    };

    assert_eq!(
        selected_name(&age_sorted, &previous_age).as_deref(),
        Some("apps")
    );
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut age_sorted,
        &previous_age,
        &moved_by_age
    ));
    assert_eq!(age_sorted.selected_idx(), 2);
    assert_eq!(
        selected_name(&age_sorted, &moved_by_age).as_deref(),
        Some("apps")
    );
}

#[test]
fn flux_reconcile_completion_after_watch_reorder_keeps_ui_identity_aligned() {
    fn kustomization(name: &str, last_reconcile_time: AppTimestamp) -> FluxResourceInfo {
        FluxResourceInfo {
            name: name.to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
            status: "Ready".to_string(),
            last_reconcile_time: Some(last_reconcile_time),
            ..FluxResourceInfo::default()
        }
    }

    fn snapshot(version: u64, resources: &[(&str, AppTimestamp)]) -> ClusterSnapshot {
        ClusterSnapshot {
            snapshot_version: version,
            flux_resources: resources
                .iter()
                .map(|(name, last_reconcile_time)| kustomization(name, *last_reconcile_time))
                .collect(),
            ..ClusterSnapshot::default()
        }
    }

    fn resource(name: &str) -> ResourceRef {
        ResourceRef::CustomResource {
            name: name.to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
        }
    }

    let base = now();
    let apps_ref = resource("apps");
    let previous = snapshot(
        1,
        &[("bootstrap", base), ("apps", base), ("platform", base)],
    );
    let reordered = snapshot(
        2,
        &[
            (
                "apps",
                base.checked_add(30.seconds()).expect("timestamp in range"),
            ),
            ("bootstrap", base),
            ("platform", base),
        ],
    );
    let mut app = AppState {
        view: AppView::FluxCDKustomizations,
        selected_idx: 1,
        detail_view: Some(DetailViewState {
            resource: Some(apps_ref.clone()),
            ..DetailViewState::default()
        }),
        ..AppState::default()
    };
    let entry_id = app.record_action_pending(
        ActionKind::FluxReconcile,
        AppView::FluxCDKustomizations,
        Some(apps_ref.clone()),
        "Kustomization 'apps'".to_string(),
        "Requesting reconcile for Kustomization 'apps'".to_string(),
    );
    let mut pending = vec![PendingFluxReconcileVerification {
        action_history_id: entry_id,
        resource: apps_ref.clone(),
        resource_label: "Kustomization 'apps'".to_string(),
        baseline: Some(flux_observed_state(&kustomization("apps", base))),
        deadline: Instant::now() + Duration::from_secs(5),
    }];

    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app, &previous, &reordered
    ));
    assert_eq!(app.selected_idx(), 0);
    assert_eq!(selected_resource(&app, &reordered), Some(apps_ref.clone()));

    assert!(process_flux_reconcile_verifications(
        &mut app,
        &reordered,
        &mut pending,
        &mut |a, msg| a.set_status(msg),
    ));

    assert!(pending.is_empty());
    assert_eq!(selected_resource(&app, &reordered), Some(apps_ref.clone()));
    assert_eq!(
        app.detail_view
            .as_ref()
            .and_then(|detail| detail.resource.as_ref()),
        Some(&apps_ref)
    );
    let entry = app
        .action_history()
        .find_by_id(entry_id)
        .expect("history entry");
    assert_eq!(entry.status, ActionStatus::Succeeded);
    assert_eq!(
        entry.target.as_ref().map(|target| &target.resource),
        Some(&apps_ref)
    );
    assert!(entry.message.contains("Kustomization 'apps'"));
    assert!(
        app.status_message()
            .expect("status message")
            .contains("Kustomization 'apps'")
    );
}

#[test]
fn flux_secondary_pane_state_tracks_selected_resource_identity_after_reorder() {
    fn kustomization(name: &str) -> FluxResourceInfo {
        FluxResourceInfo {
            name: name.to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
            ..FluxResourceInfo::default()
        }
    }

    fn snapshot(version: u64, names: &[&str]) -> ClusterSnapshot {
        ClusterSnapshot {
            snapshot_version: version,
            flux_resources: names.iter().map(|name| kustomization(name)).collect(),
            ..ClusterSnapshot::default()
        }
    }

    fn selected_name(app: &AppState, snapshot: &ClusterSnapshot) -> Option<String> {
        selected_resource(app, snapshot).map(|resource| resource.name().to_string())
    }

    let previous = snapshot(1, &["bootstrap", "apps", "platform"]);
    let reordered = snapshot(2, &["apps", "bootstrap", "platform"]);
    let mut app = AppState {
        view: AppView::FluxCDKustomizations,
        focus: Focus::Content,
        selected_idx: 1,
        content_pane_focus: ContentPaneFocus::Secondary,
        content_detail_scroll: 12,
        ..AppState::default()
    };

    assert_eq!(selected_name(&app, &previous).as_deref(), Some("apps"));
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app, &previous, &reordered
    ));
    assert_eq!(selected_name(&app, &reordered).as_deref(), Some("apps"));
    assert_eq!(app.selected_idx(), 0);
    assert_eq!(app.content_pane_focus(), ContentPaneFocus::Secondary);
    assert!(app.content_secondary_pane_active());
    assert_eq!(app.content_detail_scroll, 12);
    assert!(super::should_handle_root_enter(
        KeyEvent::from(KeyCode::Enter),
        &app
    ));
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('j'))),
        AppAction::None
    );
    assert_eq!(app.content_detail_scroll, 13);
    assert_eq!(app.selected_idx(), 0);
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::Char('k'))),
        AppAction::None
    );
    assert_eq!(app.content_detail_scroll, 12);
    assert_eq!(
        app.handle_key_event(KeyEvent::from(KeyCode::PageDown)),
        AppAction::None
    );
    assert_eq!(app.content_detail_scroll, 22);
    assert_eq!(app.selected_idx(), 0);

    let deleted_selected = snapshot(3, &["bootstrap", "platform"]);
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app,
        &reordered,
        &deleted_selected,
    ));
    assert_eq!(
        selected_name(&app, &deleted_selected).as_deref(),
        Some("bootstrap")
    );
    assert_eq!(app.content_pane_focus(), ContentPaneFocus::Secondary);
    assert_eq!(app.content_detail_scroll, 0);
}

#[test]
fn flux_repeated_watch_churn_never_moves_highlight_to_wrong_resource() {
    fn kustomization(name: &str) -> FluxResourceInfo {
        FluxResourceInfo {
            name: name.to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
            ..FluxResourceInfo::default()
        }
    }

    fn snapshot(version: u64, names: &[String]) -> ClusterSnapshot {
        ClusterSnapshot {
            snapshot_version: version,
            flux_resources: names.iter().map(|name| kustomization(name)).collect(),
            ..ClusterSnapshot::default()
        }
    }

    fn names_with_target_at(position: usize) -> Vec<String> {
        let mut names = (0..20)
            .filter(|idx| *idx != 7)
            .map(|idx| format!("resource-{idx:02}"))
            .collect::<Vec<_>>();
        names.insert(position.min(names.len()), "resource-07".to_string());
        names
    }

    let target = ResourceRef::CustomResource {
        name: "resource-07".to_string(),
        namespace: Some("flux-system".to_string()),
        group: "kustomize.toolkit.fluxcd.io".to_string(),
        version: "v1".to_string(),
        kind: "Kustomization".to_string(),
        plural: "kustomizations".to_string(),
    };
    let mut previous = snapshot(1, &names_with_target_at(7));
    let mut app = AppState {
        view: AppView::FluxCDKustomizations,
        selected_idx: 7,
        ..AppState::default()
    };

    assert_eq!(selected_resource(&app, &previous), Some(target.clone()));

    for (version, target_position) in [0, 19, 2, 16, 4, 12, 7].into_iter().enumerate() {
        let current = snapshot(version as u64 + 2, &names_with_target_at(target_position));
        assert!(preserve_selection_identity_after_snapshot_change(
            &mut app, &previous, &current
        ));
        assert_eq!(app.selected_idx(), target_position);
        assert_eq!(selected_resource(&app, &current), Some(target.clone()));
        assert_eq!(app.status_message(), None);
        previous = current;
    }
}

#[test]
fn watched_resource_views_preserve_selected_identity_after_reorder() {
    fn selected_name(app: &AppState, snapshot: &ClusterSnapshot) -> Option<String> {
        selected_resource(app, snapshot).map(|resource| resource.name().to_string())
    }

    fn assert_preserves(
        view: AppView,
        previous: ClusterSnapshot,
        current: ClusterSnapshot,
        initial_idx: usize,
        expected_idx: usize,
        expected_name: &str,
    ) {
        let mut app = AppState {
            view,
            selected_idx: initial_idx,
            ..AppState::default()
        };

        assert_eq!(
            selected_name(&app, &previous).as_deref(),
            Some(expected_name)
        );
        assert!(preserve_selection_identity_after_snapshot_change(
            &mut app, &previous, &current
        ));
        assert_eq!(app.selected_idx(), expected_idx);
        assert_eq!(
            selected_name(&app, &current).as_deref(),
            Some(expected_name)
        );
        assert_eq!(app.status_message(), None);
    }

    fn pod(name: &str) -> PodInfo {
        PodInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            ..PodInfo::default()
        }
    }

    fn deployment(name: &str) -> DeploymentInfo {
        DeploymentInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            ..DeploymentInfo::default()
        }
    }

    fn service(name: &str) -> ServiceInfo {
        ServiceInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            ..ServiceInfo::default()
        }
    }

    fn job(name: &str) -> JobInfo {
        JobInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            ..JobInfo::default()
        }
    }

    fn namespace(name: &str) -> NamespaceInfo {
        NamespaceInfo {
            name: name.to_string(),
            ..NamespaceInfo::default()
        }
    }

    assert_preserves(
        AppView::Pods,
        ClusterSnapshot {
            pods: vec![pod("api-0"), pod("api-1"), pod("api-2")],
            ..ClusterSnapshot::default()
        },
        ClusterSnapshot {
            pods: vec![pod("api-1"), pod("api-0"), pod("api-2")],
            ..ClusterSnapshot::default()
        },
        1,
        0,
        "api-1",
    );
    assert_preserves(
        AppView::Deployments,
        ClusterSnapshot {
            deployments: vec![
                deployment("worker"),
                deployment("api"),
                deployment("frontend"),
            ],
            ..ClusterSnapshot::default()
        },
        ClusterSnapshot {
            deployments: vec![
                deployment("api"),
                deployment("frontend"),
                deployment("worker"),
            ],
            ..ClusterSnapshot::default()
        },
        1,
        0,
        "api",
    );
    assert_preserves(
        AppView::Services,
        ClusterSnapshot {
            services: vec![service("web"), service("api"), service("metrics")],
            ..ClusterSnapshot::default()
        },
        ClusterSnapshot {
            services: vec![service("metrics"), service("web"), service("api")],
            ..ClusterSnapshot::default()
        },
        1,
        2,
        "api",
    );
    assert_preserves(
        AppView::Jobs,
        ClusterSnapshot {
            jobs: vec![job("seed"), job("backup"), job("cleanup")],
            ..ClusterSnapshot::default()
        },
        ClusterSnapshot {
            jobs: vec![job("cleanup"), job("seed"), job("backup")],
            ..ClusterSnapshot::default()
        },
        1,
        2,
        "backup",
    );
    assert_preserves(
        AppView::Namespaces,
        ClusterSnapshot {
            namespace_list: vec![
                namespace("default"),
                namespace("prod"),
                namespace("staging"),
            ],
            ..ClusterSnapshot::default()
        },
        ClusterSnapshot {
            namespace_list: vec![
                namespace("staging"),
                namespace("default"),
                namespace("prod"),
            ],
            ..ClusterSnapshot::default()
        },
        1,
        2,
        "prod",
    );
}

#[test]
fn watched_resource_detail_closes_when_selected_resource_is_deleted() {
    fn pod(name: &str) -> PodInfo {
        PodInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            ..PodInfo::default()
        }
    }

    fn deployment(name: &str) -> DeploymentInfo {
        DeploymentInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            ..DeploymentInfo::default()
        }
    }

    let selected_pod = ResourceRef::Pod("api-1".to_string(), "default".to_string());
    let previous_pods = ClusterSnapshot {
        pods: vec![pod("api-0"), pod("api-1"), pod("api-2")],
        ..ClusterSnapshot::default()
    };
    let current_pods = ClusterSnapshot {
        pods: vec![pod("api-0"), pod("api-2")],
        ..ClusterSnapshot::default()
    };
    let mut pod_app = AppState {
        view: AppView::Pods,
        selected_idx: 1,
        detail_view: Some(DetailViewState {
            resource: Some(selected_pod.clone()),
            ..DetailViewState::default()
        }),
        ..AppState::default()
    };

    assert_eq!(
        selected_resource(&pod_app, &previous_pods),
        Some(selected_pod)
    );
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut pod_app,
        &previous_pods,
        &current_pods
    ));
    assert_eq!(pod_app.selected_idx(), 1);
    assert_eq!(
        selected_resource(&pod_app, &current_pods),
        Some(ResourceRef::Pod("api-2".to_string(), "default".to_string()))
    );
    assert!(pod_app.detail_view.is_none());

    let selected_deployment = ResourceRef::Deployment("api".to_string(), "default".to_string());
    let previous_deployments = ClusterSnapshot {
        deployments: vec![
            deployment("worker"),
            deployment("api"),
            deployment("frontend"),
        ],
        ..ClusterSnapshot::default()
    };
    let current_deployments = ClusterSnapshot {
        deployments: vec![deployment("worker"), deployment("frontend")],
        ..ClusterSnapshot::default()
    };
    let mut deployment_app = AppState {
        view: AppView::Deployments,
        selected_idx: 1,
        detail_view: Some(DetailViewState {
            resource: Some(selected_deployment.clone()),
            ..DetailViewState::default()
        }),
        ..AppState::default()
    };

    assert_eq!(
        selected_resource(&deployment_app, &previous_deployments),
        Some(selected_deployment)
    );
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut deployment_app,
        &previous_deployments,
        &current_deployments
    ));
    assert_eq!(deployment_app.selected_idx(), 1);
    assert_eq!(
        selected_resource(&deployment_app, &current_deployments),
        Some(ResourceRef::Deployment(
            "frontend".to_string(),
            "default".to_string()
        ))
    );
    assert!(deployment_app.detail_view.is_none());
}

#[test]
fn watched_resource_detail_stays_open_when_selected_resource_reorders() {
    fn pod(name: &str) -> PodInfo {
        PodInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            ..PodInfo::default()
        }
    }

    let selected = ResourceRef::Pod("api-1".to_string(), "default".to_string());
    let previous = ClusterSnapshot {
        pods: vec![pod("api-0"), pod("api-1"), pod("api-2")],
        ..ClusterSnapshot::default()
    };
    let current = ClusterSnapshot {
        pods: vec![pod("api-1"), pod("api-0"), pod("api-2")],
        ..ClusterSnapshot::default()
    };
    let mut app = AppState {
        view: AppView::Pods,
        selected_idx: 1,
        detail_view: Some(DetailViewState {
            resource: Some(selected.clone()),
            ..DetailViewState::default()
        }),
        ..AppState::default()
    };

    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app, &previous, &current
    ));
    assert_eq!(app.selected_idx(), 0);
    assert_eq!(selected_resource(&app, &current), Some(selected.clone()));
    assert_eq!(
        app.detail_view
            .as_ref()
            .and_then(|detail| detail.resource.as_ref()),
        Some(&selected)
    );
}

#[test]
fn watched_resource_active_search_preserves_visible_selection_after_reorder() {
    fn pod(name: &str, status: &str) -> PodInfo {
        PodInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            status: status.to_string(),
            ..PodInfo::default()
        }
    }

    let selected = ResourceRef::Pod("api-1".to_string(), "default".to_string());
    let previous = ClusterSnapshot {
        pods: vec![
            pod("cache-0", "Running"),
            pod("api-0", "Running"),
            pod("api-1", "Running"),
        ],
        ..ClusterSnapshot::default()
    };
    let current = ClusterSnapshot {
        pods: vec![
            pod("api-1", "Running"),
            pod("cache-0", "Running"),
            pod("api-0", "Running"),
        ],
        ..ClusterSnapshot::default()
    };
    let mut app = AppState {
        view: AppView::Pods,
        selected_idx: 1,
        search_query: "api".to_string(),
        ..AppState::default()
    };

    assert_eq!(selected_resource(&app, &previous), Some(selected.clone()));
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app, &previous, &current
    ));
    assert_eq!(app.selected_idx(), 0);
    assert_eq!(selected_resource(&app, &current), Some(selected));
    assert_eq!(app.status_message(), None);
}

#[test]
fn watched_resource_active_search_fallback_closes_stale_detail() {
    fn pod(name: &str, status: &str) -> PodInfo {
        PodInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            status: status.to_string(),
            ..PodInfo::default()
        }
    }

    fn service(name: &str, type_: &str) -> ServiceInfo {
        ServiceInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            type_: type_.to_string(),
            ..ServiceInfo::default()
        }
    }

    let selected_pod = ResourceRef::Pod("api-1".to_string(), "default".to_string());
    let previous_pods = ClusterSnapshot {
        pods: vec![
            pod("api-0", "Running"),
            pod("api-1", "Running"),
            pod("api-2", "Running"),
        ],
        ..ClusterSnapshot::default()
    };
    let current_pods = ClusterSnapshot {
        pods: vec![
            pod("api-0", "Running"),
            pod("api-1", "Pending"),
            pod("api-2", "Running"),
        ],
        ..ClusterSnapshot::default()
    };
    let mut pod_app = AppState {
        view: AppView::Pods,
        selected_idx: 1,
        search_query: "Running".to_string(),
        detail_view: Some(DetailViewState {
            resource: Some(selected_pod.clone()),
            ..DetailViewState::default()
        }),
        ..AppState::default()
    };

    assert_eq!(
        selected_resource(&pod_app, &previous_pods),
        Some(selected_pod)
    );
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut pod_app,
        &previous_pods,
        &current_pods
    ));
    assert_eq!(pod_app.selected_idx(), 1);
    assert_eq!(
        selected_resource(&pod_app, &current_pods),
        Some(ResourceRef::Pod("api-2".to_string(), "default".to_string()))
    );
    assert!(pod_app.detail_view.is_none());
    assert_eq!(
        pod_app.status_message(),
        Some("Selected resource no longer matches search; moved to nearest visible result.")
    );

    let selected_service = ResourceRef::Service("api".to_string(), "default".to_string());
    let previous_services = ClusterSnapshot {
        services: vec![
            service("web", "LoadBalancer"),
            service("api", "LoadBalancer"),
            service("metrics", "LoadBalancer"),
        ],
        ..ClusterSnapshot::default()
    };
    let current_services = ClusterSnapshot {
        services: vec![
            service("web", "LoadBalancer"),
            service("api", "ClusterIP"),
            service("metrics", "LoadBalancer"),
        ],
        ..ClusterSnapshot::default()
    };
    let mut service_app = AppState {
        view: AppView::Services,
        selected_idx: 1,
        search_query: "LoadBalancer".to_string(),
        detail_view: Some(DetailViewState {
            resource: Some(selected_service.clone()),
            ..DetailViewState::default()
        }),
        ..AppState::default()
    };

    assert_eq!(
        selected_resource(&service_app, &previous_services),
        Some(selected_service)
    );
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut service_app,
        &previous_services,
        &current_services
    ));
    assert_eq!(service_app.selected_idx(), 1);
    assert_eq!(
        selected_resource(&service_app, &current_services),
        Some(ResourceRef::Service(
            "metrics".to_string(),
            "default".to_string()
        ))
    );
    assert!(service_app.detail_view.is_none());
    assert_eq!(
        service_app.status_message(),
        Some("Selected resource no longer matches search; moved to nearest visible result.")
    );
}

#[test]
fn watched_resource_active_search_status_clears_when_selection_matches_again() {
    fn pod(name: &str, status: &str) -> PodInfo {
        PodInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            status: status.to_string(),
            ..PodInfo::default()
        }
    }

    let previous = ClusterSnapshot {
        pods: vec![
            pod("api-0", "Running"),
            pod("api-1", "Running"),
            pod("api-2", "Running"),
        ],
        ..ClusterSnapshot::default()
    };
    let hidden = ClusterSnapshot {
        pods: vec![
            pod("api-0", "Running"),
            pod("api-1", "Pending"),
            pod("api-2", "Running"),
        ],
        ..ClusterSnapshot::default()
    };
    let visible_again = ClusterSnapshot {
        pods: vec![
            pod("api-0", "Running"),
            pod("api-1", "Running"),
            pod("api-2", "Running"),
        ],
        ..ClusterSnapshot::default()
    };
    let mut app = AppState {
        view: AppView::Pods,
        selected_idx: 1,
        search_query: "Running".to_string(),
        ..AppState::default()
    };

    assert_eq!(
        selected_resource(&app, &previous),
        Some(ResourceRef::Pod("api-1".to_string(), "default".to_string()))
    );
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app, &previous, &hidden
    ));
    assert_eq!(
        selected_resource(&app, &hidden),
        Some(ResourceRef::Pod("api-2".to_string(), "default".to_string()))
    );
    assert_eq!(
        app.status_message(),
        Some("Selected resource no longer matches search; moved to nearest visible result.")
    );

    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app,
        &hidden,
        &visible_again
    ));
    assert_eq!(app.selected_idx(), 2);
    assert_eq!(
        selected_resource(&app, &visible_again),
        Some(ResourceRef::Pod("api-2".to_string(), "default".to_string()))
    );
    assert_eq!(app.status_message(), None);
}

#[test]
fn watched_resource_active_search_visible_selection_preserves_unrelated_status() {
    fn pod(name: &str, status: &str) -> PodInfo {
        PodInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            status: status.to_string(),
            ..PodInfo::default()
        }
    }

    let selected = ResourceRef::Pod("api-1".to_string(), "default".to_string());
    let previous = ClusterSnapshot {
        pods: vec![
            pod("cache-0", "Running"),
            pod("api-0", "Running"),
            pod("api-1", "Running"),
        ],
        ..ClusterSnapshot::default()
    };
    let current = ClusterSnapshot {
        pods: vec![
            pod("api-1", "Running"),
            pod("cache-0", "Running"),
            pod("api-0", "Running"),
        ],
        ..ClusterSnapshot::default()
    };
    let mut app = AppState {
        view: AppView::Pods,
        selected_idx: 1,
        search_query: "api".to_string(),
        ..AppState::default()
    };
    app.set_status("Saved workspace: ops".to_string());

    assert_eq!(selected_resource(&app, &previous), Some(selected.clone()));
    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app, &previous, &current
    ));
    assert_eq!(selected_resource(&app, &current), Some(selected));
    assert_eq!(app.status_message(), Some("Saved workspace: ops"));
}

#[test]
fn watched_resource_active_search_empty_fallback_status_clears_when_results_return() {
    fn pod(name: &str, status: &str) -> PodInfo {
        PodInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            status: status.to_string(),
            ..PodInfo::default()
        }
    }

    let previous = ClusterSnapshot {
        pods: vec![
            pod("api-0", "Running"),
            pod("api-1", "Running"),
            pod("api-2", "Running"),
        ],
        ..ClusterSnapshot::default()
    };
    let hidden = ClusterSnapshot {
        pods: vec![
            pod("api-0", "Pending"),
            pod("api-1", "Pending"),
            pod("api-2", "Pending"),
        ],
        ..ClusterSnapshot::default()
    };
    let visible_again = ClusterSnapshot {
        pods: vec![
            pod("api-0", "Running"),
            pod("api-1", "Pending"),
            pod("api-2", "Pending"),
        ],
        ..ClusterSnapshot::default()
    };
    let mut app = AppState {
        view: AppView::Pods,
        selected_idx: 1,
        search_query: "Running".to_string(),
        detail_view: Some(DetailViewState {
            resource: Some(ResourceRef::Pod("api-1".to_string(), "default".to_string())),
            ..DetailViewState::default()
        }),
        ..AppState::default()
    };

    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app, &previous, &hidden
    ));
    assert_eq!(selected_resource(&app, &hidden), None);
    assert!(app.detail_view.is_none());
    assert_eq!(
        app.status_message(),
        Some("Selected resource no longer matches search; no visible results.")
    );

    assert!(preserve_selection_identity_after_snapshot_change(
        &mut app,
        &hidden,
        &visible_again
    ));
    assert_eq!(app.selected_idx(), 0);
    assert_eq!(
        selected_resource(&app, &visible_again),
        Some(ResourceRef::Pod("api-0".to_string(), "default".to_string()))
    );
    assert_eq!(app.status_message(), None);
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
    app.set_status(SELECTION_SEARCH_FALLBACK_STATUS.to_string());

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
    assert_eq!(app.status_message(), None);
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
fn normalize_recent_events_uses_stable_tiebreakers() {
    let now = now();
    let base = K8sEventInfo {
        last_seen: Some(now),
        count: 1,
        ..K8sEventInfo::default()
    };
    let first = vec![
        K8sEventInfo {
            name: "event-b".into(),
            namespace: "prod".into(),
            involved_object: "pod/api".into(),
            type_: "Warning".into(),
            reason: "BackOff".into(),
            message: "retry".into(),
            ..base.clone()
        },
        K8sEventInfo {
            name: "event-a".into(),
            namespace: "prod".into(),
            involved_object: "pod/api".into(),
            type_: "Normal".into(),
            reason: "Started".into(),
            message: "started".into(),
            ..base.clone()
        },
    ];
    let second = first.iter().cloned().rev().collect::<Vec<_>>();

    let normalized_first = normalize_recent_events(first);
    let normalized_second = normalize_recent_events(second);
    let names = |events: &[K8sEventInfo]| {
        events
            .iter()
            .map(|event| event.name.clone())
            .collect::<Vec<_>>()
    };

    assert_eq!(names(&normalized_first), vec!["event-a", "event-b"]);
    assert_eq!(names(&normalized_second), names(&normalized_first));
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
    assert_eq!(dispatch.primary_scope, RefreshScope::DASHBOARD_WATCHED);
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
    assert!(issues.options.scope.contains(RefreshScope::CORE_OVERVIEW));
    assert!(
        issues
            .options
            .scope
            .contains(RefreshScope::LEGACY_SECONDARY)
    );
    assert!(issues.options.scope.contains(RefreshScope::FLUX));
    assert!(issues.options.scope.contains(RefreshScope::SECURITY));
    assert_eq!(health_report.primary_scope, issues.primary_scope);
    assert_eq!(health_report.options.scope, issues.options.scope);
}

#[test]
fn project_and_governance_refresh_profiles_keep_full_workload_context() {
    let projects = refresh_options_for_view(AppView::Projects, false, false);
    let governance = refresh_options_for_view(AppView::Governance, false, false);

    assert_eq!(projects.primary_scope, RefreshScope::CORE_OVERVIEW);
    assert!(projects.options.scope.contains(RefreshScope::CORE_OVERVIEW));
    assert_eq!(governance.primary_scope, RefreshScope::CORE_OVERVIEW);
    assert!(
        governance
            .options
            .scope
            .contains(RefreshScope::CORE_OVERVIEW)
    );
    assert_eq!(
        watch_scope_for_view(AppView::Projects),
        RefreshScope::CORE_OVERVIEW
    );
    assert_eq!(
        watch_scope_for_view(AppView::Governance),
        RefreshScope::CORE_OVERVIEW
    );
}

#[test]
fn flux_views_use_polling_refresh_not_watch_scope() {
    for view in [
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
    ] {
        let dispatch = refresh_options_for_view(view, false, false);

        assert_eq!(dispatch.primary_scope, RefreshScope::FLUX, "{view:?}");
        assert_eq!(dispatch.options.scope, RefreshScope::FLUX, "{view:?}");
        assert_eq!(watch_scope_for_view(view), RefreshScope::NONE, "{view:?}");
    }
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
