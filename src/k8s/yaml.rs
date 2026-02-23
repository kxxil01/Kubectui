//! Helpers for fetching Kubernetes resources and serializing them to YAML.

use anyhow::{Context, Result, anyhow};
use kube::{
    Api, Client,
    api::{ApiResource, DynamicObject, GroupVersionKind},
    error::ErrorResponse,
};

/// Maximum rendered YAML length in bytes (10 KiB).
pub const MAX_YAML_BYTES: usize = 10 * 1024;

/// Fetches an arbitrary Kubernetes resource and serializes it to YAML.
///
/// For unsupported kinds, this function returns an error.
/// For RBAC-forbidden access, this function returns a graceful message string
/// instead of bubbling up a hard error.
pub async fn get_resource_yaml(
    client: &Client,
    kind: &str,
    name: &str,
    namespace: Option<&str>,
) -> Result<String> {
    let (api_resource, namespaced) = api_resource_for_kind(kind)
        .with_context(|| format!("unsupported resource kind '{kind}'"))?;

    let api: Api<DynamicObject> = if namespaced {
        match namespace {
            Some(ns) => Api::namespaced_with(client.clone(), ns, &api_resource),
            None => {
                return Err(anyhow!(
                    "resource kind '{}' requires a namespace",
                    kind.to_ascii_lowercase()
                ));
            }
        }
    } else {
        Api::all_with(client.clone(), &api_resource)
    };

    let fetched = match api.get(name).await {
        Ok(obj) => obj,
        Err(err) if is_forbidden_error(&err) => {
            return Ok(format!(
                "# YAML unavailable (RBAC)\n# kind: {kind}\n# name: {name}\n# namespace: {}",
                namespace.unwrap_or("<cluster-scope>")
            ));
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "failed fetching resource kind='{kind}' name='{name}' namespace='{}'",
                    namespace.unwrap_or("<cluster-scope>")
                )
            });
        }
    };

    let rendered =
        serde_yaml::to_string(&fetched).context("failed serializing resource to YAML")?;
    Ok(truncate_yaml(rendered))
}

/// Truncates YAML payload when it exceeds [`MAX_YAML_BYTES`].
pub fn truncate_yaml(yaml: String) -> String {
    if yaml.len() <= MAX_YAML_BYTES {
        return yaml;
    }

    let mut cut = 0usize;
    for (idx, _) in yaml.char_indices() {
        if idx > MAX_YAML_BYTES {
            break;
        }
        cut = idx;
    }

    let truncated = &yaml[..cut];
    format!("{truncated}\n... (truncated)")
}

fn api_resource_for_kind(kind: &str) -> Result<(ApiResource, bool)> {
    let kind = kind.to_ascii_lowercase();

    match kind.as_str() {
        "pod" | "pods" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "Pod")),
            true,
        )),
        "service" | "services" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "Service")),
            true,
        )),
        "deployment" | "deployments" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("apps", "v1", "Deployment")),
            true,
        )),
        "node" | "nodes" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "Node")),
            false,
        )),
        "resourcequota" | "resourcequotas" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "ResourceQuota")),
            true,
        )),
        "limitrange" | "limitranges" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "LimitRange")),
            true,
        )),
        "poddisruptionbudget" | "poddisruptionbudgets" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk(
                "policy",
                "v1",
                "PodDisruptionBudget",
            )),
            true,
        )),
        _ => Err(anyhow!("unsupported kind: {kind}")),
    }
}

fn is_forbidden_error(err: &kube::Error) -> bool {
    match err {
        kube::Error::Api(ErrorResponse { code, .. }) => *code == 403,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use kube::error::ErrorResponse;

    use super::*;

    /// Verifies YAML truncation keeps short payloads intact.
    #[test]
    fn truncate_yaml_keeps_small_payload() {
        let yaml = "kind: Pod\nmetadata:\n  name: p1\n".to_string();
        assert_eq!(truncate_yaml(yaml.clone()), yaml);
    }

    /// Verifies YAML payloads larger than 10KiB are truncated with suffix.
    #[test]
    fn truncate_yaml_applies_10kb_limit() {
        let long_yaml = format!("kind: Pod\n{}", "a".repeat(MAX_YAML_BYTES + 1024));
        let out = truncate_yaml(long_yaml);
        assert!(out.contains("... (truncated)"));
        assert!(out.len() <= MAX_YAML_BYTES + 32);
    }

    /// Verifies truncation handles multibyte UTF-8 boundaries safely.
    #[test]
    fn truncate_yaml_handles_utf8_boundaries() {
        let payload = "🚀".repeat(MAX_YAML_BYTES);
        let out = truncate_yaml(payload);
        assert!(out.is_char_boundary(out.len()));
    }

    /// Verifies large resources over 1MB are truncated and safe to display.
    #[test]
    fn truncate_yaml_handles_large_resource() {
        let huge = "x".repeat(1_100_000);
        let out = truncate_yaml(huge);
        assert!(out.ends_with("... (truncated)"));
    }

    /// Verifies forbidden error classifier returns true only for 403 API errors.
    #[test]
    fn forbidden_error_detection() {
        let forbidden = kube::Error::Api(ErrorResponse {
            status: "Failure".to_string(),
            message: "forbidden".to_string(),
            reason: "Forbidden".to_string(),
            code: 403,
        });
        let not_found = kube::Error::Api(ErrorResponse {
            status: "Failure".to_string(),
            message: "not found".to_string(),
            reason: "NotFound".to_string(),
            code: 404,
        });

        assert!(is_forbidden_error(&forbidden));
        assert!(!is_forbidden_error(&not_found));
    }
}
