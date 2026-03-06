//! Global state management for KubecTUI.

pub mod alerts;
pub mod filters;
pub mod port_forward;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::{collections::HashSet, fmt, sync::LazyLock, time::Duration};
use tokio::sync::Semaphore;

use crate::app::AppView;
use crate::k8s::{
    client::K8sClient,
    dtos::{
        ClusterInfo, ClusterRoleBindingInfo, ClusterRoleInfo, ConfigMapInfo, CronJobInfo,
        CustomResourceDefinitionInfo, DaemonSetInfo, DeploymentInfo, EndpointInfo,
        FluxResourceInfo, HelmReleaseInfo, HpaInfo, IngressClassInfo, IngressInfo, JobInfo,
        K8sEventInfo, LimitRangeInfo, NamespaceInfo, NetworkPolicyInfo, NodeInfo, NodeMetricsInfo,
        PodDisruptionBudgetInfo, PodInfo, PriorityClassInfo, PvInfo, PvcInfo, ReplicaSetInfo,
        ReplicationControllerInfo, ResourceQuotaInfo, RoleBindingInfo, RoleInfo, SecretInfo,
        ServiceAccountInfo, ServiceInfo, StatefulSetInfo, StorageClassInfo,
    },
};

const MAX_CONCURRENT_RESOURCE_FETCHES: usize = 8;
static RESOURCE_FETCH_SEMAPHORE: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(MAX_CONCURRENT_RESOURCE_FETCHES));

/// High-level data loading phase for cluster resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DataPhase {
    /// No data fetch attempted yet.
    #[default]
    Idle,
    /// Data fetch is currently in progress.
    Loading,
    /// Data is available from the most recent successful fetch.
    Ready,
    /// Last fetch failed.
    Error,
}

impl fmt::Display for DataPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            DataPhase::Idle => "idle",
            DataPhase::Loading => "loading",
            DataPhase::Ready => "ready",
            DataPhase::Error => "error",
        };
        f.write_str(label)
    }
}

/// Fine-grained loading state for each top-level view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewLoadState {
    /// No request has been made for this view in the current scope yet.
    #[default]
    Idle,
    /// Data is expected but has not been shown yet.
    Loading,
    /// Existing data is on-screen while a background refresh is in progress.
    Refreshing,
    /// Data for this view has completed at least one successful fetch in the current scope.
    Ready,
}

impl ViewLoadState {
    pub const fn is_loading(self) -> bool {
        matches!(self, Self::Loading | Self::Refreshing)
    }
}

/// Snapshot used by rendering layer.
#[derive(Debug, Clone)]
pub struct ClusterSnapshot {
    /// Monotonic snapshot revision incremented whenever snapshot data is replaced.
    pub snapshot_version: u64,
    /// True once the current scope has completed at least one secondary-resource fetch.
    pub secondary_resources_loaded: bool,
    pub view_load_states: [ViewLoadState; AppView::COUNT],
    pub nodes: Vec<NodeInfo>,
    pub pods: Vec<PodInfo>,
    pub services: Vec<ServiceInfo>,
    pub deployments: Vec<DeploymentInfo>,
    pub statefulsets: Vec<StatefulSetInfo>,
    pub daemonsets: Vec<DaemonSetInfo>,
    pub replicasets: Vec<ReplicaSetInfo>,
    pub replication_controllers: Vec<ReplicationControllerInfo>,
    pub jobs: Vec<JobInfo>,
    pub cronjobs: Vec<CronJobInfo>,
    pub resource_quotas: Vec<ResourceQuotaInfo>,
    pub limit_ranges: Vec<LimitRangeInfo>,
    pub pod_disruption_budgets: Vec<PodDisruptionBudgetInfo>,
    pub service_accounts: Vec<ServiceAccountInfo>,
    pub roles: Vec<RoleInfo>,
    pub role_bindings: Vec<RoleBindingInfo>,
    pub cluster_roles: Vec<ClusterRoleInfo>,
    pub cluster_role_bindings: Vec<ClusterRoleBindingInfo>,
    pub custom_resource_definitions: Vec<CustomResourceDefinitionInfo>,
    pub cluster_info: Option<ClusterInfo>,
    // New fields for previously-placeholder views
    pub endpoints: Vec<EndpointInfo>,
    pub ingresses: Vec<IngressInfo>,
    pub ingress_classes: Vec<IngressClassInfo>,
    pub network_policies: Vec<NetworkPolicyInfo>,
    pub config_maps: Vec<ConfigMapInfo>,
    pub secrets: Vec<SecretInfo>,
    pub hpas: Vec<HpaInfo>,
    pub pvcs: Vec<PvcInfo>,
    pub pvs: Vec<PvInfo>,
    pub storage_classes: Vec<StorageClassInfo>,
    pub namespace_list: Vec<NamespaceInfo>,
    pub events: Vec<K8sEventInfo>,
    pub priority_classes: Vec<PriorityClassInfo>,
    pub helm_releases: Vec<HelmReleaseInfo>,
    pub flux_resources: Vec<FluxResourceInfo>,
    pub helm_repositories: Vec<crate::k8s::dtos::HelmRepoInfo>,
    pub node_metrics: Vec<NodeMetricsInfo>,
    pub services_count: usize,
    pub namespaces_count: usize,
    pub phase: DataPhase,
    pub last_updated: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub cluster_url: Option<String>,
}

impl Default for ClusterSnapshot {
    fn default() -> Self {
        Self {
            snapshot_version: 0,
            secondary_resources_loaded: false,
            view_load_states: [ViewLoadState::Idle; AppView::COUNT],
            nodes: Vec::new(),
            pods: Vec::new(),
            services: Vec::new(),
            deployments: Vec::new(),
            statefulsets: Vec::new(),
            daemonsets: Vec::new(),
            replicasets: Vec::new(),
            replication_controllers: Vec::new(),
            jobs: Vec::new(),
            cronjobs: Vec::new(),
            resource_quotas: Vec::new(),
            limit_ranges: Vec::new(),
            pod_disruption_budgets: Vec::new(),
            service_accounts: Vec::new(),
            roles: Vec::new(),
            role_bindings: Vec::new(),
            cluster_roles: Vec::new(),
            cluster_role_bindings: Vec::new(),
            custom_resource_definitions: Vec::new(),
            cluster_info: None,
            endpoints: Vec::new(),
            ingresses: Vec::new(),
            ingress_classes: Vec::new(),
            network_policies: Vec::new(),
            config_maps: Vec::new(),
            secrets: Vec::new(),
            hpas: Vec::new(),
            pvcs: Vec::new(),
            pvs: Vec::new(),
            storage_classes: Vec::new(),
            namespace_list: Vec::new(),
            events: Vec::new(),
            priority_classes: Vec::new(),
            helm_releases: Vec::new(),
            flux_resources: Vec::new(),
            helm_repositories: Vec::new(),
            node_metrics: Vec::new(),
            services_count: 0,
            namespaces_count: 0,
            phase: DataPhase::Idle,
            last_updated: None,
            last_error: None,
            cluster_url: None,
        }
    }
}

impl ClusterSnapshot {
    /// Returns a compact string suitable for header display.
    pub fn cluster_summary(&self) -> &str {
        self.cluster_url
            .as_deref()
            .unwrap_or("Cluster endpoint unavailable")
    }

    pub fn view_load_state(&self, view: AppView) -> ViewLoadState {
        self.view_load_states[view.index()]
    }
}

/// Data source contract for retrieving Kubernetes snapshot inputs.
#[async_trait]
pub trait ClusterDataSource {
    /// Returns cluster API URL for status header.
    fn cluster_url(&self) -> &str;
    /// Fetches node list.
    async fn fetch_nodes(&self) -> Result<Vec<NodeInfo>>;
    /// Fetches available namespaces.
    async fn fetch_namespaces(&self) -> Result<Vec<String>>;
    /// Fetches pod list.
    async fn fetch_pods(&self, namespace: Option<&str>) -> Result<Vec<PodInfo>>;
    /// Fetches service list.
    async fn fetch_services(&self, namespace: Option<&str>) -> Result<Vec<ServiceInfo>>;
    /// Fetches deployment list.
    async fn fetch_deployments(&self, namespace: Option<&str>) -> Result<Vec<DeploymentInfo>>;
    /// Fetches StatefulSet list.
    async fn fetch_statefulsets(&self, namespace: Option<&str>) -> Result<Vec<StatefulSetInfo>>;
    /// Fetches DaemonSet list.
    async fn fetch_daemonsets(&self, namespace: Option<&str>) -> Result<Vec<DaemonSetInfo>>;
    /// Fetches ReplicaSet list.
    async fn fetch_replicasets(&self, namespace: Option<&str>) -> Result<Vec<ReplicaSetInfo>>;
    /// Fetches ReplicationController list.
    async fn fetch_replication_controllers(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<ReplicationControllerInfo>>;
    /// Fetches Job list.
    async fn fetch_jobs(&self, namespace: Option<&str>) -> Result<Vec<JobInfo>>;
    /// Fetches CronJob list.
    async fn fetch_cronjobs(&self, namespace: Option<&str>) -> Result<Vec<CronJobInfo>>;
    /// Fetches ResourceQuota list.
    async fn fetch_resource_quotas(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<ResourceQuotaInfo>>;
    /// Fetches LimitRange list.
    async fn fetch_limit_ranges(&self, namespace: Option<&str>) -> Result<Vec<LimitRangeInfo>>;
    /// Fetches PodDisruptionBudget list.
    async fn fetch_pod_disruption_budgets(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<PodDisruptionBudgetInfo>>;
    /// Fetches ServiceAccount list.
    async fn fetch_service_accounts(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<ServiceAccountInfo>>;
    /// Fetches Role list.
    async fn fetch_roles(&self, namespace: Option<&str>) -> Result<Vec<RoleInfo>>;
    /// Fetches RoleBinding list.
    async fn fetch_role_bindings(&self, namespace: Option<&str>) -> Result<Vec<RoleBindingInfo>>;
    /// Fetches ClusterRole list.
    async fn fetch_cluster_roles(&self) -> Result<Vec<ClusterRoleInfo>>;
    /// Fetches ClusterRoleBinding list.
    async fn fetch_cluster_role_bindings(&self) -> Result<Vec<ClusterRoleBindingInfo>>;
    /// Fetches CRD list used by Extensions view.
    async fn fetch_custom_resource_definitions(&self) -> Result<Vec<CustomResourceDefinitionInfo>>;
    /// Fetches cluster metadata.
    async fn fetch_cluster_info(&self) -> Result<ClusterInfo>;
    /// Fetches Endpoints.
    async fn fetch_endpoints(&self, namespace: Option<&str>) -> Result<Vec<EndpointInfo>>;
    /// Fetches Ingresses.
    async fn fetch_ingresses(&self, namespace: Option<&str>) -> Result<Vec<IngressInfo>>;
    /// Fetches IngressClasses.
    async fn fetch_ingress_classes(&self) -> Result<Vec<IngressClassInfo>>;
    /// Fetches NetworkPolicies.
    async fn fetch_network_policies(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<NetworkPolicyInfo>>;
    /// Fetches ConfigMaps.
    async fn fetch_config_maps(&self, namespace: Option<&str>) -> Result<Vec<ConfigMapInfo>>;
    /// Fetches Secrets.
    async fn fetch_secrets(&self, namespace: Option<&str>) -> Result<Vec<SecretInfo>>;
    /// Fetches HPAs.
    async fn fetch_hpas(&self, namespace: Option<&str>) -> Result<Vec<HpaInfo>>;
    /// Fetches PVCs.
    async fn fetch_pvcs(&self, namespace: Option<&str>) -> Result<Vec<PvcInfo>>;
    /// Fetches PVs.
    async fn fetch_pvs(&self) -> Result<Vec<PvInfo>>;
    /// Fetches StorageClasses.
    async fn fetch_storage_classes(&self) -> Result<Vec<StorageClassInfo>>;
    /// Fetches Namespaces as NamespaceInfo.
    async fn fetch_namespace_list(&self) -> Result<Vec<NamespaceInfo>>;
    /// Fetches Events.
    async fn fetch_events(&self, namespace: Option<&str>) -> Result<Vec<K8sEventInfo>>;
    /// Fetches PriorityClasses.
    async fn fetch_priority_classes(&self) -> Result<Vec<PriorityClassInfo>>;
    /// Fetches Helm releases.
    async fn fetch_helm_releases(&self, namespace: Option<&str>) -> Result<Vec<HelmReleaseInfo>>;
    /// Fetches Flux resources.
    async fn fetch_flux_resources(&self, namespace: Option<&str>) -> Result<Vec<FluxResourceInfo>>;
    /// Fetches metrics for all nodes (best-effort, returns empty if metrics-server absent).
    async fn fetch_all_node_metrics(&self) -> Result<Vec<NodeMetricsInfo>>;
}

#[async_trait]
impl ClusterDataSource for K8sClient {
    fn cluster_url(&self) -> &str {
        K8sClient::cluster_url(self)
    }

    async fn fetch_nodes(&self) -> Result<Vec<NodeInfo>> {
        K8sClient::fetch_nodes(self).await
    }

    async fn fetch_namespaces(&self) -> Result<Vec<String>> {
        K8sClient::fetch_namespaces(self).await
    }

    async fn fetch_pods(&self, namespace: Option<&str>) -> Result<Vec<PodInfo>> {
        K8sClient::fetch_pods(self, namespace).await
    }

    async fn fetch_services(&self, namespace: Option<&str>) -> Result<Vec<ServiceInfo>> {
        K8sClient::fetch_services(self, namespace).await
    }

    async fn fetch_deployments(&self, namespace: Option<&str>) -> Result<Vec<DeploymentInfo>> {
        K8sClient::fetch_deployments(self, namespace).await
    }

    async fn fetch_statefulsets(&self, namespace: Option<&str>) -> Result<Vec<StatefulSetInfo>> {
        K8sClient::fetch_statefulsets(self, namespace).await
    }

    async fn fetch_daemonsets(&self, namespace: Option<&str>) -> Result<Vec<DaemonSetInfo>> {
        K8sClient::fetch_daemonsets(self, namespace).await
    }

    async fn fetch_replicasets(&self, namespace: Option<&str>) -> Result<Vec<ReplicaSetInfo>> {
        K8sClient::fetch_replicasets(self, namespace).await
    }

    async fn fetch_replication_controllers(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<ReplicationControllerInfo>> {
        K8sClient::fetch_replication_controllers(self, namespace).await
    }

    async fn fetch_jobs(&self, namespace: Option<&str>) -> Result<Vec<JobInfo>> {
        K8sClient::fetch_jobs(self, namespace).await
    }

    async fn fetch_cronjobs(&self, namespace: Option<&str>) -> Result<Vec<CronJobInfo>> {
        K8sClient::fetch_cronjobs(self, namespace).await
    }

    async fn fetch_resource_quotas(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<ResourceQuotaInfo>> {
        K8sClient::fetch_resource_quotas(self, namespace).await
    }

    async fn fetch_limit_ranges(&self, namespace: Option<&str>) -> Result<Vec<LimitRangeInfo>> {
        K8sClient::fetch_limit_ranges(self, namespace).await
    }

    async fn fetch_pod_disruption_budgets(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<PodDisruptionBudgetInfo>> {
        K8sClient::fetch_pod_disruption_budgets(self, namespace).await
    }

    async fn fetch_service_accounts(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<ServiceAccountInfo>> {
        K8sClient::fetch_service_accounts(self, namespace).await
    }

    async fn fetch_roles(&self, namespace: Option<&str>) -> Result<Vec<RoleInfo>> {
        K8sClient::fetch_roles(self, namespace).await
    }

    async fn fetch_role_bindings(&self, namespace: Option<&str>) -> Result<Vec<RoleBindingInfo>> {
        K8sClient::fetch_role_bindings(self, namespace).await
    }

    async fn fetch_cluster_roles(&self) -> Result<Vec<ClusterRoleInfo>> {
        K8sClient::fetch_cluster_roles(self).await
    }

    async fn fetch_cluster_role_bindings(&self) -> Result<Vec<ClusterRoleBindingInfo>> {
        K8sClient::fetch_cluster_role_bindings(self).await
    }

    async fn fetch_custom_resource_definitions(&self) -> Result<Vec<CustomResourceDefinitionInfo>> {
        K8sClient::fetch_custom_resource_definitions(self).await
    }

    async fn fetch_cluster_info(&self) -> Result<ClusterInfo> {
        K8sClient::fetch_cluster_info(self).await
    }

    async fn fetch_endpoints(&self, namespace: Option<&str>) -> Result<Vec<EndpointInfo>> {
        K8sClient::fetch_endpoints(self, namespace).await
    }

    async fn fetch_ingresses(&self, namespace: Option<&str>) -> Result<Vec<IngressInfo>> {
        K8sClient::fetch_ingresses(self, namespace).await
    }

    async fn fetch_ingress_classes(&self) -> Result<Vec<IngressClassInfo>> {
        K8sClient::fetch_ingress_classes(self).await
    }

    async fn fetch_network_policies(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<NetworkPolicyInfo>> {
        K8sClient::fetch_network_policies(self, namespace).await
    }

    async fn fetch_config_maps(&self, namespace: Option<&str>) -> Result<Vec<ConfigMapInfo>> {
        K8sClient::fetch_config_maps(self, namespace).await
    }

    async fn fetch_secrets(&self, namespace: Option<&str>) -> Result<Vec<SecretInfo>> {
        K8sClient::fetch_secrets(self, namespace).await
    }

    async fn fetch_hpas(&self, namespace: Option<&str>) -> Result<Vec<HpaInfo>> {
        K8sClient::fetch_hpas(self, namespace).await
    }

    async fn fetch_pvcs(&self, namespace: Option<&str>) -> Result<Vec<PvcInfo>> {
        K8sClient::fetch_pvcs(self, namespace).await
    }

    async fn fetch_pvs(&self) -> Result<Vec<PvInfo>> {
        K8sClient::fetch_pvs(self).await
    }

    async fn fetch_storage_classes(&self) -> Result<Vec<StorageClassInfo>> {
        K8sClient::fetch_storage_classes(self).await
    }

    async fn fetch_namespace_list(&self) -> Result<Vec<NamespaceInfo>> {
        K8sClient::fetch_namespace_list(self).await
    }

    async fn fetch_events(&self, namespace: Option<&str>) -> Result<Vec<K8sEventInfo>> {
        K8sClient::fetch_events(self, namespace).await
    }

    async fn fetch_priority_classes(&self) -> Result<Vec<PriorityClassInfo>> {
        K8sClient::fetch_priority_classes(self).await
    }

    async fn fetch_helm_releases(&self, namespace: Option<&str>) -> Result<Vec<HelmReleaseInfo>> {
        K8sClient::fetch_helm_releases(self, namespace).await
    }

    async fn fetch_flux_resources(&self, namespace: Option<&str>) -> Result<Vec<FluxResourceInfo>> {
        K8sClient::fetch_flux_resources(self, namespace).await
    }

    async fn fetch_all_node_metrics(&self) -> Result<Vec<NodeMetricsInfo>> {
        K8sClient::fetch_all_node_metrics(self).await
    }
}

/// Mutable state holder with async refresh operations.
#[derive(Debug, Clone, Default)]
pub struct GlobalState {
    snapshot: ClusterSnapshot,
    /// Cached Arc snapshot — only rebuilt when data changes.
    arc_snapshot: std::sync::Arc<ClusterSnapshot>,
    pub namespaces: Vec<String>,
    snapshot_dirty: bool,
}

/// Runtime refresh knobs for optional expensive fetch paths.
#[derive(Debug, Clone, Copy)]
pub struct RefreshOptions {
    pub include_flux: bool,
    pub include_cluster_info: bool,
    pub include_secondary_resources: bool,
    pub include_events: bool,
}

impl Default for RefreshOptions {
    fn default() -> Self {
        Self {
            include_flux: true,
            include_cluster_info: true,
            include_secondary_resources: true,
            include_events: true,
        }
    }
}

const CORE_REFRESH_VIEWS: &[AppView] = &[
    AppView::Dashboard,
    AppView::Nodes,
    AppView::Namespaces,
    AppView::Pods,
    AppView::Deployments,
    AppView::StatefulSets,
    AppView::DaemonSets,
    AppView::ReplicaSets,
    AppView::ReplicationControllers,
    AppView::Jobs,
    AppView::CronJobs,
    AppView::Services,
    AppView::HelmCharts,
];

const SECONDARY_REFRESH_VIEWS: &[AppView] = &[
    AppView::Endpoints,
    AppView::Ingresses,
    AppView::IngressClasses,
    AppView::NetworkPolicies,
    AppView::ConfigMaps,
    AppView::Secrets,
    AppView::ResourceQuotas,
    AppView::LimitRanges,
    AppView::HPAs,
    AppView::PodDisruptionBudgets,
    AppView::PriorityClasses,
    AppView::PersistentVolumeClaims,
    AppView::PersistentVolumes,
    AppView::StorageClasses,
    AppView::HelmReleases,
    AppView::ServiceAccounts,
    AppView::ClusterRoles,
    AppView::Roles,
    AppView::ClusterRoleBindings,
    AppView::RoleBindings,
    AppView::Extensions,
];

const FLUX_REFRESH_VIEWS: &[AppView] = &[
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
];

const EVENT_REFRESH_VIEWS: &[AppView] = &[AppView::Events];

impl GlobalState {
    /// Returns a cheap Arc-wrapped snapshot for UI rendering.
    /// No deep clone — just an Arc pointer bump.
    pub fn snapshot(&self) -> std::sync::Arc<ClusterSnapshot> {
        self.arc_snapshot.clone()
    }

    /// Rebuilds the Arc snapshot from the inner mutable snapshot.
    /// Called after every successful refresh.
    fn publish_snapshot(&mut self) {
        if !self.snapshot_dirty {
            return;
        }
        self.snapshot_dirty = false;
        self.arc_snapshot = std::sync::Arc::new(self.snapshot.clone());
    }

    /// Returns fetched namespaces.
    pub fn namespaces(&self) -> &[String] {
        &self.namespaces
    }

    fn has_view_data(&self, view: AppView) -> bool {
        match view {
            AppView::Dashboard => {
                self.snapshot.cluster_info.is_some()
                    || !self.snapshot.nodes.is_empty()
                    || !self.snapshot.pods.is_empty()
                    || !self.snapshot.services.is_empty()
                    || !self.snapshot.deployments.is_empty()
            }
            AppView::Nodes => !self.snapshot.nodes.is_empty(),
            AppView::Namespaces => !self.snapshot.namespace_list.is_empty(),
            AppView::Events => !self.snapshot.events.is_empty(),
            AppView::Pods => !self.snapshot.pods.is_empty(),
            AppView::Deployments => !self.snapshot.deployments.is_empty(),
            AppView::StatefulSets => !self.snapshot.statefulsets.is_empty(),
            AppView::DaemonSets => !self.snapshot.daemonsets.is_empty(),
            AppView::ReplicaSets => !self.snapshot.replicasets.is_empty(),
            AppView::ReplicationControllers => !self.snapshot.replication_controllers.is_empty(),
            AppView::Jobs => !self.snapshot.jobs.is_empty(),
            AppView::CronJobs => !self.snapshot.cronjobs.is_empty(),
            AppView::Services => !self.snapshot.services.is_empty(),
            AppView::Endpoints => !self.snapshot.endpoints.is_empty(),
            AppView::Ingresses => !self.snapshot.ingresses.is_empty(),
            AppView::IngressClasses => !self.snapshot.ingress_classes.is_empty(),
            AppView::NetworkPolicies => !self.snapshot.network_policies.is_empty(),
            AppView::PortForwarding => true,
            AppView::ConfigMaps => !self.snapshot.config_maps.is_empty(),
            AppView::Secrets => !self.snapshot.secrets.is_empty(),
            AppView::ResourceQuotas => !self.snapshot.resource_quotas.is_empty(),
            AppView::LimitRanges => !self.snapshot.limit_ranges.is_empty(),
            AppView::HPAs => !self.snapshot.hpas.is_empty(),
            AppView::PodDisruptionBudgets => !self.snapshot.pod_disruption_budgets.is_empty(),
            AppView::PriorityClasses => !self.snapshot.priority_classes.is_empty(),
            AppView::PersistentVolumeClaims => !self.snapshot.pvcs.is_empty(),
            AppView::PersistentVolumes => !self.snapshot.pvs.is_empty(),
            AppView::StorageClasses => !self.snapshot.storage_classes.is_empty(),
            AppView::HelmCharts => !self.snapshot.helm_repositories.is_empty(),
            AppView::HelmReleases => !self.snapshot.helm_releases.is_empty(),
            AppView::FluxCDAlertProviders
            | AppView::FluxCDAlerts
            | AppView::FluxCDAll
            | AppView::FluxCDArtifacts
            | AppView::FluxCDHelmReleases
            | AppView::FluxCDHelmRepositories
            | AppView::FluxCDImages
            | AppView::FluxCDKustomizations
            | AppView::FluxCDReceivers
            | AppView::FluxCDSources => !self.snapshot.flux_resources.is_empty(),
            AppView::ServiceAccounts => !self.snapshot.service_accounts.is_empty(),
            AppView::ClusterRoles => !self.snapshot.cluster_roles.is_empty(),
            AppView::Roles => !self.snapshot.roles.is_empty(),
            AppView::ClusterRoleBindings => !self.snapshot.cluster_role_bindings.is_empty(),
            AppView::RoleBindings => !self.snapshot.role_bindings.is_empty(),
            AppView::Extensions => !self.snapshot.custom_resource_definitions.is_empty(),
        }
    }

    fn set_view_load_state(&mut self, view: AppView, state: ViewLoadState) -> bool {
        let slot = &mut self.snapshot.view_load_states[view.index()];
        if *slot == state {
            return false;
        }
        *slot = state;
        true
    }

    fn set_many_view_load_states(&mut self, views: &[AppView], state: ViewLoadState) -> bool {
        views.iter().fold(false, |changed, &view| {
            self.set_view_load_state(view, state) || changed
        })
    }

    fn mark_views_requested(&mut self, views: &[AppView]) -> bool {
        views.iter().fold(false, |changed, &view| {
            let next = if self.has_view_data(view) {
                ViewLoadState::Refreshing
            } else {
                ViewLoadState::Loading
            };
            self.set_view_load_state(view, next) || changed
        })
    }

    pub fn mark_refresh_requested(&mut self, options: RefreshOptions) {
        let mut changed = false;
        changed |= self.mark_views_requested(CORE_REFRESH_VIEWS);
        if options.include_secondary_resources || !self.snapshot.secondary_resources_loaded {
            changed |= self.mark_views_requested(SECONDARY_REFRESH_VIEWS);
        }
        if options.include_flux {
            changed |= self.mark_views_requested(FLUX_REFRESH_VIEWS);
        }
        if options.include_events {
            changed |= self.mark_views_requested(EVENT_REFRESH_VIEWS);
        }
        changed |= self.set_view_load_state(AppView::PortForwarding, ViewLoadState::Ready);

        if changed {
            self.snapshot_dirty = true;
            self.publish_snapshot();
        }
    }

    fn mark_refresh_completed(&mut self, options: RefreshOptions) {
        let mut changed = false;
        changed |= self.set_many_view_load_states(CORE_REFRESH_VIEWS, ViewLoadState::Ready);
        if options.include_secondary_resources {
            changed |=
                self.set_many_view_load_states(SECONDARY_REFRESH_VIEWS, ViewLoadState::Ready);
        }
        if options.include_flux {
            changed |= self.set_many_view_load_states(FLUX_REFRESH_VIEWS, ViewLoadState::Ready);
        }
        if options.include_events {
            changed |= self.set_many_view_load_states(EVENT_REFRESH_VIEWS, ViewLoadState::Ready);
        }
        changed |= self.set_view_load_state(AppView::PortForwarding, ViewLoadState::Ready);

        if changed {
            self.snapshot_dirty = true;
        }
    }

    /// Resets rendered resource data for a scope transition (namespace/context)
    /// while keeping snapshot revisions monotonic so render caches cannot
    /// accidentally reuse stale entries after rapid switches.
    pub fn begin_loading_transition(&mut self, clear_namespaces: bool) {
        let next_snapshot_version = self.snapshot.snapshot_version.saturating_add(1);
        let cluster_url = self.snapshot.cluster_url.clone();
        self.snapshot = ClusterSnapshot {
            snapshot_version: next_snapshot_version,
            phase: DataPhase::Idle,
            cluster_url,
            ..ClusterSnapshot::default()
        };
        if clear_namespaces {
            self.namespaces.clear();
        }
        self.snapshot_dirty = true;
        self.publish_snapshot();
    }

    /// Per-resource fetch timeout in seconds.
    const FETCH_TIMEOUT_SECS: u64 = 10;
    /// Retry transient transport failures before surfacing errors.
    const TRANSIENT_RETRY_ATTEMPTS: usize = 3;
    const TRANSIENT_RETRY_DELAY_MS: u64 = 150;

    async fn fetch_with_timeout<T, F, Fut>(label: &'static str, make_fut: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        for attempt in 0..=Self::TRANSIENT_RETRY_ATTEMPTS {
            let _permit = RESOURCE_FETCH_SEMAPHORE
                .acquire()
                .await
                .map_err(|_| anyhow!("resource fetch coordinator shut down"))?;
            match tokio::time::timeout(Duration::from_secs(Self::FETCH_TIMEOUT_SECS), make_fut())
                .await
            {
                Ok(Ok(value)) => return Ok(value),
                Ok(Err(err)) => {
                    if attempt < Self::TRANSIENT_RETRY_ATTEMPTS
                        && Self::is_transient_send_request_error(&err)
                    {
                        drop(_permit);
                        tokio::time::sleep(Duration::from_millis(Self::TRANSIENT_RETRY_DELAY_MS))
                            .await;
                        continue;
                    }
                    return Err(err);
                }
                Err(_) => {
                    if attempt < Self::TRANSIENT_RETRY_ATTEMPTS {
                        drop(_permit);
                        tokio::time::sleep(Duration::from_millis(Self::TRANSIENT_RETRY_DELAY_MS))
                            .await;
                        continue;
                    }
                    return Err(anyhow!(
                        "timed out fetching {label} ({}s)",
                        Self::FETCH_TIMEOUT_SECS
                    ));
                }
            }
        }
        unreachable!()
    }

    fn is_transient_send_request_error(err: &anyhow::Error) -> bool {
        err.chain().any(|cause| {
            if let Some(io_err) = cause.downcast_ref::<std::io::Error>() {
                return matches!(
                    io_err.kind(),
                    std::io::ErrorKind::ConnectionRefused
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::TimedOut
                );
            }
            let text = cause.to_string();
            text.contains("SendRequest")
                || text.contains("Connection refused")
                || text.contains("connection reset")
                || text.contains("connection closed")
                || text.contains("broken pipe")
                || text.contains("timed out sending request")
        })
    }

    fn keep_prev_vec_on_error<T: Clone>(
        result: Result<Vec<T>>,
        previous: &[T],
        label: &str,
        errors: &mut Vec<String>,
    ) -> Vec<T> {
        match result {
            Ok(items) => items,
            Err(err) => {
                errors.push(format!("{label}: {err}"));
                previous.to_vec()
            }
        }
    }

    fn filter_namespace<T, F>(items: Vec<T>, namespace: Option<&str>, namespace_of: F) -> Vec<T>
    where
        F: Fn(&T) -> &str,
    {
        match namespace {
            Some(ns) => items
                .into_iter()
                .filter(|item| namespace_of(item) == ns)
                .collect(),
            None => items,
        }
    }

    fn namespace_names_from_list(namespace_list: &[NamespaceInfo]) -> Vec<String> {
        let mut names: Vec<String> = namespace_list
            .iter()
            .map(|ns| ns.name.clone())
            .filter(|name| !name.is_empty())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Refreshes core resources in parallel, updating status and timestamps.
    ///
    /// Production hardening behavior:
    /// - Per-resource timeout protection (10s)
    /// - Global refresh timeout (60s) prevents indefinite hangs
    /// - Graceful degradation for partial API failures
    /// - Returns error only when all critical resources fail
    pub async fn refresh<D>(&mut self, client: &D, namespace: Option<&str>) -> Result<()>
    where
        D: ClusterDataSource + Sync,
    {
        self.refresh_with_options(client, namespace, RefreshOptions::default())
            .await
    }

    /// Refreshes core resources with runtime options for expensive view-specific data.
    pub async fn refresh_with_options<D>(
        &mut self,
        client: &D,
        namespace: Option<&str>,
        options: RefreshOptions,
    ) -> Result<()>
    where
        D: ClusterDataSource + Sync,
    {
        match tokio::time::timeout(
            Duration::from_secs(60),
            self.refresh_inner(client, namespace, options),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                self.snapshot.phase = DataPhase::Error;
                self.snapshot.last_error = Some("Global refresh timed out (60s)".to_string());
                self.snapshot_dirty = true;
                self.publish_snapshot();
                Err(anyhow!("Global refresh timed out (60s)"))
            }
        }
    }

    async fn refresh_inner<D>(
        &mut self,
        client: &D,
        namespace: Option<&str>,
        options: RefreshOptions,
    ) -> Result<()>
    where
        D: ClusterDataSource + Sync,
    {
        if self.snapshot.phase == DataPhase::Loading {
            return Ok(());
        }

        self.snapshot.phase = DataPhase::Loading;
        self.snapshot.last_error = None;
        self.snapshot.cluster_url = Some(client.cluster_url().to_string());

        let prev_flux_resources = self.snapshot.flux_resources.clone();
        let prev_cluster_info = self.snapshot.cluster_info.clone();
        let prev_events = self.snapshot.events.clone();
        let include_flux = options.include_flux;
        let include_cluster_info = options.include_cluster_info;
        let include_secondary_resources = options.include_secondary_resources;
        let include_events = options.include_events;
        let flux_fetch = async move {
            if include_flux {
                Self::fetch_with_timeout("fluxresources", || client.fetch_flux_resources(namespace))
                    .await
            } else {
                Ok(prev_flux_resources)
            }
        };
        let cluster_info_fetch = async move {
            if include_cluster_info {
                Self::fetch_with_timeout("cluster info", || client.fetch_cluster_info())
                    .await
                    .map(Some)
            } else {
                Ok(None)
            }
        };
        let events_fetch = async move {
            if include_events {
                Self::fetch_with_timeout("events", || client.fetch_events(namespace)).await
            } else {
                Ok(prev_events)
            }
        };

        let (
            nodes_res,
            pods_res,
            services_res,
            deployments_res,
            statefulsets_res,
            daemonsets_res,
            replicasets_res,
            replication_controllers_res,
            jobs_res,
            cronjobs_res,
            namespace_list_res,
            flux_resources_res,
            cluster_info_res,
            events_res,
        ) = tokio::join!(
            Self::fetch_with_timeout("nodes", || client.fetch_nodes()),
            Self::fetch_with_timeout("pods", || client.fetch_pods(namespace)),
            Self::fetch_with_timeout("services", || client.fetch_services(namespace)),
            Self::fetch_with_timeout("deployments", || client.fetch_deployments(namespace)),
            Self::fetch_with_timeout("statefulsets", || client.fetch_statefulsets(namespace)),
            Self::fetch_with_timeout("daemonsets", || client.fetch_daemonsets(namespace)),
            Self::fetch_with_timeout("replicasets", || client.fetch_replicasets(namespace)),
            Self::fetch_with_timeout("replicationcontrollers", || client
                .fetch_replication_controllers(namespace)),
            Self::fetch_with_timeout("jobs", || client.fetch_jobs(namespace)),
            Self::fetch_with_timeout("cronjobs", || client.fetch_cronjobs(namespace)),
            Self::fetch_with_timeout("namespacelist", || client.fetch_namespace_list()),
            flux_fetch,
            cluster_info_fetch,
            events_fetch,
        );

        let (
            resource_quotas_res,
            limit_ranges_res,
            pod_disruption_budgets_res,
            service_accounts_res,
            roles_res,
            role_bindings_res,
            cluster_roles_res,
            cluster_role_bindings_res,
            custom_resource_definitions_res,
            endpoints_res,
            ingresses_res,
            ingress_classes_res,
            network_policies_res,
            config_maps_res,
            secrets_res,
            hpas_res,
            pvcs_res,
            pvs_res,
            storage_classes_res,
            priority_classes_res,
            helm_releases_res,
            node_metrics_res,
        ) = if include_secondary_resources {
            tokio::join!(
                Self::fetch_with_timeout("resourcequotas", || client
                    .fetch_resource_quotas(namespace)),
                Self::fetch_with_timeout("limitranges", || client.fetch_limit_ranges(namespace)),
                Self::fetch_with_timeout("pdbs", || client.fetch_pod_disruption_budgets(namespace)),
                Self::fetch_with_timeout("serviceaccounts", || client
                    .fetch_service_accounts(namespace)),
                Self::fetch_with_timeout("roles", || client.fetch_roles(namespace)),
                Self::fetch_with_timeout("rolebindings", || client.fetch_role_bindings(namespace)),
                Self::fetch_with_timeout("clusterroles", || client.fetch_cluster_roles()),
                Self::fetch_with_timeout("clusterrolebindings", || client
                    .fetch_cluster_role_bindings()),
                Self::fetch_with_timeout("crds", || client.fetch_custom_resource_definitions()),
                Self::fetch_with_timeout("endpoints", || client.fetch_endpoints(namespace)),
                Self::fetch_with_timeout("ingresses", || client.fetch_ingresses(namespace)),
                Self::fetch_with_timeout("ingressclasses", || client.fetch_ingress_classes()),
                Self::fetch_with_timeout("networkpolicies", || client
                    .fetch_network_policies(namespace)),
                Self::fetch_with_timeout("configmaps", || client.fetch_config_maps(namespace)),
                Self::fetch_with_timeout("secrets", || client.fetch_secrets(namespace)),
                Self::fetch_with_timeout("hpas", || client.fetch_hpas(namespace)),
                Self::fetch_with_timeout("pvcs", || client.fetch_pvcs(namespace)),
                Self::fetch_with_timeout("pvs", || client.fetch_pvs()),
                Self::fetch_with_timeout("storageclasses", || client.fetch_storage_classes()),
                Self::fetch_with_timeout("priorityclasses", || client.fetch_priority_classes()),
                Self::fetch_with_timeout("helmreleases", || client.fetch_helm_releases(namespace)),
                Self::fetch_with_timeout("nodemetrics", || client.fetch_all_node_metrics()),
            )
        } else {
            (
                Ok(self.snapshot.resource_quotas.clone()),
                Ok(self.snapshot.limit_ranges.clone()),
                Ok(self.snapshot.pod_disruption_budgets.clone()),
                Ok(self.snapshot.service_accounts.clone()),
                Ok(self.snapshot.roles.clone()),
                Ok(self.snapshot.role_bindings.clone()),
                Ok(self.snapshot.cluster_roles.clone()),
                Ok(self.snapshot.cluster_role_bindings.clone()),
                Ok(self.snapshot.custom_resource_definitions.clone()),
                Ok(self.snapshot.endpoints.clone()),
                Ok(self.snapshot.ingresses.clone()),
                Ok(self.snapshot.ingress_classes.clone()),
                Ok(self.snapshot.network_policies.clone()),
                Ok(self.snapshot.config_maps.clone()),
                Ok(self.snapshot.secrets.clone()),
                Ok(self.snapshot.hpas.clone()),
                Ok(self.snapshot.pvcs.clone()),
                Ok(self.snapshot.pvs.clone()),
                Ok(self.snapshot.storage_classes.clone()),
                Ok(self.snapshot.priority_classes.clone()),
                Ok(self.snapshot.helm_releases.clone()),
                Ok(self.snapshot.node_metrics.clone()),
            )
        };

        let mut errors = Vec::new();

        let nodes =
            Self::keep_prev_vec_on_error(nodes_res, &self.snapshot.nodes, "nodes", &mut errors);
        let pods = Self::keep_prev_vec_on_error(pods_res, &self.snapshot.pods, "pods", &mut errors);
        let services = Self::keep_prev_vec_on_error(
            services_res,
            &self.snapshot.services,
            "services",
            &mut errors,
        );
        let deployments = Self::keep_prev_vec_on_error(
            deployments_res,
            &self.snapshot.deployments,
            "deployments",
            &mut errors,
        );
        let statefulsets = Self::keep_prev_vec_on_error(
            statefulsets_res,
            &self.snapshot.statefulsets,
            "statefulsets",
            &mut errors,
        );
        let daemonsets = Self::keep_prev_vec_on_error(
            daemonsets_res,
            &self.snapshot.daemonsets,
            "daemonsets",
            &mut errors,
        );
        let replicasets = Self::keep_prev_vec_on_error(
            replicasets_res,
            &self.snapshot.replicasets,
            "replicasets",
            &mut errors,
        );
        let replication_controllers = Self::keep_prev_vec_on_error(
            replication_controllers_res,
            &self.snapshot.replication_controllers,
            "replicationcontrollers",
            &mut errors,
        );
        let jobs = Self::keep_prev_vec_on_error(jobs_res, &self.snapshot.jobs, "jobs", &mut errors);
        let cronjobs = Self::keep_prev_vec_on_error(
            cronjobs_res,
            &self.snapshot.cronjobs,
            "cronjobs",
            &mut errors,
        );
        let resource_quotas = Self::keep_prev_vec_on_error(
            resource_quotas_res,
            &self.snapshot.resource_quotas,
            "resourcequotas",
            &mut errors,
        );
        let limit_ranges = Self::keep_prev_vec_on_error(
            limit_ranges_res,
            &self.snapshot.limit_ranges,
            "limitranges",
            &mut errors,
        );
        let pod_disruption_budgets = Self::keep_prev_vec_on_error(
            pod_disruption_budgets_res,
            &self.snapshot.pod_disruption_budgets,
            "pdbs",
            &mut errors,
        );
        let service_accounts = Self::keep_prev_vec_on_error(
            service_accounts_res,
            &self.snapshot.service_accounts,
            "serviceaccounts",
            &mut errors,
        );
        let roles =
            Self::keep_prev_vec_on_error(roles_res, &self.snapshot.roles, "roles", &mut errors);
        let role_bindings = Self::keep_prev_vec_on_error(
            role_bindings_res,
            &self.snapshot.role_bindings,
            "rolebindings",
            &mut errors,
        );
        let cluster_roles = Self::keep_prev_vec_on_error(
            cluster_roles_res,
            &self.snapshot.cluster_roles,
            "clusterroles",
            &mut errors,
        );
        let cluster_role_bindings = Self::keep_prev_vec_on_error(
            cluster_role_bindings_res,
            &self.snapshot.cluster_role_bindings,
            "clusterrolebindings",
            &mut errors,
        );
        let custom_resource_definitions = Self::keep_prev_vec_on_error(
            custom_resource_definitions_res,
            &self.snapshot.custom_resource_definitions,
            "crds",
            &mut errors,
        );
        let cluster_info = match cluster_info_res {
            Ok(Some(info)) => Some(info),
            Ok(None) => prev_cluster_info,
            Err(err) => {
                errors.push(format!("cluster info: {err}"));
                prev_cluster_info
            }
        };

        let endpoints = Self::keep_prev_vec_on_error(
            endpoints_res,
            &self.snapshot.endpoints,
            "endpoints",
            &mut errors,
        );
        let ingresses = Self::keep_prev_vec_on_error(
            ingresses_res,
            &self.snapshot.ingresses,
            "ingresses",
            &mut errors,
        );
        let ingress_classes = Self::keep_prev_vec_on_error(
            ingress_classes_res,
            &self.snapshot.ingress_classes,
            "ingressclasses",
            &mut errors,
        );
        let network_policies = Self::keep_prev_vec_on_error(
            network_policies_res,
            &self.snapshot.network_policies,
            "networkpolicies",
            &mut errors,
        );
        let config_maps = Self::keep_prev_vec_on_error(
            config_maps_res,
            &self.snapshot.config_maps,
            "configmaps",
            &mut errors,
        );
        let secrets = Self::keep_prev_vec_on_error(
            secrets_res,
            &self.snapshot.secrets,
            "secrets",
            &mut errors,
        );
        let hpas = Self::keep_prev_vec_on_error(hpas_res, &self.snapshot.hpas, "hpas", &mut errors);
        let pvcs = Self::keep_prev_vec_on_error(pvcs_res, &self.snapshot.pvcs, "pvcs", &mut errors);
        let pvs = Self::keep_prev_vec_on_error(pvs_res, &self.snapshot.pvs, "pvs", &mut errors);
        let storage_classes = Self::keep_prev_vec_on_error(
            storage_classes_res,
            &self.snapshot.storage_classes,
            "storageclasses",
            &mut errors,
        );
        let namespace_list = Self::keep_prev_vec_on_error(
            namespace_list_res,
            &self.snapshot.namespace_list,
            "namespacelist",
            &mut errors,
        );
        self.namespaces = Self::namespace_names_from_list(&namespace_list);
        let events =
            Self::keep_prev_vec_on_error(events_res, &self.snapshot.events, "events", &mut errors);
        let priority_classes = Self::keep_prev_vec_on_error(
            priority_classes_res,
            &self.snapshot.priority_classes,
            "priorityclasses",
            &mut errors,
        );
        let helm_releases = Self::filter_namespace(
            Self::keep_prev_vec_on_error(
                helm_releases_res,
                &self.snapshot.helm_releases,
                "helmreleases",
                &mut errors,
            ),
            namespace,
            |release| release.namespace.as_str(),
        );
        let flux_resources = Self::keep_prev_vec_on_error(
            flux_resources_res,
            &self.snapshot.flux_resources,
            "fluxresources",
            &mut errors,
        );
        let node_metrics = Self::keep_prev_vec_on_error(
            node_metrics_res,
            &self.snapshot.node_metrics,
            "nodemetrics",
            &mut errors,
        );

        let all_failed = nodes.is_empty()
            && pods.is_empty()
            && services.is_empty()
            && deployments.is_empty()
            && statefulsets.is_empty()
            && daemonsets.is_empty()
            && replicasets.is_empty()
            && replication_controllers.is_empty()
            && jobs.is_empty()
            && cronjobs.is_empty()
            && cluster_info.is_none();

        if all_failed {
            let message = if errors.is_empty() {
                "failed to refresh cluster state".to_string()
            } else {
                errors.join(" | ")
            };
            self.snapshot.phase = DataPhase::Error;
            self.snapshot.last_error = Some(message.clone());
            self.snapshot_dirty = true;
            self.publish_snapshot();
            return Err(anyhow!(message));
        }

        let namespaces_count = pods
            .iter()
            .map(|pod| pod.namespace.as_str())
            .chain(services.iter().map(|service| service.namespace.as_str()))
            .chain(
                deployments
                    .iter()
                    .map(|deployment| deployment.namespace.as_str()),
            )
            .collect::<HashSet<_>>()
            .len();

        self.snapshot.services_count = services.len();
        self.snapshot.namespaces_count = namespaces_count;
        self.snapshot.nodes = nodes;
        self.snapshot.pods = pods;
        self.snapshot.services = services;
        self.snapshot.deployments = deployments;
        self.snapshot.statefulsets = statefulsets;
        self.snapshot.daemonsets = daemonsets;
        self.snapshot.replicasets = replicasets;
        self.snapshot.replication_controllers = replication_controllers;
        self.snapshot.jobs = jobs;
        self.snapshot.cronjobs = cronjobs;
        self.snapshot.resource_quotas = resource_quotas;
        self.snapshot.limit_ranges = limit_ranges;
        self.snapshot.pod_disruption_budgets = pod_disruption_budgets;
        self.snapshot.service_accounts = service_accounts;
        self.snapshot.roles = roles;
        self.snapshot.role_bindings = role_bindings;
        self.snapshot.cluster_roles = cluster_roles;
        self.snapshot.cluster_role_bindings = cluster_role_bindings;
        self.snapshot.custom_resource_definitions = custom_resource_definitions;
        self.snapshot.cluster_info = cluster_info;
        self.snapshot.endpoints = endpoints;
        self.snapshot.ingresses = ingresses;
        self.snapshot.ingress_classes = ingress_classes;
        self.snapshot.network_policies = network_policies;
        self.snapshot.config_maps = config_maps;
        self.snapshot.secrets = secrets;
        self.snapshot.hpas = hpas;
        self.snapshot.pvcs = pvcs;
        self.snapshot.pvs = pvs;
        self.snapshot.storage_classes = storage_classes;
        self.snapshot.namespace_list = namespace_list;
        self.snapshot.events = events;
        self.snapshot.priority_classes = priority_classes;
        self.snapshot.helm_releases = helm_releases;
        self.snapshot.flux_resources = flux_resources;
        self.snapshot.helm_repositories = crate::k8s::helm::read_helm_repositories();
        self.snapshot.node_metrics = node_metrics;
        self.snapshot.secondary_resources_loaded = if include_secondary_resources {
            true
        } else {
            self.snapshot.secondary_resources_loaded
        };
        self.mark_refresh_completed(options);
        self.snapshot.snapshot_version = self.snapshot.snapshot_version.saturating_add(1);
        self.snapshot_dirty = true;
        self.snapshot.phase = DataPhase::Ready;
        self.snapshot.last_updated = Some(Utc::now());
        self.snapshot.last_error = if errors.is_empty() {
            None
        } else {
            Some(errors.join(" | "))
        };

        self.publish_snapshot();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct MockDataSource {
        url: String,
        nodes: Vec<NodeInfo>,
        namespaces: Vec<String>,
        pods: Vec<PodInfo>,
        services: Vec<ServiceInfo>,
        deployments: Vec<DeploymentInfo>,
        statefulsets: Vec<StatefulSetInfo>,
        daemonsets: Vec<DaemonSetInfo>,
        replicasets: Vec<ReplicaSetInfo>,
        replication_controllers: Vec<ReplicationControllerInfo>,
        jobs: Vec<JobInfo>,
        cronjobs: Vec<CronJobInfo>,
        resource_quotas: Vec<ResourceQuotaInfo>,
        limit_ranges: Vec<LimitRangeInfo>,
        pod_disruption_budgets: Vec<PodDisruptionBudgetInfo>,
        service_accounts: Vec<ServiceAccountInfo>,
        roles: Vec<RoleInfo>,
        role_bindings: Vec<RoleBindingInfo>,
        cluster_roles: Vec<ClusterRoleInfo>,
        cluster_role_bindings: Vec<ClusterRoleBindingInfo>,
        custom_resource_definitions: Vec<CustomResourceDefinitionInfo>,
        flux_resources: Vec<FluxResourceInfo>,
        helm_releases: Vec<HelmReleaseInfo>,
        helm_releases_ignore_namespace: bool,
        cluster_info: Option<ClusterInfo>,
        nodes_err: Option<String>,
        pods_err: Option<String>,
        services_err: Option<String>,
        deployments_err: Option<String>,
        statefulsets_err: Option<String>,
        daemonsets_err: Option<String>,
        jobs_err: Option<String>,
        cronjobs_err: Option<String>,
        resource_quotas_err: Option<String>,
        limit_ranges_err: Option<String>,
        pod_disruption_budgets_err: Option<String>,
        service_accounts_err: Option<String>,
        roles_err: Option<String>,
        role_bindings_err: Option<String>,
        cluster_roles_err: Option<String>,
        cluster_role_bindings_err: Option<String>,
        cluster_info_err: Option<String>,
        delay_ms: u64,
    }

    impl MockDataSource {
        fn success() -> Self {
            Self {
                url: "https://kind.local".to_string(),
                nodes: vec![NodeInfo {
                    name: "n1".to_string(),
                    ready: true,
                    ..NodeInfo::default()
                }],
                namespaces: vec!["default".to_string(), "demo".to_string()],
                pods: vec![
                    PodInfo {
                        name: "p1".to_string(),
                        namespace: "default".to_string(),
                        ..PodInfo::default()
                    },
                    PodInfo {
                        name: "p2".to_string(),
                        namespace: "demo".to_string(),
                        ..PodInfo::default()
                    },
                ],
                services: vec![ServiceInfo {
                    name: "svc".to_string(),
                    namespace: "default".to_string(),
                    ..ServiceInfo::default()
                }],
                deployments: vec![DeploymentInfo {
                    name: "dep".to_string(),
                    namespace: "default".to_string(),
                    ready: "1/1".to_string(),
                    ..DeploymentInfo::default()
                }],
                statefulsets: vec![StatefulSetInfo {
                    name: "db".to_string(),
                    namespace: "default".to_string(),
                    desired_replicas: 1,
                    ready_replicas: 1,
                    service_name: "db-headless".to_string(),
                    pod_management_policy: "OrderedReady".to_string(),
                    ..StatefulSetInfo::default()
                }],
                daemonsets: vec![DaemonSetInfo {
                    name: "agent".to_string(),
                    namespace: "kube-system".to_string(),
                    desired_count: 1,
                    ready_count: 1,
                    unavailable_count: 0,
                    ..DaemonSetInfo::default()
                }],
                replicasets: Vec::new(),
                replication_controllers: Vec::new(),
                jobs: vec![JobInfo {
                    name: "cleanup".to_string(),
                    namespace: "default".to_string(),
                    status: "Running".to_string(),
                    completions: "0/1".to_string(),
                    parallelism: 1,
                    active_pods: 1,
                    ..JobInfo::default()
                }],
                cronjobs: vec![CronJobInfo {
                    name: "nightly".to_string(),
                    namespace: "default".to_string(),
                    schedule: "0 0 * * *".to_string(),
                    active_jobs: 0,
                    ..CronJobInfo::default()
                }],
                resource_quotas: vec![ResourceQuotaInfo {
                    name: "rq-default".to_string(),
                    namespace: "default".to_string(),
                    ..ResourceQuotaInfo::default()
                }],
                limit_ranges: vec![LimitRangeInfo {
                    name: "limits-default".to_string(),
                    namespace: "default".to_string(),
                    ..LimitRangeInfo::default()
                }],
                pod_disruption_budgets: vec![PodDisruptionBudgetInfo {
                    name: "web-pdb".to_string(),
                    namespace: "default".to_string(),
                    disruptions_allowed: 1,
                    ..PodDisruptionBudgetInfo::default()
                }],
                service_accounts: vec![ServiceAccountInfo {
                    name: "default".to_string(),
                    namespace: "default".to_string(),
                    secrets_count: 1,
                    image_pull_secrets_count: 0,
                    ..ServiceAccountInfo::default()
                }],
                roles: vec![RoleInfo {
                    name: "reader".to_string(),
                    namespace: "default".to_string(),
                    ..RoleInfo::default()
                }],
                role_bindings: vec![RoleBindingInfo {
                    name: "reader-binding".to_string(),
                    namespace: "default".to_string(),
                    role_ref_kind: "Role".to_string(),
                    role_ref_name: "reader".to_string(),
                    ..RoleBindingInfo::default()
                }],
                cluster_roles: vec![ClusterRoleInfo {
                    name: "cluster-admin".to_string(),
                    ..ClusterRoleInfo::default()
                }],
                cluster_role_bindings: vec![ClusterRoleBindingInfo {
                    name: "cluster-admin-binding".to_string(),
                    role_ref_kind: "ClusterRole".to_string(),
                    role_ref_name: "cluster-admin".to_string(),
                    ..ClusterRoleBindingInfo::default()
                }],
                custom_resource_definitions: vec![CustomResourceDefinitionInfo {
                    name: "widgets.demo.io".to_string(),
                    group: "demo.io".to_string(),
                    version: "v1".to_string(),
                    kind: "Widget".to_string(),
                    plural: "widgets".to_string(),
                    scope: "Namespaced".to_string(),
                    instances: 1,
                }],
                flux_resources: vec![],
                helm_releases: vec![
                    HelmReleaseInfo {
                        name: "api".to_string(),
                        namespace: "default".to_string(),
                        status: "deployed".to_string(),
                        chart: "api".to_string(),
                        ..HelmReleaseInfo::default()
                    },
                    HelmReleaseInfo {
                        name: "api".to_string(),
                        namespace: "demo".to_string(),
                        status: "deployed".to_string(),
                        chart: "api".to_string(),
                        ..HelmReleaseInfo::default()
                    },
                ],
                helm_releases_ignore_namespace: false,
                cluster_info: Some(ClusterInfo {
                    server: "https://kind.local".to_string(),
                    node_count: 1,
                    ready_nodes: 1,
                    pod_count: 2,
                    ..ClusterInfo::default()
                }),
                nodes_err: None,
                pods_err: None,
                services_err: None,
                deployments_err: None,
                statefulsets_err: None,
                daemonsets_err: None,
                jobs_err: None,
                cronjobs_err: None,
                resource_quotas_err: None,
                limit_ranges_err: None,
                pod_disruption_budgets_err: None,
                service_accounts_err: None,
                roles_err: None,
                role_bindings_err: None,
                cluster_roles_err: None,
                cluster_role_bindings_err: None,
                cluster_info_err: None,
                delay_ms: 0,
            }
        }

        fn with_delay(mut self, delay_ms: u64) -> Self {
            self.delay_ms = delay_ms;
            self
        }

        fn filter_namespace<T: Clone, F>(
            items: &[T],
            namespace: Option<&str>,
            namespace_of: F,
        ) -> Vec<T>
        where
            F: Fn(&T) -> &str,
        {
            match namespace {
                Some(ns) => items
                    .iter()
                    .filter(|item| namespace_of(item) == ns)
                    .cloned()
                    .collect(),
                None => items.to_vec(),
            }
        }
    }

    #[async_trait]
    impl ClusterDataSource for MockDataSource {
        fn cluster_url(&self) -> &str {
            &self.url
        }

        async fn fetch_nodes(&self) -> Result<Vec<NodeInfo>> {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.nodes_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(self.nodes.clone())
        }

        async fn fetch_namespaces(&self) -> Result<Vec<String>> {
            Ok(self.namespaces.clone())
        }

        async fn fetch_pods(&self, namespace: Option<&str>) -> Result<Vec<PodInfo>> {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.pods_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(Self::filter_namespace(&self.pods, namespace, |pod| {
                &pod.namespace
            }))
        }

        async fn fetch_services(&self, namespace: Option<&str>) -> Result<Vec<ServiceInfo>> {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.services_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(Self::filter_namespace(
                &self.services,
                namespace,
                |service| &service.namespace,
            ))
        }

        async fn fetch_deployments(&self, namespace: Option<&str>) -> Result<Vec<DeploymentInfo>> {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.deployments_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(Self::filter_namespace(
                &self.deployments,
                namespace,
                |deployment| &deployment.namespace,
            ))
        }

        async fn fetch_statefulsets(
            &self,
            namespace: Option<&str>,
        ) -> Result<Vec<StatefulSetInfo>> {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.statefulsets_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(Self::filter_namespace(
                &self.statefulsets,
                namespace,
                |set| &set.namespace,
            ))
        }

        async fn fetch_daemonsets(&self, namespace: Option<&str>) -> Result<Vec<DaemonSetInfo>> {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.daemonsets_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(Self::filter_namespace(&self.daemonsets, namespace, |set| {
                &set.namespace
            }))
        }

        async fn fetch_replicasets(&self, namespace: Option<&str>) -> Result<Vec<ReplicaSetInfo>> {
            Ok(Self::filter_namespace(
                &self.replicasets,
                namespace,
                |set| &set.namespace,
            ))
        }

        async fn fetch_replication_controllers(
            &self,
            namespace: Option<&str>,
        ) -> Result<Vec<ReplicationControllerInfo>> {
            Ok(Self::filter_namespace(
                &self.replication_controllers,
                namespace,
                |controller| &controller.namespace,
            ))
        }

        async fn fetch_jobs(&self, namespace: Option<&str>) -> Result<Vec<JobInfo>> {
            if let Some(err) = &self.jobs_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(Self::filter_namespace(&self.jobs, namespace, |job| {
                &job.namespace
            }))
        }

        async fn fetch_cronjobs(&self, namespace: Option<&str>) -> Result<Vec<CronJobInfo>> {
            if let Some(err) = &self.cronjobs_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(Self::filter_namespace(&self.cronjobs, namespace, |job| {
                &job.namespace
            }))
        }

        async fn fetch_resource_quotas(
            &self,
            namespace: Option<&str>,
        ) -> Result<Vec<ResourceQuotaInfo>> {
            if let Some(err) = &self.resource_quotas_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(Self::filter_namespace(
                &self.resource_quotas,
                namespace,
                |quota| &quota.namespace,
            ))
        }

        async fn fetch_limit_ranges(&self, namespace: Option<&str>) -> Result<Vec<LimitRangeInfo>> {
            if let Some(err) = &self.limit_ranges_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(Self::filter_namespace(
                &self.limit_ranges,
                namespace,
                |limit| &limit.namespace,
            ))
        }

        async fn fetch_pod_disruption_budgets(
            &self,
            namespace: Option<&str>,
        ) -> Result<Vec<PodDisruptionBudgetInfo>> {
            if let Some(err) = &self.pod_disruption_budgets_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(Self::filter_namespace(
                &self.pod_disruption_budgets,
                namespace,
                |pdb| &pdb.namespace,
            ))
        }

        async fn fetch_service_accounts(
            &self,
            namespace: Option<&str>,
        ) -> Result<Vec<ServiceAccountInfo>> {
            if let Some(err) = &self.service_accounts_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(Self::filter_namespace(
                &self.service_accounts,
                namespace,
                |account| &account.namespace,
            ))
        }

        async fn fetch_roles(&self, namespace: Option<&str>) -> Result<Vec<RoleInfo>> {
            if let Some(err) = &self.roles_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(Self::filter_namespace(&self.roles, namespace, |role| {
                &role.namespace
            }))
        }

        async fn fetch_role_bindings(
            &self,
            namespace: Option<&str>,
        ) -> Result<Vec<RoleBindingInfo>> {
            if let Some(err) = &self.role_bindings_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(Self::filter_namespace(
                &self.role_bindings,
                namespace,
                |binding| &binding.namespace,
            ))
        }

        async fn fetch_cluster_roles(&self) -> Result<Vec<ClusterRoleInfo>> {
            if let Some(err) = &self.cluster_roles_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(self.cluster_roles.clone())
        }

        async fn fetch_cluster_role_bindings(&self) -> Result<Vec<ClusterRoleBindingInfo>> {
            if let Some(err) = &self.cluster_role_bindings_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(self.cluster_role_bindings.clone())
        }

        async fn fetch_custom_resource_definitions(
            &self,
        ) -> Result<Vec<CustomResourceDefinitionInfo>> {
            Ok(self.custom_resource_definitions.clone())
        }

        async fn fetch_cluster_info(&self) -> Result<ClusterInfo> {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.cluster_info_err {
                return Err(anyhow!(err.clone()));
            }
            self.cluster_info
                .clone()
                .ok_or_else(|| anyhow!("cluster info missing"))
        }

        async fn fetch_endpoints(&self, _namespace: Option<&str>) -> Result<Vec<EndpointInfo>> {
            Ok(vec![])
        }
        async fn fetch_ingresses(&self, _namespace: Option<&str>) -> Result<Vec<IngressInfo>> {
            Ok(vec![])
        }
        async fn fetch_ingress_classes(&self) -> Result<Vec<IngressClassInfo>> {
            Ok(vec![])
        }
        async fn fetch_network_policies(
            &self,
            _namespace: Option<&str>,
        ) -> Result<Vec<NetworkPolicyInfo>> {
            Ok(vec![])
        }
        async fn fetch_config_maps(&self, _namespace: Option<&str>) -> Result<Vec<ConfigMapInfo>> {
            Ok(vec![])
        }
        async fn fetch_secrets(&self, _namespace: Option<&str>) -> Result<Vec<SecretInfo>> {
            Ok(vec![])
        }
        async fn fetch_hpas(&self, _namespace: Option<&str>) -> Result<Vec<HpaInfo>> {
            Ok(vec![])
        }
        async fn fetch_pvcs(&self, _namespace: Option<&str>) -> Result<Vec<PvcInfo>> {
            Ok(vec![])
        }
        async fn fetch_pvs(&self) -> Result<Vec<PvInfo>> {
            Ok(vec![])
        }
        async fn fetch_storage_classes(&self) -> Result<Vec<StorageClassInfo>> {
            Ok(vec![])
        }
        async fn fetch_namespace_list(&self) -> Result<Vec<NamespaceInfo>> {
            Ok(self
                .namespaces
                .iter()
                .map(|name| NamespaceInfo {
                    name: name.clone(),
                    status: "Active".to_string(),
                    ..NamespaceInfo::default()
                })
                .collect())
        }
        async fn fetch_events(&self, _namespace: Option<&str>) -> Result<Vec<K8sEventInfo>> {
            Ok(vec![])
        }
        async fn fetch_priority_classes(&self) -> Result<Vec<PriorityClassInfo>> {
            Ok(vec![])
        }
        async fn fetch_helm_releases(
            &self,
            namespace: Option<&str>,
        ) -> Result<Vec<HelmReleaseInfo>> {
            if self.helm_releases_ignore_namespace {
                return Ok(self.helm_releases.clone());
            }
            Ok(Self::filter_namespace(
                &self.helm_releases,
                namespace,
                |release| &release.namespace,
            ))
        }
        async fn fetch_flux_resources(
            &self,
            namespace: Option<&str>,
        ) -> Result<Vec<FluxResourceInfo>> {
            Ok(Self::filter_namespace(
                &self.flux_resources,
                namespace,
                |resource| resource.namespace.as_deref().unwrap_or_default(),
            ))
        }
        async fn fetch_all_node_metrics(&self) -> Result<Vec<NodeMetricsInfo>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn refresh_success_populates_snapshot() {
        let mut state = GlobalState::default();
        let source = MockDataSource::success();

        state
            .refresh(&source, None)
            .await
            .expect("refresh should succeed");
        let snapshot = state.snapshot();

        assert_eq!(snapshot.phase, DataPhase::Ready);
        assert_eq!(snapshot.nodes.len(), 1);
        assert_eq!(snapshot.pods.len(), 2);
        assert_eq!(snapshot.services_count, 1);
        assert_eq!(snapshot.namespaces_count, 2);
        assert_eq!(snapshot.statefulsets.len(), 1);
        assert_eq!(snapshot.daemonsets.len(), 1);
        assert_eq!(snapshot.jobs.len(), 1);
        assert_eq!(snapshot.cronjobs.len(), 1);
        assert_eq!(snapshot.resource_quotas.len(), 1);
        assert_eq!(snapshot.limit_ranges.len(), 1);
        assert_eq!(snapshot.pod_disruption_budgets.len(), 1);
        assert_eq!(snapshot.service_accounts.len(), 1);
        assert_eq!(snapshot.roles.len(), 1);
        assert_eq!(snapshot.role_bindings.len(), 1);
        assert_eq!(snapshot.cluster_roles.len(), 1);
        assert_eq!(snapshot.cluster_role_bindings.len(), 1);
        assert_eq!(snapshot.custom_resource_definitions.len(), 1);
        assert_eq!(snapshot.cluster_summary(), "https://kind.local");
        assert_eq!(state.namespaces, vec!["default", "demo"]);
        assert!(snapshot.last_updated.is_some());
    }

    #[tokio::test]
    async fn refresh_namespaces_count_unions_pods_services_and_deployments() {
        let mut state = GlobalState::default();
        let source = MockDataSource {
            pods: vec![PodInfo {
                name: "pod-a".to_string(),
                namespace: "ns-a".to_string(),
                ..PodInfo::default()
            }],
            services: vec![ServiceInfo {
                name: "svc-b".to_string(),
                namespace: "ns-b".to_string(),
                ..ServiceInfo::default()
            }],
            deployments: vec![DeploymentInfo {
                name: "deploy-c".to_string(),
                namespace: "ns-c".to_string(),
                ..DeploymentInfo::default()
            }],
            ..MockDataSource::success()
        };

        state
            .refresh(&source, None)
            .await
            .expect("refresh should succeed");

        assert_eq!(state.snapshot().namespaces_count, 3);
    }

    #[tokio::test]
    async fn refresh_scopes_helm_and_flux_to_selected_namespace() {
        let mut state = GlobalState::default();
        let source = MockDataSource {
            flux_resources: vec![
                FluxResourceInfo {
                    name: "app-default".to_string(),
                    namespace: Some("default".to_string()),
                    kind: "Kustomization".to_string(),
                    group: "kustomize.toolkit.fluxcd.io".to_string(),
                    version: "v1".to_string(),
                    plural: "kustomizations".to_string(),
                    status: "Ready".to_string(),
                    ..FluxResourceInfo::default()
                },
                FluxResourceInfo {
                    name: "app-demo".to_string(),
                    namespace: Some("demo".to_string()),
                    kind: "Kustomization".to_string(),
                    group: "kustomize.toolkit.fluxcd.io".to_string(),
                    version: "v1".to_string(),
                    plural: "kustomizations".to_string(),
                    status: "NotReady".to_string(),
                    ..FluxResourceInfo::default()
                },
            ],
            ..MockDataSource::success()
        };

        state
            .refresh(&source, Some("demo"))
            .await
            .expect("refresh should succeed");
        let snapshot = state.snapshot();

        assert_eq!(snapshot.phase, DataPhase::Ready);
        assert_eq!(snapshot.pods.len(), 1);
        assert!(snapshot.pods.iter().all(|pod| pod.namespace == "demo"));
        assert!(
            snapshot
                .services
                .iter()
                .all(|service| service.namespace == "demo")
        );
        assert!(
            snapshot
                .deployments
                .iter()
                .all(|deployment| deployment.namespace == "demo")
        );
        assert!(snapshot.jobs.iter().all(|job| job.namespace == "demo"));
        assert!(
            snapshot
                .cronjobs
                .iter()
                .all(|cronjob| cronjob.namespace == "demo")
        );
        assert!(
            snapshot
                .resource_quotas
                .iter()
                .all(|quota| quota.namespace == "demo")
        );
        assert!(
            snapshot
                .service_accounts
                .iter()
                .all(|account| account.namespace == "demo")
        );
        assert_eq!(snapshot.flux_resources.len(), 1);
        assert_eq!(snapshot.flux_resources[0].name, "app-demo");
        assert_eq!(
            snapshot.flux_resources[0].namespace.as_deref(),
            Some("demo")
        );
        assert_eq!(snapshot.helm_releases.len(), 1);
        assert_eq!(snapshot.helm_releases[0].namespace, "demo");
        // Cluster-scoped resources remain unaffected by namespace scope.
        assert_eq!(snapshot.nodes.len(), 1);
        assert_eq!(snapshot.cluster_roles.len(), 1);
    }

    #[tokio::test]
    async fn refresh_enforces_helm_namespace_scope_when_source_returns_all_releases() {
        let mut state = GlobalState::default();
        let mut source = MockDataSource::success();
        source.helm_releases_ignore_namespace = true;

        state
            .refresh(&source, Some("demo"))
            .await
            .expect("refresh should succeed");
        let snapshot = state.snapshot();

        assert_eq!(snapshot.phase, DataPhase::Ready);
        assert_eq!(snapshot.helm_releases.len(), 1);
        assert!(snapshot.helm_releases.iter().all(|r| r.namespace == "demo"));
    }

    #[tokio::test]
    async fn refresh_with_options_can_skip_flux_fetch() {
        let mut state = GlobalState::default();
        let initial = MockDataSource {
            flux_resources: vec![FluxResourceInfo {
                name: "apps".to_string(),
                namespace: Some("default".to_string()),
                kind: "Kustomization".to_string(),
                group: "kustomize.toolkit.fluxcd.io".to_string(),
                version: "v1".to_string(),
                plural: "kustomizations".to_string(),
                status: "Ready".to_string(),
                ..FluxResourceInfo::default()
            }],
            ..MockDataSource::success()
        };
        state
            .refresh(&initial, Some("default"))
            .await
            .expect("initial refresh should succeed");
        assert_eq!(state.snapshot().flux_resources.len(), 1);
        assert_eq!(state.snapshot().flux_resources[0].name, "apps");

        let updated = MockDataSource {
            flux_resources: vec![FluxResourceInfo {
                name: "apps-v2".to_string(),
                namespace: Some("default".to_string()),
                kind: "Kustomization".to_string(),
                group: "kustomize.toolkit.fluxcd.io".to_string(),
                version: "v1".to_string(),
                plural: "kustomizations".to_string(),
                status: "Ready".to_string(),
                ..FluxResourceInfo::default()
            }],
            ..MockDataSource::success()
        };

        state
            .refresh_with_options(
                &updated,
                Some("default"),
                RefreshOptions {
                    include_flux: false,
                    include_cluster_info: true,
                    include_secondary_resources: true,
                    include_events: true,
                },
            )
            .await
            .expect("refresh should succeed while skipping flux");
        let snapshot = state.snapshot();
        assert_eq!(snapshot.flux_resources.len(), 1);
        assert_eq!(snapshot.flux_resources[0].name, "apps");
    }

    #[tokio::test]
    async fn refresh_with_options_tracks_secondary_resource_hydration() {
        let mut state = GlobalState::default();
        let source = MockDataSource::success();

        state.begin_loading_transition(false);
        assert!(!state.snapshot().secondary_resources_loaded);
        state.mark_refresh_requested(RefreshOptions {
            include_flux: true,
            include_cluster_info: true,
            include_secondary_resources: false,
            include_events: true,
        });
        let pending_snapshot = state.snapshot();
        assert_eq!(
            pending_snapshot.view_load_state(AppView::Pods),
            ViewLoadState::Loading
        );
        assert_eq!(
            pending_snapshot.view_load_state(AppView::NetworkPolicies),
            ViewLoadState::Loading
        );
        assert_eq!(
            pending_snapshot.view_load_state(AppView::StorageClasses),
            ViewLoadState::Loading
        );

        state
            .refresh_with_options(
                &source,
                Some("default"),
                RefreshOptions {
                    include_flux: true,
                    include_cluster_info: true,
                    include_secondary_resources: false,
                    include_events: true,
                },
            )
            .await
            .expect("fast refresh should succeed");
        let snapshot = state.snapshot();
        assert!(!snapshot.secondary_resources_loaded);
        assert_eq!(
            snapshot.view_load_state(AppView::Pods),
            ViewLoadState::Ready
        );
        assert_eq!(
            snapshot.view_load_state(AppView::NetworkPolicies),
            ViewLoadState::Loading
        );
        assert_eq!(
            snapshot.view_load_state(AppView::StorageClasses),
            ViewLoadState::Loading
        );

        state
            .refresh_with_options(
                &source,
                Some("default"),
                RefreshOptions {
                    include_flux: true,
                    include_cluster_info: true,
                    include_secondary_resources: true,
                    include_events: true,
                },
            )
            .await
            .expect("full refresh should succeed");
        let snapshot = state.snapshot();
        assert!(snapshot.secondary_resources_loaded);
        assert_eq!(
            snapshot.view_load_state(AppView::NetworkPolicies),
            ViewLoadState::Ready
        );
        assert_eq!(
            snapshot.view_load_state(AppView::StorageClasses),
            ViewLoadState::Ready
        );
    }

    #[tokio::test]
    async fn namespace_transition_with_partial_failures_does_not_leak_previous_namespace_data() {
        let mut state = GlobalState::default();
        let source = MockDataSource::success();

        state
            .refresh(&source, Some("default"))
            .await
            .expect("default namespace refresh should succeed");
        assert_eq!(state.snapshot().services.len(), 1);

        state.begin_loading_transition(false);
        let mut demo_source = MockDataSource::success();
        demo_source.services_err = Some("forbidden".to_string());

        state
            .refresh(&demo_source, Some("demo"))
            .await
            .expect("partial failure should still return ready");
        let snapshot = state.snapshot();

        assert_eq!(snapshot.phase, DataPhase::Ready);
        assert!(snapshot.pods.iter().all(|pod| pod.namespace == "demo"));
        // Must not retain stale default namespace services after switch.
        assert!(snapshot.services.is_empty());
        assert!(
            snapshot
                .last_error
                .as_deref()
                .unwrap_or_default()
                .contains("services")
        );
    }

    #[tokio::test]
    async fn refresh_snapshot_counts_match_dashboard_stats() {
        let mut state = GlobalState::default();
        let source = MockDataSource::success();

        state
            .refresh(&source, None)
            .await
            .expect("refresh should succeed");
        let snapshot = state.snapshot();
        let stats = crate::state::alerts::compute_dashboard_stats(&snapshot);

        assert_eq!(snapshot.services_count, stats.services_count);
        assert_eq!(snapshot.namespaces_count, stats.namespaces_count);
    }

    #[tokio::test]
    async fn refresh_partial_failure_degrades_gracefully() {
        let mut state = GlobalState::default();
        let mut source = MockDataSource::success();
        source.services_err = Some("forbidden".to_string());

        state
            .refresh(&source, None)
            .await
            .expect("partial failure should still return ready state");

        let snapshot = state.snapshot();
        assert_eq!(snapshot.phase, DataPhase::Ready);
        assert!(snapshot.services.is_empty());
        assert!(
            snapshot
                .last_error
                .as_deref()
                .unwrap_or_default()
                .contains("services")
        );
    }

    #[tokio::test]
    async fn refresh_with_only_replicasets_does_not_mark_all_failed() {
        let mut state = GlobalState::default();
        let source = MockDataSource {
            nodes: vec![],
            pods: vec![],
            services: vec![],
            deployments: vec![],
            statefulsets: vec![],
            daemonsets: vec![],
            replicasets: vec![ReplicaSetInfo {
                name: "rs-only".to_string(),
                namespace: "default".to_string(),
                ..ReplicaSetInfo::default()
            }],
            replication_controllers: vec![],
            jobs: vec![],
            cronjobs: vec![],
            resource_quotas: vec![],
            limit_ranges: vec![],
            pod_disruption_budgets: vec![],
            service_accounts: vec![],
            roles: vec![],
            role_bindings: vec![],
            cluster_roles: vec![],
            cluster_role_bindings: vec![],
            custom_resource_definitions: vec![],
            cluster_info: None,
            ..MockDataSource::success()
        };

        state
            .refresh(&source, None)
            .await
            .expect("replicasets-only snapshot should be valid");
        let snapshot = state.snapshot();
        assert_eq!(snapshot.phase, DataPhase::Ready);
        assert_eq!(snapshot.replicasets.len(), 1);
    }

    #[tokio::test]
    async fn refresh_all_fail_sets_error_phase() {
        let mut state = GlobalState::default();
        let source = MockDataSource {
            url: "https://broken".to_string(),
            nodes: vec![],
            namespaces: vec![],
            pods: vec![],
            services: vec![],
            deployments: vec![],
            statefulsets: vec![],
            daemonsets: vec![],
            replicasets: vec![],
            replication_controllers: vec![],
            jobs: vec![],
            cronjobs: vec![],
            resource_quotas: vec![],
            limit_ranges: vec![],
            pod_disruption_budgets: vec![],
            service_accounts: vec![],
            roles: vec![],
            role_bindings: vec![],
            cluster_roles: vec![],
            cluster_role_bindings: vec![],
            custom_resource_definitions: vec![],
            flux_resources: vec![],
            helm_releases: vec![],
            helm_releases_ignore_namespace: false,
            cluster_info: None,
            nodes_err: Some("nodes down".to_string()),
            pods_err: Some("pods down".to_string()),
            services_err: Some("services down".to_string()),
            deployments_err: Some("deployments down".to_string()),
            statefulsets_err: Some("statefulsets down".to_string()),
            daemonsets_err: Some("daemonsets down".to_string()),
            jobs_err: Some("jobs down".to_string()),
            cronjobs_err: Some("cronjobs down".to_string()),
            resource_quotas_err: Some("resourcequotas down".to_string()),
            limit_ranges_err: Some("limitranges down".to_string()),
            pod_disruption_budgets_err: Some("pdbs down".to_string()),
            service_accounts_err: Some("serviceaccounts down".to_string()),
            roles_err: Some("roles down".to_string()),
            role_bindings_err: Some("rolebindings down".to_string()),
            cluster_roles_err: Some("clusterroles down".to_string()),
            cluster_role_bindings_err: Some("clusterrolebindings down".to_string()),
            cluster_info_err: Some("cluster down".to_string()),
            delay_ms: 0,
        };

        let result = state.refresh(&source, None).await;
        assert!(result.is_err());

        let snapshot = state.snapshot();
        assert_eq!(snapshot.phase, DataPhase::Error);
        assert!(snapshot.last_error.is_some());
    }

    #[tokio::test]
    async fn refresh_handles_empty_resources_without_crashing() {
        let mut state = GlobalState::default();
        let source = MockDataSource {
            nodes: vec![],
            pods: vec![],
            services: vec![],
            deployments: vec![],
            statefulsets: vec![],
            daemonsets: vec![],
            jobs: vec![],
            cronjobs: vec![],
            resource_quotas: vec![],
            limit_ranges: vec![],
            pod_disruption_budgets: vec![],
            service_accounts: vec![],
            roles: vec![],
            role_bindings: vec![],
            cluster_roles: vec![],
            cluster_role_bindings: vec![],
            custom_resource_definitions: vec![],
            cluster_info: Some(ClusterInfo {
                server: "https://kind.local".to_string(),
                ..ClusterInfo::default()
            }),
            ..MockDataSource::success()
        };

        state
            .refresh(&source, None)
            .await
            .expect("empty lists are valid");
        let snapshot = state.snapshot();

        assert_eq!(snapshot.phase, DataPhase::Ready);
        assert_eq!(snapshot.nodes.len(), 0);
        assert_eq!(snapshot.pods.len(), 0);
        assert_eq!(snapshot.services_count, 0);
        assert_eq!(snapshot.namespaces_count, 0);
        assert!(snapshot.jobs.is_empty());
        assert!(snapshot.cronjobs.is_empty());
        assert!(snapshot.resource_quotas.is_empty());
        assert!(snapshot.limit_ranges.is_empty());
        assert!(snapshot.pod_disruption_budgets.is_empty());
        assert!(snapshot.service_accounts.is_empty());
        assert!(snapshot.roles.is_empty());
        assert!(snapshot.role_bindings.is_empty());
        assert!(snapshot.cluster_roles.is_empty());
        assert!(snapshot.cluster_role_bindings.is_empty());
        assert!(snapshot.custom_resource_definitions.is_empty());
    }

    #[tokio::test]
    async fn fetch_with_timeout_returns_timeout_error() {
        // Use a short timeout for testing (not the production 10s)
        let result: Result<Vec<NodeInfo>> =
            match tokio::time::timeout(Duration::from_millis(50), async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok(vec![])
            })
            .await
            {
                Ok(r) => r,
                Err(_) => Err(anyhow!("timed out fetching nodes")),
            };

        assert!(result.is_err());
        assert!(
            format!("{}", result.expect_err("must timeout")).contains("timed out fetching nodes")
        );
    }

    #[tokio::test]
    async fn fetch_with_timeout_retries_transient_send_request_error_once() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_fetch = Arc::clone(&calls);
        let result: Result<i32> = GlobalState::fetch_with_timeout("pods", move || {
            let calls = Arc::clone(&calls_for_fetch);
            async move {
                let attempt = calls.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    Err(anyhow::anyhow!("client error (SendRequest)"))
                } else {
                    Ok(7)
                }
            }
        })
        .await;

        assert_eq!(result.expect("transient SendRequest should retry"), 7);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn transient_send_request_detection_matches_builder_error_text() {
        let err = anyhow::anyhow!(
            "failed with error client error (SendRequest): connection closed before message completed",
        );
        assert!(GlobalState::is_transient_send_request_error(&err));
    }

    #[tokio::test]
    async fn refresh_skips_when_already_loading() {
        let mut state = GlobalState::default();
        state.snapshot.phase = DataPhase::Loading;
        state.snapshot_dirty = true;
        state.publish_snapshot();
        let source = MockDataSource::success().with_delay(100);

        state
            .refresh(&source, None)
            .await
            .expect("loading guard should no-op");

        assert_eq!(state.snapshot().phase, DataPhase::Loading);
    }

    #[test]
    fn begin_loading_transition_keeps_snapshot_version_monotonic() {
        let mut state = GlobalState::default();
        state.snapshot.snapshot_version = 7;
        state.snapshot.pods = vec![PodInfo {
            name: "pod-a".to_string(),
            namespace: "default".to_string(),
            ..PodInfo::default()
        }];
        state.namespaces = vec!["all".to_string(), "default".to_string()];
        state.snapshot_dirty = true;
        state.publish_snapshot();

        state.begin_loading_transition(false);
        let snapshot = state.snapshot();
        assert_eq!(snapshot.snapshot_version, 8);
        assert_eq!(snapshot.phase, DataPhase::Idle);
        assert!(snapshot.pods.is_empty());
        assert_eq!(
            state.namespaces,
            vec!["all".to_string(), "default".to_string()]
        );

        state.begin_loading_transition(true);
        assert_eq!(state.snapshot().snapshot_version, 9);
        assert!(state.namespaces.is_empty());
    }

    #[tokio::test]
    async fn snapshot_version_is_monotonic_across_namespace_transitions() {
        let mut state = GlobalState::default();
        let source = MockDataSource::success();

        state
            .refresh(&source, Some("default"))
            .await
            .expect("initial refresh should succeed");
        let v1 = state.snapshot().snapshot_version;

        state.begin_loading_transition(false);
        let v2 = state.snapshot().snapshot_version;
        assert!(v2 > v1);

        state
            .refresh(&source, Some("demo"))
            .await
            .expect("second refresh should succeed");
        let v3 = state.snapshot().snapshot_version;
        assert!(v3 > v2);
    }

    /// Verifies StatefulSet DTO default values are stable.
    #[test]
    fn test_statefulset_info_defaults() {
        let info = StatefulSetInfo::default();
        assert_eq!(info.name, "");
        assert_eq!(info.namespace, "");
        assert_eq!(info.desired_replicas, 0);
        assert_eq!(info.ready_replicas, 0);
        assert!(info.image.is_none());
        assert!(info.created_at.is_none());
    }

    /// Verifies DaemonSet ready calculation semantics used by UI.
    #[test]
    fn test_daemonset_info_ready_calculation() {
        let info = DaemonSetInfo {
            desired_count: 8,
            ready_count: 5,
            unavailable_count: 3,
            ..DaemonSetInfo::default()
        };

        assert_eq!(
            info.desired_count - info.ready_count,
            info.unavailable_count
        );
    }
}
