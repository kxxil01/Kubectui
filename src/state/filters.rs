//! Pure filtering helpers for list-oriented views.

use crate::k8s::dtos::{
    CronJobInfo, DaemonSetInfo, DeploymentInfo, JobInfo, LimitRangeInfo, NodeInfo,
    PodDisruptionBudgetInfo, ReplicaSetInfo, ReplicationControllerInfo, ResourceQuotaInfo,
    ServiceInfo, StatefulSetInfo,
};
use std::cmp::Ordering;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatusFilter {
    Ready,
    NotReady,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRoleFilter {
    Master,
    Worker,
    Any,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSortBy {
    Name,
    Status,
    Capacity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentHealth {
    Healthy,
    Degraded,
    Failed,
}

#[inline]
fn contains_ci(haystack: &str, needle: &str) -> bool {
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
fn cmp_ci(left: &str, right: &str) -> Ordering {
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
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (None, None) => return Ordering::Equal,
        }
    }
}

#[must_use]
pub fn filter_nodes(
    nodes: &[NodeInfo],
    query: &str,
    status: Option<NodeStatusFilter>,
    role: Option<NodeRoleFilter>,
) -> Vec<NodeInfo> {
    let query = query.trim();

    nodes
        .iter()
        .filter(|node| query.is_empty() || contains_ci(&node.name, query))
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

pub fn sort_nodes(nodes: &mut [NodeInfo], by: NodeSortBy) {
    match by {
        NodeSortBy::Name => nodes.sort_by(|a, b| cmp_ci(&a.name, &b.name)),
        NodeSortBy::Status => {
            nodes.sort_by(|a, b| b.ready.cmp(&a.ready).then_with(|| cmp_ci(&a.name, &b.name)));
        }
        NodeSortBy::Capacity => {
            nodes.sort_by(|a, b| {
                parse_cpu_millicores(&b.cpu_allocatable)
                    .cmp(&parse_cpu_millicores(&a.cpu_allocatable))
                    .then_with(|| {
                        parse_memory_bytes(&b.memory_allocatable)
                            .cmp(&parse_memory_bytes(&a.memory_allocatable))
                    })
            });
        }
    }
}

pub fn filter_services(
    items: &[ServiceInfo],
    query: &str,
    ns: Option<&str>,
    type_: Option<&str>,
) -> Vec<ServiceInfo> {
    let query = query.trim();
    let ns = ns.map(str::trim).filter(|s| !s.is_empty());
    let type_ = type_.map(str::trim).filter(|s| !s.is_empty());

    items
        .iter()
        .filter(|item| {
            let name_matches = query.is_empty() || contains_ci(&item.name, query);
            let ns_matches = ns.is_none_or(|target_ns| item.namespace == target_ns);
            let type_matches =
                type_.is_none_or(|target_type| item.type_.eq_ignore_ascii_case(target_type));
            name_matches && ns_matches && type_matches
        })
        .cloned()
        .collect()
}

pub fn filter_deployments(
    items: &[DeploymentInfo],
    query: &str,
    ns: Option<&str>,
    health: Option<DeploymentHealth>,
) -> Vec<DeploymentInfo> {
    let query = query.trim();
    let ns = ns.map(str::trim).filter(|s| !s.is_empty());

    items
        .iter()
        .filter(|item| {
            let name_matches = query.is_empty() || contains_ci(&item.name, query);
            let ns_matches = ns.is_none_or(|target_ns| item.namespace == target_ns);
            let health_matches =
                health.is_none_or(|expected| deployment_health_from_ready(&item.ready) == expected);
            name_matches && ns_matches && health_matches
        })
        .cloned()
        .collect()
}

pub fn filter_jobs(items: &[JobInfo], query: &str, ns: Option<&str>) -> Vec<JobInfo> {
    let query = query.trim();
    let ns = ns.map(str::trim).filter(|s| !s.is_empty());

    items
        .iter()
        .filter(|job| {
            let ns_match = ns.is_none_or(|target| job.namespace == target);
            let query_match = query.is_empty()
                || contains_ci(&job.name, query)
                || contains_ci(&job.status, query);
            ns_match && query_match
        })
        .cloned()
        .collect()
}

pub fn filter_cronjobs(items: &[CronJobInfo], query: &str, ns: Option<&str>) -> Vec<CronJobInfo> {
    let query = query.trim();
    let ns = ns.map(str::trim).filter(|s| !s.is_empty());

    items
        .iter()
        .filter(|cj| {
            let ns_match = ns.is_none_or(|target| cj.namespace == target);
            let query_match = query.is_empty()
                || contains_ci(&cj.name, query)
                || contains_ci(&cj.schedule, query);
            ns_match && query_match
        })
        .cloned()
        .collect()
}

pub fn filter_statefulsets(
    items: &[StatefulSetInfo],
    query: &str,
    ns: Option<&str>,
) -> Vec<StatefulSetInfo> {
    let query = query.trim();
    let ns = ns.map(str::trim).filter(|s| !s.is_empty());

    items
        .iter()
        .filter(|ss| {
            let ns_match = ns.is_none_or(|target| ss.namespace == target);
            let query_match = query.is_empty()
                || contains_ci(&ss.name, query)
                || contains_ci(ss.image.as_deref().unwrap_or_default(), query);
            ns_match && query_match
        })
        .cloned()
        .collect()
}

pub fn filter_daemonsets(
    items: &[DaemonSetInfo],
    query: &str,
    ns: Option<&str>,
) -> Vec<DaemonSetInfo> {
    let query = query.trim();
    let ns = ns.map(str::trim).filter(|s| !s.is_empty());

    items
        .iter()
        .filter(|ds| {
            let ns_match = ns.is_none_or(|target| ds.namespace == target);
            let query_match = query.is_empty()
                || contains_ci(&ds.name, query)
                || contains_ci(&ds.selector, query)
                || contains_ci(ds.image.as_deref().unwrap_or_default(), query)
                || ds
                    .labels
                    .iter()
                    .any(|(k, v)| contains_ci(k, query) || contains_ci(v, query));
            ns_match && query_match
        })
        .cloned()
        .collect()
}

pub fn filter_resource_quotas(
    items: &[ResourceQuotaInfo],
    query: &str,
    ns: Option<&str>,
) -> Vec<ResourceQuotaInfo> {
    let query = query.trim();
    let ns = ns.map(str::trim).filter(|s| !s.is_empty());

    items
        .iter()
        .filter(|rq| {
            let ns_match = ns.is_none_or(|target| rq.namespace == target);
            let query_match = query.is_empty()
                || contains_ci(&rq.name, query)
                || rq.hard.keys().any(|k| contains_ci(k, query));
            ns_match && query_match
        })
        .cloned()
        .collect()
}

pub fn filter_limit_ranges(
    items: &[LimitRangeInfo],
    query: &str,
    ns: Option<&str>,
) -> Vec<LimitRangeInfo> {
    let query = query.trim();
    let ns = ns.map(str::trim).filter(|s| !s.is_empty());

    items
        .iter()
        .filter(|lr| {
            let ns_match = ns.is_none_or(|target| lr.namespace == target);
            let query_match = query.is_empty()
                || contains_ci(&lr.name, query)
                || lr.limits.iter().any(|spec| contains_ci(&spec.type_, query));
            ns_match && query_match
        })
        .cloned()
        .collect()
}

pub fn filter_pod_disruption_budgets(
    items: &[PodDisruptionBudgetInfo],
    query: &str,
    ns: Option<&str>,
) -> Vec<PodDisruptionBudgetInfo> {
    let query = query.trim();
    let ns = ns.map(str::trim).filter(|s| !s.is_empty());

    items
        .iter()
        .filter(|pdb| {
            let ns_match = ns.is_none_or(|target| pdb.namespace == target);
            let query_match = query.is_empty()
                || contains_ci(&pdb.name, query)
                || contains_ci(pdb.min_available.as_deref().unwrap_or_default(), query)
                || contains_ci(pdb.max_unavailable.as_deref().unwrap_or_default(), query);
            ns_match && query_match
        })
        .cloned()
        .collect()
}

pub fn deployment_health_from_ready(ready: &str) -> DeploymentHealth {
    let (ready_count, desired_count) = parse_ready(ready).unwrap_or((0, 0));

    if desired_count == 0 && ready_count == 0 {
        DeploymentHealth::Healthy
    } else if ready_count == 0 {
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

#[must_use]
pub fn filter_replicasets(
    items: &[ReplicaSetInfo],
    query: &str,
    namespace: Option<&str>,
) -> Vec<ReplicaSetInfo> {
    let q = query.trim();
    items
        .iter()
        .filter(|rs| namespace.is_none_or(|ns| rs.namespace == ns))
        .filter(|rs| q.is_empty() || contains_ci(&rs.name, q) || contains_ci(&rs.namespace, q))
        .cloned()
        .collect()
}

#[must_use]
pub fn filter_replication_controllers(
    items: &[ReplicationControllerInfo],
    query: &str,
    namespace: Option<&str>,
) -> Vec<ReplicationControllerInfo> {
    let q = query.trim();
    items
        .iter()
        .filter(|rc| namespace.is_none_or(|ns| rc.namespace == ns))
        .filter(|rc| q.is_empty() || contains_ci(&rc.name, q) || contains_ci(&rc.namespace, q))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_statefulsets_by_name() {
        let items = vec![
            StatefulSetInfo {
                name: "postgres-primary".into(),
                namespace: "db".into(),
                image: Some("postgres:16".into()),
                ..StatefulSetInfo::default()
            },
            StatefulSetInfo {
                name: "redis-cache".into(),
                namespace: "cache".into(),
                image: Some("redis:7".into()),
                ..StatefulSetInfo::default()
            },
        ];
        let filtered = filter_statefulsets(&items, "POSTGRES", None);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_filter_statefulsets_by_namespace() {
        let items = vec![
            StatefulSetInfo {
                name: "postgres".into(),
                namespace: "db".into(),
                ..StatefulSetInfo::default()
            },
            StatefulSetInfo {
                name: "postgres".into(),
                namespace: "default".into(),
                ..StatefulSetInfo::default()
            },
        ];
        let filtered = filter_statefulsets(&items, "", Some("db"));
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_filter_statefulsets_by_image() {
        let items = vec![
            StatefulSetInfo {
                name: "pg".into(),
                namespace: "db".into(),
                image: Some("ghcr.io/company/postgres:16".into()),
                ..StatefulSetInfo::default()
            },
            StatefulSetInfo {
                name: "mongo".into(),
                namespace: "db".into(),
                image: Some("mongo:7".into()),
                ..StatefulSetInfo::default()
            },
        ];
        let filtered = filter_statefulsets(&items, "company/postgres", None);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_filter_daemonsets_by_name() {
        let items = vec![
            DaemonSetInfo {
                name: "node-exporter".into(),
                namespace: "monitoring".into(),
                desired_count: 10,
                ready_count: 10,
                ..DaemonSetInfo::default()
            },
            DaemonSetInfo {
                name: "fluent-bit".into(),
                namespace: "logging".into(),
                desired_count: 10,
                ready_count: 8,
                unavailable_count: 2,
                ..DaemonSetInfo::default()
            },
        ];
        let filtered = filter_daemonsets(&items, "EXPORTER", None);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_filter_daemonsets_ready_status() {
        let items = vec![
            DaemonSetInfo {
                name: "healthy".into(),
                namespace: "default".into(),
                desired_count: 8,
                ready_count: 8,
                ..DaemonSetInfo::default()
            },
            DaemonSetInfo {
                name: "degraded".into(),
                namespace: "default".into(),
                desired_count: 8,
                ready_count: 4,
                unavailable_count: 4,
                ..DaemonSetInfo::default()
            },
        ];
        let filtered = filter_daemonsets(&items, "degraded", None);
        assert_eq!(filtered[0].ready_count, 4);
    }

    #[test]
    fn test_filter_resource_quotas_by_name() {
        let items = vec![
            ResourceQuotaInfo {
                name: "team-a".into(),
                namespace: "default".into(),
                ..ResourceQuotaInfo::default()
            },
            ResourceQuotaInfo {
                name: "team-b".into(),
                namespace: "prod".into(),
                ..ResourceQuotaInfo::default()
            },
        ];
        let filtered = filter_resource_quotas(&items, "team-a", None);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_filter_limit_ranges_by_type() {
        let mut lr = LimitRangeInfo {
            name: "limits".into(),
            namespace: "default".into(),
            ..LimitRangeInfo::default()
        };
        lr.limits.push(crate::k8s::dtos::LimitSpec {
            type_: "Container".into(),
            ..crate::k8s::dtos::LimitSpec::default()
        });
        let filtered = filter_limit_ranges(&[lr], "container", None);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_filter_pdbs_by_namespace() {
        let items = vec![
            PodDisruptionBudgetInfo {
                name: "web".into(),
                namespace: "default".into(),
                ..PodDisruptionBudgetInfo::default()
            },
            PodDisruptionBudgetInfo {
                name: "api".into(),
                namespace: "prod".into(),
                ..PodDisruptionBudgetInfo::default()
            },
        ];
        let filtered = filter_pod_disruption_budgets(&items, "", Some("prod"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "api");
    }

    #[test]
    fn test_filter_daemonsets_unavailable_count() {
        let items = vec![
            DaemonSetInfo {
                name: "all-good".into(),
                namespace: "default".into(),
                desired_count: 6,
                ready_count: 6,
                ..DaemonSetInfo::default()
            },
            DaemonSetInfo {
                name: "has-unavailable".into(),
                namespace: "default".into(),
                desired_count: 6,
                ready_count: 5,
                unavailable_count: 1,
                ..DaemonSetInfo::default()
            },
        ];
        let filtered = filter_daemonsets(&items, "has-unavailable", None);
        assert_eq!(filtered[0].unavailable_count, 1);
    }
}
