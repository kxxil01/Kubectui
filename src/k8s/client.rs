//! Kubernetes API client wrapper used by KubecTUI.

use anyhow::{Context, Result};
use chrono::Utc;
use k8s_openapi::api::{
    apps::v1::Deployment,
    core::v1::{Node, Pod, Service},
};
use kube::{Api, Client, Config, api::ListParams, config::KubeConfigOptions};

use crate::k8s::{events, yaml};

pub use crate::k8s::{
    dtos::{ClusterInfo, DeploymentInfo, NodeInfo, PodInfo, ServiceInfo},
    events::EventInfo,
};

/// Configured Kubernetes client wrapper.
#[derive(Clone)]
pub struct K8sClient {
    client: Client,
    cluster_url: String,
    cluster_context: Option<String>,
}

impl K8sClient {
    /// Creates a Kubernetes client from `~/.kube/config` when available,
    /// then falls back to ambient/in-cluster configuration.
    pub async fn connect() -> Result<Self> {
        let cluster_context = kube::config::Kubeconfig::read()
            .ok()
            .and_then(|cfg| cfg.current_context);

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
            cluster_context,
        })
    }

    /// Returns the configured Kubernetes cluster API endpoint.
    pub fn cluster_url(&self) -> &str {
        &self.cluster_url
    }

    /// Returns reference to the underlying Kubernetes client.
    pub fn get_client(&self) -> Client {
        self.client.clone()
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
                let alloc = node
                    .status
                    .as_ref()
                    .and_then(|status| status.allocatable.as_ref());
                let name = node
                    .metadata
                    .name
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| "<unknown>".to_string());

                NodeInfo {
                    name,
                    ready: node_condition_true(&node, "Ready"),
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
                    role: node_role(&node),
                    cpu_allocatable: alloc.and_then(|a| a.get("cpu").map(|q| q.0.clone())),
                    memory_allocatable: alloc.and_then(|a| a.get("memory").map(|q| q.0.clone())),
                    created_at: node.metadata.creation_timestamp.as_ref().map(|ts| ts.0),
                    memory_pressure: node_condition_true(&node, "MemoryPressure"),
                    disk_pressure: node_condition_true(&node, "DiskPressure"),
                    pid_pressure: node_condition_true(&node, "PIDPressure"),
                    network_unavailable: node_condition_true(&node, "NetworkUnavailable"),
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
                let container_statuses = pod
                    .status
                    .as_ref()
                    .and_then(|status| status.container_statuses.as_ref())
                    .cloned()
                    .unwrap_or_default();

                let waiting_reasons = container_statuses
                    .iter()
                    .filter_map(|status| status.state.as_ref())
                    .filter_map(|state| state.waiting.as_ref())
                    .filter_map(|waiting| waiting.reason.clone())
                    .collect::<Vec<_>>();

                let restarts = container_statuses.iter().map(|s| s.restart_count).sum();

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
                    pod_ip: pod.status.as_ref().and_then(|status| status.pod_ip.clone()),
                    restarts,
                    created_at: pod.metadata.creation_timestamp.as_ref().map(|ts| ts.0),
                    labels: pod
                        .metadata
                        .labels
                        .unwrap_or_default()
                        .into_iter()
                        .collect(),
                    waiting_reasons,
                }
            })
            .collect();

        Ok(pods)
    }

    /// Fetches services from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_services(&self, namespace: Option<&str>) -> Result<Vec<ServiceInfo>> {
        let services_api: Api<Service> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = services_api
            .list(&ListParams::default())
            .await
            .with_context(|| {
                if let Some(ns) = namespace {
                    format!("failed fetching services in namespace '{ns}'")
                } else {
                    "failed fetching services across all namespaces".to_string()
                }
            })?;

        let now = Utc::now();
        let services = list
            .into_iter()
            .map(|svc| {
                let ports = svc
                    .spec
                    .as_ref()
                    .and_then(|spec| spec.ports.as_ref())
                    .map(|ports| {
                        ports
                            .iter()
                            .map(|p| {
                                format!(
                                    "{}/{}",
                                    p.port,
                                    p.protocol.clone().unwrap_or_else(|| "TCP".to_string())
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let service_type = svc
                    .spec
                    .as_ref()
                    .and_then(|spec| spec.type_.clone())
                    .unwrap_or_else(|| "ClusterIP".to_string());

                let created_at = svc.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

                ServiceInfo {
                    name: svc.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: svc
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    type_: service_type.clone(),
                    service_type,
                    cluster_ip: svc.spec.as_ref().and_then(|spec| spec.cluster_ip.clone()),
                    ports,
                    created_at,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                }
            })
            .collect();

        Ok(services)
    }

    /// Fetches deployments from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_deployments(&self, namespace: Option<&str>) -> Result<Vec<DeploymentInfo>> {
        let deployments_api: Api<Deployment> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = deployments_api
            .list(&ListParams::default())
            .await
            .with_context(|| {
                if let Some(ns) = namespace {
                    format!("failed fetching deployments in namespace '{ns}'")
                } else {
                    "failed fetching deployments across all namespaces".to_string()
                }
            })?;

        let now = Utc::now();
        let deployments = list
            .into_iter()
            .map(|dep| {
                let desired_replicas = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
                let ready_replicas = dep
                    .status
                    .as_ref()
                    .and_then(|s| s.ready_replicas)
                    .unwrap_or(0);
                let available_replicas = dep
                    .status
                    .as_ref()
                    .and_then(|s| s.available_replicas)
                    .unwrap_or(0);
                let updated_replicas = dep
                    .status
                    .as_ref()
                    .and_then(|s| s.updated_replicas)
                    .unwrap_or(0);

                let created_at = dep.metadata.creation_timestamp.as_ref().map(|ts| ts.0);
                let image = dep
                    .spec
                    .as_ref()
                    .and_then(|spec| spec.template.spec.as_ref())
                    .and_then(|spec| spec.containers.first())
                    .and_then(|container| container.image.clone());

                DeploymentInfo {
                    name: dep.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: dep
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    desired_replicas,
                    ready_replicas,
                    available_replicas,
                    updated_replicas,
                    created_at,
                    ready: format!("{ready_replicas}/{desired_replicas}"),
                    updated: updated_replicas,
                    available: available_replicas,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    image,
                }
            })
            .collect();

        Ok(deployments)
    }

    /// Fetches cluster summary information.
    pub async fn fetch_cluster_info(&self) -> Result<ClusterInfo> {
        let nodes = self
            .fetch_nodes()
            .await
            .context("failed gathering node list for cluster summary")?;
        let pods = self
            .fetch_pods(None)
            .await
            .context("failed gathering pod list for cluster summary")?;

        let ready_nodes = nodes.iter().filter(|node| node.ready).count();
        let version = self
            .client
            .apiserver_version()
            .await
            .context("failed fetching API server version")?;

        Ok(ClusterInfo {
            context: self.cluster_context.clone(),
            server: self.cluster_url.clone(),
            git_version: Some(version.git_version),
            platform: Some(version.platform),
            node_count: nodes.len(),
            ready_nodes,
            pod_count: pods.len(),
        })
    }

    /// Fetches a concrete resource and renders it as YAML.
    pub async fn fetch_resource_yaml(
        &self,
        kind: &str,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<String> {
        yaml::get_resource_yaml(&self.client, kind, name, namespace)
            .await
            .with_context(|| {
                format!(
                    "failed preparing YAML for kind='{kind}' name='{name}' namespace='{}'",
                    namespace.unwrap_or("<cluster-scope>")
                )
            })
    }

    /// Fetches pod events and degrades gracefully when RBAC denies access.
    pub async fn fetch_pod_events(&self, name: &str, namespace: &str) -> Result<Vec<EventInfo>> {
        events::fetch_pod_events(&self.client, name, namespace)
            .await
            .with_context(|| format!("failed preparing events for pod '{namespace}/{name}'"))
    }
}

fn node_condition_true(node: &Node, condition_type: &str) -> bool {
    node.status
        .as_ref()
        .and_then(|status| status.conditions.as_ref())
        .and_then(|conditions| {
            conditions
                .iter()
                .find(|condition| condition.type_ == condition_type)
        })
        .is_some_and(|condition| condition.status == "True")
}

fn node_role(node: &Node) -> String {
    let labels = node.metadata.labels.as_ref();

    let is_control_plane = labels.is_some_and(|labels| {
        labels.contains_key("node-role.kubernetes.io/control-plane")
            || labels.contains_key("node-role.kubernetes.io/master")
    });

    if is_control_plane {
        "master".to_string()
    } else {
        "worker".to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::api::core::v1::{NodeCondition, NodeStatus};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    use super::*;

    fn node_with_condition(condition_type: &str, status: &str) -> Node {
        Node {
            metadata: ObjectMeta::default(),
            status: Some(NodeStatus {
                conditions: Some(vec![NodeCondition {
                    type_: condition_type.to_string(),
                    status: status.to_string(),
                    ..NodeCondition::default()
                }]),
                ..NodeStatus::default()
            }),
            ..Node::default()
        }
    }

    /// Verifies node readiness helper returns true only for matching True condition.
    #[test]
    fn node_condition_true_matches_expected_condition() {
        let ready_node = node_with_condition("Ready", "True");
        let not_ready_node = node_with_condition("Ready", "False");

        assert!(node_condition_true(&ready_node, "Ready"));
        assert!(!node_condition_true(&not_ready_node, "Ready"));
    }

    /// Verifies unknown condition types are treated as false.
    #[test]
    fn node_condition_true_unknown_type_is_false() {
        let node = node_with_condition("DiskPressure", "True");
        assert!(!node_condition_true(&node, "Ready"));
    }

    /// Verifies control-plane labels map to master role.
    #[test]
    fn node_role_detects_master_from_control_plane_label() {
        let mut labels = BTreeMap::new();
        labels.insert(
            "node-role.kubernetes.io/control-plane".to_string(),
            "".to_string(),
        );

        let node = Node {
            metadata: ObjectMeta {
                labels: Some(labels),
                ..ObjectMeta::default()
            },
            ..Node::default()
        };

        assert_eq!(node_role(&node), "master");
    }

    /// Verifies nodes without control-plane labels default to worker role.
    #[test]
    fn node_role_defaults_to_worker() {
        let node = Node::default();
        assert_eq!(node_role(&node), "worker");
    }

    /// Verifies node mapping preserves defaults for missing metadata fields.
    #[test]
    fn fetch_nodes_mapping_handles_missing_fields() {
        let node = Node::default();
        let info = NodeInfo {
            name: node
                .metadata
                .name
                .clone()
                .unwrap_or_else(|| "<unknown>".to_string()),
            ready: node_condition_true(&node, "Ready"),
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
            role: node_role(&node),
            cpu_allocatable: None,
            memory_allocatable: None,
            created_at: None,
            memory_pressure: node_condition_true(&node, "MemoryPressure"),
            disk_pressure: node_condition_true(&node, "DiskPressure"),
            pid_pressure: node_condition_true(&node, "PIDPressure"),
            network_unavailable: node_condition_true(&node, "NetworkUnavailable"),
        };

        assert_eq!(info.name, "<unknown>");
        assert_eq!(info.kubelet_version, "unknown");
        assert_eq!(info.os_image, "unknown");
        assert_eq!(info.role, "worker");
    }

    /// Verifies invalid resource kind in YAML fetch returns descriptive error.
    #[tokio::test]
    async fn fetch_resource_yaml_invalid_kind_has_clear_error() {
        let cfg = kube::Config::new("http://127.0.0.1:1".parse().expect("valid URL"));
        let client = Client::try_from(cfg).expect("client should build for test URL");

        let k8s = K8sClient {
            client,
            cluster_url: "http://127.0.0.1:1".to_string(),
            cluster_context: Some("test".to_string()),
        };

        let err = k8s
            .fetch_resource_yaml("unsupported", "name", None)
            .await
            .expect_err("invalid kind should error");

        let err_text = format!("{err:#}");
        assert!(
            err_text.contains("failed preparing YAML") && err_text.contains("unsupported"),
            "error should include context and root cause, got: {err_text}"
        );
    }
}
