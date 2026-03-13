//! Cluster issue detection and caching.
//!
//! Computes per-resource issues from snapshot data (no new API calls).
//! Results are cached by `snapshot_version` and reused across frames.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use crate::app::ResourceRef;
use crate::k8s::dtos::AlertSeverity;
use crate::state::{ClusterSnapshot, RefreshScope};
use crate::ui::contains_ci;

const MAX_ISSUES: usize = 500;

const fn severity_rank(s: AlertSeverity) -> u8 {
    match s {
        AlertSeverity::Error => 0,
        AlertSeverity::Warning => 1,
        AlertSeverity::Info => 2,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IssueCategory {
    CrashLoopBackOff,
    ImagePullFailure,
    PendingPod,
    FailedPod,
    NodeNotReady,
    NodePressure,
    DegradedWorkload,
    StorageIssue,
    FluxReconcileFailure,
    ServiceNoEndpoints,
    FailedJob,
}

impl IssueCategory {
    pub const fn label(self) -> &'static str {
        match self {
            Self::CrashLoopBackOff => "CrashLoopBackOff",
            Self::ImagePullFailure => "ImagePullFailure",
            Self::PendingPod => "Pending Pod",
            Self::FailedPod => "Failed Pod",
            Self::NodeNotReady => "Node Not Ready",
            Self::NodePressure => "Node Pressure",
            Self::DegradedWorkload => "Degraded Workload",
            Self::StorageIssue => "Storage Issue",
            Self::FluxReconcileFailure => "Flux Reconcile Fail",
            Self::ServiceNoEndpoints => "No Endpoints",
            Self::FailedJob => "Failed Job",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClusterIssue {
    pub severity: AlertSeverity,
    pub category: IssueCategory,
    pub resource_kind: &'static str,
    pub resource_name: String,
    pub namespace: String,
    pub message: String,
    pub resource_ref: ResourceRef,
}

impl ClusterIssue {
    /// Returns `true` if any text field matches the query (case-insensitive).
    pub fn matches_query(&self, query: &str) -> bool {
        contains_ci(self.category.label(), query)
            || contains_ci(self.resource_kind, query)
            || contains_ci(&self.resource_name, query)
            || contains_ci(&self.namespace, query)
            || contains_ci(&self.message, query)
    }
}

/// Returns filtered issue indices matching the search query.
/// Used by both the render path and the `selected_resource` action path.
pub fn filtered_issue_indices(issues: &[ClusterIssue], query: &str) -> Vec<usize> {
    if query.is_empty() {
        (0..issues.len()).collect()
    } else {
        issues
            .iter()
            .enumerate()
            .filter_map(|(idx, issue)| issue.matches_query(query).then_some(idx))
            .collect()
    }
}

#[allow(clippy::type_complexity)]
static ISSUE_CACHE: LazyLock<Mutex<Option<(u64, Arc<Vec<ClusterIssue>>)>>> =
    LazyLock::new(|| Mutex::new(None));

/// Returns cached issues for the given snapshot, computing on first call per version.
pub fn compute_issues(snapshot: &ClusterSnapshot) -> Arc<Vec<ClusterIssue>> {
    let version = snapshot.snapshot_version;
    {
        let guard = ISSUE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached_ver, ref issues)) = *guard
            && cached_ver == version
        {
            return Arc::clone(issues);
        }
    }

    let issues = Arc::new(detect_issues(snapshot));
    {
        let mut guard = ISSUE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some((version, Arc::clone(&issues)));
    }
    issues
}

fn detect_issues(snapshot: &ClusterSnapshot) -> Vec<ClusterIssue> {
    let mut issues = Vec::new();
    let mut seen_pod_indices = HashSet::new();

    // 1. CrashLoopBackOff
    for (idx, pod) in snapshot.pods.iter().enumerate() {
        if pod
            .waiting_reasons
            .iter()
            .any(|r| r.contains("CrashLoopBackOff"))
        {
            seen_pod_indices.insert(idx);
            issues.push(ClusterIssue {
                severity: AlertSeverity::Error,
                category: IssueCategory::CrashLoopBackOff,
                resource_kind: "Pod",
                resource_name: pod.name.clone(),
                namespace: pod.namespace.clone(),
                message: format!("Container in CrashLoopBackOff (restarts: {})", pod.restarts),
                resource_ref: ResourceRef::Pod(pod.name.clone(), pod.namespace.clone()),
            });
        }
    }

    // 2. ImagePullFailure (skip pods already seen as CrashLoopBackOff)
    for (idx, pod) in snapshot.pods.iter().enumerate() {
        if seen_pod_indices.contains(&idx) {
            continue;
        }
        let has_image_issue = pod.waiting_reasons.iter().any(|r| {
            r.contains("ImagePullBackOff")
                || r.contains("ErrImagePull")
                || r.contains("CreateContainerConfigError")
        });
        if has_image_issue {
            seen_pod_indices.insert(idx);
            let reason = pod
                .waiting_reasons
                .iter()
                .find(|r| {
                    r.contains("ImagePullBackOff")
                        || r.contains("ErrImagePull")
                        || r.contains("CreateContainerConfigError")
                })
                .cloned()
                .unwrap_or_default();
            issues.push(ClusterIssue {
                severity: AlertSeverity::Error,
                category: IssueCategory::ImagePullFailure,
                resource_kind: "Pod",
                resource_name: pod.name.clone(),
                namespace: pod.namespace.clone(),
                message: reason,
                resource_ref: ResourceRef::Pod(pod.name.clone(), pod.namespace.clone()),
            });
        }
    }

    // 3. PendingPod (skip pods already categorised above)
    for (idx, pod) in snapshot.pods.iter().enumerate() {
        if seen_pod_indices.contains(&idx) {
            continue;
        }
        if pod.status.eq_ignore_ascii_case("pending") {
            let age_secs = pod
                .created_at
                .map(|t| (chrono::Utc::now().timestamp() - t.timestamp()).max(0) as u64)
                .unwrap_or(0);
            let severity = if age_secs > 300 {
                AlertSeverity::Warning
            } else {
                AlertSeverity::Info
            };
            issues.push(ClusterIssue {
                severity,
                category: IssueCategory::PendingPod,
                resource_kind: "Pod",
                resource_name: pod.name.clone(),
                namespace: pod.namespace.clone(),
                message: format!("Pod pending for {}s", age_secs),
                resource_ref: ResourceRef::Pod(pod.name.clone(), pod.namespace.clone()),
            });
        }
    }

    // 4. FailedPod (skip pods already categorised above)
    for (idx, pod) in snapshot.pods.iter().enumerate() {
        if seen_pod_indices.contains(&idx) {
            continue;
        }
        if pod.status.eq_ignore_ascii_case("failed") {
            issues.push(ClusterIssue {
                severity: AlertSeverity::Error,
                category: IssueCategory::FailedPod,
                resource_kind: "Pod",
                resource_name: pod.name.clone(),
                namespace: pod.namespace.clone(),
                message: "Pod in Failed state".to_string(),
                resource_ref: ResourceRef::Pod(pod.name.clone(), pod.namespace.clone()),
            });
        }
    }

    // 5. NodeNotReady
    for node in &snapshot.nodes {
        if !node.ready {
            issues.push(ClusterIssue {
                severity: AlertSeverity::Error,
                category: IssueCategory::NodeNotReady,
                resource_kind: "Node",
                resource_name: node.name.clone(),
                namespace: String::new(),
                message: "Node is not ready".to_string(),
                resource_ref: ResourceRef::Node(node.name.clone()),
            });
        }
    }

    // 6. NodePressure
    for node in &snapshot.nodes {
        let mut pressures = Vec::new();
        if node.memory_pressure {
            pressures.push("Memory");
        }
        if node.disk_pressure {
            pressures.push("Disk");
        }
        if node.pid_pressure {
            pressures.push("PID");
        }
        if node.network_unavailable {
            pressures.push("Network");
        }
        if !pressures.is_empty() {
            issues.push(ClusterIssue {
                severity: AlertSeverity::Warning,
                category: IssueCategory::NodePressure,
                resource_kind: "Node",
                resource_name: node.name.clone(),
                namespace: String::new(),
                message: format!("{} pressure", pressures.join(", ")),
                resource_ref: ResourceRef::Node(node.name.clone()),
            });
        }
    }

    // 7. DegradedWorkload — Deployments
    for dep in &snapshot.deployments {
        if dep.desired_replicas > 0 && dep.ready_replicas < dep.desired_replicas {
            issues.push(ClusterIssue {
                severity: AlertSeverity::Warning,
                category: IssueCategory::DegradedWorkload,
                resource_kind: "Deployment",
                resource_name: dep.name.clone(),
                namespace: dep.namespace.clone(),
                message: format!("{}/{} ready", dep.ready_replicas, dep.desired_replicas),
                resource_ref: ResourceRef::Deployment(dep.name.clone(), dep.namespace.clone()),
            });
        }
    }
    // StatefulSets
    for sts in &snapshot.statefulsets {
        if sts.desired_replicas > 0 && sts.ready_replicas < sts.desired_replicas {
            issues.push(ClusterIssue {
                severity: AlertSeverity::Warning,
                category: IssueCategory::DegradedWorkload,
                resource_kind: "StatefulSet",
                resource_name: sts.name.clone(),
                namespace: sts.namespace.clone(),
                message: format!("{}/{} ready", sts.ready_replicas, sts.desired_replicas),
                resource_ref: ResourceRef::StatefulSet(sts.name.clone(), sts.namespace.clone()),
            });
        }
    }
    // DaemonSets
    for ds in &snapshot.daemonsets {
        if ds.desired_count > 0 && ds.ready_count < ds.desired_count {
            issues.push(ClusterIssue {
                severity: AlertSeverity::Warning,
                category: IssueCategory::DegradedWorkload,
                resource_kind: "DaemonSet",
                resource_name: ds.name.clone(),
                namespace: ds.namespace.clone(),
                message: format!("{}/{} ready", ds.ready_count, ds.desired_count),
                resource_ref: ResourceRef::DaemonSet(ds.name.clone(), ds.namespace.clone()),
            });
        }
    }

    // 8. StorageIssue — PVCs
    for pvc in &snapshot.pvcs {
        let severity = match pvc.status.as_str() {
            "Lost" => Some(AlertSeverity::Error),
            "Pending" => Some(AlertSeverity::Warning),
            _ => None,
        };
        if let Some(sev) = severity {
            issues.push(ClusterIssue {
                severity: sev,
                category: IssueCategory::StorageIssue,
                resource_kind: "PVC",
                resource_name: pvc.name.clone(),
                namespace: pvc.namespace.clone(),
                message: format!("PVC {}", pvc.status),
                resource_ref: ResourceRef::Pvc(pvc.name.clone(), pvc.namespace.clone()),
            });
        }
    }
    // PVs
    for pv in &snapshot.pvs {
        let severity = match pv.status.as_str() {
            "Failed" => Some(AlertSeverity::Error),
            "Released" => Some(AlertSeverity::Warning),
            _ => None,
        };
        if let Some(sev) = severity {
            issues.push(ClusterIssue {
                severity: sev,
                category: IssueCategory::StorageIssue,
                resource_kind: "PV",
                resource_name: pv.name.clone(),
                namespace: String::new(),
                message: format!("PV {}", pv.status),
                resource_ref: ResourceRef::Pv(pv.name.clone()),
            });
        }
    }

    // 9. FluxReconcileFailure (Stalled → Error, NotReady → Warning)
    for flux in &snapshot.flux_resources {
        if flux.suspended || flux.status == "Ready" {
            continue;
        }
        let is_stalled = flux.status == "Stalled"
            || flux.conditions.iter().any(|c| {
                c.type_.eq_ignore_ascii_case("Stalled") && c.status.eq_ignore_ascii_case("True")
            });
        issues.push(ClusterIssue {
            severity: if is_stalled {
                AlertSeverity::Error
            } else {
                AlertSeverity::Warning
            },
            category: IssueCategory::FluxReconcileFailure,
            resource_kind: "FluxResource",
            resource_name: flux.name.clone(),
            namespace: flux.namespace.clone().unwrap_or_default(),
            message: if is_stalled {
                format!(
                    "Stalled: {}",
                    flux.message.as_deref().unwrap_or("reconciliation stalled")
                )
            } else {
                flux.message
                    .clone()
                    .unwrap_or_else(|| format!("Status: {}", flux.status))
            },
            resource_ref: ResourceRef::CustomResource {
                name: flux.name.clone(),
                namespace: flux.namespace.clone(),
                group: flux.group.clone(),
                version: flux.version.clone(),
                kind: flux.kind.clone(),
                plural: flux.plural.clone(),
            },
        });
    }

    // 10. ServiceNoEndpoints (O(S+E) via HashMap)
    let ep_map: HashMap<(&str, &str), &crate::k8s::dtos::EndpointInfo> = snapshot
        .endpoints
        .iter()
        .map(|ep| ((ep.name.as_str(), ep.namespace.as_str()), ep))
        .collect();
    let endpoints_loaded =
        !snapshot.endpoints.is_empty() || snapshot.loaded_scope.contains(RefreshScope::NETWORK);
    for svc in &snapshot.services {
        if svc.type_ == "ExternalName" {
            continue;
        }
        match ep_map.get(&(svc.name.as_str(), svc.namespace.as_str())) {
            Some(ep) if ep.addresses.is_empty() => {
                issues.push(ClusterIssue {
                    severity: AlertSeverity::Warning,
                    category: IssueCategory::ServiceNoEndpoints,
                    resource_kind: "Service",
                    resource_name: svc.name.clone(),
                    namespace: svc.namespace.clone(),
                    message: "Service has no ready endpoints".to_string(),
                    resource_ref: ResourceRef::Service(svc.name.clone(), svc.namespace.clone()),
                });
            }
            None if endpoints_loaded => {
                issues.push(ClusterIssue {
                    severity: AlertSeverity::Warning,
                    category: IssueCategory::ServiceNoEndpoints,
                    resource_kind: "Service",
                    resource_name: svc.name.clone(),
                    namespace: svc.namespace.clone(),
                    message: "No Endpoints object found for service".to_string(),
                    resource_ref: ResourceRef::Service(svc.name.clone(), svc.namespace.clone()),
                });
            }
            _ => {}
        }
    }

    // 11. FailedJob
    for job in &snapshot.jobs {
        if job.status.eq_ignore_ascii_case("failed") || job.failed_pods > 0 {
            issues.push(ClusterIssue {
                severity: AlertSeverity::Error,
                category: IssueCategory::FailedJob,
                resource_kind: "Job",
                resource_name: job.name.clone(),
                namespace: job.namespace.clone(),
                message: if job.failed_pods > 0 {
                    format!("{} failed pod(s)", job.failed_pods)
                } else {
                    "Job failed".to_string()
                },
                resource_ref: ResourceRef::Job(job.name.clone(), job.namespace.clone()),
            });
        }
    }

    // Sort: severity rank (Error first), then category, then name.
    issues.sort_by(|a, b| {
        severity_rank(a.severity)
            .cmp(&severity_rank(b.severity))
            .then_with(|| a.category.cmp(&b.category))
            .then_with(|| a.resource_name.cmp(&b.resource_name))
    });

    issues.truncate(MAX_ISSUES);
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::dtos::*;
    use crate::state::ClusterSnapshot;

    fn empty_snapshot() -> ClusterSnapshot {
        ClusterSnapshot::default()
    }

    #[test]
    fn crashloop_detected() {
        let mut snap = empty_snapshot();
        snap.pods.push(PodInfo {
            name: "web-0".into(),
            namespace: "default".into(),
            status: "Running".into(),
            waiting_reasons: vec!["CrashLoopBackOff".into()],
            restarts: 5,
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::CrashLoopBackOff);
        assert_eq!(issues[0].severity, AlertSeverity::Error);
    }

    #[test]
    fn image_pull_detected() {
        let mut snap = empty_snapshot();
        snap.pods.push(PodInfo {
            name: "api-0".into(),
            namespace: "prod".into(),
            waiting_reasons: vec!["ImagePullBackOff".into()],
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::ImagePullFailure);
    }

    #[test]
    fn crashloop_deduplicates_image_pull() {
        let mut snap = empty_snapshot();
        snap.pods.push(PodInfo {
            name: "web-0".into(),
            namespace: "default".into(),
            waiting_reasons: vec!["CrashLoopBackOff".into(), "ImagePullBackOff".into()],
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::CrashLoopBackOff);
    }

    #[test]
    fn pending_pod_severity_by_age() {
        let mut snap = empty_snapshot();
        // Recent pending pod → Info
        snap.pods.push(PodInfo {
            name: "new-pod".into(),
            namespace: "default".into(),
            status: "Pending".into(),
            created_at: Some(chrono::Utc::now()),
            ..Default::default()
        });
        // Old pending pod → Warning
        snap.pods.push(PodInfo {
            name: "old-pod".into(),
            namespace: "default".into(),
            status: "Pending".into(),
            created_at: Some(chrono::Utc::now() - chrono::Duration::minutes(10)),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        let pending: Vec<_> = issues
            .iter()
            .filter(|i| i.category == IssueCategory::PendingPod)
            .collect();
        assert_eq!(pending.len(), 2);
        assert!(pending.iter().any(|i| i.severity == AlertSeverity::Info));
        assert!(pending.iter().any(|i| i.severity == AlertSeverity::Warning));
    }

    #[test]
    fn failed_pod_detected() {
        let mut snap = empty_snapshot();
        snap.pods.push(PodInfo {
            name: "fail-0".into(),
            namespace: "default".into(),
            status: "Failed".into(),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::FailedPod);
    }

    #[test]
    fn node_not_ready_detected() {
        let mut snap = empty_snapshot();
        snap.nodes.push(NodeInfo {
            name: "node-1".into(),
            ready: false,
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::NodeNotReady);
    }

    #[test]
    fn node_pressure_detected() {
        let mut snap = empty_snapshot();
        snap.nodes.push(NodeInfo {
            name: "node-2".into(),
            ready: true,
            memory_pressure: true,
            disk_pressure: true,
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::NodePressure);
        assert!(issues[0].message.contains("Memory"));
        assert!(issues[0].message.contains("Disk"));
    }

    #[test]
    fn degraded_deployment_detected() {
        let mut snap = empty_snapshot();
        snap.deployments.push(DeploymentInfo {
            name: "api".into(),
            namespace: "prod".into(),
            desired_replicas: 3,
            ready_replicas: 1,
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::DegradedWorkload);
        assert!(issues[0].message.contains("1/3"));
    }

    #[test]
    fn degraded_statefulset_detected() {
        let mut snap = empty_snapshot();
        snap.statefulsets.push(StatefulSetInfo {
            name: "db".into(),
            namespace: "prod".into(),
            desired_replicas: 3,
            ready_replicas: 2,
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::DegradedWorkload);
    }

    #[test]
    fn degraded_daemonset_detected() {
        let mut snap = empty_snapshot();
        snap.daemonsets.push(DaemonSetInfo {
            name: "agent".into(),
            namespace: "kube-system".into(),
            desired_count: 5,
            ready_count: 3,
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::DegradedWorkload);
    }

    #[test]
    fn pvc_pending_detected() {
        let mut snap = empty_snapshot();
        snap.pvcs.push(PvcInfo {
            name: "data-0".into(),
            namespace: "default".into(),
            status: "Pending".into(),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::StorageIssue);
        assert_eq!(issues[0].severity, AlertSeverity::Warning);
    }

    #[test]
    fn pv_failed_detected() {
        let mut snap = empty_snapshot();
        snap.pvs.push(PvInfo {
            name: "pv-1".into(),
            status: "Failed".into(),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::StorageIssue);
        assert_eq!(issues[0].severity, AlertSeverity::Error);
    }

    #[test]
    fn flux_reconcile_failure_detected() {
        let mut snap = empty_snapshot();
        snap.flux_resources.push(FluxResourceInfo {
            name: "my-ks".into(),
            namespace: Some("flux-system".into()),
            kind: "Kustomization".into(),
            group: "kustomize.toolkit.fluxcd.io".into(),
            version: "v1".into(),
            plural: "kustomizations".into(),
            status: "False".into(),
            suspended: false,
            message: Some("apply failed".into()),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::FluxReconcileFailure);
    }

    #[test]
    fn flux_suspended_not_flagged() {
        let mut snap = empty_snapshot();
        snap.flux_resources.push(FluxResourceInfo {
            name: "suspended-ks".into(),
            status: "False".into(),
            suspended: true,
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert!(issues.is_empty());
    }

    #[test]
    fn service_no_endpoints_detected() {
        let mut snap = empty_snapshot();
        snap.services.push(ServiceInfo {
            name: "my-svc".into(),
            namespace: "default".into(),
            type_: "ClusterIP".into(),
            ..Default::default()
        });
        snap.endpoints.push(EndpointInfo {
            name: "my-svc".into(),
            namespace: "default".into(),
            addresses: vec![],
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::ServiceNoEndpoints);
    }

    #[test]
    fn external_name_skipped() {
        let mut snap = empty_snapshot();
        snap.services.push(ServiceInfo {
            name: "ext".into(),
            namespace: "default".into(),
            type_: "ExternalName".into(),
            ..Default::default()
        });
        snap.endpoints.push(EndpointInfo {
            name: "ext".into(),
            namespace: "default".into(),
            addresses: vec![],
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert!(issues.is_empty());
    }

    #[test]
    fn failed_job_detected() {
        let mut snap = empty_snapshot();
        snap.jobs.push(JobInfo {
            name: "migrate".into(),
            namespace: "default".into(),
            status: "Failed".into(),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::FailedJob);
    }

    #[test]
    fn job_with_failed_pods_detected() {
        let mut snap = empty_snapshot();
        snap.jobs.push(JobInfo {
            name: "batch".into(),
            namespace: "default".into(),
            status: "Running".into(),
            failed_pods: 2,
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("2 failed"));
    }

    #[test]
    fn sort_order_severity_then_category() {
        let mut snap = empty_snapshot();
        // Warning: pending pod
        snap.pods.push(PodInfo {
            name: "pending-1".into(),
            namespace: "default".into(),
            status: "Pending".into(),
            created_at: Some(chrono::Utc::now() - chrono::Duration::minutes(10)),
            ..Default::default()
        });
        // Error: failed pod
        snap.pods.push(PodInfo {
            name: "fail-1".into(),
            namespace: "default".into(),
            status: "Failed".into(),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert!(issues.len() >= 2);
        // Error should come before Warning
        assert_eq!(issues[0].severity, AlertSeverity::Error);
    }

    #[test]
    fn capped_at_max() {
        let mut snap = empty_snapshot();
        for i in 0..600 {
            snap.pods.push(PodInfo {
                name: format!("fail-{i}"),
                namespace: "default".into(),
                status: "Failed".into(),
                ..Default::default()
            });
        }
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), MAX_ISSUES);
    }

    #[test]
    fn healthy_cluster_no_issues() {
        let mut snap = empty_snapshot();
        snap.nodes.push(NodeInfo {
            name: "node-1".into(),
            ready: true,
            ..Default::default()
        });
        snap.pods.push(PodInfo {
            name: "web-0".into(),
            namespace: "default".into(),
            status: "Running".into(),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert!(issues.is_empty());
    }

    #[test]
    fn cache_returns_same_arc() {
        let snap = empty_snapshot();
        let a = compute_issues(&snap);
        let b = compute_issues(&snap);
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn totally_empty_snapshot_no_issues() {
        let snap = empty_snapshot();
        let issues = detect_issues(&snap);
        assert!(issues.is_empty());
    }

    #[test]
    fn err_image_pull_detected() {
        let mut snap = empty_snapshot();
        snap.pods.push(PodInfo {
            name: "api-1".into(),
            namespace: "default".into(),
            waiting_reasons: vec!["ErrImagePull".into()],
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::ImagePullFailure);
        assert!(issues[0].message.contains("ErrImagePull"));
    }

    #[test]
    fn create_container_config_error_detected() {
        let mut snap = empty_snapshot();
        snap.pods.push(PodInfo {
            name: "api-2".into(),
            namespace: "default".into(),
            waiting_reasons: vec!["CreateContainerConfigError".into()],
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::ImagePullFailure);
        assert!(issues[0].message.contains("CreateContainerConfigError"));
    }

    #[test]
    fn pvc_lost_is_error_severity() {
        let mut snap = empty_snapshot();
        snap.pvcs.push(PvcInfo {
            name: "data-lost".into(),
            namespace: "default".into(),
            status: "Lost".into(),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::StorageIssue);
        assert_eq!(issues[0].severity, AlertSeverity::Error);
        assert!(issues[0].message.contains("Lost"));
    }

    #[test]
    fn pv_released_is_warning_severity() {
        let mut snap = empty_snapshot();
        snap.pvs.push(PvInfo {
            name: "pv-released".into(),
            status: "Released".into(),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::StorageIssue);
        assert_eq!(issues[0].severity, AlertSeverity::Warning);
        assert!(issues[0].message.contains("Released"));
    }

    #[test]
    fn zero_replica_deployment_not_flagged() {
        let mut snap = empty_snapshot();
        snap.deployments.push(DeploymentInfo {
            name: "scaled-down".into(),
            namespace: "default".into(),
            desired_replicas: 0,
            ready_replicas: 0,
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert!(issues.is_empty());
    }

    #[test]
    fn zero_replica_statefulset_not_flagged() {
        let mut snap = empty_snapshot();
        snap.statefulsets.push(StatefulSetInfo {
            name: "scaled-down-sts".into(),
            namespace: "default".into(),
            desired_replicas: 0,
            ready_replicas: 0,
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert!(issues.is_empty());
    }

    #[test]
    fn crashloop_pod_not_double_counted_as_pending() {
        let mut snap = empty_snapshot();
        snap.pods.push(PodInfo {
            name: "crash-pending".into(),
            namespace: "default".into(),
            status: "Pending".into(),
            waiting_reasons: vec!["CrashLoopBackOff".into()],
            created_at: Some(chrono::Utc::now() - chrono::Duration::minutes(10)),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::CrashLoopBackOff);
    }

    #[test]
    fn crashloop_pod_not_double_counted_as_failed() {
        let mut snap = empty_snapshot();
        snap.pods.push(PodInfo {
            name: "crash-failed".into(),
            namespace: "default".into(),
            status: "Failed".into(),
            waiting_reasons: vec!["CrashLoopBackOff".into()],
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::CrashLoopBackOff);
    }

    #[test]
    fn image_pull_pod_not_double_counted_as_pending() {
        let mut snap = empty_snapshot();
        snap.pods.push(PodInfo {
            name: "img-pending".into(),
            namespace: "default".into(),
            status: "Pending".into(),
            waiting_reasons: vec!["ImagePullBackOff".into()],
            created_at: Some(chrono::Utc::now() - chrono::Duration::minutes(10)),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::ImagePullFailure);
    }

    #[test]
    fn flux_message_none_uses_status_fallback() {
        let mut snap = empty_snapshot();
        snap.flux_resources.push(FluxResourceInfo {
            name: "no-msg".into(),
            namespace: Some("flux-system".into()),
            kind: "Kustomization".into(),
            group: "kustomize.toolkit.fluxcd.io".into(),
            version: "v1".into(),
            plural: "kustomizations".into(),
            status: "False".into(),
            suspended: false,
            message: None,
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("Status: False"));
    }

    #[test]
    fn service_no_endpoint_object_detected_when_endpoints_loaded() {
        let mut snap = empty_snapshot();
        snap.loaded_scope = RefreshScope::NETWORK;
        snap.services.push(ServiceInfo {
            name: "orphan-svc".into(),
            namespace: "default".into(),
            type_: "ClusterIP".into(),
            ..Default::default()
        });
        // No matching endpoint object at all
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, IssueCategory::ServiceNoEndpoints);
        assert!(issues[0].message.contains("No Endpoints object"));
    }

    #[test]
    fn service_no_endpoint_object_not_flagged_when_endpoints_not_loaded() {
        let mut snap = empty_snapshot();
        snap.loaded_scope = RefreshScope::NONE;
        // No endpoints loaded at all — don't flag missing endpoint objects
        snap.services.push(ServiceInfo {
            name: "orphan-svc".into(),
            namespace: "default".into(),
            type_: "ClusterIP".into(),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert!(issues.is_empty());
    }

    #[test]
    fn service_with_ready_endpoints_not_flagged() {
        let mut snap = empty_snapshot();
        snap.services.push(ServiceInfo {
            name: "healthy-svc".into(),
            namespace: "default".into(),
            type_: "ClusterIP".into(),
            ..Default::default()
        });
        snap.endpoints.push(EndpointInfo {
            name: "healthy-svc".into(),
            namespace: "default".into(),
            addresses: vec!["10.0.0.1".into()],
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert!(issues.is_empty());
    }

    #[test]
    fn cache_invalidates_on_version_change() {
        let mut snap = empty_snapshot();
        snap.snapshot_version = 9999;
        let a = compute_issues(&snap);

        snap.snapshot_version = 10000;
        snap.pods.push(PodInfo {
            name: "fail".into(),
            namespace: "default".into(),
            status: "Failed".into(),
            ..Default::default()
        });
        let b = compute_issues(&snap);

        assert!(!Arc::ptr_eq(&a, &b));
        assert!(a.is_empty());
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn node_pid_pressure_detected() {
        let mut snap = empty_snapshot();
        snap.nodes.push(NodeInfo {
            name: "node-pid".into(),
            ready: true,
            pid_pressure: true,
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("PID"));
    }

    #[test]
    fn node_network_unavailable_detected() {
        let mut snap = empty_snapshot();
        snap.nodes.push(NodeInfo {
            name: "node-net".into(),
            ready: true,
            network_unavailable: true,
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("Network"));
    }

    #[test]
    fn matches_query_method() {
        let issue = ClusterIssue {
            severity: AlertSeverity::Error,
            category: IssueCategory::CrashLoopBackOff,
            resource_kind: "Pod",
            resource_name: "web-0".into(),
            namespace: "production".into(),
            message: "Container in CrashLoopBackOff".into(),
            resource_ref: ResourceRef::Pod("web-0".into(), "production".into()),
        };
        assert!(issue.matches_query("web"));
        assert!(issue.matches_query("prod"));
        assert!(issue.matches_query("CrashLoop"));
        assert!(issue.matches_query("Pod"));
        assert!(issue.matches_query("Container"));
        assert!(!issue.matches_query("nonexistent"));
    }

    #[test]
    fn filtered_issue_indices_empty_query_returns_all() {
        let mut snap = empty_snapshot();
        snap.pods.push(PodInfo {
            name: "fail-1".into(),
            namespace: "default".into(),
            status: "Failed".into(),
            ..Default::default()
        });
        snap.pods.push(PodInfo {
            name: "fail-2".into(),
            namespace: "default".into(),
            status: "Failed".into(),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        let indices = filtered_issue_indices(&issues, "");
        assert_eq!(indices.len(), issues.len());
    }

    #[test]
    fn filtered_issue_indices_filters_correctly() {
        let mut snap = empty_snapshot();
        snap.pods.push(PodInfo {
            name: "web-fail".into(),
            namespace: "default".into(),
            status: "Failed".into(),
            ..Default::default()
        });
        snap.nodes.push(NodeInfo {
            name: "node-bad".into(),
            ready: false,
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        let indices = filtered_issue_indices(&issues, "node");
        assert_eq!(indices.len(), 1);
        assert_eq!(issues[indices[0]].resource_kind, "Node");
    }
}
