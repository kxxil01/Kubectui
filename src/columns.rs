//! Column definition registry and resolution for table views.

use ratatui::layout::Constraint;

use crate::app::AppView;
use crate::preferences::ViewPreferences;

/// Describes a single table column.
#[derive(Debug, Clone, Copy)]
pub struct ColumnDef {
    /// Stable identifier used in preferences (e.g. "name", "age").
    pub id: &'static str,
    /// Display header text.
    pub label: &'static str,
    /// Default width constraint.
    pub default_width: Constraint,
    /// If false, this column cannot be hidden (e.g. "name").
    pub hideable: bool,
    /// Whether this column is visible by default.
    pub default_visible: bool,
}

/// Returns the preference key for a given [`AppView`].
pub fn view_key(view: AppView) -> &'static str {
    match view {
        AppView::Dashboard => "dashboard",
        AppView::Issues => "issues",
        AppView::Nodes => "nodes",
        AppView::Namespaces => "namespaces",
        AppView::Events => "events",
        AppView::Pods => "pods",
        AppView::Deployments => "deployments",
        AppView::StatefulSets => "statefulsets",
        AppView::DaemonSets => "daemonsets",
        AppView::ReplicaSets => "replicasets",
        AppView::ReplicationControllers => "replicationcontrollers",
        AppView::Jobs => "jobs",
        AppView::CronJobs => "cronjobs",
        AppView::Services => "services",
        AppView::Endpoints => "endpoints",
        AppView::Ingresses => "ingresses",
        AppView::IngressClasses => "ingressclasses",
        AppView::NetworkPolicies => "networkpolicies",
        AppView::PortForwarding => "portforwarding",
        AppView::ConfigMaps => "configmaps",
        AppView::Secrets => "secrets",
        AppView::ResourceQuotas => "resourcequotas",
        AppView::LimitRanges => "limitranges",
        AppView::HPAs => "hpas",
        AppView::PodDisruptionBudgets => "poddisruptionbudgets",
        AppView::PriorityClasses => "priorityclasses",
        AppView::PersistentVolumeClaims => "pvcs",
        AppView::PersistentVolumes => "pvs",
        AppView::StorageClasses => "storageclasses",
        AppView::HelmCharts => "helmcharts",
        AppView::HelmReleases => "helmreleases",
        AppView::FluxCDAll => "flux_all",
        AppView::FluxCDAlertProviders => "flux_alertproviders",
        AppView::FluxCDAlerts => "flux_alerts",
        AppView::FluxCDArtifacts => "flux_artifacts",
        AppView::FluxCDHelmReleases => "flux_helmreleases",
        AppView::FluxCDHelmRepositories => "flux_helmrepositories",
        AppView::FluxCDImages => "flux_images",
        AppView::FluxCDKustomizations => "flux_kustomizations",
        AppView::FluxCDReceivers => "flux_receivers",
        AppView::FluxCDSources => "flux_sources",
        AppView::ServiceAccounts => "serviceaccounts",
        AppView::ClusterRoles => "clusterroles",
        AppView::Roles => "roles",
        AppView::ClusterRoleBindings => "clusterrolebindings",
        AppView::RoleBindings => "rolebindings",
        AppView::Extensions => "extensions",
    }
}

// ── Per-view column registries ──────────────────────────────────────

pub const POD_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Min(28),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Length(18),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "ip",
        label: "IP",
        default_width: Constraint::Length(16),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "status",
        label: "Status",
        default_width: Constraint::Length(20),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "node",
        label: "Node",
        default_width: Constraint::Length(22),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "restarts",
        label: "Restarts",
        default_width: Constraint::Length(10),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "age",
        label: "Age",
        default_width: Constraint::Length(9),
        hideable: true,
        default_visible: true,
    },
];

pub const DEPLOYMENT_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Length(24),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Length(16),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "ready",
        label: "Ready",
        default_width: Constraint::Length(9),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "updated",
        label: "Updated",
        default_width: Constraint::Length(9),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "available",
        label: "Available",
        default_width: Constraint::Length(11),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "age",
        label: "Age",
        default_width: Constraint::Length(9),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "image",
        label: "Image",
        default_width: Constraint::Min(20),
        hideable: true,
        default_visible: true,
    },
];

pub const NODE_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Percentage(26),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "status",
        label: "Status",
        default_width: Constraint::Percentage(28),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "roles",
        label: "Role",
        default_width: Constraint::Percentage(12),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "cpu",
        label: "CPU",
        default_width: Constraint::Percentage(12),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "memory",
        label: "Memory",
        default_width: Constraint::Percentage(12),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "age",
        label: "Age",
        default_width: Constraint::Percentage(10),
        hideable: true,
        default_visible: true,
    },
];

pub const SERVICE_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Length(24),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Length(16),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "type",
        label: "Type",
        default_width: Constraint::Length(14),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "cluster_ip",
        label: "ClusterIP",
        default_width: Constraint::Length(16),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "ports",
        label: "Ports",
        default_width: Constraint::Min(18),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "age",
        label: "Age",
        default_width: Constraint::Length(9),
        hideable: true,
        default_visible: true,
    },
];

pub const STATEFULSET_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Length(22),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Length(16),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "ready",
        label: "Ready",
        default_width: Constraint::Length(10),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "service",
        label: "Service",
        default_width: Constraint::Length(22),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "image",
        label: "Image",
        default_width: Constraint::Min(20),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "age",
        label: "Age",
        default_width: Constraint::Length(9),
        hideable: true,
        default_visible: true,
    },
];

pub const DAEMONSET_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Length(20),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Length(16),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "desired",
        label: "Desired",
        default_width: Constraint::Length(9),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "ready",
        label: "Ready",
        default_width: Constraint::Length(9),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "unavailable",
        label: "Unavailable",
        default_width: Constraint::Length(13),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "image",
        label: "Image",
        default_width: Constraint::Min(24),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "age",
        label: "Age",
        default_width: Constraint::Length(9),
        hideable: true,
        default_visible: true,
    },
];

pub const REPLICASET_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Length(28),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Length(16),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "desired",
        label: "Desired",
        default_width: Constraint::Length(9),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "ready",
        label: "Ready",
        default_width: Constraint::Length(9),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "available",
        label: "Available",
        default_width: Constraint::Length(11),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "image",
        label: "Image",
        default_width: Constraint::Min(24),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "age",
        label: "Age",
        default_width: Constraint::Length(9),
        hideable: true,
        default_visible: true,
    },
];

pub const JOB_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Length(22),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Length(16),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "status",
        label: "Status",
        default_width: Constraint::Length(11),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "completions",
        label: "Completions",
        default_width: Constraint::Length(13),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "duration",
        label: "Duration",
        default_width: Constraint::Length(11),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "active",
        label: "Active",
        default_width: Constraint::Length(8),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "failed",
        label: "Failed",
        default_width: Constraint::Length(8),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "age",
        label: "Age",
        default_width: Constraint::Length(9),
        hideable: true,
        default_visible: true,
    },
];

pub const CRONJOB_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Length(20),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Length(16),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "schedule",
        label: "Schedule",
        default_width: Constraint::Length(16),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "last_run",
        label: "Last Run",
        default_width: Constraint::Length(14),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "next_run",
        label: "Next Run",
        default_width: Constraint::Length(14),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "active",
        label: "Active",
        default_width: Constraint::Length(8),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "suspend",
        label: "Suspend",
        default_width: Constraint::Length(10),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "age",
        label: "Age",
        default_width: Constraint::Length(9),
        hideable: true,
        default_visible: true,
    },
];

pub const EVENT_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "type",
        label: "Type",
        default_width: Constraint::Length(10),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Length(16),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "object",
        label: "Object",
        default_width: Constraint::Length(24),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "reason",
        label: "Reason",
        default_width: Constraint::Length(16),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "count",
        label: "Count",
        default_width: Constraint::Length(8),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "message",
        label: "Message",
        default_width: Constraint::Min(20),
        hideable: true,
        default_visible: true,
    },
];

pub const NAMESPACE_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Percentage(75),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "status",
        label: "Status",
        default_width: Constraint::Percentage(25),
        hideable: true,
        default_visible: true,
    },
];

pub const CONFIGMAP_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Percentage(52),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Percentage(33),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "data",
        label: "Data",
        default_width: Constraint::Percentage(15),
        hideable: true,
        default_visible: true,
    },
];

pub const SECRET_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Percentage(35),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Percentage(25),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "type",
        label: "Type",
        default_width: Constraint::Percentage(25),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "data",
        label: "Data",
        default_width: Constraint::Percentage(15),
        hideable: true,
        default_visible: true,
    },
];

pub const PVC_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Percentage(25),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Percentage(15),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "status",
        label: "Status",
        default_width: Constraint::Percentage(10),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "capacity",
        label: "Capacity",
        default_width: Constraint::Percentage(12),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "access_modes",
        label: "Access Modes",
        default_width: Constraint::Percentage(18),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "storageclass",
        label: "StorageClass",
        default_width: Constraint::Percentage(20),
        hideable: true,
        default_visible: true,
    },
];

pub const PV_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Percentage(20),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "capacity",
        label: "Capacity",
        default_width: Constraint::Percentage(12),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "access_modes",
        label: "Access Modes",
        default_width: Constraint::Percentage(15),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "reclaim",
        label: "Reclaim",
        default_width: Constraint::Percentage(12),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "status",
        label: "Status",
        default_width: Constraint::Percentage(12),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "claim",
        label: "Claim",
        default_width: Constraint::Percentage(15),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "storageclass",
        label: "StorageClass",
        default_width: Constraint::Percentage(14),
        hideable: true,
        default_visible: true,
    },
];

pub const STORAGECLASS_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Percentage(25),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "provisioner",
        label: "Provisioner",
        default_width: Constraint::Percentage(25),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "reclaim",
        label: "Reclaim",
        default_width: Constraint::Percentage(15),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "binding_mode",
        label: "Binding Mode",
        default_width: Constraint::Percentage(20),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "expand",
        label: "Expand",
        default_width: Constraint::Percentage(15),
        hideable: true,
        default_visible: true,
    },
];

pub const HPA_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Percentage(23),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Percentage(18),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "reference",
        label: "Reference",
        default_width: Constraint::Percentage(29),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "min",
        label: "Min",
        default_width: Constraint::Percentage(8),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "max",
        label: "Max",
        default_width: Constraint::Percentage(8),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "replicas",
        label: "Replicas",
        default_width: Constraint::Percentage(14),
        hideable: true,
        default_visible: true,
    },
];

pub const PRIORITY_CLASS_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Percentage(30),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "value",
        label: "Value",
        default_width: Constraint::Percentage(10),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "global_default",
        label: "Global Default",
        default_width: Constraint::Percentage(15),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "description",
        label: "Description",
        default_width: Constraint::Percentage(45),
        hideable: true,
        default_visible: true,
    },
];

pub const NETWORK_POLICY_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Percentage(26),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Percentage(20),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "pod_selector",
        label: "Pod Selector",
        default_width: Constraint::Percentage(34),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "ingress",
        label: "Ingress",
        default_width: Constraint::Percentage(10),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "egress",
        label: "Egress",
        default_width: Constraint::Percentage(10),
        hideable: true,
        default_visible: true,
    },
];

pub const ENDPOINT_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Percentage(28),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Percentage(20),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "addresses",
        label: "Addresses",
        default_width: Constraint::Percentage(30),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "ports",
        label: "Ports",
        default_width: Constraint::Percentage(22),
        hideable: true,
        default_visible: true,
    },
];

pub const INGRESS_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Percentage(26),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Percentage(16),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "class",
        label: "Class",
        default_width: Constraint::Percentage(16),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "hosts",
        label: "Hosts",
        default_width: Constraint::Percentage(27),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "address",
        label: "Address",
        default_width: Constraint::Percentage(15),
        hideable: true,
        default_visible: true,
    },
];

pub const HELM_RELEASE_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Percentage(18),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Percentage(14),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "chart",
        label: "Chart",
        default_width: Constraint::Percentage(20),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "version",
        label: "Version",
        default_width: Constraint::Percentage(10),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "status",
        label: "Status",
        default_width: Constraint::Percentage(14),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "revision",
        label: "Revision",
        default_width: Constraint::Percentage(8),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "updated",
        label: "Updated",
        default_width: Constraint::Percentage(16),
        hideable: true,
        default_visible: true,
    },
];

pub const ISSUE_COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        id: "severity",
        label: "Sev",
        default_width: Constraint::Length(3),
        hideable: false,
        default_visible: true,
    },
    ColumnDef {
        id: "category",
        label: "Category",
        default_width: Constraint::Length(20),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "kind",
        label: "Kind",
        default_width: Constraint::Length(14),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "name",
        label: "Name",
        default_width: Constraint::Min(20),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "namespace",
        label: "Namespace",
        default_width: Constraint::Length(16),
        hideable: true,
        default_visible: true,
    },
    ColumnDef {
        id: "message",
        label: "Message",
        default_width: Constraint::Min(20),
        hideable: true,
        default_visible: true,
    },
];

/// Returns the column registry for a view, or `None` for views without table
/// columns (Dashboard, PortForwarding, Extensions, HelmCharts, Flux views).
pub fn columns_for_view(view: AppView) -> Option<&'static [ColumnDef]> {
    match view {
        AppView::Pods => Some(POD_COLUMNS),
        AppView::Deployments => Some(DEPLOYMENT_COLUMNS),
        AppView::Nodes => Some(NODE_COLUMNS),
        AppView::Services => Some(SERVICE_COLUMNS),
        AppView::StatefulSets => Some(STATEFULSET_COLUMNS),
        AppView::DaemonSets => Some(DAEMONSET_COLUMNS),
        AppView::ReplicaSets | AppView::ReplicationControllers => Some(REPLICASET_COLUMNS),
        AppView::Jobs => Some(JOB_COLUMNS),
        AppView::CronJobs => Some(CRONJOB_COLUMNS),
        AppView::Events => Some(EVENT_COLUMNS),
        AppView::Namespaces => Some(NAMESPACE_COLUMNS),
        AppView::ConfigMaps => Some(CONFIGMAP_COLUMNS),
        AppView::Secrets => Some(SECRET_COLUMNS),
        AppView::PersistentVolumeClaims => Some(PVC_COLUMNS),
        AppView::PersistentVolumes => Some(PV_COLUMNS),
        AppView::StorageClasses => Some(STORAGECLASS_COLUMNS),
        AppView::HPAs => Some(HPA_COLUMNS),
        AppView::PriorityClasses => Some(PRIORITY_CLASS_COLUMNS),
        AppView::NetworkPolicies => Some(NETWORK_POLICY_COLUMNS),
        AppView::Endpoints => Some(ENDPOINT_COLUMNS),
        AppView::Ingresses => Some(INGRESS_COLUMNS),
        AppView::HelmReleases => Some(HELM_RELEASE_COLUMNS),
        AppView::Issues => Some(ISSUE_COLUMNS),
        _ => None,
    }
}

/// Resolves the visible columns for a view given user preferences.
///
/// 1. Start with all columns where `default_visible` is true
/// 2. Remove columns listed in `hidden_columns` (skip non-hideable)
/// 3. Apply `column_order` if set (unknown IDs skipped, remaining appended)
pub fn resolve_columns(registry: &[ColumnDef], prefs: &ViewPreferences) -> Vec<ColumnDef> {
    let mut visible: Vec<ColumnDef> = registry
        .iter()
        .filter(|c| c.default_visible)
        .copied()
        .collect();

    // Remove hidden columns (respect hideable flag)
    if !prefs.hidden_columns.is_empty() {
        visible.retain(|c| !c.hideable || !prefs.hidden_columns.iter().any(|h| h == c.id));
    }

    // Apply custom ordering if set
    if let Some(order) = &prefs.column_order {
        let mut ordered = Vec::with_capacity(visible.len());
        for id in order {
            if let Some(pos) = visible.iter().position(|c| c.id == id.as_str()) {
                ordered.push(visible.remove(pos));
            }
        }
        ordered.extend(visible);
        return ordered;
    }

    visible
}

/// Builds a `Vec<Constraint>` from the resolved visible columns.
pub fn visible_constraints(columns: &[ColumnDef]) -> Vec<Constraint> {
    columns.iter().map(|c| c.default_width).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_COLS: &[ColumnDef] = &[
        ColumnDef {
            id: "name",
            label: "Name",
            default_width: Constraint::Min(20),
            hideable: false,
            default_visible: true,
        },
        ColumnDef {
            id: "namespace",
            label: "Namespace",
            default_width: Constraint::Length(18),
            hideable: true,
            default_visible: true,
        },
        ColumnDef {
            id: "status",
            label: "Status",
            default_width: Constraint::Length(12),
            hideable: true,
            default_visible: true,
        },
        ColumnDef {
            id: "age",
            label: "Age",
            default_width: Constraint::Length(9),
            hideable: true,
            default_visible: true,
        },
        ColumnDef {
            id: "image",
            label: "Image",
            default_width: Constraint::Length(30),
            hideable: true,
            default_visible: false,
        },
    ];

    #[test]
    fn default_visible_columns() {
        let prefs = ViewPreferences::default();
        let visible = resolve_columns(TEST_COLS, &prefs);
        let ids: Vec<&str> = visible.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["name", "namespace", "status", "age"]);
    }

    #[test]
    fn hidden_columns_removed() {
        let prefs = ViewPreferences {
            hidden_columns: vec!["namespace".into()],
            ..Default::default()
        };
        let visible = resolve_columns(TEST_COLS, &prefs);
        let ids: Vec<&str> = visible.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["name", "status", "age"]);
    }

    #[test]
    fn non_hideable_column_cannot_be_hidden() {
        let prefs = ViewPreferences {
            hidden_columns: vec!["name".into()],
            ..Default::default()
        };
        let visible = resolve_columns(TEST_COLS, &prefs);
        assert!(visible.iter().any(|c| c.id == "name"));
    }

    #[test]
    fn column_order_applied() {
        let prefs = ViewPreferences {
            column_order: Some(vec![
                "age".into(),
                "name".into(),
                "namespace".into(),
                "status".into(),
            ]),
            ..Default::default()
        };
        let visible = resolve_columns(TEST_COLS, &prefs);
        let ids: Vec<&str> = visible.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["age", "name", "namespace", "status"]);
    }

    #[test]
    fn column_order_with_unknown_ids_skipped() {
        let prefs = ViewPreferences {
            column_order: Some(vec!["age".into(), "unknown".into(), "name".into()]),
            ..Default::default()
        };
        let visible = resolve_columns(TEST_COLS, &prefs);
        let ids: Vec<&str> = visible.iter().map(|c| c.id).collect();
        // age, name from order, then namespace, status (remaining default-visible)
        assert_eq!(ids, vec!["age", "name", "namespace", "status"]);
    }

    #[test]
    fn default_invisible_column_not_shown() {
        let prefs = ViewPreferences::default();
        let visible = resolve_columns(TEST_COLS, &prefs);
        assert!(!visible.iter().any(|c| c.id == "image"));
    }

    #[test]
    fn constraints_from_visible() {
        let prefs = ViewPreferences::default();
        let visible = resolve_columns(TEST_COLS, &prefs);
        let constraints = visible_constraints(&visible);
        assert_eq!(constraints.len(), 4);
    }

    #[test]
    fn view_key_for_known_views() {
        assert_eq!(view_key(AppView::Pods), "pods");
        assert_eq!(view_key(AppView::Deployments), "deployments");
        assert_eq!(view_key(AppView::Nodes), "nodes");
        assert_eq!(view_key(AppView::FluxCDAll), "flux_all");
    }
}
