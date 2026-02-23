//! Pod logs streaming client for KubecTUI.

use anyhow::Context;
use k8s_openapi::api::core::v1::Pod;
use kube::{Api, Client};

/// Pod reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodRef {
    pub name: String,
    pub namespace: String,
}

impl PodRef {
    pub fn new(name: String, namespace: String) -> Self {
        Self { name, namespace }
    }
}

/// Logs client.
#[derive(Clone)]
pub struct LogsClient {
    client: Client,
}

impl LogsClient {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn tail_logs(
        &self,
        pod_ref: &PodRef,
        _tail_lines: Option<i64>,
    ) -> anyhow::Result<Vec<String>> {
        self.verify_pod_exists(pod_ref).await?;
        let _pods: Api<Pod> = Api::namespaced(self.client.clone(), &pod_ref.namespace);
        Ok(vec![])
    }

    async fn verify_pod_exists(&self, pod_ref: &PodRef) -> anyhow::Result<()> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &pod_ref.namespace);
        pods.get(&pod_ref.name)
            .await
            .context("Pod not found")?;
        Ok(())
    }
}
