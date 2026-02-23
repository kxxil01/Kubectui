//! Kubernetes deployment scaling operations.
use anyhow::{anyhow, Context, Result};
use k8s_openapi::api::apps::v1::Deployment;
use kube::{Api, api::Patch, api::PatchParams};
use serde_json::json;
use serde::{Deserialize, Serialize};
use crate::k8s::client::K8sClient;

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
            return Err(anyhow!("invalid replica count {}: must be between 0 and 100", self.replicas));
        }
        Ok(())
    }
}

impl K8sClient {
    pub async fn scale_deployment(&self, name: &str, namespace: &str, replicas: i32) -> Result<()> {
        if replicas < 0 || replicas > 100 {
            return Err(anyhow!("invalid replica count {}: must be between 0 and 100", replicas));
        }
        let deployments: Api<Deployment> = Api::namespaced(self.client.clone(), namespace);
        deployments.get(name).await.with_context(|| format!("deployment '{}' not found in namespace '{}'", name, namespace))?;
        let patch = Patch::Merge(json!({"spec": {"replicas": replicas}}));
        deployments.patch(name, &PatchParams::apply("kubectui"), &patch).await.with_context(|| format!("failed to patch deployment '{}' in namespace '{}'", name, namespace))?;
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
