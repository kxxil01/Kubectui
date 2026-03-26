//! Cached vulnerability aggregation built from snapshot-level Trivy Operator reports.

use std::sync::{Arc, LazyLock, Mutex};

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
    containers: Vec<String>,
    artifacts: Vec<String>,
    scanners: Vec<String>,
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
    let mut aggregates: Vec<AggregateFinding> = Vec::new();
    for report in reports {
        let namespace = report.resource_namespace.clone();
        let Some(existing) = aggregates.iter_mut().find(|aggregate| {
            aggregate.resource_kind == report.resource_kind
                && aggregate.resource_name == report.resource_name
                && aggregate.namespace == namespace
                && aggregate.cluster_scoped == report.cluster_scoped
        }) else {
            aggregates.push(AggregateFinding {
                resource_kind: report.resource_kind.clone(),
                resource_name: report.resource_name.clone(),
                namespace,
                counts: report.counts.clone(),
                fixable_count: report.fixable_count,
                report_count: 1,
                containers: report.container_name.iter().cloned().collect(),
                artifacts: report_artifacts(report),
                scanners: report_scanners(report),
                cluster_scoped: report.cluster_scoped,
            });
            continue;
        };

        existing.counts.critical += report.counts.critical;
        existing.counts.high += report.counts.high;
        existing.counts.medium += report.counts.medium;
        existing.counts.low += report.counts.low;
        existing.counts.unknown += report.counts.unknown;
        existing.fixable_count += report.fixable_count;
        existing.report_count += 1;
        extend_unique(
            &mut existing.containers,
            report.container_name.iter().cloned(),
        );
        extend_unique(
            &mut existing.artifacts,
            report_artifacts(report).into_iter(),
        );
        extend_unique(&mut existing.scanners, report_scanners(report).into_iter());
    }

    let mut findings = aggregates
        .into_iter()
        .map(|aggregate| VulnerabilityFinding {
            severity: aggregate.counts.highest_severity(),
            resource_kind: aggregate.resource_kind.clone(),
            resource_name: aggregate.resource_name.clone(),
            namespace: aggregate.namespace.clone(),
            counts: aggregate.counts,
            fixable_count: aggregate.fixable_count,
            report_count: aggregate.report_count,
            container_count: aggregate.containers.len(),
            containers: aggregate.containers,
            artifacts: aggregate.artifacts,
            scanners: aggregate.scanners,
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

fn extend_unique<T, I>(items: &mut Vec<T>, values: I)
where
    T: PartialEq,
    I: IntoIterator<Item = T>,
{
    for value in values {
        if !items.contains(&value) {
            items.push(value);
        }
    }
}

fn report_artifacts(report: &VulnerabilityReportInfo) -> Vec<String> {
    let mut artifacts = Vec::new();
    if let Some(repository) = report.artifact_repository.as_deref() {
        if let Some(tag) = report.artifact_tag.as_deref() {
            artifacts.push(format!("{repository}:{tag}"));
        } else {
            artifacts.push(repository.to_string());
        }
    }
    artifacts
}

fn report_scanners(report: &VulnerabilityReportInfo) -> Vec<String> {
    let mut scanners = Vec::new();
    if let Some(name) = report.scanner_name.as_deref() {
        if let Some(version) = report.scanner_version.as_deref() {
            scanners.push(format!("{name} {version}"));
        } else {
            scanners.push(name.to_string());
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
}
