//! Cached vulnerability aggregation built from snapshot-level Trivy Operator reports.

use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, LazyLock, Mutex},
};

use crate::{
    app::ResourceRef,
    k8s::dtos::{AlertSeverity, VulnerabilityReportInfo, VulnerabilitySummaryCounts},
    state::ClusterSnapshot,
    ui::contains_ci,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VulnerabilityFinding {
    pub severity: AlertSeverity,
    pub resource_kind: String,
    pub resource_name: String,
    pub namespace: String,
    pub counts: VulnerabilitySummaryCounts,
    pub fixable_count: usize,
    pub report_count: usize,
    pub container_count: usize,
    pub containers: Vec<String>,
    pub artifacts: Vec<String>,
    pub scanners: Vec<String>,
    pub resource_ref: Option<ResourceRef>,
    pub cluster_scoped: bool,
}

impl VulnerabilityFinding {
    pub fn matches_query(&self, query: &str) -> bool {
        contains_ci(&self.resource_kind, query)
            || contains_ci(&self.resource_name, query)
            || contains_ci(&self.namespace, query)
            || self
                .artifacts
                .iter()
                .any(|artifact| contains_ci(artifact, query))
            || self
                .containers
                .iter()
                .any(|container| contains_ci(container, query))
            || self
                .scanners
                .iter()
                .any(|scanner| contains_ci(scanner, query))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AggregateFinding {
    resource_kind: String,
    resource_name: String,
    namespace: String,
    counts: VulnerabilitySummaryCounts,
    fixable_count: usize,
    report_count: usize,
    containers: BTreeSet<String>,
    artifacts: BTreeSet<String>,
    scanners: BTreeSet<String>,
    cluster_scoped: bool,
}

type FindingCacheValue = Arc<Vec<VulnerabilityFinding>>;

static FINDING_CACHE: LazyLock<Mutex<Option<(u64, FindingCacheValue)>>> =
    LazyLock::new(|| Mutex::new(None));

pub fn compute_vulnerability_findings(snapshot: &ClusterSnapshot) -> FindingCacheValue {
    let version = snapshot.snapshot_version;
    {
        let guard = FINDING_CACHE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((cached_version, findings)) = guard.as_ref()
            && *cached_version == version
        {
            return Arc::clone(findings);
        }
    }

    let findings = Arc::new(build_findings(&snapshot.vulnerability_reports));
    {
        let mut guard = FINDING_CACHE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *guard = Some((version, Arc::clone(&findings)));
    }
    findings
}

pub fn filtered_vulnerability_indices(
    findings: &[VulnerabilityFinding],
    query: &str,
) -> Vec<usize> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return (0..findings.len()).collect();
    }

    findings
        .iter()
        .enumerate()
        .filter_map(|(idx, finding)| finding.matches_query(trimmed).then_some(idx))
        .collect()
}

fn build_findings(reports: &[VulnerabilityReportInfo]) -> Vec<VulnerabilityFinding> {
    let mut aggregates = HashMap::<(String, String, String, bool), AggregateFinding>::new();
    for report in reports {
        let key = (
            report.resource_kind.clone(),
            report.resource_name.clone(),
            report.resource_namespace.clone(),
            report.cluster_scoped,
        );
        let existing = aggregates.entry(key).or_insert_with(|| AggregateFinding {
            resource_kind: report.resource_kind.clone(),
            resource_name: report.resource_name.clone(),
            namespace: report.resource_namespace.clone(),
            counts: VulnerabilitySummaryCounts::default(),
            fixable_count: 0,
            report_count: 0,
            containers: BTreeSet::new(),
            artifacts: BTreeSet::new(),
            scanners: BTreeSet::new(),
            cluster_scoped: report.cluster_scoped,
        });

        existing.counts.critical += report.counts.critical;
        existing.counts.high += report.counts.high;
        existing.counts.medium += report.counts.medium;
        existing.counts.low += report.counts.low;
        existing.counts.unknown += report.counts.unknown;
        existing.fixable_count += report.fixable_count;
        existing.report_count += 1;
        existing
            .containers
            .extend(report.container_name.iter().cloned());
        existing.artifacts.extend(report_artifacts(report));
        existing.scanners.extend(report_scanners(report));
    }

    let mut findings = aggregates
        .into_values()
        .map(|aggregate| VulnerabilityFinding {
            severity: aggregate.counts.highest_severity(),
            resource_kind: aggregate.resource_kind.clone(),
            resource_name: aggregate.resource_name.clone(),
            namespace: aggregate.namespace.clone(),
            counts: aggregate.counts,
            fixable_count: aggregate.fixable_count,
            report_count: aggregate.report_count,
            container_count: aggregate.containers.len(),
            containers: aggregate.containers.into_iter().collect(),
            artifacts: aggregate.artifacts.into_iter().collect(),
            scanners: aggregate.scanners.into_iter().collect(),
            resource_ref: vulnerability_resource_ref(
                &aggregate.resource_kind,
                &aggregate.resource_name,
                &aggregate.namespace,
            ),
            cluster_scoped: aggregate.cluster_scoped,
        })
        .collect::<Vec<_>>();

    findings.sort_unstable_by(|left, right| {
        severity_rank(right.severity)
            .cmp(&severity_rank(left.severity))
            .then_with(|| right.counts.total().cmp(&left.counts.total()))
            .then_with(|| left.namespace.cmp(&right.namespace))
            .then_with(|| left.resource_kind.cmp(&right.resource_kind))
            .then_with(|| left.resource_name.cmp(&right.resource_name))
    });
    findings
}

fn severity_rank(severity: AlertSeverity) -> u8 {
    match severity {
        AlertSeverity::Error => 3,
        AlertSeverity::Warning => 2,
        AlertSeverity::Info => 1,
    }
}

fn report_artifacts(report: &VulnerabilityReportInfo) -> BTreeSet<String> {
    let mut artifacts = BTreeSet::new();
    if let Some(repository) = report.artifact_repository.as_deref() {
        if let Some(tag) = report.artifact_tag.as_deref() {
            artifacts.insert(format!("{repository}:{tag}"));
        } else {
            artifacts.insert(repository.to_string());
        }
    }
    artifacts
}

fn report_scanners(report: &VulnerabilityReportInfo) -> BTreeSet<String> {
    let mut scanners = BTreeSet::new();
    if let Some(name) = report.scanner_name.as_deref() {
        if let Some(version) = report.scanner_version.as_deref() {
            scanners.insert(format!("{name} {version}"));
        } else {
            scanners.insert(name.to_string());
        }
    }
    scanners
}

fn vulnerability_resource_ref(kind: &str, name: &str, namespace: &str) -> Option<ResourceRef> {
    match kind {
        "Deployment" => Some(ResourceRef::Deployment(
            name.to_string(),
            namespace.to_string(),
        )),
        "StatefulSet" => Some(ResourceRef::StatefulSet(
            name.to_string(),
            namespace.to_string(),
        )),
        "DaemonSet" => Some(ResourceRef::DaemonSet(
            name.to_string(),
            namespace.to_string(),
        )),
        "ReplicaSet" => Some(ResourceRef::ReplicaSet(
            name.to_string(),
            namespace.to_string(),
        )),
        "ReplicationController" => Some(ResourceRef::ReplicationController(
            name.to_string(),
            namespace.to_string(),
        )),
        "Job" => Some(ResourceRef::Job(name.to_string(), namespace.to_string())),
        "CronJob" => Some(ResourceRef::CronJob(
            name.to_string(),
            namespace.to_string(),
        )),
        "Pod" => Some(ResourceRef::Pod(name.to_string(), namespace.to_string())),
        "Node" => Some(ResourceRef::Node(name.to_string())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(kind: &str, name: &str, namespace: &str, container: &str) -> VulnerabilityReportInfo {
        VulnerabilityReportInfo {
            resource_kind: kind.to_string(),
            resource_name: name.to_string(),
            resource_namespace: namespace.to_string(),
            namespace: namespace.to_string(),
            container_name: Some(container.to_string()),
            artifact_repository: Some("ghcr.io/demo/app".to_string()),
            artifact_tag: Some(container.to_string()),
            counts: VulnerabilitySummaryCounts {
                critical: 1,
                high: 2,
                medium: 0,
                low: 0,
                unknown: 0,
            },
            fixable_count: 2,
            ..VulnerabilityReportInfo::default()
        }
    }

    #[test]
    fn aggregates_reports_per_resource() {
        let findings = build_findings(&[
            report("Deployment", "api", "default", "web"),
            report("Deployment", "api", "default", "sidecar"),
        ]);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].report_count, 2);
        assert_eq!(findings[0].container_count, 2);
        assert_eq!(findings[0].counts.critical, 2);
        assert_eq!(findings[0].fixable_count, 4);
    }

    #[test]
    fn filters_across_kind_name_and_artifact() {
        let findings = build_findings(&[report("Deployment", "api", "default", "web")]);
        assert_eq!(filtered_vulnerability_indices(&findings, "deploy"), vec![0]);
        assert_eq!(
            filtered_vulnerability_indices(&findings, "ghcr.io"),
            vec![0]
        );
        assert!(filtered_vulnerability_indices(&findings, "missing").is_empty());
    }

    #[test]
    fn aggregation_keeps_unique_sorted_metadata() {
        let mut first = report("Deployment", "api", "default", "web");
        first.scanner_name = Some("Trivy".into());
        first.scanner_version = Some("0.58.0".into());
        let mut second = report("Deployment", "api", "default", "api");
        second.scanner_name = Some("Trivy".into());
        second.scanner_version = Some("0.58.0".into());
        second.artifact_tag = Some("api".into());

        let findings = build_findings(&[first, second]);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].containers,
            vec!["api".to_string(), "web".to_string()]
        );
        assert_eq!(
            findings[0].artifacts,
            vec![
                "ghcr.io/demo/app:api".to_string(),
                "ghcr.io/demo/app:web".to_string()
            ]
        );
        assert_eq!(findings[0].scanners, vec!["Trivy 0.58.0".to_string()]);
    }
}
