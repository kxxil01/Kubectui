//! Kubernetes deployment scaling operations with progress tracking.

use crate::k8s::client::K8sClient;
use anyhow::{Context, Result, anyhow};
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
use kube::{Api, api::Patch, api::PatchParams};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Error types for scaling operations.
#[derive(Debug, Clone)]
pub enum ScaleError {
    DeploymentNotFound(String, String),
    InvalidReplicaCount(i32),
    ApiError(String),
    Timeout(String),
    Cancelled,
}

impl fmt::Display for ScaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScaleError::DeploymentNotFound(name, ns) => {
                write!(f, "Deployment '{}' not found in namespace '{}'", name, ns)
            }
            ScaleError::InvalidReplicaCount(count) => {
                write!(
                    f,
                    "Invalid replica count {}: must be between 0 and 100",
                    count
                )
            }
            ScaleError::ApiError(msg) => write!(f, "API error: {}", msg),
            ScaleError::Timeout(msg) => write!(f, "Timeout: {}", msg),
            ScaleError::Cancelled => write!(f, "Scale operation cancelled"),
        }
    }
}

impl std::error::Error for ScaleError {}

/// Progress update for a scale operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScaleProgress {
    /// Operation initiated
    Initiated,
    /// Scaling started, waiting for replicas
    Scaling { current: i32, target: i32 },
    /// Operation succeeded
    Success { current: i32, target: i32 },
    /// Operation failed
    Error(String),
    /// Operation cancelled by user
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleRequest {
    pub deployment: String,
    pub namespace: String,
    pub replicas: i32,
}

impl ScaleRequest {
    pub fn new(deployment: impl Into<String>, namespace: impl Into<String>, replicas: i32) -> Self {
        Self {
            deployment: deployment.into(),
            namespace: namespace.into(),
            replicas,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.replicas < 0 || self.replicas > 100 {
            return Err(anyhow!(
                "invalid replica count {}: must be between 0 and 100",
                self.replicas
            ));
        }
        Ok(())
    }
}

impl K8sClient {
    pub async fn scale_deployment(&self, name: &str, namespace: &str, replicas: i32) -> Result<()> {
        if !(0..=100).contains(&replicas) {
            return Err(anyhow!(
                "invalid replica count {}: must be between 0 and 100",
                replicas
            ));
        }
        let client = self.get_client();
        let deployments: Api<Deployment> = Api::namespaced(client, namespace);
        deployments.get(name).await.with_context(|| {
            format!(
                "deployment '{}' not found in namespace '{}'",
                name, namespace
            )
        })?;
        let patch = Patch::Merge(json!({"spec": {"replicas": replicas}}));
        deployments
            .patch(name, &PatchParams::default(), &patch)
            .await
            .with_context(|| {
                format!(
                    "failed to patch deployment '{}' in namespace '{}'",
                    name, namespace
                )
            })?;
        Ok(())
    }

    pub async fn scale_statefulset(
        &self,
        name: &str,
        namespace: &str,
        replicas: i32,
    ) -> Result<()> {
        if !(0..=100).contains(&replicas) {
            return Err(anyhow!(
                "invalid replica count {}: must be between 0 and 100",
                replicas
            ));
        }
        let client = self.get_client();
        let statefulsets: Api<StatefulSet> = Api::namespaced(client, namespace);
        statefulsets.get(name).await.with_context(|| {
            format!(
                "statefulset '{}' not found in namespace '{}'",
                name, namespace
            )
        })?;
        let patch = Patch::Merge(json!({"spec": {"replicas": replicas}}));
        statefulsets
            .patch(name, &PatchParams::default(), &patch)
            .await
            .with_context(|| {
                format!(
                    "failed to patch statefulset '{}' in namespace '{}'",
                    name, namespace
                )
            })?;
        Ok(())
    }

    /// Triggers a rolling restart by patching the pod template annotation
    /// `kubectl.kubernetes.io/restartedAt` with the current UTC timestamp.
    /// Works for Deployments, StatefulSets, and DaemonSets.
    pub async fn rollout_restart(&self, kind: &str, name: &str, namespace: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let patch = Patch::Merge(json!({
            "spec": {
                "template": {
                    "metadata": {
                        "annotations": {
                            "kubectl.kubernetes.io/restartedAt": now
                        }
                    }
                }
            }
        }));
        let pp = PatchParams::default();
        let client = self.get_client();
        match kind.to_lowercase().as_str() {
            "deployment" => {
                let api: Api<Deployment> = Api::namespaced(client, namespace);
                api.patch(name, &pp, &patch).await.with_context(|| {
                    format!("failed to restart deployment '{name}' in '{namespace}'")
                })?;
            }
            "statefulset" => {
                let api: Api<StatefulSet> = Api::namespaced(client, namespace);
                api.patch(name, &pp, &patch).await.with_context(|| {
                    format!("failed to restart statefulset '{name}' in '{namespace}'")
                })?;
            }
            "daemonset" => {
                let api: Api<DaemonSet> = Api::namespaced(client, namespace);
                api.patch(name, &pp, &patch).await.with_context(|| {
                    format!("failed to restart daemonset '{name}' in '{namespace}'")
                })?;
            }
            other => {
                return Err(anyhow::anyhow!(
                    "rollout restart not supported for kind '{other}'"
                ));
            }
        }
        Ok(())
    }
}

/// Executes a scale operation with progress tracking.
pub async fn execute_scale(
    client: Arc<K8sClient>,
    request: ScaleRequest,
    progress_tx: mpsc::Sender<ScaleProgress>,
) -> Result<(), ScaleError> {
    // Validate inputs
    if request.replicas < 0 || request.replicas > 100 {
        return Err(ScaleError::InvalidReplicaCount(request.replicas));
    }

    // Send initiated progress
    let _ = progress_tx.send(ScaleProgress::Initiated).await;

    // Call scale API
    if let Err(e) = client
        .scale_deployment(&request.deployment, &request.namespace, request.replicas)
        .await
    {
        let err_msg = format!("{}", e);
        let _ = progress_tx
            .send(ScaleProgress::Error(err_msg.clone()))
            .await;
        return Err(ScaleError::ApiError(err_msg));
    }

    // Poll for replicas with timeout of 2 minutes (120 seconds)
    match client
        .wait_for_replicas(
            &request.deployment,
            &request.namespace,
            request.replicas,
            120u64,
        )
        .await
    {
        Ok(_) => {
            // Get final state
            if let Ok((current, _)) = client
                .get_deployment_replicas(&request.deployment, &request.namespace)
                .await
            {
                let _ = progress_tx
                    .send(ScaleProgress::Success {
                        current,
                        target: request.replicas,
                    })
                    .await;
            }
            Ok(())
        }
        Err(e) => {
            let err_msg = format!("{}", e);
            let _ = progress_tx
                .send(ScaleProgress::Error(err_msg.clone()))
                .await;
            Err(ScaleError::Timeout(err_msg))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scale_request_creation() {
        let req = ScaleRequest::new("my-deploy", "default", 3);
        assert_eq!(req.deployment, "my-deploy");
        assert_eq!(req.namespace, "default");
        assert_eq!(req.replicas, 3);
    }

    #[test]
    fn test_scale_request_valid_zero() {
        let req = ScaleRequest::new("my-deploy", "default", 0);
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_scale_request_valid_one_hundred() {
        let req = ScaleRequest::new("my-deploy", "default", 100);
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_scale_request_invalid_negative() {
        let req = ScaleRequest::new("my-deploy", "default", -1);
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_scale_request_invalid_over_one_hundred() {
        let req = ScaleRequest::new("my-deploy", "default", 101);
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_scale_error_display() {
        let err = ScaleError::DeploymentNotFound("nginx".to_string(), "default".to_string());
        assert!(format!("{}", err).contains("nginx"));
        assert!(format!("{}", err).contains("default"));
    }

    #[test]
    fn test_scale_progress_initiated() {
        let progress = ScaleProgress::Initiated;
        let serialized = serde_json::to_string(&progress).expect("should serialize");
        assert!(serialized.contains("Initiated"));
    }

    #[test]
    fn test_scale_error_display_variants() {
        assert!(format!("{}", ScaleError::ApiError("oops".to_string())).contains("API error"));
        assert!(format!("{}", ScaleError::Timeout("slow".to_string())).contains("Timeout"));
        assert_eq!(
            format!("{}", ScaleError::Cancelled),
            "Scale operation cancelled"
        );
    }

    #[tokio::test]
    async fn execute_scale_rejects_invalid_replica_count_and_sends_no_progress() {
        let client = Arc::new(
            K8sClient::connect()
                .await
                .expect("kind cluster should be available"),
        );
        let request = ScaleRequest::new("any", "default", 101);
        let (tx, mut rx) = mpsc::channel(4);

        let err = execute_scale(client, request, tx)
            .await
            .expect_err("invalid replicas should fail early");

        match err {
            ScaleError::InvalidReplicaCount(101) => {}
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "no progress events expected");
    }

    #[tokio::test]
    async fn execute_scale_reports_api_error_when_deployment_missing() {
        let client = Arc::new(
            K8sClient::connect()
                .await
                .expect("kind cluster should be available"),
        );
        let request = ScaleRequest::new("missing-deployment-xyz", "default", 1);
        let (tx, mut rx) = mpsc::channel(8);

        let err = execute_scale(client, request, tx)
            .await
            .expect_err("missing deployment should fail");

        match err {
            ScaleError::ApiError(msg) => {
                assert!(msg.contains("not found") || msg.contains("failed"));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let first = rx.recv().await.expect("initiated event expected");
        assert!(matches!(first, ScaleProgress::Initiated));

        let second = rx.recv().await.expect("error event expected");
        assert!(matches!(second, ScaleProgress::Error(_)));
    }
}
