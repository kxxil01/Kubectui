use jiff::ToSpan;

use super::flux_reconcile::{
    flux_observed_state, flux_reconcile_progress_observed, process_flux_reconcile_verifications,
};
use super::{
    ExtensionFetchResult, MAX_RECENT_EVENTS_CACHE_ITEMS, PendingFluxReconcileVerification,
    apply_extension_fetch_result, map_palette_detail_action, mutation_refresh_options,
    normalize_recent_events, palette_action_requires_loaded_detail, prepare_bookmark_target,
    queued_refresh_requires_two_phase, refresh_options_for_view, selected_extension_crd,
    selected_flux_reconcile_resource, selected_resource, workbench_follow_streams_to_stop,
};
use kubectui::{
    action_history::ActionKind,
    app::{AppAction, AppState, AppView, DetailViewState, ResourceRef},
    bookmarks::BookmarkEntry,
    k8s::dtos::{CustomResourceDefinitionInfo, FluxResourceInfo, K8sEventInfo, NodeInfo},
    policy::DetailAction,
    state::{ClusterSnapshot, RefreshOptions, RefreshScope},
    time::{AppTimestamp, now},
    workbench::{PodLogsTabState, WorkbenchTabState},
};
use std::time::{Duration, Instant};

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
fn closing_non_logs_workbench_does_not_collect_streams() {
    let app = AppState::default();
    let streams = workbench_follow_streams_to_stop(&app, AppAction::WorkbenchCloseActiveTab);
    assert!(streams.is_empty());
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
fn services_and_issues_refresh_profiles_split_background_scopes() {
    let services = refresh_options_for_view(AppView::Services, false, false);
    let issues = refresh_options_for_view(AppView::Issues, false, false);

    assert_eq!(services.primary_scope, RefreshScope::SERVICES);
    assert!(services.options.scope.contains(RefreshScope::NETWORK));
    assert_eq!(issues.primary_scope, RefreshScope::CORE_OVERVIEW);
    assert!(
        issues
            .options
            .scope
            .contains(RefreshScope::LEGACY_SECONDARY)
    );
    assert!(issues.options.scope.contains(RefreshScope::FLUX));
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
fn palette_drain_maps_to_confirmation_action() {
    assert_eq!(
        map_palette_detail_action(DetailAction::Drain),
        AppAction::ConfirmDrainNode
    );
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
