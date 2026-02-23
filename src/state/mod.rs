//! Global state management for KubecTUI.

pub mod alerts;
pub mod filters;

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::{collections::BTreeSet, fmt};

use crate::k8s::{
    client::K8sClient,
    dtos::{ClusterInfo, DeploymentInfo, NodeInfo, PodInfo, ServiceInfo},
};

/// High-level data loading phase for cluster resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DataPhase {
    #[default]
    Idle,
    Loading,
    Ready,
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
    pub fn cluster_summary(&self) -> &str {
        self.cluster_url
            .as_deref()
            .unwrap_or("Cluster endpoint unavailable")
    }
}

#[derive(Debug, Clone, Default)]
pub struct GlobalState {
    snapshot: ClusterSnapshot,
}

impl GlobalState {
    pub fn snapshot(&self) -> ClusterSnapshot {
        self.snapshot.clone()
    }

    pub async fn refresh(&mut self, client: &K8sClient) -> Result<()> {
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
