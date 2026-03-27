//! NavGroup and AppView enum definitions for sidebar navigation.

use crate::icons::{group_icon, view_icon};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

/// Sidebar navigation groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NavGroup {
    Overview,
    Workloads,
    Network,
    Config,
    Storage,
    Helm,
    FluxCD,
    AccessControl,
    CustomResources,
}

impl NavGroup {
    pub const fn persisted_key(self) -> &'static str {
        match self {
            NavGroup::Overview => "overview",
            NavGroup::Workloads => "workloads",
            NavGroup::Network => "network",
            NavGroup::Config => "config",
            NavGroup::Storage => "storage",
            NavGroup::Helm => "helm",
            NavGroup::FluxCD => "flux_cd",
            NavGroup::AccessControl => "access_control",
            NavGroup::CustomResources => "custom_resources",
        }
    }

    pub fn from_persisted_str(value: &str) -> Option<Self> {
        match value {
            "overview" | "Overview" => Some(NavGroup::Overview),
            "workloads" | "Workloads" => Some(NavGroup::Workloads),
            "network" | "Network" => Some(NavGroup::Network),
            "config" | "Config" => Some(NavGroup::Config),
            "storage" | "Storage" => Some(NavGroup::Storage),
            "helm" | "Helm" => Some(NavGroup::Helm),
            "flux_cd" | "FluxCD" => Some(NavGroup::FluxCD),
            "access_control" | "AccessControl" | "Access Control" => Some(NavGroup::AccessControl),
            "custom_resources" | "CustomResources" | "Custom Resources" => {
                Some(NavGroup::CustomResources)
            }
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            NavGroup::Overview => "Overview",
            NavGroup::Workloads => "Workloads",
            NavGroup::Network => "Network",
            NavGroup::Config => "Config",
            NavGroup::Storage => "Storage",
            NavGroup::Helm => "Helm",
            NavGroup::FluxCD => "FluxCD",
            NavGroup::AccessControl => "Access Control",
            NavGroup::CustomResources => "Custom Resources",
        }
    }

    pub fn icon(self) -> &'static str {
        group_icon(self.label()).active()
    }

    /// Returns a preformatted sidebar label including collapse state marker.
    pub fn sidebar_text(self, collapsed: bool) -> String {
        let arrow = if collapsed { "▶" } else { "▼" };
        let icon = group_icon(self.label()).active();
        format!(" {arrow} {icon}{}", self.label())
    }
}

impl Serialize for NavGroup {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.persisted_key())
    }
}

impl<'de> Deserialize<'de> for NavGroup {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        NavGroup::from_persisted_str(&value)
            .ok_or_else(|| D::Error::custom(format!("unknown nav group '{value}'")))
    }
}

/// Top-level views displayed by KubecTUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppView {
    // Overview
    Dashboard,
    Projects,
    Governance,
    Bookmarks,
    HealthReport,
    Vulnerabilities,
    Nodes,
    // Workloads
    Pods,
    Deployments,
    StatefulSets,
    DaemonSets,
    ReplicaSets,
    ReplicationControllers,
    Jobs,
    CronJobs,
    // Network
    Services,
    Endpoints,
    Ingresses,
    IngressClasses,
    GatewayClasses,
    Gateways,
    HttpRoutes,
    GrpcRoutes,
    ReferenceGrants,
    NetworkPolicies,
    PortForwarding,
    // Config
    ConfigMaps,
    Secrets,
    ResourceQuotas,
    LimitRanges,
    HPAs,
    PodDisruptionBudgets,
    PriorityClasses,
    // Storage
    PersistentVolumeClaims,
    PersistentVolumes,
    StorageClasses,
    // Standalone
    Namespaces,
    Events,
    // Helm
    HelmCharts,
    HelmReleases,
    // FluxCD
    FluxCDAlertProviders,
    FluxCDAlerts,
    FluxCDAll,
    FluxCDArtifacts,
    FluxCDHelmReleases,
    FluxCDHelmRepositories,
    FluxCDImages,
    FluxCDKustomizations,
    FluxCDReceivers,
    FluxCDSources,
    // Access Control
    ServiceAccounts,
    ClusterRoles,
    Roles,
    ClusterRoleBindings,
    RoleBindings,
    // Custom Resources
    Extensions,
    // Issue Center
    Issues,
}

impl AppView {
    pub const fn persisted_key(self) -> &'static str {
        match self {
            AppView::Dashboard => "dashboard",
            AppView::Projects => "projects",
            AppView::Governance => "governance",
            AppView::Bookmarks => "bookmarks",
            AppView::HealthReport => "health_report",
            AppView::Vulnerabilities => "vulnerabilities",
            AppView::Nodes => "nodes",
            AppView::Pods => "pods",
            AppView::Deployments => "deployments",
            AppView::StatefulSets => "stateful_sets",
            AppView::DaemonSets => "daemon_sets",
            AppView::ReplicaSets => "replica_sets",
            AppView::ReplicationControllers => "replication_controllers",
            AppView::Jobs => "jobs",
            AppView::CronJobs => "cron_jobs",
            AppView::Services => "services",
            AppView::Endpoints => "endpoints",
            AppView::Ingresses => "ingresses",
            AppView::IngressClasses => "ingress_classes",
            AppView::GatewayClasses => "gateway_classes",
            AppView::Gateways => "gateways",
            AppView::HttpRoutes => "http_routes",
            AppView::GrpcRoutes => "grpc_routes",
            AppView::ReferenceGrants => "reference_grants",
            AppView::NetworkPolicies => "network_policies",
            AppView::PortForwarding => "port_forwarding",
            AppView::ConfigMaps => "config_maps",
            AppView::Secrets => "secrets",
            AppView::ResourceQuotas => "resource_quotas",
            AppView::LimitRanges => "limit_ranges",
            AppView::HPAs => "hpas",
            AppView::PodDisruptionBudgets => "pod_disruption_budgets",
            AppView::PriorityClasses => "priority_classes",
            AppView::PersistentVolumeClaims => "persistent_volume_claims",
            AppView::PersistentVolumes => "persistent_volumes",
            AppView::StorageClasses => "storage_classes",
            AppView::Namespaces => "namespaces",
            AppView::Events => "events",
            AppView::HelmCharts => "helm_charts",
            AppView::HelmReleases => "helm_releases",
            AppView::FluxCDAlertProviders => "flux_cd_alert_providers",
            AppView::FluxCDAlerts => "flux_cd_alerts",
            AppView::FluxCDAll => "flux_cd_all",
            AppView::FluxCDArtifacts => "flux_cd_artifacts",
            AppView::FluxCDHelmReleases => "flux_cd_helm_releases",
            AppView::FluxCDHelmRepositories => "flux_cd_helm_repositories",
            AppView::FluxCDImages => "flux_cd_images",
            AppView::FluxCDKustomizations => "flux_cd_kustomizations",
            AppView::FluxCDReceivers => "flux_cd_receivers",
            AppView::FluxCDSources => "flux_cd_sources",
            AppView::ServiceAccounts => "service_accounts",
            AppView::ClusterRoles => "cluster_roles",
            AppView::Roles => "roles",
            AppView::ClusterRoleBindings => "cluster_role_bindings",
            AppView::RoleBindings => "role_bindings",
            AppView::Extensions => "extensions",
            AppView::Issues => "issues",
        }
    }

    pub fn from_persisted_str(value: &str) -> Option<Self> {
        match value {
            "dashboard" | "Dashboard" => Some(AppView::Dashboard),
            "projects" | "Projects" => Some(AppView::Projects),
            "governance" | "Governance" | "CostCenter" | "cost_center" => Some(AppView::Governance),
            "bookmarks" | "Bookmarks" => Some(AppView::Bookmarks),
            "health_report" | "HealthReport" => Some(AppView::HealthReport),
            "vulnerabilities" | "Vulnerabilities" | "SecurityCenter" => {
                Some(AppView::Vulnerabilities)
            }
            "nodes" | "Nodes" => Some(AppView::Nodes),
            "pods" | "Pods" => Some(AppView::Pods),
            "deployments" | "Deployments" => Some(AppView::Deployments),
            "stateful_sets" | "StatefulSets" => Some(AppView::StatefulSets),
            "daemon_sets" | "DaemonSets" => Some(AppView::DaemonSets),
            "replica_sets" | "ReplicaSets" => Some(AppView::ReplicaSets),
            "replication_controllers" | "ReplicationControllers" => {
                Some(AppView::ReplicationControllers)
            }
            "jobs" | "Jobs" => Some(AppView::Jobs),
            "cron_jobs" | "CronJobs" => Some(AppView::CronJobs),
            "services" | "Services" => Some(AppView::Services),
            "endpoints" | "Endpoints" => Some(AppView::Endpoints),
            "ingresses" | "Ingresses" => Some(AppView::Ingresses),
            "ingress_classes" | "IngressClasses" => Some(AppView::IngressClasses),
            "gateway_classes" | "GatewayClasses" => Some(AppView::GatewayClasses),
            "gateways" | "Gateways" => Some(AppView::Gateways),
            "http_routes" | "HttpRoutes" | "HTTPRoutes" => Some(AppView::HttpRoutes),
            "grpc_routes" | "GrpcRoutes" | "GRPCRoutes" => Some(AppView::GrpcRoutes),
            "reference_grants" | "ReferenceGrants" => Some(AppView::ReferenceGrants),
            "network_policies" | "NetworkPolicies" => Some(AppView::NetworkPolicies),
            "port_forwarding" | "PortForwarding" => Some(AppView::PortForwarding),
            "config_maps" | "ConfigMaps" => Some(AppView::ConfigMaps),
            "secrets" | "Secrets" => Some(AppView::Secrets),
            "resource_quotas" | "ResourceQuotas" => Some(AppView::ResourceQuotas),
            "limit_ranges" | "LimitRanges" => Some(AppView::LimitRanges),
            "hpas" | "HPAs" => Some(AppView::HPAs),
            "pod_disruption_budgets" | "PodDisruptionBudgets" => {
                Some(AppView::PodDisruptionBudgets)
            }
            "priority_classes" | "PriorityClasses" => Some(AppView::PriorityClasses),
            "persistent_volume_claims" | "PersistentVolumeClaims" => {
                Some(AppView::PersistentVolumeClaims)
            }
            "persistent_volumes" | "PersistentVolumes" => Some(AppView::PersistentVolumes),
            "storage_classes" | "StorageClasses" => Some(AppView::StorageClasses),
            "namespaces" | "Namespaces" => Some(AppView::Namespaces),
            "events" | "Events" => Some(AppView::Events),
            "helm_charts" | "HelmCharts" => Some(AppView::HelmCharts),
            "helm_releases" | "HelmReleases" => Some(AppView::HelmReleases),
            "flux_cd_alert_providers" | "FluxCDAlertProviders" => {
                Some(AppView::FluxCDAlertProviders)
            }
            "flux_cd_alerts" | "FluxCDAlerts" => Some(AppView::FluxCDAlerts),
            "flux_cd_all" | "FluxCDAll" => Some(AppView::FluxCDAll),
            "flux_cd_artifacts" | "FluxCDArtifacts" => Some(AppView::FluxCDArtifacts),
            "flux_cd_helm_releases" | "FluxCDHelmReleases" => Some(AppView::FluxCDHelmReleases),
            "flux_cd_helm_repositories" | "FluxCDHelmRepositories" => {
                Some(AppView::FluxCDHelmRepositories)
            }
            "flux_cd_images" | "FluxCDImages" => Some(AppView::FluxCDImages),
            "flux_cd_kustomizations" | "FluxCDKustomizations" => {
                Some(AppView::FluxCDKustomizations)
            }
            "flux_cd_receivers" | "FluxCDReceivers" => Some(AppView::FluxCDReceivers),
            "flux_cd_sources" | "FluxCDSources" => Some(AppView::FluxCDSources),
            "service_accounts" | "ServiceAccounts" => Some(AppView::ServiceAccounts),
            "cluster_roles" | "ClusterRoles" => Some(AppView::ClusterRoles),
            "roles" | "Roles" => Some(AppView::Roles),
            "cluster_role_bindings" | "ClusterRoleBindings" => Some(AppView::ClusterRoleBindings),
            "role_bindings" | "RoleBindings" => Some(AppView::RoleBindings),
            "extensions" | "Extensions" => Some(AppView::Extensions),
            "issues" | "Issues" => Some(AppView::Issues),
            _ => None,
        }
    }

    const ORDER: [AppView; 57] = [
        // Overview
        AppView::Dashboard,
        AppView::Projects,
        AppView::Governance,
        AppView::Bookmarks,
        AppView::Issues,
        AppView::HealthReport,
        AppView::Vulnerabilities,
        AppView::Nodes,
        AppView::Namespaces,
        AppView::Events,
        // Workloads
        AppView::Pods,
        AppView::Deployments,
        AppView::StatefulSets,
        AppView::DaemonSets,
        AppView::ReplicaSets,
        AppView::ReplicationControllers,
        AppView::Jobs,
        AppView::CronJobs,
        // Network
        AppView::Services,
        AppView::Endpoints,
        AppView::Ingresses,
        AppView::IngressClasses,
        AppView::GatewayClasses,
        AppView::Gateways,
        AppView::HttpRoutes,
        AppView::GrpcRoutes,
        AppView::ReferenceGrants,
        AppView::NetworkPolicies,
        AppView::PortForwarding,
        // Config
        AppView::ConfigMaps,
        AppView::Secrets,
        AppView::ResourceQuotas,
        AppView::LimitRanges,
        AppView::HPAs,
        AppView::PodDisruptionBudgets,
        AppView::PriorityClasses,
        // Storage
        AppView::PersistentVolumeClaims,
        AppView::PersistentVolumes,
        AppView::StorageClasses,
        // Helm
        AppView::HelmCharts,
        AppView::HelmReleases,
        // FluxCD
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
        // Access Control
        AppView::ServiceAccounts,
        AppView::ClusterRoles,
        AppView::Roles,
        AppView::ClusterRoleBindings,
        AppView::RoleBindings,
        // Custom Resources
        AppView::Extensions,
    ];

    pub const COUNT: usize = Self::ORDER.len();

    /// Returns a static display label for this view.
    pub const fn label(self) -> &'static str {
        match self {
            AppView::Dashboard => "Dashboard",
            AppView::Projects => "Projects",
            AppView::Governance => "Governance",
            AppView::Bookmarks => "Bookmarks",
            AppView::HealthReport => "Health Report",
            AppView::Vulnerabilities => "Vulnerabilities",
            AppView::Nodes => "Nodes",
            AppView::Pods => "Pods",
            AppView::Deployments => "Deployments",
            AppView::StatefulSets => "Stateful Sets",
            AppView::DaemonSets => "Daemon Sets",
            AppView::ReplicaSets => "Replica Sets",
            AppView::ReplicationControllers => "Replication Controllers",
            AppView::Jobs => "Jobs",
            AppView::CronJobs => "Cron Jobs",
            AppView::Services => "Services",
            AppView::Endpoints => "Endpoints",
            AppView::Ingresses => "Ingresses",
            AppView::IngressClasses => "Ingress Classes",
            AppView::GatewayClasses => "Gateway Classes",
            AppView::Gateways => "Gateways",
            AppView::HttpRoutes => "HTTP Routes",
            AppView::GrpcRoutes => "gRPC Routes",
            AppView::ReferenceGrants => "Reference Grants",
            AppView::NetworkPolicies => "Network Policies",
            AppView::PortForwarding => "Port Forwarding",
            AppView::ConfigMaps => "Config Maps",
            AppView::Secrets => "Secrets",
            AppView::ResourceQuotas => "Resource Quotas",
            AppView::LimitRanges => "Limit Ranges",
            AppView::HPAs => "Horiz. Pod Autoscalers",
            AppView::PodDisruptionBudgets => "Pod Disruption Budgets",
            AppView::PriorityClasses => "Priority Classes",
            AppView::PersistentVolumeClaims => "Persistent Vol. Claims",
            AppView::PersistentVolumes => "Persistent Volumes",
            AppView::StorageClasses => "Storage Classes",
            AppView::Namespaces => "Namespaces",
            AppView::Events => "Events",
            AppView::HelmCharts => "Repositories",
            AppView::HelmReleases => "Releases",
            AppView::FluxCDAlertProviders => "Alert Providers",
            AppView::FluxCDAlerts => "Alerts",
            AppView::FluxCDAll => "All",
            AppView::FluxCDArtifacts => "Artifacts",
            AppView::FluxCDHelmReleases => "HelmReleases",
            AppView::FluxCDHelmRepositories => "HelmRepositories",
            AppView::FluxCDImages => "Images",
            AppView::FluxCDKustomizations => "Kustomizations",
            AppView::FluxCDReceivers => "Receivers",
            AppView::FluxCDSources => "Sources",
            AppView::ServiceAccounts => "Service Accounts",
            AppView::ClusterRoles => "Cluster Roles",
            AppView::Roles => "Roles",
            AppView::ClusterRoleBindings => "Cluster Role Bindings",
            AppView::RoleBindings => "Role Bindings",
            AppView::Extensions => "Definitions",
            AppView::Issues => "Issues",
        }
    }

    /// Returns the sidebar icon for this view.
    pub fn icon(self) -> &'static str {
        view_icon(self).active()
    }

    /// Returns the preformatted sidebar row text for this view.
    pub fn sidebar_text(self) -> String {
        let icon = view_icon(self).active();
        format!("  {icon}{}", self.label())
    }

    /// Returns a stable key for render profiling spans.
    pub const fn profiling_key(self) -> &'static str {
        match self {
            AppView::Dashboard => "view.dashboard",
            AppView::Projects => "view.projects",
            AppView::Governance => "view.governance",
            AppView::Bookmarks => "view.bookmarks",
            AppView::HealthReport => "view.health_report",
            AppView::Vulnerabilities => "view.vulnerabilities",
            AppView::Nodes => "view.nodes",
            AppView::Pods => "view.pods",
            AppView::Deployments => "view.deployments",
            AppView::StatefulSets => "view.statefulsets",
            AppView::DaemonSets => "view.daemonsets",
            AppView::ReplicaSets => "view.replicasets",
            AppView::ReplicationControllers => "view.replication_controllers",
            AppView::Jobs => "view.jobs",
            AppView::CronJobs => "view.cronjobs",
            AppView::Services => "view.services",
            AppView::Endpoints => "view.endpoints",
            AppView::Ingresses => "view.ingresses",
            AppView::IngressClasses => "view.ingress_classes",
            AppView::GatewayClasses => "view.gateway_classes",
            AppView::Gateways => "view.gateways",
            AppView::HttpRoutes => "view.http_routes",
            AppView::GrpcRoutes => "view.grpc_routes",
            AppView::ReferenceGrants => "view.reference_grants",
            AppView::NetworkPolicies => "view.network_policies",
            AppView::PortForwarding => "view.port_forwarding",
            AppView::ConfigMaps => "view.config_maps",
            AppView::Secrets => "view.secrets",
            AppView::ResourceQuotas => "view.resource_quotas",
            AppView::LimitRanges => "view.limit_ranges",
            AppView::HPAs => "view.hpas",
            AppView::PodDisruptionBudgets => "view.pod_disruption_budgets",
            AppView::PriorityClasses => "view.priority_classes",
            AppView::PersistentVolumeClaims => "view.pvcs",
            AppView::PersistentVolumes => "view.pvs",
            AppView::StorageClasses => "view.storage_classes",
            AppView::Namespaces => "view.namespaces",
            AppView::Events => "view.events",
            AppView::HelmCharts => "view.helm_charts",
            AppView::HelmReleases => "view.helm_releases",
            AppView::FluxCDAlertProviders => "view.fluxcd.alert_providers",
            AppView::FluxCDAlerts => "view.fluxcd.alerts",
            AppView::FluxCDAll => "view.fluxcd.all",
            AppView::FluxCDArtifacts => "view.fluxcd.artifacts",
            AppView::FluxCDHelmReleases => "view.fluxcd.helm_releases",
            AppView::FluxCDHelmRepositories => "view.fluxcd.helm_repositories",
            AppView::FluxCDImages => "view.fluxcd.images",
            AppView::FluxCDKustomizations => "view.fluxcd.kustomizations",
            AppView::FluxCDReceivers => "view.fluxcd.receivers",
            AppView::FluxCDSources => "view.fluxcd.sources",
            AppView::ServiceAccounts => "view.service_accounts",
            AppView::ClusterRoles => "view.cluster_roles",
            AppView::Roles => "view.roles",
            AppView::ClusterRoleBindings => "view.cluster_role_bindings",
            AppView::RoleBindings => "view.role_bindings",
            AppView::Extensions => "view.extensions",
            AppView::Issues => "view.issues",
        }
    }

    /// Returns the NavGroup this view belongs to.
    pub const fn group(self) -> NavGroup {
        match self {
            AppView::Dashboard
            | AppView::Projects
            | AppView::Governance
            | AppView::Bookmarks
            | AppView::Issues
            | AppView::HealthReport
            | AppView::Vulnerabilities
            | AppView::Nodes => NavGroup::Overview,
            AppView::Pods
            | AppView::Deployments
            | AppView::StatefulSets
            | AppView::DaemonSets
            | AppView::ReplicaSets
            | AppView::ReplicationControllers
            | AppView::Jobs
            | AppView::CronJobs => NavGroup::Workloads,
            AppView::Services
            | AppView::Endpoints
            | AppView::Ingresses
            | AppView::IngressClasses
            | AppView::GatewayClasses
            | AppView::Gateways
            | AppView::HttpRoutes
            | AppView::GrpcRoutes
            | AppView::ReferenceGrants
            | AppView::NetworkPolicies
            | AppView::PortForwarding => NavGroup::Network,
            AppView::ConfigMaps
            | AppView::Secrets
            | AppView::ResourceQuotas
            | AppView::LimitRanges
            | AppView::HPAs
            | AppView::PodDisruptionBudgets
            | AppView::PriorityClasses => NavGroup::Config,
            AppView::PersistentVolumeClaims
            | AppView::PersistentVolumes
            | AppView::StorageClasses => NavGroup::Storage,
            AppView::Namespaces | AppView::Events => NavGroup::Overview,
            AppView::HelmCharts | AppView::HelmReleases => NavGroup::Helm,
            AppView::FluxCDAlertProviders
            | AppView::FluxCDAlerts
            | AppView::FluxCDAll
            | AppView::FluxCDArtifacts
            | AppView::FluxCDHelmReleases
            | AppView::FluxCDHelmRepositories
            | AppView::FluxCDImages
            | AppView::FluxCDKustomizations
            | AppView::FluxCDReceivers
            | AppView::FluxCDSources => NavGroup::FluxCD,
            AppView::ServiceAccounts
            | AppView::ClusterRoles
            | AppView::Roles
            | AppView::ClusterRoleBindings
            | AppView::RoleBindings => NavGroup::AccessControl,
            AppView::Extensions => NavGroup::CustomResources,
        }
    }

    pub const fn is_fluxcd(self) -> bool {
        matches!(
            self,
            AppView::FluxCDAlertProviders
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

    pub(crate) fn index(self) -> usize {
        Self::ORDER
            .iter()
            .position(|view| *view == self)
            .expect("AppView::ORDER must contain all enum variants")
    }

    fn from_index(index: usize) -> Self {
        Self::ORDER[index % Self::ORDER.len()]
    }

    pub(crate) fn next(self) -> Self {
        Self::from_index(self.index() + 1)
    }

    pub(crate) fn previous(self) -> Self {
        let current = self.index();
        let next_idx = if current == 0 {
            Self::ORDER.len() - 1
        } else {
            current - 1
        };
        Self::from_index(next_idx)
    }

    /// Enumerates all available top-level tabs in stable order.
    pub const fn tabs() -> &'static [AppView; Self::COUNT] {
        &Self::ORDER
    }
}

impl Serialize for AppView {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.persisted_key())
    }
}

impl<'de> Deserialize<'de> for AppView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        AppView::from_persisted_str(&value)
            .ok_or_else(|| D::Error::custom(format!("unknown app view '{value}'")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_view_serializes_to_stable_key() {
        let encoded = serde_json::to_string(&AppView::FluxCDHelmRepositories).expect("serialize");
        assert_eq!(encoded, "\"flux_cd_helm_repositories\"");
    }

    #[test]
    fn app_view_deserializes_legacy_variant_name() {
        let decoded: AppView = serde_json::from_str("\"FluxCDHelmRepositories\"")
            .expect("deserialize legacy app view");
        assert_eq!(decoded, AppView::FluxCDHelmRepositories);
    }

    #[test]
    fn nav_group_serializes_to_stable_key() {
        let encoded = serde_json::to_string(&NavGroup::AccessControl).expect("serialize");
        assert_eq!(encoded, "\"access_control\"");
    }

    #[test]
    fn nav_group_deserializes_legacy_variant_name() {
        let decoded: NavGroup =
            serde_json::from_str("\"CustomResources\"").expect("deserialize legacy nav group");
        assert_eq!(decoded, NavGroup::CustomResources);
    }
}
