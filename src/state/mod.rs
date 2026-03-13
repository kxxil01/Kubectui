//! Global state management for KubecTUI.

pub mod alerts;
pub mod issues;
pub mod port_forward;
pub mod watch;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::{collections::HashSet, fmt, sync::Arc, sync::LazyLock, time::Duration};
use tokio::sync::Semaphore;

use crate::app::{AppView, ResourceRef};
use crate::k8s::{
    client::K8sClient,
    dtos::{
        ClusterInfo, ClusterRoleBindingInfo, ClusterRoleInfo, ClusterVersionInfo, ConfigMapInfo,
        CronJobInfo, CustomResourceDefinitionInfo, DaemonSetInfo, DeploymentInfo, EndpointInfo,
        FluxResourceInfo, HelmReleaseInfo, HpaInfo, IngressClassInfo, IngressInfo, JobInfo,
        K8sEventInfo, LimitRangeInfo, NamespaceInfo, NetworkPolicyInfo, NodeInfo, NodeMetricsInfo,
        PodDisruptionBudgetInfo, PodInfo, PodMetricsInfo, PriorityClassInfo, PvInfo, PvcInfo,
        ReplicaSetInfo, ReplicationControllerInfo, ResourceQuotaInfo, RoleBindingInfo, RoleInfo,
        SecretInfo, ServiceAccountInfo, ServiceInfo, StatefulSetInfo, StorageClassInfo,
    },
};

const MAX_CONCURRENT_CORE_FETCHES: usize = 8;
const MAX_CONCURRENT_SECONDARY_FETCHES: usize = 4;
static CORE_FETCH_SEMAPHORE: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(MAX_CONCURRENT_CORE_FETCHES));
static SECONDARY_FETCH_SEMAPHORE: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(MAX_CONCURRENT_SECONDARY_FETCHES));

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

/// Connection health state, computed after each refresh cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConnectionHealth {
    #[default]
    Unknown,
    Connected,
    /// Some resource fetches failed. Inner value is the count of failed resources.
    Degraded(usize),
    Disconnected,
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
    /// Buckets that have completed at least one fetch in the current scope.
    pub loaded_scope: RefreshScope,
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
    pub events_last_error: Option<String>,
    pub priority_classes: Vec<PriorityClassInfo>,
    pub helm_releases: Vec<HelmReleaseInfo>,
    pub flux_resources: Vec<FluxResourceInfo>,
    pub helm_repositories: Vec<crate::k8s::dtos::HelmRepoInfo>,
    pub node_metrics: Vec<NodeMetricsInfo>,
    pub pod_metrics: Vec<PodMetricsInfo>,
    pub issue_count: usize,
    pub services_count: usize,
    pub namespaces_count: usize,
    pub phase: DataPhase,
    pub last_updated: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub cluster_url: Option<String>,
    pub connection_health: ConnectionHealth,
    pub failed_resource_count: usize,
}

impl Default for ClusterSnapshot {
    fn default() -> Self {
        Self {
            snapshot_version: 0,
            loaded_scope: RefreshScope::NONE,
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
            events_last_error: None,
            priority_classes: Vec::new(),
            helm_releases: Vec::new(),
            flux_resources: Vec::new(),
            helm_repositories: Vec::new(),
            node_metrics: Vec::new(),
            pod_metrics: Vec::new(),
            issue_count: 0,
            services_count: 0,
            namespaces_count: 0,
            phase: DataPhase::Idle,
            last_updated: None,
            last_error: None,
            cluster_url: None,
            connection_health: ConnectionHealth::Unknown,
            failed_resource_count: 0,
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

    pub const fn scope_loaded(&self, scope: RefreshScope) -> bool {
        self.loaded_scope.contains(scope)
    }

    /// Returns the count of resources for a given view, or None if the view
    /// doesn't map to a direct collection.
    pub fn resource_count(&self, view: AppView) -> Option<usize> {
        let required_scope = GlobalState::view_ready_scope(view);
        if !required_scope.is_empty() && !self.loaded_scope.contains(required_scope) {
            return None;
        }

        match view {
            AppView::Nodes => Some(self.nodes.len()),
            AppView::Pods => Some(self.pods.len()),
            AppView::Deployments => Some(self.deployments.len()),
            AppView::StatefulSets => Some(self.statefulsets.len()),
            AppView::DaemonSets => Some(self.daemonsets.len()),
            AppView::ReplicaSets => Some(self.replicasets.len()),
            AppView::ReplicationControllers => Some(self.replication_controllers.len()),
            AppView::Jobs => Some(self.jobs.len()),
            AppView::CronJobs => Some(self.cronjobs.len()),
            AppView::Services => Some(self.services.len()),
            AppView::Endpoints => Some(self.endpoints.len()),
            AppView::Ingresses => Some(self.ingresses.len()),
            AppView::IngressClasses => Some(self.ingress_classes.len()),
            AppView::NetworkPolicies => Some(self.network_policies.len()),
            AppView::ConfigMaps => Some(self.config_maps.len()),
            AppView::Secrets => Some(self.secrets.len()),
            AppView::ResourceQuotas => Some(self.resource_quotas.len()),
            AppView::LimitRanges => Some(self.limit_ranges.len()),
            AppView::HPAs => Some(self.hpas.len()),
            AppView::PodDisruptionBudgets => Some(self.pod_disruption_budgets.len()),
            AppView::PriorityClasses => Some(self.priority_classes.len()),
            AppView::PersistentVolumeClaims => Some(self.pvcs.len()),
            AppView::PersistentVolumes => Some(self.pvs.len()),
            AppView::StorageClasses => Some(self.storage_classes.len()),
            AppView::Namespaces => Some(self.namespace_list.len()),
            AppView::Events => Some(self.events.len()),
            AppView::ServiceAccounts => Some(self.service_accounts.len()),
            AppView::Roles => Some(self.roles.len()),
            AppView::RoleBindings => Some(self.role_bindings.len()),
            AppView::ClusterRoles => Some(self.cluster_roles.len()),
            AppView::ClusterRoleBindings => Some(self.cluster_role_bindings.len()),
            AppView::Extensions => Some(self.custom_resource_definitions.len()),
            AppView::HelmReleases => Some(self.helm_releases.len()),
            AppView::HelmCharts => Some(self.helm_repositories.len()),
            AppView::FluxCDAll => Some(self.flux_resources.len()),
            AppView::FluxCDKustomizations => Some(
                self.flux_resources
                    .iter()
                    .filter(|r| {
                        r.group == "kustomize.toolkit.fluxcd.io" && r.kind == "Kustomization"
                    })
                    .count(),
            ),
            AppView::FluxCDHelmReleases => Some(
                self.flux_resources
                    .iter()
                    .filter(|r| r.group == "helm.toolkit.fluxcd.io" && r.kind == "HelmRelease")
                    .count(),
            ),
            AppView::FluxCDHelmRepositories => Some(
                self.flux_resources
                    .iter()
                    .filter(|r| r.group == "source.toolkit.fluxcd.io" && r.kind == "HelmRepository")
                    .count(),
            ),
            AppView::FluxCDAlertProviders => Some(
                self.flux_resources
                    .iter()
                    .filter(|r| {
                        r.group == "notification.toolkit.fluxcd.io" && r.kind == "AlertProvider"
                    })
                    .count(),
            ),
            AppView::FluxCDAlerts => Some(
                self.flux_resources
                    .iter()
                    .filter(|r| r.group == "notification.toolkit.fluxcd.io" && r.kind == "Alert")
                    .count(),
            ),
            AppView::FluxCDArtifacts => Some(
                self.flux_resources
                    .iter()
                    .filter(|r| r.artifact.is_some())
                    .count(),
            ),
            AppView::FluxCDImages => Some(
                self.flux_resources
                    .iter()
                    .filter(|r| r.group == "image.toolkit.fluxcd.io")
                    .count(),
            ),
            AppView::FluxCDReceivers => Some(
                self.flux_resources
                    .iter()
                    .filter(|r| r.group == "notification.toolkit.fluxcd.io" && r.kind == "Receiver")
                    .count(),
            ),
            AppView::FluxCDSources => Some(
                self.flux_resources
                    .iter()
                    .filter(|r| r.group == "source.toolkit.fluxcd.io")
                    .count(),
            ),
            AppView::Issues => Some(self.issue_count),
            // Dashboard, Bookmarks, and PortForwarding don't have direct collections
            AppView::Dashboard | AppView::Bookmarks | AppView::PortForwarding => None,
        }
    }
}

/// Data source contract for retrieving Kubernetes snapshot inputs.
#[async_trait]
pub trait ClusterDataSource {
    /// Returns cluster API URL for status header.
    fn cluster_url(&self) -> &str;
    /// Returns the current kube context name when known.
    fn cluster_context(&self) -> Option<&str>;
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
    /// Fetches cached API server version metadata.
    async fn fetch_cluster_version(&self) -> Result<ClusterVersionInfo>;
    /// Fetches the cluster-wide pod count regardless of active namespace scope.
    async fn fetch_cluster_pod_count(&self) -> Result<usize>;
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
    /// Fetches metrics for all pods (best-effort, returns empty if metrics-server absent).
    async fn fetch_all_pod_metrics(&self, namespace: Option<&str>) -> Result<Vec<PodMetricsInfo>>;
}

#[async_trait]
impl ClusterDataSource for K8sClient {
    fn cluster_url(&self) -> &str {
        K8sClient::cluster_url(self)
    }

    fn cluster_context(&self) -> Option<&str> {
        K8sClient::cluster_context(self)
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

    async fn fetch_cluster_version(&self) -> Result<ClusterVersionInfo> {
        K8sClient::fetch_cluster_version(self).await
    }

    async fn fetch_cluster_pod_count(&self) -> Result<usize> {
        K8sClient::fetch_cluster_pod_count(self).await
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

    async fn fetch_all_pod_metrics(&self, namespace: Option<&str>) -> Result<Vec<PodMetricsInfo>> {
        K8sClient::fetch_all_pod_metrics(self, namespace).await
    }
}

/// Mutable state holder with async refresh operations.
///
/// `snapshot` is `Arc<ClusterSnapshot>` so that `GlobalState::clone()` is a
/// cheap atomic refcount bump instead of deep-copying 30+ `Vec<T>` fields.
/// When the spawned refresh task needs to mutate the snapshot it calls
/// `Arc::make_mut`, triggering a copy-on-write clone asynchronously — off the
/// main event loop.
#[derive(Debug, Clone, Default)]
pub struct GlobalState {
    snapshot: Arc<ClusterSnapshot>,
    pub namespaces: Vec<String>,
    snapshot_dirty: bool,
}

/// Runtime refresh knobs for optional expensive fetch paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RefreshScope(u32);

impl RefreshScope {
    pub const NONE: Self = Self(0);
    pub const NODES: Self = Self(1 << 0);
    pub const PODS: Self = Self(1 << 1);
    pub const SERVICES: Self = Self(1 << 2);
    pub const DEPLOYMENTS: Self = Self(1 << 3);
    pub const STATEFULSETS: Self = Self(1 << 4);
    pub const DAEMONSETS: Self = Self(1 << 5);
    pub const REPLICASETS: Self = Self(1 << 6);
    pub const REPLICATION_CONTROLLERS: Self = Self(1 << 7);
    pub const JOBS: Self = Self(1 << 8);
    pub const CRONJOBS: Self = Self(1 << 9);
    pub const NAMESPACES: Self = Self(1 << 10);
    pub const METRICS: Self = Self(1 << 11);
    pub const NETWORK: Self = Self(1 << 12);
    pub const CONFIG: Self = Self(1 << 13);
    pub const STORAGE: Self = Self(1 << 14);
    pub const SECURITY: Self = Self(1 << 15);
    pub const HELM: Self = Self(1 << 16);
    pub const EXTENSIONS: Self = Self(1 << 17);
    pub const FLUX: Self = Self(1 << 18);
    pub const EVENTS: Self = Self(1 << 19);
    pub const LOCAL_HELM_REPOSITORIES: Self = Self(1 << 20);
    pub const WATCHED_SCOPES: Self = Self(
        Self::NODES.0
            | Self::PODS.0
            | Self::SERVICES.0
            | Self::DEPLOYMENTS.0
            | Self::STATEFULSETS.0
            | Self::DAEMONSETS.0
            | Self::REPLICASETS.0,
    );
    pub const CORE_OVERVIEW: Self = Self(
        Self::WATCHED_SCOPES.0
            | Self::REPLICATION_CONTROLLERS.0
            | Self::JOBS.0
            | Self::CRONJOBS.0
            | Self::NAMESPACES.0,
    );
    pub const LEGACY_SECONDARY: Self = Self(
        Self::NETWORK.0
            | Self::CONFIG.0
            | Self::STORAGE.0
            | Self::SECURITY.0
            | Self::HELM.0
            | Self::EXTENSIONS.0,
    );
    pub const DEFAULT: Self = Self(
        Self::CORE_OVERVIEW.0
            | Self::METRICS.0
            | Self::LEGACY_SECONDARY.0
            | Self::FLUX.0
            | Self::LOCAL_HELM_REPOSITORIES.0,
    );

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RefreshOptions {
    pub scope: RefreshScope,
    pub include_cluster_info: bool,
    /// When true, wave1 (core resources) returns previous snapshot values
    /// instead of fetching — used for secondary-only backfill passes.
    pub skip_core: bool,
}

impl RefreshOptions {
    pub const fn completed_scope(self) -> RefreshScope {
        if self.skip_core {
            self.scope.without(RefreshScope::CORE_OVERVIEW)
        } else {
            self.scope
        }
    }
}

impl Default for RefreshOptions {
    fn default() -> Self {
        Self {
            scope: RefreshScope::DEFAULT,
            include_cluster_info: true,
            skip_core: false,
        }
    }
}

fn remove_named<T, F>(items: &mut Vec<T>, key: F, expected_name: &str) -> bool
where
    F: Fn(&T) -> &String,
{
    let before = items.len();
    items.retain(|item| key(item) != expected_name);
    before != items.len()
}

fn remove_named_in_namespace<T, F>(
    items: &mut Vec<T>,
    key: F,
    expected_name: &str,
    expected_namespace: &str,
) -> bool
where
    F: Fn(&T) -> (&String, &String),
{
    let before = items.len();
    items.retain(|item| {
        let (name, namespace) = key(item);
        name != expected_name || namespace != expected_namespace
    });
    before != items.len()
}

impl GlobalState {
    /// Returns a cheap Arc-wrapped snapshot for UI rendering.
    /// No deep clone — just an Arc refcount bump.
    pub fn snapshot(&self) -> Arc<ClusterSnapshot> {
        self.snapshot.clone()
    }

    /// Recomputes derived fields (issue count) and clears the dirty flag.
    /// Called after every successful refresh or optimistic mutation.
    fn publish_snapshot(&mut self) {
        if !self.snapshot_dirty {
            return;
        }
        self.snapshot_dirty = false;
        let count = issues::compute_issues(&self.snapshot).len();
        Arc::make_mut(&mut self.snapshot).issue_count = count;
    }

    /// Returns fetched namespaces.
    pub fn namespaces(&self) -> &[String] {
        &self.namespaces
    }

    /// Applies an optimistic node schedulable state change after cordon/uncordon.
    pub fn apply_optimistic_node_schedulable(&mut self, node_name: &str, unschedulable: bool) {
        let snap = Arc::make_mut(&mut self.snapshot);
        if let Some(node) = snap.nodes.iter_mut().find(|n| n.name == node_name) {
            node.unschedulable = unschedulable;
            snap.snapshot_version = snap.snapshot_version.saturating_add(1);
            self.snapshot_dirty = true;
            self.publish_snapshot();
        }
    }

    /// Applies a successful delete locally so the list updates immediately
    /// before the background refresh completes.
    pub fn apply_optimistic_delete(&mut self, resource: &ResourceRef) {
        let snap = Arc::make_mut(&mut self.snapshot);
        let changed = match resource {
            ResourceRef::Node(name) => remove_named(&mut snap.nodes, |item| &item.name, name),
            ResourceRef::Pod(name, ns) => remove_named_in_namespace(
                &mut snap.pods,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Service(name, ns) => remove_named_in_namespace(
                &mut snap.services,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Deployment(name, ns) => remove_named_in_namespace(
                &mut snap.deployments,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::StatefulSet(name, ns) => remove_named_in_namespace(
                &mut snap.statefulsets,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::DaemonSet(name, ns) => remove_named_in_namespace(
                &mut snap.daemonsets,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::ReplicaSet(name, ns) => remove_named_in_namespace(
                &mut snap.replicasets,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::ReplicationController(name, ns) => remove_named_in_namespace(
                &mut snap.replication_controllers,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Job(name, ns) => remove_named_in_namespace(
                &mut snap.jobs,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::CronJob(name, ns) => remove_named_in_namespace(
                &mut snap.cronjobs,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::ResourceQuota(name, ns) => remove_named_in_namespace(
                &mut snap.resource_quotas,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::LimitRange(name, ns) => remove_named_in_namespace(
                &mut snap.limit_ranges,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::PodDisruptionBudget(name, ns) => remove_named_in_namespace(
                &mut snap.pod_disruption_budgets,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Endpoint(name, ns) => remove_named_in_namespace(
                &mut snap.endpoints,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Ingress(name, ns) => remove_named_in_namespace(
                &mut snap.ingresses,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::IngressClass(name) => {
                remove_named(&mut snap.ingress_classes, |item| &item.name, name)
            }
            ResourceRef::NetworkPolicy(name, ns) => remove_named_in_namespace(
                &mut snap.network_policies,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::ConfigMap(name, ns) => remove_named_in_namespace(
                &mut snap.config_maps,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Secret(name, ns) => remove_named_in_namespace(
                &mut snap.secrets,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Hpa(name, ns) => remove_named_in_namespace(
                &mut snap.hpas,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::PriorityClass(name) => {
                remove_named(&mut snap.priority_classes, |item| &item.name, name)
            }
            ResourceRef::Pvc(name, ns) => remove_named_in_namespace(
                &mut snap.pvcs,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Pv(name) => remove_named(&mut snap.pvs, |item| &item.name, name),
            ResourceRef::StorageClass(name) => {
                remove_named(&mut snap.storage_classes, |item| &item.name, name)
            }
            ResourceRef::Namespace(name) => {
                remove_named(&mut snap.namespace_list, |item| &item.name, name)
            }
            ResourceRef::Event(name, ns) => remove_named_in_namespace(
                &mut snap.events,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::ServiceAccount(name, ns) => remove_named_in_namespace(
                &mut snap.service_accounts,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Role(name, ns) => remove_named_in_namespace(
                &mut snap.roles,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::RoleBinding(name, ns) => remove_named_in_namespace(
                &mut snap.role_bindings,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::ClusterRole(name) => {
                remove_named(&mut snap.cluster_roles, |item| &item.name, name)
            }
            ResourceRef::ClusterRoleBinding(name) => {
                remove_named(&mut snap.cluster_role_bindings, |item| &item.name, name)
            }
            ResourceRef::HelmRelease(name, ns) => remove_named_in_namespace(
                &mut snap.helm_releases,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::CustomResource {
                name,
                namespace,
                group,
                version,
                kind,
                plural,
            } => {
                let before = snap.flux_resources.len();
                snap.flux_resources.retain(|item| {
                    item.name != *name
                        || item.namespace != *namespace
                        || item.group != *group
                        || item.version != *version
                        || item.kind != *kind
                        || item.plural != *plural
                });
                before != snap.flux_resources.len()
            }
        };

        if !changed {
            return;
        }

        snap.services_count = snap.services.len();
        snap.namespaces_count = snap
            .pods
            .iter()
            .map(|pod| pod.namespace.as_str())
            .chain(
                snap.services
                    .iter()
                    .map(|service| service.namespace.as_str()),
            )
            .chain(
                snap.deployments
                    .iter()
                    .map(|deployment| deployment.namespace.as_str()),
            )
            .collect::<HashSet<_>>()
            .len();
        snap.snapshot_version = snap.snapshot_version.saturating_add(1);
        self.snapshot_dirty = true;
        self.publish_snapshot();
    }

    /// Applies a successful scale locally so list views reflect the requested
    /// replica target immediately before the background refresh completes.
    pub fn apply_optimistic_scale(&mut self, resource: &ResourceRef, replicas: i32) {
        let snap = Arc::make_mut(&mut self.snapshot);
        let changed = match resource {
            ResourceRef::Deployment(name, ns) => snap
                .deployments
                .iter_mut()
                .find(|item| item.name == *name && item.namespace == *ns)
                .is_some_and(|deployment| {
                    if deployment.desired_replicas == replicas {
                        return false;
                    }
                    deployment.desired_replicas = replicas;
                    deployment.ready = format!("{}/{}", deployment.ready_replicas, replicas);
                    true
                }),
            ResourceRef::StatefulSet(name, ns) => snap
                .statefulsets
                .iter_mut()
                .find(|item| item.name == *name && item.namespace == *ns)
                .is_some_and(|statefulset| {
                    if statefulset.desired_replicas == replicas {
                        return false;
                    }
                    statefulset.desired_replicas = replicas;
                    true
                }),
            _ => false,
        };

        if !changed {
            return;
        }

        snap.snapshot_version = snap.snapshot_version.saturating_add(1);
        self.snapshot_dirty = true;
        self.publish_snapshot();
    }

    const fn view_ready_scope(view: AppView) -> RefreshScope {
        match view {
            AppView::Dashboard => RefreshScope::CORE_OVERVIEW,
            AppView::Bookmarks | AppView::PortForwarding => RefreshScope::NONE,
            AppView::Issues => RefreshScope::CORE_OVERVIEW
                .union(RefreshScope::LEGACY_SECONDARY)
                .union(RefreshScope::FLUX),
            AppView::Nodes => RefreshScope::NODES,
            AppView::Namespaces => RefreshScope::NAMESPACES,
            AppView::Pods => RefreshScope::PODS,
            AppView::Deployments => RefreshScope::DEPLOYMENTS,
            AppView::StatefulSets => RefreshScope::STATEFULSETS,
            AppView::DaemonSets => RefreshScope::DAEMONSETS,
            AppView::ReplicaSets => RefreshScope::REPLICASETS,
            AppView::ReplicationControllers => RefreshScope::REPLICATION_CONTROLLERS,
            AppView::Jobs => RefreshScope::JOBS,
            AppView::CronJobs => RefreshScope::CRONJOBS,
            AppView::Services => RefreshScope::SERVICES,
            AppView::HelmCharts => RefreshScope::LOCAL_HELM_REPOSITORIES,
            AppView::Endpoints
            | AppView::Ingresses
            | AppView::IngressClasses
            | AppView::NetworkPolicies => RefreshScope::NETWORK,
            AppView::ConfigMaps
            | AppView::Secrets
            | AppView::ResourceQuotas
            | AppView::LimitRanges
            | AppView::HPAs
            | AppView::PodDisruptionBudgets
            | AppView::PriorityClasses => RefreshScope::CONFIG,
            AppView::PersistentVolumeClaims
            | AppView::PersistentVolumes
            | AppView::StorageClasses => RefreshScope::STORAGE,
            AppView::HelmReleases => RefreshScope::HELM,
            AppView::FluxCDAlertProviders
            | AppView::FluxCDAlerts
            | AppView::FluxCDAll
            | AppView::FluxCDArtifacts
            | AppView::FluxCDHelmReleases
            | AppView::FluxCDHelmRepositories
            | AppView::FluxCDImages
            | AppView::FluxCDKustomizations
            | AppView::FluxCDReceivers
            | AppView::FluxCDSources => RefreshScope::FLUX,
            AppView::ServiceAccounts
            | AppView::ClusterRoles
            | AppView::Roles
            | AppView::ClusterRoleBindings
            | AppView::RoleBindings => RefreshScope::SECURITY,
            AppView::Extensions => RefreshScope::EXTENSIONS,
            AppView::Events => RefreshScope::EVENTS,
        }
    }

    fn has_view_data(&self, view: AppView) -> bool {
        let required_scope = Self::view_ready_scope(view);
        if !required_scope.is_empty() && self.snapshot.loaded_scope.contains(required_scope) {
            return true;
        }

        match view {
            AppView::Dashboard => {
                self.snapshot.cluster_info.is_some()
                    || !self.snapshot.nodes.is_empty()
                    || !self.snapshot.pods.is_empty()
                    || !self.snapshot.services.is_empty()
                    || !self.snapshot.deployments.is_empty()
            }
            AppView::Bookmarks => false,
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
            AppView::Issues => !self.snapshot.pods.is_empty() || !self.snapshot.nodes.is_empty(),
        }
    }

    fn set_view_load_state(&mut self, view: AppView, state: ViewLoadState) -> bool {
        let slot = &mut Arc::make_mut(&mut self.snapshot).view_load_states[view.index()];
        if *slot == state {
            return false;
        }
        *slot = state;
        true
    }

    fn mark_scope_requested(&mut self, scope: RefreshScope) -> bool {
        AppView::tabs().iter().fold(false, |changed, &view| {
            let required_scope = Self::view_ready_scope(view);
            if required_scope.is_empty() || !scope.intersects(required_scope) {
                return changed;
            }

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
        changed |= self.mark_scope_requested(options.scope);
        changed |= self.set_view_load_state(AppView::PortForwarding, ViewLoadState::Ready);

        if changed {
            self.snapshot_dirty = true;
            self.publish_snapshot();
        }
    }

    pub fn mark_events_refresh_requested(&mut self) -> bool {
        let mut changed = self.mark_scope_requested(RefreshScope::EVENTS);
        {
            let snap = Arc::make_mut(&mut self.snapshot);
            if snap.events_last_error.take().is_some() {
                changed = true;
            }
        }
        if changed {
            self.snapshot_dirty = true;
            self.publish_snapshot();
        }
        changed
    }

    /// Applies a watch-backed resource update to the snapshot.
    ///
    /// Only updates the snapshot if the data actually changed, avoiding
    /// unnecessary version bumps and downstream cache invalidation.
    pub fn apply_watch_update(&mut self, update: watch::WatchUpdate) {
        let mut changed = false;
        {
            let snap = Arc::make_mut(&mut self.snapshot);
            match update.data {
                watch::WatchPayload::Pods(pods) => {
                    if snap.pods != pods {
                        snap.pods = pods;
                        snap.snapshot_version = snap.snapshot_version.saturating_add(1);
                        changed = true;
                    }
                    let slot = &mut snap.view_load_states[AppView::Pods.index()];
                    if *slot != ViewLoadState::Ready {
                        *slot = ViewLoadState::Ready;
                        changed = true;
                    }
                }
                watch::WatchPayload::Deployments(items) => {
                    snap.deployments = items;
                    snap.snapshot_version = snap.snapshot_version.saturating_add(1);
                    changed = true;
                    let slot = &mut snap.view_load_states[AppView::Deployments.index()];
                    if *slot != ViewLoadState::Ready {
                        *slot = ViewLoadState::Ready;
                    }
                }
                watch::WatchPayload::ReplicaSets(items) => {
                    snap.replicasets = items;
                    snap.snapshot_version = snap.snapshot_version.saturating_add(1);
                    changed = true;
                    let slot = &mut snap.view_load_states[AppView::ReplicaSets.index()];
                    if *slot != ViewLoadState::Ready {
                        *slot = ViewLoadState::Ready;
                    }
                }
                watch::WatchPayload::StatefulSets(items) => {
                    snap.statefulsets = items;
                    snap.snapshot_version = snap.snapshot_version.saturating_add(1);
                    changed = true;
                    let slot = &mut snap.view_load_states[AppView::StatefulSets.index()];
                    if *slot != ViewLoadState::Ready {
                        *slot = ViewLoadState::Ready;
                    }
                }
                watch::WatchPayload::DaemonSets(items) => {
                    snap.daemonsets = items;
                    snap.snapshot_version = snap.snapshot_version.saturating_add(1);
                    changed = true;
                    let slot = &mut snap.view_load_states[AppView::DaemonSets.index()];
                    if *slot != ViewLoadState::Ready {
                        *slot = ViewLoadState::Ready;
                    }
                }
                watch::WatchPayload::Services(items) => {
                    snap.services = items;
                    snap.snapshot_version = snap.snapshot_version.saturating_add(1);
                    snap.services_count = snap.services.len();
                    changed = true;
                    let slot = &mut snap.view_load_states[AppView::Services.index()];
                    if *slot != ViewLoadState::Ready {
                        *slot = ViewLoadState::Ready;
                    }
                }
                watch::WatchPayload::Nodes(nodes) => {
                    if snap.nodes != nodes {
                        snap.nodes = nodes;
                        snap.snapshot_version = snap.snapshot_version.saturating_add(1);
                        changed = true;
                    }
                    let slot = &mut snap.view_load_states[AppView::Nodes.index()];
                    if *slot != ViewLoadState::Ready {
                        *slot = ViewLoadState::Ready;
                        changed = true;
                    }
                }
                watch::WatchPayload::Error { .. } => {
                    // Watcher errors are informational — do not clear existing
                    // snapshot data. Polling will continue to refresh on its
                    // own schedule.
                }
            }
        }
        if changed {
            self.snapshot_dirty = true;
            self.publish_snapshot();
        }
    }

    pub fn apply_events_update(&mut self, events: Vec<K8sEventInfo>) {
        let mut changed = false;
        {
            let snap = Arc::make_mut(&mut self.snapshot);
            if snap.events != events {
                snap.events = events;
                snap.snapshot_version = snap.snapshot_version.saturating_add(1);
                changed = true;
            }
            if snap.events_last_error.take().is_some() {
                changed = true;
            }
            let slot = &mut snap.view_load_states[AppView::Events.index()];
            if *slot != ViewLoadState::Ready {
                *slot = ViewLoadState::Ready;
                changed = true;
            }
        }

        if changed {
            self.snapshot_dirty = true;
            self.publish_snapshot();
        }
    }

    pub fn fail_events_refresh(&mut self, error: impl Into<String>) {
        let error = error.into();
        let mut changed = false;
        {
            let snap = Arc::make_mut(&mut self.snapshot);
            if snap.events_last_error.as_deref() != Some(error.as_str()) {
                snap.events_last_error = Some(error);
                changed = true;
            }
        }
        if self.set_view_load_state(AppView::Events, ViewLoadState::Ready) {
            changed = true;
        }
        if changed {
            self.snapshot_dirty = true;
            self.publish_snapshot();
        }
    }

    fn mark_refresh_completed(&mut self, options: RefreshOptions) {
        let loaded_scope = self.snapshot.loaded_scope.union(options.completed_scope());
        let mut changed = false;
        changed |= AppView::tabs().iter().fold(false, |acc, &view| {
            let required_scope = Self::view_ready_scope(view);
            if required_scope.is_empty() || !loaded_scope.contains(required_scope) {
                return acc;
            }
            self.set_view_load_state(view, ViewLoadState::Ready) || acc
        });
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
        self.snapshot = Arc::new(ClusterSnapshot {
            snapshot_version: next_snapshot_version,
            phase: DataPhase::Idle,
            cluster_url,
            ..ClusterSnapshot::default()
        });
        if clear_namespaces {
            self.namespaces.clear();
        }
        self.snapshot_dirty = true;
        self.publish_snapshot();
    }

    /// Overrides the snapshot phase (e.g. to mark an error after a failed
    /// background context switch).
    pub fn set_phase(&mut self, phase: DataPhase) {
        Arc::make_mut(&mut self.snapshot).phase = phase;
        self.snapshot_dirty = true;
        self.publish_snapshot();
    }

    /// Per-resource fetch timeout in seconds.
    const FETCH_TIMEOUT_SECS: u64 = 10;
    /// Retry transient transport failures before surfacing errors.
    const TRANSIENT_RETRY_ATTEMPTS: usize = 3;
    const TRANSIENT_RETRY_DELAY_MS: u64 = 150;

    async fn fetch_with_timeout<T, F, Fut>(
        label: &'static str,
        semaphore: &Semaphore,
        make_fut: F,
    ) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        for attempt in 0..=Self::TRANSIENT_RETRY_ATTEMPTS {
            let _permit = semaphore
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

    async fn maybe_fetch<T, F, Fut>(
        enabled: bool,
        label: &'static str,
        semaphore: &Semaphore,
        make_fut: F,
    ) -> Option<Result<T>>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        if enabled {
            Some(Self::fetch_with_timeout(label, semaphore, make_fut).await)
        } else {
            None
        }
    }

    fn apply_vec_fetch_result<T>(
        slot: &mut Vec<T>,
        result: Option<Result<Vec<T>>>,
        label: &str,
        errors: &mut Vec<String>,
        total_fetches: &mut usize,
    ) {
        let Some(result) = result else {
            return;
        };
        *total_fetches += 1;
        match result {
            Ok(items) => *slot = items,
            Err(err) => {
                errors.push(format!("{label}: {err}"));
            }
        }
    }

    fn apply_optional_fetch_result<T>(
        result: Option<Result<T>>,
        label: &str,
        errors: &mut Vec<String>,
        total_fetches: &mut usize,
    ) -> Option<T> {
        let result = result?;
        *total_fetches += 1;
        match result {
            Ok(value) => Some(value),
            Err(err) => {
                errors.push(format!("{label}: {err}"));
                None
            }
        }
    }

    fn build_cluster_info(
        client: &impl ClusterDataSource,
        nodes: &[NodeInfo],
        pod_count: usize,
        version: ClusterVersionInfo,
    ) -> ClusterInfo {
        ClusterInfo {
            context: client.cluster_context().map(str::to_string),
            server: client.cluster_url().to_string(),
            git_version: Some(version.git_version),
            platform: Some(version.platform),
            node_count: nodes.len(),
            ready_nodes: nodes.iter().filter(|node| node.ready).count(),
            pod_count,
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
                let snap = Arc::make_mut(&mut self.snapshot);
                snap.phase = DataPhase::Error;
                snap.last_error = Some("Global refresh timed out (60s)".to_string());
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

        // Trigger copy-on-write — the deep clone happens here (asynchronously,
        // off the main event loop) rather than at the GlobalState::clone() site.
        let snap = Arc::make_mut(&mut self.snapshot);
        snap.phase = DataPhase::Loading;
        snap.last_error = None;
        snap.cluster_url = Some(client.cluster_url().to_string());

        let fetch_nodes = options.scope.intersects(RefreshScope::NODES);
        let fetch_pods = options.scope.intersects(RefreshScope::PODS);
        let fetch_services = options.scope.intersects(RefreshScope::SERVICES);
        let fetch_deployments = options.scope.intersects(RefreshScope::DEPLOYMENTS);
        let fetch_statefulsets = options.scope.intersects(RefreshScope::STATEFULSETS);
        let fetch_daemonsets = options.scope.intersects(RefreshScope::DAEMONSETS);
        let fetch_replicasets = options.scope.intersects(RefreshScope::REPLICASETS);
        let fetch_replication_controllers = options
            .scope
            .intersects(RefreshScope::REPLICATION_CONTROLLERS);
        let fetch_jobs = options.scope.intersects(RefreshScope::JOBS);
        let fetch_cronjobs = options.scope.intersects(RefreshScope::CRONJOBS);
        let fetch_namespaces = options.scope.intersects(RefreshScope::NAMESPACES);
        let fetch_metrics = options.scope.intersects(RefreshScope::METRICS);
        let fetch_network = options.scope.intersects(RefreshScope::NETWORK);
        let fetch_config = options.scope.intersects(RefreshScope::CONFIG);
        let fetch_storage = options.scope.intersects(RefreshScope::STORAGE);
        let fetch_security = options.scope.intersects(RefreshScope::SECURITY);
        let fetch_helm = options.scope.intersects(RefreshScope::HELM);
        let fetch_extensions = options.scope.intersects(RefreshScope::EXTENSIONS);
        let fetch_flux = options.scope.intersects(RefreshScope::FLUX);
        let fetch_local_helm_repositories = options
            .scope
            .intersects(RefreshScope::LOCAL_HELM_REPOSITORIES);
        let fetch_cluster_info = options.include_cluster_info;
        let skip_core = options.skip_core;
        let wave1 = tokio::join!(
            Self::maybe_fetch(
                fetch_nodes && !skip_core,
                "nodes",
                &CORE_FETCH_SEMAPHORE,
                || client.fetch_nodes()
            ),
            Self::maybe_fetch(
                fetch_pods && !skip_core,
                "pods",
                &CORE_FETCH_SEMAPHORE,
                || client.fetch_pods(namespace)
            ),
            Self::maybe_fetch(
                fetch_services && !skip_core,
                "services",
                &CORE_FETCH_SEMAPHORE,
                || client.fetch_services(namespace)
            ),
            Self::maybe_fetch(
                fetch_deployments && !skip_core,
                "deployments",
                &CORE_FETCH_SEMAPHORE,
                || client.fetch_deployments(namespace)
            ),
            Self::maybe_fetch(
                fetch_statefulsets && !skip_core,
                "statefulsets",
                &CORE_FETCH_SEMAPHORE,
                || client.fetch_statefulsets(namespace)
            ),
            Self::maybe_fetch(
                fetch_daemonsets && !skip_core,
                "daemonsets",
                &CORE_FETCH_SEMAPHORE,
                || client.fetch_daemonsets(namespace)
            ),
            Self::maybe_fetch(
                fetch_replicasets && !skip_core,
                "replicasets",
                &CORE_FETCH_SEMAPHORE,
                || client.fetch_replicasets(namespace)
            ),
            Self::maybe_fetch(
                fetch_replication_controllers && !skip_core,
                "replicationcontrollers",
                &CORE_FETCH_SEMAPHORE,
                || client.fetch_replication_controllers(namespace)
            ),
            Self::maybe_fetch(
                fetch_jobs && !skip_core,
                "jobs",
                &CORE_FETCH_SEMAPHORE,
                || client.fetch_jobs(namespace)
            ),
            Self::maybe_fetch(
                fetch_cronjobs && !skip_core,
                "cronjobs",
                &CORE_FETCH_SEMAPHORE,
                || client.fetch_cronjobs(namespace)
            ),
            Self::maybe_fetch(
                fetch_namespaces && !skip_core,
                "namespacelist",
                &CORE_FETCH_SEMAPHORE,
                || { client.fetch_namespace_list() }
            ),
            Self::maybe_fetch(fetch_flux, "fluxresources", &CORE_FETCH_SEMAPHORE, || {
                client.fetch_flux_resources(namespace)
            }),
            Self::maybe_fetch(fetch_metrics, "cluster info", &CORE_FETCH_SEMAPHORE, || {
                client.fetch_cluster_version()
            }),
            Self::maybe_fetch(
                fetch_cluster_info && namespace.is_some(),
                "cluster pod count",
                &CORE_FETCH_SEMAPHORE,
                || client.fetch_cluster_pod_count()
            ),
        );

        let wave2 = tokio::join!(
            Self::maybe_fetch(
                fetch_config,
                "resourcequotas",
                &SECONDARY_FETCH_SEMAPHORE,
                || client.fetch_resource_quotas(namespace)
            ),
            Self::maybe_fetch(
                fetch_config,
                "limitranges",
                &SECONDARY_FETCH_SEMAPHORE,
                || client.fetch_limit_ranges(namespace)
            ),
            Self::maybe_fetch(fetch_config, "pdbs", &SECONDARY_FETCH_SEMAPHORE, || {
                client.fetch_pod_disruption_budgets(namespace)
            }),
            Self::maybe_fetch(
                fetch_security,
                "serviceaccounts",
                &SECONDARY_FETCH_SEMAPHORE,
                || client.fetch_service_accounts(namespace)
            ),
            Self::maybe_fetch(fetch_security, "roles", &SECONDARY_FETCH_SEMAPHORE, || {
                client.fetch_roles(namespace)
            }),
            Self::maybe_fetch(
                fetch_security,
                "rolebindings",
                &SECONDARY_FETCH_SEMAPHORE,
                || client.fetch_role_bindings(namespace)
            ),
            Self::maybe_fetch(
                fetch_security,
                "clusterroles",
                &SECONDARY_FETCH_SEMAPHORE,
                || client.fetch_cluster_roles()
            ),
            Self::maybe_fetch(
                fetch_security,
                "clusterrolebindings",
                &SECONDARY_FETCH_SEMAPHORE,
                || client.fetch_cluster_role_bindings()
            ),
            Self::maybe_fetch(fetch_extensions, "crds", &SECONDARY_FETCH_SEMAPHORE, || {
                client.fetch_custom_resource_definitions()
            }),
            Self::maybe_fetch(
                fetch_network,
                "endpoints",
                &SECONDARY_FETCH_SEMAPHORE,
                || { client.fetch_endpoints(namespace) }
            ),
            Self::maybe_fetch(
                fetch_network,
                "ingresses",
                &SECONDARY_FETCH_SEMAPHORE,
                || { client.fetch_ingresses(namespace) }
            ),
            Self::maybe_fetch(
                fetch_network,
                "ingressclasses",
                &SECONDARY_FETCH_SEMAPHORE,
                || client.fetch_ingress_classes()
            ),
            Self::maybe_fetch(
                fetch_network,
                "networkpolicies",
                &SECONDARY_FETCH_SEMAPHORE,
                || client.fetch_network_policies(namespace)
            ),
            Self::maybe_fetch(
                fetch_config,
                "configmaps",
                &SECONDARY_FETCH_SEMAPHORE,
                || { client.fetch_config_maps(namespace) }
            ),
            Self::maybe_fetch(fetch_config, "secrets", &SECONDARY_FETCH_SEMAPHORE, || {
                client.fetch_secrets(namespace)
            }),
            Self::maybe_fetch(fetch_config, "hpas", &SECONDARY_FETCH_SEMAPHORE, || {
                client.fetch_hpas(namespace)
            }),
            Self::maybe_fetch(fetch_storage, "pvcs", &SECONDARY_FETCH_SEMAPHORE, || {
                client.fetch_pvcs(namespace)
            }),
            Self::maybe_fetch(fetch_storage, "pvs", &SECONDARY_FETCH_SEMAPHORE, || {
                client.fetch_pvs()
            }),
            Self::maybe_fetch(
                fetch_storage,
                "storageclasses",
                &SECONDARY_FETCH_SEMAPHORE,
                || client.fetch_storage_classes()
            ),
            Self::maybe_fetch(
                fetch_config,
                "priorityclasses",
                &SECONDARY_FETCH_SEMAPHORE,
                || client.fetch_priority_classes()
            ),
            Self::maybe_fetch(
                fetch_helm,
                "helmreleases",
                &SECONDARY_FETCH_SEMAPHORE,
                || { client.fetch_helm_releases(namespace) }
            ),
            Self::maybe_fetch(
                fetch_metrics,
                "nodemetrics",
                &SECONDARY_FETCH_SEMAPHORE,
                || { client.fetch_all_node_metrics() }
            ),
            Self::maybe_fetch(
                fetch_metrics,
                "podmetrics",
                &SECONDARY_FETCH_SEMAPHORE,
                || { client.fetch_all_pod_metrics(namespace) }
            ),
        );

        let (
            (
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
                cluster_pod_count_res,
            ),
            (
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
                pod_metrics_res,
            ),
        ) = (wave1, wave2);

        let core_fetch_succeeded = matches!(nodes_res.as_ref(), Some(Ok(_)))
            || matches!(pods_res.as_ref(), Some(Ok(_)))
            || matches!(services_res.as_ref(), Some(Ok(_)))
            || matches!(deployments_res.as_ref(), Some(Ok(_)))
            || matches!(statefulsets_res.as_ref(), Some(Ok(_)))
            || matches!(daemonsets_res.as_ref(), Some(Ok(_)))
            || matches!(jobs_res.as_ref(), Some(Ok(_)))
            || matches!(cronjobs_res.as_ref(), Some(Ok(_)))
            || matches!(cluster_info_res.as_ref(), Some(Ok(_)));

        let mut errors = Vec::new();
        let mut total_fetches: usize = 0;

        {
            let snap = Arc::make_mut(&mut self.snapshot);
            Self::apply_vec_fetch_result(
                &mut snap.nodes,
                nodes_res,
                "nodes",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.pods,
                pods_res,
                "pods",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.services,
                services_res,
                "services",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.deployments,
                deployments_res,
                "deployments",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.statefulsets,
                statefulsets_res,
                "statefulsets",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.daemonsets,
                daemonsets_res,
                "daemonsets",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.replicasets,
                replicasets_res,
                "replicasets",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.replication_controllers,
                replication_controllers_res,
                "replicationcontrollers",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.jobs,
                jobs_res,
                "jobs",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.cronjobs,
                cronjobs_res,
                "cronjobs",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.namespace_list,
                namespace_list_res,
                "namespacelist",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.resource_quotas,
                resource_quotas_res,
                "resourcequotas",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.limit_ranges,
                limit_ranges_res,
                "limitranges",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.pod_disruption_budgets,
                pod_disruption_budgets_res,
                "pdbs",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.service_accounts,
                service_accounts_res,
                "serviceaccounts",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.roles,
                roles_res,
                "roles",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.role_bindings,
                role_bindings_res,
                "rolebindings",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.cluster_roles,
                cluster_roles_res,
                "clusterroles",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.cluster_role_bindings,
                cluster_role_bindings_res,
                "clusterrolebindings",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.custom_resource_definitions,
                custom_resource_definitions_res,
                "crds",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.endpoints,
                endpoints_res,
                "endpoints",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.ingresses,
                ingresses_res,
                "ingresses",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.ingress_classes,
                ingress_classes_res,
                "ingressclasses",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.network_policies,
                network_policies_res,
                "networkpolicies",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.config_maps,
                config_maps_res,
                "configmaps",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.secrets,
                secrets_res,
                "secrets",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.hpas,
                hpas_res,
                "hpas",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.pvcs,
                pvcs_res,
                "pvcs",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.pvs,
                pvs_res,
                "pvs",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.storage_classes,
                storage_classes_res,
                "storageclasses",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.priority_classes,
                priority_classes_res,
                "priorityclasses",
                &mut errors,
                &mut total_fetches,
            );
            if let Some(helm_releases) = Self::apply_optional_fetch_result(
                helm_releases_res,
                "helmreleases",
                &mut errors,
                &mut total_fetches,
            ) {
                snap.helm_releases = Self::filter_namespace(helm_releases, namespace, |release| {
                    release.namespace.as_str()
                });
            }
            Self::apply_vec_fetch_result(
                &mut snap.flux_resources,
                flux_resources_res,
                "fluxresources",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.node_metrics,
                node_metrics_res,
                "nodemetrics",
                &mut errors,
                &mut total_fetches,
            );
            Self::apply_vec_fetch_result(
                &mut snap.pod_metrics,
                pod_metrics_res,
                "podmetrics",
                &mut errors,
                &mut total_fetches,
            );
            if let Some(cluster_version) = Self::apply_optional_fetch_result(
                cluster_info_res,
                "cluster info",
                &mut errors,
                &mut total_fetches,
            ) && !snap.nodes.is_empty()
            {
                let cluster_pod_count = if namespace.is_some() {
                    Self::apply_optional_fetch_result(
                        cluster_pod_count_res,
                        "cluster pod count",
                        &mut errors,
                        &mut total_fetches,
                    )
                } else {
                    None
                };
                if let Some(pod_count) =
                    cluster_pod_count.or_else(|| namespace.is_none().then_some(snap.pods.len()))
                {
                    snap.cluster_info = Some(Self::build_cluster_info(
                        client,
                        &snap.nodes,
                        pod_count,
                        cluster_version,
                    ));
                }
            }
        }

        self.namespaces = Self::namespace_names_from_list(&self.snapshot.namespace_list);

        let all_failed = !skip_core
            && total_fetches > 0
            && errors.len() >= total_fetches
            && !core_fetch_succeeded;

        if all_failed {
            let message = if errors.is_empty() {
                "failed to refresh cluster state".to_string()
            } else if errors.len() <= 3 {
                errors.join(" | ")
            } else {
                format!("{} (+{} more)", errors[..3].join(" | "), errors.len() - 3)
            };
            let snap = Arc::make_mut(&mut self.snapshot);
            snap.phase = DataPhase::Error;
            snap.last_error = Some(message.clone());
            snap.connection_health = ConnectionHealth::Disconnected;
            snap.failed_resource_count = errors.len();
            self.snapshot_dirty = true;
            self.publish_snapshot();
            return Err(anyhow!(message));
        }

        let namespaces_count = self
            .snapshot
            .pods
            .iter()
            .map(|pod| pod.namespace.as_str())
            .chain(
                self.snapshot
                    .services
                    .iter()
                    .map(|service| service.namespace.as_str()),
            )
            .chain(
                self.snapshot
                    .deployments
                    .iter()
                    .map(|deployment| deployment.namespace.as_str()),
            )
            .collect::<HashSet<_>>()
            .len();

        let prev_loaded_scope = self.snapshot.loaded_scope;
        let prev_connection_health = self.snapshot.connection_health;
        {
            let snap = Arc::make_mut(&mut self.snapshot);
            snap.services_count = snap.services.len();
            snap.namespaces_count = namespaces_count;
            if fetch_local_helm_repositories {
                snap.helm_repositories = crate::k8s::helm::read_helm_repositories();
            }
            snap.loaded_scope = prev_loaded_scope.union(options.completed_scope());
        }
        self.mark_refresh_completed(options);
        // Arc refcount is 1 here — make_mut is a no-op pointer return.
        let snap = Arc::make_mut(&mut self.snapshot);
        snap.snapshot_version = snap.snapshot_version.saturating_add(1);
        snap.phase = DataPhase::Ready;
        snap.last_updated = Some(Utc::now());
        snap.failed_resource_count = errors.len();
        snap.connection_health = if total_fetches == 0 {
            prev_connection_health
        } else if errors.is_empty() {
            ConnectionHealth::Connected
        } else if errors.len() >= total_fetches {
            ConnectionHealth::Disconnected
        } else {
            ConnectionHealth::Degraded(errors.len())
        };
        snap.last_error = if errors.is_empty() {
            None
        } else if errors.len() <= 3 {
            Some(errors.join(" | "))
        } else {
            Some(format!(
                "{} (+{} more)",
                errors[..3].join(" | "),
                errors.len() - 3
            ))
        };
        self.snapshot_dirty = true;

        self.publish_snapshot();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn refresh_options(scope: RefreshScope, skip_core: bool) -> RefreshOptions {
        RefreshOptions {
            scope,
            include_cluster_info: false,
            skip_core,
        }
    }

    #[derive(Default)]
    struct MockFetchCounters {
        total: AtomicUsize,
        nodes: AtomicUsize,
        pods: AtomicUsize,
        services: AtomicUsize,
        namespaces: AtomicUsize,
        workload_calls: AtomicUsize,
        network_calls: AtomicUsize,
        config_calls: AtomicUsize,
        storage_calls: AtomicUsize,
        security_calls: AtomicUsize,
        helm_calls: AtomicUsize,
        extension_calls: AtomicUsize,
        flux_resources: AtomicUsize,
        cluster_version: AtomicUsize,
        cluster_pod_count: AtomicUsize,
        node_metrics: AtomicUsize,
        pod_metrics: AtomicUsize,
        service_accounts: AtomicUsize,
        pvcs: AtomicUsize,
    }

    #[derive(Clone)]
    struct MockDataSource {
        url: String,
        context: Option<String>,
        fetch_counters: Arc<MockFetchCounters>,
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
        pod_metrics: Vec<PodMetricsInfo>,
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
        node_metrics_err: Option<String>,
        pod_metrics_err: Option<String>,
        config_maps_err: Option<String>,
        secrets_err: Option<String>,
        hpas_err: Option<String>,
        priority_classes_err: Option<String>,
        delay_ms: u64,
    }

    impl MockDataSource {
        fn success() -> Self {
            Self {
                url: "https://kind.local".to_string(),
                context: Some("kind-kind".to_string()),
                fetch_counters: Arc::new(MockFetchCounters::default()),
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
                pod_metrics: vec![],
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
                node_metrics_err: None,
                pod_metrics_err: None,
                config_maps_err: None,
                secrets_err: None,
                hpas_err: None,
                priority_classes_err: None,
                delay_ms: 0,
            }
        }

        fn with_delay(mut self, delay_ms: u64) -> Self {
            self.delay_ms = delay_ms;
            self
        }

        fn bump(&self, counter: &AtomicUsize) {
            self.fetch_counters.total.fetch_add(1, Ordering::Relaxed);
            counter.fetch_add(1, Ordering::Relaxed);
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

        fn cluster_context(&self) -> Option<&str> {
            self.context.as_deref()
        }

        async fn fetch_nodes(&self) -> Result<Vec<NodeInfo>> {
            self.bump(&self.fetch_counters.nodes);
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
            self.bump(&self.fetch_counters.pods);
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
            self.bump(&self.fetch_counters.services);
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
            self.bump(&self.fetch_counters.workload_calls);
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
            self.bump(&self.fetch_counters.workload_calls);
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
            self.bump(&self.fetch_counters.workload_calls);
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
            self.bump(&self.fetch_counters.workload_calls);
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
            self.bump(&self.fetch_counters.workload_calls);
            Ok(Self::filter_namespace(
                &self.replication_controllers,
                namespace,
                |controller| &controller.namespace,
            ))
        }

        async fn fetch_jobs(&self, namespace: Option<&str>) -> Result<Vec<JobInfo>> {
            self.bump(&self.fetch_counters.workload_calls);
            if let Some(err) = &self.jobs_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(Self::filter_namespace(&self.jobs, namespace, |job| {
                &job.namespace
            }))
        }

        async fn fetch_cronjobs(&self, namespace: Option<&str>) -> Result<Vec<CronJobInfo>> {
            self.bump(&self.fetch_counters.workload_calls);
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
            self.bump(&self.fetch_counters.config_calls);
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
            self.bump(&self.fetch_counters.config_calls);
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
            self.bump(&self.fetch_counters.config_calls);
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
            self.bump(&self.fetch_counters.security_calls);
            self.fetch_counters
                .service_accounts
                .fetch_add(1, Ordering::Relaxed);
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
            self.bump(&self.fetch_counters.security_calls);
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
            self.bump(&self.fetch_counters.security_calls);
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
            self.bump(&self.fetch_counters.security_calls);
            if let Some(err) = &self.cluster_roles_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(self.cluster_roles.clone())
        }

        async fn fetch_cluster_role_bindings(&self) -> Result<Vec<ClusterRoleBindingInfo>> {
            self.bump(&self.fetch_counters.security_calls);
            if let Some(err) = &self.cluster_role_bindings_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(self.cluster_role_bindings.clone())
        }

        async fn fetch_custom_resource_definitions(
            &self,
        ) -> Result<Vec<CustomResourceDefinitionInfo>> {
            self.bump(&self.fetch_counters.extension_calls);
            Ok(self.custom_resource_definitions.clone())
        }

        async fn fetch_cluster_version(&self) -> Result<ClusterVersionInfo> {
            self.bump(&self.fetch_counters.cluster_version);
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.cluster_info_err {
                return Err(anyhow!(err.clone()));
            }
            let info = self
                .cluster_info
                .as_ref()
                .ok_or_else(|| anyhow!("cluster info missing"))?;
            Ok(ClusterVersionInfo {
                git_version: info.git_version.clone().unwrap_or_default(),
                platform: info.platform.clone().unwrap_or_default(),
            })
        }

        async fn fetch_cluster_pod_count(&self) -> Result<usize> {
            self.bump(&self.fetch_counters.cluster_pod_count);
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.cluster_info_err {
                return Err(anyhow!(err.clone()));
            }
            let info = self
                .cluster_info
                .as_ref()
                .ok_or_else(|| anyhow!("cluster info missing"))?;
            Ok(info.pod_count)
        }

        async fn fetch_endpoints(&self, _namespace: Option<&str>) -> Result<Vec<EndpointInfo>> {
            self.bump(&self.fetch_counters.network_calls);
            Ok(vec![])
        }
        async fn fetch_ingresses(&self, _namespace: Option<&str>) -> Result<Vec<IngressInfo>> {
            self.bump(&self.fetch_counters.network_calls);
            Ok(vec![])
        }
        async fn fetch_ingress_classes(&self) -> Result<Vec<IngressClassInfo>> {
            self.bump(&self.fetch_counters.network_calls);
            Ok(vec![])
        }
        async fn fetch_network_policies(
            &self,
            _namespace: Option<&str>,
        ) -> Result<Vec<NetworkPolicyInfo>> {
            self.bump(&self.fetch_counters.network_calls);
            Ok(vec![])
        }
        async fn fetch_config_maps(&self, _namespace: Option<&str>) -> Result<Vec<ConfigMapInfo>> {
            self.bump(&self.fetch_counters.config_calls);
            if let Some(err) = &self.config_maps_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(vec![])
        }
        async fn fetch_secrets(&self, _namespace: Option<&str>) -> Result<Vec<SecretInfo>> {
            self.bump(&self.fetch_counters.config_calls);
            if let Some(err) = &self.secrets_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(vec![])
        }
        async fn fetch_hpas(&self, _namespace: Option<&str>) -> Result<Vec<HpaInfo>> {
            self.bump(&self.fetch_counters.config_calls);
            if let Some(err) = &self.hpas_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(vec![])
        }
        async fn fetch_pvcs(&self, _namespace: Option<&str>) -> Result<Vec<PvcInfo>> {
            self.bump(&self.fetch_counters.storage_calls);
            self.fetch_counters.pvcs.fetch_add(1, Ordering::Relaxed);
            Ok(vec![])
        }
        async fn fetch_pvs(&self) -> Result<Vec<PvInfo>> {
            self.bump(&self.fetch_counters.storage_calls);
            Ok(vec![])
        }
        async fn fetch_storage_classes(&self) -> Result<Vec<StorageClassInfo>> {
            self.bump(&self.fetch_counters.storage_calls);
            Ok(vec![])
        }
        async fn fetch_namespace_list(&self) -> Result<Vec<NamespaceInfo>> {
            self.bump(&self.fetch_counters.namespaces);
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
            self.bump(&self.fetch_counters.config_calls);
            if let Some(err) = &self.priority_classes_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(vec![])
        }
        async fn fetch_helm_releases(
            &self,
            namespace: Option<&str>,
        ) -> Result<Vec<HelmReleaseInfo>> {
            self.bump(&self.fetch_counters.helm_calls);
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
            self.bump(&self.fetch_counters.flux_resources);
            Ok(Self::filter_namespace(
                &self.flux_resources,
                namespace,
                |resource| resource.namespace.as_deref().unwrap_or_default(),
            ))
        }
        async fn fetch_all_node_metrics(&self) -> Result<Vec<NodeMetricsInfo>> {
            self.bump(&self.fetch_counters.node_metrics);
            if let Some(err) = &self.node_metrics_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(vec![])
        }
        async fn fetch_all_pod_metrics(
            &self,
            _namespace: Option<&str>,
        ) -> Result<Vec<PodMetricsInfo>> {
            self.bump(&self.fetch_counters.pod_metrics);
            if let Some(err) = &self.pod_metrics_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(self.pod_metrics.clone())
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
                refresh_options(
                    RefreshScope::CORE_OVERVIEW
                        .union(RefreshScope::METRICS)
                        .union(RefreshScope::LEGACY_SECONDARY),
                    false,
                ),
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
        assert!(
            !state
                .snapshot()
                .loaded_scope
                .contains(RefreshScope::LEGACY_SECONDARY)
        );
        state.mark_refresh_requested(refresh_options(
            RefreshScope::CORE_OVERVIEW
                .union(RefreshScope::METRICS)
                .union(RefreshScope::FLUX),
            false,
        ));
        let pending_snapshot = state.snapshot();
        assert_eq!(
            pending_snapshot.view_load_state(AppView::Pods),
            ViewLoadState::Loading
        );
        assert_eq!(
            pending_snapshot.view_load_state(AppView::Issues),
            ViewLoadState::Loading
        );
        assert_eq!(
            pending_snapshot.view_load_state(AppView::NetworkPolicies),
            ViewLoadState::Idle
        );
        assert_eq!(
            pending_snapshot.view_load_state(AppView::StorageClasses),
            ViewLoadState::Idle
        );

        state
            .refresh_with_options(
                &source,
                Some("default"),
                refresh_options(
                    RefreshScope::CORE_OVERVIEW
                        .union(RefreshScope::METRICS)
                        .union(RefreshScope::FLUX),
                    false,
                ),
            )
            .await
            .expect("fast refresh should succeed");
        let snapshot = state.snapshot();
        assert!(
            !snapshot
                .loaded_scope
                .contains(RefreshScope::LEGACY_SECONDARY)
        );
        assert_eq!(
            snapshot.view_load_state(AppView::Pods),
            ViewLoadState::Ready
        );
        assert_eq!(
            snapshot.view_load_state(AppView::Issues),
            ViewLoadState::Loading
        );
        assert_eq!(
            snapshot.view_load_state(AppView::NetworkPolicies),
            ViewLoadState::Idle
        );
        assert_eq!(
            snapshot.view_load_state(AppView::StorageClasses),
            ViewLoadState::Idle
        );

        state
            .refresh_with_options(
                &source,
                Some("default"),
                refresh_options(
                    RefreshScope::CORE_OVERVIEW
                        .union(RefreshScope::METRICS)
                        .union(RefreshScope::LEGACY_SECONDARY)
                        .union(RefreshScope::FLUX),
                    false,
                ),
            )
            .await
            .expect("full refresh should succeed");
        let snapshot = state.snapshot();
        assert!(
            snapshot
                .loaded_scope
                .contains(RefreshScope::LEGACY_SECONDARY)
        );
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
    async fn refresh_skip_core_preserves_core_and_updates_secondary() {
        let mut state = GlobalState::default();
        let source = MockDataSource::success();

        // First: full refresh to populate everything.
        state
            .refresh_with_options(
                &source,
                Some("default"),
                refresh_options(
                    RefreshScope::CORE_OVERVIEW
                        .union(RefreshScope::METRICS)
                        .union(RefreshScope::LEGACY_SECONDARY)
                        .union(RefreshScope::FLUX),
                    false,
                ),
            )
            .await
            .expect("initial full refresh should succeed");

        let initial = state.snapshot();
        let initial_pods_len = initial.pods.len();
        let initial_nodes_len = initial.nodes.len();
        assert!(
            initial
                .loaded_scope
                .contains(RefreshScope::LEGACY_SECONDARY)
        );

        // Create an updated source with different secondary data but same core.
        let mut updated = source.clone();
        updated.service_accounts = vec![]; // clear secondary

        // Refresh with skip_core: core fields should stay, secondary updates.
        state
            .refresh_with_options(
                &updated,
                Some("default"),
                refresh_options(RefreshScope::LEGACY_SECONDARY, true),
            )
            .await
            .expect("skip_core refresh should succeed");

        let after = state.snapshot();
        // Core data preserved (came from prev snapshot, not re-fetched).
        assert_eq!(after.pods.len(), initial_pods_len);
        assert_eq!(after.nodes.len(), initial_nodes_len);
        // Secondary data updated (service_accounts cleared in updated source).
        assert!(after.service_accounts.is_empty());
    }

    #[tokio::test]
    async fn refresh_skip_core_counts_only_attempted_fetches_for_health() {
        let mut state = GlobalState::default();
        let source = MockDataSource::success();

        state
            .refresh_with_options(
                &source,
                Some("default"),
                refresh_options(
                    RefreshScope::CORE_OVERVIEW
                        .union(RefreshScope::METRICS)
                        .union(RefreshScope::LEGACY_SECONDARY)
                        .union(RefreshScope::FLUX),
                    false,
                ),
            )
            .await
            .expect("initial full refresh should succeed");

        let mut failing = source.clone();
        failing.cluster_info_err = Some("cluster down".to_string());
        failing.node_metrics_err = Some("node metrics down".to_string());
        failing.pod_metrics_err = Some("pod metrics down".to_string());

        state
            .refresh_with_options(
                &failing,
                Some("default"),
                refresh_options(RefreshScope::METRICS, true),
            )
            .await
            .expect("skip_core refresh should preserve prior data on failure");

        let snapshot = state.snapshot();
        assert_eq!(snapshot.failed_resource_count, 3);
        assert_eq!(snapshot.connection_health, ConnectionHealth::Disconnected);
        let last_error = snapshot.last_error.as_deref().unwrap_or_default();
        assert!(last_error.contains("cluster info: cluster down"));
        assert!(last_error.contains("nodemetrics: node metrics down"));
        assert!(last_error.contains("podmetrics: pod metrics down"));
    }

    #[tokio::test]
    async fn refresh_with_metrics_fetches_core_lists_only_once() {
        let mut state = GlobalState::default();
        let source = MockDataSource::success();
        let counters = Arc::clone(&source.fetch_counters);

        state
            .refresh_with_options(
                &source,
                Some("default"),
                refresh_options(
                    RefreshScope::NODES
                        .union(RefreshScope::PODS)
                        .union(RefreshScope::METRICS),
                    false,
                ),
            )
            .await
            .expect("metrics refresh should succeed");

        assert_eq!(counters.nodes.load(Ordering::Relaxed), 1);
        assert_eq!(counters.pods.load(Ordering::Relaxed), 1);
        assert_eq!(counters.cluster_version.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn refresh_with_cluster_info_keeps_cluster_wide_pod_count_when_namespaced() {
        let mut state = GlobalState::default();
        let source = MockDataSource::success();
        let counters = Arc::clone(&source.fetch_counters);

        state
            .refresh_with_options(
                &source,
                Some("default"),
                RefreshOptions {
                    scope: RefreshScope::CORE_OVERVIEW.union(RefreshScope::METRICS),
                    include_cluster_info: true,
                    skip_core: false,
                },
            )
            .await
            .expect("namespaced refresh should succeed");

        let snapshot = state.snapshot();
        assert_eq!(snapshot.pods.len(), 1);
        assert_eq!(
            snapshot.cluster_info.as_ref().map(|info| info.pod_count),
            Some(2)
        );
        assert_eq!(counters.cluster_version.load(Ordering::Relaxed), 1);
        assert_eq!(counters.cluster_pod_count.load(Ordering::Relaxed), 1);
    }

    #[derive(Debug, Clone, Copy)]
    struct ExpectedFetchCounts {
        total: usize,
        nodes: usize,
        pods: usize,
        services: usize,
        namespaces: usize,
        workload_calls: usize,
        network_calls: usize,
        config_calls: usize,
        storage_calls: usize,
        security_calls: usize,
        helm_calls: usize,
        extension_calls: usize,
        flux_resources: usize,
        cluster_version: usize,
        cluster_pod_count: usize,
        node_metrics: usize,
        pod_metrics: usize,
        service_accounts: usize,
        pvcs: usize,
    }

    fn assert_fetch_counts(
        scenario: &str,
        counters: &MockFetchCounters,
        expected: ExpectedFetchCounts,
    ) {
        assert_eq!(
            counters.total.load(Ordering::Relaxed),
            expected.total,
            "{scenario}: total API calls"
        );
        assert_eq!(
            counters.nodes.load(Ordering::Relaxed),
            expected.nodes,
            "{scenario}: nodes"
        );
        assert_eq!(
            counters.pods.load(Ordering::Relaxed),
            expected.pods,
            "{scenario}: pods"
        );
        assert_eq!(
            counters.services.load(Ordering::Relaxed),
            expected.services,
            "{scenario}: services"
        );
        assert_eq!(
            counters.namespaces.load(Ordering::Relaxed),
            expected.namespaces,
            "{scenario}: namespaces"
        );
        assert_eq!(
            counters.workload_calls.load(Ordering::Relaxed),
            expected.workload_calls,
            "{scenario}: workload calls"
        );
        assert_eq!(
            counters.network_calls.load(Ordering::Relaxed),
            expected.network_calls,
            "{scenario}: network bucket"
        );
        assert_eq!(
            counters.config_calls.load(Ordering::Relaxed),
            expected.config_calls,
            "{scenario}: config bucket"
        );
        assert_eq!(
            counters.storage_calls.load(Ordering::Relaxed),
            expected.storage_calls,
            "{scenario}: storage bucket"
        );
        assert_eq!(
            counters.security_calls.load(Ordering::Relaxed),
            expected.security_calls,
            "{scenario}: security bucket"
        );
        assert_eq!(
            counters.helm_calls.load(Ordering::Relaxed),
            expected.helm_calls,
            "{scenario}: helm bucket"
        );
        assert_eq!(
            counters.extension_calls.load(Ordering::Relaxed),
            expected.extension_calls,
            "{scenario}: extension bucket"
        );
        assert_eq!(
            counters.flux_resources.load(Ordering::Relaxed),
            expected.flux_resources,
            "{scenario}: flux resources"
        );
        assert_eq!(
            counters.cluster_version.load(Ordering::Relaxed),
            expected.cluster_version,
            "{scenario}: cluster version"
        );
        assert_eq!(
            counters.cluster_pod_count.load(Ordering::Relaxed),
            expected.cluster_pod_count,
            "{scenario}: cluster pod count"
        );
        assert_eq!(
            counters.node_metrics.load(Ordering::Relaxed),
            expected.node_metrics,
            "{scenario}: node metrics"
        );
        assert_eq!(
            counters.pod_metrics.load(Ordering::Relaxed),
            expected.pod_metrics,
            "{scenario}: pod metrics"
        );
        assert_eq!(
            counters.service_accounts.load(Ordering::Relaxed),
            expected.service_accounts,
            "{scenario}: service accounts"
        );
        assert_eq!(
            counters.pvcs.load(Ordering::Relaxed),
            expected.pvcs,
            "{scenario}: pvcs"
        );
    }

    #[tokio::test]
    async fn view_refresh_scopes_limit_api_calls_to_expected_buckets() {
        let scenarios = [
            (
                "dashboard",
                RefreshScope::CORE_OVERVIEW.union(RefreshScope::METRICS),
                ExpectedFetchCounts {
                    total: 14,
                    nodes: 1,
                    pods: 1,
                    services: 1,
                    namespaces: 1,
                    workload_calls: 7,
                    network_calls: 0,
                    config_calls: 0,
                    storage_calls: 0,
                    security_calls: 0,
                    helm_calls: 0,
                    extension_calls: 0,
                    flux_resources: 0,
                    cluster_version: 1,
                    cluster_pod_count: 0,
                    node_metrics: 1,
                    pod_metrics: 1,
                    service_accounts: 0,
                    pvcs: 0,
                },
            ),
            (
                "pods",
                RefreshScope::PODS.union(RefreshScope::METRICS),
                ExpectedFetchCounts {
                    total: 4,
                    nodes: 0,
                    pods: 1,
                    services: 0,
                    namespaces: 0,
                    workload_calls: 0,
                    network_calls: 0,
                    config_calls: 0,
                    storage_calls: 0,
                    security_calls: 0,
                    helm_calls: 0,
                    extension_calls: 0,
                    flux_resources: 0,
                    cluster_version: 1,
                    cluster_pod_count: 0,
                    node_metrics: 1,
                    pod_metrics: 1,
                    service_accounts: 0,
                    pvcs: 0,
                },
            ),
            (
                "nodes",
                RefreshScope::NODES.union(RefreshScope::METRICS),
                ExpectedFetchCounts {
                    total: 4,
                    nodes: 1,
                    pods: 0,
                    services: 0,
                    namespaces: 0,
                    workload_calls: 0,
                    network_calls: 0,
                    config_calls: 0,
                    storage_calls: 0,
                    security_calls: 0,
                    helm_calls: 0,
                    extension_calls: 0,
                    flux_resources: 0,
                    cluster_version: 1,
                    cluster_pod_count: 0,
                    node_metrics: 1,
                    pod_metrics: 1,
                    service_accounts: 0,
                    pvcs: 0,
                },
            ),
            (
                "services",
                RefreshScope::SERVICES.union(RefreshScope::NETWORK),
                ExpectedFetchCounts {
                    total: 5,
                    nodes: 0,
                    pods: 0,
                    services: 1,
                    namespaces: 0,
                    workload_calls: 0,
                    network_calls: 4,
                    config_calls: 0,
                    storage_calls: 0,
                    security_calls: 0,
                    helm_calls: 0,
                    extension_calls: 0,
                    flux_resources: 0,
                    cluster_version: 0,
                    cluster_pod_count: 0,
                    node_metrics: 0,
                    pod_metrics: 0,
                    service_accounts: 0,
                    pvcs: 0,
                },
            ),
            (
                "service_accounts",
                RefreshScope::SECURITY,
                ExpectedFetchCounts {
                    total: 5,
                    nodes: 0,
                    pods: 0,
                    services: 0,
                    namespaces: 0,
                    workload_calls: 0,
                    network_calls: 0,
                    config_calls: 0,
                    storage_calls: 0,
                    security_calls: 5,
                    helm_calls: 0,
                    extension_calls: 0,
                    flux_resources: 0,
                    cluster_version: 0,
                    cluster_pod_count: 0,
                    node_metrics: 0,
                    pod_metrics: 0,
                    service_accounts: 1,
                    pvcs: 0,
                },
            ),
            (
                "pvcs",
                RefreshScope::STORAGE,
                ExpectedFetchCounts {
                    total: 3,
                    nodes: 0,
                    pods: 0,
                    services: 0,
                    namespaces: 0,
                    workload_calls: 0,
                    network_calls: 0,
                    config_calls: 0,
                    storage_calls: 3,
                    security_calls: 0,
                    helm_calls: 0,
                    extension_calls: 0,
                    flux_resources: 0,
                    cluster_version: 0,
                    cluster_pod_count: 0,
                    node_metrics: 0,
                    pod_metrics: 0,
                    service_accounts: 0,
                    pvcs: 1,
                },
            ),
            (
                "helm_charts",
                RefreshScope::LOCAL_HELM_REPOSITORIES,
                ExpectedFetchCounts {
                    total: 0,
                    nodes: 0,
                    pods: 0,
                    services: 0,
                    namespaces: 0,
                    workload_calls: 0,
                    network_calls: 0,
                    config_calls: 0,
                    storage_calls: 0,
                    security_calls: 0,
                    helm_calls: 0,
                    extension_calls: 0,
                    flux_resources: 0,
                    cluster_version: 0,
                    cluster_pod_count: 0,
                    node_metrics: 0,
                    pod_metrics: 0,
                    service_accounts: 0,
                    pvcs: 0,
                },
            ),
            (
                "flux",
                RefreshScope::FLUX,
                ExpectedFetchCounts {
                    total: 1,
                    nodes: 0,
                    pods: 0,
                    services: 0,
                    namespaces: 0,
                    workload_calls: 0,
                    network_calls: 0,
                    config_calls: 0,
                    storage_calls: 0,
                    security_calls: 0,
                    helm_calls: 0,
                    extension_calls: 0,
                    flux_resources: 1,
                    cluster_version: 0,
                    cluster_pod_count: 0,
                    node_metrics: 0,
                    pod_metrics: 0,
                    service_accounts: 0,
                    pvcs: 0,
                },
            ),
            (
                "issues",
                RefreshScope::CORE_OVERVIEW
                    .union(RefreshScope::LEGACY_SECONDARY)
                    .union(RefreshScope::FLUX),
                ExpectedFetchCounts {
                    total: 33,
                    nodes: 1,
                    pods: 1,
                    services: 1,
                    namespaces: 1,
                    workload_calls: 7,
                    network_calls: 4,
                    config_calls: 7,
                    storage_calls: 3,
                    security_calls: 5,
                    helm_calls: 1,
                    extension_calls: 1,
                    flux_resources: 1,
                    cluster_version: 0,
                    cluster_pod_count: 0,
                    node_metrics: 0,
                    pod_metrics: 0,
                    service_accounts: 1,
                    pvcs: 1,
                },
            ),
        ];

        for (scenario, scope, expected) in scenarios {
            let mut state = GlobalState::default();
            let source = MockDataSource::success();
            let counters = Arc::clone(&source.fetch_counters);

            state
                .refresh_with_options(&source, Some("default"), refresh_options(scope, false))
                .await
                .unwrap_or_else(|err| panic!("{scenario}: refresh should succeed: {err}"));

            assert_fetch_counts(scenario, &counters, expected);
        }
    }

    #[tokio::test]
    async fn local_helm_repositories_refresh_marks_view_ready_without_touching_cluster_health() {
        let mut state = GlobalState::default();
        let source = MockDataSource::success();

        state.begin_loading_transition(false);
        state.mark_refresh_requested(refresh_options(
            RefreshScope::LOCAL_HELM_REPOSITORIES,
            false,
        ));
        assert_eq!(
            state.snapshot().view_load_state(AppView::HelmCharts),
            ViewLoadState::Loading
        );

        {
            let snap = Arc::make_mut(&mut state.snapshot);
            snap.connection_health = ConnectionHealth::Degraded(2);
        }

        state
            .refresh_with_options(
                &source,
                Some("default"),
                refresh_options(RefreshScope::LOCAL_HELM_REPOSITORIES, false),
            )
            .await
            .expect("local helm repositories refresh should succeed");

        let snapshot = state.snapshot();
        assert_eq!(
            snapshot.view_load_state(AppView::HelmCharts),
            ViewLoadState::Ready
        );
        assert!(
            snapshot
                .loaded_scope
                .contains(RefreshScope::LOCAL_HELM_REPOSITORIES)
        );
        assert_eq!(snapshot.connection_health, ConnectionHealth::Degraded(2));
    }

    #[test]
    fn resource_count_stays_unknown_until_view_scope_is_loaded() {
        let snapshot = ClusterSnapshot::default();
        assert_eq!(snapshot.resource_count(AppView::NetworkPolicies), None);
        assert_eq!(snapshot.resource_count(AppView::Pods), None);

        let mut snapshot = ClusterSnapshot {
            loaded_scope: RefreshScope::PODS,
            ..ClusterSnapshot::default()
        };
        snapshot.pods.push(PodInfo::default());

        assert_eq!(snapshot.resource_count(AppView::Pods), Some(1));
        assert_eq!(snapshot.resource_count(AppView::NetworkPolicies), None);
    }

    #[test]
    fn optimistic_delete_removes_resource_from_snapshot_immediately() {
        let mut state = GlobalState::default();
        let snap = Arc::make_mut(&mut state.snapshot);
        snap.pods = vec![
            PodInfo {
                name: "pod-a".to_string(),
                namespace: "default".to_string(),
                ..PodInfo::default()
            },
            PodInfo {
                name: "pod-b".to_string(),
                namespace: "default".to_string(),
                ..PodInfo::default()
            },
        ];
        snap.services = vec![ServiceInfo {
            name: "svc-a".to_string(),
            namespace: "default".to_string(),
            ..ServiceInfo::default()
        }];
        snap.deployments = vec![DeploymentInfo {
            name: "deploy-a".to_string(),
            namespace: "default".to_string(),
            ..DeploymentInfo::default()
        }];
        snap.services_count = 1;
        snap.namespaces_count = 1;
        snap.snapshot_version = 41;
        state.snapshot_dirty = true;
        state.publish_snapshot();

        state.apply_optimistic_delete(&ResourceRef::Pod(
            "pod-a".to_string(),
            "default".to_string(),
        ));

        let snapshot = state.snapshot();
        assert_eq!(snapshot.pods.len(), 1);
        assert_eq!(snapshot.pods[0].name, "pod-b");
        assert_eq!(snapshot.snapshot_version, 42);
        assert_eq!(snapshot.namespaces_count, 1);
    }

    #[test]
    fn optimistic_scale_updates_workload_replica_targets_immediately() {
        let mut state = GlobalState::default();
        let snap = Arc::make_mut(&mut state.snapshot);
        snap.deployments = vec![DeploymentInfo {
            name: "deploy-a".to_string(),
            namespace: "default".to_string(),
            desired_replicas: 2,
            ready_replicas: 1,
            ready: "1/2".to_string(),
            ..DeploymentInfo::default()
        }];
        snap.statefulsets = vec![StatefulSetInfo {
            name: "db".to_string(),
            namespace: "default".to_string(),
            desired_replicas: 3,
            ready_replicas: 3,
            ..StatefulSetInfo::default()
        }];
        snap.snapshot_version = 10;
        state.snapshot_dirty = true;
        state.publish_snapshot();

        state.apply_optimistic_scale(
            &ResourceRef::Deployment("deploy-a".to_string(), "default".to_string()),
            5,
        );
        state.apply_optimistic_scale(
            &ResourceRef::StatefulSet("db".to_string(), "default".to_string()),
            1,
        );

        let snapshot = state.snapshot();
        assert_eq!(snapshot.deployments[0].desired_replicas, 5);
        assert_eq!(snapshot.deployments[0].ready, "1/5");
        assert_eq!(snapshot.statefulsets[0].desired_replicas, 1);
        assert_eq!(snapshot.snapshot_version, 12);
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
            context: Some("broken".to_string()),
            fetch_counters: Arc::new(MockFetchCounters::default()),
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
            pod_metrics: vec![],
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
            node_metrics_err: Some("node metrics down".to_string()),
            pod_metrics_err: Some("pod metrics down".to_string()),
            config_maps_err: Some("configmaps down".to_string()),
            secrets_err: Some("secrets down".to_string()),
            hpas_err: Some("hpas down".to_string()),
            priority_classes_err: Some("priorityclasses down".to_string()),
            delay_ms: 0,
        };

        let result = state
            .refresh_with_options(
                &source,
                None,
                refresh_options(
                    RefreshScope::NODES
                        .union(RefreshScope::PODS)
                        .union(RefreshScope::SERVICES)
                        .union(RefreshScope::DEPLOYMENTS)
                        .union(RefreshScope::STATEFULSETS)
                        .union(RefreshScope::DAEMONSETS)
                        .union(RefreshScope::JOBS)
                        .union(RefreshScope::CRONJOBS)
                        .union(RefreshScope::METRICS)
                        .union(RefreshScope::CONFIG)
                        .union(RefreshScope::SECURITY),
                    false,
                ),
            )
            .await;
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
        let result: Result<i32> =
            GlobalState::fetch_with_timeout("pods", &CORE_FETCH_SEMAPHORE, move || {
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
        Arc::make_mut(&mut state.snapshot).phase = DataPhase::Loading;
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
        let snap = Arc::make_mut(&mut state.snapshot);
        snap.snapshot_version = 7;
        snap.pods = vec![PodInfo {
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

    fn make_pod_info(name: &str) -> PodInfo {
        PodInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn apply_watch_update_updates_pods() {
        let mut state = GlobalState::default();
        let initial_version = state.snapshot.snapshot_version;

        let update = watch::WatchUpdate {
            resource: watch::WatchedResource::Pods,
            context_generation: 0,
            data: watch::WatchPayload::Pods(vec![make_pod_info("pod-a"), make_pod_info("pod-b")]),
        };

        state.apply_watch_update(update);

        assert_eq!(state.snapshot.pods.len(), 2);
        assert_eq!(state.snapshot.pods[0].name, "pod-a");
        assert!(state.snapshot.snapshot_version > initial_version);
    }

    #[test]
    fn apply_watch_update_identical_data_no_version_bump() {
        let mut state = GlobalState::default();
        let pods = vec![make_pod_info("pod-a")];

        // First update
        let update = watch::WatchUpdate {
            resource: watch::WatchedResource::Pods,
            context_generation: 0,
            data: watch::WatchPayload::Pods(pods.clone()),
        };
        state.apply_watch_update(update);
        let version_after_first = state.snapshot.snapshot_version;

        // Second identical update
        let update = watch::WatchUpdate {
            resource: watch::WatchedResource::Pods,
            context_generation: 0,
            data: watch::WatchPayload::Pods(pods),
        };
        state.apply_watch_update(update);
        assert_eq!(state.snapshot.snapshot_version, version_after_first);
    }

    #[test]
    fn apply_watch_update_sets_pod_view_ready() {
        let mut state = GlobalState::default();
        assert_eq!(
            state.snapshot.view_load_states[AppView::Pods.index()],
            ViewLoadState::Idle
        );

        let update = watch::WatchUpdate {
            resource: watch::WatchedResource::Pods,
            context_generation: 0,
            data: watch::WatchPayload::Pods(vec![make_pod_info("pod-a")]),
        };
        state.apply_watch_update(update);
        assert_eq!(
            state.snapshot.view_load_states[AppView::Pods.index()],
            ViewLoadState::Ready
        );
    }
}
