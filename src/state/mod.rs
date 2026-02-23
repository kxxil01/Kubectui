//! Global state management for KubecTUI.

pub mod alerts;
pub mod filters;
pub mod port_forward;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::{collections::BTreeSet, fmt};

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

    /// Refreshes core resources in parallel, updating status and timestamps.
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

        let (nodes, pods, services, deployments, cluster_info) = match tokio::try_join!(
            client.fetch_nodes(),
            client.fetch_pods(None),
            client.fetch_services(None),
            client.fetch_deployments(None),
            client.fetch_cluster_info(),
        ) {
            Ok(data) => data,
            Err(err) => {
                self.snapshot.phase = DataPhase::Error;
                self.snapshot.last_error = Some(err.to_string());
                return Err(err);
            }
        };

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
        self.snapshot.cluster_info = Some(cluster_info);
        self.snapshot.phase = DataPhase::Ready;
        self.snapshot.last_updated = Some(Utc::now());

        Ok(())
    }
}
