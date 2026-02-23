//! Pure filtering helpers for list-oriented views.

use crate::k8s::dtos::{DeploymentInfo, NodeInfo, ServiceInfo};

/// Health categories derived from deployment ready replica state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentHealth {
    /// All desired replicas are ready.
    Healthy,
    /// Some replicas are ready, but not all.
    Degraded,
    /// No replicas are ready.
    Failed,
}

/// Filter selector for node readiness state.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatusFilter {
    /// Keep only Ready nodes.
    Ready,
    /// Keep only NotReady nodes.
    NotReady,
}

/// Filter selector for node role classification.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRoleFilter {
    /// Control-plane/master nodes.
    Master,
    /// Worker nodes.
    Worker,
    /// Any role.
    Any,
}

/// Supported sort keys for node lists.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSortBy {
    /// Sort by node name (case-insensitive).
    Name,
    /// Sort by readiness status, then by name.
    Status,
    /// Sort by allocatable capacity (CPU then memory), descending.
    Capacity,
}

/// Filters nodes by free-text query plus optional status and role constraints.
#[must_use]
pub fn filter_nodes(
    nodes: &[NodeInfo],
    query: &str,
    status: Option<NodeStatusFilter>,
    role: Option<NodeRoleFilter>,
) -> Vec<NodeInfo> {
    let query = query.trim().to_ascii_lowercase();

    nodes
        .iter()
        .filter(|node| query.is_empty() || node.name.to_ascii_lowercase().contains(query.as_str()))
        .filter(|node| match status {
            Some(NodeStatusFilter::Ready) => node.ready,
            Some(NodeStatusFilter::NotReady) => !node.ready,
            None => true,
        })
        .filter(|node| match role.unwrap_or(NodeRoleFilter::Any) {
            NodeRoleFilter::Master => node.role.eq_ignore_ascii_case("master"),
            NodeRoleFilter::Worker => node.role.eq_ignore_ascii_case("worker"),
            NodeRoleFilter::Any => true,
        })
        .cloned()
        .collect()
}

/// Sorts node data in-place according to the requested key.
pub fn sort_nodes(nodes: &mut [NodeInfo], by: NodeSortBy) {
    match by {
        NodeSortBy::Name => nodes.sort_by_cached_key(|n| n.name.to_ascii_lowercase()),
        NodeSortBy::Status => {
            nodes.sort_by(|a, b| {
                b.ready.cmp(&a.ready).then_with(|| {
                    a.name
                        .to_ascii_lowercase()
                        .cmp(&b.name.to_ascii_lowercase())
                })
            });
        }
        NodeSortBy::Capacity => {
            nodes.sort_by(|a, b| {
                parse_cpu_millicores(&b.cpu_allocatable)
                    .cmp(&parse_cpu_millicores(&a.cpu_allocatable))
                    .then_with(|| {
                        parse_memory_bytes(&b.memory_allocatable)
                            .cmp(&parse_memory_bytes(&a.memory_allocatable))
                    })
                    .then_with(|| {
                        a.name
                            .to_ascii_lowercase()
                            .cmp(&b.name.to_ascii_lowercase())
                    })
            });
        }
    }
}

/// Filters services by free-text query and optional namespace/type constraints.
pub fn filter_services(
    items: &[ServiceInfo],
    query: &str,
    ns: Option<&str>,
    type_: Option<&str>,
) -> Vec<ServiceInfo> {
    let query = query.trim().to_ascii_lowercase();
    let ns = ns.map(str::trim).filter(|s| !s.is_empty());
    let type_ = type_
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase);

    items
        .iter()
        .filter(|item| {
            let name_matches = query.is_empty() || item.name.to_ascii_lowercase().contains(&query);
            let ns_matches = ns.is_none_or(|target_ns| item.namespace == target_ns);
            let type_matches = type_
                .as_deref()
                .is_none_or(|target_type| item.type_.to_ascii_lowercase() == target_type);

            name_matches && ns_matches && type_matches
        })
        .cloned()
        .collect()
}

/// Filters deployments by free-text query and optional namespace/health constraints.
pub fn filter_deployments(
    items: &[DeploymentInfo],
    query: &str,
    ns: Option<&str>,
    health: Option<DeploymentHealth>,
) -> Vec<DeploymentInfo> {
    let query = query.trim().to_ascii_lowercase();
    let ns = ns.map(str::trim).filter(|s| !s.is_empty());

    items
        .iter()
        .filter(|item| {
            let name_matches = query.is_empty() || item.name.to_ascii_lowercase().contains(&query);
            let ns_matches = ns.is_none_or(|target_ns| item.namespace == target_ns);
            let health_matches =
                health.is_none_or(|expected| deployment_health_from_ready(&item.ready) == expected);

            name_matches && ns_matches && health_matches
        })
        .cloned()
        .collect()
}

/// Computes deployment health from a `current/desired` ready string.
pub fn deployment_health_from_ready(ready: &str) -> DeploymentHealth {
    let (ready_count, desired_count) = parse_ready(ready).unwrap_or((0, 0));

    if ready_count == 0 {
        DeploymentHealth::Failed
    } else if ready_count >= desired_count {
        DeploymentHealth::Healthy
    } else {
        DeploymentHealth::Degraded
    }
}

fn parse_ready(ready: &str) -> Option<(i32, i32)> {
    let mut parts = ready.trim().split('/');
    let ready = parts.next()?.trim().parse::<i32>().ok()?;
    let desired = parts.next()?.trim().parse::<i32>().ok()?;

    if parts.next().is_some() {
        return None;
    }

    Some((ready.max(0), desired.max(0)))
}

fn parse_cpu_millicores(cpu: &Option<String>) -> i64 {
    let Some(raw) = cpu.as_deref() else {
        return 0;
    };

    if let Some(value) = raw.strip_suffix('m') {
        value.parse::<i64>().unwrap_or(0)
    } else {
        raw.parse::<f64>()
            .map(|cores| (cores * 1000.0) as i64)
            .unwrap_or(0)
    }
}

fn parse_memory_bytes(mem: &Option<String>) -> i128 {
    let Some(raw) = mem.as_deref() else {
        return 0;
    };

    let units = [
        ("Ki", 1024_i128),
        ("Mi", 1024_i128.pow(2)),
        ("Gi", 1024_i128.pow(3)),
        ("Ti", 1024_i128.pow(4)),
        ("K", 1000_i128),
        ("M", 1000_i128.pow(2)),
        ("G", 1000_i128.pow(3)),
        ("T", 1000_i128.pow(4)),
    ];

    for (suffix, factor) in units {
        if let Some(value) = raw.strip_suffix(suffix) {
            return value
                .parse::<f64>()
                .map(|v| (v * factor as f64) as i128)
                .unwrap_or(0);
        }
    }

    raw.parse::<i128>().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, ready: bool, role: &str) -> NodeInfo {
        NodeInfo {
            name: name.to_string(),
            ready,
            role: role.to_string(),
            ..NodeInfo::default()
        }
    }

    fn service(name: &str, namespace: &str, type_: &str, ports: &[&str]) -> ServiceInfo {
        ServiceInfo {
            name: name.to_string(),
            namespace: namespace.to_string(),
            type_: type_.to_string(),
            service_type: type_.to_string(),
            ports: ports.iter().map(|p| p.to_string()).collect(),
            ..ServiceInfo::default()
        }
    }

    fn deployment(name: &str, namespace: &str, ready: &str) -> DeploymentInfo {
        DeploymentInfo {
            name: name.to_string(),
            namespace: namespace.to_string(),
            ready: ready.to_string(),
            ..DeploymentInfo::default()
        }
    }

    /// Verifies node filtering returns empty output for empty input.
    #[test]
    fn filter_nodes_empty_input() {
        let result = filter_nodes(&[], "worker", None, None);
        assert!(result.is_empty());
    }

    /// Verifies single-node positive and negative name matching.
    #[test]
    fn filter_nodes_single_match_and_no_match() {
        let nodes = vec![node("node-a", true, "worker")];
        assert_eq!(filter_nodes(&nodes, "node", None, None).len(), 1);
        assert!(filter_nodes(&nodes, "zzz", None, None).is_empty());
    }

    /// Verifies case-insensitive substring matching across multiple nodes.
    #[test]
    fn filter_nodes_case_insensitive_substring() {
        let nodes = vec![
            node("Alpha-NODE", true, "worker"),
            node("beta", true, "worker"),
        ];
        let result = filter_nodes(&nodes, "node", None, None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "Alpha-NODE");
    }

    /// Verifies Ready and NotReady status filtering.
    #[test]
    fn filter_nodes_status_filtering() {
        let nodes = vec![node("n1", true, "worker"), node("n2", false, "worker")];
        assert_eq!(
            filter_nodes(&nodes, "", Some(NodeStatusFilter::Ready), None).len(),
            1
        );
        assert_eq!(
            filter_nodes(&nodes, "", Some(NodeStatusFilter::NotReady), None).len(),
            1
        );
    }

    /// Verifies role-based filtering for master, worker, and any role.
    #[test]
    fn filter_nodes_role_filtering() {
        let nodes = vec![node("cp", true, "master"), node("wk", true, "worker")];
        assert_eq!(
            filter_nodes(&nodes, "", None, Some(NodeRoleFilter::Master)).len(),
            1
        );
        assert_eq!(
            filter_nodes(&nodes, "", None, Some(NodeRoleFilter::Worker)).len(),
            1
        );
        assert_eq!(
            filter_nodes(&nodes, "", None, Some(NodeRoleFilter::Any)).len(),
            2
        );
    }

    /// Verifies combined node filters use AND semantics.
    #[test]
    fn filter_nodes_combined_filters_and_logic() {
        let nodes = vec![
            node("master-ready", true, "master"),
            node("master-down", false, "master"),
            node("worker-ready", true, "worker"),
        ];

        let result = filter_nodes(
            &nodes,
            "master",
            Some(NodeStatusFilter::Ready),
            Some(NodeRoleFilter::Master),
        );

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "master-ready");
    }

    /// Verifies filtering handles special characters literally in names.
    #[test]
    fn filter_nodes_special_characters() {
        let nodes = vec![node("node.*[]+?", true, "worker")];
        assert_eq!(filter_nodes(&nodes, ".*[]+?", None, None).len(), 1);
    }

    /// Verifies filtering supports unicode names and queries.
    #[test]
    fn filter_nodes_unicode_query() {
        let nodes = vec![
            node("café", true, "worker"),
            node("日本語-🚀", true, "worker"),
        ];
        assert_eq!(filter_nodes(&nodes, "café", None, None).len(), 1);
        assert_eq!(filter_nodes(&nodes, "日本語", None, None).len(), 1);
        assert_eq!(filter_nodes(&nodes, "🚀", None, None).len(), 1);
    }

    /// Verifies empty and whitespace-only queries behave as match-all.
    #[test]
    fn filter_nodes_empty_and_spaces_match_all() {
        let nodes = vec![node("n1", true, "worker"), node("n2", false, "master")];
        assert_eq!(filter_nodes(&nodes, "", None, None).len(), 2);
        assert_eq!(filter_nodes(&nodes, "    ", None, None).len(), 2);
    }

    /// Verifies very long query strings do not panic and return deterministic results.
    #[test]
    fn filter_nodes_very_long_query() {
        let nodes = vec![node("n1", true, "worker")];
        let long_query = "x".repeat(1500);
        assert!(filter_nodes(&nodes, &long_query, None, None).is_empty());
    }

    /// Verifies service filtering handles empty input.
    #[test]
    fn filter_services_empty_input() {
        let result = filter_services(&[], "api", None, None);
        assert!(result.is_empty());
    }

    /// Verifies service name substring matching is case-insensitive.
    #[test]
    fn filter_services_name_matching() {
        let items = vec![service("Api-Gateway", "default", "ClusterIP", &["80/TCP"])];
        let result = filter_services(&items, "gateway", None, None);
        assert_eq!(result.len(), 1);
    }

    /// Verifies service namespace and type filters.
    #[test]
    fn filter_services_namespace_and_type() {
        let items = vec![
            service("s1", "default", "ClusterIP", &["80/TCP"]),
            service("s2", "kube-system", "NodePort", &["443/TCP"]),
        ];

        assert_eq!(
            filter_services(&items, "", Some("kube-system"), Some("NodePort")).len(),
            1
        );
        assert!(filter_services(&items, "", Some("kube-system"), Some("LoadBalancer")).is_empty());
    }

    /// Verifies combined service filters use AND semantics.
    #[test]
    fn filter_services_combined_filters() {
        let items = vec![
            service("front", "prod", "LoadBalancer", &["80/TCP"]),
            service("front", "dev", "LoadBalancer", &["80/TCP"]),
        ];

        let result = filter_services(&items, "front", Some("prod"), Some("LoadBalancer"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].namespace, "prod");
    }

    /// Verifies deployment filtering handles empty input.
    #[test]
    fn filter_deployments_empty_input() {
        let result = filter_deployments(&[], "api", None, None);
        assert!(result.is_empty());
    }

    /// Verifies deployment ready parsing and health classification.
    #[test]
    fn deployment_health_classification() {
        assert_eq!(
            deployment_health_from_ready("3/3"),
            DeploymentHealth::Healthy
        );
        assert_eq!(
            deployment_health_from_ready("1/3"),
            DeploymentHealth::Degraded
        );
        assert_eq!(
            deployment_health_from_ready("0/3"),
            DeploymentHealth::Failed
        );
    }

    /// Verifies deployment health filtering across mixed states.
    #[test]
    fn filter_deployments_by_health() {
        let items = vec![
            deployment("ok", "default", "2/2"),
            deployment("warn", "default", "1/2"),
            deployment("bad", "default", "0/2"),
        ];

        assert_eq!(
            filter_deployments(&items, "", None, Some(DeploymentHealth::Healthy)).len(),
            1
        );
        assert_eq!(
            filter_deployments(&items, "", None, Some(DeploymentHealth::Degraded)).len(),
            1
        );
        assert_eq!(
            filter_deployments(&items, "", None, Some(DeploymentHealth::Failed)).len(),
            1
        );
    }

    /// Verifies deployment combined filters use AND semantics.
    #[test]
    fn filter_deployments_combined_filters() {
        let items = vec![
            deployment("api", "prod", "2/3"),
            deployment("api", "dev", "2/2"),
        ];

        let result = filter_deployments(
            &items,
            "api",
            Some("prod"),
            Some(DeploymentHealth::Degraded),
        );

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].namespace, "prod");
    }

    /// Verifies node sorting by capacity handles unit parsing.
    #[test]
    fn sort_nodes_by_capacity_descending() {
        let mut nodes = vec![
            NodeInfo {
                name: "small".to_string(),
                cpu_allocatable: Some("500m".to_string()),
                memory_allocatable: Some("512Mi".to_string()),
                ..NodeInfo::default()
            },
            NodeInfo {
                name: "large".to_string(),
                cpu_allocatable: Some("2".to_string()),
                memory_allocatable: Some("4Gi".to_string()),
                ..NodeInfo::default()
            },
        ];

        sort_nodes(&mut nodes, NodeSortBy::Capacity);
        assert_eq!(nodes[0].name, "large");
        assert_eq!(nodes[1].name, "small");
    }
}
