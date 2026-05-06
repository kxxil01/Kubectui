//! Cluster issue detection and caching.
//!
//! Computes per-resource issues from snapshot data (no new API calls).
//! Results are cached by `snapshot_version` and reused across frames.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use crate::app::ResourceRef;
use crate::k8s::dtos::{AlertSeverity, PodInfo, ServiceInfo};
use crate::k8s::selectors::{selector_is_empty, selector_matches_map};
use crate::state::vulnerabilities::compute_vulnerability_findings;
use crate::state::{ClusterSnapshot, RefreshScope};
use crate::time::{age_seconds_since, now_unix_seconds};
use crate::ui::contains_ci;

const MAX_ISSUES: usize = 500;
const SANITIZER_IGNORE_ANNOTATION: &str = "kubectui.io/ignore";

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
    MissingResourceRequirements,
    MissingProbes,
    SecurityContextGap,
    LatestImageTag,
    MissingPodDisruptionBudget,
    NakedPod,
    ServicePortMismatch,
    UnusedConfigMap,
    UnusedSecret,
    VulnerabilityExposure,
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
            Self::MissingResourceRequirements => "Missing Resources",
            Self::MissingProbes => "Missing Probes",
            Self::SecurityContextGap => "Security Context",
            Self::LatestImageTag => "Image Tag",
            Self::MissingPodDisruptionBudget => "Missing PDB",
            Self::NakedPod => "Naked Pod",
            Self::ServicePortMismatch => "Service Port Mismatch",
            Self::UnusedConfigMap => "Unused ConfigMap",
            Self::UnusedSecret => "Unused Secret",
            Self::VulnerabilityExposure => "Vulnerability Exposure",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ClusterIssueSource {
    Runtime,
    Sanitizer,
    Security,
}

impl ClusterIssueSource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Runtime => "Runtime",
            Self::Sanitizer => "Sanitizer",
            Self::Security => "Security",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClusterIssue {
    pub source: ClusterIssueSource,
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
        contains_ci(self.source.label(), query)
            || contains_ci(self.category.label(), query)
            || contains_ci(self.resource_kind, query)
            || contains_ci(&self.resource_name, query)
            || contains_ci(&self.namespace, query)
            || contains_ci(&self.message, query)
    }
}

pub fn sanitizer_issue_count(snapshot: &ClusterSnapshot) -> usize {
    compute_issues(snapshot)
        .iter()
        .filter(|issue| issue.source == ClusterIssueSource::Sanitizer)
        .count()
}

fn ignored_rule_names(annotations: &[(String, String)]) -> BTreeSet<String> {
    annotations
        .iter()
        .find(|(key, _)| key == SANITIZER_IGNORE_ANNOTATION)
        .map(|(_, value)| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

fn rule_ignored(annotations: &[(String, String)], rule_name: &str) -> bool {
    ignored_rule_names(annotations).contains(&rule_name.to_ascii_lowercase())
}

/// Returns filtered issue indices matching the search query.
/// Used by both the render path and the `selected_resource` action path.
pub fn filtered_issue_indices(issues: &[ClusterIssue], query: &str) -> Vec<usize> {
    filtered_issue_indices_by_source(issues, query, None)
}

pub fn filtered_issue_indices_by_source(
    issues: &[ClusterIssue],
    query: &str,
    source: Option<ClusterIssueSource>,
) -> Vec<usize> {
    if query.is_empty() {
        issues
            .iter()
            .enumerate()
            .filter_map(|(idx, issue)| {
                source
                    .map(|expected| issue.source == expected)
                    .unwrap_or(true)
                    .then_some(idx)
            })
            .collect()
    } else {
        issues
            .iter()
            .enumerate()
            .filter_map(|(idx, issue)| {
                let source_matches = source
                    .map(|expected| issue.source == expected)
                    .unwrap_or(true);
                (source_matches && issue.matches_query(query)).then_some(idx)
            })
            .collect()
    }
}

#[allow(clippy::type_complexity)]
static ISSUE_CACHE: LazyLock<Mutex<Option<((u64, usize), Arc<Vec<ClusterIssue>>)>>> =
    LazyLock::new(|| Mutex::new(None));

/// Returns cached issues for the given snapshot, computing on first call per version.
pub fn compute_issues(snapshot: &ClusterSnapshot) -> Arc<Vec<ClusterIssue>> {
    let key = (
        snapshot.snapshot_version,
        std::ptr::from_ref(snapshot) as usize,
    );
    {
        let guard = ISSUE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached_key, ref issues)) = *guard
            && cached_key == key
        {
            return Arc::clone(issues);
        }
    }

    let issues = Arc::new(detect_issues(snapshot));
    {
        let mut guard = ISSUE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some((key, Arc::clone(&issues)));
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
                source: ClusterIssueSource::Runtime,
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
                source: ClusterIssueSource::Runtime,
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
                .map(|t| age_seconds_since(t, now_unix_seconds()) as u64)
                .unwrap_or(0);
            let severity = if age_secs > 300 {
                AlertSeverity::Warning
            } else {
                AlertSeverity::Info
            };
            issues.push(ClusterIssue {
                source: ClusterIssueSource::Runtime,
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
                source: ClusterIssueSource::Runtime,
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
                source: ClusterIssueSource::Runtime,
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
                source: ClusterIssueSource::Runtime,
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
                source: ClusterIssueSource::Runtime,
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
                source: ClusterIssueSource::Runtime,
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
                source: ClusterIssueSource::Runtime,
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
                source: ClusterIssueSource::Runtime,
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
                source: ClusterIssueSource::Runtime,
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
            source: ClusterIssueSource::Runtime,
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
                    source: ClusterIssueSource::Runtime,
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
                    source: ClusterIssueSource::Runtime,
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
                source: ClusterIssueSource::Runtime,
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

    detect_sanitizer_findings(snapshot, &mut issues);
    detect_vulnerability_findings(snapshot, &mut issues);

    // Sort: severity rank (Error first), then source, then category, namespace, kind, and name.
    issues.sort_unstable_by(|a, b| {
        severity_rank(a.severity)
            .cmp(&severity_rank(b.severity))
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.category.cmp(&b.category))
            .then_with(|| a.namespace.cmp(&b.namespace))
            .then_with(|| a.resource_kind.cmp(b.resource_kind))
            .then_with(|| a.resource_name.cmp(&b.resource_name))
    });

    issues.truncate(MAX_ISSUES);
    issues
}

fn detect_sanitizer_findings(snapshot: &ClusterSnapshot, issues: &mut Vec<ClusterIssue>) {
    detect_pod_sanitizer_findings(snapshot, issues);
    detect_deployment_pdb_gaps(snapshot, issues);
    detect_service_port_mismatches(snapshot, issues);
    detect_unused_config_maps(snapshot, issues);
    detect_unused_secrets(snapshot, issues);
}

fn detect_vulnerability_findings(snapshot: &ClusterSnapshot, issues: &mut Vec<ClusterIssue>) {
    let findings = compute_vulnerability_findings(snapshot);
    for finding in findings.iter() {
        let Some(resource_ref) = finding.resource_ref.clone() else {
            continue;
        };
        if finding.counts.total() == 0 {
            continue;
        }
        let message = if finding.fixable_count > 0 {
            format!(
                "{} total vulnerabilities (critical {}, high {}, medium {}, low {}, unknown {}), {} fixable",
                finding.counts.total(),
                finding.counts.critical,
                finding.counts.high,
                finding.counts.medium,
                finding.counts.low,
                finding.counts.unknown,
                finding.fixable_count,
            )
        } else {
            format!(
                "{} total vulnerabilities (critical {}, high {}, medium {}, low {}, unknown {})",
                finding.counts.total(),
                finding.counts.critical,
                finding.counts.high,
                finding.counts.medium,
                finding.counts.low,
                finding.counts.unknown,
            )
        };
        let resource_kind = issue_resource_kind(&resource_ref);
        issues.push(ClusterIssue {
            source: ClusterIssueSource::Security,
            severity: finding.severity,
            category: IssueCategory::VulnerabilityExposure,
            resource_kind,
            resource_name: finding.resource_name.clone(),
            namespace: finding.namespace.clone(),
            message,
            resource_ref,
        });
    }
}

fn issue_resource_kind(resource_ref: &ResourceRef) -> &'static str {
    match resource_ref {
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
        ResourceRef::CustomResource { .. } => "CustomResource",
    }
}

fn detect_pod_sanitizer_findings(snapshot: &ClusterSnapshot, issues: &mut Vec<ClusterIssue>) {
    for pod in &snapshot.pods {
        let resource_ref = ResourceRef::Pod(pod.name.clone(), pod.namespace.clone());
        let ignored_rules = ignored_rule_names(&pod.annotations);

        if !ignored_rules.contains("missing-resources")
            && (pod.cpu_request.is_none()
                || pod.memory_request.is_none()
                || pod.cpu_limit.is_none()
                || pod.memory_limit.is_none())
        {
            let mut missing = Vec::new();
            if pod.cpu_request.is_none() {
                missing.push("cpu request");
            }
            if pod.memory_request.is_none() {
                missing.push("memory request");
            }
            if pod.cpu_limit.is_none() {
                missing.push("cpu limit");
            }
            if pod.memory_limit.is_none() {
                missing.push("memory limit");
            }
            issues.push(ClusterIssue {
                source: ClusterIssueSource::Sanitizer,
                severity: AlertSeverity::Warning,
                category: IssueCategory::MissingResourceRequirements,
                resource_kind: "Pod",
                resource_name: pod.name.clone(),
                namespace: pod.namespace.clone(),
                message: format!("Missing {}.", missing.join(", ")),
                resource_ref: resource_ref.clone(),
            });
        }

        if !ignored_rules.contains("missing-probes")
            && (pod.missing_liveness_probes > 0 || pod.missing_readiness_probes > 0)
        {
            let mut gaps = Vec::new();
            if pod.missing_liveness_probes > 0 {
                gaps.push(format!(
                    "{} container(s) missing liveness probes",
                    pod.missing_liveness_probes
                ));
            }
            if pod.missing_readiness_probes > 0 {
                gaps.push(format!(
                    "{} container(s) missing readiness probes",
                    pod.missing_readiness_probes
                ));
            }
            issues.push(ClusterIssue {
                source: ClusterIssueSource::Sanitizer,
                severity: AlertSeverity::Warning,
                category: IssueCategory::MissingProbes,
                resource_kind: "Pod",
                resource_name: pod.name.clone(),
                namespace: pod.namespace.clone(),
                message: gaps.join("; "),
                resource_ref: resource_ref.clone(),
            });
        }

        if !ignored_rules.contains("security-context")
            && (!pod.run_as_non_root_configured || pod.host_network || pod.host_pid || pod.host_ipc)
        {
            let mut gaps = Vec::new();
            if !pod.run_as_non_root_configured {
                gaps.push("runAsNonRoot is not enforced".to_string());
            }
            if pod.host_network {
                gaps.push("hostNetwork enabled".to_string());
            }
            if pod.host_pid {
                gaps.push("hostPID enabled".to_string());
            }
            if pod.host_ipc {
                gaps.push("hostIPC enabled".to_string());
            }
            let severity = if pod.host_network || pod.host_pid || pod.host_ipc {
                AlertSeverity::Error
            } else {
                AlertSeverity::Warning
            };
            issues.push(ClusterIssue {
                source: ClusterIssueSource::Sanitizer,
                severity,
                category: IssueCategory::SecurityContextGap,
                resource_kind: "Pod",
                resource_name: pod.name.clone(),
                namespace: pod.namespace.clone(),
                message: gaps.join("; "),
                resource_ref: resource_ref.clone(),
            });
        }

        if !ignored_rules.contains("latest-tag") {
            let drifting_images = pod
                .container_images
                .iter()
                .filter(|image| image_uses_unstable_tag(image))
                .cloned()
                .collect::<Vec<_>>();
            if !drifting_images.is_empty() {
                issues.push(ClusterIssue {
                    source: ClusterIssueSource::Sanitizer,
                    severity: AlertSeverity::Warning,
                    category: IssueCategory::LatestImageTag,
                    resource_kind: "Pod",
                    resource_name: pod.name.clone(),
                    namespace: pod.namespace.clone(),
                    message: format!(
                        "Unstable image reference(s): {}",
                        drifting_images.join(", ")
                    ),
                    resource_ref: resource_ref.clone(),
                });
            }
        }

        if !ignored_rules.contains("naked-pod") && pod.owner_references.is_empty() {
            issues.push(ClusterIssue {
                source: ClusterIssueSource::Sanitizer,
                severity: AlertSeverity::Warning,
                category: IssueCategory::NakedPod,
                resource_kind: "Pod",
                resource_name: pod.name.clone(),
                namespace: pod.namespace.clone(),
                message: "Pod has no owning controller.".to_string(),
                resource_ref,
            });
        }
    }
}

fn detect_deployment_pdb_gaps(snapshot: &ClusterSnapshot, issues: &mut Vec<ClusterIssue>) {
    for deployment in &snapshot.deployments {
        if deployment.desired_replicas <= 1
            || selector_is_empty(&deployment.selector)
            || rule_ignored(&deployment.annotations, "missing-pdb")
        {
            continue;
        }

        let deployment_labels = if deployment.pod_template_labels.is_empty() {
            &deployment.selector.match_labels
        } else {
            &deployment.pod_template_labels
        };
        let covered = snapshot.pod_disruption_budgets.iter().any(|pdb| {
            pdb.namespace == deployment.namespace
                && pdb
                    .selector
                    .as_ref()
                    .is_some_and(|selector| selector_matches_map(selector, deployment_labels))
        });
        if covered {
            continue;
        }

        issues.push(ClusterIssue {
            source: ClusterIssueSource::Sanitizer,
            severity: AlertSeverity::Warning,
            category: IssueCategory::MissingPodDisruptionBudget,
            resource_kind: "Deployment",
            resource_name: deployment.name.clone(),
            namespace: deployment.namespace.clone(),
            message: format!(
                "Deployment has {} desired replicas but no matching PodDisruptionBudget.",
                deployment.desired_replicas
            ),
            resource_ref: ResourceRef::Deployment(
                deployment.name.clone(),
                deployment.namespace.clone(),
            ),
        });
    }
}

fn collect_used_config_map_refs(snapshot: &ClusterSnapshot) -> BTreeSet<(String, String)> {
    let mut used = BTreeSet::new();
    for pod in &snapshot.pods {
        for name in &pod.referenced_config_maps {
            used.insert((pod.namespace.clone(), name.clone()));
        }
    }
    for workload in &snapshot.deployments {
        for name in &workload.referenced_config_maps {
            used.insert((workload.namespace.clone(), name.clone()));
        }
    }
    for workload in &snapshot.statefulsets {
        for name in &workload.referenced_config_maps {
            used.insert((workload.namespace.clone(), name.clone()));
        }
    }
    for workload in &snapshot.daemonsets {
        for name in &workload.referenced_config_maps {
            used.insert((workload.namespace.clone(), name.clone()));
        }
    }
    for workload in &snapshot.replicasets {
        for name in &workload.referenced_config_maps {
            used.insert((workload.namespace.clone(), name.clone()));
        }
    }
    for workload in &snapshot.replication_controllers {
        for name in &workload.referenced_config_maps {
            used.insert((workload.namespace.clone(), name.clone()));
        }
    }
    for workload in &snapshot.jobs {
        for name in &workload.referenced_config_maps {
            used.insert((workload.namespace.clone(), name.clone()));
        }
    }
    for workload in &snapshot.cronjobs {
        for name in &workload.referenced_config_maps {
            used.insert((workload.namespace.clone(), name.clone()));
        }
    }
    used
}

fn collect_used_secret_refs(snapshot: &ClusterSnapshot) -> BTreeSet<(String, String)> {
    let mut used = BTreeSet::new();
    for pod in &snapshot.pods {
        for name in &pod.referenced_secrets {
            used.insert((pod.namespace.clone(), name.clone()));
        }
    }
    for workload in &snapshot.deployments {
        for name in &workload.referenced_secrets {
            used.insert((workload.namespace.clone(), name.clone()));
        }
    }
    for workload in &snapshot.statefulsets {
        for name in &workload.referenced_secrets {
            used.insert((workload.namespace.clone(), name.clone()));
        }
    }
    for workload in &snapshot.daemonsets {
        for name in &workload.referenced_secrets {
            used.insert((workload.namespace.clone(), name.clone()));
        }
    }
    for workload in &snapshot.replicasets {
        for name in &workload.referenced_secrets {
            used.insert((workload.namespace.clone(), name.clone()));
        }
    }
    for workload in &snapshot.replication_controllers {
        for name in &workload.referenced_secrets {
            used.insert((workload.namespace.clone(), name.clone()));
        }
    }
    for workload in &snapshot.jobs {
        for name in &workload.referenced_secrets {
            used.insert((workload.namespace.clone(), name.clone()));
        }
    }
    for workload in &snapshot.cronjobs {
        for name in &workload.referenced_secrets {
            used.insert((workload.namespace.clone(), name.clone()));
        }
    }
    for service_account in &snapshot.service_accounts {
        for name in &service_account.secret_names {
            used.insert((service_account.namespace.clone(), name.clone()));
        }
        for name in &service_account.image_pull_secret_names {
            used.insert((service_account.namespace.clone(), name.clone()));
        }
    }
    used
}

fn detect_service_port_mismatches(snapshot: &ClusterSnapshot, issues: &mut Vec<ClusterIssue>) {
    for service in &snapshot.services {
        if service.selector.is_empty()
            || rule_ignored(&service.annotations, "service-port-mismatch")
        {
            continue;
        }

        if !snapshot
            .pods
            .iter()
            .any(|pod| service_matches_pod_selector(service, pod))
        {
            continue;
        }

        let mismatched_ports = service
            .port_mappings
            .iter()
            .filter(|port| {
                let protocol = port.protocol.as_str();
                snapshot
                    .pods
                    .iter()
                    .filter(|pod| service_matches_pod_selector(service, pod))
                    .all(|pod| {
                        pod.container_ports.iter().all(|container_port| {
                            if container_port.protocol != protocol {
                                return true;
                            }
                            match (port.target_port_number, port.target_port_name.as_deref()) {
                                (Some(number), _) => container_port.container_port != number,
                                (None, Some(name)) => container_port.name.as_deref() != Some(name),
                                (None, None) => container_port.container_port != port.port,
                            }
                        })
                    })
            })
            .map(|port| port.port.to_string())
            .collect::<Vec<_>>();

        if mismatched_ports.is_empty() {
            continue;
        }

        issues.push(ClusterIssue {
            source: ClusterIssueSource::Sanitizer,
            severity: AlertSeverity::Warning,
            category: IssueCategory::ServicePortMismatch,
            resource_kind: "Service",
            resource_name: service.name.clone(),
            namespace: service.namespace.clone(),
            message: format!(
                "No matching container port found for Service port(s): {}.",
                mismatched_ports.join(", ")
            ),
            resource_ref: ResourceRef::Service(service.name.clone(), service.namespace.clone()),
        });
    }
}

fn service_matches_pod_selector(service: &ServiceInfo, pod: &PodInfo) -> bool {
    pod.namespace == service.namespace
        && service.selector.iter().all(|(key, expected)| {
            pod.labels
                .iter()
                .any(|(label_key, label_value)| label_key == key && label_value == expected)
        })
}

fn detect_unused_config_maps(snapshot: &ClusterSnapshot, issues: &mut Vec<ClusterIssue>) {
    let used = collect_used_config_map_refs(snapshot);

    for config_map in &snapshot.config_maps {
        if config_map.name == "kube-root-ca.crt"
            || rule_ignored(&config_map.annotations, "unused-configmap")
        {
            continue;
        }
        if used.contains(&(config_map.namespace.clone(), config_map.name.clone())) {
            continue;
        }
        issues.push(ClusterIssue {
            source: ClusterIssueSource::Sanitizer,
            severity: AlertSeverity::Info,
            category: IssueCategory::UnusedConfigMap,
            resource_kind: "ConfigMap",
            resource_name: config_map.name.clone(),
            namespace: config_map.namespace.clone(),
            message: "ConfigMap is not referenced by any current Pod or workload template."
                .to_string(),
            resource_ref: ResourceRef::ConfigMap(
                config_map.name.clone(),
                config_map.namespace.clone(),
            ),
        });
    }
}

fn detect_unused_secrets(snapshot: &ClusterSnapshot, issues: &mut Vec<ClusterIssue>) {
    let used = collect_used_secret_refs(snapshot);

    for secret in &snapshot.secrets {
        if matches!(
            secret.type_.as_str(),
            "kubernetes.io/service-account-token" | "helm.sh/release.v1"
        ) || rule_ignored(&secret.annotations, "unused-secret")
        {
            continue;
        }
        if used.contains(&(secret.namespace.clone(), secret.name.clone())) {
            continue;
        }
        issues.push(ClusterIssue {
            source: ClusterIssueSource::Sanitizer,
            severity: AlertSeverity::Info,
            category: IssueCategory::UnusedSecret,
            resource_kind: "Secret",
            resource_name: secret.name.clone(),
            namespace: secret.namespace.clone(),
            message: "Secret is not referenced by any current Pod or workload template."
                .to_string(),
            resource_ref: ResourceRef::Secret(secret.name.clone(), secret.namespace.clone()),
        });
    }
}

fn image_uses_unstable_tag(image: &str) -> bool {
    let image_without_digest = image.split('@').next().unwrap_or(image);
    match image_without_digest.rsplit_once(':') {
        None => true,
        Some((_, tag)) => tag.eq_ignore_ascii_case("latest"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use jiff::ToSpan;

    use super::*;
    use crate::k8s::dtos::*;
    use crate::state::ClusterSnapshot;
    use crate::time::now;

    fn empty_snapshot() -> ClusterSnapshot {
        ClusterSnapshot::default()
    }

    fn runtime_issues(issues: &[ClusterIssue]) -> Vec<&ClusterIssue> {
        issues
            .iter()
            .filter(|issue| issue.source == ClusterIssueSource::Runtime)
            .collect()
    }

    fn sanitizer_issues(issues: &[ClusterIssue]) -> Vec<&ClusterIssue> {
        issues
            .iter()
            .filter(|issue| issue.source == ClusterIssueSource::Sanitizer)
            .collect()
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
        let runtime = runtime_issues(&issues);
        assert_eq!(runtime.len(), 1);
        assert_eq!(runtime[0].category, IssueCategory::CrashLoopBackOff);
        assert_eq!(runtime[0].severity, AlertSeverity::Error);
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
        let runtime = runtime_issues(&issues);
        assert_eq!(runtime.len(), 1);
        assert_eq!(runtime[0].category, IssueCategory::ImagePullFailure);
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
        let runtime = runtime_issues(&issues);
        assert_eq!(runtime.len(), 1);
        assert_eq!(runtime[0].category, IssueCategory::CrashLoopBackOff);
    }

    #[test]
    fn issues_tie_break_by_namespace_before_name() {
        let mut snap = empty_snapshot();
        for namespace in ["ns-b", "ns-a"] {
            snap.jobs.push(JobInfo {
                name: "daily".into(),
                namespace: namespace.into(),
                status: "Failed".into(),
                failed_pods: 1,
                ..Default::default()
            });
        }

        let issues = detect_issues(&snap);
        let failed_jobs = issues
            .iter()
            .filter(|issue| issue.category == IssueCategory::FailedJob)
            .collect::<Vec<_>>();

        assert_eq!(failed_jobs.len(), 2);
        assert_eq!(failed_jobs[0].namespace, "ns-a");
        assert_eq!(failed_jobs[1].namespace, "ns-b");
    }

    #[test]
    fn pending_pod_severity_by_age() {
        let mut snap = empty_snapshot();
        let baseline = now();
        // Recent pending pod → Info
        snap.pods.push(PodInfo {
            name: "new-pod".into(),
            namespace: "default".into(),
            status: "Pending".into(),
            created_at: Some(baseline),
            ..Default::default()
        });
        // Old pending pod → Warning
        snap.pods.push(PodInfo {
            name: "old-pod".into(),
            namespace: "default".into(),
            status: "Pending".into(),
            created_at: Some(
                baseline
                    .checked_sub(10.minutes())
                    .expect("timestamp in range"),
            ),
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
        let runtime = runtime_issues(&issues);
        assert_eq!(runtime.len(), 1);
        assert_eq!(runtime[0].category, IssueCategory::FailedPod);
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
        let baseline = now();
        // Warning: pending pod
        snap.pods.push(PodInfo {
            name: "pending-1".into(),
            namespace: "default".into(),
            status: "Pending".into(),
            created_at: Some(
                baseline
                    .checked_sub(10.minutes())
                    .expect("timestamp in range"),
            ),
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
            cpu_request: Some("100m".into()),
            memory_request: Some("128Mi".into()),
            cpu_limit: Some("500m".into()),
            memory_limit: Some("256Mi".into()),
            container_images: vec!["nginx:1.29.0".into()],
            run_as_non_root_configured: true,
            owner_references: vec![OwnerRefInfo {
                kind: "ReplicaSet".into(),
                name: "web".into(),
                uid: "uid-1".into(),
            }],
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
        let runtime = runtime_issues(&issues);
        assert_eq!(runtime.len(), 1);
        assert_eq!(runtime[0].category, IssueCategory::ImagePullFailure);
        assert!(runtime[0].message.contains("ErrImagePull"));
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
        let runtime = runtime_issues(&issues);
        assert_eq!(runtime.len(), 1);
        assert_eq!(runtime[0].category, IssueCategory::ImagePullFailure);
        assert!(runtime[0].message.contains("CreateContainerConfigError"));
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
            created_at: Some(now().checked_sub(10.minutes()).expect("timestamp in range")),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        let runtime = runtime_issues(&issues);
        assert_eq!(runtime.len(), 1);
        assert_eq!(runtime[0].category, IssueCategory::CrashLoopBackOff);
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
        let runtime = runtime_issues(&issues);
        assert_eq!(runtime.len(), 1);
        assert_eq!(runtime[0].category, IssueCategory::CrashLoopBackOff);
    }

    #[test]
    fn image_pull_pod_not_double_counted_as_pending() {
        let mut snap = empty_snapshot();
        snap.pods.push(PodInfo {
            name: "img-pending".into(),
            namespace: "default".into(),
            status: "Pending".into(),
            waiting_reasons: vec!["ImagePullBackOff".into()],
            created_at: Some(now().checked_sub(10.minutes()).expect("timestamp in range")),
            ..Default::default()
        });
        let issues = detect_issues(&snap);
        let runtime = runtime_issues(&issues);
        assert_eq!(runtime.len(), 1);
        assert_eq!(runtime[0].category, IssueCategory::ImagePullFailure);
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
        assert_eq!(runtime_issues(&b).len(), 1);
    }

    #[test]
    fn sanitizer_flags_missing_resources_and_probes_for_running_pod() {
        let mut snap = empty_snapshot();
        snap.pods.push(PodInfo {
            name: "lint-me".into(),
            namespace: "default".into(),
            status: "Running".into(),
            container_images: vec!["repo/app:1.0.0".into()],
            missing_liveness_probes: 1,
            missing_readiness_probes: 1,
            owner_references: vec![OwnerRefInfo {
                kind: "ReplicaSet".into(),
                name: "lint-me-rs".into(),
                uid: "uid-2".into(),
            }],
            ..Default::default()
        });

        let issues = detect_issues(&snap);
        let sanitizer = sanitizer_issues(&issues);

        assert!(
            sanitizer
                .iter()
                .any(|issue| issue.category == IssueCategory::MissingResourceRequirements)
        );
        assert!(
            sanitizer
                .iter()
                .any(|issue| issue.category == IssueCategory::MissingProbes)
        );
    }

    #[test]
    fn sanitizer_honors_ignore_annotation() {
        let mut snap = empty_snapshot();
        snap.pods.push(PodInfo {
            name: "ignored".into(),
            namespace: "default".into(),
            status: "Running".into(),
            annotations: vec![(
                SANITIZER_IGNORE_ANNOTATION.to_string(),
                "missing-resources,missing-probes,security-context,latest-tag,naked-pod"
                    .to_string(),
            )],
            ..Default::default()
        });

        let issues = detect_issues(&snap);
        assert!(sanitizer_issues(&issues).is_empty());
    }

    #[test]
    fn sanitizer_detects_missing_pdb_for_scaled_deployment() {
        let mut snap = empty_snapshot();
        snap.deployments.push(DeploymentInfo {
            name: "web".into(),
            namespace: "default".into(),
            desired_replicas: 3,
            selector: LabelSelectorInfo {
                match_labels: BTreeMap::from([("app".into(), "web".into())]),
                match_expressions: Vec::new(),
            },
            ..Default::default()
        });

        let issues = detect_issues(&snap);
        assert!(sanitizer_issues(&issues).iter().any(|issue| {
            issue.category == IssueCategory::MissingPodDisruptionBudget
                && issue.resource_name == "web"
        }));
    }

    #[test]
    fn sanitizer_skips_missing_pdb_when_matching_pdb_exists() {
        let mut snap = empty_snapshot();
        let selector = LabelSelectorInfo {
            match_labels: BTreeMap::from([("app".into(), "web".into())]),
            match_expressions: Vec::new(),
        };
        snap.deployments.push(DeploymentInfo {
            name: "web".into(),
            namespace: "default".into(),
            desired_replicas: 3,
            selector: selector.clone(),
            ..Default::default()
        });
        snap.pod_disruption_budgets.push(PodDisruptionBudgetInfo {
            name: "web-pdb".into(),
            namespace: "default".into(),
            selector: Some(selector),
            ..Default::default()
        });

        let issues = detect_issues(&snap);
        assert!(!sanitizer_issues(&issues).iter().any(|issue| {
            issue.category == IssueCategory::MissingPodDisruptionBudget
                && issue.resource_name == "web"
        }));
    }

    #[test]
    fn sanitizer_matches_pdb_against_pod_template_labels() {
        let mut snap = empty_snapshot();
        snap.deployments.push(DeploymentInfo {
            name: "web".into(),
            namespace: "default".into(),
            desired_replicas: 3,
            selector: LabelSelectorInfo {
                match_labels: BTreeMap::from([("app".into(), "web".into())]),
                match_expressions: Vec::new(),
            },
            pod_template_labels: BTreeMap::from([
                ("app".into(), "web".into()),
                ("tier".into(), "frontend".into()),
            ]),
            ..Default::default()
        });
        snap.pod_disruption_budgets.push(PodDisruptionBudgetInfo {
            name: "web-pdb".into(),
            namespace: "default".into(),
            selector: Some(LabelSelectorInfo {
                match_labels: BTreeMap::from([
                    ("app".into(), "web".into()),
                    ("tier".into(), "frontend".into()),
                ]),
                match_expressions: Vec::new(),
            }),
            ..Default::default()
        });

        let issues = detect_issues(&snap);
        assert!(!sanitizer_issues(&issues).iter().any(|issue| {
            issue.category == IssueCategory::MissingPodDisruptionBudget
                && issue.resource_name == "web"
        }));
    }

    #[test]
    fn sanitizer_detects_service_target_port_mismatch() {
        let mut snap = empty_snapshot();
        snap.services.push(ServiceInfo {
            name: "api".into(),
            namespace: "default".into(),
            selector: BTreeMap::from([("app".into(), "api".into())]),
            port_mappings: vec![ServicePortInfo {
                port: 80,
                protocol: "TCP".into(),
                target_port_name: None,
                target_port_number: Some(8080),
            }],
            ..Default::default()
        });
        snap.pods.push(PodInfo {
            name: "api-0".into(),
            namespace: "default".into(),
            labels: vec![("app".into(), "api".into())],
            container_ports: vec![ContainerPortInfo {
                name: Some("http".into()),
                container_port: 9090,
                protocol: "TCP".into(),
            }],
            ..Default::default()
        });

        let issues = detect_issues(&snap);
        assert!(sanitizer_issues(&issues).iter().any(|issue| {
            issue.category == IssueCategory::ServicePortMismatch && issue.resource_name == "api"
        }));
    }

    #[test]
    fn sanitizer_detects_unused_config_map_and_secret() {
        let mut snap = empty_snapshot();
        snap.config_maps.push(ConfigMapInfo {
            name: "unused-config".into(),
            namespace: "default".into(),
            ..Default::default()
        });
        snap.secrets.push(SecretInfo {
            name: "unused-secret".into(),
            namespace: "default".into(),
            type_: "Opaque".into(),
            ..Default::default()
        });

        let issues = detect_issues(&snap);
        let sanitizer = sanitizer_issues(&issues);

        assert!(
            sanitizer
                .iter()
                .any(|issue| issue.category == IssueCategory::UnusedConfigMap)
        );
        assert!(
            sanitizer
                .iter()
                .any(|issue| issue.category == IssueCategory::UnusedSecret)
        );
    }

    #[test]
    fn security_issues_include_workload_vulnerability_findings() {
        let mut snap = empty_snapshot();
        snap.snapshot_version = 7;
        snap.vulnerability_reports.push(VulnerabilityReportInfo {
            name: "api-web".into(),
            namespace: "default".into(),
            resource_kind: "Deployment".into(),
            resource_name: "api".into(),
            resource_namespace: "default".into(),
            container_name: Some("web".into()),
            counts: VulnerabilitySummaryCounts {
                critical: 1,
                high: 2,
                medium: 0,
                low: 0,
                unknown: 0,
            },
            fixable_count: 2,
            ..VulnerabilityReportInfo::default()
        });

        let issues = detect_issues(&snap);
        let issue = issues
            .iter()
            .find(|issue| issue.category == IssueCategory::VulnerabilityExposure)
            .expect("vulnerability issue should exist");
        assert_eq!(issue.source, ClusterIssueSource::Security);
        assert_eq!(issue.resource_kind, "Deployment");
        assert_eq!(issue.resource_name, "api");
        assert_eq!(issue.namespace, "default");
        assert!(issue.message.contains("3 total vulnerabilities"));
    }

    #[test]
    fn sanitizer_skips_unused_refs_when_workload_template_uses_them() {
        let mut snap = empty_snapshot();
        snap.config_maps.push(ConfigMapInfo {
            name: "used-config".into(),
            namespace: "default".into(),
            ..Default::default()
        });
        snap.secrets.push(SecretInfo {
            name: "used-secret".into(),
            namespace: "default".into(),
            type_: "Opaque".into(),
            ..Default::default()
        });
        snap.deployments.push(DeploymentInfo {
            name: "web".into(),
            namespace: "default".into(),
            referenced_config_maps: vec!["used-config".into()],
            referenced_secrets: vec!["used-secret".into()],
            ..Default::default()
        });

        let issues = detect_issues(&snap);
        assert!(!sanitizer_issues(&issues).iter().any(|issue| {
            matches!(
                issue.category,
                IssueCategory::UnusedConfigMap | IssueCategory::UnusedSecret
            )
        }));
    }

    #[test]
    fn sanitizer_skips_unused_secret_when_service_account_references_it() {
        let mut snap = empty_snapshot();
        snap.secrets.push(SecretInfo {
            name: "registry-creds".into(),
            namespace: "default".into(),
            type_: "kubernetes.io/dockerconfigjson".into(),
            ..Default::default()
        });
        snap.secrets.push(SecretInfo {
            name: "bound-token".into(),
            namespace: "default".into(),
            type_: "Opaque".into(),
            ..Default::default()
        });
        snap.service_accounts.push(ServiceAccountInfo {
            name: "builder".into(),
            namespace: "default".into(),
            image_pull_secret_names: vec!["registry-creds".into()],
            secret_names: vec!["bound-token".into()],
            ..Default::default()
        });

        let issues = detect_issues(&snap);
        assert!(!sanitizer_issues(&issues).iter().any(|issue| {
            issue.category == IssueCategory::UnusedSecret
                && matches!(
                    issue.resource_name.as_str(),
                    "registry-creds" | "bound-token"
                )
        }));
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
            source: ClusterIssueSource::Runtime,
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
