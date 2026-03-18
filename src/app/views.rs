//! NavGroup and AppView enum definitions for sidebar navigation.

use crate::icons::{group_icon, view_icon};

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

/// Top-level views displayed by KubecTUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppView {
    // Overview
    Dashboard,
    Bookmarks,
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
    const ORDER: [AppView; 48] = [
        // Overview
        AppView::Dashboard,
        AppView::Bookmarks,
        AppView::Issues,
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
            AppView::Bookmarks => "Bookmarks",
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
            AppView::Bookmarks => "view.bookmarks",
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
            AppView::Dashboard | AppView::Bookmarks | AppView::Issues | AppView::Nodes => {
                NavGroup::Overview
            }
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
    pub const fn tabs() -> &'static [AppView; 48] {
        &Self::ORDER
    }
}
