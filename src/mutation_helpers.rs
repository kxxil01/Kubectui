//! Pure helper functions extracted from the main event loop.
//!
//! These cover mutation status updates, refresh-scope construction,
//! palette-action mapping, and view-specific refresh preferences.

use std::time::{Duration, Instant};

use kubectui::{
    app::{AppAction, AppState, AppView, Focus, ResourceRef},
    policy::DetailAction,
    state::RefreshScope,
};

use crate::async_types::{DeferredRefreshTrigger, RefreshDispatch};
use crate::selection_helpers::namespace_scope;

/// Timeout applied to transient status messages and toasts.
pub const STATUS_MESSAGE_TIMEOUT_SECS: u64 = 12;

/// Builds a fast (core-overview only) refresh dispatch, optionally including Flux.
pub fn fast_refresh_options(include_flux: bool) -> RefreshDispatch {
    let mut scope = RefreshScope::CORE_OVERVIEW;
    if include_flux {
        scope = scope.union(RefreshScope::FLUX);
    }
    RefreshDispatch::new(scope, scope)
}

/// Sets a transient status bar message and pushes a matching toast.
pub fn set_transient_status(
    app: &mut AppState,
    status_message_clear_at: &mut Option<Instant>,
    message: impl Into<String>,
) {
    let msg = message.into();
    app.push_toast(msg.clone(), false);
    app.set_status(msg);
    *status_message_clear_at =
        Some(Instant::now() + Duration::from_secs(STATUS_MESSAGE_TIMEOUT_SECS));
}

/// Clears the detail panel, returns focus to content, and shows a transient message.
pub fn begin_detail_mutation(
    app: &mut AppState,
    status_message_clear_at: &mut Option<Instant>,
    message: impl Into<String>,
) {
    app.detail_view = None;
    app.focus = Focus::Content;
    set_transient_status(app, status_message_clear_at, message);
}

/// Data returned by [`finish_mutation_success`] so the caller can issue the
/// actual refresh and deferred-refresh calls (which depend on runtime handles).
pub struct MutationRefreshPlan {
    /// The computed refresh dispatch for this mutation.
    pub dispatch: RefreshDispatch,
    /// Resolved namespace scope (or `None` for all-namespaces).
    pub namespace: Option<String>,
    /// The view where the mutation originated.
    pub origin_view: AppView,
}

/// Computes the refresh plan for a successful mutation and sets the transient
/// status message. The caller is responsible for issuing `request_refresh`,
/// `queue_deferred_refreshes`, and resetting `auto_refresh`.
pub fn finish_mutation_success(
    app: &mut AppState,
    status_message_clear_at: &mut Option<Instant>,
    origin_view: AppView,
    message: impl Into<String>,
    force_include_flux: bool,
) -> MutationRefreshPlan {
    let namespace = namespace_scope(app.get_namespace()).map(str::to_string);
    let include_flux = force_include_flux || origin_view.is_fluxcd();
    let dispatch = mutation_refresh_options(origin_view, include_flux);
    set_transient_status(app, status_message_clear_at, message);
    MutationRefreshPlan {
        dispatch,
        namespace,
        origin_view,
    }
}

/// Builds a full refresh dispatch covering core, legacy-secondary, and helm.
pub fn full_refresh_options(include_flux: bool, include_cluster_info: bool) -> RefreshDispatch {
    let mut scope = RefreshScope::CORE_OVERVIEW
        .union(RefreshScope::LEGACY_SECONDARY)
        .union(RefreshScope::LOCAL_HELM_REPOSITORIES);
    if include_flux {
        scope = scope.union(RefreshScope::FLUX);
    }
    if include_cluster_info {
        scope = scope.union(RefreshScope::METRICS);
    }
    let mut dispatch = RefreshDispatch::new(scope.without(RefreshScope::LEGACY_SECONDARY), scope);
    dispatch.options.include_cluster_info = include_cluster_info;
    dispatch
}

/// Returns `true` when a palette detail action must first target a resource.
pub fn palette_detail_action_needs_detail(action: DetailAction) -> bool {
    matches!(
        action,
        DetailAction::ViewYaml
            | DetailAction::ViewConfigDrift
            | DetailAction::ViewRollout
            | DetailAction::ViewHelmHistory
            | DetailAction::ViewHelmValuesDiff
            | DetailAction::ViewDecodedSecret
            | DetailAction::ToggleBookmark
            | DetailAction::ViewEvents
            | DetailAction::ViewAccessReview
            | DetailAction::Logs
            | DetailAction::Exec
            | DetailAction::DebugContainer
            | DetailAction::NodeDebugShell
            | DetailAction::PortForward
            | DetailAction::Probes
            | DetailAction::Scale
            | DetailAction::Restart
            | DetailAction::PauseRollout
            | DetailAction::ResumeRollout
            | DetailAction::RollbackRollout
            | DetailAction::FluxReconcile
            | DetailAction::RollbackHelm
            | DetailAction::EditYaml
            | DetailAction::Delete
            | DetailAction::Trigger
            | DetailAction::SuspendCronJob
            | DetailAction::ResumeCronJob
            | DetailAction::ViewNetworkPolicies
            | DetailAction::CheckNetworkConnectivity
            | DetailAction::ViewTrafficDebug
            | DetailAction::ViewRelationships
            | DetailAction::Cordon
            | DetailAction::Uncordon
            | DetailAction::Drain
    )
}

/// Returns `true` when a detail action must first load the requested resource.
pub fn palette_detail_action_needs_resource_load(
    app: &AppState,
    action: DetailAction,
    resource: &ResourceRef,
) -> bool {
    palette_detail_action_needs_detail(action)
        && app
            .detail_view
            .as_ref()
            .and_then(|detail| detail.resource.as_ref())
            != Some(resource)
}

/// Converts a palette [`DetailAction`] into the corresponding [`AppAction`].
pub fn map_palette_detail_action(action: DetailAction) -> AppAction {
    match action {
        DetailAction::ViewYaml => AppAction::OpenResourceYaml,
        DetailAction::ViewConfigDrift => AppAction::OpenResourceDiff,
        DetailAction::ViewRollout => AppAction::OpenRollout,
        DetailAction::ViewHelmHistory => AppAction::OpenHelmHistory,
        DetailAction::ViewHelmValuesDiff => AppAction::OpenHelmValuesDiff,
        DetailAction::ViewDecodedSecret => AppAction::OpenDecodedSecret,
        DetailAction::ToggleBookmark => AppAction::ToggleBookmark,
        DetailAction::ViewEvents => AppAction::OpenResourceEvents,
        DetailAction::ViewAccessReview => AppAction::OpenAccessReview,
        DetailAction::Logs => AppAction::LogsViewerOpen,
        DetailAction::Exec => AppAction::OpenExec,
        DetailAction::DebugContainer => AppAction::DebugContainerDialogOpen,
        DetailAction::NodeDebugShell => AppAction::NodeDebugDialogOpen,
        DetailAction::PortForward => AppAction::PortForwardOpen,
        DetailAction::Probes => AppAction::ProbePanelOpen,
        DetailAction::Scale => AppAction::ScaleDialogOpen,
        DetailAction::Restart => AppAction::RolloutRestart,
        DetailAction::PauseRollout | DetailAction::ResumeRollout => {
            AppAction::ToggleRolloutPauseResume
        }
        DetailAction::RollbackRollout => AppAction::ConfirmRolloutUndo,
        DetailAction::FluxReconcile => AppAction::FluxReconcile,
        DetailAction::RollbackHelm => AppAction::ConfirmHelmRollback,
        DetailAction::EditYaml => AppAction::EditYaml,
        DetailAction::Delete => AppAction::ConfirmDeleteResource,
        DetailAction::Trigger => AppAction::TriggerCronJob,
        DetailAction::SuspendCronJob => AppAction::ConfirmCronJobSuspend(true),
        DetailAction::ResumeCronJob => AppAction::ConfirmCronJobSuspend(false),
        DetailAction::ViewNetworkPolicies => AppAction::OpenNetworkPolicyView,
        DetailAction::CheckNetworkConnectivity => AppAction::OpenNetworkConnectivity,
        DetailAction::ViewTrafficDebug => AppAction::OpenTrafficDebug,
        DetailAction::ViewRelationships => AppAction::OpenRelationships,
        DetailAction::Cordon => AppAction::CordonNode,
        DetailAction::Uncordon => AppAction::UncordonNode,
        DetailAction::Drain => AppAction::ConfirmDrainNode,
    }
}

/// Returns `true` when the mapped action requires the detail resource to be loaded.
pub fn palette_action_requires_loaded_detail(action: &AppAction) -> bool {
    matches!(
        action,
        AppAction::ScaleDialogOpen
            | AppAction::RolloutRestart
            | AppAction::ProbePanelOpen
            | AppAction::DebugContainerDialogOpen
            | AppAction::NodeDebugDialogOpen
            | AppAction::ConfirmDeleteResource
            | AppAction::DeleteResource
            | AppAction::EditYaml
            | AppAction::TriggerCronJob
            | AppAction::ConfirmCronJobSuspend(_)
            | AppAction::CordonNode
            | AppAction::UncordonNode
            | AppAction::ConfirmDrainNode
    )
}

/// Deferred palette action plus the detail resource it was prepared for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPaletteAction {
    pub action: AppAction,
    pub expected_detail_resource: Option<ResourceRef>,
}

impl PendingPaletteAction {
    pub fn new(action: AppAction, expected_detail_resource: Option<ResourceRef>) -> Self {
        Self {
            action,
            expected_detail_resource,
        }
    }
}

/// Returns `true` once a deferred palette action can be dispatched.
pub fn pending_palette_action_ready(app: &AppState, pending: &PendingPaletteAction) -> bool {
    if !palette_action_requires_loaded_detail(&pending.action) {
        return true;
    }

    app.detail_view.as_ref().is_some_and(|detail| {
        !detail.loading
            && detail.error.is_none()
            && pending
                .expected_detail_resource
                .as_ref()
                .is_some_and(|resource| detail.resource.as_ref() == Some(resource))
    })
}

/// Returns `true` when a pending palette action cannot safely target its resource anymore.
pub fn pending_palette_action_stale(app: &AppState, pending: &PendingPaletteAction) -> bool {
    if !palette_action_requires_loaded_detail(&pending.action) {
        return false;
    }

    let Some(detail) = app.detail_view.as_ref() else {
        return true;
    };

    if detail.error.is_some() {
        return true;
    }

    pending
        .expected_detail_resource
        .as_ref()
        .is_none_or(|resource| detail.resource.as_ref() != Some(resource))
}

/// Returns `true` when the view does not appear in the primary/fast refresh set
/// and should instead use its own targeted secondary refresh scope.
pub fn view_prefers_secondary_refresh(view: AppView) -> bool {
    !matches!(
        view,
        AppView::Dashboard
            | AppView::Nodes
            | AppView::Namespaces
            | AppView::Pods
            | AppView::Deployments
            | AppView::StatefulSets
            | AppView::DaemonSets
            | AppView::ReplicaSets
            | AppView::ReplicationControllers
            | AppView::Jobs
            | AppView::CronJobs
            | AppView::Services
            | AppView::Events
            | AppView::PortForwarding
            | AppView::HelmCharts
            | AppView::FluxCDAlertProviders
            | AppView::FluxCDAlerts
            | AppView::FluxCDAll
            | AppView::FluxCDArtifacts
            | AppView::FluxCDHelmReleases
            | AppView::FluxCDHelmRepositories
            | AppView::FluxCDImages
            | AppView::FluxCDKustomizations
            | AppView::FluxCDReceivers
            | AppView::FluxCDSources
    )
}

/// Returns `true` when the view needs cluster-info (metrics) data.
pub fn view_wants_cluster_info(view: AppView) -> bool {
    matches!(view, AppView::Dashboard)
}

/// Generates the [`RefreshDispatch`] tailored to a specific view.
pub fn refresh_options_for_view(
    view: AppView,
    include_flux: bool,
    force_cluster_info: bool,
) -> RefreshDispatch {
    let include_cluster_info = force_cluster_info || view_wants_cluster_info(view);
    let dispatch = match view {
        AppView::Dashboard => {
            let mut dispatch = RefreshDispatch::new(
                RefreshScope::DASHBOARD_WATCHED,
                RefreshScope::DASHBOARD_WATCHED.union(RefreshScope::METRICS),
            );
            dispatch.options.include_cluster_info = include_cluster_info;
            dispatch
        }
        AppView::Projects => RefreshDispatch::new(
            RefreshScope::CORE_OVERVIEW,
            RefreshScope::CORE_OVERVIEW
                .union(RefreshScope::LEGACY_SECONDARY)
                .union(RefreshScope::NETWORK)
                .union(RefreshScope::SECURITY),
        ),
        AppView::Governance => RefreshDispatch::new(
            RefreshScope::CORE_OVERVIEW,
            RefreshScope::CORE_OVERVIEW
                .union(RefreshScope::METRICS)
                .union(RefreshScope::LEGACY_SECONDARY)
                .union(RefreshScope::NETWORK)
                .union(RefreshScope::SECURITY),
        ),
        AppView::Pods => RefreshDispatch::new(
            RefreshScope::PODS,
            RefreshScope::PODS.union(RefreshScope::METRICS),
        ),
        AppView::Services => RefreshDispatch::new(RefreshScope::SERVICES, RefreshScope::SERVICES),
        AppView::Nodes => RefreshDispatch::new(
            RefreshScope::NODES,
            RefreshScope::NODES.union(RefreshScope::METRICS),
        ),
        AppView::Deployments => {
            RefreshDispatch::new(RefreshScope::DEPLOYMENTS, RefreshScope::DEPLOYMENTS)
        }
        AppView::StatefulSets => {
            RefreshDispatch::new(RefreshScope::STATEFULSETS, RefreshScope::STATEFULSETS)
        }
        AppView::DaemonSets => {
            RefreshDispatch::new(RefreshScope::DAEMONSETS, RefreshScope::DAEMONSETS)
        }
        AppView::ReplicaSets => {
            RefreshDispatch::new(RefreshScope::REPLICASETS, RefreshScope::REPLICASETS)
        }
        AppView::ReplicationControllers => RefreshDispatch::new(
            RefreshScope::REPLICATION_CONTROLLERS,
            RefreshScope::REPLICATION_CONTROLLERS,
        ),
        AppView::Jobs => RefreshDispatch::new(RefreshScope::JOBS, RefreshScope::JOBS),
        AppView::CronJobs => RefreshDispatch::new(RefreshScope::CRONJOBS, RefreshScope::CRONJOBS),
        AppView::Namespaces => {
            RefreshDispatch::new(RefreshScope::NAMESPACES, RefreshScope::NAMESPACES)
        }
        AppView::Bookmarks => full_refresh_options(include_flux, include_cluster_info),
        AppView::HelmCharts => RefreshDispatch::new(
            RefreshScope::LOCAL_HELM_REPOSITORIES,
            RefreshScope::LOCAL_HELM_REPOSITORIES,
        ),
        AppView::PortForwarding => RefreshDispatch::new(RefreshScope::NONE, RefreshScope::NONE),
        AppView::Issues | AppView::HealthReport => RefreshDispatch::new(
            RefreshScope::CORE_OVERVIEW,
            RefreshScope::CORE_OVERVIEW
                .union(RefreshScope::LEGACY_SECONDARY)
                .union(RefreshScope::SECURITY)
                .union(RefreshScope::FLUX),
        ),
        AppView::Vulnerabilities => {
            RefreshDispatch::new(RefreshScope::SECURITY, RefreshScope::SECURITY)
        }
        AppView::Events => RefreshDispatch::new(RefreshScope::NONE, RefreshScope::NONE),
        AppView::Endpoints
        | AppView::Ingresses
        | AppView::IngressClasses
        | AppView::GatewayClasses
        | AppView::Gateways
        | AppView::HttpRoutes
        | AppView::GrpcRoutes
        | AppView::ReferenceGrants
        | AppView::NetworkPolicies => {
            RefreshDispatch::new(RefreshScope::NETWORK, RefreshScope::NETWORK)
        }
        AppView::ConfigMaps
        | AppView::Secrets
        | AppView::ResourceQuotas
        | AppView::LimitRanges
        | AppView::HPAs
        | AppView::PodDisruptionBudgets
        | AppView::PriorityClasses => {
            RefreshDispatch::new(RefreshScope::CONFIG, RefreshScope::CONFIG)
        }
        AppView::PersistentVolumeClaims | AppView::PersistentVolumes | AppView::StorageClasses => {
            RefreshDispatch::new(RefreshScope::STORAGE, RefreshScope::STORAGE)
        }
        AppView::HelmReleases => RefreshDispatch::new(RefreshScope::HELM, RefreshScope::HELM),
        AppView::FluxCDAlertProviders
        | AppView::FluxCDAlerts
        | AppView::FluxCDAll
        | AppView::FluxCDArtifacts
        | AppView::FluxCDHelmReleases
        | AppView::FluxCDHelmRepositories
        | AppView::FluxCDImages
        | AppView::FluxCDKustomizations
        | AppView::FluxCDReceivers
        | AppView::FluxCDSources => RefreshDispatch::new(RefreshScope::FLUX, RefreshScope::FLUX),
        AppView::ServiceAccounts
        | AppView::ClusterRoles
        | AppView::Roles
        | AppView::ClusterRoleBindings
        | AppView::RoleBindings => {
            RefreshDispatch::new(RefreshScope::SECURITY, RefreshScope::SECURITY)
        }
        AppView::Extensions => {
            RefreshDispatch::new(RefreshScope::EXTENSIONS, RefreshScope::EXTENSIONS)
        }
    };
    dispatch.for_view(view)
}

/// Builds the refresh dispatch appropriate for a post-mutation refresh.
/// Views that prefer secondary refresh get their own targeted scope;
/// others fall back to the fast core-overview refresh.
pub fn mutation_refresh_options(view: AppView, include_flux: bool) -> RefreshDispatch {
    if view_prefers_secondary_refresh(view) {
        refresh_options_for_view(view, include_flux, false)
    } else {
        fast_refresh_options(include_flux)
    }
}

/// Enqueues deferred refresh triggers at the given delay intervals.
pub fn queue_deferred_refreshes(
    tx: &tokio::sync::mpsc::Sender<DeferredRefreshTrigger>,
    context_generation: u64,
    view: AppView,
    namespace: Option<String>,
    dispatch: RefreshDispatch,
    delays_secs: &[u64],
) {
    for &delay_secs in delays_secs {
        let tx = tx.clone();
        let trigger = DeferredRefreshTrigger {
            context_generation,
            view,
            dispatch,
            namespace: namespace.clone(),
        };
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(delay_secs)).await;
            let _ = tx.send(trigger).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PendingPaletteAction, map_palette_detail_action, palette_action_requires_loaded_detail,
        palette_detail_action_needs_resource_load, pending_palette_action_ready,
        pending_palette_action_stale,
    };
    use kubectui::{
        app::{AppAction, AppState, DetailViewState, ResourceRef},
        policy::DetailAction,
    };

    #[test]
    fn palette_detail_action_loads_when_no_detail_is_open() {
        let app = AppState::default();
        let target = ResourceRef::Pod("api-0".to_string(), "default".to_string());

        assert!(palette_detail_action_needs_resource_load(
            &app,
            DetailAction::Delete,
            &target
        ));
    }

    #[test]
    fn palette_detail_action_loads_when_open_detail_is_different_resource() {
        let target = ResourceRef::Pod("api-1".to_string(), "default".to_string());
        let mut app = AppState {
            detail_view: Some(DetailViewState {
                resource: Some(ResourceRef::Pod("api-0".to_string(), "default".to_string())),
                ..DetailViewState::default()
            }),
            ..AppState::default()
        };

        assert!(palette_detail_action_needs_resource_load(
            &app,
            DetailAction::Delete,
            &target
        ));

        app.detail_view = Some(DetailViewState {
            resource: Some(target.clone()),
            ..DetailViewState::default()
        });

        assert!(!palette_detail_action_needs_resource_load(
            &app,
            DetailAction::Delete,
            &target
        ));
    }

    #[test]
    fn palette_all_resource_actions_load_target_when_detail_missing() {
        let app = AppState::default();
        let target = ResourceRef::Pod("api-0".to_string(), "default".to_string());

        for action in DetailAction::ALL {
            assert!(
                palette_detail_action_needs_resource_load(&app, *action, &target),
                "{action:?} should point detail state at the palette target"
            );
        }
    }

    #[test]
    fn palette_resource_only_actions_do_not_wait_for_loaded_detail_payload() {
        for action in [
            DetailAction::ViewYaml,
            DetailAction::ViewConfigDrift,
            DetailAction::ToggleBookmark,
            DetailAction::Logs,
            DetailAction::Exec,
            DetailAction::PortForward,
            DetailAction::ViewRelationships,
        ] {
            let mapped = map_palette_detail_action(action);
            assert!(
                !palette_action_requires_loaded_detail(&mapped),
                "{action:?} only needs the target resource identity"
            );
        }
    }

    #[test]
    fn pending_palette_action_waits_for_expected_detail_resource() {
        let target = ResourceRef::Pod("api-0".to_string(), "default".to_string());
        let other = ResourceRef::Pod("api-1".to_string(), "default".to_string());
        let pending =
            PendingPaletteAction::new(AppAction::ConfirmDeleteResource, Some(target.clone()));

        let loading_target = AppState {
            detail_view: Some(DetailViewState {
                resource: Some(target.clone()),
                loading: true,
                ..DetailViewState::default()
            }),
            ..AppState::default()
        };
        assert!(!pending_palette_action_ready(&loading_target, &pending));
        assert!(!pending_palette_action_stale(&loading_target, &pending));

        let loaded_other = AppState {
            detail_view: Some(DetailViewState {
                resource: Some(other),
                loading: false,
                ..DetailViewState::default()
            }),
            ..AppState::default()
        };
        assert!(!pending_palette_action_ready(&loaded_other, &pending));
        assert!(pending_palette_action_stale(&loaded_other, &pending));

        let loaded_target = AppState {
            detail_view: Some(DetailViewState {
                resource: Some(target),
                loading: false,
                ..DetailViewState::default()
            }),
            ..AppState::default()
        };
        assert!(pending_palette_action_ready(&loaded_target, &pending));
        assert!(!pending_palette_action_stale(&loaded_target, &pending));
    }

    #[test]
    fn pending_palette_action_without_detail_requirement_dispatches_immediately() {
        let pending = PendingPaletteAction::new(AppAction::ApplyWorkspace("ops".into()), None);

        assert!(pending_palette_action_ready(&AppState::default(), &pending));
        assert!(!pending_palette_action_stale(
            &AppState::default(),
            &pending
        ));
    }
}
