//! Application state machine and keyboard input handling.

use std::{collections::HashSet, fs, path::Path, sync::LazyLock};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

use crate::{
    action_history::{ActionHistoryState, ActionHistoryTarget, ActionKind, ActionStatus},
    k8s::{
        client::EventInfo,
        dtos::{CustomResourceInfo, NodeMetricsInfo, PodInfo, PodMetricsInfo},
    },
    policy::{DetailAction, ViewAction},
    ui::components::{
        CommandPalette, CommandPaletteAction, ContextPicker, ContextPickerAction, NamespacePicker,
        NamespacePickerAction,
        port_forward_dialog::PortForwardDialog,
        probe_panel::ProbePanelState as ProbePanelComponentState,
        scale_dialog::{ScaleDialogState, ScaleTargetKind},
    },
    workbench::{
        ActionHistoryTabState, DEFAULT_WORKBENCH_HEIGHT, ExecTabState, PodLogsTabState,
        PortForwardTabState, ResourceEventsTabState, ResourceYamlTabState, WorkbenchState,
        WorkbenchTabState, WorkloadLogsTabState,
    },
};

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

    pub const fn icon(self) -> &'static str {
        match self {
            NavGroup::Overview => "󰋗",
            NavGroup::Workloads => "󰆧",
            NavGroup::Network => "󰛳",
            NavGroup::Config => "�",
            NavGroup::Storage => "󰋊",
            NavGroup::Helm => "󰱥",
            NavGroup::FluxCD => "󰠳",
            NavGroup::AccessControl => "󰒃",
            NavGroup::CustomResources => "󰏗",
        }
    }

    /// Returns a preformatted sidebar label including collapse state marker.
    pub const fn sidebar_text(self, collapsed: bool) -> &'static str {
        match (self, collapsed) {
            (NavGroup::Overview, false) => " ▼ 󰋗 Overview",
            (NavGroup::Overview, true) => " ▶ 󰋗 Overview",
            (NavGroup::Workloads, false) => " ▼ 󰆧 Workloads",
            (NavGroup::Workloads, true) => " ▶ 󰆧 Workloads",
            (NavGroup::Network, false) => " ▼ 󰛳 Network",
            (NavGroup::Network, true) => " ▶ 󰛳 Network",
            (NavGroup::Config, false) => " ▼ � Config",
            (NavGroup::Config, true) => " ▶ � Config",
            (NavGroup::Storage, false) => " ▼ 󰋊 Storage",
            (NavGroup::Storage, true) => " ▶ 󰋊 Storage",
            (NavGroup::Helm, false) => " ▼ 󰱥 Helm",
            (NavGroup::Helm, true) => " ▶ 󰱥 Helm",
            (NavGroup::FluxCD, false) => " ▼ 󰠳 FluxCD",
            (NavGroup::FluxCD, true) => " ▶ 󰠳 FluxCD",
            (NavGroup::AccessControl, false) => " ▼ 󰒃 Access Control",
            (NavGroup::AccessControl, true) => " ▶ 󰒃 Access Control",
            (NavGroup::CustomResources, false) => " ▼ 󰏗 Custom Resources",
            (NavGroup::CustomResources, true) => " ▶ 󰏗 Custom Resources",
        }
    }
}

/// Top-level views displayed by KubecTUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppView {
    // Overview
    Dashboard,
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
}

impl AppView {
    const ORDER: [AppView; 46] = [
        // Overview
        AppView::Dashboard,
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
        }
    }

    /// Returns the sidebar icon for this view.
    pub const fn icon(self) -> &'static str {
        match self {
            AppView::Dashboard => "󰋗",
            AppView::Nodes => "󰒋",
            AppView::Pods => "󰠳",
            AppView::Deployments => "󰆧",
            AppView::StatefulSets => "󰆼",
            AppView::DaemonSets => "󰒓",
            AppView::ReplicaSets => "󰆧",
            AppView::ReplicationControllers => "󰆧",
            AppView::Jobs => "󰃰",
            AppView::CronJobs => "󰔠",
            AppView::Services => "󰛳",
            AppView::Endpoints => "�",
            AppView::Ingresses => "󰱓",
            AppView::IngressClasses => "󰱓",
            AppView::NetworkPolicies => "󰒃",
            AppView::PortForwarding => "󰛳",
            AppView::ConfigMaps => "󰒓",
            AppView::Secrets => "󰌋",
            AppView::ResourceQuotas => "󰏗",
            AppView::LimitRanges => "󰳗",
            AppView::HPAs => "󰦕",
            AppView::PodDisruptionBudgets => "󰦕",
            AppView::PriorityClasses => "󰔠",
            AppView::PersistentVolumeClaims => "󰋊",
            AppView::PersistentVolumes => "󰋊",
            AppView::StorageClasses => "󰋊",
            AppView::Namespaces => "󰏗",
            AppView::Events => "󰃰",
            AppView::HelmCharts => "󰱥",
            AppView::HelmReleases => "󰱥",
            AppView::FluxCDAlertProviders => "󰖂",
            AppView::FluxCDAlerts => "󰀬",
            AppView::FluxCDAll => "󰠳",
            AppView::FluxCDArtifacts => "󰏗",
            AppView::FluxCDHelmReleases => "󰱥",
            AppView::FluxCDHelmRepositories => "󰱥",
            AppView::FluxCDImages => "󰄾",
            AppView::FluxCDKustomizations => "󰆧",
            AppView::FluxCDReceivers => "󰜗",
            AppView::FluxCDSources => "󰑐",
            AppView::ServiceAccounts => "󰀄",
            AppView::ClusterRoles => "󰒃",
            AppView::Roles => "󰒃",
            AppView::ClusterRoleBindings => "󰌋",
            AppView::RoleBindings => "󰌋",
            AppView::Extensions => "󰏗",
        }
    }

    /// Returns the preformatted sidebar row text for this view.
    pub const fn sidebar_text(self) -> &'static str {
        match self {
            AppView::Dashboard => "  󰋗 Dashboard",
            AppView::Nodes => "  󰒋 Nodes",
            AppView::Pods => "  󰠳 Pods",
            AppView::Deployments => "  󰆧 Deployments",
            AppView::StatefulSets => "  󰆼 Stateful Sets",
            AppView::DaemonSets => "  󰒓 Daemon Sets",
            AppView::ReplicaSets => "  󰆧 Replica Sets",
            AppView::ReplicationControllers => "  󰆧 Replication Controllers",
            AppView::Jobs => "  󰃰 Jobs",
            AppView::CronJobs => "  󰔠 Cron Jobs",
            AppView::Services => "  󰛳 Services",
            AppView::Endpoints => "  � Endpoints",
            AppView::Ingresses => "  󰱓 Ingresses",
            AppView::IngressClasses => "  󰱓 Ingress Classes",
            AppView::NetworkPolicies => "  󰒃 Network Policies",
            AppView::PortForwarding => "  󰛳 Port Forwarding",
            AppView::ConfigMaps => "  󰒓 Config Maps",
            AppView::Secrets => "  󰌋 Secrets",
            AppView::ResourceQuotas => "  󰏗 Resource Quotas",
            AppView::LimitRanges => "  󰳗 Limit Ranges",
            AppView::HPAs => "  󰦕 Horiz. Pod Autoscalers",
            AppView::PodDisruptionBudgets => "  󰦕 Pod Disruption Budgets",
            AppView::PriorityClasses => "  󰔠 Priority Classes",
            AppView::PersistentVolumeClaims => "  󰋊 Persistent Vol. Claims",
            AppView::PersistentVolumes => "  󰋊 Persistent Volumes",
            AppView::StorageClasses => "  󰋊 Storage Classes",
            AppView::Namespaces => "  󰏗 Namespaces",
            AppView::Events => "  󰃰 Events",
            AppView::HelmCharts => "  󰱥 Repositories",
            AppView::HelmReleases => "  󰱥 Releases",
            AppView::FluxCDAlertProviders => "  󰖂 Alert Providers",
            AppView::FluxCDAlerts => "  󰀬 Alerts",
            AppView::FluxCDAll => "  󰠳 All",
            AppView::FluxCDArtifacts => "  󰏗 Artifacts",
            AppView::FluxCDHelmReleases => "  󰱥 HelmReleases",
            AppView::FluxCDHelmRepositories => "  󰱥 HelmRepositories",
            AppView::FluxCDImages => "  󰄾 Images",
            AppView::FluxCDKustomizations => "  󰆧 Kustomizations",
            AppView::FluxCDReceivers => "  󰜗 Receivers",
            AppView::FluxCDSources => "  󰑐 Sources",
            AppView::ServiceAccounts => "  󰀄 Service Accounts",
            AppView::ClusterRoles => "  󰒃 Cluster Roles",
            AppView::Roles => "  󰒃 Roles",
            AppView::ClusterRoleBindings => "  󰌋 Cluster Role Bindings",
            AppView::RoleBindings => "  󰌋 Role Bindings",
            AppView::Extensions => "  󰏗 Definitions",
        }
    }

    /// Returns a stable key for render profiling spans.
    pub const fn profiling_key(self) -> &'static str {
        match self {
            AppView::Dashboard => "view.dashboard",
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
        }
    }

    /// Returns the NavGroup this view belongs to.
    pub const fn group(self) -> NavGroup {
        match self {
            AppView::Dashboard | AppView::Nodes => NavGroup::Overview,
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

    fn next(self) -> Self {
        Self::from_index(self.index() + 1)
    }

    fn previous(self) -> Self {
        let current = self.index();
        let next_idx = if current == 0 {
            Self::ORDER.len() - 1
        } else {
            current - 1
        };
        Self::from_index(next_idx)
    }

    /// Enumerates all available top-level tabs in stable order.
    pub const fn tabs() -> &'static [AppView; 46] {
        &Self::ORDER
    }
}

/// Shared sortable columns for cross-view list sorting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkloadSortColumn {
    Name,
    Age,
}

impl WorkloadSortColumn {
    const fn default_descending(self) -> bool {
        match self {
            WorkloadSortColumn::Name => false,
            WorkloadSortColumn::Age => true,
        }
    }
}

/// Active shared sort configuration for cross-view list sorting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkloadSortState {
    pub column: WorkloadSortColumn,
    pub descending: bool,
}

impl WorkloadSortState {
    pub const fn new(column: WorkloadSortColumn, descending: bool) -> Self {
        Self { column, descending }
    }

    pub const fn cache_variant(self) -> u64 {
        let column = match self.column {
            WorkloadSortColumn::Name => 1_u64,
            WorkloadSortColumn::Age => 2_u64,
        };
        let direction = if self.descending { 1_u64 } else { 0_u64 };
        (column << 1) | direction
    }

    pub const fn short_label(self) -> &'static str {
        match (self.column, self.descending) {
            (WorkloadSortColumn::Name, true) => "name desc",
            (WorkloadSortColumn::Name, false) => "name asc",
            (WorkloadSortColumn::Age, true) => "age desc",
            (WorkloadSortColumn::Age, false) => "age asc",
        }
    }
}

/// Sortable columns for Pods view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PodSortColumn {
    Name,
    Age,
    Status,
    Restarts,
}

impl PodSortColumn {
    const fn default_descending(self) -> bool {
        match self {
            PodSortColumn::Name | PodSortColumn::Status => false,
            PodSortColumn::Age | PodSortColumn::Restarts => true,
        }
    }
}

/// Active Pods sort configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PodSortState {
    pub column: PodSortColumn,
    pub descending: bool,
}

impl PodSortState {
    pub const fn new(column: PodSortColumn, descending: bool) -> Self {
        Self { column, descending }
    }

    pub const fn cache_variant(self) -> u64 {
        let column = match self.column {
            PodSortColumn::Name => 1_u64,
            PodSortColumn::Age => 2_u64,
            PodSortColumn::Status => 3_u64,
            PodSortColumn::Restarts => 4_u64,
        };
        let direction = if self.descending { 1_u64 } else { 0_u64 };
        (column << 1) | direction
    }

    pub const fn short_label(self) -> &'static str {
        match (self.column, self.descending) {
            (PodSortColumn::Name, true) => "name desc",
            (PodSortColumn::Name, false) => "name asc",
            (PodSortColumn::Age, true) => "age desc",
            (PodSortColumn::Age, false) => "age asc",
            (PodSortColumn::Status, true) => "status desc",
            (PodSortColumn::Status, false) => "status asc",
            (PodSortColumn::Restarts, true) => "restarts desc",
            (PodSortColumn::Restarts, false) => "restarts asc",
        }
    }
}

#[inline]
fn contains_ci_ascii(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

#[inline]
fn cmp_ci_ascii(left: &str, right: &str) -> std::cmp::Ordering {
    let mut l = left.bytes();
    let mut r = right.bytes();
    loop {
        match (l.next(), r.next()) {
            (Some(lb), Some(rb)) => {
                let lc = lb.to_ascii_lowercase();
                let rc = rb.to_ascii_lowercase();
                if lc != rc {
                    return lc.cmp(&rc);
                }
            }
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (None, None) => return std::cmp::Ordering::Equal,
        }
    }
}

/// Builds filtered pod indices and applies optional sort.
///
/// This function is the canonical pods list ordering path used by both rendering
/// and selected-row resource resolution, so table selection and Enter-open stay aligned.
pub fn filtered_pod_indices(
    pods: &[PodInfo],
    query: &str,
    sort: Option<PodSortState>,
) -> Vec<usize> {
    let query = query.trim();
    let mut out: Vec<usize> = if query.is_empty() {
        (0..pods.len()).collect()
    } else {
        pods.iter()
            .enumerate()
            .filter_map(|(idx, pod)| {
                if contains_ci_ascii(&pod.name, query)
                    || contains_ci_ascii(&pod.namespace, query)
                    || contains_ci_ascii(&pod.status, query)
                {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    };

    if let Some(sort) = sort {
        out.sort_by(|left_idx, right_idx| {
            let left = &pods[*left_idx];
            let right = &pods[*right_idx];
            let ordered = match sort.column {
                PodSortColumn::Name => {
                    let base = cmp_ci_ascii(&left.name, &right.name);
                    if sort.descending {
                        base.reverse()
                    } else {
                        base
                    }
                }
                PodSortColumn::Age => {
                    // Sort None (unknown created_at) to the end regardless of direction.
                    match (left.created_at, right.created_at) {
                        (None, None) => std::cmp::Ordering::Equal,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (Some(l), Some(r)) => {
                            let base = l.cmp(&r);
                            if sort.descending {
                                base.reverse()
                            } else {
                                base
                            }
                        }
                    }
                }
                PodSortColumn::Status => {
                    let base = cmp_ci_ascii(&left.status, &right.status);
                    if sort.descending {
                        base.reverse()
                    } else {
                        base
                    }
                }
                PodSortColumn::Restarts => {
                    let base = left.restarts.cmp(&right.restarts);
                    if sort.descending {
                        base.reverse()
                    } else {
                        base
                    }
                }
            };
            if ordered != std::cmp::Ordering::Equal {
                return ordered;
            }
            let ns = cmp_ci_ascii(&left.namespace, &right.namespace);
            if ns != std::cmp::Ordering::Equal {
                return ns;
            }
            let name = cmp_ci_ascii(&left.name, &right.name);
            if name != std::cmp::Ordering::Equal {
                return name;
            }
            left_idx.cmp(right_idx)
        });
    }

    out
}

/// Builds filtered workload indices and applies shared name/age sorting.
pub fn filtered_workload_indices<T, Match, Name, Namespace, Age>(
    items: &[T],
    query: &str,
    sort: Option<WorkloadSortState>,
    matches_query: Match,
    name: Name,
    namespace: Namespace,
    age: Age,
) -> Vec<usize>
where
    Match: Fn(&T, &str) -> bool,
    Name: Fn(&T) -> &str,
    Namespace: Fn(&T) -> &str,
    Age: Fn(&T) -> Option<std::time::Duration>,
{
    let query = query.trim();
    let mut out: Vec<usize> = items
        .iter()
        .enumerate()
        .filter_map(|(idx, item)| matches_query(item, query).then_some(idx))
        .collect();

    if let Some(sort) = sort {
        out.sort_by(|left_idx, right_idx| {
            let left = &items[*left_idx];
            let right = &items[*right_idx];
            let ordered = match sort.column {
                WorkloadSortColumn::Name => {
                    let base = cmp_ci_ascii(name(left), name(right));
                    if sort.descending {
                        base.reverse()
                    } else {
                        base
                    }
                }
                WorkloadSortColumn::Age => {
                    // Sort None (unknown age) to the end regardless of direction.
                    match (age(left), age(right)) {
                        (None, None) => std::cmp::Ordering::Equal,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (Some(l), Some(r)) => {
                            let base = l.cmp(&r);
                            if sort.descending {
                                base.reverse()
                            } else {
                                base
                            }
                        }
                    }
                }
            };
            if ordered != std::cmp::Ordering::Equal {
                return ordered;
            }
            let ns = cmp_ci_ascii(namespace(left), namespace(right));
            if ns != std::cmp::Ordering::Equal {
                return ns;
            }
            let item_name = cmp_ci_ascii(name(left), name(right));
            if item_name != std::cmp::Ordering::Equal {
                return item_name;
            }
            left_idx.cmp(right_idx)
        });
    }

    out
}

/// Logical pointer to a resource selected in the current view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceRef {
    Node(String),
    Pod(String, String),
    Service(String, String),
    Deployment(String, String),
    StatefulSet(String, String),
    DaemonSet(String, String),
    ReplicaSet(String, String),
    ReplicationController(String, String),
    Job(String, String),
    CronJob(String, String),
    ResourceQuota(String, String),
    LimitRange(String, String),
    PodDisruptionBudget(String, String),
    Endpoint(String, String),
    Ingress(String, String),
    IngressClass(String),
    NetworkPolicy(String, String),
    ConfigMap(String, String),
    Secret(String, String),
    Hpa(String, String),
    PriorityClass(String),
    Pvc(String, String),
    Pv(String),
    StorageClass(String),
    Namespace(String),
    Event(String, String),
    ServiceAccount(String, String),
    Role(String, String),
    RoleBinding(String, String),
    ClusterRole(String),
    ClusterRoleBinding(String),
    HelmRelease(String, String),
    /// A custom resource instance identified by its CRD coordinates.
    /// Fields: (name, namespace_opt, group, version, kind, plural)
    CustomResource {
        name: String,
        namespace: Option<String>,
        group: String,
        version: String,
        kind: String,
        plural: String,
    },
}

impl ResourceRef {
    /// Returns resource kind label used by UI and fetch routing.
    pub fn kind(&self) -> &str {
        match self {
            ResourceRef::Node(_) => "Node",
            ResourceRef::Pod(_, _) => "Pod",
            ResourceRef::Service(_, _) => "Service",
            ResourceRef::Deployment(_, _) => "Deployment",
            ResourceRef::StatefulSet(_, _) => "StatefulSet",
            ResourceRef::DaemonSet(_, _) => "DaemonSet",
            ResourceRef::ReplicaSet(_, _) => "ReplicaSet",
            ResourceRef::ReplicationController(_, _) => "ReplicationController",
            ResourceRef::Job(_, _) => "Job",
            ResourceRef::CronJob(_, _) => "CronJob",
            ResourceRef::ResourceQuota(_, _) => "ResourceQuota",
            ResourceRef::LimitRange(_, _) => "LimitRange",
            ResourceRef::PodDisruptionBudget(_, _) => "PodDisruptionBudget",
            ResourceRef::Endpoint(_, _) => "Endpoints",
            ResourceRef::Ingress(_, _) => "Ingress",
            ResourceRef::IngressClass(_) => "IngressClass",
            ResourceRef::NetworkPolicy(_, _) => "NetworkPolicy",
            ResourceRef::ConfigMap(_, _) => "ConfigMap",
            ResourceRef::Secret(_, _) => "Secret",
            ResourceRef::Hpa(_, _) => "HorizontalPodAutoscaler",
            ResourceRef::PriorityClass(_) => "PriorityClass",
            ResourceRef::Pvc(_, _) => "PersistentVolumeClaim",
            ResourceRef::Pv(_) => "PersistentVolume",
            ResourceRef::StorageClass(_) => "StorageClass",
            ResourceRef::Namespace(_) => "Namespace",
            ResourceRef::Event(_, _) => "Event",
            ResourceRef::ServiceAccount(_, _) => "ServiceAccount",
            ResourceRef::Role(_, _) => "Role",
            ResourceRef::RoleBinding(_, _) => "RoleBinding",
            ResourceRef::ClusterRole(_) => "ClusterRole",
            ResourceRef::ClusterRoleBinding(_) => "ClusterRoleBinding",
            ResourceRef::HelmRelease(_, _) => "HelmRelease",
            ResourceRef::CustomResource { kind, .. } => kind.as_str(),
        }
    }

    /// Returns resource name.
    pub fn name(&self) -> &str {
        match self {
            ResourceRef::Node(name)
            | ResourceRef::Pod(name, _)
            | ResourceRef::Service(name, _)
            | ResourceRef::Deployment(name, _)
            | ResourceRef::StatefulSet(name, _)
            | ResourceRef::DaemonSet(name, _)
            | ResourceRef::ReplicaSet(name, _)
            | ResourceRef::ReplicationController(name, _)
            | ResourceRef::Job(name, _)
            | ResourceRef::CronJob(name, _)
            | ResourceRef::ResourceQuota(name, _)
            | ResourceRef::LimitRange(name, _)
            | ResourceRef::PodDisruptionBudget(name, _)
            | ResourceRef::Endpoint(name, _)
            | ResourceRef::Ingress(name, _)
            | ResourceRef::IngressClass(name)
            | ResourceRef::NetworkPolicy(name, _)
            | ResourceRef::ConfigMap(name, _)
            | ResourceRef::Secret(name, _)
            | ResourceRef::Hpa(name, _)
            | ResourceRef::PriorityClass(name)
            | ResourceRef::Pvc(name, _)
            | ResourceRef::Pv(name)
            | ResourceRef::StorageClass(name)
            | ResourceRef::Namespace(name)
            | ResourceRef::Event(name, _)
            | ResourceRef::ServiceAccount(name, _)
            | ResourceRef::Role(name, _)
            | ResourceRef::RoleBinding(name, _)
            | ResourceRef::ClusterRole(name)
            | ResourceRef::ClusterRoleBinding(name) => name,
            ResourceRef::HelmRelease(name, _) => name,
            ResourceRef::CustomResource { name, .. } => name,
        }
    }

    /// Returns namespace when this is a namespaced resource.
    pub fn namespace(&self) -> Option<&str> {
        match self {
            ResourceRef::Node(_)
            | ResourceRef::IngressClass(_)
            | ResourceRef::PriorityClass(_)
            | ResourceRef::Pv(_)
            | ResourceRef::StorageClass(_)
            | ResourceRef::Namespace(_)
            | ResourceRef::ClusterRole(_)
            | ResourceRef::ClusterRoleBinding(_) => None,
            ResourceRef::Pod(_, ns)
            | ResourceRef::Service(_, ns)
            | ResourceRef::Deployment(_, ns)
            | ResourceRef::StatefulSet(_, ns)
            | ResourceRef::DaemonSet(_, ns)
            | ResourceRef::ReplicaSet(_, ns)
            | ResourceRef::ReplicationController(_, ns)
            | ResourceRef::Job(_, ns)
            | ResourceRef::CronJob(_, ns)
            | ResourceRef::ResourceQuota(_, ns)
            | ResourceRef::LimitRange(_, ns)
            | ResourceRef::PodDisruptionBudget(_, ns)
            | ResourceRef::Endpoint(_, ns)
            | ResourceRef::Ingress(_, ns)
            | ResourceRef::NetworkPolicy(_, ns)
            | ResourceRef::ConfigMap(_, ns)
            | ResourceRef::Secret(_, ns)
            | ResourceRef::Hpa(_, ns)
            | ResourceRef::Pvc(_, ns)
            | ResourceRef::Event(_, ns)
            | ResourceRef::ServiceAccount(_, ns)
            | ResourceRef::Role(_, ns)
            | ResourceRef::RoleBinding(_, ns) => Some(ns),
            ResourceRef::HelmRelease(_, ns) => Some(ns),
            ResourceRef::CustomResource { namespace, .. } => namespace.as_deref(),
        }
    }
}

/// Human-readable metadata displayed in the detail modal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetailMetadata {
    pub name: String,
    pub namespace: Option<String>,
    pub status: Option<String>,
    pub node: Option<String>,
    pub ip: Option<String>,
    pub created: Option<String>,
    pub labels: Vec<(String, String)>,
    pub flux_reconcile_enabled: bool,
}

/// Top-level active component when detail modal is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveComponent {
    None,
    LogsViewer,
    PortForward,
    Scale,
    ProbePanel,
}

/// Maximum number of log lines retained in the viewer buffer.
/// Older lines are dropped when this limit is exceeded.
pub const MAX_LOG_LINES: usize = 50_000;

/// In-detail logs viewer state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogsViewerState {
    pub scroll_offset: usize,
    pub follow_mode: bool,
    pub lines: Vec<String>,
    pub pod_name: String,
    pub pod_namespace: String,
    pub container_name: String,
    /// All containers in this pod — populated before logs are fetched.
    pub containers: Vec<String>,
    /// When true, show the container picker instead of logs.
    pub picking_container: bool,
    /// Cursor index in the container picker list.
    pub container_cursor: usize,
    /// Monotonic request id for in-flight container list fetch.
    pub pending_container_request_id: Option<u64>,
    /// Monotonic request id for in-flight tail logs fetch.
    pub pending_logs_request_id: Option<u64>,
    pub loading: bool,
    pub error: Option<String>,
    /// When true, fetch logs from the previous (crashed/restarted) container.
    pub previous_logs: bool,
    /// When true, request timestamps from the Kubernetes API.
    pub show_timestamps: bool,
    pub search_query: String,
    pub search_input: String,
    pub searching: bool,
}

impl LogsViewerState {
    /// Appends a log line, evicting the oldest lines if the buffer exceeds [`MAX_LOG_LINES`].
    pub fn push_line(&mut self, line: String) {
        const MAX_LINE_BYTES: usize = 10_000;
        let line = if line.len() > MAX_LINE_BYTES {
            // Find the nearest char boundary at or before the limit to avoid
            // panicking on multi-byte UTF-8 sequences.
            let end = line.floor_char_boundary(MAX_LINE_BYTES);
            let mut truncated = line;
            truncated.truncate(end);
            truncated.push_str("…[truncated]");
            truncated
        } else {
            line
        };
        self.lines.push(line);
        if self.lines.len() > MAX_LOG_LINES {
            let excess = self.lines.len() - MAX_LOG_LINES;
            self.lines.drain(..excess);
            self.scroll_offset = self.scroll_offset.saturating_sub(excess);
        }
    }
}

/// Active form field in the lightweight port-forward dialog state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortForwardField {
    LocalPort,
    RemotePort,
    TunnelList,
}

/// In-detail port-forward dialog state used by keyboard routing tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortForwardDialogState {
    pub active_field: PortForwardField,
    pub local_port: String,
    pub remote_port: String,
}

impl Default for PortForwardDialogState {
    fn default() -> Self {
        Self {
            active_field: PortForwardField::LocalPort,
            local_port: String::new(),
            remote_port: String::new(),
        }
    }
}

/// In-detail scale dialog state used by keyboard routing tests.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScaleDialogInputState {
    pub replica_input: String,
    pub target_replicas: i32,
}

/// In-detail probe panel state used by keyboard routing tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProbePanelState {
    pub probes: Vec<String>,
    pub expanded: Vec<bool>,
    pub selected_idx: usize,
}

/// Detail modal state for the currently focused resource.
#[derive(Debug, Clone, Default)]
pub struct DetailViewState {
    pub resource: Option<ResourceRef>,
    pub metadata: DetailMetadata,
    pub yaml: Option<String>,
    pub events: Vec<EventInfo>,
    pub sections: Vec<String>,
    pub pod_metrics: Option<PodMetricsInfo>,
    pub node_metrics: Option<NodeMetricsInfo>,
    pub metrics_unavailable_message: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub scale_dialog: Option<ScaleDialogState>,
    pub probe_panel: Option<ProbePanelComponentState>,
    /// When true, a delete confirmation prompt is shown in the detail view.
    pub confirm_delete: bool,
}

/// A row in the sidebar — either a group header or a leaf view item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarItem {
    Group(NavGroup),
    View(AppView),
}

const SIDEBAR_GROUPS: &[(NavGroup, &[AppView])] = &[
    (
        NavGroup::Overview,
        &[
            AppView::Dashboard,
            AppView::Nodes,
            AppView::Namespaces,
            AppView::Events,
        ],
    ),
    (
        NavGroup::Workloads,
        &[
            AppView::Pods,
            AppView::Deployments,
            AppView::StatefulSets,
            AppView::DaemonSets,
            AppView::ReplicaSets,
            AppView::ReplicationControllers,
            AppView::Jobs,
            AppView::CronJobs,
        ],
    ),
    (
        NavGroup::Network,
        &[
            AppView::Services,
            AppView::Endpoints,
            AppView::Ingresses,
            AppView::IngressClasses,
            AppView::NetworkPolicies,
            AppView::PortForwarding,
        ],
    ),
    (
        NavGroup::Config,
        &[
            AppView::ConfigMaps,
            AppView::Secrets,
            AppView::ResourceQuotas,
            AppView::LimitRanges,
            AppView::HPAs,
            AppView::PodDisruptionBudgets,
            AppView::PriorityClasses,
        ],
    ),
    (
        NavGroup::Storage,
        &[
            AppView::PersistentVolumeClaims,
            AppView::PersistentVolumes,
            AppView::StorageClasses,
        ],
    ),
    (
        NavGroup::Helm,
        &[AppView::HelmCharts, AppView::HelmReleases],
    ),
    (
        NavGroup::FluxCD,
        &[
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
        ],
    ),
    (
        NavGroup::AccessControl,
        &[
            AppView::ServiceAccounts,
            AppView::ClusterRoles,
            AppView::Roles,
            AppView::ClusterRoleBindings,
            AppView::RoleBindings,
        ],
    ),
    (NavGroup::CustomResources, &[AppView::Extensions]),
];

const fn nav_group_bit(group: NavGroup) -> u16 {
    match group {
        NavGroup::Overview => 1 << 0,
        NavGroup::Workloads => 1 << 1,
        NavGroup::Network => 1 << 2,
        NavGroup::Config => 1 << 3,
        NavGroup::Storage => 1 << 4,
        NavGroup::Helm => 1 << 5,
        NavGroup::FluxCD => 1 << 6,
        NavGroup::AccessControl => 1 << 7,
        NavGroup::CustomResources => 1 << 8,
    }
}

fn collapsed_mask(collapsed: &HashSet<NavGroup>) -> u16 {
    collapsed
        .iter()
        .fold(0u16, |mask, group| mask | nav_group_bit(*group))
}

static SIDEBAR_ROWS_CACHE: LazyLock<Vec<Box<[SidebarItem]>>> = LazyLock::new(|| {
    let num_groups = SIDEBAR_GROUPS.len();
    let combos = 1usize << num_groups;
    let mut cache = Vec::with_capacity(combos);
    for mask in 0u16..(combos as u16) {
        let mut rows = Vec::with_capacity(56);
        for (group, views) in SIDEBAR_GROUPS {
            rows.push(SidebarItem::Group(*group));
            if mask & nav_group_bit(*group) == 0 {
                for view in *views {
                    rows.push(SidebarItem::View(*view));
                }
            }
        }
        cache.push(rows.into_boxed_slice());
    }
    cache
});

/// Ordered sidebar rows for the current collapsed state.
pub fn sidebar_rows(collapsed: &HashSet<NavGroup>) -> &'static [SidebarItem] {
    let mask = collapsed_mask(collapsed) as usize;
    &SIDEBAR_ROWS_CACHE[mask]
}

/// Actions emitted by input handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    None,
    RefreshData,
    FluxReconcile,
    Quit,
    OpenDetail(ResourceRef),
    CloseDetail,
    OpenNamespacePicker,
    CloseNamespacePicker,
    SelectNamespace(String),
    OpenContextPicker,
    CloseContextPicker,
    SelectContext(String),
    OpenCommandPalette,
    CloseCommandPalette,
    NavigateTo(AppView),
    ToggleNavGroup(NavGroup),
    EscapePressed,
    LogsViewerOpen,
    LogsViewerClose,
    LogsViewerScrollUp,
    LogsViewerScrollDown,
    LogsViewerScrollTop,
    LogsViewerScrollBottom,
    LogsViewerToggleFollow,
    LogsViewerTogglePrevious,
    LogsViewerToggleTimestamps,
    LogsViewerSelectContainer(String),
    /// User chose "All Containers" in the pod logs picker.
    LogsViewerSelectAllContainers,
    LogsViewerPickerUp,
    LogsViewerPickerDown,
    LogsViewerSearchOpen,
    LogsViewerSearchClose,
    LogsViewerSearchNext,
    LogsViewerSearchPrev,
    OpenResourceYaml,
    OpenResourceEvents,
    OpenActionHistory,
    OpenExec,
    PortForwardOpen,
    PortForwardCreate(
        (
            crate::k8s::portforward::PortForwardTarget,
            crate::k8s::portforward::PortForwardConfig,
        ),
    ),
    PortForwardRefresh,
    PortForwardStop(String),
    ScaleDialogOpen,
    ScaleDialogClose,
    ScaleDialogUpdateInput(char),
    ScaleDialogBackspace,
    ScaleDialogIncrement,
    ScaleDialogDecrement,
    ScaleDialogSubmit,
    ProbePanelOpen,
    ProbePanelClose,
    ProbeToggleExpand,
    ProbeSelectNext,
    ProbeSelectPrev,
    ToggleWorkbench,
    WorkbenchNextTab,
    WorkbenchPreviousTab,
    WorkbenchCloseActiveTab,
    WorkbenchIncreaseHeight,
    WorkbenchDecreaseHeight,
    WorkbenchToggleMaximize,
    ActionHistoryOpenSelected,
    ExecSelectContainer(String),
    ExecSendInput,
    RolloutRestart,
    EditYaml,
    DeleteResource,
    CycleTheme,
    OpenHelp,
    CloseHelp,
}

/// Which panel currently owns keyboard focus.
///
/// Focus determines how `j`/`k`/`↑`/`↓` are routed:
/// - [`Focus::Sidebar`] → moves `sidebar_cursor` through the nav tree.
/// - [`Focus::Content`] → increments/decrements `selected_idx` in the active list.
///
/// # Transitions
/// - **Sidebar → Content**: `Enter` on a [`SidebarItem::View`] row (via [`AppState::sidebar_activate`]).
/// - **Content → Sidebar**: `Esc` while no detail view is open.
/// - **Tab / BackTab**: cycle through views directly, always lands in Content focus.
/// - **Command palette `NavigateTo`**: jumps to a view, lands in Content focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    /// Sidebar navigation panel has focus (default on startup).
    ///
    /// `j`/`k` move the sidebar cursor. `Enter` activates the highlighted row
    /// (either toggling a [`NavGroup`] or navigating to an [`AppView`]).
    #[default]
    Sidebar,
    /// Main content area has focus.
    ///
    /// `j`/`k` scroll `selected_idx` through the resource list. `Enter` opens
    /// the detail view for the highlighted row. `Esc` returns focus to the sidebar.
    Content,
    /// Bottom workbench owns keyboard input.
    ///
    /// `j`/`k` and related keys are delegated to the active workbench tab while
    /// the main list remains visible in the background.
    Workbench,
}

/// Runtime state for UI interaction and navigation.
///
/// # Navigation model
///
/// The UI has two independently navigable panels: the **sidebar** and the **content area**.
/// [`AppState::focus`] tracks which panel owns keyboard input at any given moment.
///
/// ```text
/// ┌─ Sidebar (Focus::Sidebar) ──┐  ┌─ Content (Focus::Content) ──────────────┐
/// │  ▼ Workloads                │  │  NAME        READY  STATUS  RESTARTS AGE │
/// │    Pods              ←─ Enter activates ──→  row 0  ← selected_idx        │
/// │    Deployments              │  │  row 1                                    │
/// │    ...                      │  │  row 2                                    │
/// └─────────────────────────────┘  └───────────────────────────────────────────┘
///       j/k: sidebar_cursor              j/k: selected_idx
///       Enter: navigate → Content        Enter: open detail view
///                                        Esc: return → Sidebar
/// ```
#[derive(Debug, Clone)]
pub struct AppState {
    /// The currently active top-level view (e.g. Pods, Deployments).
    pub view: AppView,
    /// Zero-based index of the highlighted row in the active content list.
    /// Reset to `0` on every view change.
    pub selected_idx: usize,
    pub search_query: String,
    pub is_search_mode: bool,
    pub should_quit: bool,
    pub confirm_quit: bool,
    pub error_message: Option<String>,
    pub status_message: Option<String>,
    pub detail_view: Option<DetailViewState>,
    pub current_namespace: String,
    pub namespace_picker: NamespacePicker,
    pub context_picker: ContextPicker,
    pub command_palette: CommandPalette,
    pub help_overlay: crate::ui::components::help_overlay::HelpOverlay,
    /// Set of [`NavGroup`]s that are currently collapsed in the sidebar.
    pub collapsed_groups: HashSet<NavGroup>,
    /// Zero-based index of the highlighted row in the sidebar nav tree.
    /// Includes both group headers and view rows; collapsed groups hide their children.
    pub sidebar_cursor: usize,
    /// Which panel currently owns keyboard focus. See [`Focus`] for routing rules.
    pub focus: Focus,
    pub extension_instances: Vec<CustomResourceInfo>,
    pub extension_error: Option<String>,
    pub extension_selected_crd: Option<String>,
    /// When true, keyboard focus is on the instances pane (right) instead of CRD picker (left).
    pub extension_in_instances: bool,
    /// Cursor index within the instances list.
    pub extension_instance_cursor: usize,
    /// Auto-refresh interval in seconds (0 = disabled).
    pub refresh_interval_secs: u64,
    /// Optional shared name/age sort mode for workload list views.
    pub workload_sort: Option<WorkloadSortState>,
    /// Optional sort mode for Pods view.
    pub pod_sort: Option<PodSortState>,
    /// Active port-forward tunnels displayed in the PortForwarding view.
    pub tunnel_registry: crate::state::port_forward::TunnelRegistry,
    /// Canonical mutation/action history.
    pub action_history: ActionHistoryState,
    /// Persistent bottom workbench state.
    pub workbench: WorkbenchState,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            view: AppView::Dashboard,
            selected_idx: 0,
            search_query: String::new(),
            is_search_mode: false,
            should_quit: false,
            confirm_quit: false,
            error_message: None,
            status_message: None,
            detail_view: None,
            current_namespace: "all".to_string(),
            namespace_picker: NamespacePicker::new(vec!["all".to_string(), "default".to_string()]),
            context_picker: ContextPicker::default(),
            command_palette: CommandPalette::default(),
            help_overlay: crate::ui::components::help_overlay::HelpOverlay::default(),
            collapsed_groups: HashSet::new(),
            sidebar_cursor: 0,
            focus: Focus::Sidebar,
            extension_instances: Vec::new(),
            extension_error: None,
            extension_selected_crd: None,
            extension_in_instances: false,
            extension_instance_cursor: 0,
            refresh_interval_secs: 30,
            workload_sort: None,
            pod_sort: None,
            tunnel_registry: crate::state::port_forward::TunnelRegistry::new(),
            action_history: ActionHistoryState::default(),
            workbench: WorkbenchState::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    namespace: String,
    #[serde(default)]
    theme: Option<String>,
    /// Auto-refresh interval in seconds (0 = disabled, default = 30).
    #[serde(default = "default_refresh_interval")]
    refresh_interval_secs: u64,
    #[serde(default)]
    workbench_open: bool,
    #[serde(default = "default_workbench_height")]
    workbench_height: u16,
}

fn default_refresh_interval() -> u64 {
    30
}

fn default_workbench_height() -> u16 {
    DEFAULT_WORKBENCH_HEIGHT
}

impl AppState {
    /// Returns the active top-level view.
    pub fn view(&self) -> AppView {
        self.view
    }

    /// Returns the currently selected list index.
    pub fn selected_idx(&self) -> usize {
        self.selected_idx
    }

    /// Returns the active shared sort mode for the given view, if supported.
    pub fn workload_sort_for_view(&self, view: AppView) -> Option<WorkloadSortState> {
        self.workload_sort
            .filter(|sort| view.supports_shared_sort(sort.column))
    }

    /// Returns the active shared sort mode for the current view, if supported.
    pub fn workload_sort(&self) -> Option<WorkloadSortState> {
        self.workload_sort_for_view(self.view)
    }

    /// Returns the active search query.
    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    /// Returns the currently active Pods sort mode, if any.
    pub fn pod_sort(&self) -> Option<PodSortState> {
        self.pod_sort
    }

    /// Returns whether the app is currently in search input mode.
    pub fn is_search_mode(&self) -> bool {
        self.is_search_mode
    }

    /// Returns the current workbench state.
    pub fn workbench(&self) -> &WorkbenchState {
        &self.workbench
    }

    pub fn workbench_mut(&mut self) -> &mut WorkbenchState {
        &mut self.workbench
    }

    pub fn action_history(&self) -> &ActionHistoryState {
        &self.action_history
    }

    pub fn open_action_history_tab(&mut self, focus: bool) {
        let history_key = crate::workbench::WorkbenchTabKey::ActionHistory;
        if focus {
            if !self.workbench.activate_tab(&history_key) {
                self.workbench.open_tab(WorkbenchTabState::ActionHistory(
                    ActionHistoryTabState::default(),
                ));
            }
            self.focus_workbench();
        } else if !self.workbench.has_tab(&history_key) {
            self.workbench
                .ensure_background_tab(WorkbenchTabState::ActionHistory(
                    ActionHistoryTabState::default(),
                ));
        }
    }

    pub fn record_action_pending(
        &mut self,
        kind: ActionKind,
        origin_view: AppView,
        resource: Option<ResourceRef>,
        resource_label: impl Into<String>,
        message: impl Into<String>,
    ) -> u64 {
        self.open_action_history_tab(false);
        let target = resource.map(|resource| ActionHistoryTarget {
            view: origin_view,
            resource,
        });
        self.action_history
            .record_pending(kind, resource_label, message, target)
    }

    pub fn complete_action_history(
        &mut self,
        entry_id: u64,
        status: ActionStatus,
        message: impl Into<String>,
        keep_target: bool,
    ) {
        self.action_history
            .complete(entry_id, status, message, keep_target);
    }

    pub fn selected_action_history_target(&self) -> Option<&ActionHistoryTarget> {
        let tab = self.workbench.active_tab()?;
        let WorkbenchTabState::ActionHistory(history_tab) = &tab.state else {
            return None;
        };
        self.action_history
            .get(history_tab.selected)
            .and_then(|entry| entry.target.as_ref())
    }

    /// Returns whether the event loop should terminate.
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Returns the latest UI-level error, if any.
    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    /// Returns the latest non-error status message, if any.
    pub fn status_message(&self) -> Option<&str> {
        self.status_message.as_deref()
    }

    /// Sets an error message to be shown in the status bar.
    pub fn set_error(&mut self, message: String) {
        self.status_message = None;
        self.error_message = Some(message);
    }

    /// Clears any active error message.
    pub fn clear_error(&mut self) {
        self.error_message = None;
    }

    /// Sets a transient non-error status message in the status bar.
    pub fn set_status(&mut self, message: String) {
        self.error_message = None;
        self.status_message = Some(message);
    }

    /// Clears any active non-error status message.
    pub fn clear_status(&mut self) {
        self.status_message = None;
    }

    pub fn toggle_workbench(&mut self) {
        self.workbench.toggle_open();
        if !self.workbench.open && self.focus == Focus::Workbench {
            self.focus = Focus::Content;
        }
    }

    pub fn workbench_next_tab(&mut self) {
        self.workbench.next_tab();
    }

    pub fn workbench_previous_tab(&mut self) {
        self.workbench.previous_tab();
    }

    pub fn workbench_close_active_tab(&mut self) {
        self.workbench.close_active_tab();
        if self.workbench.tabs.is_empty() && self.focus == Focus::Workbench {
            self.focus = Focus::Content;
        }
    }

    pub fn workbench_increase_height(&mut self) {
        self.workbench.resize_larger();
    }

    pub fn workbench_decrease_height(&mut self) {
        self.workbench.resize_smaller();
    }

    pub fn workbench_toggle_maximize(&mut self) {
        self.workbench.toggle_maximize();
    }

    /// Sets active namespace for namespaced resource fetches.
    pub fn set_namespace(&mut self, ns: String) {
        self.current_namespace = ns;
        self.selected_idx = 0;
    }

    /// Returns currently active namespace (`all` means cluster-wide listing).
    pub fn get_namespace(&self) -> &str {
        &self.current_namespace
    }

    /// Returns true when namespace picker modal is open.
    pub fn is_namespace_picker_open(&self) -> bool {
        self.namespace_picker.is_open()
    }

    /// Returns true when context picker modal is open.
    pub fn is_context_picker_open(&self) -> bool {
        self.context_picker.is_open()
    }

    /// Opens the context picker modal with the given contexts.
    pub fn open_context_picker(&mut self, contexts: Vec<String>, current: Option<String>) {
        self.context_picker.set_contexts(contexts, current);
        self.context_picker.open();
    }

    /// Closes the context picker modal.
    pub fn close_context_picker(&mut self) {
        self.context_picker.close();
    }

    /// Returns namespace picker state.
    pub fn namespace_picker(&self) -> &NamespacePicker {
        &self.namespace_picker
    }

    /// Replaces available namespace options while preserving current selection if possible.
    pub fn set_available_namespaces(&mut self, mut namespaces: Vec<String>) {
        namespaces.retain(|ns| !ns.is_empty());
        namespaces.sort();
        namespaces.dedup();

        if !namespaces.iter().any(|ns| ns == "all") {
            namespaces.insert(0, "all".to_string());
        }

        if !namespaces.iter().any(|ns| ns == &self.current_namespace) {
            namespaces.push(self.current_namespace.clone());
            namespaces.sort();
            namespaces.dedup();
        }

        self.namespace_picker.set_namespaces(namespaces);
    }

    /// Opens namespace picker modal.
    pub fn open_namespace_picker(&mut self) {
        self.namespace_picker.open();
    }

    /// Closes namespace picker modal.
    pub fn close_namespace_picker(&mut self) {
        self.namespace_picker.close();
    }

    /// Updates the currently displayed custom resource instances for Extensions view.
    pub fn set_extension_instances(
        &mut self,
        crd_name: String,
        instances: Vec<CustomResourceInfo>,
        error: Option<String>,
    ) {
        self.extension_selected_crd = Some(crd_name);
        self.extension_instances = instances;
        self.extension_error = error;
        self.extension_instance_cursor = 0;
    }

    /// Advances to the next view in [`AppView::ORDER`], wrapping around.
    /// Resets `selected_idx` and syncs `sidebar_cursor` to the new view.
    /// Triggered by `Tab`. Focus is not changed (Tab always targets content).
    fn next_view(&mut self) {
        self.view = self.view.next();
        self.selected_idx = 0;
        self.sync_sidebar_cursor_to_view();
    }

    /// Retreats to the previous view in [`AppView::ORDER`], wrapping around.
    /// Resets `selected_idx` and syncs `sidebar_cursor` to the new view.
    /// Triggered by `Shift+Tab`.
    fn previous_view(&mut self) {
        self.view = self.view.previous();
        self.selected_idx = 0;
        self.sync_sidebar_cursor_to_view();
    }

    /// Moves the content list selection down one row (saturates at `usize::MAX`).
    /// Called when [`Focus::Content`] is active and `j`/`↓` is pressed.
    fn select_next(&mut self) {
        self.selected_idx = self.selected_idx.saturating_add(1);
    }

    /// Moves the content list selection up one row (saturates at `0`).
    /// Called when [`Focus::Content`] is active and `k`/`↑` is pressed.
    fn select_previous(&mut self) {
        self.selected_idx = self.selected_idx.saturating_sub(1);
    }

    fn set_or_toggle_pod_sort(&mut self, column: PodSortColumn) {
        self.selected_idx = 0;
        self.pod_sort = match self.pod_sort {
            Some(current) if current.column == column => {
                Some(PodSortState::new(column, !current.descending))
            }
            _ => Some(PodSortState::new(column, column.default_descending())),
        };
    }

    fn clear_pod_sort(&mut self) {
        self.selected_idx = 0;
        self.pod_sort = None;
    }

    fn set_or_toggle_workload_sort(&mut self, column: WorkloadSortColumn) {
        self.selected_idx = 0;
        self.workload_sort = match self.workload_sort {
            Some(current) if current.column == column => {
                Some(WorkloadSortState::new(column, !current.descending))
            }
            _ => Some(WorkloadSortState::new(column, column.default_descending())),
        };
    }

    fn clear_workload_sort(&mut self) {
        self.selected_idx = 0;
        self.workload_sort = None;
    }

    /// Moves the sidebar cursor down one row, wrapping from the last row back to the first.
    /// Only called when [`Focus::Sidebar`] is active and `j`/`↓` is pressed.
    pub fn sidebar_cursor_down(&mut self) {
        let rows = sidebar_rows(&self.collapsed_groups);
        if rows.is_empty() {
            return;
        }
        self.sidebar_cursor = (self.sidebar_cursor + 1) % rows.len();
    }

    /// Moves the sidebar cursor up one row, wrapping from the first row back to the last.
    /// Only called when [`Focus::Sidebar`] is active and `k`/`↑` is pressed.
    pub fn sidebar_cursor_up(&mut self) {
        let rows = sidebar_rows(&self.collapsed_groups);
        if rows.is_empty() {
            return;
        }
        self.sidebar_cursor = if self.sidebar_cursor == 0 {
            rows.len() - 1
        } else {
            self.sidebar_cursor - 1
        };
    }

    /// Activates the currently highlighted sidebar row.
    ///
    /// - [`SidebarItem::Group`] → emits [`AppAction::ToggleNavGroup`] to collapse/expand it.
    /// - [`SidebarItem::View`] → switches `view`, resets `selected_idx` to `0`, and sets
    ///   [`Focus::Content`] so subsequent `j`/`k` scroll the resource list.
    ///
    /// Called from `main.rs` when `Enter` is pressed while [`Focus::Sidebar`] is active.
    pub fn sidebar_activate(&mut self) -> AppAction {
        let rows = sidebar_rows(&self.collapsed_groups);
        match rows.get(self.sidebar_cursor) {
            Some(SidebarItem::Group(g)) => AppAction::ToggleNavGroup(*g),
            Some(SidebarItem::View(v)) => {
                self.focus = Focus::Content;
                AppAction::NavigateTo(*v)
            }
            None => AppAction::None,
        }
    }

    /// Keeps `sidebar_cursor` pointing at the active view row after external navigation.
    ///
    /// Called after `Tab`/`Shift+Tab` view cycling so the sidebar highlight stays in sync
    /// with the active view even when the user didn't navigate via the sidebar cursor.
    /// No-op if the current view is not visible (e.g. its group is collapsed).
    pub fn sync_sidebar_cursor_to_view(&mut self) {
        let rows = sidebar_rows(&self.collapsed_groups);
        if let Some(idx) = rows.iter().position(|r| *r == SidebarItem::View(self.view)) {
            self.sidebar_cursor = idx;
        }
    }

    /// Toggles a nav group collapsed/expanded and clamps the cursor.
    pub fn toggle_nav_group(&mut self, group: NavGroup) {
        if self.collapsed_groups.contains(&group) {
            self.collapsed_groups.remove(&group);
        } else {
            self.collapsed_groups.insert(group);
        }
        let rows = sidebar_rows(&self.collapsed_groups);
        self.sidebar_cursor = self.sidebar_cursor.min(rows.len().saturating_sub(1));
    }

    /// Returns which detail sub-component is currently active.
    pub fn active_component(&self) -> ActiveComponent {
        if let Some(tab) = self.workbench.active_tab() {
            match tab.state {
                WorkbenchTabState::PodLogs(_) if self.focus == Focus::Workbench => {
                    return ActiveComponent::LogsViewer;
                }
                WorkbenchTabState::PortForward(_) if self.focus == Focus::Workbench => {
                    return ActiveComponent::PortForward;
                }
                _ => {}
            }
        }

        let Some(detail) = &self.detail_view else {
            return ActiveComponent::None;
        };

        if detail.scale_dialog.is_some() {
            ActiveComponent::Scale
        } else if detail.probe_panel.is_some() {
            ActiveComponent::ProbePanel
        } else {
            ActiveComponent::None
        }
    }

    /// Compatibility helper for tests and callers that previously opened the
    /// in-detail logs overlay. This now opens the canonical workbench tab.
    pub fn open_logs_viewer(&mut self) {
        if let Some(detail) = &self.detail_view
            && let Some(resource @ ResourceRef::Pod(_, _)) = detail.resource.clone()
        {
            self.open_pod_logs_tab(resource);
        }
    }

    pub fn close_logs_viewer(&mut self) {
        if matches!(
            self.workbench.active_tab().map(|tab| &tab.state),
            Some(WorkbenchTabState::PodLogs(_))
        ) {
            self.workbench_close_active_tab();
        }
        self.blur_workbench();
    }

    /// Compatibility helper for tests and callers that previously opened the
    /// in-detail port-forward overlay. This now opens the canonical workbench tab.
    pub fn open_port_forward(&mut self) {
        if let Some(detail) = &self.detail_view
            && let Some(ResourceRef::Pod(name, namespace)) = detail.resource.as_ref()
        {
            self.open_port_forward_tab(
                Some(ResourceRef::Pod(name.clone(), namespace.clone())),
                PortForwardDialog::with_target(namespace, name, 0),
            );
        }
    }

    pub fn close_port_forward(&mut self) {
        if matches!(
            self.workbench.active_tab().map(|tab| &tab.state),
            Some(WorkbenchTabState::PortForward(_))
        ) {
            self.workbench_close_active_tab();
        }
        self.blur_workbench();
    }

    pub fn focus_workbench(&mut self) {
        if self.workbench.open && !self.workbench.tabs.is_empty() {
            self.focus = Focus::Workbench;
        }
    }

    pub fn blur_workbench(&mut self) {
        if self.focus == Focus::Workbench {
            self.focus = Focus::Content;
        }
    }

    pub fn open_resource_yaml_tab(
        &mut self,
        resource: ResourceRef,
        yaml: Option<String>,
        error: Option<String>,
    ) {
        let mut tab = ResourceYamlTabState::new(resource);
        tab.yaml = yaml;
        tab.loading = tab.yaml.is_none() && error.is_none();
        tab.error = error;
        self.workbench
            .open_tab(WorkbenchTabState::ResourceYaml(tab));
        self.focus = Focus::Workbench;
    }

    pub fn open_resource_events_tab(
        &mut self,
        resource: ResourceRef,
        events: Vec<EventInfo>,
        loading: bool,
        error: Option<String>,
    ) {
        let mut tab = ResourceEventsTabState::new(resource);
        tab.events = events;
        tab.loading = loading;
        tab.error = error;
        self.workbench
            .open_tab(WorkbenchTabState::ResourceEvents(tab));
        self.focus = Focus::Workbench;
    }

    pub fn open_pod_logs_tab(&mut self, resource: ResourceRef) {
        self.workbench
            .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(resource)));
        self.focus = Focus::Workbench;
    }

    pub fn open_workload_logs_tab(&mut self, resource: ResourceRef, session_id: u64) {
        self.workbench
            .open_tab(WorkbenchTabState::WorkloadLogs(WorkloadLogsTabState::new(
                resource, session_id,
            )));
        self.focus = Focus::Workbench;
    }

    pub fn open_exec_tab(
        &mut self,
        resource: ResourceRef,
        session_id: u64,
        pod_name: String,
        namespace: String,
    ) {
        self.workbench
            .open_tab(WorkbenchTabState::Exec(ExecTabState::new(
                resource, session_id, pod_name, namespace,
            )));
        self.focus = Focus::Workbench;
    }

    pub fn open_port_forward_tab(
        &mut self,
        target: Option<ResourceRef>,
        dialog: PortForwardDialog,
    ) {
        self.workbench
            .open_tab(WorkbenchTabState::PortForward(PortForwardTabState::new(
                target, dialog,
            )));
        self.focus = Focus::Workbench;
    }

    /// Convenience initializer used by tests and non-runtime callers.
    /// The runtime path in `main.rs` overrides this with snapshot-derived replicas.
    pub fn open_scale_dialog(&mut self) {
        if let Some(detail) = &mut self.detail_view
            && detail.supports_action(DetailAction::Scale)
        {
            let (target_kind, name, namespace, current_replicas) = detail
                .resource
                .as_ref()
                .and_then(|r| match r {
                    ResourceRef::Deployment(name, ns) => {
                        Some((ScaleTargetKind::Deployment, name.clone(), ns.clone(), 1i32))
                    }
                    ResourceRef::StatefulSet(name, ns) => {
                        Some((ScaleTargetKind::StatefulSet, name.clone(), ns.clone(), 1i32))
                    }
                    _ => None,
                })
                .unwrap_or((
                    ScaleTargetKind::Deployment,
                    String::new(),
                    "default".to_string(),
                    1,
                ));
            detail.scale_dialog = Some(ScaleDialogState::new(
                target_kind,
                name,
                namespace,
                current_replicas,
            ));
        }
    }

    pub fn close_scale_dialog(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.scale_dialog = None;
        }
    }

    pub fn open_probe_panel(&mut self) {
        if let Some(detail) = &mut self.detail_view
            && detail.supports_action(DetailAction::Probes)
        {
            let (pod_name, namespace) = detail
                .resource
                .as_ref()
                .and_then(|r| match r {
                    ResourceRef::Pod(name, ns) => Some((name.clone(), ns.clone())),
                    _ => None,
                })
                .unwrap_or_default();
            detail.probe_panel = Some(ProbePanelComponentState::new(
                pod_name,
                namespace,
                Vec::new(),
            ));
        }
    }

    pub fn close_probe_panel(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.probe_panel = None;
        }
    }

    fn handle_workbench_key_event(&mut self, key: KeyEvent) -> AppAction {
        use crate::ui::components::port_forward_dialog::PortForwardAction;

        // Common workbench keys (apply to all tab types)
        if key.code == KeyCode::Char('z') {
            return AppAction::WorkbenchToggleMaximize;
        }

        let action_history_len = self.action_history.entries().len();
        let Some(tab) = self.workbench.active_tab_mut() else {
            return AppAction::None;
        };

        match &mut tab.state {
            WorkbenchTabState::ActionHistory(tab) => match key.code {
                KeyCode::Esc => AppAction::EscapePressed,
                KeyCode::Char('j') | KeyCode::Down => {
                    tab.select_next(action_history_len);
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    tab.select_previous();
                    AppAction::None
                }
                KeyCode::Char('g') => {
                    tab.select_top();
                    AppAction::None
                }
                KeyCode::Char('G') => {
                    tab.select_bottom(action_history_len);
                    AppAction::None
                }
                KeyCode::PageDown => {
                    for _ in 0..10 {
                        tab.select_next(action_history_len);
                    }
                    AppAction::None
                }
                KeyCode::PageUp => {
                    for _ in 0..10 {
                        tab.select_previous();
                    }
                    AppAction::None
                }
                KeyCode::Enter => AppAction::ActionHistoryOpenSelected,
                _ => AppAction::None,
            },
            WorkbenchTabState::ResourceYaml(tab) => match key.code {
                KeyCode::Esc => AppAction::EscapePressed,
                KeyCode::Char('j') | KeyCode::Down => {
                    tab.scroll = tab.scroll.saturating_add(1);
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    tab.scroll = tab.scroll.saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('g') => {
                    tab.scroll = 0;
                    AppAction::None
                }
                KeyCode::Char('G') => {
                    let total = tab
                        .yaml
                        .as_ref()
                        .map(|yaml| yaml.lines().count())
                        .unwrap_or(0);
                    tab.scroll = total.saturating_sub(1);
                    AppAction::None
                }
                KeyCode::PageDown => {
                    tab.scroll = tab.scroll.saturating_add(10);
                    AppAction::None
                }
                KeyCode::PageUp => {
                    tab.scroll = tab.scroll.saturating_sub(10);
                    AppAction::None
                }
                _ => AppAction::None,
            },
            WorkbenchTabState::ResourceEvents(tab) => match key.code {
                KeyCode::Esc => AppAction::EscapePressed,
                KeyCode::Char('j') | KeyCode::Down => {
                    tab.scroll = tab.scroll.saturating_add(1);
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    tab.scroll = tab.scroll.saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('g') => {
                    tab.scroll = 0;
                    AppAction::None
                }
                KeyCode::Char('G') => {
                    tab.scroll = tab.events.len().saturating_sub(1);
                    AppAction::None
                }
                KeyCode::PageDown => {
                    tab.scroll = tab.scroll.saturating_add(10);
                    AppAction::None
                }
                KeyCode::PageUp => {
                    tab.scroll = tab.scroll.saturating_sub(10);
                    AppAction::None
                }
                _ => AppAction::None,
            },
            WorkbenchTabState::PodLogs(tab) => match key.code {
                KeyCode::Esc if !tab.viewer.searching => AppAction::EscapePressed,
                KeyCode::Char('k') | KeyCode::Up => {
                    if tab.viewer.picking_container {
                        AppAction::LogsViewerPickerUp
                    } else {
                        AppAction::LogsViewerScrollUp
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    if tab.viewer.picking_container {
                        AppAction::LogsViewerPickerDown
                    } else {
                        AppAction::LogsViewerScrollDown
                    }
                }
                KeyCode::Enter if tab.viewer.picking_container => {
                    if tab.viewer.container_cursor == 0 && tab.viewer.containers.len() > 1 {
                        // "All Containers" entry at index 0
                        AppAction::LogsViewerSelectAllContainers
                    } else {
                        // Single container: offset by 1 to skip the "All" entry
                        let real_idx = if tab.viewer.containers.len() > 1 {
                            tab.viewer.container_cursor.saturating_sub(1)
                        } else {
                            tab.viewer.container_cursor
                        };
                        tab.viewer
                            .containers
                            .get(real_idx)
                            .cloned()
                            .map(AppAction::LogsViewerSelectContainer)
                            .unwrap_or(AppAction::None)
                    }
                }
                KeyCode::Char('g') => AppAction::LogsViewerScrollTop,
                KeyCode::Char('G') => AppAction::LogsViewerScrollBottom,
                KeyCode::Char('f') if !tab.viewer.searching => AppAction::LogsViewerToggleFollow,
                KeyCode::Char('P') if !tab.viewer.picking_container => {
                    AppAction::LogsViewerTogglePrevious
                }
                KeyCode::Char('t') if !tab.viewer.picking_container && !tab.viewer.searching => {
                    AppAction::LogsViewerToggleTimestamps
                }
                KeyCode::Char('/') if !tab.viewer.searching && !tab.viewer.picking_container => {
                    AppAction::LogsViewerSearchOpen
                }
                KeyCode::Esc if tab.viewer.searching => AppAction::LogsViewerSearchClose,
                KeyCode::Enter if tab.viewer.searching => AppAction::LogsViewerSearchClose,
                KeyCode::Char('n') if !tab.viewer.searching && !tab.viewer.picking_container => {
                    AppAction::LogsViewerSearchNext
                }
                KeyCode::Char('N') if !tab.viewer.searching && !tab.viewer.picking_container => {
                    AppAction::LogsViewerSearchPrev
                }
                KeyCode::Backspace if tab.viewer.searching => {
                    tab.viewer.search_input.pop();
                    AppAction::None
                }
                KeyCode::Char(c) if tab.viewer.searching => {
                    tab.viewer.search_input.push(c);
                    AppAction::None
                }
                _ => AppAction::None,
            },
            WorkbenchTabState::WorkloadLogs(tab) => {
                let filtered_len = tab
                    .lines
                    .iter()
                    .filter(|line| tab.matches_filter(line))
                    .count();
                if tab.editing_text_filter {
                    match key.code {
                        KeyCode::Esc => {
                            tab.editing_text_filter = false;
                            tab.filter_input.clear();
                            AppAction::None
                        }
                        KeyCode::Enter => {
                            tab.text_filter = tab.filter_input.clone();
                            tab.editing_text_filter = false;
                            tab.scroll = 0;
                            AppAction::None
                        }
                        KeyCode::Backspace => {
                            tab.filter_input.pop();
                            AppAction::None
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            tab.filter_input.push(c);
                            AppAction::None
                        }
                        _ => AppAction::None,
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => AppAction::EscapePressed,
                        KeyCode::Char('j') | KeyCode::Down => {
                            tab.scroll = (tab.scroll + 1).min(filtered_len.saturating_sub(1));
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            tab.scroll = tab.scroll.saturating_sub(1);
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::Char('g') => {
                            tab.scroll = 0;
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::Char('G') => {
                            tab.scroll = filtered_len.saturating_sub(1);
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::PageDown => {
                            tab.scroll = (tab.scroll + 10).min(filtered_len.saturating_sub(1));
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::PageUp => {
                            tab.scroll = tab.scroll.saturating_sub(10);
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::Char('f') => {
                            tab.follow_mode = !tab.follow_mode;
                            if tab.follow_mode {
                                tab.scroll = filtered_len.saturating_sub(1);
                            }
                            AppAction::None
                        }
                        KeyCode::Char('/') => {
                            tab.editing_text_filter = true;
                            tab.filter_input = tab.text_filter.clone();
                            AppAction::None
                        }
                        KeyCode::Char('p') => {
                            tab.cycle_pod_filter();
                            AppAction::None
                        }
                        KeyCode::Char('c') => {
                            tab.cycle_container_filter();
                            AppAction::None
                        }
                        _ => AppAction::None,
                    }
                }
            }
            WorkbenchTabState::Exec(tab) => {
                if tab.picking_container {
                    match key.code {
                        KeyCode::Esc => AppAction::EscapePressed,
                        KeyCode::Char('k') | KeyCode::Up => {
                            tab.container_cursor = tab.container_cursor.saturating_sub(1);
                            AppAction::None
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            let max = tab.containers.len().saturating_sub(1);
                            tab.container_cursor = (tab.container_cursor + 1).min(max);
                            AppAction::None
                        }
                        KeyCode::Enter => tab
                            .containers
                            .get(tab.container_cursor)
                            .cloned()
                            .map(AppAction::ExecSelectContainer)
                            .unwrap_or(AppAction::None),
                        _ => AppAction::None,
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => AppAction::EscapePressed,
                        KeyCode::Enter => AppAction::ExecSendInput,
                        KeyCode::Backspace => {
                            tab.input.pop();
                            AppAction::None
                        }
                        KeyCode::Up => {
                            tab.scroll = tab.scroll.saturating_sub(1);
                            AppAction::None
                        }
                        KeyCode::Down => {
                            tab.scroll = (tab.scroll + 1).min(tab.lines.len().saturating_sub(1));
                            AppAction::None
                        }
                        KeyCode::PageUp => {
                            tab.scroll = tab.scroll.saturating_sub(10);
                            AppAction::None
                        }
                        KeyCode::PageDown => {
                            tab.scroll = (tab.scroll + 10).min(tab.lines.len().saturating_sub(1));
                            AppAction::None
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            tab.input.push(c);
                            AppAction::None
                        }
                        _ => AppAction::None,
                    }
                }
            }
            WorkbenchTabState::PortForward(tab) => match tab.dialog.handle_key(key) {
                PortForwardAction::None => AppAction::None,
                PortForwardAction::Refresh => AppAction::PortForwardRefresh,
                PortForwardAction::Close => AppAction::EscapePressed,
                PortForwardAction::Create(args) => AppAction::PortForwardCreate(args),
                PortForwardAction::Stop(tunnel_id) => AppAction::PortForwardStop(tunnel_id),
            },
        }
    }

    /// Routes a raw keyboard event to the appropriate handler and returns the resulting action.
    ///
    /// # Input routing priority (highest → lowest)
    ///
    /// 1. **Command palette** — when open, all keys are consumed by the palette.
    /// 2. **Context picker** — when open, all keys are consumed by the picker.
    /// 3. **Namespace picker** — when open, all keys are consumed by the picker.
    /// 4. **Search mode** — `/` activates it; `Esc`/`Enter` exits; all printable chars append to query.
    /// 5. **Active sub-component** (detail overlay):
    ///    - `LogsViewer`: `j`/`k` scroll lines, `g`/`G` jump to top/bottom, `f` toggles follow.
    ///    - `PortForward`: `Tab`/`BackTab` cycle fields, digits update port inputs.
    ///    - `Scale`: digits update replica count, `Backspace` deletes.
    ///    - `ProbePanel`: `j`/`k` select probe, `Space` toggles expand.
    /// 6. **Quit confirmation** — after `q`/`Esc`, `q`/`y`/`Enter` confirms; any other key cancels.
    /// 7. **Main navigation** (see table below).
    ///
    /// # Main navigation keys
    ///
    /// | Key | Condition | Effect |
    /// |-----|-----------|--------|
    /// | `q` | — | Enter quit confirmation |
    /// | `Esc` | detail view open | Close detail view |
    /// | `Esc` | `focus == Content` | Return focus to sidebar |
    /// | `Esc` | — | Enter quit confirmation |
    /// | `Tab` | — | Next view in [`AppView::ORDER`], sync sidebar cursor |
    /// | `Shift+Tab` | — | Previous view in [`AppView::ORDER`], sync sidebar cursor |
    /// | `j` / `↓` | no detail, `focus == Sidebar` | Move sidebar cursor down |
    /// | `j` / `↓` | no detail, `focus == Content` | Move content selection down |
    /// | `k` / `↑` | no detail, `focus == Sidebar` | Move sidebar cursor up |
    /// | `k` / `↑` | no detail, `focus == Content` | Move content selection up |
    /// | `n` | workload view, no detail | Sort by Name (toggle asc/desc on repeat) |
    /// | `a` | workload view, no detail | Sort by Age (toggle asc/desc on repeat) |
    /// | `1` | Pods view, no detail | Sort pods by Age (toggle asc/desc on repeat) |
    /// | `2` | Pods view, no detail | Sort pods by Status (toggle asc/desc on repeat) |
    /// | `3` | Pods view, no detail | Sort pods by Restarts (toggle asc/desc on repeat) |
    /// | `0` | workload view, no detail | Clear active sort and return to default order |
    /// | `/` | — | Enter search mode |
    /// | `~` | — | Open namespace picker |
    /// | `c` | no detail | Open context picker |
    /// | `:` | no detail | Open command palette |
    /// | `r` / `Ctrl+R` | — | Trigger data refresh |
    /// | `Shift+R` | Flux view or Flux detail | Reconcile selected Flux resource |
    ///
    /// `Enter` is **not** handled here — it is intercepted in `main.rs` before this method
    /// is called, because its behaviour depends on both `focus` and `detail_view`.
    pub fn handle_key_event(&mut self, key: KeyEvent) -> AppAction {
        if self.help_overlay.is_open() {
            return match key.code {
                KeyCode::Esc | KeyCode::Char('?') => AppAction::CloseHelp,
                KeyCode::Char('j') | KeyCode::Down => {
                    self.help_overlay.scroll_down();
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.help_overlay.scroll_up();
                    AppAction::None
                }
                _ => AppAction::None,
            };
        }

        if self.command_palette.is_open() {
            return match self.command_palette.handle_key(key) {
                CommandPaletteAction::None => AppAction::None,
                CommandPaletteAction::Navigate(view) => AppAction::NavigateTo(view),
                CommandPaletteAction::Close => AppAction::CloseCommandPalette,
            };
        }

        if self.context_picker.is_open() {
            return match self.context_picker.handle_key(key) {
                ContextPickerAction::None => AppAction::None,
                ContextPickerAction::Select(ctx) => AppAction::SelectContext(ctx),
                ContextPickerAction::Close => AppAction::CloseContextPicker,
            };
        }

        if self.namespace_picker.is_open() {
            return match self.namespace_picker.handle_key(key) {
                NamespacePickerAction::None => AppAction::None,
                NamespacePickerAction::Select(ns) => AppAction::SelectNamespace(ns),
                NamespacePickerAction::Close => AppAction::CloseNamespacePicker,
            };
        }

        if self.is_search_mode {
            return self.handle_search_input(key);
        }

        if self.focus == Focus::Workbench && self.workbench.open {
            return self.handle_workbench_key_event(key);
        }

        // Component-level routing priority:
        // Scale > ProbePanel > DetailView > MainView
        match self.active_component() {
            ActiveComponent::LogsViewer | ActiveComponent::PortForward => {
                return self.handle_workbench_key_event(key);
            }
            ActiveComponent::Scale => {
                return match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Enter => AppAction::ScaleDialogSubmit,
                    KeyCode::Backspace => AppAction::ScaleDialogBackspace,
                    KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Up => {
                        AppAction::ScaleDialogIncrement
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') | KeyCode::Down => {
                        AppAction::ScaleDialogDecrement
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() => AppAction::ScaleDialogUpdateInput(c),
                    _ => AppAction::None,
                };
            }
            ActiveComponent::ProbePanel => {
                return match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Char(' ') => AppAction::ProbeToggleExpand,
                    KeyCode::Char('j') | KeyCode::Down => AppAction::ProbeSelectNext,
                    KeyCode::Char('k') | KeyCode::Up => AppAction::ProbeSelectPrev,
                    _ => AppAction::None,
                };
            }
            ActiveComponent::None => {}
        }

        if self.confirm_quit {
            return match key.code {
                KeyCode::Char('q') | KeyCode::Char('y') | KeyCode::Enter => {
                    self.should_quit = true;
                    AppAction::Quit
                }
                _ => {
                    self.confirm_quit = false;
                    AppAction::None
                }
            };
        }

        match key.code {
            KeyCode::Char('q') => {
                self.confirm_quit = true;
                AppAction::None
            }
            KeyCode::Esc
                if self
                    .detail_view
                    .as_ref()
                    .map(|d| d.confirm_delete)
                    .unwrap_or(false) =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.confirm_delete = false;
                }
                AppAction::None
            }
            KeyCode::Esc if self.detail_view.is_some() => AppAction::CloseDetail,
            KeyCode::Esc if self.focus == Focus::Content => {
                self.focus = Focus::Sidebar;
                AppAction::None
            }
            KeyCode::Esc if self.focus == Focus::Workbench => {
                self.focus = Focus::Content;
                AppAction::None
            }
            KeyCode::Esc => {
                self.confirm_quit = true;
                AppAction::None
            }
            KeyCode::Char('l') | KeyCode::Char('L')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Logs))
                    || (self.detail_view.is_none() && self.focus == Focus::Content) =>
            {
                AppAction::LogsViewerOpen
            }
            KeyCode::Char('y')
                if (self.detail_view.as_ref().is_some_and(|detail| {
                    detail.supports_action(DetailAction::ViewYaml) && !detail.confirm_delete
                }) || (self.detail_view.is_none() && self.focus == Focus::Content)) =>
            {
                AppAction::OpenResourceYaml
            }
            KeyCode::Char('v')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::ViewEvents))
                    || (self.detail_view.is_none() && self.focus == Focus::Content) =>
            {
                AppAction::OpenResourceEvents
            }
            KeyCode::Char('H') => AppAction::OpenActionHistory,
            KeyCode::Char('x')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Exec))
                    || (self.detail_view.is_none() && self.focus == Focus::Content) =>
            {
                AppAction::OpenExec
            }
            KeyCode::Char('f')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::PortForward))
                    || (self.detail_view.is_none() && self.focus == Focus::Content) =>
            {
                AppAction::PortForwardOpen
            }
            KeyCode::Char('s')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Scale)) =>
            {
                AppAction::ScaleDialogOpen
            }
            KeyCode::Char('p')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Probes)) =>
            {
                AppAction::ProbePanelOpen
            }
            KeyCode::Char('R')
                if self.detail_view.is_some() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                match self.detail_view.as_ref() {
                    Some(detail) if detail.supports_action(DetailAction::Restart) => {
                        AppAction::RolloutRestart
                    }
                    Some(detail) if detail.supports_action(DetailAction::FluxReconcile) => {
                        AppAction::FluxReconcile
                    }
                    _ => AppAction::None,
                }
            }
            KeyCode::Char('e')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::EditYaml)) =>
            {
                AppAction::EditYaml
            }
            KeyCode::Char('d')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Delete)) =>
            {
                // Toggle delete confirmation prompt
                if let Some(detail) = &mut self.detail_view {
                    detail.confirm_delete = true;
                }
                AppAction::None
            }
            KeyCode::Char('D') | KeyCode::Char('y') | KeyCode::Enter
                if self
                    .detail_view
                    .as_ref()
                    .map(|d| d.confirm_delete)
                    .unwrap_or(false) =>
            {
                AppAction::DeleteResource
            }
            KeyCode::Tab if self.detail_view.is_none() => {
                self.next_view();
                AppAction::None
            }
            KeyCode::BackTab if self.detail_view.is_none() => {
                self.previous_view();
                AppAction::None
            }
            KeyCode::Char('j') | KeyCode::Down
                if self.detail_view.is_none() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                match self.focus {
                    Focus::Sidebar => self.sidebar_cursor_down(),
                    Focus::Content
                        if self.view == AppView::Extensions && self.extension_in_instances =>
                    {
                        if !self.extension_instances.is_empty() {
                            self.extension_instance_cursor = (self.extension_instance_cursor + 1)
                                % self.extension_instances.len();
                        }
                    }
                    Focus::Content => self.select_next(),
                    Focus::Workbench => {}
                }
                AppAction::None
            }
            KeyCode::Char('k') | KeyCode::Up
                if self.detail_view.is_none() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                match self.focus {
                    Focus::Sidebar => self.sidebar_cursor_up(),
                    Focus::Content
                        if self.view == AppView::Extensions && self.extension_in_instances =>
                    {
                        if !self.extension_instances.is_empty() {
                            self.extension_instance_cursor = if self.extension_instance_cursor == 0
                            {
                                self.extension_instances.len() - 1
                            } else {
                                self.extension_instance_cursor - 1
                            };
                        }
                    }
                    Focus::Content => self.select_previous(),
                    Focus::Workbench => {}
                }
                AppAction::None
            }
            KeyCode::Down
                if self.detail_view.is_none() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.select_next();
                AppAction::None
            }
            KeyCode::Up
                if self.detail_view.is_none() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.select_previous();
                AppAction::None
            }
            KeyCode::Char('n') if self.detail_view.is_none() && self.view == AppView::Pods => {
                self.set_or_toggle_pod_sort(PodSortColumn::Name);
                AppAction::None
            }
            KeyCode::Char('n')
                if self.detail_view.is_none()
                    && self.view.supports_shared_sort(WorkloadSortColumn::Name) =>
            {
                self.set_or_toggle_workload_sort(WorkloadSortColumn::Name);
                AppAction::None
            }
            KeyCode::Char('a') if self.detail_view.is_none() && self.view == AppView::Pods => {
                self.set_or_toggle_pod_sort(PodSortColumn::Age);
                AppAction::None
            }
            KeyCode::Char('a')
                if self.detail_view.is_none()
                    && self.view.supports_shared_sort(WorkloadSortColumn::Age) =>
            {
                self.set_or_toggle_workload_sort(WorkloadSortColumn::Age);
                AppAction::None
            }
            KeyCode::Char('1') if self.detail_view.is_none() && self.view == AppView::Pods => {
                self.set_or_toggle_pod_sort(PodSortColumn::Age);
                AppAction::None
            }
            KeyCode::Char('2') if self.detail_view.is_none() && self.view == AppView::Pods => {
                self.set_or_toggle_pod_sort(PodSortColumn::Status);
                AppAction::None
            }
            KeyCode::Char('3') if self.detail_view.is_none() && self.view == AppView::Pods => {
                self.set_or_toggle_pod_sort(PodSortColumn::Restarts);
                AppAction::None
            }
            KeyCode::Char('0') if self.detail_view.is_none() && self.view == AppView::Pods => {
                self.clear_pod_sort();
                AppAction::None
            }
            KeyCode::Char('0')
                if self.detail_view.is_none()
                    && !self.view.shared_sort_capabilities().is_empty() =>
            {
                self.clear_workload_sort();
                AppAction::None
            }
            KeyCode::Char('/') => {
                self.is_search_mode = true;
                AppAction::None
            }
            KeyCode::Char('~') => AppAction::OpenNamespacePicker,
            KeyCode::Char('b') if self.detail_view.is_none() => AppAction::ToggleWorkbench,
            KeyCode::Char('[') if self.detail_view.is_none() && self.workbench.open => {
                AppAction::WorkbenchPreviousTab
            }
            KeyCode::Char(']') if self.detail_view.is_none() && self.workbench.open => {
                AppAction::WorkbenchNextTab
            }
            KeyCode::Char('w')
                if self.detail_view.is_none()
                    && self.workbench.open
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                AppAction::WorkbenchCloseActiveTab
            }
            KeyCode::Up
                if self.detail_view.is_none()
                    && self.workbench.open
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                AppAction::WorkbenchIncreaseHeight
            }
            KeyCode::Down
                if self.detail_view.is_none()
                    && self.workbench.open
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                AppAction::WorkbenchDecreaseHeight
            }
            KeyCode::Char('c') if self.detail_view.is_none() => AppAction::OpenContextPicker,
            KeyCode::Char(':') if self.detail_view.is_none() => AppAction::OpenCommandPalette,
            KeyCode::Char('R')
                if self.detail_view.is_none()
                    && self
                        .view
                        .supports_view_action(ViewAction::SelectedFluxReconcile)
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                AppAction::FluxReconcile
            }
            KeyCode::Char('r') => AppAction::RefreshData,
            KeyCode::Char('R') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                AppAction::RefreshData
            }
            KeyCode::Char('T') if self.detail_view.is_none() => AppAction::CycleTheme,
            KeyCode::Char('?') => AppAction::OpenHelp,
            _ => AppAction::None,
        }
    }

    fn handle_search_input(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.search_query.clear();
                self.is_search_mode = false;
            }
            KeyCode::Enter => {
                self.is_search_mode = false;
            }
            KeyCode::Backspace => {
                self.search_query.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_query.clear();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_query.push(c);
            }
            _ => {}
        }
        AppAction::None
    }
}

/// Loads app state config from a given path.
pub fn load_config_from_path(path: &Path) -> AppState {
    let mut app = AppState::default();

    if let Ok(content) = fs::read_to_string(path)
        && let Ok(cfg) = serde_json::from_str::<AppConfig>(&content)
    {
        if !cfg.namespace.trim().is_empty() {
            app.set_namespace(cfg.namespace);
        }
        if let Some(theme_name) = &cfg.theme {
            let idx = match theme_name.to_lowercase().as_str() {
                "nord" => 1,
                "dracula" => 2,
                "catppuccin" | "mocha" => 3,
                "light" => 4,
                _ => 0,
            };
            crate::ui::theme::set_active_theme(idx);
        }
        app.refresh_interval_secs = cfg.refresh_interval_secs;
        app.workbench
            .set_open_and_height(cfg.workbench_open, cfg.workbench_height);
    }

    app
}

/// Saves app namespace config to a given path.
pub fn save_config_to_path(app: &AppState, path: &Path) {
    let theme_name = crate::ui::theme::active_theme().name;
    let cfg = AppConfig {
        namespace: app.current_namespace.clone(),
        theme: Some(theme_name.to_string()),
        refresh_interval_secs: app.refresh_interval_secs,
        workbench_open: app.workbench.open,
        workbench_height: app.workbench.height,
    };

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let serialized = serde_json::to_string(&cfg).unwrap_or_else(|_| "{}".to_string());
    let tmp = path.with_extension("tmp");
    if fs::write(&tmp, &serialized).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

/// Loads app config from ~/.kube/kubectui-config.json.
pub fn load_config() -> AppState {
    let path = dirs::home_dir()
        .unwrap_or_default()
        .join(".kube")
        .join("kubectui-config.json");
    load_config_from_path(&path)
}

/// Saves app config to ~/.kube/kubectui-config.json.
pub fn save_config(app: &AppState) {
    let path = dirs::home_dir()
        .unwrap_or_default()
        .join(".kube")
        .join("kubectui-config.json");
    save_config_to_path(app, &path);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies full forward tab cycle across all views and wraps to Dashboard.
    #[test]
    fn tab_cycles_all_views_forward() {
        let mut app = AppState::default();
        let expected = [
            // Overview
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
            // Wraps back to start
            AppView::Dashboard,
        ];
        for view in expected {
            app.handle_key_event(KeyEvent::from(KeyCode::Tab));
            assert_eq!(app.view(), view);
        }
    }

    /// Verifies reverse tab cycle wraps from Dashboard to Extensions.
    #[test]
    fn shift_tab_cycles_reverse() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::BackTab));
        assert_eq!(app.view(), AppView::Extensions);
    }

    /// Verifies entering search mode and adding/removing characters.
    #[test]
    fn search_query_add_backspace_and_clear() {
        let mut app = AppState::default();

        app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
        app.handle_key_event(KeyEvent::from(KeyCode::Char('a')));
        app.handle_key_event(KeyEvent::from(KeyCode::Char('b')));
        app.handle_key_event(KeyEvent::from(KeyCode::Backspace));

        assert_eq!(app.search_query(), "a");

        app.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(app.search_query(), "");
    }

    /// Verifies pressing Esc in search mode exits mode and clears query.
    #[test]
    fn search_mode_esc_clears_and_exits() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
        app.handle_key_event(KeyEvent::from(KeyCode::Char('x')));

        app.handle_key_event(KeyEvent::from(KeyCode::Esc));

        assert_eq!(app.search_query(), "");
        assert!(!app.is_search_mode());
    }

    /// Verifies refresh actions are emitted for `r` and Ctrl+R.
    #[test]
    fn refresh_action_transitions() {
        let mut app = AppState::default();
        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('r'))),
            AppAction::RefreshData
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::CONTROL)),
            AppAction::RefreshData
        );
    }

    #[test]
    fn flux_view_uppercase_r_triggers_reconcile_without_overriding_ctrl_r() {
        let mut app = AppState::default();
        app.view = AppView::FluxCDKustomizations;

        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('R'))),
            AppAction::FluxReconcile
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::CONTROL)),
            AppAction::RefreshData
        );
    }

    #[test]
    fn flux_detail_uppercase_r_triggers_reconcile_for_supported_resource() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::CustomResource {
                name: "apps".to_string(),
                namespace: Some("flux-system".to_string()),
                group: "kustomize.toolkit.fluxcd.io".to_string(),
                version: "v1".to_string(),
                kind: "Kustomization".to_string(),
                plural: "kustomizations".to_string(),
            }),
            ..DetailViewState::default()
        });

        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('R'))),
            AppAction::FluxReconcile
        );
    }

    #[test]
    fn unsupported_flux_detail_uppercase_r_is_noop() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::CustomResource {
                name: "webhook".to_string(),
                namespace: Some("flux-system".to_string()),
                group: "notification.toolkit.fluxcd.io".to_string(),
                version: "v1beta3".to_string(),
                kind: "Alert".to_string(),
                plural: "alerts".to_string(),
            }),
            ..DetailViewState::default()
        });

        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('R'))),
            AppAction::None
        );
    }

    /// Verifies namespace can be switched through dedicated mutators.
    #[test]
    fn test_appstate_namespace_switching() {
        let mut app = AppState::default();
        assert_eq!(app.get_namespace(), "all");

        app.set_namespace("kube-system".to_string());
        assert_eq!(app.get_namespace(), "kube-system");
    }

    /// Verifies namespace picker shortcut emits open action.
    #[test]
    fn tilde_opens_namespace_picker_action() {
        let mut app = AppState::default();
        let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('~')));
        assert_eq!(action, AppAction::OpenNamespacePicker);
    }

    #[test]
    fn pods_sort_keybindings_toggle_and_clear() {
        let mut app = AppState::default();
        app.view = AppView::Pods;
        app.focus = Focus::Content;

        assert_eq!(app.pod_sort(), None);

        app.handle_key_event(KeyEvent::from(KeyCode::Char('1')));
        assert_eq!(
            app.pod_sort(),
            Some(PodSortState::new(PodSortColumn::Age, true))
        );

        app.handle_key_event(KeyEvent::from(KeyCode::Char('1')));
        assert_eq!(
            app.pod_sort(),
            Some(PodSortState::new(PodSortColumn::Age, false))
        );

        app.handle_key_event(KeyEvent::from(KeyCode::Char('3')));
        assert_eq!(
            app.pod_sort(),
            Some(PodSortState::new(PodSortColumn::Restarts, true))
        );

        app.handle_key_event(KeyEvent::from(KeyCode::Char('0')));
        assert_eq!(app.pod_sort(), None);
    }

    #[test]
    fn pods_name_sort_shortcut_toggles() {
        let mut app = AppState::default();
        app.view = AppView::Pods;
        app.focus = Focus::Content;

        app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
        assert_eq!(
            app.pod_sort(),
            Some(PodSortState::new(PodSortColumn::Name, false))
        );

        app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
        assert_eq!(
            app.pod_sort(),
            Some(PodSortState::new(PodSortColumn::Name, true))
        );
    }

    #[test]
    fn pods_sort_keybindings_are_scoped_to_pods_view() {
        let mut app = AppState::default();
        app.view = AppView::Services;
        app.focus = Focus::Content;

        app.handle_key_event(KeyEvent::from(KeyCode::Char('1')));
        assert_eq!(app.pod_sort(), None);
    }

    #[test]
    fn workload_sort_keybindings_toggle_and_clear() {
        let mut app = AppState::default();
        app.view = AppView::Deployments;
        app.focus = Focus::Content;

        assert_eq!(app.workload_sort(), None);

        app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
        assert_eq!(
            app.workload_sort(),
            Some(WorkloadSortState::new(WorkloadSortColumn::Name, false))
        );

        app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
        assert_eq!(
            app.workload_sort(),
            Some(WorkloadSortState::new(WorkloadSortColumn::Name, true))
        );

        app.handle_key_event(KeyEvent::from(KeyCode::Char('a')));
        assert_eq!(
            app.workload_sort(),
            Some(WorkloadSortState::new(WorkloadSortColumn::Age, true))
        );

        app.handle_key_event(KeyEvent::from(KeyCode::Char('0')));
        assert_eq!(app.workload_sort(), None);
    }

    #[test]
    fn workload_sort_keybindings_are_scoped_to_workload_views() {
        let mut app = AppState::default();
        app.view = AppView::ConfigMaps;
        app.focus = Focus::Content;

        app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
        assert_eq!(app.workload_sort(), None);
    }

    #[test]
    fn workbench_keybindings_emit_expected_actions() {
        let mut app = AppState::default();

        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('b'))),
            AppAction::ToggleWorkbench
        );

        app.toggle_workbench();
        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char(']'))),
            AppAction::WorkbenchNextTab
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('['))),
            AppAction::WorkbenchPreviousTab
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)),
            AppAction::WorkbenchCloseActiveTab
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL)),
            AppAction::WorkbenchIncreaseHeight
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL)),
            AppAction::WorkbenchDecreaseHeight
        );
    }

    #[test]
    fn filtered_pod_indices_apply_restarts_sort_with_stable_tie_breakers() {
        let mut pods = vec![
            PodInfo {
                name: "zeta".to_string(),
                namespace: "prod".to_string(),
                status: "Running".to_string(),
                restarts: 2,
                ..PodInfo::default()
            },
            PodInfo {
                name: "alpha".to_string(),
                namespace: "dev".to_string(),
                status: "Pending".to_string(),
                restarts: 2,
                ..PodInfo::default()
            },
            PodInfo {
                name: "beta".to_string(),
                namespace: "prod".to_string(),
                status: "Running".to_string(),
                restarts: 5,
                ..PodInfo::default()
            },
        ];
        // Ensure deterministic age field ordering is not involved in this test.
        for pod in &mut pods {
            pod.created_at = None;
        }

        let sorted = filtered_pod_indices(
            &pods,
            "",
            Some(PodSortState::new(PodSortColumn::Restarts, true)),
        );

        // Highest restarts first, then namespace/name tie-breakers for equal restart count.
        assert_eq!(sorted, vec![2, 1, 0]);
    }

    #[test]
    fn filtered_workload_indices_apply_age_sort_with_name_tie_breaker() {
        #[derive(Clone)]
        struct Item {
            name: String,
            namespace: String,
            age: Option<std::time::Duration>,
        }

        let items = vec![
            Item {
                name: "zeta".to_string(),
                namespace: "prod".to_string(),
                age: Some(std::time::Duration::from_secs(60)),
            },
            Item {
                name: "alpha".to_string(),
                namespace: "dev".to_string(),
                age: Some(std::time::Duration::from_secs(60)),
            },
            Item {
                name: "beta".to_string(),
                namespace: "prod".to_string(),
                age: Some(std::time::Duration::from_secs(120)),
            },
        ];

        let sorted = filtered_workload_indices(
            &items,
            "",
            Some(WorkloadSortState::new(WorkloadSortColumn::Age, true)),
            |item, _| !item.name.is_empty(),
            |item| item.name.as_str(),
            |item| item.namespace.as_str(),
            |item| item.age,
        );

        assert_eq!(sorted, vec![2, 1, 0]);
    }

    /// Verifies namespace persistence round-trip via config helpers.
    #[test]
    fn test_namespace_persistence() {
        let path =
            std::env::temp_dir().join(format!("kubectui-config-test-{}.json", std::process::id()));

        let mut app = AppState::default();
        app.set_namespace("demo".to_string());
        app.toggle_workbench();
        app.workbench.height = 15;
        save_config_to_path(&app, &path);

        let loaded = load_config_from_path(&path);
        assert_eq!(loaded.get_namespace(), "demo");
        assert!(loaded.workbench.open);
        assert_eq!(loaded.workbench.height, 15);

        let _ = std::fs::remove_file(path);
    }

    /// Verifies quit requires confirmation: first q sets confirm_quit, second q quits.
    #[test]
    fn quit_action_sets_should_quit() {
        let mut app = AppState::default();

        let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('q')));
        assert_eq!(action, AppAction::None);
        assert!(app.confirm_quit);
        assert!(!app.should_quit());

        let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('q')));
        assert_eq!(action, AppAction::Quit);
        assert!(app.should_quit());
    }

    /// Verifies any other key cancels the quit confirmation.
    #[test]
    fn quit_confirm_cancelled_by_other_key() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Char('q')));
        assert!(app.confirm_quit);

        app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
        assert!(!app.confirm_quit);
        assert!(!app.should_quit());
    }

    /// Verifies Esc closes detail view before requesting app quit.
    #[test]
    fn esc_closes_detail_before_quit() {
        let mut app = AppState {
            detail_view: Some(DetailViewState::default()),
            ..AppState::default()
        };

        let action = app.handle_key_event(KeyEvent::from(KeyCode::Esc));

        assert_eq!(action, AppAction::CloseDetail);
        assert!(!app.should_quit());
    }

    /// Verifies selection index saturates at zero when moving up.
    #[test]
    fn selected_index_never_underflows() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Up));
        assert_eq!(app.selected_idx(), 0);
    }

    /// Verifies j/k move the sidebar cursor (not selected_idx) when no detail view.
    #[test]
    fn selected_index_grows_with_down_events() {
        let mut app = AppState::default();
        for _ in 0..5 {
            app.handle_key_event(KeyEvent::from(KeyCode::Down));
        }
        assert_eq!(app.sidebar_cursor, 5);
    }

    /// Verifies selection resets to zero when switching tabs.
    #[test]
    fn view_switch_resets_selection_index() {
        let mut app = AppState::default();
        app.selected_idx = 2;
        assert_eq!(app.selected_idx(), 2);

        app.handle_key_event(KeyEvent::from(KeyCode::Tab));

        assert_eq!(app.selected_idx(), 0);
    }

    /// Verifies rapid tab switching remains stable.
    #[test]
    fn rapid_tab_switching_is_stable() {
        let mut app = AppState::default();

        for _ in 0..(AppView::tabs().len() * 3) {
            app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        }

        assert_eq!(app.view(), AppView::Dashboard);
    }

    /// Verifies search input ignores Ctrl-modified characters except supported shortcuts.
    #[test]
    fn search_input_ignores_ctrl_characters() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));

        app.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));

        assert_eq!(app.search_query(), "");
    }

    /// Verifies error message can be set and cleared.
    #[test]
    fn error_message_set_and_clear() {
        let mut app = AppState::default();
        app.set_error("boom".to_string());
        assert_eq!(app.error_message(), Some("boom"));

        app.clear_error();
        assert_eq!(app.error_message(), None);
    }

    #[test]
    fn status_message_set_and_clear() {
        let mut app = AppState::default();
        app.set_status("working".to_string());
        assert_eq!(app.status_message(), Some("working"));
        assert_eq!(app.error_message(), None);

        app.clear_status();
        assert_eq!(app.status_message(), None);
    }

    /// Verifies resource reference helper methods return expected kind/name/namespace.
    #[test]
    fn resource_ref_helpers_work_for_each_variant() {
        let node = ResourceRef::Node("n1".to_string());
        let pod = ResourceRef::Pod("p1".to_string(), "ns1".to_string());
        let statefulset = ResourceRef::StatefulSet("ss1".to_string(), "ns1".to_string());
        let quota = ResourceRef::ResourceQuota("rq1".to_string(), "ns1".to_string());
        let daemonset = ResourceRef::DaemonSet("ds1".to_string(), "ns1".to_string());
        let pv = ResourceRef::Pv("pv1".to_string());
        let cluster_role = ResourceRef::ClusterRole("cr1".to_string());

        assert_eq!(node.kind(), "Node");
        assert_eq!(node.name(), "n1");
        assert_eq!(node.namespace(), None);

        assert_eq!(pod.kind(), "Pod");
        assert_eq!(pod.name(), "p1");
        assert_eq!(pod.namespace(), Some("ns1"));

        assert_eq!(statefulset.kind(), "StatefulSet");
        assert_eq!(statefulset.name(), "ss1");
        assert_eq!(statefulset.namespace(), Some("ns1"));

        assert_eq!(quota.kind(), "ResourceQuota");
        assert_eq!(quota.name(), "rq1");
        assert_eq!(quota.namespace(), Some("ns1"));

        assert_eq!(daemonset.kind(), "DaemonSet");
        assert_eq!(daemonset.name(), "ds1");
        assert_eq!(daemonset.namespace(), Some("ns1"));

        assert_eq!(pv.kind(), "PersistentVolume");
        assert_eq!(pv.name(), "pv1");
        assert_eq!(pv.namespace(), None);

        assert_eq!(cluster_role.kind(), "ClusterRole");
        assert_eq!(cluster_role.name(), "cr1");
        assert_eq!(cluster_role.namespace(), None);

        let helm = ResourceRef::HelmRelease("my-release".to_string(), "default".to_string());
        assert_eq!(helm.kind(), "HelmRelease");
        assert_eq!(helm.name(), "my-release");
        assert_eq!(helm.namespace(), Some("default"));

        let cr = ResourceRef::CustomResource {
            name: "my-widget".to_string(),
            namespace: Some("prod".to_string()),
            group: "demo.io".to_string(),
            version: "v1".to_string(),
            kind: "Widget".to_string(),
            plural: "widgets".to_string(),
        };
        assert_eq!(cr.kind(), "Widget");
        assert_eq!(cr.name(), "my-widget");
        assert_eq!(cr.namespace(), Some("prod"));

        let cr_cluster = ResourceRef::CustomResource {
            name: "global".to_string(),
            namespace: None,
            group: "infra.io".to_string(),
            version: "v1beta1".to_string(),
            kind: "ClusterWidget".to_string(),
            plural: "clusterwidgets".to_string(),
        };
        assert_eq!(cr_cluster.kind(), "ClusterWidget");
        assert_eq!(cr_cluster.name(), "global");
        assert_eq!(cr_cluster.namespace(), None);
    }
}
