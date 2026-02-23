//! Global state management for KubecTUI.

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::fmt;

use crate::k8s::client::{K8sClient, NodeInfo, PodInfo};

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

    /// Refreshes nodes and pods in parallel, updating status and timestamps.
    pub async fn refresh(&mut self, client: &K8sClient) -> Result<()> {
        self.snapshot.phase = DataPhase::Loading;
        self.snapshot.last_error = None;
        self.snapshot.cluster_url = Some(client.cluster_url().to_string());

        let refresh_result = async {
            let (nodes, pods) = tokio::try_join!(client.fetch_nodes(), client.fetch_pods(None))?;
            Ok::<(Vec<NodeInfo>, Vec<PodInfo>), anyhow::Error>((nodes, pods))
        }
        .await;

        match refresh_result {
            Ok((nodes, pods)) => {
                self.snapshot.nodes = nodes;
                self.snapshot.pods = pods;
                self.snapshot.phase = DataPhase::Ready;
                self.snapshot.last_updated = Some(Utc::now());
                Ok(())
            }
            Err(err) => {
                self.snapshot.phase = DataPhase::Error;
                self.snapshot.last_error = Some(err.to_string());
                Err(err)
            }
        }
    }
}
