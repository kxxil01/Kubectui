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
    dtos::{ClusterInfo, DeploymentInfo, NodeInfo, PodInfo, ServiceInfo},
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
    /// Fetches pod list.
    async fn fetch_pods(&self, namespace: Option<&str>) -> Result<Vec<PodInfo>>;
    /// Fetches service list.
    async fn fetch_services(&self, namespace: Option<&str>) -> Result<Vec<ServiceInfo>>;
    /// Fetches deployment list.
    async fn fetch_deployments(&self, namespace: Option<&str>) -> Result<Vec<DeploymentInfo>>;
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

    async fn fetch_pods(&self, namespace: Option<&str>) -> Result<Vec<PodInfo>> {
        K8sClient::fetch_pods(self, namespace).await
    }

    async fn fetch_services(&self, namespace: Option<&str>) -> Result<Vec<ServiceInfo>> {
        K8sClient::fetch_services(self, namespace).await
    }

    async fn fetch_deployments(&self, namespace: Option<&str>) -> Result<Vec<DeploymentInfo>> {
        K8sClient::fetch_deployments(self, namespace).await
    }

    async fn fetch_cluster_info(&self) -> Result<ClusterInfo> {
        K8sClient::fetch_cluster_info(self).await
    }
}

/// Mutable state holder with async refresh operations.
#[derive(Debug, Clone, Default)]
pub struct GlobalState {
    snapshot: ClusterSnapshot,
}

impl GlobalState {
    /// Returns a cloneable immutable snapshot for UI rendering.
    pub fn snapshot(&self) -> ClusterSnapshot {
        self.snapshot.clone()
    }

    async fn fetch_with_timeout<T>(label: &'static str, fut: impl std::future::Future<Output = Result<T>>) -> Result<T> {
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
    pub async fn refresh<D>(&mut self, client: &D) -> Result<()>
    where
        D: ClusterDataSource + Sync,
    {
        if self.snapshot.phase == DataPhase::Loading {
            return Ok(());
        }

        self.snapshot.phase = DataPhase::Loading;
        self.snapshot.last_error = None;
        self.snapshot.cluster_url = Some(client.cluster_url().to_string());

        let (nodes_res, pods_res, services_res, deployments_res, cluster_info_res) = tokio::join!(
            Self::fetch_with_timeout("nodes", client.fetch_nodes()),
            Self::fetch_with_timeout("pods", client.fetch_pods(None)),
            Self::fetch_with_timeout("services", client.fetch_services(None)),
            Self::fetch_with_timeout("deployments", client.fetch_deployments(None)),
            Self::fetch_with_timeout("cluster info", client.fetch_cluster_info()),
        );

        let mut errors = Vec::new();

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
        pods: Vec<PodInfo>,
        services: Vec<ServiceInfo>,
        deployments: Vec<DeploymentInfo>,
        cluster_info: Option<ClusterInfo>,
        nodes_err: Option<String>,
        pods_err: Option<String>,
        services_err: Option<String>,
        deployments_err: Option<String>,
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

        state.refresh(&source).await.expect("refresh should succeed");
        let snapshot = state.snapshot();

        assert_eq!(snapshot.phase, DataPhase::Ready);
        assert_eq!(snapshot.nodes.len(), 1);
        assert_eq!(snapshot.pods.len(), 2);
        assert_eq!(snapshot.services_count, 1);
        assert_eq!(snapshot.namespaces_count, 2);
        assert_eq!(snapshot.cluster_summary(), "https://kind.local");
        assert!(snapshot.last_updated.is_some());
    }

    #[tokio::test]
    async fn refresh_partial_failure_degrades_gracefully() {
        let mut state = GlobalState::default();
        let mut source = MockDataSource::success();
        source.services_err = Some("forbidden".to_string());

        state
            .refresh(&source)
            .await
            .expect("partial failure should still return ready state");

        let snapshot = state.snapshot();
        assert_eq!(snapshot.phase, DataPhase::Ready);
        assert!(snapshot.services.is_empty());
        assert!(snapshot
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("services"));
    }

    #[tokio::test]
    async fn refresh_all_fail_sets_error_phase() {
        let mut state = GlobalState::default();
        let source = MockDataSource {
            url: "https://broken".to_string(),
            nodes: vec![],
            pods: vec![],
            services: vec![],
            deployments: vec![],
            cluster_info: None,
            nodes_err: Some("nodes down".to_string()),
            pods_err: Some("pods down".to_string()),
            services_err: Some("services down".to_string()),
            deployments_err: Some("deployments down".to_string()),
            cluster_info_err: Some("cluster down".to_string()),
            delay_ms: 0,
        };

        let result = state.refresh(&source).await;
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
            cluster_info: Some(ClusterInfo {
                server: "https://kind.local".to_string(),
                ..ClusterInfo::default()
            }),
            ..MockDataSource::success()
        };

        state.refresh(&source).await.expect("empty lists are valid");
        let snapshot = state.snapshot();

        assert_eq!(snapshot.phase, DataPhase::Ready);
        assert_eq!(snapshot.nodes.len(), 0);
        assert_eq!(snapshot.pods.len(), 0);
        assert_eq!(snapshot.services_count, 0);
        assert_eq!(snapshot.namespaces_count, 0);
    }

    #[tokio::test]
    async fn fetch_with_timeout_returns_timeout_error() {
        let result = GlobalState::fetch_with_timeout("nodes", async {
            tokio::time::sleep(Duration::from_millis(5200)).await;
            Ok::<Vec<NodeInfo>, anyhow::Error>(vec![])
        })
        .await;

        assert!(result.is_err());
        assert!(format!("{}", result.expect_err("must timeout")).contains("timed out fetching nodes"));
    }

    #[tokio::test]
    async fn refresh_skips_when_already_loading() {
        let mut state = GlobalState::default();
        state.snapshot.phase = DataPhase::Loading;
        let source = MockDataSource::success().with_delay(100);

        state
            .refresh(&source)
            .await
            .expect("loading guard should no-op");

        assert_eq!(state.snapshot().phase, DataPhase::Loading);
    }
}

