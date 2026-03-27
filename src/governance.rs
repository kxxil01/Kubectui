//! Snapshot-cached governance and cost-oriented rollups built from existing app surfaces.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, LazyLock, Mutex},
};

use crate::{
    app::ResourceRef,
    k8s::dtos::AlertSeverity,
    projects::compute_projects,
    state::{
        ClusterSnapshot,
        alerts::{compute_namespace_utilization, parse_mib, parse_millicores},
        issues::{ClusterIssueSource, IssueCategory, compute_issues},
        vulnerabilities::compute_vulnerability_findings,
    },
    ui::contains_ci,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernanceWorkloadSummary {
    pub resource_ref: ResourceRef,
    pub issue_count: usize,
    pub vulnerability_total: usize,
    pub fixable_vulnerabilities: usize,
    pub cpu_request_m: u64,
    pub mem_request_mib: u64,
    pub cpu_usage_m: u64,
    pub mem_usage_mib: u64,
    pub missing_requests: usize,
    pub missing_limits: usize,
    pub highest_severity: AlertSeverity,
    pub compact_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceGovernanceSummary {
    pub namespace: String,
    pub project_count: usize,
    pub project_count_label: String,
    pub workload_count: usize,
    pub workload_count_label: String,
    pub pod_count: usize,
    pub runtime_issue_count: usize,
    pub sanitizer_issue_count: usize,
    pub security_issue_count: usize,
    pub total_issue_count_label: String,
    pub vulnerability_total: usize,
    pub vulnerability_total_label: String,
    pub fixable_vulnerabilities: usize,
    pub quota_count: usize,
    pub limit_range_count: usize,
    pub pdb_gap_count: usize,
    pub policy_surface_count_label: String,
    pub missing_cpu_request_pods: usize,
    pub missing_mem_request_pods: usize,
    pub missing_limit_pods: usize,
    pub cpu_usage_m: u64,
    pub mem_usage_mib: u64,
    pub cpu_request_m: u64,
    pub mem_request_mib: u64,
    pub cpu_req_utilization_pct: Option<u16>,
    pub cpu_req_utilization_label: String,
    pub mem_req_utilization_pct: Option<u16>,
    pub mem_req_utilization_label: String,
    pub idle_cpu_request_m: u64,
    pub idle_mem_request_mib: u64,
    pub idle_request_label: String,
    pub highest_severity: AlertSeverity,
    pub representative: Option<ResourceRef>,
    pub projects: Vec<String>,
    pub projects_label: String,
    pub counts_summary_label: String,
    pub policy_surfaces_label: String,
    pub vulnerabilities_label: String,
    pub requests_label: String,
    pub coverage_gaps_label: String,
    pub risk_signals: Vec<String>,
    pub top_workloads: Vec<GovernanceWorkloadSummary>,
}

impl NamespaceGovernanceSummary {
    pub fn matches_query(&self, query: &str) -> bool {
        contains_ci(&self.namespace, query)
            || self
                .projects
                .iter()
                .any(|project| contains_ci(project, query))
            || self
                .risk_signals
                .iter()
                .any(|signal| contains_ci(signal, query))
            || self.top_workloads.iter().any(|workload| {
                contains_ci(workload.resource_ref.kind(), query)
                    || contains_ci(workload.resource_ref.name(), query)
            })
    }

    pub fn total_issue_count(&self) -> usize {
        self.runtime_issue_count + self.sanitizer_issue_count + self.security_issue_count
    }

    pub fn representative_label(&self) -> Option<String> {
        self.representative.as_ref().map(ResourceRef::summary_label)
    }
}

type GovernanceCache = Arc<Vec<NamespaceGovernanceSummary>>;
type GovernanceCacheKey = (u64, usize);

static GOVERNANCE_CACHE: LazyLock<Mutex<Option<(GovernanceCacheKey, GovernanceCache)>>> =
    LazyLock::new(|| Mutex::new(None));

fn namespace_fallback_representative(
    snapshot: &ClusterSnapshot,
    namespace: &str,
) -> Option<ResourceRef> {
    if namespace == "cluster" {
        return None;
    }

    snapshot
        .namespace_list
        .iter()
        .any(|entry| entry.name == namespace)
        .then(|| ResourceRef::Namespace(namespace.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct WorkloadKey {
    namespace: String,
    kind: &'static str,
    name: String,
}

#[derive(Debug, Clone)]
struct WorkloadAccumulator {
    resource_ref: ResourceRef,
    issue_count: usize,
    vulnerability_total: usize,
    fixable_vulnerabilities: usize,
    cpu_request_m: u64,
    mem_request_mib: u64,
    cpu_usage_m: u64,
    mem_usage_mib: u64,
    missing_requests: usize,
    missing_limits: usize,
    highest_severity: AlertSeverity,
}

impl WorkloadAccumulator {
    fn new(resource_ref: ResourceRef) -> Self {
        Self {
            resource_ref,
            issue_count: 0,
            vulnerability_total: 0,
            fixable_vulnerabilities: 0,
            cpu_request_m: 0,
            mem_request_mib: 0,
            cpu_usage_m: 0,
            mem_usage_mib: 0,
            missing_requests: 0,
            missing_limits: 0,
            highest_severity: AlertSeverity::Info,
        }
    }

    fn finish(self) -> GovernanceWorkloadSummary {
        let idle_cpu_request_m = self.cpu_request_m.saturating_sub(self.cpu_usage_m);
        let idle_mem_request_mib = self.mem_request_mib.saturating_sub(self.mem_usage_mib);
        GovernanceWorkloadSummary {
            compact_label: format!(
                "{}/{} • {} issue(s) • {} vuln • req util {}/{} • idle {}/{}{}{}",
                self.resource_ref.kind(),
                self.resource_ref.name(),
                self.issue_count,
                self.vulnerability_total,
                utilization_label(self.cpu_usage_m, self.cpu_request_m),
                utilization_label(self.mem_usage_mib, self.mem_request_mib),
                crate::state::alerts::format_millicores(idle_cpu_request_m),
                crate::state::alerts::format_mib(idle_mem_request_mib),
                if self.missing_requests > 0 {
                    " • missing req"
                } else {
                    ""
                },
                if self.missing_limits > 0 {
                    " • missing lim"
                } else {
                    ""
                },
            ),
            resource_ref: self.resource_ref,
            issue_count: self.issue_count,
            vulnerability_total: self.vulnerability_total,
            fixable_vulnerabilities: self.fixable_vulnerabilities,
            cpu_request_m: self.cpu_request_m,
            mem_request_mib: self.mem_request_mib,
            cpu_usage_m: self.cpu_usage_m,
            mem_usage_mib: self.mem_usage_mib,
            missing_requests: self.missing_requests,
            missing_limits: self.missing_limits,
            highest_severity: self.highest_severity,
        }
    }
}

#[derive(Debug, Clone)]
struct NamespaceAccumulator {
    project_names: BTreeSet<String>,
    runtime_issue_count: usize,
    sanitizer_issue_count: usize,
    security_issue_count: usize,
    vulnerability_total: usize,
    fixable_vulnerabilities: usize,
    quota_count: usize,
    limit_range_count: usize,
    pdb_gap_count: usize,
    missing_cpu_request_pods: usize,
    missing_mem_request_pods: usize,
    missing_limit_pods: usize,
    highest_severity: AlertSeverity,
    representative: Option<ResourceRef>,
    risk_signals: Vec<String>,
    top_workloads: BTreeMap<WorkloadKey, WorkloadAccumulator>,
}

impl NamespaceAccumulator {
    fn new() -> Self {
        Self {
            project_names: BTreeSet::new(),
            runtime_issue_count: 0,
            sanitizer_issue_count: 0,
            security_issue_count: 0,
            vulnerability_total: 0,
            fixable_vulnerabilities: 0,
            quota_count: 0,
            limit_range_count: 0,
            pdb_gap_count: 0,
            missing_cpu_request_pods: 0,
            missing_mem_request_pods: 0,
            missing_limit_pods: 0,
            highest_severity: AlertSeverity::Info,
            representative: None,
            risk_signals: Vec::new(),
            top_workloads: BTreeMap::new(),
        }
    }

    fn push_signal(&mut self, signal: String) {
        if self.risk_signals.len() < 4 && !self.risk_signals.contains(&signal) {
            self.risk_signals.push(signal);
        }
    }
}

pub fn compute_governance(snapshot: &ClusterSnapshot) -> GovernanceCache {
    let key = (
        snapshot.snapshot_version,
        std::ptr::from_ref(snapshot) as usize,
    );
    {
        let guard = GOVERNANCE_CACHE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((cached_key, summaries)) = guard.as_ref()
            && *cached_key == key
        {
            return Arc::clone(summaries);
        }
    }

    let summaries = Arc::new(build_governance(snapshot));
    {
        let mut guard = GOVERNANCE_CACHE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *guard = Some((key, Arc::clone(&summaries)));
    }
    summaries
}

pub fn filtered_governance_indices(
    summaries: &[NamespaceGovernanceSummary],
    query: &str,
) -> Vec<usize> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return (0..summaries.len()).collect();
    }

    summaries
        .iter()
        .enumerate()
        .filter_map(|(idx, summary)| summary.matches_query(trimmed).then_some(idx))
        .collect()
}

fn build_governance(snapshot: &ClusterSnapshot) -> Vec<NamespaceGovernanceSummary> {
    let mut namespaces = BTreeMap::<String, NamespaceAccumulator>::new();
    for namespace in &snapshot.namespace_list {
        namespaces.insert(namespace.name.clone(), NamespaceAccumulator::new());
    }

    let replica_set_owners = snapshot
        .replicasets
        .iter()
        .filter_map(|replicaset| {
            replicaset
                .owner_references
                .iter()
                .find(|owner| owner.kind == "Deployment")
                .map(|owner| {
                    (
                        (replicaset.namespace.clone(), replicaset.name.clone()),
                        owner.name.clone(),
                    )
                })
        })
        .collect::<HashMap<_, _>>();
    let job_owners = snapshot
        .jobs
        .iter()
        .filter_map(|job| {
            job.owner_references
                .iter()
                .find(|owner| owner.kind == "CronJob")
                .map(|owner| {
                    (
                        (job.namespace.clone(), job.name.clone()),
                        owner.name.clone(),
                    )
                })
        })
        .collect::<HashMap<_, _>>();
    let pod_metrics = snapshot
        .pod_metrics
        .iter()
        .map(|metrics| {
            let usage = metrics
                .containers
                .iter()
                .fold((0u64, 0u64), |(cpu, mem), container| {
                    (
                        cpu + parse_millicores(&container.cpu),
                        mem + parse_mib(&container.memory),
                    )
                });
            ((metrics.namespace.clone(), metrics.name.clone()), usage)
        })
        .collect::<HashMap<_, _>>();
    let namespace_utilization = compute_namespace_utilization(snapshot)
        .into_iter()
        .map(|summary| (summary.namespace.clone(), summary))
        .collect::<HashMap<_, _>>();

    for quota in &snapshot.resource_quotas {
        namespaces
            .entry(quota.namespace.clone())
            .or_insert_with(NamespaceAccumulator::new)
            .quota_count += 1;
    }
    for limit_range in &snapshot.limit_ranges {
        namespaces
            .entry(limit_range.namespace.clone())
            .or_insert_with(NamespaceAccumulator::new)
            .limit_range_count += 1;
    }

    for project in compute_projects(snapshot).iter() {
        for namespace in &project.namespaces {
            let accumulator = namespaces
                .entry(namespace.clone())
                .or_insert_with(NamespaceAccumulator::new);
            accumulator.project_names.insert(project.name.clone());
            if accumulator.representative.is_none() {
                accumulator.representative = project.representative.clone();
            }
        }
    }

    for pod in &snapshot.pods {
        let namespace = pod.namespace.clone();
        let accumulator = namespaces
            .entry(namespace.clone())
            .or_insert_with(NamespaceAccumulator::new);
        if pod.cpu_request.is_none() {
            accumulator.missing_cpu_request_pods += 1;
        }
        if pod.memory_request.is_none() {
            accumulator.missing_mem_request_pods += 1;
        }
        if pod.cpu_limit.is_none() || pod.memory_limit.is_none() {
            accumulator.missing_limit_pods += 1;
        }

        let workload_key = workload_key_for_pod(pod, &replica_set_owners, &job_owners);
        let workload = accumulator
            .top_workloads
            .entry(workload_key.clone())
            .or_insert_with(|| WorkloadAccumulator::new(workload_resource_ref(&workload_key)));
        if let Some(cpu_request) = pod.cpu_request.as_deref() {
            workload.cpu_request_m += parse_millicores(cpu_request);
        } else {
            workload.missing_requests += 1;
        }
        if let Some(memory_request) = pod.memory_request.as_deref() {
            workload.mem_request_mib += parse_mib(memory_request);
        }
        if pod.cpu_limit.is_none() || pod.memory_limit.is_none() {
            workload.missing_limits += 1;
        }
        if let Some((cpu_usage, mem_usage)) =
            pod_metrics.get(&(pod.namespace.clone(), pod.name.clone()))
        {
            workload.cpu_usage_m += cpu_usage;
            workload.mem_usage_mib += mem_usage;
        }
    }

    for issue in compute_issues(snapshot).iter() {
        let namespace_key = if issue.namespace.is_empty() {
            "cluster".to_string()
        } else {
            issue.namespace.clone()
        };
        let accumulator = namespaces
            .entry(namespace_key.clone())
            .or_insert_with(NamespaceAccumulator::new);
        promote_severity(&mut accumulator.highest_severity, issue.severity);
        match issue.source {
            ClusterIssueSource::Runtime => accumulator.runtime_issue_count += 1,
            ClusterIssueSource::Sanitizer => accumulator.sanitizer_issue_count += 1,
            ClusterIssueSource::Security => accumulator.security_issue_count += 1,
        }
        if issue.category == IssueCategory::MissingPodDisruptionBudget {
            accumulator.pdb_gap_count += 1;
        }
        accumulator.push_signal(issue.message.clone());
        if accumulator.representative.is_none() {
            accumulator.representative = Some(issue.resource_ref.clone());
        }
        if let Some(workload_key) = workload_key_for_resource(
            &issue.resource_ref,
            &replica_set_owners,
            &job_owners,
            snapshot,
        ) {
            let workload = accumulator
                .top_workloads
                .entry(workload_key.clone())
                .or_insert_with(|| WorkloadAccumulator::new(workload_resource_ref(&workload_key)));
            workload.issue_count += 1;
            promote_severity(&mut workload.highest_severity, issue.severity);
        }
    }

    for finding in compute_vulnerability_findings(snapshot).iter() {
        let namespace_key = if finding.namespace.is_empty() {
            "cluster".to_string()
        } else {
            finding.namespace.clone()
        };
        let accumulator = namespaces
            .entry(namespace_key.clone())
            .or_insert_with(NamespaceAccumulator::new);
        accumulator.vulnerability_total += finding.counts.total();
        accumulator.fixable_vulnerabilities += finding.fixable_count;
        promote_severity(&mut accumulator.highest_severity, finding.severity);
        if accumulator.representative.is_none() {
            accumulator.representative = finding.resource_ref.clone();
        }
        if let Some(resource_ref) = &finding.resource_ref
            && let Some(workload_key) =
                workload_key_for_resource(resource_ref, &replica_set_owners, &job_owners, snapshot)
        {
            let workload = accumulator
                .top_workloads
                .entry(workload_key.clone())
                .or_insert_with(|| WorkloadAccumulator::new(workload_resource_ref(&workload_key)));
            workload.vulnerability_total += finding.counts.total();
            workload.fixable_vulnerabilities += finding.fixable_count;
            promote_severity(&mut workload.highest_severity, finding.severity);
        }
    }

    let mut results = namespaces
        .into_iter()
        .map(|(namespace, mut accumulator)| {
            let utilization = namespace_utilization.get(&namespace);
            let cpu_usage_m = utilization.map_or(0, |summary| summary.cpu_usage_m);
            let mem_usage_mib = utilization.map_or(0, |summary| summary.mem_usage_mib);
            let cpu_request_m = utilization.map_or(0, |summary| summary.cpu_request_m);
            let mem_request_mib = utilization.map_or(0, |summary| summary.mem_request_mib);
            let cpu_req_utilization_pct =
                utilization.and_then(|summary| summary.cpu_req_utilization_pct);
            let mem_req_utilization_pct =
                utilization.and_then(|summary| summary.mem_req_utilization_pct);
            let pod_count = utilization.map_or(0, |summary| summary.pod_count);
            let idle_cpu_request_m = cpu_request_m.saturating_sub(cpu_usage_m);
            let idle_mem_request_mib = mem_request_mib.saturating_sub(mem_usage_mib);
            let total_issue_count = accumulator.runtime_issue_count
                + accumulator.sanitizer_issue_count
                + accumulator.security_issue_count;
            let policy_surface_count = accumulator.sanitizer_issue_count
                + accumulator.pdb_gap_count
                + accumulator.quota_count
                + accumulator.limit_range_count;

            if cost_signal(cpu_request_m, cpu_req_utilization_pct, idle_cpu_request_m)
                || cost_signal(
                    mem_request_mib,
                    mem_req_utilization_pct,
                    idle_mem_request_mib,
                )
            {
                promote_severity(&mut accumulator.highest_severity, AlertSeverity::Warning);
                accumulator.push_signal(format!(
                    "Low request utilization with idle cost proxy {}/{}",
                    crate::state::alerts::format_millicores(idle_cpu_request_m),
                    crate::state::alerts::format_mib(idle_mem_request_mib),
                ));
            }

            let mut top_workloads = accumulator
                .top_workloads
                .into_values()
                .map(WorkloadAccumulator::finish)
                .collect::<Vec<_>>();
            let workload_count = top_workloads.len();
            top_workloads.sort_unstable_by(|left, right| {
                severity_rank(right.highest_severity)
                    .cmp(&severity_rank(left.highest_severity))
                    .then_with(|| right.issue_count.cmp(&left.issue_count))
                    .then_with(|| right.vulnerability_total.cmp(&left.vulnerability_total))
                    .then_with(|| {
                        (right.missing_requests + right.missing_limits)
                            .cmp(&(left.missing_requests + left.missing_limits))
                    })
                    .then_with(|| {
                        right
                            .cpu_request_m
                            .saturating_sub(right.cpu_usage_m)
                            .cmp(&left.cpu_request_m.saturating_sub(left.cpu_usage_m))
                    })
                    .then_with(|| {
                        right
                            .mem_request_mib
                            .saturating_sub(right.mem_usage_mib)
                            .cmp(&left.mem_request_mib.saturating_sub(left.mem_usage_mib))
                    })
                    .then_with(|| left.resource_ref.kind().cmp(right.resource_ref.kind()))
                    .then_with(|| left.resource_ref.name().cmp(right.resource_ref.name()))
            });
            top_workloads.truncate(5);
            let projects = accumulator.project_names.into_iter().collect::<Vec<_>>();
            let projects_label = if projects.is_empty() {
                "none".to_string()
            } else {
                projects.join(", ")
            };
            let cpu_req_utilization_label = utilization_pct_label(cpu_req_utilization_pct);
            let mem_req_utilization_label = utilization_pct_label(mem_req_utilization_pct);
            let idle_request_label = format!(
                "{}/{}",
                crate::state::alerts::format_millicores(idle_cpu_request_m),
                crate::state::alerts::format_mib(idle_mem_request_mib)
            );

            NamespaceGovernanceSummary {
                namespace: namespace.clone(),
                project_count: projects.len(),
                project_count_label: projects.len().to_string(),
                workload_count,
                workload_count_label: workload_count.to_string(),
                pod_count,
                runtime_issue_count: accumulator.runtime_issue_count,
                sanitizer_issue_count: accumulator.sanitizer_issue_count,
                security_issue_count: accumulator.security_issue_count,
                total_issue_count_label: total_issue_count.to_string(),
                vulnerability_total: accumulator.vulnerability_total,
                vulnerability_total_label: accumulator.vulnerability_total.to_string(),
                fixable_vulnerabilities: accumulator.fixable_vulnerabilities,
                quota_count: accumulator.quota_count,
                limit_range_count: accumulator.limit_range_count,
                pdb_gap_count: accumulator.pdb_gap_count,
                policy_surface_count_label: policy_surface_count.to_string(),
                missing_cpu_request_pods: accumulator.missing_cpu_request_pods,
                missing_mem_request_pods: accumulator.missing_mem_request_pods,
                missing_limit_pods: accumulator.missing_limit_pods,
                cpu_usage_m,
                mem_usage_mib,
                cpu_request_m,
                mem_request_mib,
                cpu_req_utilization_pct,
                cpu_req_utilization_label: cpu_req_utilization_label.clone(),
                mem_req_utilization_pct,
                mem_req_utilization_label: mem_req_utilization_label.clone(),
                idle_cpu_request_m,
                idle_mem_request_mib,
                idle_request_label: idle_request_label.clone(),
                highest_severity: accumulator.highest_severity,
                representative: accumulator
                    .representative
                    .or_else(|| namespace_fallback_representative(snapshot, &namespace)),
                projects,
                projects_label: projects_label.clone(),
                counts_summary_label: format!(
                    "Workloads: {} • Pods: {} • Issues: runtime {} / sanitizer {} / security {}",
                    workload_count,
                    pod_count,
                    accumulator.runtime_issue_count,
                    accumulator.sanitizer_issue_count,
                    accumulator.security_issue_count
                ),
                policy_surfaces_label: format!(
                    "Policy surfaces: ResourceQuota {} • LimitRange {} • Missing PDB {}",
                    accumulator.quota_count,
                    accumulator.limit_range_count,
                    accumulator.pdb_gap_count
                ),
                vulnerabilities_label: format!(
                    "Vulnerabilities: {} total • {} fixable",
                    accumulator.vulnerability_total, accumulator.fixable_vulnerabilities
                ),
                requests_label: format!(
                    "Requests: CPU {}/{} ({}) • Mem {}/{} ({})",
                    crate::state::alerts::format_millicores(cpu_usage_m),
                    crate::state::alerts::format_millicores(cpu_request_m),
                    cpu_req_utilization_label,
                    crate::state::alerts::format_mib(mem_usage_mib),
                    crate::state::alerts::format_mib(mem_request_mib),
                    mem_req_utilization_label
                ),
                coverage_gaps_label: format!(
                    "Coverage gaps: missing CPU req {} • missing Mem req {} • missing limit {}",
                    accumulator.missing_cpu_request_pods,
                    accumulator.missing_mem_request_pods,
                    accumulator.missing_limit_pods
                ),
                risk_signals: accumulator.risk_signals,
                top_workloads,
            }
        })
        .collect::<Vec<_>>();

    results.sort_unstable_by(|left, right| {
        severity_rank(right.highest_severity)
            .cmp(&severity_rank(left.highest_severity))
            .then_with(|| right.total_issue_count().cmp(&left.total_issue_count()))
            .then_with(|| right.vulnerability_total.cmp(&left.vulnerability_total))
            .then_with(|| right.namespace.cmp(&left.namespace))
    });
    results
}

fn workload_key_for_pod(
    pod: &crate::k8s::dtos::PodInfo,
    replica_set_owners: &HashMap<(String, String), String>,
    job_owners: &HashMap<(String, String), String>,
) -> WorkloadKey {
    if let Some(owner) = pod.owner_references.first() {
        match owner.kind.as_str() {
            "ReplicaSet" => {
                if let Some(deployment_name) =
                    replica_set_owners.get(&(pod.namespace.clone(), owner.name.clone()))
                {
                    return WorkloadKey {
                        namespace: pod.namespace.clone(),
                        kind: "Deployment",
                        name: deployment_name.clone(),
                    };
                }
                return WorkloadKey {
                    namespace: pod.namespace.clone(),
                    kind: "ReplicaSet",
                    name: owner.name.clone(),
                };
            }
            "Job" => {
                if let Some(cronjob_name) =
                    job_owners.get(&(pod.namespace.clone(), owner.name.clone()))
                {
                    return WorkloadKey {
                        namespace: pod.namespace.clone(),
                        kind: "CronJob",
                        name: cronjob_name.clone(),
                    };
                }
                return WorkloadKey {
                    namespace: pod.namespace.clone(),
                    kind: "Job",
                    name: owner.name.clone(),
                };
            }
            kind => {
                return WorkloadKey {
                    namespace: pod.namespace.clone(),
                    kind: stable_workload_kind(kind),
                    name: owner.name.clone(),
                };
            }
        }
    }

    WorkloadKey {
        namespace: pod.namespace.clone(),
        kind: "Pod",
        name: pod.name.clone(),
    }
}

fn workload_key_for_resource(
    resource: &ResourceRef,
    replica_set_owners: &HashMap<(String, String), String>,
    job_owners: &HashMap<(String, String), String>,
    snapshot: &ClusterSnapshot,
) -> Option<WorkloadKey> {
    match resource {
        ResourceRef::Deployment(name, namespace) => Some(WorkloadKey {
            namespace: namespace.clone(),
            kind: "Deployment",
            name: name.clone(),
        }),
        ResourceRef::StatefulSet(name, namespace) => Some(WorkloadKey {
            namespace: namespace.clone(),
            kind: "StatefulSet",
            name: name.clone(),
        }),
        ResourceRef::DaemonSet(name, namespace) => Some(WorkloadKey {
            namespace: namespace.clone(),
            kind: "DaemonSet",
            name: name.clone(),
        }),
        ResourceRef::CronJob(name, namespace) => Some(WorkloadKey {
            namespace: namespace.clone(),
            kind: "CronJob",
            name: name.clone(),
        }),
        ResourceRef::Job(name, namespace) => {
            if let Some(cronjob_name) = job_owners.get(&(namespace.clone(), name.clone())) {
                Some(WorkloadKey {
                    namespace: namespace.clone(),
                    kind: "CronJob",
                    name: cronjob_name.clone(),
                })
            } else {
                Some(WorkloadKey {
                    namespace: namespace.clone(),
                    kind: "Job",
                    name: name.clone(),
                })
            }
        }
        ResourceRef::ReplicaSet(name, namespace) => {
            if let Some(deployment_name) =
                replica_set_owners.get(&(namespace.clone(), name.clone()))
            {
                Some(WorkloadKey {
                    namespace: namespace.clone(),
                    kind: "Deployment",
                    name: deployment_name.clone(),
                })
            } else {
                Some(WorkloadKey {
                    namespace: namespace.clone(),
                    kind: "ReplicaSet",
                    name: name.clone(),
                })
            }
        }
        ResourceRef::Pod(name, namespace) => snapshot
            .pods
            .iter()
            .find(|pod| pod.name == *name && pod.namespace == *namespace)
            .map(|pod| workload_key_for_pod(pod, replica_set_owners, job_owners)),
        _ => None,
    }
}

fn workload_resource_ref(key: &WorkloadKey) -> ResourceRef {
    match key.kind {
        "Deployment" => ResourceRef::Deployment(key.name.clone(), key.namespace.clone()),
        "StatefulSet" => ResourceRef::StatefulSet(key.name.clone(), key.namespace.clone()),
        "DaemonSet" => ResourceRef::DaemonSet(key.name.clone(), key.namespace.clone()),
        "CronJob" => ResourceRef::CronJob(key.name.clone(), key.namespace.clone()),
        "Job" => ResourceRef::Job(key.name.clone(), key.namespace.clone()),
        "ReplicaSet" => ResourceRef::ReplicaSet(key.name.clone(), key.namespace.clone()),
        _ => ResourceRef::Pod(key.name.clone(), key.namespace.clone()),
    }
}

fn stable_workload_kind(kind: &str) -> &'static str {
    match kind {
        "Deployment" => "Deployment",
        "StatefulSet" => "StatefulSet",
        "DaemonSet" => "DaemonSet",
        "CronJob" => "CronJob",
        "Job" => "Job",
        "ReplicaSet" => "ReplicaSet",
        _ => "Pod",
    }
}

fn promote_severity(current: &mut AlertSeverity, candidate: AlertSeverity) {
    if severity_rank(candidate) > severity_rank(*current) {
        *current = candidate;
    }
}

fn severity_rank(severity: AlertSeverity) -> u8 {
    match severity {
        AlertSeverity::Info => 0,
        AlertSeverity::Warning => 1,
        AlertSeverity::Error => 2,
    }
}

fn utilization_label(usage: u64, request: u64) -> String {
    if request == 0 {
        return "n/a".to_string();
    }
    format!("{}%", ((usage * 100) / request).min(999))
}

fn utilization_pct_label(value: Option<u16>) -> String {
    value.map_or_else(|| "n/a".to_string(), |pct| format!("{pct}%"))
}

fn cost_signal(request_total: u64, utilization_pct: Option<u16>, idle_total: u64) -> bool {
    request_total > 0
        && idle_total.saturating_mul(100) >= request_total.saturating_mul(50)
        && utilization_pct.is_some_and(|pct| pct < 40)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::ResourceRef,
        k8s::dtos::{
            ContainerMetrics, NamespaceInfo, OwnerRefInfo, PodInfo, PodMetricsInfo,
            ResourceQuotaInfo, VulnerabilityReportInfo, VulnerabilitySummaryCounts,
        },
        state::ClusterSnapshot,
    };

    #[test]
    fn governance_aggregates_namespace_and_workload_risks() {
        let mut snapshot = ClusterSnapshot {
            namespace_list: vec![NamespaceInfo {
                name: "team-a".to_string(),
                ..NamespaceInfo::default()
            }],
            ..ClusterSnapshot::default()
        };
        snapshot.pods.push(PodInfo {
            name: "api-123".to_string(),
            namespace: "team-a".to_string(),
            status: "Running".to_string(),
            owner_references: vec![OwnerRefInfo {
                kind: "ReplicaSet".to_string(),
                name: "api-rs".to_string(),
                uid: "rs-uid".to_string(),
            }],
            cpu_request: Some("1000m".to_string()),
            memory_request: Some("1Gi".to_string()),
            ..PodInfo::default()
        });
        snapshot.replicasets.push(crate::k8s::dtos::ReplicaSetInfo {
            name: "api-rs".to_string(),
            namespace: "team-a".to_string(),
            owner_references: vec![OwnerRefInfo {
                kind: "Deployment".to_string(),
                name: "api".to_string(),
                uid: "deploy-uid".to_string(),
            }],
            ..crate::k8s::dtos::ReplicaSetInfo::default()
        });
        snapshot.pod_metrics.push(PodMetricsInfo {
            name: "api-123".to_string(),
            namespace: "team-a".to_string(),
            containers: vec![ContainerMetrics {
                name: "api".to_string(),
                cpu: "100m".to_string(),
                memory: "256Mi".to_string(),
            }],
            ..PodMetricsInfo::default()
        });
        snapshot.resource_quotas.push(ResourceQuotaInfo {
            name: "quota".to_string(),
            namespace: "team-a".to_string(),
            ..ResourceQuotaInfo::default()
        });
        snapshot
            .vulnerability_reports
            .push(VulnerabilityReportInfo {
                resource_kind: "Deployment".to_string(),
                resource_name: "api".to_string(),
                resource_namespace: "team-a".to_string(),
                counts: VulnerabilitySummaryCounts {
                    high: 2,
                    ..VulnerabilitySummaryCounts::default()
                },
                fixable_count: 1,
                ..VulnerabilityReportInfo::default()
            });
        snapshot.snapshot_version = 1;

        let governance = compute_governance(&snapshot);
        let summary = governance.first().expect("namespace summary");
        assert_eq!(summary.namespace, "team-a");
        assert_eq!(summary.quota_count, 1);
        assert_eq!(summary.vulnerability_total, 2);
        assert_eq!(summary.idle_cpu_request_m, 900);
        assert_eq!(summary.idle_request_label, "900m/768Mi");
        assert_eq!(summary.top_workloads.len(), 1);
        assert_eq!(
            summary.top_workloads[0].resource_ref,
            ResourceRef::Deployment("api".to_string(), "team-a".to_string())
        );
        assert!(
            summary.top_workloads[0]
                .compact_label
                .contains("Deployment/api")
        );
    }

    #[test]
    fn governance_search_matches_workload_names() {
        let summary = NamespaceGovernanceSummary {
            namespace: "team-a".to_string(),
            project_count: 0,
            project_count_label: "0".to_string(),
            workload_count: 1,
            workload_count_label: "1".to_string(),
            pod_count: 0,
            runtime_issue_count: 0,
            sanitizer_issue_count: 0,
            security_issue_count: 0,
            total_issue_count_label: "0".to_string(),
            vulnerability_total: 0,
            vulnerability_total_label: "0".to_string(),
            fixable_vulnerabilities: 0,
            quota_count: 0,
            limit_range_count: 0,
            pdb_gap_count: 0,
            policy_surface_count_label: "0".to_string(),
            missing_cpu_request_pods: 0,
            missing_mem_request_pods: 0,
            missing_limit_pods: 0,
            cpu_usage_m: 0,
            mem_usage_mib: 0,
            cpu_request_m: 0,
            mem_request_mib: 0,
            cpu_req_utilization_pct: None,
            cpu_req_utilization_label: "n/a".to_string(),
            mem_req_utilization_pct: None,
            mem_req_utilization_label: "n/a".to_string(),
            idle_cpu_request_m: 0,
            idle_mem_request_mib: 0,
            idle_request_label: "0m/0Mi".to_string(),
            highest_severity: AlertSeverity::Info,
            representative: Some(ResourceRef::Namespace("team-a".to_string())),
            projects: vec!["payments".to_string()],
            projects_label: "payments".to_string(),
            counts_summary_label:
                "Workloads: 1 • Pods: 0 • Issues: runtime 0 / sanitizer 0 / security 0".to_string(),
            policy_surfaces_label:
                "Policy surfaces: ResourceQuota 0 • LimitRange 0 • Missing PDB 0".to_string(),
            vulnerabilities_label: "Vulnerabilities: 0 total • 0 fixable".to_string(),
            requests_label: "Requests: CPU 0m/0m (n/a) • Mem 0Mi/0Mi (n/a)".to_string(),
            coverage_gaps_label:
                "Coverage gaps: missing CPU req 0 • missing Mem req 0 • missing limit 0".to_string(),
            risk_signals: vec!["Missing requests".to_string()],
            top_workloads: vec![GovernanceWorkloadSummary {
                resource_ref: ResourceRef::Deployment("api".to_string(), "team-a".to_string()),
                issue_count: 0,
                vulnerability_total: 0,
                fixable_vulnerabilities: 0,
                cpu_request_m: 0,
                mem_request_mib: 0,
                cpu_usage_m: 0,
                mem_usage_mib: 0,
                missing_requests: 0,
                missing_limits: 0,
                highest_severity: AlertSeverity::Info,
                compact_label:
                    "Deployment/api • 0 issue(s) • 0 vuln • req util n/a/n/a • idle 0m/0Mi"
                        .to_string(),
            }],
        };

        assert!(summary.matches_query("api"));
        assert!(summary.matches_query("payments"));
        assert!(summary.matches_query("missing requests"));
    }

    #[test]
    fn governance_cluster_row_does_not_fallback_to_fake_namespace_target() {
        let snapshot = ClusterSnapshot::default();

        assert_eq!(
            namespace_fallback_representative(&snapshot, "cluster"),
            None
        );
    }

    #[test]
    fn governance_namespace_row_falls_back_to_real_namespace_target() {
        let snapshot = ClusterSnapshot {
            namespace_list: vec![NamespaceInfo {
                name: "team-a".to_string(),
                ..NamespaceInfo::default()
            }],
            ..ClusterSnapshot::default()
        };

        assert_eq!(
            namespace_fallback_representative(&snapshot, "team-a"),
            Some(ResourceRef::Namespace("team-a".to_string()))
        );
    }

    #[test]
    fn governance_representative_label_uses_actual_target() {
        let summary = NamespaceGovernanceSummary {
            namespace: "team-a".to_string(),
            representative: Some(ResourceRef::Deployment(
                "api".to_string(),
                "team-a".to_string(),
            )),
            project_count: 0,
            project_count_label: "0".to_string(),
            workload_count: 0,
            workload_count_label: "0".to_string(),
            pod_count: 0,
            runtime_issue_count: 0,
            sanitizer_issue_count: 0,
            security_issue_count: 0,
            total_issue_count_label: "0".to_string(),
            vulnerability_total: 0,
            vulnerability_total_label: "0".to_string(),
            fixable_vulnerabilities: 0,
            quota_count: 0,
            limit_range_count: 0,
            pdb_gap_count: 0,
            policy_surface_count_label: "0".to_string(),
            missing_cpu_request_pods: 0,
            missing_mem_request_pods: 0,
            missing_limit_pods: 0,
            cpu_usage_m: 0,
            mem_usage_mib: 0,
            cpu_request_m: 0,
            mem_request_mib: 0,
            cpu_req_utilization_pct: None,
            cpu_req_utilization_label: "n/a".to_string(),
            mem_req_utilization_pct: None,
            mem_req_utilization_label: "n/a".to_string(),
            idle_cpu_request_m: 0,
            idle_mem_request_mib: 0,
            idle_request_label: "0m/0Mi".to_string(),
            highest_severity: AlertSeverity::Info,
            projects: Vec::new(),
            projects_label: "none".to_string(),
            counts_summary_label: String::new(),
            policy_surfaces_label: String::new(),
            vulnerabilities_label: String::new(),
            requests_label: String::new(),
            coverage_gaps_label: String::new(),
            risk_signals: Vec::new(),
            top_workloads: Vec::new(),
        };

        assert_eq!(
            summary.representative_label().as_deref(),
            Some("Deployment/team-a/api")
        );
    }
}
