//! Canonical cross-view and detail action policies.

use crate::app::{AppView, DetailViewState, ResourceRef, WorkloadSortColumn};
use crate::k8s::flux::flux_reconcile_support;

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
    StorageBindings,
    FluxLineage,
    RbacBindings,
}

/// Actions that can appear in the detail footer or be triggered from detail context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DetailAction {
    ViewYaml,
    ViewEvents,
    Logs,
    Exec,
    PortForward,
    Probes,
    Scale,
    Restart,
    FluxReconcile,
    EditYaml,
    Delete,
    Trigger,
    ViewRelationships,
    Cordon,
    Uncordon,
    Drain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceActionContext {
    pub resource: ResourceRef,
    pub node_unschedulable: Option<bool>,
}

impl DetailAction {
    pub const ORDER: [DetailAction; 16] = [
        DetailAction::ViewYaml,
        DetailAction::ViewEvents,
        DetailAction::Logs,
        DetailAction::Exec,
        DetailAction::PortForward,
        DetailAction::Probes,
        DetailAction::Scale,
        DetailAction::Restart,
        DetailAction::FluxReconcile,
        DetailAction::EditYaml,
        DetailAction::Delete,
        DetailAction::Trigger,
        DetailAction::ViewRelationships,
        DetailAction::Cordon,
        DetailAction::Uncordon,
        DetailAction::Drain,
    ];

    pub const fn key_hint(self) -> &'static str {
        match self {
            DetailAction::ViewYaml => "[y]",
            DetailAction::ViewEvents => "[v]",
            DetailAction::Logs => "[l]",
            DetailAction::Exec => "[x]",
            DetailAction::PortForward => "[f]",
            DetailAction::Probes => "[p]",
            DetailAction::Scale => "[s]",
            DetailAction::Restart | DetailAction::FluxReconcile => "[R]",
            DetailAction::EditYaml => "[e]",
            DetailAction::Delete => "[d]",
            DetailAction::Trigger => "[T]",
            DetailAction::ViewRelationships => "[w]",
            DetailAction::Cordon => "[c]",
            DetailAction::Uncordon => "[u]",
            DetailAction::Drain => "[D]",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            DetailAction::ViewYaml => "YAML",
            DetailAction::ViewEvents => "Events",
            DetailAction::Logs => "Logs",
            DetailAction::Exec => "Exec",
            DetailAction::PortForward => "Port-Fwd",
            DetailAction::Probes => "Probes",
            DetailAction::Scale => "Scale",
            DetailAction::Restart => "Restart",
            DetailAction::FluxReconcile => "Reconcile",
            DetailAction::EditYaml => "Edit",
            DetailAction::Delete => "Delete",
            DetailAction::Trigger => "Trigger",
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
            | AppView::PortForwarding
            | AppView::HelmCharts
            | AppView::Extensions => PERSISTENCE_NONE,
            AppView::Pods
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
    ) -> bool {
        match action {
            DetailAction::ViewYaml | DetailAction::ViewEvents => true,
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
            DetailAction::Exec | DetailAction::PortForward | DetailAction::Probes => {
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
            DetailAction::EditYaml | DetailAction::Delete => true,
            DetailAction::Trigger => matches!(self, ResourceRef::CronJob(_, _)),
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

impl ResourceActionContext {
    pub fn supports_action(&self, action: DetailAction) -> bool {
        self.resource
            .supports_detail_action(action, self.node_unschedulable)
    }
}

impl DetailViewState {
    pub fn resource_action_context(&self) -> Option<ResourceActionContext> {
        self.resource.clone().map(|resource| ResourceActionContext {
            resource,
            node_unschedulable: self.metadata.node_unschedulable,
        })
    }

    pub fn has_blocking_detail_overlay(&self) -> bool {
        self.scale_dialog.is_some()
            || self.probe_panel.is_some()
            || self.confirm_delete
            || self.confirm_drain
    }

    pub fn supports_action(&self, action: DetailAction) -> bool {
        let Some(resource) = self.resource_action_context() else {
            return false;
        };

        let requires_clear_surface = matches!(
            action,
            DetailAction::ViewYaml
                | DetailAction::ViewEvents
                | DetailAction::Logs
                | DetailAction::Exec
                | DetailAction::PortForward
                | DetailAction::Probes
                | DetailAction::Scale
                | DetailAction::Restart
                | DetailAction::FluxReconcile
                | DetailAction::EditYaml
                | DetailAction::Delete
                | DetailAction::Trigger
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

        resource.supports_action(action)
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
    use crate::app::AppView;

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
        assert!(detail.supports_action(DetailAction::PortForward));
        assert!(detail.supports_action(DetailAction::Probes));
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
        assert!(detail.supports_action(DetailAction::EditYaml));
        assert!(detail.supports_action(DetailAction::Delete));
        assert!(detail.supports_action(DetailAction::Logs));
        assert!(!detail.supports_action(DetailAction::FluxReconcile));
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
    fn view_relationships_available_for_relationship_capable_resources() {
        let pod = ResourceRef::Pod("pod-0".to_string(), "ns".to_string());
        assert!(pod.supports_detail_action(DetailAction::ViewRelationships, None));

        let deploy = ResourceRef::Deployment("api".to_string(), "ns".to_string());
        assert!(deploy.supports_detail_action(DetailAction::ViewRelationships, None));

        let svc = ResourceRef::Service("svc".to_string(), "ns".to_string());
        assert!(svc.supports_detail_action(DetailAction::ViewRelationships, None));

        let pvc = ResourceRef::Pvc("pvc".to_string(), "ns".to_string());
        assert!(pvc.supports_detail_action(DetailAction::ViewRelationships, None));
    }

    #[test]
    fn view_relationships_unavailable_for_non_relationship_resources() {
        let node = ResourceRef::Node("node-0".to_string());
        assert!(!node.supports_detail_action(DetailAction::ViewRelationships, None));

        let cm = ResourceRef::ConfigMap("cm".to_string(), "ns".to_string());
        assert!(!cm.supports_detail_action(DetailAction::ViewRelationships, None));
    }

    #[test]
    fn node_actions_are_state_aware() {
        let node = ResourceRef::Node("node-0".to_string());
        assert!(node.supports_detail_action(DetailAction::Cordon, Some(false)));
        assert!(!node.supports_detail_action(DetailAction::Uncordon, Some(false)));
        assert!(node.supports_detail_action(DetailAction::Drain, Some(false)));

        assert!(!node.supports_detail_action(DetailAction::Cordon, Some(true)));
        assert!(node.supports_detail_action(DetailAction::Uncordon, Some(true)));
        assert!(node.supports_detail_action(DetailAction::Drain, Some(true)));
    }

    #[test]
    fn non_node_resources_do_not_support_node_ops() {
        let pod = ResourceRef::Pod("pod-0".to_string(), "ns".to_string());
        assert!(!pod.supports_detail_action(DetailAction::Cordon, None));
        assert!(!pod.supports_detail_action(DetailAction::Uncordon, None));
        assert!(!pod.supports_detail_action(DetailAction::Drain, None));

        let deploy = ResourceRef::Deployment("api".to_string(), "ns".to_string());
        assert!(!deploy.supports_detail_action(DetailAction::Cordon, None));
        assert!(!deploy.supports_detail_action(DetailAction::Drain, None));
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

        detail.metadata.node_unschedulable = Some(true);
        assert!(!detail.supports_action(DetailAction::Cordon));
        assert!(detail.supports_action(DetailAction::Uncordon));
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
