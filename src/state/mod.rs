//! Global state management for KubecTUI.

pub mod alerts;
pub mod filters;
pub mod port_forward;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::{collections::BTreeSet, fmt, time::Duration};

use crate::k8s::{
    client::K8sClient,
    dtos::{
        ClusterInfo, CronJobInfo, DaemonSetInfo, DeploymentInfo, JobInfo, NodeInfo, PodInfo,
        ServiceInfo, StatefulSetInfo,
    },
};

/// High-level data loading phase for cluster resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DataPhase {
    /// No data fetch attempted yet.
    #[default]
    Idle,
    /// Data fetch is currently in progress.
    Loading,
    /// Data is available from the most recent successful fetch.
    Ready,
    /// Last fetch failed.
    Error,
}

impl fmt::Display for DataPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            DataPhase::Idle => "idle",
            DataPhase::Loading => "loading",
            DataPhase::Ready => "ready",
            DataPhase::Error => "error",
        };
        f.write_str(label)
    }
}

/// Snapshot used by rendering layer.
#[derive(Debug, Clone, Default)]
pub struct ClusterSnapshot {
    pub nodes: Vec<NodeInfo>,
    pub pods: Vec<PodInfo>,
    pub services: Vec<ServiceInfo>,
    pub deployments: Vec<DeploymentInfo>,
    pub statefulsets: Vec<StatefulSetInfo>,
    pub daemonsets: Vec<DaemonSetInfo>,
    pub jobs: Vec<JobInfo>,
    pub cronjobs: Vec<CronJobInfo>,
    pub cluster_info: Option<ClusterInfo>,
    pub services_count: usize,
    pub namespaces_count: usize,
    pub phase: DataPhase,
    pub last_updated: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub cluster_url: Option<String>,
}

impl ClusterSnapshot {
    /// Returns a compact string suitable for header display.
    pub fn cluster_summary(&self) -> &str {
        self.cluster_url
            .as_deref()
            .unwrap_or("Cluster endpoint unavailable")
    }
}

/// Data source contract for retrieving Kubernetes snapshot inputs.
#[async_trait]
pub trait ClusterDataSource {
    /// Returns cluster API URL for status header.
    fn cluster_url(&self) -> &str;
    /// Fetches node list.
    async fn fetch_nodes(&self) -> Result<Vec<NodeInfo>>;
    /// Fetches available namespaces.
    async fn fetch_namespaces(&self) -> Result<Vec<String>>;
    /// Fetches pod list.
    async fn fetch_pods(&self, namespace: Option<&str>) -> Result<Vec<PodInfo>>;
    /// Fetches service list.
    async fn fetch_services(&self, namespace: Option<&str>) -> Result<Vec<ServiceInfo>>;
    /// Fetches deployment list.
    async fn fetch_deployments(&self, namespace: Option<&str>) -> Result<Vec<DeploymentInfo>>;
    /// Fetches StatefulSet list.
    async fn fetch_statefulsets(&self, namespace: Option<&str>) -> Result<Vec<StatefulSetInfo>>;
    /// Fetches DaemonSet list.
    async fn fetch_daemonsets(&self, namespace: Option<&str>) -> Result<Vec<DaemonSetInfo>>;
    /// Fetches Job list.
    async fn fetch_jobs(&self, namespace: Option<&str>) -> Result<Vec<JobInfo>>;
    /// Fetches CronJob list.
    async fn fetch_cronjobs(&self, namespace: Option<&str>) -> Result<Vec<CronJobInfo>>;
    /// Fetches cluster metadata.
    async fn fetch_cluster_info(&self) -> Result<ClusterInfo>;
}

#[async_trait]
impl ClusterDataSource for K8sClient {
    fn cluster_url(&self) -> &str {
        K8sClient::cluster_url(self)
    }

    async fn fetch_nodes(&self) -> Result<Vec<NodeInfo>> {
        K8sClient::fetch_nodes(self).await
    }

    async fn fetch_namespaces(&self) -> Result<Vec<String>> {
        K8sClient::fetch_namespaces(self).await
    }

    async fn fetch_pods(&self, namespace: Option<&str>) -> Result<Vec<PodInfo>> {
        K8sClient::fetch_pods(self, namespace).await
    }

    async fn fetch_services(&self, namespace: Option<&str>) -> Result<Vec<ServiceInfo>> {
        K8sClient::fetch_services(self, namespace).await
    }

    async fn fetch_deployments(&self, namespace: Option<&str>) -> Result<Vec<DeploymentInfo>> {
        K8sClient::fetch_deployments(self, namespace).await
    }

    async fn fetch_statefulsets(&self, namespace: Option<&str>) -> Result<Vec<StatefulSetInfo>> {
        K8sClient::fetch_statefulsets(self, namespace).await
    }

    async fn fetch_daemonsets(&self, namespace: Option<&str>) -> Result<Vec<DaemonSetInfo>> {
        K8sClient::fetch_daemonsets(self, namespace).await
    }

    async fn fetch_jobs(&self, namespace: Option<&str>) -> Result<Vec<JobInfo>> {
        K8sClient::fetch_jobs(self, namespace).await
    }

    async fn fetch_cronjobs(&self, namespace: Option<&str>) -> Result<Vec<CronJobInfo>> {
        K8sClient::fetch_cronjobs(self, namespace).await
    }

    async fn fetch_cluster_info(&self) -> Result<ClusterInfo> {
        K8sClient::fetch_cluster_info(self).await
    }
}

/// Mutable state holder with async refresh operations.
#[derive(Debug, Clone, Default)]
pub struct GlobalState {
    snapshot: ClusterSnapshot,
    pub namespaces: Vec<String>,
}

impl GlobalState {
    /// Returns a cloneable immutable snapshot for UI rendering.
    pub fn snapshot(&self) -> ClusterSnapshot {
        self.snapshot.clone()
    }

    /// Returns fetched namespaces.
    pub fn namespaces(&self) -> &[String] {
        &self.namespaces
    }

    async fn fetch_with_timeout<T>(
        label: &'static str,
        fut: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        match tokio::time::timeout(Duration::from_secs(5), fut).await {
            Ok(result) => result,
            Err(_) => Err(anyhow!("timed out fetching {label}")),
        }
    }

    /// Refreshes core resources in parallel, updating status and timestamps.
    ///
    /// Production hardening behavior:
    /// - Per-resource timeout protection (5s)
    /// - Graceful degradation for partial API failures
    /// - Returns error only when all critical resources fail
    pub async fn refresh<D>(&mut self, client: &D, namespace: Option<&str>) -> Result<()>
    where
        D: ClusterDataSource + Sync,
    {
        if self.snapshot.phase == DataPhase::Loading {
            return Ok(());
        }

        self.snapshot.phase = DataPhase::Loading;
        self.snapshot.last_error = None;
        self.snapshot.cluster_url = Some(client.cluster_url().to_string());

        let namespaces_res =
            Self::fetch_with_timeout("namespaces", client.fetch_namespaces()).await;

        let (
            nodes_res,
            pods_res,
            services_res,
            deployments_res,
            statefulsets_res,
            daemonsets_res,
            jobs_res,
            cronjobs_res,
            cluster_info_res,
        ) = tokio::join!(
            Self::fetch_with_timeout("nodes", client.fetch_nodes()),
            Self::fetch_with_timeout("pods", client.fetch_pods(namespace)),
            Self::fetch_with_timeout("services", client.fetch_services(namespace)),
            Self::fetch_with_timeout("deployments", client.fetch_deployments(namespace)),
            Self::fetch_with_timeout("statefulsets", client.fetch_statefulsets(namespace)),
            Self::fetch_with_timeout("daemonsets", client.fetch_daemonsets(namespace)),
            Self::fetch_with_timeout("jobs", client.fetch_jobs(namespace)),
            Self::fetch_with_timeout("cronjobs", client.fetch_cronjobs(namespace)),
            Self::fetch_with_timeout("cluster info", client.fetch_cluster_info()),
        );

        let mut errors = Vec::new();

        self.namespaces = match namespaces_res {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("namespaces: {e}"));
                Vec::new()
            }
        };

        let nodes = match nodes_res {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("nodes: {e}"));
                Vec::new()
            }
        };
        let pods = match pods_res {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("pods: {e}"));
                Vec::new()
            }
        };
        let services = match services_res {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("services: {e}"));
                Vec::new()
            }
        };
        let deployments = match deployments_res {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("deployments: {e}"));
                Vec::new()
            }
        };
        let statefulsets = match statefulsets_res {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("statefulsets: {e}"));
                Vec::new()
            }
        };
        let daemonsets = match daemonsets_res {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("daemonsets: {e}"));
                Vec::new()
            }
        };
        let jobs = match jobs_res {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("jobs: {e}"));
                Vec::new()
            }
        };
        let cronjobs = match cronjobs_res {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("cronjobs: {e}"));
                Vec::new()
            }
        };
        let cluster_info = match cluster_info_res {
            Ok(v) => Some(v),
            Err(e) => {
                errors.push(format!("cluster info: {e}"));
                None
            }
        };

        let all_failed = nodes.is_empty()
            && pods.is_empty()
            && services.is_empty()
            && deployments.is_empty()
            && statefulsets.is_empty()
            && daemonsets.is_empty()
            && jobs.is_empty()
            && cronjobs.is_empty()
            && cluster_info.is_none();

        if all_failed {
            let message = if errors.is_empty() {
                "failed to refresh cluster state".to_string()
            } else {
                errors.join(" | ")
            };
            self.snapshot.phase = DataPhase::Error;
            self.snapshot.last_error = Some(message.clone());
            return Err(anyhow!(message));
        }

        let namespaces_count = pods
            .iter()
            .map(|pod| pod.namespace.as_str())
            .collect::<BTreeSet<_>>()
            .len();

        self.snapshot.services_count = services.len();
        self.snapshot.namespaces_count = namespaces_count;
        self.snapshot.nodes = nodes;
        self.snapshot.pods = pods;
        self.snapshot.services = services;
        self.snapshot.deployments = deployments;
        self.snapshot.statefulsets = statefulsets;
        self.snapshot.daemonsets = daemonsets;
        self.snapshot.jobs = jobs;
        self.snapshot.cronjobs = cronjobs;
        self.snapshot.cluster_info = cluster_info;
        self.snapshot.phase = DataPhase::Ready;
        self.snapshot.last_updated = Some(Utc::now());
        self.snapshot.last_error = if errors.is_empty() {
            None
        } else {
            Some(errors.join(" | "))
        };

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct MockDataSource {
        url: String,
        nodes: Vec<NodeInfo>,
        namespaces: Vec<String>,
        pods: Vec<PodInfo>,
        services: Vec<ServiceInfo>,
        deployments: Vec<DeploymentInfo>,
        statefulsets: Vec<StatefulSetInfo>,
        daemonsets: Vec<DaemonSetInfo>,
        jobs: Vec<JobInfo>,
        cronjobs: Vec<CronJobInfo>,
        cluster_info: Option<ClusterInfo>,
        nodes_err: Option<String>,
        pods_err: Option<String>,
        services_err: Option<String>,
        deployments_err: Option<String>,
        statefulsets_err: Option<String>,
        daemonsets_err: Option<String>,
        jobs_err: Option<String>,
        cronjobs_err: Option<String>,
        cluster_info_err: Option<String>,
        delay_ms: u64,
    }

    impl MockDataSource {
        fn success() -> Self {
            Self {
                url: "https://kind.local".to_string(),
                nodes: vec![NodeInfo {
                    name: "n1".to_string(),
                    ready: true,
                    ..NodeInfo::default()
                }],
                namespaces: vec!["default".to_string(), "demo".to_string()],
                pods: vec![
                    PodInfo {
                        name: "p1".to_string(),
                        namespace: "default".to_string(),
                        ..PodInfo::default()
                    },
                    PodInfo {
                        name: "p2".to_string(),
                        namespace: "demo".to_string(),
                        ..PodInfo::default()
                    },
                ],
                services: vec![ServiceInfo {
                    name: "svc".to_string(),
                    namespace: "default".to_string(),
                    ..ServiceInfo::default()
                }],
                deployments: vec![DeploymentInfo {
                    name: "dep".to_string(),
                    namespace: "default".to_string(),
                    ready: "1/1".to_string(),
                    ..DeploymentInfo::default()
                }],
                statefulsets: vec![StatefulSetInfo {
                    name: "db".to_string(),
                    namespace: "default".to_string(),
                    desired_replicas: 1,
                    ready_replicas: 1,
                    service_name: "db-headless".to_string(),
                    pod_management_policy: "OrderedReady".to_string(),
                    ..StatefulSetInfo::default()
                }],
                daemonsets: vec![DaemonSetInfo {
                    name: "agent".to_string(),
                    namespace: "kube-system".to_string(),
                    desired_count: 1,
                    ready_count: 1,
                    unavailable_count: 0,
                    ..DaemonSetInfo::default()
                }],
                jobs: vec![JobInfo {
                    name: "cleanup".to_string(),
                    namespace: "default".to_string(),
                    status: "Running".to_string(),
                    completions: "0/1".to_string(),
                    parallelism: 1,
                    active_pods: 1,
                    ..JobInfo::default()
                }],
                cronjobs: vec![CronJobInfo {
                    name: "nightly".to_string(),
                    namespace: "default".to_string(),
                    schedule: "0 0 * * *".to_string(),
                    active_jobs: 0,
                    ..CronJobInfo::default()
                }],
                cluster_info: Some(ClusterInfo {
                    server: "https://kind.local".to_string(),
                    node_count: 1,
                    ready_nodes: 1,
                    pod_count: 2,
                    ..ClusterInfo::default()
                }),
                nodes_err: None,
                pods_err: None,
                services_err: None,
                deployments_err: None,
                statefulsets_err: None,
                daemonsets_err: None,
                jobs_err: None,
                cronjobs_err: None,
                cluster_info_err: None,
                delay_ms: 0,
            }
        }

        fn with_delay(mut self, delay_ms: u64) -> Self {
            self.delay_ms = delay_ms;
            self
        }
    }

    #[async_trait]
    impl ClusterDataSource for MockDataSource {
        fn cluster_url(&self) -> &str {
            &self.url
        }

        async fn fetch_nodes(&self) -> Result<Vec<NodeInfo>> {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.nodes_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(self.nodes.clone())
        }

        async fn fetch_namespaces(&self) -> Result<Vec<String>> {
            Ok(self.namespaces.clone())
        }

        async fn fetch_pods(&self, _namespace: Option<&str>) -> Result<Vec<PodInfo>> {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.pods_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(self.pods.clone())
        }

        async fn fetch_services(&self, _namespace: Option<&str>) -> Result<Vec<ServiceInfo>> {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.services_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(self.services.clone())
        }

        async fn fetch_deployments(&self, _namespace: Option<&str>) -> Result<Vec<DeploymentInfo>> {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.deployments_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(self.deployments.clone())
        }

        async fn fetch_statefulsets(
            &self,
            _namespace: Option<&str>,
        ) -> Result<Vec<StatefulSetInfo>> {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.statefulsets_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(self.statefulsets.clone())
        }

        async fn fetch_daemonsets(&self, _namespace: Option<&str>) -> Result<Vec<DaemonSetInfo>> {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.daemonsets_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(self.daemonsets.clone())
        }

        async fn fetch_jobs(&self, _namespace: Option<&str>) -> Result<Vec<JobInfo>> {
            if let Some(err) = &self.jobs_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(self.jobs.clone())
        }

        async fn fetch_cronjobs(&self, _namespace: Option<&str>) -> Result<Vec<CronJobInfo>> {
            if let Some(err) = &self.cronjobs_err {
                return Err(anyhow!(err.clone()));
            }
            Ok(self.cronjobs.clone())
        }

        async fn fetch_cluster_info(&self) -> Result<ClusterInfo> {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if let Some(err) = &self.cluster_info_err {
                return Err(anyhow!(err.clone()));
            }
            self.cluster_info
                .clone()
                .ok_or_else(|| anyhow!("cluster info missing"))
        }
    }

    #[tokio::test]
    async fn refresh_success_populates_snapshot() {
        let mut state = GlobalState::default();
        let source = MockDataSource::success();

        state
            .refresh(&source, None)
            .await
            .expect("refresh should succeed");
        let snapshot = state.snapshot();

        assert_eq!(snapshot.phase, DataPhase::Ready);
        assert_eq!(snapshot.nodes.len(), 1);
        assert_eq!(snapshot.pods.len(), 2);
        assert_eq!(snapshot.services_count, 1);
        assert_eq!(snapshot.namespaces_count, 2);
        assert_eq!(snapshot.statefulsets.len(), 1);
        assert_eq!(snapshot.daemonsets.len(), 1);
        assert_eq!(snapshot.jobs.len(), 1);
        assert_eq!(snapshot.cronjobs.len(), 1);
        assert_eq!(snapshot.cluster_summary(), "https://kind.local");
        assert_eq!(state.namespaces, vec!["default", "demo"]);
        assert!(snapshot.last_updated.is_some());
    }

    #[tokio::test]
    async fn refresh_partial_failure_degrades_gracefully() {
        let mut state = GlobalState::default();
        let mut source = MockDataSource::success();
        source.services_err = Some("forbidden".to_string());

        state
            .refresh(&source, None)
            .await
            .expect("partial failure should still return ready state");

        let snapshot = state.snapshot();
        assert_eq!(snapshot.phase, DataPhase::Ready);
        assert!(snapshot.services.is_empty());
        assert!(
            snapshot
                .last_error
                .as_deref()
                .unwrap_or_default()
                .contains("services")
        );
    }

    #[tokio::test]
    async fn refresh_all_fail_sets_error_phase() {
        let mut state = GlobalState::default();
        let source = MockDataSource {
            url: "https://broken".to_string(),
            nodes: vec![],
            namespaces: vec![],
            pods: vec![],
            services: vec![],
            deployments: vec![],
            statefulsets: vec![],
            daemonsets: vec![],
            jobs: vec![],
            cronjobs: vec![],
            cluster_info: None,
            nodes_err: Some("nodes down".to_string()),
            pods_err: Some("pods down".to_string()),
            services_err: Some("services down".to_string()),
            deployments_err: Some("deployments down".to_string()),
            statefulsets_err: Some("statefulsets down".to_string()),
            daemonsets_err: Some("daemonsets down".to_string()),
            jobs_err: Some("jobs down".to_string()),
            cronjobs_err: Some("cronjobs down".to_string()),
            cluster_info_err: Some("cluster down".to_string()),
            delay_ms: 0,
        };

        let result = state.refresh(&source, None).await;
        assert!(result.is_err());

        let snapshot = state.snapshot();
        assert_eq!(snapshot.phase, DataPhase::Error);
        assert!(snapshot.last_error.is_some());
    }

    #[tokio::test]
    async fn refresh_handles_empty_resources_without_crashing() {
        let mut state = GlobalState::default();
        let source = MockDataSource {
            nodes: vec![],
            pods: vec![],
            services: vec![],
            deployments: vec![],
            statefulsets: vec![],
            daemonsets: vec![],
            jobs: vec![],
            cronjobs: vec![],
            cluster_info: Some(ClusterInfo {
                server: "https://kind.local".to_string(),
                ..ClusterInfo::default()
            }),
            ..MockDataSource::success()
        };

        state
            .refresh(&source, None)
            .await
            .expect("empty lists are valid");
        let snapshot = state.snapshot();

        assert_eq!(snapshot.phase, DataPhase::Ready);
        assert_eq!(snapshot.nodes.len(), 0);
        assert_eq!(snapshot.pods.len(), 0);
        assert_eq!(snapshot.services_count, 0);
        assert_eq!(snapshot.namespaces_count, 0);
        assert!(snapshot.jobs.is_empty());
        assert!(snapshot.cronjobs.is_empty());
    }

    #[tokio::test]
    async fn fetch_with_timeout_returns_timeout_error() {
        let result = GlobalState::fetch_with_timeout("nodes", async {
            tokio::time::sleep(Duration::from_millis(5200)).await;
            Ok::<Vec<NodeInfo>, anyhow::Error>(vec![])
        })
        .await;

        assert!(result.is_err());
        assert!(
            format!("{}", result.expect_err("must timeout")).contains("timed out fetching nodes")
        );
    }

    #[tokio::test]
    async fn refresh_skips_when_already_loading() {
        let mut state = GlobalState::default();
        state.snapshot.phase = DataPhase::Loading;
        let source = MockDataSource::success().with_delay(100);

        state
            .refresh(&source, None)
            .await
            .expect("loading guard should no-op");

        assert_eq!(state.snapshot().phase, DataPhase::Loading);
    }

    /// Verifies StatefulSet DTO default values are stable.
    #[test]
    fn test_statefulset_info_defaults() {
        let info = StatefulSetInfo::default();
        assert_eq!(info.name, "");
        assert_eq!(info.namespace, "");
        assert_eq!(info.desired_replicas, 0);
        assert_eq!(info.ready_replicas, 0);
        assert!(info.image.is_none());
        assert!(info.created_at.is_none());
    }

    /// Verifies DaemonSet ready calculation semantics used by UI.
    #[test]
    fn test_daemonset_info_ready_calculation() {
        let info = DaemonSetInfo {
            desired_count: 8,
            ready_count: 5,
            unavailable_count: 3,
            ..DaemonSetInfo::default()
        };

        assert_eq!(
            info.desired_count - info.ready_count,
            info.unavailable_count
        );
    }
}
