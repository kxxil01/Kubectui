//! Pure helper functions extracted from the main event loop.
//!
//! These cover mutation status updates, refresh-scope construction,
//! palette-action mapping, and view-specific refresh preferences.

use std::time::{Duration, Instant};

use kubectui::{
    app::{AppAction, AppState, AppView, Focus},
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

/// Returns `true` when the palette detail action requires a loaded detail resource.
pub fn palette_detail_action_needs_detail(action: DetailAction) -> bool {
    matches!(
        action,
        DetailAction::Scale
            | DetailAction::Restart
            | DetailAction::Probes
            | DetailAction::DebugContainer
            | DetailAction::Delete
            | DetailAction::EditYaml
            | DetailAction::Trigger
            | DetailAction::SuspendCronJob
            | DetailAction::ResumeCronJob
            | DetailAction::Cordon
            | DetailAction::Uncordon
            | DetailAction::Drain
    )
}

/// Converts a palette [`DetailAction`] into the corresponding [`AppAction`].
pub fn map_palette_detail_action(action: DetailAction) -> AppAction {
    match action {
        DetailAction::ViewYaml => AppAction::OpenResourceYaml,
        DetailAction::ViewConfigDrift => AppAction::OpenResourceDiff,
        DetailAction::ViewRollout => AppAction::OpenRollout,
        DetailAction::ViewHelmHistory => AppAction::OpenHelmHistory,
        DetailAction::ViewDecodedSecret => AppAction::OpenDecodedSecret,
        DetailAction::ToggleBookmark => AppAction::ToggleBookmark,
        DetailAction::ViewEvents => AppAction::OpenResourceEvents,
        DetailAction::Logs => AppAction::LogsViewerOpen,
        DetailAction::Exec => AppAction::OpenExec,
        DetailAction::DebugContainer => AppAction::DebugContainerDialogOpen,
        DetailAction::PortForward => AppAction::PortForwardOpen,
        DetailAction::Probes => AppAction::ProbePanelOpen,
        DetailAction::Scale => AppAction::ScaleDialogOpen,
        DetailAction::Restart => AppAction::RolloutRestart,
        DetailAction::FluxReconcile => AppAction::FluxReconcile,
        DetailAction::EditYaml => AppAction::EditYaml,
        DetailAction::Delete => AppAction::DeleteResource,
        DetailAction::Trigger => AppAction::TriggerCronJob,
        DetailAction::SuspendCronJob => AppAction::ConfirmCronJobSuspend(true),
        DetailAction::ResumeCronJob => AppAction::ConfirmCronJobSuspend(false),
        DetailAction::ViewNetworkPolicies => AppAction::OpenNetworkPolicyView,
        DetailAction::CheckNetworkConnectivity => AppAction::OpenNetworkConnectivity,
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
            | AppAction::DeleteResource
            | AppAction::EditYaml
            | AppAction::TriggerCronJob
            | AppAction::ConfirmCronJobSuspend(_)
            | AppAction::CordonNode
            | AppAction::UncordonNode
            | AppAction::ConfirmDrainNode
    )
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
    match view {
        AppView::Dashboard => {
            let mut dispatch = RefreshDispatch::new(
                RefreshScope::CORE_OVERVIEW,
                RefreshScope::CORE_OVERVIEW.union(RefreshScope::METRICS),
            );
            dispatch.options.include_cluster_info = include_cluster_info;
            dispatch
        }
        AppView::Pods => RefreshDispatch::new(
            RefreshScope::PODS,
            RefreshScope::PODS.union(RefreshScope::METRICS),
        ),
        AppView::Services => RefreshDispatch::new(
            RefreshScope::SERVICES,
            RefreshScope::SERVICES.union(RefreshScope::NETWORK),
        ),
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
    }
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
