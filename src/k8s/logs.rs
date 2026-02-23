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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pod_ref_constructor_sets_fields() {
        let pod = PodRef::new("my-pod".to_string(), "default".to_string());
        assert_eq!(pod.name, "my-pod");
        assert_eq!(pod.namespace, "default");
    }

    #[tokio::test]
    async fn verify_pod_exists_returns_context_when_missing() {
        let cfg = kube::Config::new("http://127.0.0.1:1".parse().expect("valid URL"));
        let client = Client::try_from(cfg).expect("test client should initialize");
        let logs_client = LogsClient::new(client);
        let pod_ref = PodRef::new("missing-pod".to_string(), "default".to_string());

        let err = logs_client
            .verify_pod_exists(&pod_ref)
            .await
            .expect_err("missing pod should error");

        assert!(format!("{err:#}").contains("Pod not found"));
    }

    #[tokio::test]
    async fn tail_logs_propagates_verify_errors() {
        let cfg = kube::Config::new("http://127.0.0.1:1".parse().expect("valid URL"));
        let client = Client::try_from(cfg).expect("test client should initialize");
        let logs_client = LogsClient::new(client);
        let pod_ref = PodRef::new("missing-pod".to_string(), "default".to_string());

        let err = logs_client
            .tail_logs(&pod_ref, Some(10))
            .await
            .expect_err("tail_logs should fail for missing pod");

        let text = format!("{err:#}");
        assert!(text.contains("Pod not found"));
    }
}

