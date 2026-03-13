//! Pure dashboard statistics and alert aggregation logic.

use std::collections::{BTreeSet, HashMap};

use chrono::{Duration, Utc};

use crate::{
    k8s::dtos::{AlertItem, AlertSeverity},
    state::ClusterSnapshot,
};

/// Aggregated values displayed in the dashboard metrics panel.
#[derive(Debug, Clone, Copy, Default)]
pub struct DashboardStats {
    pub total_nodes: usize,
    pub ready_nodes: usize,
    pub total_pods: usize,
    pub running_pods: usize,
    pub failed_pods: usize,
    pub services_count: usize,
    pub namespaces_count: usize,
    pub ready_nodes_percent: u8,
    pub running_pods_percent: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DashboardHealthState {
    #[default]
    Healthy,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeUtilizationSummary {
    pub name: String,
    pub cpu_pct: u8,
    pub mem_pct: u8,
    pub cpu_used_m: u64,
    pub cpu_alloc_m: u64,
    pub mem_used_mib: u64,
    pub mem_alloc_mib: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardInsights {
    pub health_state: DashboardHealthState,
    pub not_ready_nodes: usize,
    pub pressure_nodes: usize,
    pub pending_pods: usize,
    pub failed_pods: usize,
    pub metrics_reported_nodes: usize,
    pub utilization_nodes: usize,
    pub avg_cpu_pct: u8,
    pub avg_mem_pct: u8,
    pub high_cpu_nodes: usize,
    pub high_mem_nodes: usize,
    pub hot_cpu_nodes: Vec<NodeUtilizationSummary>,
    pub hot_mem_nodes: Vec<NodeUtilizationSummary>,
}

/// Computes dashboard metrics from a snapshot without side effects.
pub fn compute_dashboard_stats(snapshot: &ClusterSnapshot) -> DashboardStats {
    let total_nodes = snapshot.nodes.len();
    let ready_nodes = snapshot.nodes.iter().filter(|node| node.ready).count();

    let total_pods = snapshot.pods.len();
    let running_pods = snapshot
        .pods
        .iter()
        .filter(|pod| pod.status.eq_ignore_ascii_case("running"))
        .count();
    let failed_pods = snapshot
        .pods
        .iter()
        .filter(|pod| pod.status.eq_ignore_ascii_case("failed"))
        .count();

    let namespaces_count = snapshot
        .pods
        .iter()
        .map(|pod| pod.namespace.as_str())
        .chain(
            snapshot
                .services
                .iter()
                .map(|service| service.namespace.as_str()),
        )
        .chain(
            snapshot
                .deployments
                .iter()
                .map(|deployment| deployment.namespace.as_str()),
        )
        .collect::<BTreeSet<_>>()
        .len();

    DashboardStats {
        total_nodes,
        ready_nodes,
        total_pods,
        running_pods,
        failed_pods,
        services_count: snapshot.services.len(),
        namespaces_count,
        ready_nodes_percent: ratio_percent(ready_nodes, total_nodes),
        running_pods_percent: ratio_percent(running_pods, total_pods),
    }
}

/// Computes workload readiness percentage across Deployments, StatefulSets, and DaemonSets.
pub fn compute_workload_ready_percent(snapshot: &ClusterSnapshot) -> u8 {
    let (dep_ready, dep_total) = snapshot.deployments.iter().fold((0i64, 0i64), |(r, t), d| {
        (
            r + i64::from(d.ready_replicas.max(0).min(d.desired_replicas.max(0))),
            t + i64::from(d.desired_replicas.max(0)),
        )
    });
    let (ss_ready, ss_total) = snapshot
        .statefulsets
        .iter()
        .fold((0i64, 0i64), |(r, t), s| {
            (
                r + i64::from(s.ready_replicas.max(0).min(s.desired_replicas.max(0))),
                t + i64::from(s.desired_replicas.max(0)),
            )
        });
    let (ds_ready, ds_total) = snapshot.daemonsets.iter().fold((0i64, 0i64), |(r, t), d| {
        (
            r + i64::from(d.ready_count.max(0).min(d.desired_count.max(0))),
            t + i64::from(d.desired_count.max(0)),
        )
    });

    let total = dep_total + ss_total + ds_total;
    if total == 0 {
        100
    } else {
        (((dep_ready + ss_ready + ds_ready) * 100 / total).clamp(0, 100)) as u8
    }
}

/// Computes dashboard health/risk insights from node metrics and workload state.
pub fn compute_dashboard_insights(snapshot: &ClusterSnapshot) -> DashboardInsights {
    const HOT_NODE_LIMIT: usize = 3;
    const SATURATION_WARNING_PCT: u8 = 80;

    let not_ready_nodes = snapshot.nodes.iter().filter(|node| !node.ready).count();
    let pressure_nodes = snapshot
        .nodes
        .iter()
        .filter(|node| {
            node.memory_pressure
                || node.disk_pressure
                || node.pid_pressure
                || node.network_unavailable
        })
        .count();
    let (pending_pods, failed_pods) =
        snapshot
            .pods
            .iter()
            .fold((0usize, 0usize), |(pending, failed), pod| {
                if pod.status.eq_ignore_ascii_case("pending") {
                    (pending + 1, failed)
                } else if pod.status.eq_ignore_ascii_case("failed") {
                    (pending, failed + 1)
                } else {
                    (pending, failed)
                }
            });

    let metrics_by_node: HashMap<&str, &crate::k8s::dtos::NodeMetricsInfo> = snapshot
        .node_metrics
        .iter()
        .map(|metric| (metric.name.as_str(), metric))
        .collect();

    let mut utilization = Vec::new();
    for node in &snapshot.nodes {
        let Some(metric) = metrics_by_node.get(node.name.as_str()) else {
            continue;
        };

        let cpu_alloc_m = node
            .cpu_allocatable
            .as_deref()
            .map(parse_millicores)
            .unwrap_or(0);
        let mem_alloc_mib = node
            .memory_allocatable
            .as_deref()
            .map(parse_mib)
            .unwrap_or(0);
        if cpu_alloc_m == 0 || mem_alloc_mib == 0 {
            continue;
        }

        let cpu_used_m = parse_millicores(&metric.cpu);
        let mem_used_mib = parse_mib(&metric.memory);
        utilization.push(NodeUtilizationSummary {
            name: node.name.clone(),
            cpu_pct: ratio_percent_u64(cpu_used_m, cpu_alloc_m),
            mem_pct: ratio_percent_u64(mem_used_mib, mem_alloc_mib),
            cpu_used_m,
            cpu_alloc_m,
            mem_used_mib,
            mem_alloc_mib,
        });
    }

    let utilization_nodes = utilization.len();
    let avg_cpu_pct = if utilization.is_empty() {
        0
    } else {
        (utilization
            .iter()
            .map(|item| u64::from(item.cpu_pct))
            .sum::<u64>()
            / utilization.len() as u64)
            .min(100) as u8
    };
    let avg_mem_pct = if utilization.is_empty() {
        0
    } else {
        (utilization
            .iter()
            .map(|item| u64::from(item.mem_pct))
            .sum::<u64>()
            / utilization.len() as u64)
            .min(100) as u8
    };

    let high_cpu_nodes = utilization
        .iter()
        .filter(|item| item.cpu_pct >= SATURATION_WARNING_PCT)
        .count();
    let high_mem_nodes = utilization
        .iter()
        .filter(|item| item.mem_pct >= SATURATION_WARNING_PCT)
        .count();

    let mut hot_cpu_nodes = utilization.clone();
    hot_cpu_nodes.sort_unstable_by(|a, b| {
        b.cpu_pct
            .cmp(&a.cpu_pct)
            .then_with(|| b.cpu_used_m.cmp(&a.cpu_used_m))
    });
    hot_cpu_nodes.truncate(HOT_NODE_LIMIT);

    let mut hot_mem_nodes = utilization;
    hot_mem_nodes.sort_unstable_by(|a, b| {
        b.mem_pct
            .cmp(&a.mem_pct)
            .then_with(|| b.mem_used_mib.cmp(&a.mem_used_mib))
    });
    hot_mem_nodes.truncate(HOT_NODE_LIMIT);

    let health_state = if not_ready_nodes > 0 || pressure_nodes > 0 || failed_pods > 0 {
        DashboardHealthState::Critical
    } else if pending_pods > 0
        || high_cpu_nodes > 0
        || high_mem_nodes > 0
        || avg_cpu_pct >= 70
        || avg_mem_pct >= 75
    {
        DashboardHealthState::Warning
    } else {
        DashboardHealthState::Healthy
    };

    DashboardInsights {
        health_state,
        not_ready_nodes,
        pressure_nodes,
        pending_pods,
        failed_pods,
        metrics_reported_nodes: snapshot.node_metrics.len(),
        utilization_nodes,
        avg_cpu_pct,
        avg_mem_pct,
        high_cpu_nodes,
        high_mem_nodes,
        hot_cpu_nodes,
        hot_mem_nodes,
    }
}

/// Computes top dashboard alerts from nodes and pods.
///
/// Returned alerts are ordered by severity and importance and capped to top 5.
pub fn compute_alerts(snapshot: &ClusterSnapshot) -> Vec<AlertItem> {
    let memory_pressure_nodes = snapshot
        .nodes
        .iter()
        .filter(|node| node.memory_pressure)
        .count();

    let crash_loop_backoff = snapshot
        .pods
        .iter()
        .filter(|pod| has_reason(&pod.waiting_reasons, "CrashLoopBackOff"))
        .count();

    let image_pull_backoff = snapshot
        .pods
        .iter()
        .filter(|pod| has_reason(&pod.waiting_reasons, "ImagePullBackOff"))
        .count();

    let failed_pods = snapshot
        .pods
        .iter()
        .filter(|pod| pod.status.eq_ignore_ascii_case("failed"))
        .count();

    let pending_over_5m = snapshot
        .pods
        .iter()
        .filter(|pod| pod.status.eq_ignore_ascii_case("pending"))
        .filter(|pod| {
            pod.created_at
                .map(|created_at| Utc::now() - created_at > Duration::minutes(5))
                .unwrap_or(false)
        })
        .count();

    let mut alerts = vec![
        AlertItem {
            severity: severity_from_count(memory_pressure_nodes, AlertSeverity::Warning),
            title: "MemoryPressure".to_string(),
            message: if memory_pressure_nodes > 0 {
                format!("{memory_pressure_nodes} node(s) report MemoryPressure")
            } else {
                "No nodes report MemoryPressure".to_string()
            },
        },
        AlertItem {
            severity: severity_from_count(crash_loop_backoff, AlertSeverity::Error),
            title: "CrashLoopBackOff".to_string(),
            message: if crash_loop_backoff > 0 {
                format!("{crash_loop_backoff} pod(s) are in CrashLoopBackOff")
            } else {
                "No pods in CrashLoopBackOff".to_string()
            },
        },
        AlertItem {
            severity: severity_from_count(failed_pods, AlertSeverity::Error),
            title: "Failed pods".to_string(),
            message: if failed_pods > 0 {
                format!("{failed_pods} pod(s) are in Failed phase")
            } else {
                "No failed pods detected".to_string()
            },
        },
        AlertItem {
            severity: severity_from_count(image_pull_backoff, AlertSeverity::Error),
            title: "ImagePullBackOff".to_string(),
            message: if image_pull_backoff > 0 {
                format!("{image_pull_backoff} pod(s) are in ImagePullBackOff")
            } else {
                "No image pull backoff detected".to_string()
            },
        },
        AlertItem {
            severity: severity_from_count(pending_over_5m, AlertSeverity::Warning),
            title: "Pending > 5m".to_string(),
            message: if pending_over_5m > 0 {
                format!("{pending_over_5m} pod(s) have been pending for over 5 minutes")
            } else {
                "No long-pending pods".to_string()
            },
        },
    ];

    alerts.sort_unstable_by_key(|item| severity_rank(item.severity));
    alerts.truncate(5);
    alerts
}

fn ratio_percent(numerator: usize, denominator: usize) -> u8 {
    if denominator == 0 {
        return 0;
    }

    let ratio = ((numerator as f64 / denominator as f64) * 100.0).round();
    ratio.clamp(0.0, 100.0) as u8
}

fn ratio_percent_u64(numerator: u64, denominator: u64) -> u8 {
    if denominator == 0 {
        return 0;
    }
    ((numerator.saturating_mul(100) / denominator).min(100)) as u8
}

pub(crate) fn parse_millicores(raw: &str) -> u64 {
    if let Some(m) = raw.strip_suffix('m') {
        m.parse().unwrap_or(0)
    } else if let Some(n) = raw.strip_suffix('n') {
        n.parse::<u64>().unwrap_or(0) / 1_000_000
    } else if raw.contains('.') {
        // Decimal cores (e.g. "0.5" → 500m, "1.25" → 1250m)
        raw.parse::<f64>().map(|v| (v * 1000.0) as u64).unwrap_or(0)
    } else {
        raw.parse::<u64>().unwrap_or(0) * 1000
    }
}

pub(crate) fn parse_mib(raw: &str) -> u64 {
    if let Some(v) = raw.strip_suffix("Ki") {
        return v.parse::<u64>().unwrap_or(0) / 1024;
    }
    if let Some(v) = raw.strip_suffix("Mi") {
        return v.parse().unwrap_or(0);
    }
    if let Some(v) = raw.strip_suffix("Gi") {
        return v.parse::<u64>().unwrap_or(0) * 1024;
    }
    if let Some(v) = raw.strip_suffix("Ti") {
        return v.parse::<u64>().unwrap_or(0) * 1024 * 1024;
    }
    raw.parse::<u64>().unwrap_or(0) / (1024 * 1024)
}

pub(crate) fn format_millicores(m: u64) -> String {
    if m >= 1000 && m.is_multiple_of(1000) {
        (m / 1000).to_string()
    } else if m >= 1000 {
        let whole = m / 1000;
        let frac = m % 1000;
        // Trim trailing zeros: 1500 → "1.5", 1250 → "1.25"
        if frac.is_multiple_of(100) {
            format!("{whole}.{}", frac / 100)
        } else if frac.is_multiple_of(10) {
            format!("{whole}.{:02}", frac / 10)
        } else {
            format!("{whole}.{frac:03}")
        }
    } else {
        format!("{m}m")
    }
}

pub(crate) fn format_mib(mib: u64) -> String {
    if mib >= 1024 && mib.is_multiple_of(1024) {
        format!("{}Gi", mib / 1024)
    } else {
        format!("{mib}Mi")
    }
}

/// Cluster-wide resource utilization, overcommitment, and governance summary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClusterResourceSummary {
    /// Total CPU used across all nodes (millicores, from node_metrics).
    pub total_cpu_used_m: u64,
    /// Total memory used across all nodes (MiB, from node_metrics).
    pub total_mem_used_mib: u64,
    /// Total CPU allocatable across all nodes (millicores).
    pub total_cpu_allocatable_m: u64,
    /// Total memory allocatable across all nodes (MiB).
    pub total_mem_allocatable_mib: u64,
    /// Cluster-wide CPU utilization percentage (used / allocatable).
    pub cluster_cpu_pct: u8,
    /// Cluster-wide memory utilization percentage (used / allocatable).
    pub cluster_mem_pct: u8,
    /// Total pod CPU requests / total allocatable (can exceed 100%).
    pub cpu_request_commitment_pct: u16,
    /// Total pod memory requests / total allocatable (can exceed 100%).
    pub mem_request_commitment_pct: u16,
    /// Total pod CPU limits / total allocatable (can exceed 100%).
    pub cpu_limit_commitment_pct: u16,
    /// Total pod memory limits / total allocatable (can exceed 100%).
    pub mem_limit_commitment_pct: u16,
    /// Running pods missing a CPU request.
    pub pods_missing_cpu_request: usize,
    /// Running pods missing a memory request.
    pub pods_missing_mem_request: usize,
    /// Running pods missing at least one limit (CPU or memory).
    pub pods_missing_any_limit: usize,
    /// Total running pods considered for governance.
    pub total_running_pods: usize,
}

/// Computes cluster-wide resource utilization, overcommitment, and governance.
pub fn compute_cluster_resource_summary(snapshot: &ClusterSnapshot) -> ClusterResourceSummary {
    let mut summary = ClusterResourceSummary::default();

    // Sum allocatable capacity from nodes
    for node in &snapshot.nodes {
        if let Some(ref cpu) = node.cpu_allocatable {
            summary.total_cpu_allocatable_m += parse_millicores(cpu);
        }
        if let Some(ref mem) = node.memory_allocatable {
            summary.total_mem_allocatable_mib += parse_mib(mem);
        }
    }

    // Sum actual usage from node metrics
    for nm in &snapshot.node_metrics {
        summary.total_cpu_used_m += parse_millicores(&nm.cpu);
        summary.total_mem_used_mib += parse_mib(&nm.memory);
    }

    // Utilization percentages
    summary.cluster_cpu_pct =
        ratio_percent_u64(summary.total_cpu_used_m, summary.total_cpu_allocatable_m);
    summary.cluster_mem_pct = ratio_percent_u64(
        summary.total_mem_used_mib,
        summary.total_mem_allocatable_mib,
    );

    // Aggregate pod requests/limits and governance (running pods only)
    let mut total_cpu_req: u64 = 0;
    let mut total_mem_req: u64 = 0;
    let mut total_cpu_lim: u64 = 0;
    let mut total_mem_lim: u64 = 0;

    for pod in &snapshot.pods {
        if !pod.status.eq_ignore_ascii_case("running") {
            continue;
        }
        summary.total_running_pods += 1;

        if let Some(ref req) = pod.cpu_request {
            total_cpu_req += parse_millicores(req);
        } else {
            summary.pods_missing_cpu_request += 1;
        }
        if let Some(ref req) = pod.memory_request {
            total_mem_req += parse_mib(req);
        } else {
            summary.pods_missing_mem_request += 1;
        }
        if let Some(ref lim) = pod.cpu_limit {
            total_cpu_lim += parse_millicores(lim);
        }
        if let Some(ref lim) = pod.memory_limit {
            total_mem_lim += parse_mib(lim);
        }
        if pod.cpu_limit.is_none() || pod.memory_limit.is_none() {
            summary.pods_missing_any_limit += 1;
        }
    }

    // Commitment percentages (can exceed 100%)
    if summary.total_cpu_allocatable_m > 0 {
        summary.cpu_request_commitment_pct =
            (total_cpu_req * 100 / summary.total_cpu_allocatable_m).min(999) as u16;
        summary.cpu_limit_commitment_pct =
            (total_cpu_lim * 100 / summary.total_cpu_allocatable_m).min(999) as u16;
    }
    if summary.total_mem_allocatable_mib > 0 {
        summary.mem_request_commitment_pct =
            (total_mem_req * 100 / summary.total_mem_allocatable_mib).min(999) as u16;
        summary.mem_limit_commitment_pct =
            (total_mem_lim * 100 / summary.total_mem_allocatable_mib).min(999) as u16;
    }

    summary
}

/// A single pod's resource usage for "top consumers" display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodConsumerSummary {
    pub name: String,
    pub namespace: String,
    pub cpu_usage_m: u64,
    pub mem_usage_mib: u64,
}

/// Maximum entries shown in top-consumer and namespace utilization panels.
pub(crate) const TOP_N: usize = 5;

/// Returns (top_cpu_pods, top_mem_pods), each sorted by respective metric descending.
pub fn compute_top_pod_consumers(
    snapshot: &ClusterSnapshot,
) -> (Vec<PodConsumerSummary>, Vec<PodConsumerSummary>) {
    let mut consumers: Vec<PodConsumerSummary> = snapshot
        .pod_metrics
        .iter()
        .map(|pm| {
            let (cpu, mem) = pm.containers.iter().fold((0u64, 0u64), |(ac, am), c| {
                (ac + parse_millicores(&c.cpu), am + parse_mib(&c.memory))
            });
            PodConsumerSummary {
                name: pm.name.clone(),
                namespace: pm.namespace.clone(),
                cpu_usage_m: cpu,
                mem_usage_mib: mem,
            }
        })
        .collect();

    // Sort by CPU, take top N, then re-sort remainder by memory
    consumers.sort_unstable_by(|a, b| b.cpu_usage_m.cmp(&a.cpu_usage_m));
    let by_cpu: Vec<PodConsumerSummary> = consumers.iter().take(TOP_N).cloned().collect();

    consumers.sort_unstable_by(|a, b| b.mem_usage_mib.cmp(&a.mem_usage_mib));
    consumers.truncate(TOP_N);

    (by_cpu, consumers)
}

/// Per-namespace resource utilization aggregation for the dashboard.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamespaceUtilizationSummary {
    pub namespace: String,
    pub pod_count: usize,
    pub cpu_usage_m: u64,
    pub mem_usage_mib: u64,
    pub cpu_request_m: u64,
    pub mem_request_mib: u64,
    /// Actual CPU usage / CPU requests percentage. `None` if no requests.
    /// Can exceed 100% when pods burst above requests.
    pub cpu_req_utilization_pct: Option<u16>,
    /// Actual memory usage / memory requests percentage. `None` if no requests.
    /// Can exceed 100% when pods burst above requests.
    pub mem_req_utilization_pct: Option<u16>,
}

/// Aggregates pod metrics and resource requests by namespace.
/// Returns top namespaces sorted by CPU usage descending.
pub fn compute_namespace_utilization(
    snapshot: &ClusterSnapshot,
) -> Vec<NamespaceUtilizationSummary> {
    let mut by_ns: HashMap<&str, NamespaceUtilizationSummary> = HashMap::new();

    // Aggregate resource requests from running pods only
    for pod in &snapshot.pods {
        if !pod.status.eq_ignore_ascii_case("running") {
            continue;
        }
        let entry =
            by_ns
                .entry(pod.namespace.as_str())
                .or_insert_with(|| NamespaceUtilizationSummary {
                    namespace: pod.namespace.clone(),
                    ..Default::default()
                });
        entry.pod_count += 1;
        if let Some(ref req) = pod.cpu_request {
            entry.cpu_request_m += parse_millicores(req);
        }
        if let Some(ref req) = pod.memory_request {
            entry.mem_request_mib += parse_mib(req);
        }
    }

    // Aggregate actual usage from pod metrics
    for pm in &snapshot.pod_metrics {
        let entry =
            by_ns
                .entry(pm.namespace.as_str())
                .or_insert_with(|| NamespaceUtilizationSummary {
                    namespace: pm.namespace.clone(),
                    ..Default::default()
                });
        for c in &pm.containers {
            entry.cpu_usage_m += parse_millicores(&c.cpu);
            entry.mem_usage_mib += parse_mib(&c.memory);
        }
    }

    let mut result: Vec<NamespaceUtilizationSummary> = by_ns.into_values().collect();

    // Compute request utilization percentages (uncapped — can exceed 100% on burst)
    for ns in &mut result {
        ns.cpu_req_utilization_pct = if ns.cpu_request_m > 0 {
            Some((ns.cpu_usage_m * 100 / ns.cpu_request_m).min(999) as u16)
        } else {
            None
        };
        ns.mem_req_utilization_pct = if ns.mem_request_mib > 0 {
            Some((ns.mem_usage_mib * 100 / ns.mem_request_mib).min(999) as u16)
        } else {
            None
        };
    }

    result.sort_unstable_by(|a, b| b.cpu_usage_m.cmp(&a.cpu_usage_m));
    result
}

fn has_reason(reasons: &[String], expected_reason: &str) -> bool {
    reasons.iter().any(|reason| reason == expected_reason)
}

fn severity_from_count(count: usize, elevated: AlertSeverity) -> AlertSeverity {
    if count > 0 {
        elevated
    } else {
        AlertSeverity::Info
    }
}

fn severity_rank(severity: AlertSeverity) -> u8 {
    match severity {
        AlertSeverity::Error => 0,
        AlertSeverity::Warning => 1,
        AlertSeverity::Info => 2,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use crate::{
        k8s::dtos::{NodeInfo, PodInfo},
        state::ClusterSnapshot,
    };

    use super::*;

    fn pod(name: &str, status: &str) -> PodInfo {
        PodInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            status: status.to_string(),
            ..PodInfo::default()
        }
    }

    /// Verifies alerts are present with informational severity for an empty snapshot.
    #[test]
    fn compute_alerts_empty_snapshot_no_elevated_alerts() {
        let snapshot = ClusterSnapshot::default();
        let alerts = compute_alerts(&snapshot);

        assert_eq!(alerts.len(), 5);
        assert!(alerts.iter().all(|a| a.severity == AlertSeverity::Info));
    }

    /// Verifies MemoryPressure nodes produce warning severity and expected message.
    #[test]
    fn compute_alerts_memory_pressure_single_type() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.nodes.push(NodeInfo {
            name: "n1".to_string(),
            memory_pressure: true,
            ..NodeInfo::default()
        });

        let alerts = compute_alerts(&snapshot);
        let memory = alerts
            .iter()
            .find(|a| a.title == "MemoryPressure")
            .expect("memory alert should exist");

        assert_eq!(memory.severity, AlertSeverity::Warning);
        assert!(memory.message.contains("1 node(s)"));
    }

    /// Verifies CrashLoopBackOff pods produce error severity.
    #[test]
    fn compute_alerts_crash_loop_backoff_single_type() {
        let mut snapshot = ClusterSnapshot::default();
        let mut p = pod("p1", "Running");
        p.waiting_reasons = vec!["CrashLoopBackOff".to_string()];
        snapshot.pods.push(p);

        let alerts = compute_alerts(&snapshot);
        let crash = alerts
            .iter()
            .find(|a| a.title == "CrashLoopBackOff")
            .expect("crash alert should exist");

        assert_eq!(crash.severity, AlertSeverity::Error);
        assert!(crash.message.contains("1 pod(s)"));
    }

    /// Verifies failed pods produce error severity and proper wording.
    #[test]
    fn compute_alerts_failed_pods_single_type() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.pods.push(pod("p1", "Failed"));

        let alerts = compute_alerts(&snapshot);
        let failed = alerts
            .iter()
            .find(|a| a.title == "Failed pods")
            .expect("failed pods alert should exist");

        assert_eq!(failed.severity, AlertSeverity::Error);
        assert!(failed.message.contains("Failed phase"));
    }

    /// Verifies alert sorting prioritizes errors before warnings before infos.
    #[test]
    fn compute_alerts_severity_ordering() {
        let mut snapshot = ClusterSnapshot::default();
        let mut crash = pod("crash", "Running");
        crash.waiting_reasons = vec!["CrashLoopBackOff".to_string()];

        let mut image = pod("image", "Running");
        image.waiting_reasons = vec!["ImagePullBackOff".to_string()];

        snapshot.pods.push(crash);
        snapshot.pods.push(image);
        snapshot.pods.push(pod("failed", "Failed"));
        snapshot.nodes.push(NodeInfo {
            name: "n1".to_string(),
            memory_pressure: true,
            ..NodeInfo::default()
        });

        let alerts = compute_alerts(&snapshot);

        assert!(alerts[0].severity == AlertSeverity::Error);
        assert!(alerts[1].severity == AlertSeverity::Error);
        assert!(
            alerts[2].severity == AlertSeverity::Error
                || alerts[2].severity == AlertSeverity::Warning
        );
    }

    /// Verifies pending pod age boundary at 5 minutes is handled correctly.
    #[test]
    fn compute_alerts_pending_timestamp_boundary() {
        let mut snapshot = ClusterSnapshot::default();

        let mut fresh = pod("fresh", "Pending");
        fresh.created_at = Some(Utc::now() - Duration::minutes(4) - Duration::seconds(59));

        let mut old = pod("old", "Pending");
        old.created_at = Some(Utc::now() - Duration::minutes(6));

        snapshot.pods.push(fresh);
        snapshot.pods.push(old);

        let alerts = compute_alerts(&snapshot);
        let pending = alerts
            .iter()
            .find(|a| a.title == "Pending > 5m")
            .expect("pending alert should exist");

        assert!(pending.message.contains("1 pod(s)"));
    }

    /// Verifies very long pod names do not break alert message formatting.
    #[test]
    fn compute_alerts_long_names_do_not_panic() {
        let mut snapshot = ClusterSnapshot::default();
        let long_name = "x".repeat(500);
        snapshot.pods.push(PodInfo {
            name: long_name,
            namespace: "default".to_string(),
            status: "Failed".to_string(),
            ..PodInfo::default()
        });

        let alerts = compute_alerts(&snapshot);
        let failed = alerts
            .iter()
            .find(|a| a.title == "Failed pods")
            .expect("failed alert should exist");
        assert_eq!(failed.severity, AlertSeverity::Error);
    }

    /// Verifies all ready nodes and no failing pods produce only informational alerts.
    #[test]
    fn compute_alerts_all_nodes_ready_no_elevated() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.nodes.push(NodeInfo {
            name: "n1".to_string(),
            ready: true,
            ..NodeInfo::default()
        });

        let alerts = compute_alerts(&snapshot);
        assert!(alerts.iter().all(|a| a.severity == AlertSeverity::Info));
    }

    /// Verifies repeated MemoryPressure conditions are aggregated into one count.
    #[test]
    fn compute_alerts_memory_pressure_aggregation() {
        let mut snapshot = ClusterSnapshot::default();
        for i in 0..3 {
            snapshot.nodes.push(NodeInfo {
                name: format!("n{i}"),
                memory_pressure: true,
                ..NodeInfo::default()
            });
        }

        let alerts = compute_alerts(&snapshot);
        let memory = alerts
            .iter()
            .find(|a| a.title == "MemoryPressure")
            .expect("memory alert should exist");

        assert!(memory.message.contains("3 node(s)"));
    }

    /// Verifies dashboard stats percentages for zero denominators are zero.
    #[test]
    fn compute_dashboard_stats_zero_denominator() {
        let stats = compute_dashboard_stats(&ClusterSnapshot::default());
        assert_eq!(stats.ready_nodes_percent, 0);
        assert_eq!(stats.running_pods_percent, 0);
    }

    /// Verifies dashboard stats count namespaces across pods, services, and deployments.
    #[test]
    fn compute_dashboard_stats_namespace_union() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "a".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });
        snapshot.services.push(crate::k8s::dtos::ServiceInfo {
            name: "s1".to_string(),
            namespace: "b".to_string(),
            ..crate::k8s::dtos::ServiceInfo::default()
        });
        snapshot.deployments.push(crate::k8s::dtos::DeploymentInfo {
            name: "d1".to_string(),
            namespace: "c".to_string(),
            ready: "1/1".to_string(),
            ..crate::k8s::dtos::DeploymentInfo::default()
        });

        let stats = compute_dashboard_stats(&snapshot);
        assert_eq!(stats.namespaces_count, 3);
    }

    #[test]
    fn compute_dashboard_insights_marks_critical_for_not_ready_and_failed() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.nodes.push(NodeInfo {
            name: "node-a".to_string(),
            ready: false,
            ..NodeInfo::default()
        });
        snapshot.pods.push(PodInfo {
            name: "pod-a".to_string(),
            namespace: "default".to_string(),
            status: "Failed".to_string(),
            ..PodInfo::default()
        });

        let insights = compute_dashboard_insights(&snapshot);
        assert_eq!(insights.health_state, DashboardHealthState::Critical);
        assert_eq!(insights.not_ready_nodes, 1);
        assert_eq!(insights.failed_pods, 1);
    }

    #[test]
    fn compute_dashboard_insights_node_utilization_and_hot_nodes() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.nodes.push(NodeInfo {
            name: "node-a".to_string(),
            ready: true,
            cpu_allocatable: Some("2000m".to_string()),
            memory_allocatable: Some("2048Mi".to_string()),
            ..NodeInfo::default()
        });
        snapshot.nodes.push(NodeInfo {
            name: "node-b".to_string(),
            ready: true,
            cpu_allocatable: Some("2000m".to_string()),
            memory_allocatable: Some("2048Mi".to_string()),
            ..NodeInfo::default()
        });
        snapshot
            .node_metrics
            .push(crate::k8s::dtos::NodeMetricsInfo {
                name: "node-a".to_string(),
                cpu: "500m".to_string(),
                memory: "1024Mi".to_string(),
                ..crate::k8s::dtos::NodeMetricsInfo::default()
            });
        snapshot
            .node_metrics
            .push(crate::k8s::dtos::NodeMetricsInfo {
                name: "node-b".to_string(),
                cpu: "1800m".to_string(),
                memory: "1536Mi".to_string(),
                ..crate::k8s::dtos::NodeMetricsInfo::default()
            });

        let insights = compute_dashboard_insights(&snapshot);
        assert_eq!(insights.utilization_nodes, 2);
        assert_eq!(insights.avg_cpu_pct, 57);
        assert_eq!(insights.avg_mem_pct, 62);
        assert_eq!(insights.hot_cpu_nodes[0].name, "node-b");
        assert_eq!(insights.hot_mem_nodes[0].name, "node-b");
    }

    #[test]
    fn compute_workload_ready_percent_aggregates_workloads() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.deployments.push(crate::k8s::dtos::DeploymentInfo {
            desired_replicas: 4,
            ready_replicas: 3,
            ..crate::k8s::dtos::DeploymentInfo::default()
        });
        snapshot
            .statefulsets
            .push(crate::k8s::dtos::StatefulSetInfo {
                desired_replicas: 2,
                ready_replicas: 1,
                ..crate::k8s::dtos::StatefulSetInfo::default()
            });
        snapshot.daemonsets.push(crate::k8s::dtos::DaemonSetInfo {
            desired_count: 5,
            ready_count: 4,
            ..crate::k8s::dtos::DaemonSetInfo::default()
        });

        assert_eq!(compute_workload_ready_percent(&snapshot), 72);
    }

    #[test]
    fn compute_dashboard_insights_warns_on_high_saturation_without_failures() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.nodes.push(NodeInfo {
            name: "node-a".to_string(),
            ready: true,
            cpu_allocatable: Some("1000m".to_string()),
            memory_allocatable: Some("1024Mi".to_string()),
            ..NodeInfo::default()
        });
        snapshot
            .node_metrics
            .push(crate::k8s::dtos::NodeMetricsInfo {
                name: "node-a".to_string(),
                cpu: "900m".to_string(),
                memory: "700Mi".to_string(),
                ..crate::k8s::dtos::NodeMetricsInfo::default()
            });
        snapshot.pods.push(PodInfo {
            name: "pod-a".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });

        let insights = compute_dashboard_insights(&snapshot);
        assert_eq!(insights.health_state, DashboardHealthState::Warning);
        assert_eq!(insights.high_cpu_nodes, 1);
        assert_eq!(insights.failed_pods, 0);
        assert_eq!(insights.not_ready_nodes, 0);
    }

    #[test]
    fn compute_workload_ready_percent_defaults_to_full_when_no_workloads() {
        assert_eq!(
            compute_workload_ready_percent(&ClusterSnapshot::default()),
            100
        );
    }

    #[test]
    fn format_millicores_whole_cores() {
        assert_eq!(format_millicores(1000), "1");
        assert_eq!(format_millicores(2000), "2");
    }

    #[test]
    fn format_millicores_fractional() {
        assert_eq!(format_millicores(500), "500m");
        assert_eq!(format_millicores(1500), "1.5");
        assert_eq!(format_millicores(1250), "1.25");
        assert_eq!(format_millicores(1001), "1.001");
    }

    #[test]
    fn format_millicores_zero() {
        assert_eq!(format_millicores(0), "0m");
    }

    #[test]
    fn format_mib_gibibytes() {
        assert_eq!(format_mib(1024), "1Gi");
        assert_eq!(format_mib(2048), "2Gi");
    }

    #[test]
    fn format_mib_mebibytes() {
        assert_eq!(format_mib(256), "256Mi");
        assert_eq!(format_mib(512), "512Mi");
    }

    #[test]
    fn format_mib_zero() {
        assert_eq!(format_mib(0), "0Mi");
    }

    #[test]
    fn parse_millicores_nanocores() {
        assert_eq!(parse_millicores("500000000n"), 500);
        assert_eq!(parse_millicores("0n"), 0);
    }

    #[test]
    fn parse_millicores_whole_cores() {
        assert_eq!(parse_millicores("2"), 2000);
        assert_eq!(parse_millicores("250m"), 250);
    }

    #[test]
    fn parse_millicores_decimal_cores() {
        assert_eq!(parse_millicores("0.5"), 500);
        assert_eq!(parse_millicores("1.25"), 1250);
        assert_eq!(parse_millicores("0.1"), 100);
        assert_eq!(parse_millicores("2.0"), 2000);
    }

    #[test]
    fn compute_namespace_utilization_empty_snapshot() {
        let snapshot = ClusterSnapshot::default();
        let result = compute_namespace_utilization(&snapshot);
        assert!(result.is_empty());
    }

    #[test]
    fn compute_namespace_utilization_aggregates_by_namespace() {
        use crate::k8s::dtos::{ContainerMetrics, PodMetricsInfo};

        let mut snapshot = ClusterSnapshot::default();
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "ns-a".to_string(),
            status: "Running".to_string(),
            cpu_request: Some("100m".to_string()),
            memory_request: Some("256Mi".to_string()),
            ..PodInfo::default()
        });
        snapshot.pods.push(PodInfo {
            name: "p2".to_string(),
            namespace: "ns-a".to_string(),
            status: "Running".to_string(),
            cpu_request: Some("200m".to_string()),
            memory_request: Some("512Mi".to_string()),
            ..PodInfo::default()
        });
        snapshot.pods.push(PodInfo {
            name: "p3".to_string(),
            namespace: "ns-b".to_string(),
            status: "Running".to_string(),
            cpu_request: Some("500m".to_string()),
            memory_request: Some("1024Mi".to_string()),
            ..PodInfo::default()
        });

        snapshot.pod_metrics.push(PodMetricsInfo {
            name: "p1".to_string(),
            namespace: "ns-a".to_string(),
            containers: vec![ContainerMetrics {
                name: "c1".to_string(),
                cpu: "50m".to_string(),
                memory: "128Mi".to_string(),
            }],
            ..PodMetricsInfo::default()
        });
        snapshot.pod_metrics.push(PodMetricsInfo {
            name: "p2".to_string(),
            namespace: "ns-a".to_string(),
            containers: vec![ContainerMetrics {
                name: "c1".to_string(),
                cpu: "100m".to_string(),
                memory: "256Mi".to_string(),
            }],
            ..PodMetricsInfo::default()
        });
        snapshot.pod_metrics.push(PodMetricsInfo {
            name: "p3".to_string(),
            namespace: "ns-b".to_string(),
            containers: vec![ContainerMetrics {
                name: "c1".to_string(),
                cpu: "400m".to_string(),
                memory: "512Mi".to_string(),
            }],
            ..PodMetricsInfo::default()
        });

        let result = compute_namespace_utilization(&snapshot);
        assert_eq!(result.len(), 2);

        // Sorted by CPU usage descending: ns-b (400m) > ns-a (150m)
        assert_eq!(result[0].namespace, "ns-b");
        assert_eq!(result[0].pod_count, 1);
        assert_eq!(result[0].cpu_usage_m, 400);
        assert_eq!(result[0].mem_usage_mib, 512);
        assert_eq!(result[0].cpu_request_m, 500);
        assert_eq!(result[0].mem_request_mib, 1024);

        assert_eq!(result[1].namespace, "ns-a");
        assert_eq!(result[1].pod_count, 2);
        assert_eq!(result[1].cpu_usage_m, 150);
        assert_eq!(result[1].mem_usage_mib, 384);
        assert_eq!(result[1].cpu_request_m, 300);
        assert_eq!(result[1].mem_request_mib, 768);
    }

    #[test]
    fn compute_cluster_resource_summary_empty() {
        let summary = compute_cluster_resource_summary(&ClusterSnapshot::default());
        assert_eq!(summary.total_cpu_used_m, 0);
        assert_eq!(summary.total_mem_used_mib, 0);
        assert_eq!(summary.total_cpu_allocatable_m, 0);
        assert_eq!(summary.total_mem_allocatable_mib, 0);
        assert_eq!(summary.cluster_cpu_pct, 0);
        assert_eq!(summary.cluster_mem_pct, 0);
        assert_eq!(summary.total_running_pods, 0);
    }

    #[test]
    fn compute_cluster_resource_summary_with_metrics() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.nodes.push(NodeInfo {
            name: "n1".to_string(),
            ready: true,
            cpu_allocatable: Some("4000m".to_string()),
            memory_allocatable: Some("8192Mi".to_string()),
            ..NodeInfo::default()
        });
        snapshot
            .node_metrics
            .push(crate::k8s::dtos::NodeMetricsInfo {
                name: "n1".to_string(),
                cpu: "2000m".to_string(),
                memory: "4096Mi".to_string(),
                ..crate::k8s::dtos::NodeMetricsInfo::default()
            });
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            cpu_request: Some("1000m".to_string()),
            memory_request: Some("2048Mi".to_string()),
            cpu_limit: Some("2000m".to_string()),
            memory_limit: Some("4096Mi".to_string()),
            ..PodInfo::default()
        });

        let summary = compute_cluster_resource_summary(&snapshot);
        assert_eq!(summary.cluster_cpu_pct, 50);
        assert_eq!(summary.cluster_mem_pct, 50);
        assert_eq!(summary.cpu_request_commitment_pct, 25);
        assert_eq!(summary.mem_request_commitment_pct, 25);
        assert_eq!(summary.cpu_limit_commitment_pct, 50);
        assert_eq!(summary.mem_limit_commitment_pct, 50);
        assert_eq!(summary.total_running_pods, 1);
        assert_eq!(summary.pods_missing_cpu_request, 0);
        assert_eq!(summary.pods_missing_mem_request, 0);
        assert_eq!(summary.pods_missing_any_limit, 0);
    }

    #[test]
    fn compute_cluster_resource_summary_overcommitted() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.nodes.push(NodeInfo {
            name: "n1".to_string(),
            cpu_allocatable: Some("1000m".to_string()),
            memory_allocatable: Some("1024Mi".to_string()),
            ..NodeInfo::default()
        });
        // Pod requests exceed allocatable
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            cpu_request: Some("800m".to_string()),
            memory_request: Some("512Mi".to_string()),
            ..PodInfo::default()
        });
        snapshot.pods.push(PodInfo {
            name: "p2".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            cpu_request: Some("500m".to_string()),
            memory_request: Some("768Mi".to_string()),
            ..PodInfo::default()
        });

        let summary = compute_cluster_resource_summary(&snapshot);
        assert_eq!(summary.cpu_request_commitment_pct, 130); // 1300m / 1000m
        assert_eq!(summary.mem_request_commitment_pct, 125); // 1280Mi / 1024Mi
    }

    #[test]
    fn compute_cluster_resource_summary_no_metrics_server() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.nodes.push(NodeInfo {
            name: "n1".to_string(),
            cpu_allocatable: Some("2000m".to_string()),
            memory_allocatable: Some("4096Mi".to_string()),
            ..NodeInfo::default()
        });
        // No node_metrics → usage stays 0
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            cpu_request: Some("500m".to_string()),
            memory_request: Some("1024Mi".to_string()),
            ..PodInfo::default()
        });

        let summary = compute_cluster_resource_summary(&snapshot);
        assert_eq!(summary.cluster_cpu_pct, 0);
        assert_eq!(summary.cluster_mem_pct, 0);
        assert_eq!(summary.cpu_request_commitment_pct, 25);
        assert_eq!(summary.mem_request_commitment_pct, 25);
    }

    #[test]
    fn compute_cluster_resource_summary_missing_requests_count() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.nodes.push(NodeInfo {
            name: "n1".to_string(),
            cpu_allocatable: Some("2000m".to_string()),
            memory_allocatable: Some("4096Mi".to_string()),
            ..NodeInfo::default()
        });
        // Pod with no requests or limits
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });
        // Pod with CPU request but no memory request, and one limit
        snapshot.pods.push(PodInfo {
            name: "p2".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            cpu_request: Some("100m".to_string()),
            cpu_limit: Some("200m".to_string()),
            ..PodInfo::default()
        });
        // Non-running pod should be ignored
        snapshot.pods.push(PodInfo {
            name: "p3".to_string(),
            namespace: "default".to_string(),
            status: "Pending".to_string(),
            ..PodInfo::default()
        });

        let summary = compute_cluster_resource_summary(&snapshot);
        assert_eq!(summary.total_running_pods, 2);
        assert_eq!(summary.pods_missing_cpu_request, 1); // p1
        assert_eq!(summary.pods_missing_mem_request, 2); // p1 + p2
        assert_eq!(summary.pods_missing_any_limit, 2); // p1 (no limits) + p2 (no mem_limit)
    }

    #[test]
    fn compute_top_pod_consumers_empty() {
        let (cpu, mem) = compute_top_pod_consumers(&ClusterSnapshot::default());
        assert!(cpu.is_empty());
        assert!(mem.is_empty());
    }

    #[test]
    fn compute_top_pod_consumers_ordering() {
        use crate::k8s::dtos::{ContainerMetrics, PodMetricsInfo};

        let mut snapshot = ClusterSnapshot::default();
        // Pod A: high CPU, low mem
        snapshot.pod_metrics.push(PodMetricsInfo {
            name: "pod-a".to_string(),
            namespace: "default".to_string(),
            containers: vec![ContainerMetrics {
                name: "c1".to_string(),
                cpu: "500m".to_string(),
                memory: "100Mi".to_string(),
            }],
            ..PodMetricsInfo::default()
        });
        // Pod B: low CPU, high mem
        snapshot.pod_metrics.push(PodMetricsInfo {
            name: "pod-b".to_string(),
            namespace: "default".to_string(),
            containers: vec![ContainerMetrics {
                name: "c1".to_string(),
                cpu: "100m".to_string(),
                memory: "2048Mi".to_string(),
            }],
            ..PodMetricsInfo::default()
        });

        let (by_cpu, by_mem) = compute_top_pod_consumers(&snapshot);
        assert_eq!(by_cpu[0].name, "pod-a");
        assert_eq!(by_cpu[1].name, "pod-b");
        assert_eq!(by_mem[0].name, "pod-b");
        assert_eq!(by_mem[1].name, "pod-a");
    }

    #[test]
    fn compute_top_pod_consumers_fewer_than_5() {
        use crate::k8s::dtos::{ContainerMetrics, PodMetricsInfo};

        let mut snapshot = ClusterSnapshot::default();
        for i in 0..3 {
            snapshot.pod_metrics.push(PodMetricsInfo {
                name: format!("pod-{i}"),
                namespace: "default".to_string(),
                containers: vec![ContainerMetrics {
                    name: "c1".to_string(),
                    cpu: format!("{}m", (i + 1) * 100),
                    memory: format!("{}Mi", (i + 1) * 256),
                }],
                ..PodMetricsInfo::default()
            });
        }

        let (by_cpu, by_mem) = compute_top_pod_consumers(&snapshot);
        assert_eq!(by_cpu.len(), 3);
        assert_eq!(by_mem.len(), 3);
    }

    #[test]
    fn compute_namespace_utilization_includes_req_pct() {
        use crate::k8s::dtos::{ContainerMetrics, PodMetricsInfo};

        let mut snapshot = ClusterSnapshot::default();
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "ns-a".to_string(),
            status: "Running".to_string(),
            cpu_request: Some("200m".to_string()),
            memory_request: Some("1024Mi".to_string()),
            ..PodInfo::default()
        });
        snapshot.pod_metrics.push(PodMetricsInfo {
            name: "p1".to_string(),
            namespace: "ns-a".to_string(),
            containers: vec![ContainerMetrics {
                name: "c1".to_string(),
                cpu: "100m".to_string(),
                memory: "512Mi".to_string(),
            }],
            ..PodMetricsInfo::default()
        });

        let result = compute_namespace_utilization(&snapshot);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].cpu_req_utilization_pct, Some(50));
        assert_eq!(result[0].mem_req_utilization_pct, Some(50));
    }

    #[test]
    fn compute_namespace_utilization_no_requests_yields_none() {
        use crate::k8s::dtos::{ContainerMetrics, PodMetricsInfo};

        let mut snapshot = ClusterSnapshot::default();
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "ns-a".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });
        snapshot.pod_metrics.push(PodMetricsInfo {
            name: "p1".to_string(),
            namespace: "ns-a".to_string(),
            containers: vec![ContainerMetrics {
                name: "c1".to_string(),
                cpu: "100m".to_string(),
                memory: "512Mi".to_string(),
            }],
            ..PodMetricsInfo::default()
        });

        let result = compute_namespace_utilization(&snapshot);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].cpu_req_utilization_pct, None);
        assert_eq!(result[0].mem_req_utilization_pct, None);
    }

    #[test]
    fn compute_namespace_utilization_excludes_non_running_pods() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.pods.push(PodInfo {
            name: "running".to_string(),
            namespace: "ns-a".to_string(),
            status: "Running".to_string(),
            cpu_request: Some("100m".to_string()),
            memory_request: Some("256Mi".to_string()),
            ..PodInfo::default()
        });
        // Completed pod should not inflate request denominators
        snapshot.pods.push(PodInfo {
            name: "completed".to_string(),
            namespace: "ns-a".to_string(),
            status: "Succeeded".to_string(),
            cpu_request: Some("500m".to_string()),
            memory_request: Some("1024Mi".to_string()),
            ..PodInfo::default()
        });

        let result = compute_namespace_utilization(&snapshot);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].pod_count, 1);
        assert_eq!(result[0].cpu_request_m, 100);
        assert_eq!(result[0].mem_request_mib, 256);
    }

    #[test]
    fn compute_namespace_utilization_over_100_pct() {
        use crate::k8s::dtos::{ContainerMetrics, PodMetricsInfo};

        let mut snapshot = ClusterSnapshot::default();
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "ns-a".to_string(),
            status: "Running".to_string(),
            cpu_request: Some("100m".to_string()),
            memory_request: Some("256Mi".to_string()),
            ..PodInfo::default()
        });
        // Usage exceeds requests (burst)
        snapshot.pod_metrics.push(PodMetricsInfo {
            name: "p1".to_string(),
            namespace: "ns-a".to_string(),
            containers: vec![ContainerMetrics {
                name: "c1".to_string(),
                cpu: "300m".to_string(),
                memory: "768Mi".to_string(),
            }],
            ..PodMetricsInfo::default()
        });

        let result = compute_namespace_utilization(&snapshot);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].cpu_req_utilization_pct, Some(300));
        assert_eq!(result[0].mem_req_utilization_pct, Some(300));
    }

    #[test]
    fn parse_millicores_edge_cases() {
        assert_eq!(parse_millicores(""), 0);
        assert_eq!(parse_millicores("abc"), 0);
        assert_eq!(parse_millicores("0"), 0);
        assert_eq!(parse_millicores("0m"), 0);
        assert_eq!(parse_millicores("0.0"), 0);
        assert_eq!(parse_millicores("-100m"), 0); // negative → unwrap_or(0)
    }
}
