//! Global state management for KubecTUI.

pub mod alerts;
mod fetch;
pub mod issues;
mod optimistic;
pub mod port_forward;
mod refresh;
pub mod vulnerabilities;
pub mod watch;

use crate::time::AppTimestamp;
use anyhow::Result;
use async_trait::async_trait;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    hash::{Hash, Hasher},
    sync::Arc,
};

use crate::app::AppView;
use crate::governance::compute_governance;
use crate::k8s::{
    client::{FluxWatchTarget, K8sClient},
    dtos::{
        ClusterInfo, ClusterRoleBindingInfo, ClusterRoleInfo, ClusterVersionInfo, ConfigMapInfo,
        CronJobInfo, CustomResourceDefinitionInfo, DaemonSetInfo, DeploymentInfo, EndpointInfo,
        FluxResourceInfo, GatewayClassInfo, GatewayInfo, GrpcRouteInfo, HelmReleaseInfo, HpaInfo,
        HttpRouteInfo, IngressClassInfo, IngressInfo, JobInfo, K8sEventInfo, LimitRangeInfo,
        NamespaceInfo, NetworkPolicyInfo, NodeInfo, NodeMetricsInfo, PodDisruptionBudgetInfo,
        PodInfo, PodMetricsInfo, PriorityClassInfo, PvInfo, PvcInfo, ReferenceGrantInfo,
        ReplicaSetInfo, ReplicationControllerInfo, ResourceQuotaInfo, RoleBindingInfo, RoleInfo,
        SecretInfo, ServiceAccountInfo, ServiceInfo, StatefulSetInfo, StorageClassInfo,
        VulnerabilityReportInfo,
    },
};
use crate::projects::compute_projects;

fn flux_resource_matches_target(resource: &FluxResourceInfo, target: FluxWatchTarget) -> bool {
    resource.group == target.group
        && resource.version == target.version
        && resource.kind == target.kind
        && resource.plural == target.plural
}

fn sort_flux_resources(resources: &mut [FluxResourceInfo]) {
    resources.sort_unstable_by(|left, right| {
        left.namespace
            .cmp(&right.namespace)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.name.cmp(&right.name))
    });
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FluxResourceTargetKey {
    group: String,
    version: String,
    kind: String,
    plural: String,
}

impl FluxResourceTargetKey {
    pub fn new(group: &str, version: &str, kind: &str, plural: &str) -> Self {
        Self {
            group: group.to_string(),
            version: version.to_string(),
            kind: kind.to_string(),
            plural: plural.to_string(),
        }
    }
}

pub type FluxTargetFingerprints = BTreeMap<FluxResourceTargetKey, u64>;

fn flux_resource_target_key(resource: &FluxResourceInfo) -> FluxResourceTargetKey {
    FluxResourceTargetKey::new(
        &resource.group,
        &resource.version,
        &resource.kind,
        &resource.plural,
    )
}

fn hash_flux_resource_stable(resource: &FluxResourceInfo, hasher: &mut impl Hasher) {
    resource.name.hash(hasher);
    resource.namespace.hash(hasher);
    resource.kind.hash(hasher);
    resource.group.hash(hasher);
    resource.version.hash(hasher);
    resource.plural.hash(hasher);
    resource.source_url.hash(hasher);
    resource.status.hash(hasher);
    resource.message.hash(hasher);
    resource.artifact.hash(hasher);
    resource.suspended.hash(hasher);
    resource
        .created_at
        .map(|timestamp| timestamp.to_string())
        .hash(hasher);
    for condition in &resource.conditions {
        condition.type_.hash(hasher);
        condition.status.hash(hasher);
        condition.reason.hash(hasher);
        condition.message.hash(hasher);
        condition
            .timestamp
            .map(|timestamp| timestamp.to_string())
            .hash(hasher);
    }
    resource
        .last_reconcile_time
        .map(|timestamp| timestamp.to_string())
        .hash(hasher);
    resource.last_applied_revision.hash(hasher);
    resource.last_attempted_revision.hash(hasher);
    resource.observed_generation.hash(hasher);
    resource.generation.hash(hasher);
    resource.source_ref.hash(hasher);
    resource.interval.hash(hasher);
    resource.timeout.hash(hasher);
}

fn flux_resources_fingerprint(resources: &[FluxResourceInfo]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    resources.len().hash(&mut hasher);
    for resource in resources {
        hash_flux_resource_stable(resource, &mut hasher);
    }
    hasher.finish()
}

fn flux_target_fingerprints_from_resources(
    resources: &[FluxResourceInfo],
) -> FluxTargetFingerprints {
    let mut grouped: BTreeMap<FluxResourceTargetKey, Vec<&FluxResourceInfo>> = BTreeMap::new();
    for resource in resources {
        grouped
            .entry(flux_resource_target_key(resource))
            .or_default()
            .push(resource);
    }

    let mut fingerprints = FluxTargetFingerprints::new();
    for (target, mut target_resources) in grouped {
        target_resources.sort_unstable_by(|left, right| {
            left.namespace
                .cmp(&right.namespace)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.name.cmp(&right.name))
        });

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        target_resources.len().hash(&mut hasher);
        for resource in target_resources {
            hash_flux_resource_stable(resource, &mut hasher);
        }
        fingerprints.insert(target, hasher.finish());
    }
    fingerprints
}

fn changed_flux_targets(
    start: &FluxTargetFingerprints,
    current: &FluxTargetFingerprints,
) -> BTreeSet<FluxResourceTargetKey> {
    start
        .keys()
        .chain(current.keys())
        .filter(|target| start.get(*target) != current.get(*target))
        .cloned()
        .collect()
}

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

/// Pre-computed Flux resource counts to avoid per-frame iteration over `flux_resources`.
#[derive(Debug, Clone, Default)]
pub struct FluxCounts {
    pub kustomizations: usize,
    pub helm_releases: usize,
    pub helm_repositories: usize,
    pub alert_providers: usize,
    pub alerts: usize,
    pub artifacts: usize,
    pub images: usize,
    pub receivers: usize,
    pub sources: usize,
}

impl FluxCounts {
    pub fn compute(resources: &[FluxResourceInfo]) -> Self {
        let mut counts = Self::default();
        for r in resources {
            // Group-level counters (non-exclusive with kind-level counters below).
            match r.group.as_str() {
                "source.toolkit.fluxcd.io" => counts.sources += 1,
                "image.toolkit.fluxcd.io" => counts.images += 1,
                _ => {}
            }
            // Kind-level counters.
            match (r.group.as_str(), r.kind.as_str()) {
                ("kustomize.toolkit.fluxcd.io", "Kustomization") => counts.kustomizations += 1,
                ("helm.toolkit.fluxcd.io", "HelmRelease") => counts.helm_releases += 1,
                ("source.toolkit.fluxcd.io", "HelmRepository") => counts.helm_repositories += 1,
                ("notification.toolkit.fluxcd.io", "AlertProvider") => counts.alert_providers += 1,
                ("notification.toolkit.fluxcd.io", "Alert") => counts.alerts += 1,
                ("notification.toolkit.fluxcd.io", "Receiver") => counts.receivers += 1,
                _ => {}
            }
            if r.artifact.is_some() {
                counts.artifacts += 1;
            }
        }
        counts
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
    pub vulnerability_reports: Vec<VulnerabilityReportInfo>,
    pub custom_resource_definitions: Vec<CustomResourceDefinitionInfo>,
    pub cluster_info: Option<ClusterInfo>,
    pub endpoints: Vec<EndpointInfo>,
    pub ingresses: Vec<IngressInfo>,
    pub ingress_classes: Vec<IngressClassInfo>,
    pub gateway_classes: Vec<GatewayClassInfo>,
    pub gateways: Vec<GatewayInfo>,
    pub http_routes: Vec<HttpRouteInfo>,
    pub grpc_routes: Vec<GrpcRouteInfo>,
    pub reference_grants: Vec<ReferenceGrantInfo>,
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
    pub flux_counts: FluxCounts,
    pub helm_repositories: Vec<crate::k8s::dtos::HelmRepoInfo>,
    pub node_metrics: Vec<NodeMetricsInfo>,
    pub pod_metrics: Vec<PodMetricsInfo>,
    pub issue_count: usize,
    pub sanitizer_count: usize,
    pub vulnerability_count: usize,
    pub namespaces_count: usize,
    pub phase: DataPhase,
    pub last_updated: Option<AppTimestamp>,
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
            vulnerability_reports: Vec::new(),
            custom_resource_definitions: Vec::new(),
            cluster_info: None,
            endpoints: Vec::new(),
            ingresses: Vec::new(),
            ingress_classes: Vec::new(),
            gateway_classes: Vec::new(),
            gateways: Vec::new(),
            http_routes: Vec::new(),
            grpc_routes: Vec::new(),
            reference_grants: Vec::new(),
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
            flux_counts: FluxCounts::default(),
            helm_repositories: Vec::new(),
            node_metrics: Vec::new(),
            pod_metrics: Vec::new(),
            issue_count: 0,
            sanitizer_count: 0,
            vulnerability_count: 0,
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
        if !required_scope.is_empty()
            && !self.loaded_scope.contains(required_scope)
            && self.view_load_state(view) != ViewLoadState::Ready
        {
            return None;
        }

        match view {
            AppView::Projects => Some(compute_projects(self).len()),
            AppView::Governance => Some(compute_governance(self).len()),
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
            AppView::GatewayClasses => Some(self.gateway_classes.len()),
            AppView::Gateways => Some(self.gateways.len()),
            AppView::HttpRoutes => Some(self.http_routes.len()),
            AppView::GrpcRoutes => Some(self.grpc_routes.len()),
            AppView::ReferenceGrants => Some(self.reference_grants.len()),
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
            AppView::FluxCDKustomizations => Some(self.flux_counts.kustomizations),
            AppView::FluxCDHelmReleases => Some(self.flux_counts.helm_releases),
            AppView::FluxCDHelmRepositories => Some(self.flux_counts.helm_repositories),
            AppView::FluxCDAlertProviders => Some(self.flux_counts.alert_providers),
            AppView::FluxCDAlerts => Some(self.flux_counts.alerts),
            AppView::FluxCDArtifacts => Some(self.flux_counts.artifacts),
            AppView::FluxCDImages => Some(self.flux_counts.images),
            AppView::FluxCDReceivers => Some(self.flux_counts.receivers),
            AppView::FluxCDSources => Some(self.flux_counts.sources),
            AppView::Issues => Some(self.issue_count),
            AppView::HealthReport => Some(self.sanitizer_count),
            AppView::Vulnerabilities => Some(self.vulnerability_count),
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
    /// Fetches Trivy Operator vulnerability reports.
    async fn fetch_vulnerability_reports(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<VulnerabilityReportInfo>>;
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
    /// Fetches GatewayClasses when Gateway API is installed.
    async fn fetch_gateway_classes(&self) -> Result<Vec<GatewayClassInfo>>;
    /// Fetches Gateways when Gateway API is installed.
    async fn fetch_gateways(&self, namespace: Option<&str>) -> Result<Vec<GatewayInfo>>;
    /// Fetches HTTPRoutes when Gateway API is installed.
    async fn fetch_http_routes(&self, namespace: Option<&str>) -> Result<Vec<HttpRouteInfo>>;
    /// Fetches GRPCRoutes when Gateway API is installed.
    async fn fetch_grpc_routes(&self, namespace: Option<&str>) -> Result<Vec<GrpcRouteInfo>>;
    /// Fetches ReferenceGrants when Gateway API is installed.
    async fn fetch_reference_grants(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<ReferenceGrantInfo>>;
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

    async fn fetch_vulnerability_reports(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<VulnerabilityReportInfo>> {
        K8sClient::fetch_vulnerability_reports(self, namespace).await
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

    async fn fetch_gateway_classes(&self) -> Result<Vec<GatewayClassInfo>> {
        K8sClient::fetch_gateway_classes(self).await
    }

    async fn fetch_gateways(&self, namespace: Option<&str>) -> Result<Vec<GatewayInfo>> {
        K8sClient::fetch_gateways(self, namespace).await
    }

    async fn fetch_http_routes(&self, namespace: Option<&str>) -> Result<Vec<HttpRouteInfo>> {
        K8sClient::fetch_http_routes(self, namespace).await
    }

    async fn fetch_grpc_routes(&self, namespace: Option<&str>) -> Result<Vec<GrpcRouteInfo>> {
        K8sClient::fetch_grpc_routes(self, namespace).await
    }

    async fn fetch_reference_grants(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<ReferenceGrantInfo>> {
        K8sClient::fetch_reference_grants(self, namespace).await
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
    pub(super) snapshot: Arc<ClusterSnapshot>,
    pub namespaces: Vec<String>,
    pub(super) snapshot_dirty: bool,
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
            | Self::REPLICASETS.0
            | Self::REPLICATION_CONTROLLERS.0
            | Self::JOBS.0
            | Self::CRONJOBS.0
            | Self::NAMESPACES.0,
    );
    pub const DASHBOARD_WATCHED: Self = Self(
        Self::NODES.0
            | Self::PODS.0
            | Self::SERVICES.0
            | Self::DEPLOYMENTS.0
            | Self::STATEFULSETS.0
            | Self::DAEMONSETS.0
            | Self::NAMESPACES.0,
    );
    pub const CORE_OVERVIEW: Self = Self(Self::WATCHED_SCOPES.0 | Self::NAMESPACES.0);
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

impl GlobalState {
    /// Returns a cheap Arc-wrapped snapshot for UI rendering.
    /// No deep clone — just an Arc refcount bump.
    pub fn snapshot(&self) -> Arc<ClusterSnapshot> {
        self.snapshot.clone()
    }

    pub fn flux_fingerprint(&self) -> u64 {
        flux_resources_fingerprint(&self.snapshot.flux_resources)
    }

    pub fn flux_target_fingerprints(&self) -> FluxTargetFingerprints {
        flux_target_fingerprints_from_resources(&self.snapshot.flux_resources)
    }

    /// Recomputes derived fields (issue count) and clears the dirty flag.
    /// Called after every successful refresh or optimistic mutation.
    pub(super) fn publish_snapshot(&mut self) {
        if !self.snapshot_dirty {
            return;
        }
        self.snapshot_dirty = false;
        let count = issues::compute_issues(&self.snapshot).len();
        let sanitizer_count = issues::sanitizer_issue_count(&self.snapshot);
        let vulnerability_count =
            vulnerabilities::compute_vulnerability_findings(&self.snapshot).len();
        let snapshot = Arc::make_mut(&mut self.snapshot);
        snapshot.issue_count = count;
        snapshot.sanitizer_count = sanitizer_count;
        snapshot.vulnerability_count = vulnerability_count;
    }

    /// Returns fetched namespaces.
    pub fn namespaces(&self) -> &[String] {
        &self.namespaces
    }

    pub const fn view_ready_scope(view: AppView) -> RefreshScope {
        match view {
            AppView::Dashboard => RefreshScope::DASHBOARD_WATCHED,
            AppView::Projects => RefreshScope::CORE_OVERVIEW
                .union(RefreshScope::LEGACY_SECONDARY)
                .union(RefreshScope::NETWORK)
                .union(RefreshScope::SECURITY),
            AppView::Governance => RefreshScope::CORE_OVERVIEW
                .union(RefreshScope::METRICS)
                .union(RefreshScope::LEGACY_SECONDARY)
                .union(RefreshScope::NETWORK)
                .union(RefreshScope::SECURITY),
            AppView::Bookmarks | AppView::PortForwarding => RefreshScope::NONE,
            AppView::Issues | AppView::HealthReport => RefreshScope::CORE_OVERVIEW
                .union(RefreshScope::LEGACY_SECONDARY)
                .union(RefreshScope::SECURITY)
                .union(RefreshScope::FLUX),
            AppView::Vulnerabilities => RefreshScope::SECURITY,
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
            | AppView::GatewayClasses
            | AppView::Gateways
            | AppView::HttpRoutes
            | AppView::GrpcRoutes
            | AppView::ReferenceGrants
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
            AppView::Projects => !compute_projects(&self.snapshot).is_empty(),
            AppView::Governance => {
                !compute_governance(&self.snapshot).is_empty()
                    || !self.snapshot.namespace_list.is_empty()
            }
            AppView::Bookmarks => false,
            AppView::Vulnerabilities => !self.snapshot.vulnerability_reports.is_empty(),
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
            AppView::GatewayClasses => !self.snapshot.gateway_classes.is_empty(),
            AppView::Gateways => !self.snapshot.gateways.is_empty(),
            AppView::HttpRoutes => !self.snapshot.http_routes.is_empty(),
            AppView::GrpcRoutes => !self.snapshot.grpc_routes.is_empty(),
            AppView::ReferenceGrants => !self.snapshot.reference_grants.is_empty(),
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
            AppView::Issues => {
                self.snapshot.issue_count > 0
                    || !self.snapshot.pods.is_empty()
                    || !self.snapshot.nodes.is_empty()
                    || !self.snapshot.vulnerability_reports.is_empty()
            }
            AppView::HealthReport => {
                self.snapshot.sanitizer_count > 0
                    || !self.snapshot.pods.is_empty()
                    || !self.snapshot.nodes.is_empty()
            }
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

    pub fn mark_view_refresh_requested(&mut self, view: AppView) {
        let next = if self.has_view_data(view) {
            ViewLoadState::Refreshing
        } else {
            ViewLoadState::Loading
        };
        let mut changed = self.set_view_load_state(view, next);
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
        /// Applies a watched resource update to a snapshot field, bumping the
        /// version only when data actually changed. Does NOT touch view load
        /// state — that is managed by the refresh pipeline so the loading
        /// spinner can display correctly.
        macro_rules! apply_watched {
            ($snap:ident, $changed:ident, $field:ident, $items:expr, $sort:path) => {{
                let mut items = $items;
                $sort(&mut items);
                if $snap.$field != items {
                    $snap.$field = items;
                    $snap.snapshot_version = $snap.snapshot_version.saturating_add(1);
                    $changed = true;
                }
            }};
        }

        let mut changed = false;
        {
            let snap = Arc::make_mut(&mut self.snapshot);
            match update.data {
                watch::WatchPayload::Pods(items) => {
                    apply_watched!(snap, changed, pods, items, watch::sort_pods);
                }
                watch::WatchPayload::Deployments(items) => {
                    apply_watched!(snap, changed, deployments, items, watch::sort_deployments);
                }
                watch::WatchPayload::ReplicaSets(items) => {
                    apply_watched!(snap, changed, replicasets, items, watch::sort_replicasets);
                }
                watch::WatchPayload::StatefulSets(items) => {
                    apply_watched!(snap, changed, statefulsets, items, watch::sort_statefulsets);
                }
                watch::WatchPayload::DaemonSets(items) => {
                    apply_watched!(snap, changed, daemonsets, items, watch::sort_daemonsets);
                }
                watch::WatchPayload::Services(items) => {
                    apply_watched!(snap, changed, services, items, watch::sort_services);
                }
                watch::WatchPayload::Nodes(items) => {
                    apply_watched!(snap, changed, nodes, items, watch::sort_nodes);
                }
                watch::WatchPayload::ReplicationControllers(items) => {
                    apply_watched!(
                        snap,
                        changed,
                        replication_controllers,
                        items,
                        watch::sort_replication_controllers
                    );
                }
                watch::WatchPayload::Jobs(items) => {
                    apply_watched!(snap, changed, jobs, items, watch::sort_jobs);
                }
                watch::WatchPayload::CronJobs(items) => {
                    apply_watched!(snap, changed, cronjobs, items, watch::sort_cronjobs);
                }
                watch::WatchPayload::Namespaces(items) => {
                    apply_watched!(snap, changed, namespace_list, items, watch::sort_namespaces);
                }
                watch::WatchPayload::Flux { target, items } => {
                    let mut merged = Vec::with_capacity(snap.flux_resources.len() + items.len());
                    merged.extend(
                        snap.flux_resources
                            .iter()
                            .filter(|resource| !flux_resource_matches_target(resource, target))
                            .cloned(),
                    );
                    merged.extend(items);
                    sort_flux_resources(&mut merged);
                    if snap.flux_resources != merged {
                        snap.flux_resources = merged;
                        snap.flux_counts = FluxCounts::compute(&snap.flux_resources);
                        snap.snapshot_version = snap.snapshot_version.saturating_add(1);
                        changed = true;
                    }
                }
                watch::WatchPayload::Error { .. } => {
                    // Watcher errors are informational — do not clear existing
                    // snapshot data. The watch stream's built-in backoff will
                    // reconnect automatically.
                }
            }
        }
        if changed {
            self.namespaces = Self::namespace_names_from_list(&self.snapshot.namespace_list);
            self.snapshot_dirty = true;
            self.publish_snapshot();
        }
    }

    /// Preserves only Flux targets that changed on the main runtime while a
    /// refresh was in flight.
    pub fn preserve_changed_flux_targets_from_snapshot(
        &mut self,
        source: &ClusterSnapshot,
        start_fingerprints: &FluxTargetFingerprints,
    ) {
        let current_fingerprints = flux_target_fingerprints_from_resources(&source.flux_resources);
        let changed_targets = changed_flux_targets(start_fingerprints, &current_fingerprints);
        if changed_targets.is_empty() {
            return;
        }

        let mut merged =
            Vec::with_capacity(self.snapshot.flux_resources.len() + source.flux_resources.len());
        merged.extend(
            self.snapshot
                .flux_resources
                .iter()
                .filter(|resource| !changed_targets.contains(&flux_resource_target_key(resource)))
                .cloned(),
        );
        merged.extend(
            source
                .flux_resources
                .iter()
                .filter(|resource| changed_targets.contains(&flux_resource_target_key(resource)))
                .cloned(),
        );
        sort_flux_resources(&mut merged);
        if self.snapshot.flux_resources == merged {
            return;
        }

        {
            let snap = Arc::make_mut(&mut self.snapshot);
            snap.flux_resources = merged;
            snap.flux_counts = FluxCounts::compute(&snap.flux_resources);
            snap.snapshot_version = snap.snapshot_version.saturating_add(1);
        }
        self.snapshot_dirty = true;
        self.publish_snapshot();
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ResourceRef;
    use anyhow::anyhow;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use super::fetch::CORE_FETCH_SEMAPHORE;

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
        active_core_fetches: AtomicUsize,
        secondary_started_while_core_active: AtomicUsize,
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
        vulnerability_reports: Vec<VulnerabilityReportInfo>,
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
        replicasets_err: Option<String>,
        replication_controllers_err: Option<String>,
        jobs_err: Option<String>,
        cronjobs_err: Option<String>,
        namespace_list_err: Option<String>,
        resource_quotas_err: Option<String>,
        limit_ranges_err: Option<String>,
        pod_disruption_budgets_err: Option<String>,
        service_accounts_err: Option<String>,
        roles_err: Option<String>,
        role_bindings_err: Option<String>,
        cluster_roles_err: Option<String>,
        cluster_role_bindings_err: Option<String>,
        vulnerability_reports_err: Option<String>,
        cluster_info_err: Option<String>,
        node_metrics_err: Option<String>,
        pod_metrics_err: Option<String>,
        config_maps_err: Option<String>,
        secrets_err: Option<String>,
        hpas_err: Option<String>,
        priority_classes_err: Option<String>,
        delay_ms: u64,
    }

    struct ActiveCoreFetchGuard {
        counters: Arc<MockFetchCounters>,
    }

    impl ActiveCoreFetchGuard {
        fn new(counters: &Arc<MockFetchCounters>) -> Self {
            counters.active_core_fetches.fetch_add(1, Ordering::Relaxed);
            Self {
                counters: Arc::clone(counters),
            }
        }
    }

    impl Drop for ActiveCoreFetchGuard {
        fn drop(&mut self) {
            self.counters
                .active_core_fetches
                .fetch_sub(1, Ordering::Relaxed);
        }
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
                vulnerability_reports: Vec::new(),
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
                replicasets_err: None,
                replication_controllers_err: None,
                jobs_err: None,
                cronjobs_err: None,
                namespace_list_err: None,
                resource_quotas_err: None,
                limit_ranges_err: None,
                pod_disruption_budgets_err: None,
                service_accounts_err: None,
                roles_err: None,
                role_bindings_err: None,
                cluster_roles_err: None,
                cluster_role_bindings_err: None,
                vulnerability_reports_err: None,
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

        async fn delay_core_fetch(&self) {
            let _guard = ActiveCoreFetchGuard::new(&self.fetch_counters);
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
        }

        fn observe_secondary_fetch_start(&self) {
            if self
                .fetch_counters
                .active_core_fetches
                .load(Ordering::Relaxed)
                > 0
            {
                self.fetch_counters
                    .secondary_started_while_core_active
                    .fetch_add(1, Ordering::Relaxed);
            }
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
            self.delay_core_fetch().await;
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
            if let Some(err) = &self.replicasets_err {
                return Err(anyhow!(err.clone()));
            }
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
            if let Some(err) = &self.replication_controllers_err {
                return Err(anyhow!(err.clone()));
            }
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
            self.observe_secondary_fetch_start();
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
            self.observe_secondary_fetch_start();
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
            self.observe_secondary_fetch_start();
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

        async fn fetch_vulnerability_reports(
            &self,
            namespace: Option<&str>,
        ) -> Result<Vec<VulnerabilityReportInfo>> {
            self.bump(&self.fetch_counters.security_calls);
            if let Some(err) = &self.vulnerability_reports_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(Self::filter_namespace(
                &self.vulnerability_reports,
                namespace,
                |report| &report.namespace,
            ))
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
        async fn fetch_gateway_classes(&self) -> Result<Vec<GatewayClassInfo>> {
            self.bump(&self.fetch_counters.network_calls);
            Ok(vec![])
        }
        async fn fetch_gateways(&self, _namespace: Option<&str>) -> Result<Vec<GatewayInfo>> {
            self.bump(&self.fetch_counters.network_calls);
            Ok(vec![])
        }
        async fn fetch_http_routes(&self, _namespace: Option<&str>) -> Result<Vec<HttpRouteInfo>> {
            self.bump(&self.fetch_counters.network_calls);
            Ok(vec![])
        }
        async fn fetch_grpc_routes(&self, _namespace: Option<&str>) -> Result<Vec<GrpcRouteInfo>> {
            self.bump(&self.fetch_counters.network_calls);
            Ok(vec![])
        }
        async fn fetch_reference_grants(
            &self,
            _namespace: Option<&str>,
        ) -> Result<Vec<ReferenceGrantInfo>> {
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
            self.observe_secondary_fetch_start();
            self.bump(&self.fetch_counters.config_calls);
            if let Some(err) = &self.config_maps_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(vec![])
        }
        async fn fetch_secrets(&self, _namespace: Option<&str>) -> Result<Vec<SecretInfo>> {
            self.observe_secondary_fetch_start();
            self.bump(&self.fetch_counters.config_calls);
            if let Some(err) = &self.secrets_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(vec![])
        }
        async fn fetch_hpas(&self, _namespace: Option<&str>) -> Result<Vec<HpaInfo>> {
            self.observe_secondary_fetch_start();
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
            if let Some(err) = &self.namespace_list_err {
                return Err(anyhow!(err.clone()));
            }
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
            self.observe_secondary_fetch_start();
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
        assert_eq!(snapshot.services.len(), 1);
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
    async fn refresh_overlaps_core_and_secondary_fetch_waves() {
        let mut state = GlobalState::default();
        let source = MockDataSource::success().with_delay(50);
        let counters = Arc::clone(&source.fetch_counters);

        state
            .refresh_with_options(
                &source,
                Some("default"),
                refresh_options(RefreshScope::NODES.union(RefreshScope::CONFIG), false),
            )
            .await
            .expect("refresh should succeed");

        assert!(
            counters
                .secondary_started_while_core_active
                .load(Ordering::Relaxed)
                > 0,
            "secondary fetches should start while core fetches are still in flight"
        );
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
                    total: 10,
                    nodes: 0,
                    pods: 0,
                    services: 1,
                    namespaces: 0,
                    workload_calls: 0,
                    network_calls: 9,
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
                    total: 6,
                    nodes: 0,
                    pods: 0,
                    services: 0,
                    namespaces: 0,
                    workload_calls: 0,
                    network_calls: 0,
                    config_calls: 0,
                    storage_calls: 0,
                    security_calls: 6,
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
                    total: 39,
                    nodes: 1,
                    pods: 1,
                    services: 1,
                    namespaces: 1,
                    workload_calls: 7,
                    network_calls: 9,
                    config_calls: 7,
                    storage_calls: 3,
                    security_calls: 6,
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
    async fn targeted_group_view_refresh_fetches_only_selected_empty_resource() {
        let scenarios = [
            (
                "network policies",
                AppView::NetworkPolicies,
                RefreshScope::NETWORK,
                ExpectedFetchCounts {
                    total: 1,
                    nodes: 0,
                    pods: 0,
                    services: 0,
                    namespaces: 0,
                    workload_calls: 0,
                    network_calls: 1,
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
                "config maps",
                AppView::ConfigMaps,
                RefreshScope::CONFIG,
                ExpectedFetchCounts {
                    total: 1,
                    nodes: 0,
                    pods: 0,
                    services: 0,
                    namespaces: 0,
                    workload_calls: 0,
                    network_calls: 0,
                    config_calls: 1,
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
                "storage classes",
                AppView::StorageClasses,
                RefreshScope::STORAGE,
                ExpectedFetchCounts {
                    total: 1,
                    nodes: 0,
                    pods: 0,
                    services: 0,
                    namespaces: 0,
                    workload_calls: 0,
                    network_calls: 0,
                    config_calls: 0,
                    storage_calls: 1,
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
        ];

        for (scenario, view, scope, expected) in scenarios {
            let mut state = GlobalState::default();
            let source = MockDataSource::success();
            let counters = Arc::clone(&source.fetch_counters);

            state.begin_loading_transition(false);
            state.mark_view_refresh_requested(view);
            assert_eq!(
                state.snapshot().view_load_state(view),
                ViewLoadState::Loading
            );
            assert_eq!(
                state.snapshot().view_load_state(match scope {
                    RefreshScope::NETWORK => AppView::Endpoints,
                    RefreshScope::CONFIG => AppView::Secrets,
                    RefreshScope::STORAGE => AppView::PersistentVolumes,
                    _ => unreachable!(),
                }),
                ViewLoadState::Idle
            );

            state
                .refresh_view_with_options(
                    &source,
                    Some("default"),
                    refresh_options(scope, false),
                    Some(view),
                )
                .await
                .unwrap_or_else(|err| panic!("{scenario}: refresh should succeed: {err}"));

            let snapshot = state.snapshot();
            assert_eq!(snapshot.view_load_state(view), ViewLoadState::Ready);
            assert_eq!(snapshot.resource_count(view), Some(0));
            assert_fetch_counts(scenario, &counters, expected);
        }
    }

    #[tokio::test]
    async fn aggregate_view_stays_loading_until_all_required_scopes_complete() {
        let mut state = GlobalState::default();
        let source = MockDataSource::success();

        state.begin_loading_transition(false);
        state.mark_view_refresh_requested(AppView::Projects);
        state
            .refresh_view_with_options(
                &source,
                Some("default"),
                refresh_options(RefreshScope::CORE_OVERVIEW, false),
                Some(AppView::Projects),
            )
            .await
            .expect("core refresh should succeed");

        assert_eq!(
            state.snapshot().view_load_state(AppView::Projects),
            ViewLoadState::Loading
        );

        state
            .refresh_view_with_options(
                &source,
                Some("default"),
                refresh_options(
                    RefreshScope::LEGACY_SECONDARY
                        .union(RefreshScope::NETWORK)
                        .union(RefreshScope::SECURITY),
                    true,
                ),
                Some(AppView::Projects),
            )
            .await
            .expect("secondary refresh should succeed");

        assert_eq!(
            state.snapshot().view_load_state(AppView::Projects),
            ViewLoadState::Ready
        );
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
    async fn refresh_core_failure_is_not_masked_by_secondary_success() {
        let mut state = GlobalState::default();
        let mut source = MockDataSource::success();
        source.nodes_err = Some("nodes down".to_string());

        let result = state
            .refresh_with_options(
                &source,
                None,
                refresh_options(RefreshScope::NODES.union(RefreshScope::CONFIG), false),
            )
            .await;

        assert!(result.is_err());
        let snapshot = state.snapshot();
        assert_eq!(snapshot.phase, DataPhase::Error);
        assert_eq!(snapshot.connection_health, ConnectionHealth::Disconnected);
        assert!(
            snapshot
                .last_error
                .as_deref()
                .unwrap_or_default()
                .contains("nodes")
        );
    }

    #[tokio::test]
    async fn refresh_metrics_cluster_info_failure_degrades_when_metrics_succeed() {
        let mut state = GlobalState::default();
        let mut source = MockDataSource::success();
        source.cluster_info_err = Some("cluster down".to_string());

        state
            .refresh_with_options(&source, None, refresh_options(RefreshScope::METRICS, false))
            .await
            .expect("metrics refresh should degrade instead of hard-failing");

        let snapshot = state.snapshot();
        assert_eq!(snapshot.phase, DataPhase::Ready);
        assert_eq!(snapshot.connection_health, ConnectionHealth::Degraded(1));
        assert!(
            snapshot
                .last_error
                .as_deref()
                .unwrap_or_default()
                .contains("cluster info")
        );
    }

    #[tokio::test]
    async fn refresh_primary_resource_failure_is_not_masked_by_secondary_success() {
        let mut state = GlobalState::default();
        let mut source = MockDataSource::success();
        source.replicasets_err = Some("replicasets down".to_string());

        let result = state
            .refresh_with_options(
                &source,
                None,
                refresh_options(RefreshScope::REPLICASETS.union(RefreshScope::CONFIG), false),
            )
            .await;

        assert!(result.is_err());
        let snapshot = state.snapshot();
        assert_eq!(snapshot.phase, DataPhase::Error);
        assert_eq!(snapshot.connection_health, ConnectionHealth::Disconnected);
        assert!(
            snapshot
                .last_error
                .as_deref()
                .unwrap_or_default()
                .contains("replicasets")
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
            vulnerability_reports: vec![],
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
            vulnerability_reports: vec![],
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
            replicasets_err: Some("replicasets down".to_string()),
            replication_controllers_err: Some("replicationcontrollers down".to_string()),
            jobs_err: Some("jobs down".to_string()),
            cronjobs_err: Some("cronjobs down".to_string()),
            namespace_list_err: Some("namespaces down".to_string()),
            resource_quotas_err: Some("resourcequotas down".to_string()),
            limit_ranges_err: Some("limitranges down".to_string()),
            pod_disruption_budgets_err: Some("pdbs down".to_string()),
            service_accounts_err: Some("serviceaccounts down".to_string()),
            roles_err: Some("roles down".to_string()),
            role_bindings_err: Some("rolebindings down".to_string()),
            cluster_roles_err: Some("clusterroles down".to_string()),
            cluster_role_bindings_err: Some("clusterrolebindings down".to_string()),
            vulnerability_reports_err: Some("vulnerabilityreports down".to_string()),
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
            vulnerability_reports: vec![],
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
        assert_eq!(snapshot.services.len(), 0);
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
            fetch::fetch_with_timeout("pods", &CORE_FETCH_SEMAPHORE, move || {
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
        assert!(fetch::is_transient_send_request_error(&err));
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

    fn make_namespace_info(name: &str, status: &str) -> NamespaceInfo {
        NamespaceInfo {
            name: name.to_string(),
            status: status.to_string(),
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
    fn apply_watch_update_normalizes_payload_order_before_version_compare() {
        let mut state = GlobalState::default();

        state.apply_watch_update(watch::WatchUpdate {
            resource: watch::WatchedResource::Pods,
            context_generation: 0,
            data: watch::WatchPayload::Pods(vec![make_pod_info("pod-b"), make_pod_info("pod-a")]),
        });

        let version_after_first = state.snapshot.snapshot_version;
        assert_eq!(state.snapshot.pods[0].name, "pod-a");
        assert_eq!(state.snapshot.pods[1].name, "pod-b");

        state.apply_watch_update(watch::WatchUpdate {
            resource: watch::WatchedResource::Pods,
            context_generation: 0,
            data: watch::WatchPayload::Pods(vec![make_pod_info("pod-a"), make_pod_info("pod-b")]),
        });

        assert_eq!(state.snapshot.snapshot_version, version_after_first);
    }

    #[test]
    fn apply_watch_update_does_not_change_view_load_state() {
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
        // Watch updates must NOT touch view load state — the refresh pipeline
        // manages it so the loading spinner renders correctly.
        assert_eq!(
            state.snapshot.view_load_states[AppView::Pods.index()],
            ViewLoadState::Idle
        );
    }

    #[test]
    fn apply_watch_update_updates_namespaces_and_picker_cache() {
        let mut state = GlobalState::default();

        let update = watch::WatchUpdate {
            resource: watch::WatchedResource::Namespaces,
            context_generation: 0,
            data: watch::WatchPayload::Namespaces(vec![
                make_namespace_info("prod", "Active"),
                make_namespace_info("default", "Active"),
                make_namespace_info("prod", "Active"),
                make_namespace_info("", "Active"),
            ]),
        };

        state.apply_watch_update(update);

        assert_eq!(state.snapshot.namespace_list.len(), 4);
        assert_eq!(
            state.namespaces,
            vec!["default".to_string(), "prod".to_string()]
        );
    }

    #[test]
    fn apply_watch_update_flux_merges_target_payload() {
        let mut state = GlobalState::default();
        Arc::make_mut(&mut state.snapshot).flux_resources = vec![FluxResourceInfo {
            name: "apps".to_string(),
            namespace: Some("flux-system".to_string()),
            kind: "Kustomization".to_string(),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            plural: "kustomizations".to_string(),
            ..FluxResourceInfo::default()
        }];
        {
            let snap = Arc::make_mut(&mut state.snapshot);
            snap.flux_counts = FluxCounts::compute(&snap.flux_resources);
        }
        let initial_version = state.snapshot.snapshot_version;

        let update = watch::WatchUpdate {
            resource: watch::WatchedResource::Flux,
            context_generation: 0,
            data: watch::WatchPayload::Flux {
                target: FluxWatchTarget {
                    group: "helm.toolkit.fluxcd.io",
                    version: "v2",
                    kind: "HelmRelease",
                    plural: "helmreleases",
                    namespaced: true,
                },
                items: vec![FluxResourceInfo {
                    name: "backend".to_string(),
                    namespace: Some("flux-system".to_string()),
                    group: "helm.toolkit.fluxcd.io".to_string(),
                    version: "v2".to_string(),
                    kind: "HelmRelease".to_string(),
                    plural: "helmreleases".to_string(),
                    ..FluxResourceInfo::default()
                }],
            },
        };
        state.apply_watch_update(update);

        assert_eq!(state.snapshot.flux_resources.len(), 2);
        assert!(
            state
                .snapshot
                .flux_resources
                .iter()
                .any(|resource| resource.name == "apps")
        );
        assert!(
            state
                .snapshot
                .flux_resources
                .iter()
                .any(|resource| resource.name == "backend")
        );
        assert_eq!(state.snapshot.flux_counts.kustomizations, 1);
        assert_eq!(state.snapshot.flux_counts.helm_releases, 1);
        assert!(state.snapshot.snapshot_version > initial_version);
    }

    fn flux_resource(
        name: &str,
        group: &str,
        version: &str,
        kind: &str,
        plural: &str,
    ) -> FluxResourceInfo {
        FluxResourceInfo {
            name: name.to_string(),
            namespace: Some("flux-system".to_string()),
            group: group.to_string(),
            version: version.to_string(),
            kind: kind.to_string(),
            plural: plural.to_string(),
            ..FluxResourceInfo::default()
        }
    }

    #[test]
    fn preserve_changed_flux_targets_keeps_refresh_rows_for_unchanged_targets() {
        let kustomize_group = "kustomize.toolkit.fluxcd.io";
        let helm_group = "helm.toolkit.fluxcd.io";
        let mut start = GlobalState::default();
        Arc::make_mut(&mut start.snapshot).flux_resources = vec![
            flux_resource(
                "apps-start",
                kustomize_group,
                "v1",
                "Kustomization",
                "kustomizations",
            ),
            flux_resource(
                "backend-start",
                helm_group,
                "v2",
                "HelmRelease",
                "helmreleases",
            ),
        ];
        let start_fingerprints = start.flux_target_fingerprints();

        let mut current = GlobalState::default();
        Arc::make_mut(&mut current.snapshot).flux_resources = vec![
            flux_resource(
                "apps-live",
                kustomize_group,
                "v1",
                "Kustomization",
                "kustomizations",
            ),
            flux_resource(
                "backend-start",
                helm_group,
                "v2",
                "HelmRelease",
                "helmreleases",
            ),
        ];

        let mut refresh_result = GlobalState::default();
        {
            let snap = Arc::make_mut(&mut refresh_result.snapshot);
            snap.flux_resources = vec![
                flux_resource(
                    "apps-stale",
                    kustomize_group,
                    "v1",
                    "Kustomization",
                    "kustomizations",
                ),
                flux_resource(
                    "backend-fetched",
                    helm_group,
                    "v2",
                    "HelmRelease",
                    "helmreleases",
                ),
            ];
            snap.flux_counts = FluxCounts::compute(&snap.flux_resources);
        }
        let initial_version = refresh_result.snapshot.snapshot_version;

        refresh_result
            .preserve_changed_flux_targets_from_snapshot(&current.snapshot(), &start_fingerprints);

        assert_eq!(refresh_result.snapshot.flux_resources.len(), 2);
        assert!(
            refresh_result
                .snapshot
                .flux_resources
                .iter()
                .any(|resource| resource.name == "apps-live")
        );
        assert!(
            refresh_result
                .snapshot
                .flux_resources
                .iter()
                .any(|resource| resource.name == "backend-fetched")
        );
        assert_eq!(refresh_result.snapshot.flux_counts.kustomizations, 1);
        assert_eq!(refresh_result.snapshot.flux_counts.helm_releases, 1);
        assert!(refresh_result.snapshot.snapshot_version > initial_version);
    }

    #[test]
    fn preserve_changed_flux_targets_keeps_watch_reconcile_progress_over_stale_refresh() {
        let kustomize_group = "kustomize.toolkit.fluxcd.io";
        let mut start = GlobalState::default();
        Arc::make_mut(&mut start.snapshot).flux_resources = vec![FluxResourceInfo {
            status: "Reconciling".to_string(),
            last_applied_revision: Some("main@sha1:old".to_string()),
            ..flux_resource(
                "apps",
                kustomize_group,
                "v1",
                "Kustomization",
                "kustomizations",
            )
        }];
        let start_fingerprints = start.flux_target_fingerprints();

        let mut current = GlobalState::default();
        Arc::make_mut(&mut current.snapshot).flux_resources = vec![FluxResourceInfo {
            status: "Ready".to_string(),
            last_applied_revision: Some("main@sha1:new".to_string()),
            ..flux_resource(
                "apps",
                kustomize_group,
                "v1",
                "Kustomization",
                "kustomizations",
            )
        }];

        let mut refresh_result = GlobalState::default();
        Arc::make_mut(&mut refresh_result.snapshot).flux_resources = vec![FluxResourceInfo {
            status: "Reconciling".to_string(),
            last_applied_revision: Some("main@sha1:old".to_string()),
            ..flux_resource(
                "apps",
                kustomize_group,
                "v1",
                "Kustomization",
                "kustomizations",
            )
        }];

        refresh_result
            .preserve_changed_flux_targets_from_snapshot(&current.snapshot(), &start_fingerprints);

        let apps = refresh_result
            .snapshot
            .flux_resources
            .iter()
            .find(|resource| resource.name == "apps")
            .expect("apps kustomization");
        assert_eq!(apps.status, "Ready");
        assert_eq!(apps.last_applied_revision.as_deref(), Some("main@sha1:new"));
    }

    #[test]
    fn flux_fingerprint_ignores_age_and_tracks_status() {
        let mut state = GlobalState::default();
        Arc::make_mut(&mut state.snapshot).flux_resources = vec![FluxResourceInfo {
            name: "apps".to_string(),
            namespace: Some("flux-system".to_string()),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            kind: "Kustomization".to_string(),
            plural: "kustomizations".to_string(),
            status: "Ready".to_string(),
            age: Some(Duration::from_secs(10)),
            ..FluxResourceInfo::default()
        }];
        let first = state.flux_fingerprint();

        Arc::make_mut(&mut state.snapshot).flux_resources[0].age = Some(Duration::from_secs(20));
        assert_eq!(state.flux_fingerprint(), first);

        Arc::make_mut(&mut state.snapshot).flux_resources[0].status = "NotReady".to_string();
        assert_ne!(state.flux_fingerprint(), first);
    }
}
