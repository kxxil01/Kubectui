//! Kubernetes deployment scaling operations.

use anyhow::{anyhow, Context, Result};
use k8s_openapi::api::apps::v1::Deployment;
use kube::{Api, api::Patch, api::PatchParams};
use serde_json::json;
use serde::{Deserialize, Serialize};

use crate::k8s::client::K8sClient;

/// Request to scale a deployment to a specific replica count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleRequest {
    /// Deployment name.
    pub deployment: String,
    /// Target namespace.
    pub namespace: String,
    /// Desired replica count (0-100).
    pub replicas: i32,
}

impl ScaleRequest {
    /// Creates a new scale request.
    pub fn new(deployment: impl Into<String>, namespace: impl Into<String>, replicas: i32) -> Self {
        Self {
            deployment: deployment.into(),
            namespace: namespace.into(),
            replicas,
        }
    }

    /// Validates that the replica count is within acceptable bounds.
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
    /// Scales a deployment to the desired replica count.
    pub async fn scale_deployment(
        &self,
        name: &str,
        namespace: &str,
        replicas: i32,
    ) -> Result<()> {
        // Validate replica count
        if replicas < 0 || replicas > 100 {
            return Err(anyhow!(
                "invalid replica count {}: must be between 0 and 100",
                replicas
            ));
        }

        // Create a typed API client for deployments in the target namespace
        let deployments: Api<Deployment> = Api::namespaced(self.client.clone(), namespace);

        // Verify deployment exists
        deployments
            .get(name)
            .await
            .with_context(|| format!("deployment '{}' not found in namespace '{}'", name, namespace))?;

        // Patch the spec.replicas field
        let patch = Patch::Merge(json!({
            "spec": {
                "replicas": replicas
            }
        }));

        deployments
            .patch(name, &PatchParams::apply("kubectui"), &patch)
            .await
            .with_context(|| format!("failed to patch deployment '{}' in namespace '{}'", name, namespace))?;

        Ok(())
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
}
