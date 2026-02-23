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
