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
}
