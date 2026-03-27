//! Canonical cross-view and detail action policies.

use crate::k8s::flux::flux_reconcile_support;
use crate::{
    app::{AppView, DetailViewState, ResourceRef, WorkloadSortColumn},
    authorization::{ActionAuthorizationMap, detail_action_requires_authorization},
};

/// Shared list-level actions that are view-dependent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewAction {
    SortName,
    SortAge,
    ClearSort,
    PodSortStatus,
    PodSortRestarts,
    SelectedFluxReconcile,
}

/// Future-facing persistence capabilities per view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewPersistenceCapability {
    Sort,
    ColumnLayout,
}

/// Relationship categories a view/resource family can support in future milestones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelationshipCapability {
    OwnerChain,
    ServiceBackends,
    IngressBackends,
    GatewayRoutes,
    StorageBindings,
    FluxLineage,
    RbacBindings,
}

/// Actions that can appear in the detail footer or be triggered from detail context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DetailAction {
    ViewYaml,
    ViewConfigDrift,
    ViewRollout,
    ViewHelmHistory,
    ViewDecodedSecret,
    ToggleBookmark,
    ViewEvents,
    Logs,
    Exec,
    DebugContainer,
    NodeDebugShell,
    PortForward,
    Probes,
    Scale,
    Restart,
    FluxReconcile,
    EditYaml,
    Delete,
    Trigger,
    SuspendCronJob,
    ResumeCronJob,
    ViewNetworkPolicies,
    CheckNetworkConnectivity,
    ViewTrafficDebug,
    ViewRelationships,
    Cordon,
    Uncordon,
    Drain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceActionContext {
    pub resource: ResourceRef,
    pub node_unschedulable: Option<bool>,
    pub cronjob_suspended: Option<bool>,
    pub cronjob_history_logs_available: bool,
    pub action_authorizations: ActionAuthorizationMap,
}

impl DetailAction {
    pub const ORDER: [DetailAction; 28] = [
        DetailAction::ViewYaml,
        DetailAction::ViewConfigDrift,
        DetailAction::ViewRollout,
        DetailAction::ViewHelmHistory,
        DetailAction::ViewDecodedSecret,
        DetailAction::ToggleBookmark,
        DetailAction::ViewEvents,
        DetailAction::Logs,
        DetailAction::Exec,
        DetailAction::DebugContainer,
        DetailAction::NodeDebugShell,
        DetailAction::PortForward,
        DetailAction::Probes,
        DetailAction::Scale,
        DetailAction::Restart,
        DetailAction::FluxReconcile,
        DetailAction::EditYaml,
        DetailAction::Delete,
        DetailAction::Trigger,
        DetailAction::SuspendCronJob,
        DetailAction::ResumeCronJob,
        DetailAction::ViewNetworkPolicies,
        DetailAction::CheckNetworkConnectivity,
        DetailAction::ViewTrafficDebug,
        DetailAction::ViewRelationships,
        DetailAction::Cordon,
        DetailAction::Uncordon,
        DetailAction::Drain,
    ];

    pub const fn key_hint(self) -> &'static str {
        match self {
            DetailAction::ViewYaml => "[y]",
            DetailAction::ViewConfigDrift => "[D]",
            DetailAction::ViewRollout => "[O]",
            DetailAction::ViewHelmHistory => "[h]",
            DetailAction::ViewDecodedSecret => "[o]",
            DetailAction::ToggleBookmark => "[B]",
            DetailAction::ViewEvents => "[v]",
            DetailAction::Logs => "[l]",
            DetailAction::Exec => "[x]",
            DetailAction::DebugContainer => "[g]",
            DetailAction::NodeDebugShell => "[g]",
            DetailAction::PortForward => "[f]",
            DetailAction::Probes => "[p]",
            DetailAction::Scale => "[s]",
            DetailAction::Restart | DetailAction::FluxReconcile => "[R]",
            DetailAction::EditYaml => "[e]",
            DetailAction::Delete => "[d]",
            DetailAction::Trigger => "[T]",
            DetailAction::SuspendCronJob | DetailAction::ResumeCronJob => "[S]",
            DetailAction::ViewNetworkPolicies => "[N]",
            DetailAction::CheckNetworkConnectivity => "[C]",
            DetailAction::ViewTrafficDebug => "[t]",
            DetailAction::ViewRelationships => "[w]",
            DetailAction::Cordon => "[c]",
            DetailAction::Uncordon => "[u]",
            DetailAction::Drain => "[D]",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            DetailAction::ViewYaml => "YAML",
            DetailAction::ViewConfigDrift => "Drift",
            DetailAction::ViewRollout => "Rollout",
            DetailAction::ViewHelmHistory => "Helm",
            DetailAction::ViewDecodedSecret => "Decoded",
            DetailAction::ToggleBookmark => "Bookmark",
            DetailAction::ViewEvents => "Events",
            DetailAction::Logs => "Logs",
            DetailAction::Exec => "Exec",
            DetailAction::DebugContainer => "Debug",
            DetailAction::NodeDebugShell => "NodeDbg",
            DetailAction::PortForward => "Port-Fwd",
            DetailAction::Probes => "Probes",
            DetailAction::Scale => "Scale",
            DetailAction::Restart => "Restart",
            DetailAction::FluxReconcile => "Reconcile",
            DetailAction::EditYaml => "Edit",
            DetailAction::Delete => "Delete",
            DetailAction::Trigger => "Trigger",
            DetailAction::SuspendCronJob => "Suspend",
            DetailAction::ResumeCronJob => "Resume",
            DetailAction::ViewNetworkPolicies => "NetPol",
            DetailAction::CheckNetworkConnectivity => "Reach",
            DetailAction::ViewTrafficDebug => "Traffic",
            DetailAction::ViewRelationships => "Relations",
            DetailAction::Cordon => "Cordon",
            DetailAction::Uncordon => "Uncordon",
            DetailAction::Drain => "Drain",
        }
    }
}

const SHARED_SORT_NONE: &[WorkloadSortColumn] = &[];
const SHARED_SORT_NAME_ONLY: &[WorkloadSortColumn] = &[WorkloadSortColumn::Name];
const SHARED_SORT_NAME_AGE: &[WorkloadSortColumn] =
    &[WorkloadSortColumn::Name, WorkloadSortColumn::Age];

const VIEW_ACTION_NONE: &[ViewAction] = &[];
const VIEW_ACTION_PODS: &[ViewAction] = &[
    ViewAction::SortName,
    ViewAction::SortAge,
    ViewAction::PodSortStatus,
    ViewAction::PodSortRestarts,
    ViewAction::ClearSort,
];
const VIEW_ACTION_NAME_ONLY: &[ViewAction] = &[ViewAction::SortName, ViewAction::ClearSort];
const VIEW_ACTION_NAME_AGE: &[ViewAction] = &[
    ViewAction::SortName,
    ViewAction::SortAge,
    ViewAction::ClearSort,
];
const VIEW_ACTION_NAME_AGE_WITH_RECONCILE: &[ViewAction] = &[
    ViewAction::SortName,
    ViewAction::SortAge,
    ViewAction::ClearSort,
    ViewAction::SelectedFluxReconcile,
];

const PERSISTENCE_NONE: &[ViewPersistenceCapability] = &[];
const PERSISTENCE_SORT_AND_COLUMNS: &[ViewPersistenceCapability] = &[
    ViewPersistenceCapability::Sort,
    ViewPersistenceCapability::ColumnLayout,
];

const RELATIONSHIPS_NONE: &[RelationshipCapability] = &[];
const RELATIONSHIPS_OWNER_CHAIN: &[RelationshipCapability] = &[RelationshipCapability::OwnerChain];
const RELATIONSHIPS_SERVICE_BACKENDS: &[RelationshipCapability] =
    &[RelationshipCapability::ServiceBackends];
const RELATIONSHIPS_INGRESS_BACKENDS: &[RelationshipCapability] =
    &[RelationshipCapability::IngressBackends];
const RELATIONSHIPS_STORAGE: &[RelationshipCapability] = &[RelationshipCapability::StorageBindings];
const RELATIONSHIPS_FLUX: &[RelationshipCapability] = &[RelationshipCapability::FluxLineage];
const RELATIONSHIPS_RBAC: &[RelationshipCapability] = &[RelationshipCapability::RbacBindings];

impl AppView {
    pub const fn shared_sort_capabilities(self) -> &'static [WorkloadSortColumn] {
        match self {
            AppView::Nodes
            | AppView::Services
            | AppView::Deployments
            | AppView::StatefulSets
            | AppView::DaemonSets
            | AppView::ReplicaSets
            | AppView::ReplicationControllers
            | AppView::Jobs
            | AppView::CronJobs
            | AppView::ResourceQuotas
            | AppView::LimitRanges
            | AppView::PodDisruptionBudgets
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
            | AppView::ServiceAccounts
            | AppView::ClusterRoles
            | AppView::Roles
            | AppView::ClusterRoleBindings
            | AppView::RoleBindings => SHARED_SORT_NAME_AGE,
            AppView::PersistentVolumeClaims
            | AppView::PersistentVolumes
            | AppView::StorageClasses => SHARED_SORT_NAME_ONLY,
            _ => SHARED_SORT_NONE,
        }
    }

    pub fn supports_shared_sort(self, column: WorkloadSortColumn) -> bool {
        self.shared_sort_capabilities().contains(&column)
    }

    pub const fn action_capabilities(self) -> &'static [ViewAction] {
        match self {
            AppView::Pods => VIEW_ACTION_PODS,
            AppView::Nodes
            | AppView::Services
            | AppView::Deployments
            | AppView::StatefulSets
            | AppView::DaemonSets
            | AppView::ReplicaSets
            | AppView::ReplicationControllers
            | AppView::Jobs
            | AppView::CronJobs
            | AppView::ResourceQuotas
            | AppView::LimitRanges
            | AppView::PodDisruptionBudgets
            | AppView::ServiceAccounts
            | AppView::ClusterRoles
            | AppView::Roles
            | AppView::ClusterRoleBindings
            | AppView::RoleBindings => VIEW_ACTION_NAME_AGE,
            AppView::PersistentVolumeClaims
            | AppView::PersistentVolumes
            | AppView::StorageClasses => VIEW_ACTION_NAME_ONLY,
            AppView::FluxCDAlertProviders | AppView::FluxCDAlerts => VIEW_ACTION_NAME_AGE,
            AppView::FluxCDAll
            | AppView::FluxCDArtifacts
            | AppView::FluxCDHelmReleases
            | AppView::FluxCDHelmRepositories
            | AppView::FluxCDImages
            | AppView::FluxCDKustomizations
            | AppView::FluxCDReceivers
            | AppView::FluxCDSources => VIEW_ACTION_NAME_AGE_WITH_RECONCILE,
            _ => VIEW_ACTION_NONE,
        }
    }

    pub fn supports_view_action(self, action: ViewAction) -> bool {
        self.action_capabilities().contains(&action)
    }

    pub const fn persistence_capabilities(self) -> &'static [ViewPersistenceCapability] {
        match self {
            AppView::Dashboard
            | AppView::Projects
            | AppView::Governance
            | AppView::Bookmarks
            | AppView::PortForwarding
            | AppView::HelmCharts
            | AppView::Extensions
            | AppView::Issues
            | AppView::HealthReport => PERSISTENCE_NONE,
            AppView::Pods
            | AppView::Vulnerabilities
            | AppView::Nodes
            | AppView::Services
            | AppView::Deployments
            | AppView::StatefulSets
            | AppView::DaemonSets
            | AppView::ReplicaSets
            | AppView::ReplicationControllers
            | AppView::Jobs
            | AppView::CronJobs
            | AppView::Endpoints
            | AppView::Ingresses
            | AppView::IngressClasses
            | AppView::GatewayClasses
            | AppView::Gateways
            | AppView::HttpRoutes
            | AppView::GrpcRoutes
            | AppView::ReferenceGrants
            | AppView::NetworkPolicies
            | AppView::ConfigMaps
            | AppView::Secrets
            | AppView::ResourceQuotas
            | AppView::LimitRanges
            | AppView::HPAs
            | AppView::PodDisruptionBudgets
            | AppView::PriorityClasses
            | AppView::PersistentVolumeClaims
            | AppView::PersistentVolumes
            | AppView::StorageClasses
            | AppView::Namespaces
            | AppView::Events
            | AppView::HelmReleases
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
            | AppView::ServiceAccounts
            | AppView::ClusterRoles
            | AppView::Roles
            | AppView::ClusterRoleBindings
            | AppView::RoleBindings => PERSISTENCE_SORT_AND_COLUMNS,
        }
    }

    pub const fn relationship_capabilities(self) -> &'static [RelationshipCapability] {
        match self {
            AppView::Deployments
            | AppView::StatefulSets
            | AppView::DaemonSets
            | AppView::ReplicaSets
            | AppView::ReplicationControllers
            | AppView::Jobs
            | AppView::CronJobs
            | AppView::Pods => RELATIONSHIPS_OWNER_CHAIN,
            AppView::Services | AppView::Endpoints => RELATIONSHIPS_SERVICE_BACKENDS,
            AppView::Ingresses | AppView::IngressClasses => RELATIONSHIPS_INGRESS_BACKENDS,
            AppView::GatewayClasses
            | AppView::Gateways
            | AppView::HttpRoutes
            | AppView::GrpcRoutes
            | AppView::ReferenceGrants => &[RelationshipCapability::GatewayRoutes],
            AppView::PersistentVolumeClaims
            | AppView::PersistentVolumes
            | AppView::StorageClasses => RELATIONSHIPS_STORAGE,
            AppView::FluxCDAlertProviders
            | AppView::FluxCDAlerts
            | AppView::FluxCDAll
            | AppView::FluxCDArtifacts
            | AppView::FluxCDHelmReleases
            | AppView::FluxCDHelmRepositories
            | AppView::FluxCDImages
            | AppView::FluxCDKustomizations
            | AppView::FluxCDReceivers
            | AppView::FluxCDSources => RELATIONSHIPS_FLUX,
            AppView::ServiceAccounts
            | AppView::ClusterRoles
            | AppView::Roles
            | AppView::ClusterRoleBindings
            | AppView::RoleBindings => RELATIONSHIPS_RBAC,
            _ => RELATIONSHIPS_NONE,
        }
    }
}

impl ResourceRef {
    pub fn supports_events_tab(&self) -> bool {
        matches!(
            self,
            ResourceRef::Pod(_, _)
                | ResourceRef::Deployment(_, _)
                | ResourceRef::StatefulSet(_, _)
                | ResourceRef::DaemonSet(_, _)
                | ResourceRef::ReplicaSet(_, _)
                | ResourceRef::Job(_, _)
                | ResourceRef::CronJob(_, _)
                | ResourceRef::Service(_, _)
                | ResourceRef::Ingress(_, _)
                | ResourceRef::ConfigMap(_, _)
                | ResourceRef::Pvc(_, _)
                | ResourceRef::HelmRelease(_, _)
        )
    }

    /// Returns true when this resource is a Flux custom resource that supports
    /// the direct reconcile action.
    pub fn supports_flux_reconcile(&self) -> bool {
        matches!(
            self,
            ResourceRef::CustomResource { group, kind, .. }
                if flux_reconcile_support(group, kind).is_supported()
        )
    }

    /// Returns the disabled reason for Flux reconcile when not supported.
    pub fn flux_reconcile_disabled_reason(&self) -> Option<&'static str> {
        match self {
            ResourceRef::CustomResource { group, kind, .. } => {
                flux_reconcile_support(group, kind).unsupported_reason()
            }
            _ => Some("Flux reconcile is only available for Flux toolkit resources."),
        }
    }

    pub fn supports_detail_action(
        &self,
        action: DetailAction,
        node_unschedulable: Option<bool>,
        cronjob_suspended: Option<bool>,
    ) -> bool {
        match action {
            DetailAction::ViewYaml => true,
            DetailAction::ViewConfigDrift => !matches!(self, ResourceRef::Node(_)),
            DetailAction::ViewRollout => matches!(
                self,
                ResourceRef::Deployment(_, _)
                    | ResourceRef::StatefulSet(_, _)
                    | ResourceRef::DaemonSet(_, _)
            ),
            DetailAction::ViewHelmHistory => matches!(self, ResourceRef::HelmRelease(_, _)),
            DetailAction::ViewEvents => self.supports_events_tab(),
            DetailAction::ViewDecodedSecret => matches!(self, ResourceRef::Secret(_, _)),
            DetailAction::ToggleBookmark => true,
            DetailAction::Logs => matches!(
                self,
                ResourceRef::Pod(_, _)
                    | ResourceRef::Deployment(_, _)
                    | ResourceRef::StatefulSet(_, _)
                    | ResourceRef::DaemonSet(_, _)
                    | ResourceRef::ReplicaSet(_, _)
                    | ResourceRef::ReplicationController(_, _)
                    | ResourceRef::Job(_, _)
            ),
            DetailAction::Exec
            | DetailAction::DebugContainer
            | DetailAction::PortForward
            | DetailAction::Probes => {
                matches!(self, ResourceRef::Pod(_, _))
            }
            DetailAction::Scale => {
                matches!(
                    self,
                    ResourceRef::Deployment(_, _) | ResourceRef::StatefulSet(_, _)
                )
            }
            DetailAction::Restart => matches!(
                self,
                ResourceRef::Deployment(_, _)
                    | ResourceRef::StatefulSet(_, _)
                    | ResourceRef::DaemonSet(_, _)
            ),
            DetailAction::FluxReconcile => self.supports_flux_reconcile(),
            DetailAction::EditYaml | DetailAction::Delete => {
                !matches!(self, ResourceRef::HelmRelease(_, _))
            }
            DetailAction::Trigger => matches!(self, ResourceRef::CronJob(_, _)),
            DetailAction::SuspendCronJob => {
                matches!(self, ResourceRef::CronJob(_, _)) && !cronjob_suspended.unwrap_or(false)
            }
            DetailAction::ResumeCronJob => {
                matches!(self, ResourceRef::CronJob(_, _)) && cronjob_suspended.unwrap_or(false)
            }
            DetailAction::ViewNetworkPolicies => matches!(
                self,
                ResourceRef::Pod(_, _)
                    | ResourceRef::Namespace(_)
                    | ResourceRef::NetworkPolicy(_, _)
            ),
            DetailAction::CheckNetworkConnectivity => matches!(self, ResourceRef::Pod(_, _)),
            DetailAction::ViewTrafficDebug => {
                matches!(
                    self,
                    ResourceRef::Pod(_, _)
                        | ResourceRef::Service(_, _)
                        | ResourceRef::Endpoint(_, _)
                        | ResourceRef::Ingress(_, _)
                ) || matches!(
                    self,
                    ResourceRef::CustomResource { group, kind, .. }
                        if group == "gateway.networking.k8s.io"
                            && matches!(kind.as_str(), "Gateway" | "HTTPRoute" | "GRPCRoute")
                )
            }
            DetailAction::NodeDebugShell => matches!(self, ResourceRef::Node(_)),
            DetailAction::ViewRelationships => {
                crate::k8s::relationships::resource_has_relationships(self)
            }
            DetailAction::Cordon => {
                matches!(self, ResourceRef::Node(_)) && !node_unschedulable.unwrap_or(false)
            }
            DetailAction::Uncordon => {
                matches!(self, ResourceRef::Node(_)) && node_unschedulable.unwrap_or(false)
            }
            DetailAction::Drain => matches!(self, ResourceRef::Node(_)),
        }
    }
}

fn supports_action_borrowed(
    resource: &ResourceRef,
    node_unschedulable: Option<bool>,
    cronjob_suspended: Option<bool>,
    cronjob_history_logs_available: bool,
    action_authorizations: &ActionAuthorizationMap,
    action: DetailAction,
) -> bool {
    let supported =
        if matches!(action, DetailAction::Logs) && matches!(resource, ResourceRef::CronJob(_, _)) {
            cronjob_history_logs_available
        } else {
            resource.supports_detail_action(action, node_unschedulable, cronjob_suspended)
        };

    if !supported {
        return false;
    }

    if matches!(action, DetailAction::Logs) && matches!(resource, ResourceRef::CronJob(_, _)) {
        return true;
    }

    if detail_action_requires_authorization(action) {
        return action_authorizations
            .get(&action)
            .is_none_or(|status| status.permits(action));
    }

    true
}

impl ResourceActionContext {
    pub fn supports_action(&self, action: DetailAction) -> bool {
        supports_action_borrowed(
            &self.resource,
            self.node_unschedulable,
            self.cronjob_suspended,
            self.cronjob_history_logs_available,
            &self.action_authorizations,
            action,
        )
    }
}

impl DetailViewState {
    pub fn resource_action_context(&self) -> Option<ResourceActionContext> {
        self.resource.clone().map(|resource| ResourceActionContext {
            resource,
            node_unschedulable: self.metadata.node_unschedulable,
            cronjob_suspended: self.metadata.cronjob_suspended,
            cronjob_history_logs_available: self
                .selected_cronjob_history()
                .is_some_and(|entry| entry.has_log_target()),
            action_authorizations: self.metadata.action_authorizations.clone(),
        })
    }

    pub fn has_blocking_detail_overlay(&self) -> bool {
        self.scale_dialog.is_some()
            || self.debug_dialog.is_some()
            || self.node_debug_dialog.is_some()
            || self.probe_panel.is_some()
            || self.confirm_delete
            || self.confirm_drain
            || self.confirm_cronjob_suspend.is_some()
    }

    pub fn supports_action(&self, action: DetailAction) -> bool {
        let Some(resource) = self.resource.as_ref() else {
            return false;
        };

        let requires_clear_surface = matches!(
            action,
            DetailAction::ViewYaml
                | DetailAction::ViewDecodedSecret
                | DetailAction::ViewHelmHistory
                | DetailAction::ToggleBookmark
                | DetailAction::ViewEvents
                | DetailAction::Logs
                | DetailAction::Exec
                | DetailAction::DebugContainer
                | DetailAction::NodeDebugShell
                | DetailAction::PortForward
                | DetailAction::Probes
                | DetailAction::Scale
                | DetailAction::Restart
                | DetailAction::FluxReconcile
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
        );

        if self.loading {
            return false;
        }
        if requires_clear_surface && self.has_blocking_detail_overlay() {
            return false;
        }
        if action == DetailAction::EditYaml && self.yaml.is_none() {
            return false;
        }
        if matches!(action, DetailAction::Logs) && matches!(resource, ResourceRef::CronJob(_, _)) {
            return self
                .selected_cronjob_history()
                .is_some_and(|entry| entry.has_log_target())
                && !self.has_blocking_detail_overlay();
        }

        supports_action_borrowed(
            resource,
            self.metadata.node_unschedulable,
            self.metadata.cronjob_suspended,
            self.selected_cronjob_history()
                .is_some_and(|entry| entry.has_log_target()),
            &self.metadata.action_authorizations,
            action,
        )
    }

    pub fn footer_actions(&self) -> Vec<DetailAction> {
        DetailAction::ORDER
            .into_iter()
            .filter(|action| self.supports_action(*action))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::AppView, authorization::DetailActionAuthorization};

    #[test]
    fn flux_views_only_offer_list_reconcile_where_supported() {
        assert!(
            AppView::FluxCDKustomizations.supports_view_action(ViewAction::SelectedFluxReconcile)
        );
        assert!(AppView::FluxCDAll.supports_view_action(ViewAction::SelectedFluxReconcile));
        assert!(!AppView::FluxCDAlerts.supports_view_action(ViewAction::SelectedFluxReconcile));
        assert!(
            !AppView::FluxCDAlertProviders.supports_view_action(ViewAction::SelectedFluxReconcile)
        );
    }

    #[test]
    fn pod_detail_actions_match_operator_expectations() {
        let detail = DetailViewState {
            resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
            yaml: Some("kind: Pod".to_string()),
            ..DetailViewState::default()
        };

        assert!(detail.supports_action(DetailAction::Logs));
        assert!(detail.supports_action(DetailAction::DebugContainer));
        assert!(detail.supports_action(DetailAction::PortForward));
        assert!(detail.supports_action(DetailAction::Probes));
        assert!(detail.supports_action(DetailAction::CheckNetworkConnectivity));
        assert!(detail.supports_action(DetailAction::EditYaml));
        assert!(detail.supports_action(DetailAction::Delete));
        assert!(!detail.supports_action(DetailAction::Scale));
        assert!(!detail.supports_action(DetailAction::Restart));
        assert!(!detail.supports_action(DetailAction::FluxReconcile));
    }

    #[test]
    fn deployment_detail_actions_match_operator_expectations() {
        let detail = DetailViewState {
            resource: Some(ResourceRef::Deployment(
                "api".to_string(),
                "default".to_string(),
            )),
            yaml: Some("kind: Deployment".to_string()),
            ..DetailViewState::default()
        };

        assert!(detail.supports_action(DetailAction::Scale));
        assert!(detail.supports_action(DetailAction::Restart));
        assert!(detail.supports_action(DetailAction::ViewRollout));
        assert!(detail.supports_action(DetailAction::EditYaml));
        assert!(detail.supports_action(DetailAction::Delete));
        assert!(detail.supports_action(DetailAction::Logs));
        assert!(!detail.supports_action(DetailAction::FluxReconcile));
    }

    #[test]
    fn node_detail_actions_match_operator_expectations() {
        let detail = DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            yaml: Some("kind: Node".to_string()),
            ..DetailViewState::default()
        };

        assert!(detail.supports_action(DetailAction::NodeDebugShell));
        assert!(detail.supports_action(DetailAction::Cordon));
        assert!(detail.supports_action(DetailAction::Drain));
        assert!(!detail.supports_action(DetailAction::DebugContainer));
        assert!(!detail.supports_action(DetailAction::PortForward));
    }

    #[test]
    fn rollout_action_is_limited_to_workload_controllers() {
        let deployment = ResourceRef::Deployment("api".to_string(), "ns".to_string());
        let statefulset = ResourceRef::StatefulSet("db".to_string(), "ns".to_string());
        let daemonset = ResourceRef::DaemonSet("agent".to_string(), "ns".to_string());
        let pod = ResourceRef::Pod("api-0".to_string(), "ns".to_string());

        assert!(deployment.supports_detail_action(DetailAction::ViewRollout, None, None));
        assert!(statefulset.supports_detail_action(DetailAction::ViewRollout, None, None));
        assert!(daemonset.supports_detail_action(DetailAction::ViewRollout, None, None));
        assert!(!pod.supports_detail_action(DetailAction::ViewRollout, None, None));
    }

    #[test]
    fn detail_overlay_blocks_conflicting_actions() {
        let detail = DetailViewState {
            resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
            yaml: Some("kind: Pod".to_string()),
            scale_dialog: Some(crate::ui::components::scale_dialog::ScaleDialogState::new(
                crate::ui::components::scale_dialog::ScaleTargetKind::Deployment,
                "pod-0".to_string(),
                "ns".to_string(),
                1,
            )),
            ..DetailViewState::default()
        };

        assert!(!detail.supports_action(DetailAction::Logs));
        assert!(!detail.supports_action(DetailAction::EditYaml));
        assert!(!detail.supports_action(DetailAction::Delete));
        assert!(!detail.supports_action(DetailAction::PortForward));
    }

    #[test]
    fn denied_authorization_hides_permission_gated_actions() {
        let mut detail = DetailViewState {
            resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
            yaml: Some("kind: Pod".to_string()),
            ..DetailViewState::default()
        };
        detail
            .metadata
            .action_authorizations
            .insert(DetailAction::Exec, DetailActionAuthorization::Denied);
        detail
            .metadata
            .action_authorizations
            .insert(DetailAction::Logs, DetailActionAuthorization::Allowed);

        assert!(detail.supports_action(DetailAction::Logs));
        assert!(!detail.supports_action(DetailAction::Exec));
    }

    #[test]
    fn unknown_authorization_hides_strict_actions_but_keeps_reads_available() {
        let mut detail = DetailViewState {
            resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
            yaml: Some("kind: Pod".to_string()),
            ..DetailViewState::default()
        };
        detail
            .metadata
            .action_authorizations
            .insert(DetailAction::Exec, DetailActionAuthorization::Unknown);
        detail
            .metadata
            .action_authorizations
            .insert(DetailAction::Logs, DetailActionAuthorization::Unknown);

        assert!(detail.supports_action(DetailAction::Logs));
        assert!(!detail.supports_action(DetailAction::Exec));
    }

    #[test]
    fn events_only_appear_for_supported_resources() {
        let pod = ResourceRef::Pod("pod-0".to_string(), "ns".to_string());
        let node = ResourceRef::Node("node-0".to_string());

        assert!(pod.supports_detail_action(DetailAction::ViewEvents, None, None));
        assert!(!node.supports_detail_action(DetailAction::ViewEvents, None, None));
    }

    #[test]
    fn helm_release_does_not_offer_unsupported_mutations() {
        let helm = ResourceRef::HelmRelease("release".to_string(), "default".to_string());

        assert!(helm.supports_detail_action(DetailAction::ViewHelmHistory, None, None));
        assert!(!helm.supports_detail_action(DetailAction::EditYaml, None, None));
        assert!(!helm.supports_detail_action(DetailAction::Delete, None, None));
    }

    #[test]
    fn view_relationships_available_for_relationship_capable_resources() {
        let pod = ResourceRef::Pod("pod-0".to_string(), "ns".to_string());
        assert!(pod.supports_detail_action(DetailAction::ViewRelationships, None, None));

        let deploy = ResourceRef::Deployment("api".to_string(), "ns".to_string());
        assert!(deploy.supports_detail_action(DetailAction::ViewRelationships, None, None));

        let svc = ResourceRef::Service("svc".to_string(), "ns".to_string());
        assert!(svc.supports_detail_action(DetailAction::ViewRelationships, None, None));

        let pvc = ResourceRef::Pvc("pvc".to_string(), "ns".to_string());
        assert!(pvc.supports_detail_action(DetailAction::ViewRelationships, None, None));
    }

    #[test]
    fn view_relationships_unavailable_for_non_relationship_resources() {
        let node = ResourceRef::Node("node-0".to_string());
        assert!(!node.supports_detail_action(DetailAction::ViewRelationships, None, None));

        let cm = ResourceRef::ConfigMap("cm".to_string(), "ns".to_string());
        assert!(!cm.supports_detail_action(DetailAction::ViewRelationships, None, None));
    }

    #[test]
    fn network_policy_view_action_is_available_for_supported_resources() {
        assert!(
            ResourceRef::Pod("pod-0".to_string(), "ns".to_string()).supports_detail_action(
                DetailAction::ViewNetworkPolicies,
                None,
                None,
            )
        );
        assert!(
            ResourceRef::Namespace("ns".to_string()).supports_detail_action(
                DetailAction::ViewNetworkPolicies,
                None,
                None,
            )
        );
        assert!(
            ResourceRef::NetworkPolicy("np".to_string(), "ns".to_string()).supports_detail_action(
                DetailAction::ViewNetworkPolicies,
                None,
                None
            )
        );
        assert!(
            !ResourceRef::Node("node-0".to_string()).supports_detail_action(
                DetailAction::ViewNetworkPolicies,
                None,
                None,
            )
        );
        assert!(
            !ResourceRef::Service("svc".to_string(), "ns".to_string()).supports_detail_action(
                DetailAction::ViewNetworkPolicies,
                None,
                None,
            )
        );
    }

    #[test]
    fn traffic_debug_action_is_available_for_service_ingress_endpoint_and_pod() {
        for resource in [
            ResourceRef::Pod("pod-0".to_string(), "ns".to_string()),
            ResourceRef::Service("svc".to_string(), "ns".to_string()),
            ResourceRef::Endpoint("svc".to_string(), "ns".to_string()),
            ResourceRef::Ingress("edge".to_string(), "ns".to_string()),
        ] {
            assert!(resource.supports_detail_action(DetailAction::ViewTrafficDebug, None, None));
        }
        assert!(
            !ResourceRef::Node("node-0".to_string()).supports_detail_action(
                DetailAction::ViewTrafficDebug,
                None,
                None,
            )
        );
    }

    #[test]
    fn node_actions_are_state_aware() {
        let node = ResourceRef::Node("node-0".to_string());
        assert!(node.supports_detail_action(DetailAction::Cordon, Some(false), None));
        assert!(!node.supports_detail_action(DetailAction::Uncordon, Some(false), None));
        assert!(node.supports_detail_action(DetailAction::Drain, Some(false), None));

        assert!(!node.supports_detail_action(DetailAction::Cordon, Some(true), None));
        assert!(node.supports_detail_action(DetailAction::Uncordon, Some(true), None));
        assert!(node.supports_detail_action(DetailAction::Drain, Some(true), None));
    }

    #[test]
    fn non_node_resources_do_not_support_node_ops() {
        let pod = ResourceRef::Pod("pod-0".to_string(), "ns".to_string());
        assert!(!pod.supports_detail_action(DetailAction::Cordon, None, None));
        assert!(!pod.supports_detail_action(DetailAction::Uncordon, None, None));
        assert!(!pod.supports_detail_action(DetailAction::Drain, None, None));

        let deploy = ResourceRef::Deployment("api".to_string(), "ns".to_string());
        assert!(!deploy.supports_detail_action(DetailAction::Cordon, None, None));
        assert!(!deploy.supports_detail_action(DetailAction::Drain, None, None));
    }

    #[test]
    fn detail_node_actions_follow_unschedulable_state() {
        let mut detail = DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            yaml: Some("kind: Node".to_string()),
            ..DetailViewState::default()
        };
        detail.metadata.node_unschedulable = Some(false);
        assert!(detail.supports_action(DetailAction::Cordon));
        assert!(!detail.supports_action(DetailAction::Uncordon));
        assert!(!detail.supports_action(DetailAction::DebugContainer));

        detail.metadata.node_unschedulable = Some(true);
        assert!(!detail.supports_action(DetailAction::Cordon));
        assert!(detail.supports_action(DetailAction::Uncordon));
        assert!(!detail.supports_action(DetailAction::DebugContainer));
    }

    #[test]
    fn confirm_drain_blocks_detail_actions() {
        let detail = DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            yaml: Some("kind: Node".to_string()),
            confirm_drain: true,
            ..DetailViewState::default()
        };

        assert!(!detail.supports_action(DetailAction::ViewYaml));
        assert!(!detail.supports_action(DetailAction::Delete));
        assert!(!detail.supports_action(DetailAction::Cordon));
    }

    #[test]
    fn secret_resources_support_decoded_secret_action() {
        let secret = ResourceRef::Secret("app-secret".to_string(), "default".to_string());
        let config_map = ResourceRef::ConfigMap("app-config".to_string(), "default".to_string());

        assert!(secret.supports_detail_action(DetailAction::ViewDecodedSecret, None, None));
        assert!(!config_map.supports_detail_action(DetailAction::ViewDecodedSecret, None, None));
    }

    #[test]
    fn bookmark_action_is_available_for_resources() {
        let pod = ResourceRef::Pod("api".to_string(), "default".to_string());
        let cluster_role = ResourceRef::ClusterRole("admin".to_string());

        assert!(pod.supports_detail_action(DetailAction::ToggleBookmark, None, None));
        assert!(cluster_role.supports_detail_action(DetailAction::ToggleBookmark, None, None));
    }

    #[test]
    fn cronjob_actions_follow_suspend_state() {
        let cronjob = ResourceRef::CronJob("nightly".to_string(), "ops".to_string());

        assert!(cronjob.supports_detail_action(DetailAction::Trigger, None, Some(false)));
        assert!(cronjob.supports_detail_action(DetailAction::SuspendCronJob, None, Some(false),));
        assert!(!cronjob.supports_detail_action(DetailAction::ResumeCronJob, None, Some(false),));

        assert!(!cronjob.supports_detail_action(DetailAction::SuspendCronJob, None, Some(true),));
        assert!(cronjob.supports_detail_action(DetailAction::ResumeCronJob, None, Some(true),));
    }

    #[test]
    fn cronjob_logs_follow_selected_history_availability() {
        let mut detail = DetailViewState {
            resource: Some(ResourceRef::CronJob(
                "nightly".to_string(),
                "ops".to_string(),
            )),
            yaml: Some("kind: CronJob".to_string()),
            cronjob_history: vec![crate::cronjob::CronJobHistoryEntry {
                job_name: "nightly-001".to_string(),
                namespace: "ops".to_string(),
                status: "Running".to_string(),
                completions: "0/1".to_string(),
                duration: None,
                pod_count: 1,
                live_pod_count: 1,
                completion_pct: Some(0),
                active_pods: 1,
                failed_pods: 0,
                age: None,
                created_at: None,
                logs_authorized: Some(true),
            }],
            ..DetailViewState::default()
        };

        assert!(detail.supports_action(DetailAction::Logs));
        detail.cronjob_history[0].logs_authorized = Some(false);
        assert!(!detail.supports_action(DetailAction::Logs));
    }

    // ── supports_action_borrowed tri-state edge cases ──────────────

    #[test]
    fn empty_auth_map_permits_all_auth_required_actions() {
        let resource = ResourceRef::Pod("pod-0".to_string(), "ns".to_string());
        let empty = ActionAuthorizationMap::new();
        assert!(supports_action_borrowed(
            &resource,
            None,
            None,
            false,
            &empty,
            DetailAction::Exec
        ));
        assert!(supports_action_borrowed(
            &resource,
            None,
            None,
            false,
            &empty,
            DetailAction::Logs
        ));
        assert!(supports_action_borrowed(
            &resource,
            None,
            None,
            false,
            &empty,
            DetailAction::Delete
        ));
    }

    #[test]
    fn unknown_in_auth_map_blocks_strict_allows_soft() {
        let resource = ResourceRef::Pod("pod-0".to_string(), "ns".to_string());
        let mut auths = ActionAuthorizationMap::new();
        auths.insert(DetailAction::Exec, DetailActionAuthorization::Unknown);
        auths.insert(DetailAction::Logs, DetailActionAuthorization::Unknown);

        assert!(!supports_action_borrowed(
            &resource,
            None,
            None,
            false,
            &auths,
            DetailAction::Exec
        ));
        assert!(supports_action_borrowed(
            &resource,
            None,
            None,
            false,
            &auths,
            DetailAction::Logs
        ));
    }

    #[test]
    fn denied_in_auth_map_blocks_even_soft_actions() {
        let resource = ResourceRef::Pod("pod-0".to_string(), "ns".to_string());
        let mut auths = ActionAuthorizationMap::new();
        auths.insert(DetailAction::Logs, DetailActionAuthorization::Denied);
        auths.insert(DetailAction::ViewYaml, DetailActionAuthorization::Denied);

        assert!(!supports_action_borrowed(
            &resource,
            None,
            None,
            false,
            &auths,
            DetailAction::Logs
        ));
        assert!(!supports_action_borrowed(
            &resource,
            None,
            None,
            false,
            &auths,
            DetailAction::ViewYaml
        ));
    }

    #[test]
    fn allowed_in_auth_map_permits_strict_actions() {
        let resource = ResourceRef::Pod("pod-0".to_string(), "ns".to_string());
        let mut auths = ActionAuthorizationMap::new();
        auths.insert(DetailAction::Exec, DetailActionAuthorization::Allowed);
        auths.insert(DetailAction::Delete, DetailActionAuthorization::Allowed);

        assert!(supports_action_borrowed(
            &resource,
            None,
            None,
            false,
            &auths,
            DetailAction::Exec
        ));
        assert!(supports_action_borrowed(
            &resource,
            None,
            None,
            false,
            &auths,
            DetailAction::Delete
        ));
    }

    #[test]
    fn non_auth_actions_always_pass_regardless_of_map() {
        let resource = ResourceRef::Pod("pod-0".to_string(), "ns".to_string());
        let empty = ActionAuthorizationMap::new();
        assert!(supports_action_borrowed(
            &resource,
            None,
            None,
            false,
            &empty,
            DetailAction::ToggleBookmark
        ));
        assert!(supports_action_borrowed(
            &resource,
            None,
            None,
            false,
            &empty,
            DetailAction::ViewRelationships
        ));
    }

    #[test]
    fn unsupported_resource_action_blocked_even_when_allowed_in_auth_map() {
        let pod = ResourceRef::Pod("pod-0".to_string(), "ns".to_string());
        let mut auths = ActionAuthorizationMap::new();
        auths.insert(DetailAction::Scale, DetailActionAuthorization::Allowed);

        assert!(!supports_action_borrowed(
            &pod,
            None,
            None,
            false,
            &auths,
            DetailAction::Scale
        ));
    }

    #[test]
    fn node_auth_interacts_with_unschedulable_state() {
        let node = ResourceRef::Node("node-0".to_string());
        let mut auths = ActionAuthorizationMap::new();
        auths.insert(DetailAction::Cordon, DetailActionAuthorization::Allowed);
        auths.insert(DetailAction::Uncordon, DetailActionAuthorization::Allowed);

        assert!(supports_action_borrowed(
            &node,
            Some(false),
            None,
            false,
            &auths,
            DetailAction::Cordon
        ));
        assert!(!supports_action_borrowed(
            &node,
            Some(false),
            None,
            false,
            &auths,
            DetailAction::Uncordon
        ));

        assert!(!supports_action_borrowed(
            &node,
            Some(true),
            None,
            false,
            &auths,
            DetailAction::Cordon
        ));
        assert!(supports_action_borrowed(
            &node,
            Some(true),
            None,
            false,
            &auths,
            DetailAction::Uncordon
        ));
    }

    #[test]
    fn node_drain_unknown_auth_is_blocked() {
        let node = ResourceRef::Node("node-0".to_string());
        let mut auths = ActionAuthorizationMap::new();
        auths.insert(DetailAction::Drain, DetailActionAuthorization::Unknown);

        assert!(!supports_action_borrowed(
            &node,
            Some(false),
            None,
            false,
            &auths,
            DetailAction::Drain
        ));
    }

    #[test]
    fn cronjob_logs_bypass_auth_when_history_available() {
        let cj = ResourceRef::CronJob("nightly".to_string(), "ops".to_string());
        let mut auths = ActionAuthorizationMap::new();
        auths.insert(DetailAction::Logs, DetailActionAuthorization::Denied);

        assert!(supports_action_borrowed(
            &cj,
            None,
            None,
            true,
            &auths,
            DetailAction::Logs
        ));
        assert!(!supports_action_borrowed(
            &cj,
            None,
            None,
            false,
            &auths,
            DetailAction::Logs
        ));
    }

    #[test]
    fn relationship_and_persistence_tables_cover_core_views() {
        assert!(
            AppView::Services
                .relationship_capabilities()
                .contains(&RelationshipCapability::ServiceBackends)
        );
        assert!(
            AppView::PersistentVolumeClaims
                .relationship_capabilities()
                .contains(&RelationshipCapability::StorageBindings)
        );
        assert!(
            AppView::Pods
                .persistence_capabilities()
                .contains(&ViewPersistenceCapability::Sort)
        );
        assert!(
            AppView::Pods
                .persistence_capabilities()
                .contains(&ViewPersistenceCapability::ColumnLayout)
        );
    }
}
