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

/// Standard visible, hideable column.
const fn col(id: &'static str, label: &'static str, width: Constraint) -> ColumnDef {
    ColumnDef {
        id,
        label,
        default_width: width,
        hideable: true,
        default_visible: true,
    }
}

/// Non-hideable column (always shown, typically the primary identifier).
const fn col_fixed(id: &'static str, label: &'static str, width: Constraint) -> ColumnDef {
    ColumnDef {
        id,
        label,
        default_width: width,
        hideable: false,
        default_visible: true,
    }
}

/// Hidden-by-default column (opt-in via user preferences).
const fn col_hidden(id: &'static str, label: &'static str, width: Constraint) -> ColumnDef {
    ColumnDef {
        id,
        label,
        default_width: width,
        hideable: true,
        default_visible: false,
    }
}

/// Returns the preference key and optional column registry for a given [`AppView`].
///
/// Single source of truth — [`view_key`] and [`columns_for_view`] delegate here.
fn view_info(view: AppView) -> (&'static str, Option<&'static [ColumnDef]>) {
    match view {
        AppView::Dashboard => ("dashboard", None),
        AppView::Projects => ("projects", None),
        AppView::Governance => ("governance", None),
        AppView::Bookmarks => ("bookmarks", None),
        AppView::Issues => ("issues", Some(ISSUE_COLUMNS)),
        AppView::HealthReport => ("health_report", Some(ISSUE_COLUMNS)),
        AppView::Vulnerabilities => ("vulnerabilities", Some(VULNERABILITY_COLUMNS)),
        AppView::Nodes => ("nodes", Some(NODE_COLUMNS)),
        AppView::Namespaces => ("namespaces", Some(NAMESPACE_COLUMNS)),
        AppView::Events => ("events", Some(EVENT_COLUMNS)),
        AppView::Pods => ("pods", Some(POD_COLUMNS)),
        AppView::Deployments => ("deployments", Some(DEPLOYMENT_COLUMNS)),
        AppView::StatefulSets => ("statefulsets", Some(STATEFULSET_COLUMNS)),
        AppView::DaemonSets => ("daemonsets", Some(DAEMONSET_COLUMNS)),
        AppView::ReplicaSets => ("replicasets", Some(REPLICASET_COLUMNS)),
        AppView::ReplicationControllers => ("replicationcontrollers", Some(REPLICASET_COLUMNS)),
        AppView::Jobs => ("jobs", Some(JOB_COLUMNS)),
        AppView::CronJobs => ("cronjobs", Some(CRONJOB_COLUMNS)),
        AppView::Services => ("services", Some(SERVICE_COLUMNS)),
        AppView::Endpoints => ("endpoints", Some(ENDPOINT_COLUMNS)),
        AppView::Ingresses => ("ingresses", Some(INGRESS_COLUMNS)),
        AppView::IngressClasses => ("ingressclasses", None),
        AppView::GatewayClasses => ("gatewayclasses", None),
        AppView::Gateways => ("gateways", None),
        AppView::HttpRoutes => ("httproutes", None),
        AppView::GrpcRoutes => ("grpcroutes", None),
        AppView::ReferenceGrants => ("referencegrants", None),
        AppView::NetworkPolicies => ("networkpolicies", Some(NETWORK_POLICY_COLUMNS)),
        AppView::PortForwarding => ("portforwarding", None),
        AppView::ConfigMaps => ("configmaps", Some(CONFIGMAP_COLUMNS)),
        AppView::Secrets => ("secrets", Some(SECRET_COLUMNS)),
        AppView::ResourceQuotas => ("resourcequotas", None),
        AppView::LimitRanges => ("limitranges", None),
        AppView::HPAs => ("hpas", Some(HPA_COLUMNS)),
        AppView::PodDisruptionBudgets => ("poddisruptionbudgets", None),
        AppView::PriorityClasses => ("priorityclasses", Some(PRIORITY_CLASS_COLUMNS)),
        AppView::PersistentVolumeClaims => ("pvcs", Some(PVC_COLUMNS)),
        AppView::PersistentVolumes => ("pvs", Some(PV_COLUMNS)),
        AppView::StorageClasses => ("storageclasses", Some(STORAGECLASS_COLUMNS)),
        AppView::HelmCharts => ("helmcharts", None),
        AppView::HelmReleases => ("helmreleases", Some(HELM_RELEASE_COLUMNS)),
        AppView::FluxCDAll => ("flux_all", None),
        AppView::FluxCDAlertProviders => ("flux_alertproviders", None),
        AppView::FluxCDAlerts => ("flux_alerts", None),
        AppView::FluxCDArtifacts => ("flux_artifacts", None),
        AppView::FluxCDHelmReleases => ("flux_helmreleases", None),
        AppView::FluxCDHelmRepositories => ("flux_helmrepositories", None),
        AppView::FluxCDImages => ("flux_images", None),
        AppView::FluxCDKustomizations => ("flux_kustomizations", None),
        AppView::FluxCDReceivers => ("flux_receivers", None),
        AppView::FluxCDSources => ("flux_sources", None),
        AppView::ServiceAccounts => ("serviceaccounts", None),
        AppView::ClusterRoles => ("clusterroles", None),
        AppView::Roles => ("roles", None),
        AppView::ClusterRoleBindings => ("clusterrolebindings", None),
        AppView::RoleBindings => ("rolebindings", None),
        AppView::Extensions => ("extensions", None),
    }
}

/// Returns the preference key for a given [`AppView`].
pub fn view_key(view: AppView) -> &'static str {
    view_info(view).0
}

// ── Per-view column registries ──────────────────────────────────────

pub const POD_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Min(28)),
    col("namespace", "Namespace", Constraint::Length(18)),
    col("ip", "IP", Constraint::Length(16)),
    col("status", "Status", Constraint::Length(20)),
    col("node", "Node", Constraint::Length(22)),
    col("restarts", "Restarts", Constraint::Length(10)),
    col("age", "Age", Constraint::Length(9)),
    col_hidden("cpu_usage", "CPU", Constraint::Length(10)),
    col_hidden("mem_usage", "Memory", Constraint::Length(10)),
    col_hidden("cpu_req", "CPU Req", Constraint::Length(10)),
    col_hidden("mem_req", "Mem Req", Constraint::Length(10)),
    col_hidden("cpu_lim", "CPU Lim", Constraint::Length(10)),
    col_hidden("mem_lim", "Mem Lim", Constraint::Length(10)),
    col_hidden("cpu_pct_req", "%CPU/R", Constraint::Length(8)),
    col_hidden("mem_pct_req", "%MEM/R", Constraint::Length(8)),
    col_hidden("cpu_pct_lim", "%CPU/L", Constraint::Length(8)),
    col_hidden("mem_pct_lim", "%MEM/L", Constraint::Length(8)),
];

pub const DEPLOYMENT_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Length(24)),
    col("namespace", "Namespace", Constraint::Length(16)),
    col("ready", "Ready", Constraint::Length(9)),
    col("updated", "Updated", Constraint::Length(9)),
    col("available", "Available", Constraint::Length(11)),
    col("age", "Age", Constraint::Length(9)),
    col("image", "Image", Constraint::Min(20)),
];

pub const NODE_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Percentage(22)),
    col("status", "Status", Constraint::Percentage(22)),
    col("roles", "Role", Constraint::Percentage(12)),
    col("cpu", "CPU", Constraint::Percentage(16)),
    col("memory", "Memory", Constraint::Percentage(16)),
    col("age", "Age", Constraint::Percentage(10)),
];

pub const VULNERABILITY_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Min(22)),
    col("namespace", "Namespace", Constraint::Length(18)),
    col("kind", "Kind", Constraint::Length(18)),
    col("critical", "Critical", Constraint::Length(10)),
    col("high", "High", Constraint::Length(8)),
    col("medium", "Medium", Constraint::Length(10)),
    col("fixable", "Fixable", Constraint::Length(10)),
    col("artifacts", "Artifacts", Constraint::Min(22)),
];

pub const SERVICE_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Length(24)),
    col("namespace", "Namespace", Constraint::Length(16)),
    col("type", "Type", Constraint::Length(14)),
    col("cluster_ip", "ClusterIP", Constraint::Length(16)),
    col("ports", "Ports", Constraint::Min(18)),
    col("age", "Age", Constraint::Length(9)),
];

pub const STATEFULSET_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Length(22)),
    col("namespace", "Namespace", Constraint::Length(16)),
    col("ready", "Ready", Constraint::Length(10)),
    col("service", "Service", Constraint::Length(22)),
    col("image", "Image", Constraint::Min(20)),
    col("age", "Age", Constraint::Length(9)),
];

pub const DAEMONSET_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Length(20)),
    col("namespace", "Namespace", Constraint::Length(16)),
    col("desired", "Desired", Constraint::Length(9)),
    col("ready", "Ready", Constraint::Length(9)),
    col("unavailable", "Unavailable", Constraint::Length(13)),
    col("image", "Image", Constraint::Min(24)),
    col("age", "Age", Constraint::Length(9)),
];

pub const REPLICASET_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Length(28)),
    col("namespace", "Namespace", Constraint::Length(16)),
    col("desired", "Desired", Constraint::Length(9)),
    col("ready", "Ready", Constraint::Length(9)),
    col("available", "Available", Constraint::Length(11)),
    col("image", "Image", Constraint::Min(24)),
    col("age", "Age", Constraint::Length(9)),
];

pub const JOB_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Length(22)),
    col("namespace", "Namespace", Constraint::Length(16)),
    col("status", "Status", Constraint::Length(11)),
    col("completions", "Completions", Constraint::Length(13)),
    col("duration", "Duration", Constraint::Length(11)),
    col("active", "Active", Constraint::Length(8)),
    col("failed", "Failed", Constraint::Length(8)),
    col("age", "Age", Constraint::Length(9)),
];

pub const CRONJOB_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Length(20)),
    col("namespace", "Namespace", Constraint::Length(16)),
    col("schedule", "Schedule", Constraint::Length(16)),
    col("last_run", "Last Run", Constraint::Length(14)),
    col("next_run", "Next Run", Constraint::Length(14)),
    col("active", "Active", Constraint::Length(8)),
    col("suspend", "Suspend", Constraint::Length(10)),
    col("age", "Age", Constraint::Length(9)),
];

pub const EVENT_COLUMNS: &[ColumnDef] = &[
    col_fixed("type", "Type", Constraint::Length(10)),
    col("namespace", "Namespace", Constraint::Length(16)),
    col("object", "Object", Constraint::Length(24)),
    col("reason", "Reason", Constraint::Length(16)),
    col("count", "Count", Constraint::Length(8)),
    col("message", "Message", Constraint::Min(20)),
];

pub const NAMESPACE_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Percentage(75)),
    col("status", "Status", Constraint::Percentage(25)),
];

pub const CONFIGMAP_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Percentage(52)),
    col("namespace", "Namespace", Constraint::Percentage(33)),
    col("data", "Data", Constraint::Percentage(15)),
];

pub const SECRET_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Percentage(35)),
    col("namespace", "Namespace", Constraint::Percentage(25)),
    col("type", "Type", Constraint::Percentage(25)),
    col("data", "Data", Constraint::Percentage(15)),
];

pub const PVC_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Percentage(25)),
    col("namespace", "Namespace", Constraint::Percentage(15)),
    col("status", "Status", Constraint::Percentage(10)),
    col("capacity", "Capacity", Constraint::Percentage(12)),
    col("access_modes", "Access Modes", Constraint::Percentage(18)),
    col("storageclass", "StorageClass", Constraint::Percentage(20)),
];

pub const PV_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Percentage(20)),
    col("capacity", "Capacity", Constraint::Percentage(12)),
    col("access_modes", "Access Modes", Constraint::Percentage(15)),
    col("reclaim", "Reclaim", Constraint::Percentage(12)),
    col("status", "Status", Constraint::Percentage(12)),
    col("claim", "Claim", Constraint::Percentage(15)),
    col("storageclass", "StorageClass", Constraint::Percentage(14)),
];

pub const STORAGECLASS_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Percentage(25)),
    col("provisioner", "Provisioner", Constraint::Percentage(25)),
    col("reclaim", "Reclaim", Constraint::Percentage(15)),
    col("binding_mode", "Binding Mode", Constraint::Percentage(20)),
    col("expand", "Expand", Constraint::Percentage(15)),
];

pub const HPA_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Percentage(23)),
    col("namespace", "Namespace", Constraint::Percentage(18)),
    col("reference", "Reference", Constraint::Percentage(29)),
    col("min", "Min", Constraint::Percentage(8)),
    col("max", "Max", Constraint::Percentage(8)),
    col("replicas", "Replicas", Constraint::Percentage(14)),
];

pub const PRIORITY_CLASS_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Percentage(30)),
    col("value", "Value", Constraint::Percentage(10)),
    col(
        "global_default",
        "Global Default",
        Constraint::Percentage(15),
    ),
    col("description", "Description", Constraint::Percentage(45)),
];

pub const NETWORK_POLICY_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Percentage(26)),
    col("namespace", "Namespace", Constraint::Percentage(20)),
    col("pod_selector", "Pod Selector", Constraint::Percentage(34)),
    col("ingress", "Ingress", Constraint::Percentage(10)),
    col("egress", "Egress", Constraint::Percentage(10)),
];

pub const ENDPOINT_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Percentage(28)),
    col("namespace", "Namespace", Constraint::Percentage(20)),
    col("addresses", "Addresses", Constraint::Percentage(30)),
    col("ports", "Ports", Constraint::Percentage(22)),
];

pub const INGRESS_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Percentage(26)),
    col("namespace", "Namespace", Constraint::Percentage(16)),
    col("class", "Class", Constraint::Percentage(16)),
    col("hosts", "Hosts", Constraint::Percentage(27)),
    col("address", "Address", Constraint::Percentage(15)),
];

pub const HELM_RELEASE_COLUMNS: &[ColumnDef] = &[
    col_fixed("name", "Name", Constraint::Percentage(18)),
    col("namespace", "Namespace", Constraint::Percentage(14)),
    col("chart", "Chart", Constraint::Percentage(20)),
    col("version", "Version", Constraint::Percentage(10)),
    col("status", "Status", Constraint::Percentage(14)),
    col("revision", "Revision", Constraint::Percentage(8)),
    col("updated", "Updated", Constraint::Percentage(16)),
];

pub const ISSUE_COLUMNS: &[ColumnDef] = &[
    col_fixed("severity", "Sev", Constraint::Length(3)),
    col("category", "Category", Constraint::Length(20)),
    col("kind", "Kind", Constraint::Length(14)),
    col("name", "Name", Constraint::Min(20)),
    col("namespace", "Namespace", Constraint::Length(16)),
    col("message", "Message", Constraint::Min(20)),
];

/// Returns the column registry for a view, or `None` for views without table
/// columns (Dashboard, PortForwarding, Extensions, HelmCharts, Flux views).
pub fn columns_for_view(view: AppView) -> Option<&'static [ColumnDef]> {
    view_info(view).1
}

/// Resolves the visible columns for a view given user preferences.
///
/// 1. Include non-hideable columns and default-visible columns unless hidden
/// 2. Include default-hidden columns only when explicitly listed in `shown_columns`
/// 3. Apply `column_order` if set (unknown IDs skipped, remaining appended)
pub fn resolve_columns(registry: &[ColumnDef], prefs: &ViewPreferences) -> Vec<ColumnDef> {
    let mut visible: Vec<ColumnDef> = registry
        .iter()
        .filter(|c| {
            if !c.hideable {
                return true;
            }
            if c.default_visible {
                !prefs.hidden_columns.iter().any(|hidden| hidden == c.id)
            } else {
                prefs.shown_columns.iter().any(|shown| shown == c.id)
            }
        })
        .copied()
        .collect();

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

const NARROW_DEPLOYMENT_WIDTH: u16 = 104;
const NARROW_NODE_WIDTH: u16 = 104;

fn compact_constraint_for_view(view: AppView, area_width: u16, id: &str) -> Option<Constraint> {
    match view {
        AppView::Deployments if area_width < NARROW_DEPLOYMENT_WIDTH => Some(match id {
            "name" => Constraint::Min(18),
            "namespace" => Constraint::Length(14),
            "ready" => Constraint::Length(8),
            "updated" => Constraint::Length(7),
            "available" => Constraint::Length(8),
            "age" => Constraint::Length(8),
            "image" => Constraint::Min(16),
            _ => return None,
        }),
        AppView::Nodes if area_width < NARROW_NODE_WIDTH => Some(match id {
            "name" => Constraint::Min(18),
            "status" => Constraint::Min(18),
            "roles" => Constraint::Length(10),
            "cpu" => Constraint::Length(14),
            "memory" => Constraint::Length(14),
            "age" => Constraint::Length(8),
            _ => return None,
        }),
        _ => None,
    }
}

pub fn visible_constraints_for_area(
    view: AppView,
    columns: &[ColumnDef],
    area_width: u16,
) -> Vec<Constraint> {
    columns
        .iter()
        .map(|column| {
            compact_constraint_for_view(view, area_width, column.id).unwrap_or(column.default_width)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_COLS: &[ColumnDef] = &[
        col_fixed("name", "Name", Constraint::Min(20)),
        col("namespace", "Namespace", Constraint::Length(18)),
        col("status", "Status", Constraint::Length(12)),
        col("age", "Age", Constraint::Length(9)),
        col_hidden("image", "Image", Constraint::Length(30)),
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
    fn shown_columns_add_default_hidden_column() {
        let prefs = ViewPreferences {
            shown_columns: vec!["image".into()],
            ..Default::default()
        };
        let visible = resolve_columns(TEST_COLS, &prefs);
        let ids: Vec<&str> = visible.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["name", "namespace", "status", "age", "image"]);
    }

    #[test]
    fn hidden_columns_remove_default_shown_column_even_when_shown_is_set() {
        let prefs = ViewPreferences {
            hidden_columns: vec!["namespace".into()],
            shown_columns: vec!["image".into()],
            ..Default::default()
        };
        let visible = resolve_columns(TEST_COLS, &prefs);
        let ids: Vec<&str> = visible.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["name", "status", "age", "image"]);
    }

    #[test]
    fn constraints_from_visible() {
        let prefs = ViewPreferences::default();
        let visible = resolve_columns(TEST_COLS, &prefs);
        let constraints = visible_constraints(&visible);
        assert_eq!(constraints.len(), 4);
    }

    #[test]
    fn deployment_constraints_switch_to_compact_profile() {
        let constraints =
            visible_constraints_for_area(AppView::Deployments, DEPLOYMENT_COLUMNS, 96);
        assert_eq!(constraints[0], Constraint::Min(18));
        assert_eq!(constraints[1], Constraint::Length(14));
        assert_eq!(constraints[6], Constraint::Min(16));
    }

    #[test]
    fn deployment_constraints_keep_wide_profile() {
        let constraints =
            visible_constraints_for_area(AppView::Deployments, DEPLOYMENT_COLUMNS, 132);
        assert_eq!(constraints[0], Constraint::Length(24));
        assert_eq!(constraints[4], Constraint::Length(11));
        assert_eq!(constraints[6], Constraint::Min(20));
    }

    #[test]
    fn node_constraints_switch_to_compact_profile() {
        let constraints = visible_constraints_for_area(AppView::Nodes, NODE_COLUMNS, 96);
        assert_eq!(constraints[0], Constraint::Min(18));
        assert_eq!(constraints[1], Constraint::Min(18));
        assert_eq!(constraints[2], Constraint::Length(10));
        assert_eq!(constraints[5], Constraint::Length(8));
    }

    #[test]
    fn node_constraints_keep_wide_profile() {
        let constraints = visible_constraints_for_area(AppView::Nodes, NODE_COLUMNS, 132);
        assert_eq!(constraints[0], Constraint::Percentage(22));
        assert_eq!(constraints[1], Constraint::Percentage(22));
        assert_eq!(constraints[5], Constraint::Percentage(10));
    }

    #[test]
    fn view_key_for_known_views() {
        assert_eq!(view_key(AppView::Pods), "pods");
        assert_eq!(view_key(AppView::Deployments), "deployments");
        assert_eq!(view_key(AppView::Nodes), "nodes");
        assert_eq!(view_key(AppView::FluxCDAll), "flux_all");
    }

    #[test]
    fn pod_columns_has_17_entries_with_10_metrics_hidden() {
        assert_eq!(POD_COLUMNS.len(), 17);
        let hidden: Vec<&str> = POD_COLUMNS
            .iter()
            .filter(|c| !c.default_visible)
            .map(|c| c.id)
            .collect();
        assert_eq!(
            hidden,
            vec![
                "cpu_usage",
                "mem_usage",
                "cpu_req",
                "mem_req",
                "cpu_lim",
                "mem_lim",
                "cpu_pct_req",
                "mem_pct_req",
                "cpu_pct_lim",
                "mem_pct_lim",
            ]
        );
    }

    #[test]
    fn node_columns_cpu_memory_wider_for_utilization() {
        let cpu = NODE_COLUMNS.iter().find(|c| c.id == "cpu").unwrap();
        let mem = NODE_COLUMNS.iter().find(|c| c.id == "memory").unwrap();
        assert_eq!(cpu.default_width, Constraint::Percentage(16));
        assert_eq!(mem.default_width, Constraint::Percentage(16));
    }
}
