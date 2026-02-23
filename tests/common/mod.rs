//! Shared test helpers for integration tests.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use kubectui::{
    k8s::dtos::{ClusterInfo, DeploymentInfo, NodeInfo, PodInfo, ServiceInfo},
    state::ClusterDataSource,
};

pub fn make_node(name: &str, ready: bool, role: &str) -> NodeInfo {
    NodeInfo {
        name: name.to_string(),
        ready,
        role: role.to_string(),
        ..NodeInfo::default()
    }
}

pub fn make_service(name: &str, namespace: &str, type_: &str) -> ServiceInfo {
    ServiceInfo {
        name: name.to_string(),
        namespace: namespace.to_string(),
        type_: type_.to_string(),
        service_type: type_.to_string(),
        ports: vec!["80/TCP".to_string()],
        ..ServiceInfo::default()
    }
}

pub fn make_deployment(name: &str, namespace: &str, ready: &str) -> DeploymentInfo {
    DeploymentInfo {
        name: name.to_string(),
        namespace: namespace.to_string(),
        ready: ready.to_string(),
        ..DeploymentInfo::default()
    }
}

pub fn make_pod(name: &str, namespace: &str, status: &str) -> PodInfo {
    PodInfo {
        name: name.to_string(),
        namespace: namespace.to_string(),
        status: status.to_string(),
        ..PodInfo::default()
    }
}

#[derive(Clone)]
pub struct MockDataSource {
    pub url: String,
    pub nodes: Vec<NodeInfo>,
    pub pods: Vec<PodInfo>,
    pub services: Vec<ServiceInfo>,
    pub deployments: Vec<DeploymentInfo>,
    pub fail: bool,
    pub calls: Arc<AtomicUsize>,
}

impl Default for MockDataSource {
    fn default() -> Self {
        Self {
            url: "https://mock.cluster".to_string(),
            nodes: vec![make_node("n1", true, "worker")],
            pods: vec![make_pod("p1", "default", "Running")],
            services: vec![make_service("svc1", "default", "ClusterIP")],
            deployments: vec![make_deployment("dep1", "default", "1/1")],
            fail: false,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl ClusterDataSource for MockDataSource {
    fn cluster_url(&self) -> &str {
        &self.url
    }

    async fn fetch_nodes(&self) -> Result<Vec<NodeInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock nodes error"));
        }
        Ok(self.nodes.clone())
    }

    async fn fetch_pods(&self, _namespace: Option<&str>) -> Result<Vec<PodInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock pods error"));
        }
        Ok(self.pods.clone())
    }

    async fn fetch_services(&self, _namespace: Option<&str>) -> Result<Vec<ServiceInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock services error"));
        }
        Ok(self.services.clone())
    }

    async fn fetch_deployments(&self, _namespace: Option<&str>) -> Result<Vec<DeploymentInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock deployments error"));
        }
        Ok(self.deployments.clone())
    }

    async fn fetch_cluster_info(&self) -> Result<ClusterInfo> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock cluster info error"));
        }
        Ok(ClusterInfo {
            context: Some("mock".to_string()),
            server: self.url.clone(),
            git_version: Some("v1.30.0".to_string()),
            platform: Some("linux/amd64".to_string()),
            node_count: self.nodes.len(),
            ready_nodes: self.nodes.iter().filter(|n| n.ready).count(),
            pod_count: self.pods.len(),
        })
    }
}
