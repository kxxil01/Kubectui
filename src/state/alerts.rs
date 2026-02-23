//! Pure dashboard statistics and alert aggregation logic.

use std::collections::BTreeSet;

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

    alerts.sort_by_key(|item| severity_rank(item.severity));
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
