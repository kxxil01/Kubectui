//! Kubernetes API client wrapper used by KubecTUI.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::{Api, Client, Config, api::ListParams, config::KubeConfigOptions};
use serde::{Deserialize, Serialize};

/// Lightweight node view used by state management and rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub name: String,
    pub ready: bool,
    pub kubelet_version: String,
    pub os_image: String,
}

/// Lightweight pod view used by state management and rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodInfo {
    pub name: String,
    pub namespace: String,
    pub status: String,
    pub node: Option<String>,
    pub restarts: i32,
    pub created_at: Option<DateTime<Utc>>,
}

/// Configured Kubernetes client wrapper.
#[derive(Clone)]
pub struct K8sClient {
    client: Client,
    cluster_url: String,
}

impl K8sClient {
    /// Creates a Kubernetes client from `~/.kube/config` when available,
    /// then falls back to ambient/in-cluster configuration.
    pub async fn connect() -> Result<Self> {
        let config = match Config::from_kubeconfig(&KubeConfigOptions::default()).await {
            Ok(cfg) => cfg,
            Err(kubeconfig_err) => Config::infer().await.with_context(|| {
                format!(
                    "failed loading kubeconfig from ~/.kube/config and failed inferring config: {kubeconfig_err}"
                )
            })?,
        };

        let cluster_url = config.cluster_url.to_string();
        let client = Client::try_from(config).context("failed to build kube client")?;

        Ok(Self {
            client,
            cluster_url,
        })
    }

    /// Returns the configured Kubernetes cluster API endpoint.
    pub fn cluster_url(&self) -> &str {
        &self.cluster_url
    }

    /// Fetches all nodes from the current cluster.
    pub async fn fetch_nodes(&self) -> Result<Vec<NodeInfo>> {
        let nodes_api: Api<Node> = Api::all(self.client.clone());
        let list = nodes_api
            .list(&ListParams::default())
            .await
            .context("failed fetching Kubernetes nodes")?;

        let nodes = list
            .into_iter()
            .map(|node| {
                let ready = node_ready(&node);
                NodeInfo {
                    name: node
                        .metadata
                        .name
                        .unwrap_or_else(|| "<unknown>".to_string()),
                    ready,
                    kubelet_version: node
                        .status
                        .as_ref()
                        .and_then(|status| status.node_info.as_ref())
                        .map(|info| info.kubelet_version.clone())
                        .unwrap_or_else(|| "unknown".to_string()),
                    os_image: node
                        .status
                        .as_ref()
                        .and_then(|status| status.node_info.as_ref())
                        .map(|info| info.os_image.clone())
                        .unwrap_or_else(|| "unknown".to_string()),
                }
            })
            .collect();

        Ok(nodes)
    }

    /// Fetches pods from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_pods(&self, namespace: Option<&str>) -> Result<Vec<PodInfo>> {
        let pods_api: Api<Pod> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = pods_api
            .list(&ListParams::default())
            .await
            .with_context(|| {
                if let Some(ns) = namespace {
                    format!("failed fetching pods in namespace '{ns}'")
                } else {
                    "failed fetching pods across all namespaces".to_string()
                }
            })?;

        let pods = list
            .into_iter()
            .map(|pod| {
                let restarts = pod
                    .status
                    .as_ref()
                    .and_then(|status| status.container_statuses.as_ref())
                    .map(|statuses| statuses.iter().map(|s| s.restart_count).sum())
                    .unwrap_or_default();

                PodInfo {
                    name: pod.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: pod
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    status: pod
                        .status
                        .as_ref()
                        .and_then(|status| status.phase.clone())
                        .unwrap_or_else(|| "Unknown".to_string()),
                    node: pod.spec.as_ref().and_then(|spec| spec.node_name.clone()),
                    restarts,
                    created_at: pod.metadata.creation_timestamp.as_ref().map(|ts| ts.0),
                }
            })
            .collect();

        Ok(pods)
    }
}

fn node_ready(node: &Node) -> bool {
    node.status
        .as_ref()
        .and_then(|status| status.conditions.as_ref())
        .and_then(|conditions| {
            conditions
                .iter()
                .find(|condition| condition.type_ == "Ready")
        })
        .is_some_and(|condition| condition.status == "True")
}
