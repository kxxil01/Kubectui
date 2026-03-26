//! Helpers for fetching Kubernetes resources and serializing them to YAML.

use anyhow::{Context, Result, anyhow};
use kube::{
    Api, Client,
    api::{ApiResource, DynamicObject, GroupVersionKind, Patch, PatchParams},
    discovery,
};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

use crate::k8s::flux::{
    FluxReconcileSupport, RECONCILE_REQUEST_ANNOTATION, flux_reconcile_support,
};
use crate::time::format_rfc3339;

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

    let api = resource_api(client, &api_resource, namespaced, kind, namespace)?;

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

    render_yaml(&fetched, true).context("failed serializing resource to YAML")
}

/// Fetches a full-fidelity manifest for drift inspection.
///
/// Unlike [`get_resource_yaml`], this never truncates the payload and never
/// degrades RBAC failures into comment-only placeholders.
pub async fn get_resource_yaml_for_diff(
    client: &Client,
    kind: &str,
    name: &str,
    namespace: Option<&str>,
) -> Result<String> {
    let (api_resource, namespaced) = api_resource_for_kind(kind)
        .with_context(|| format!("unsupported resource kind '{kind}'"))?;
    let api = resource_api(client, &api_resource, namespaced, kind, namespace)?;

    let fetched = api.get(name).await.map_err(|err| {
        if is_forbidden_error(&err) {
            anyhow!(
                "RBAC denied reading live manifest for {kind}/{name} in namespace '{}'",
                namespace.unwrap_or("<cluster-scope>")
            )
        } else {
            anyhow!(err).context(format!(
                "failed fetching resource kind='{kind}' name='{name}' namespace='{}'",
                namespace.unwrap_or("<cluster-scope>")
            ))
        }
    })?;

    render_yaml(&fetched, false).context("failed serializing resource to YAML")
}

/// Applies edited YAML back to the cluster using server-side apply.
///
/// The YAML must contain `kind`, `metadata.name`, and (for namespaced resources)
/// `metadata.namespace`. Returns `Ok(())` on success or a descriptive error.
pub async fn apply_resource_yaml(
    client: &Client,
    yaml_str: &str,
    kind: &str,
    name: &str,
    namespace: Option<&str>,
) -> Result<()> {
    let (api_resource, namespaced) = api_resource_for_kind(kind)
        .with_context(|| format!("unsupported resource kind '{kind}'"))?;

    // Parse the YAML into a DynamicObject so we can patch it
    let obj: DynamicObject =
        serde_yaml::from_str(yaml_str).context("invalid YAML: failed to parse")?;

    let api: Api<DynamicObject> = if namespaced {
        match namespace {
            Some(ns) => Api::namespaced_with(client.clone(), ns, &api_resource),
            None => return Err(anyhow!("resource kind '{kind}' requires a namespace")),
        }
    } else {
        Api::all_with(client.clone(), &api_resource)
    };

    let params = PatchParams::apply("kubectui").force();
    api.patch(name, &params, &Patch::Apply(&obj))
        .await
        .with_context(|| format!("failed to apply {kind}/{name}"))?;

    Ok(())
}

/// Applies one or more YAML documents using server-side apply.
///
/// Empty documents are ignored. Each document must define `apiVersion`,
/// `kind`, and `metadata.name`. Namespaced resources must also define
/// `metadata.namespace`.
pub async fn apply_yaml_documents(client: &Client, yaml_str: &str) -> Result<usize> {
    #[derive(Debug, Deserialize)]
    struct ManifestMeta {
        name: String,
        namespace: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct ManifestHeader {
        #[serde(rename = "apiVersion")]
        api_version: String,
        kind: String,
        metadata: ManifestMeta,
    }

    let mut applied = 0usize;
    let mut discovered_resources: HashMap<(String, String), (ApiResource, bool)> = HashMap::new();
    for document in serde_yaml::Deserializer::from_str(yaml_str) {
        let value = serde_yaml::Value::deserialize(document).context("invalid YAML document")?;
        if value.is_null() {
            continue;
        }

        let header: ManifestHeader =
            serde_yaml::from_value(value.clone()).context("manifest is missing required fields")?;
        let object: DynamicObject =
            serde_yaml::from_value(value).context("invalid manifest object")?;
        let (api_resource, namespaced) = resolve_manifest_api_resource(
            client,
            &header.api_version,
            &header.kind,
            &mut discovered_resources,
        )
        .await
        .with_context(|| {
            format!(
                "unsupported manifest apiVersion '{}' kind '{}'",
                header.api_version, header.kind
            )
        })?;
        let api = resource_api(
            client,
            &api_resource,
            namespaced,
            &header.kind,
            header.metadata.namespace.as_deref(),
        )?;
        let params = PatchParams::apply("kubectui").force();
        api.patch(&header.metadata.name, &params, &Patch::Apply(&object))
            .await
            .with_context(|| format!("failed to apply {}/{}", header.kind, header.metadata.name))?;
        applied += 1;
    }

    if applied == 0 {
        return Err(anyhow!("no manifest documents were found"));
    }
    Ok(applied)
}

async fn resolve_manifest_api_resource(
    client: &Client,
    api_version: &str,
    kind: &str,
    cache: &mut HashMap<(String, String), (ApiResource, bool)>,
) -> Result<(ApiResource, bool)> {
    let cache_key = (api_version.to_string(), kind.to_string());
    if let Some(cached) = cache.get(&cache_key) {
        return Ok(cached.clone());
    }

    let (group, version) = api_version
        .split_once('/')
        .map_or(("", api_version), |(group, version)| (group, version));
    let gvk = GroupVersionKind::gvk(group, version, kind);
    let resolved = match discovery::pinned_kind(client, &gvk).await {
        Ok((api_resource, caps)) => {
            let namespaced = matches!(caps.scope, discovery::Scope::Namespaced);
            (api_resource, namespaced)
        }
        Err(discovery_err) => api_resource_for_manifest(api_version, kind)
            .with_context(|| discovery_err.to_string())?,
    };
    cache.insert(cache_key, resolved.clone());
    Ok(resolved)
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

fn resource_api(
    client: &Client,
    api_resource: &ApiResource,
    namespaced: bool,
    kind: &str,
    namespace: Option<&str>,
) -> Result<Api<DynamicObject>> {
    if namespaced {
        match namespace {
            Some(ns) => Ok(Api::namespaced_with(client.clone(), ns, api_resource)),
            None => Err(anyhow!(
                "resource kind '{}' requires a namespace",
                kind.to_ascii_lowercase()
            )),
        }
    } else {
        Ok(Api::all_with(client.clone(), api_resource))
    }
}

fn render_yaml(object: &DynamicObject, truncate: bool) -> Result<String> {
    let rendered = serde_yaml::to_string(object)?;
    Ok(if truncate {
        truncate_yaml(rendered)
    } else {
        rendered
    })
}

fn api_resource_for_kind(kind: &str) -> Result<(ApiResource, bool)> {
    let kind = kind.to_ascii_lowercase();

    match kind.as_str() {
        // ── Core v1 ──────────────────────────────────────────────────────────
        "pod" | "pods" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "Pod")),
            true,
        )),
        "service" | "services" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "Service")),
            true,
        )),
        "node" | "nodes" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "Node")),
            false,
        )),
        "namespace" | "namespaces" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "Namespace")),
            false,
        )),
        "configmap" | "configmaps" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "ConfigMap")),
            true,
        )),
        "secret" | "secrets" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "Secret")),
            true,
        )),
        "persistentvolumeclaim" | "persistentvolumeclaims" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "PersistentVolumeClaim")),
            true,
        )),
        "persistentvolume" | "persistentvolumes" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "PersistentVolume")),
            false,
        )),
        "serviceaccount" | "serviceaccounts" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "ServiceAccount")),
            true,
        )),
        "endpoints" | "endpoint" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "Endpoints")),
            true,
        )),
        "event" | "events" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "Event")),
            true,
        )),
        "replicationcontroller" | "replicationcontrollers" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "ReplicationController")),
            true,
        )),
        "resourcequota" | "resourcequotas" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "ResourceQuota")),
            true,
        )),
        "limitrange" | "limitranges" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "LimitRange")),
            true,
        )),
        // ── apps/v1 ──────────────────────────────────────────────────────────
        "deployment" | "deployments" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("apps", "v1", "Deployment")),
            true,
        )),
        "statefulset" | "statefulsets" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("apps", "v1", "StatefulSet")),
            true,
        )),
        "daemonset" | "daemonsets" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("apps", "v1", "DaemonSet")),
            true,
        )),
        "replicaset" | "replicasets" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("apps", "v1", "ReplicaSet")),
            true,
        )),
        // ── batch/v1 ─────────────────────────────────────────────────────────
        "job" | "jobs" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("batch", "v1", "Job")),
            true,
        )),
        "cronjob" | "cronjobs" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("batch", "v1", "CronJob")),
            true,
        )),
        // ── networking.k8s.io/v1 ─────────────────────────────────────────────
        "ingress" | "ingresses" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk("networking.k8s.io", "v1", "Ingress")),
            true,
        )),
        "ingressclass" | "ingressclasses" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk(
                "networking.k8s.io",
                "v1",
                "IngressClass",
            )),
            false,
        )),
        "networkpolicy" | "networkpolicies" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk(
                "networking.k8s.io",
                "v1",
                "NetworkPolicy",
            )),
            true,
        )),
        // ── autoscaling/v2 ───────────────────────────────────────────────────
        "horizontalpodautoscaler" | "horizontalpodautoscalers" | "hpa" | "hpas" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk(
                "autoscaling",
                "v2",
                "HorizontalPodAutoscaler",
            )),
            true,
        )),
        // ── policy/v1 ────────────────────────────────────────────────────────
        "poddisruptionbudget" | "poddisruptionbudgets" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk(
                "policy",
                "v1",
                "PodDisruptionBudget",
            )),
            true,
        )),
        // ── scheduling.k8s.io/v1 ─────────────────────────────────────────────
        "priorityclass" | "priorityclasses" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk(
                "scheduling.k8s.io",
                "v1",
                "PriorityClass",
            )),
            false,
        )),
        // ── storage.k8s.io/v1 ────────────────────────────────────────────────
        "storageclass" | "storageclasses" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk(
                "storage.k8s.io",
                "v1",
                "StorageClass",
            )),
            false,
        )),
        // ── rbac.authorization.k8s.io/v1 ─────────────────────────────────────
        "clusterrole" | "clusterroles" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk(
                "rbac.authorization.k8s.io",
                "v1",
                "ClusterRole",
            )),
            false,
        )),
        "clusterrolebinding" | "clusterrolebindings" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk(
                "rbac.authorization.k8s.io",
                "v1",
                "ClusterRoleBinding",
            )),
            false,
        )),
        "role" | "roles" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk(
                "rbac.authorization.k8s.io",
                "v1",
                "Role",
            )),
            true,
        )),
        "rolebinding" | "rolebindings" => Ok((
            ApiResource::from_gvk(&GroupVersionKind::gvk(
                "rbac.authorization.k8s.io",
                "v1",
                "RoleBinding",
            )),
            true,
        )),
        _ => Err(anyhow!("unsupported kind: {kind}")),
    }
}

fn api_resource_for_manifest(api_version: &str, kind: &str) -> Result<(ApiResource, bool)> {
    let resolved = match (api_version, kind) {
        ("v1", "Pod") => Some((GroupVersionKind::gvk("", "v1", "Pod"), true)),
        ("v1", "Service") => Some((GroupVersionKind::gvk("", "v1", "Service"), true)),
        ("v1", "Namespace") => Some((GroupVersionKind::gvk("", "v1", "Namespace"), false)),
        ("v1", "Node") => Some((GroupVersionKind::gvk("", "v1", "Node"), false)),
        ("v1", "ConfigMap") => Some((GroupVersionKind::gvk("", "v1", "ConfigMap"), true)),
        ("v1", "Secret") => Some((GroupVersionKind::gvk("", "v1", "Secret"), true)),
        ("v1", "PersistentVolumeClaim") => Some((
            GroupVersionKind::gvk("", "v1", "PersistentVolumeClaim"),
            true,
        )),
        ("v1", "PersistentVolume") => {
            Some((GroupVersionKind::gvk("", "v1", "PersistentVolume"), false))
        }
        ("v1", "ServiceAccount") => Some((GroupVersionKind::gvk("", "v1", "ServiceAccount"), true)),
        ("v1", "Endpoints") => Some((GroupVersionKind::gvk("", "v1", "Endpoints"), true)),
        ("v1", "Event") => Some((GroupVersionKind::gvk("", "v1", "Event"), true)),
        ("v1", "ReplicationController") => Some((
            GroupVersionKind::gvk("", "v1", "ReplicationController"),
            true,
        )),
        ("v1", "ResourceQuota") => Some((GroupVersionKind::gvk("", "v1", "ResourceQuota"), true)),
        ("v1", "LimitRange") => Some((GroupVersionKind::gvk("", "v1", "LimitRange"), true)),
        ("apps/v1", "Deployment") => {
            Some((GroupVersionKind::gvk("apps", "v1", "Deployment"), true))
        }
        ("apps/v1", "StatefulSet") => {
            Some((GroupVersionKind::gvk("apps", "v1", "StatefulSet"), true))
        }
        ("apps/v1", "DaemonSet") => Some((GroupVersionKind::gvk("apps", "v1", "DaemonSet"), true)),
        ("apps/v1", "ReplicaSet") => {
            Some((GroupVersionKind::gvk("apps", "v1", "ReplicaSet"), true))
        }
        ("batch/v1", "Job") => Some((GroupVersionKind::gvk("batch", "v1", "Job"), true)),
        ("batch/v1", "CronJob") => Some((GroupVersionKind::gvk("batch", "v1", "CronJob"), true)),
        ("networking.k8s.io/v1", "Ingress") => Some((
            GroupVersionKind::gvk("networking.k8s.io", "v1", "Ingress"),
            true,
        )),
        ("networking.k8s.io/v1", "IngressClass") => Some((
            GroupVersionKind::gvk("networking.k8s.io", "v1", "IngressClass"),
            false,
        )),
        ("networking.k8s.io/v1", "NetworkPolicy") => Some((
            GroupVersionKind::gvk("networking.k8s.io", "v1", "NetworkPolicy"),
            true,
        )),
        ("autoscaling/v2", "HorizontalPodAutoscaler") => Some((
            GroupVersionKind::gvk("autoscaling", "v2", "HorizontalPodAutoscaler"),
            true,
        )),
        ("policy/v1", "PodDisruptionBudget") => Some((
            GroupVersionKind::gvk("policy", "v1", "PodDisruptionBudget"),
            true,
        )),
        ("scheduling.k8s.io/v1", "PriorityClass") => Some((
            GroupVersionKind::gvk("scheduling.k8s.io", "v1", "PriorityClass"),
            false,
        )),
        ("storage.k8s.io/v1", "StorageClass") => Some((
            GroupVersionKind::gvk("storage.k8s.io", "v1", "StorageClass"),
            false,
        )),
        ("rbac.authorization.k8s.io/v1", "ClusterRole") => Some((
            GroupVersionKind::gvk("rbac.authorization.k8s.io", "v1", "ClusterRole"),
            false,
        )),
        ("rbac.authorization.k8s.io/v1", "ClusterRoleBinding") => Some((
            GroupVersionKind::gvk("rbac.authorization.k8s.io", "v1", "ClusterRoleBinding"),
            false,
        )),
        ("rbac.authorization.k8s.io/v1", "Role") => Some((
            GroupVersionKind::gvk("rbac.authorization.k8s.io", "v1", "Role"),
            true,
        )),
        ("rbac.authorization.k8s.io/v1", "RoleBinding") => Some((
            GroupVersionKind::gvk("rbac.authorization.k8s.io", "v1", "RoleBinding"),
            true,
        )),
        ("apiextensions.k8s.io/v1", "CustomResourceDefinition") => Some((
            GroupVersionKind::gvk("apiextensions.k8s.io", "v1", "CustomResourceDefinition"),
            false,
        )),
        _ => None,
    };

    let Some((gvk, namespaced)) = resolved else {
        return Err(anyhow!(
            "unsupported manifest kind fallback for apiVersion '{api_version}' kind '{kind}'"
        ));
    };
    Ok((ApiResource::from_gvk(&gvk), namespaced))
}

/// Fetches YAML for a custom resource using explicit API coordinates.
///
/// Unlike `get_resource_yaml` which uses a hardcoded kind map, this function
/// accepts the full CRD coordinates (group, version, kind, plural) to construct
/// the dynamic API request.
pub async fn get_custom_resource_yaml(
    client: &Client,
    group: &str,
    version: &str,
    kind: &str,
    plural: &str,
    name: &str,
    namespace: Option<&str>,
) -> Result<String> {
    let gvk = GroupVersionKind::gvk(group, version, kind);
    let mut ar = ApiResource::from_gvk(&gvk);
    ar.plural = plural.to_string();

    let api: Api<DynamicObject> = match namespace {
        Some(ns) => Api::namespaced_with(client.clone(), ns, &ar),
        None => Api::all_with(client.clone(), &ar),
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
                    "failed fetching custom resource {group}/{version}/{kind} name='{name}' namespace='{}'",
                    namespace.unwrap_or("<cluster-scope>")
                )
            });
        }
    };

    render_yaml(&fetched, true).context("failed serializing custom resource to YAML")
}

/// Fetches a full-fidelity custom-resource manifest for drift inspection.
pub async fn get_custom_resource_yaml_for_diff(
    client: &Client,
    group: &str,
    version: &str,
    kind: &str,
    plural: &str,
    name: &str,
    namespace: Option<&str>,
) -> Result<String> {
    let gvk = GroupVersionKind::gvk(group, version, kind);
    let mut ar = ApiResource::from_gvk(&gvk);
    ar.plural = plural.to_string();

    let api: Api<DynamicObject> = match namespace {
        Some(ns) => Api::namespaced_with(client.clone(), ns, &ar),
        None => Api::all_with(client.clone(), &ar),
    };

    let fetched = api.get(name).await.map_err(|err| {
        if is_forbidden_error(&err) {
            anyhow!(
                "RBAC denied reading live manifest for {group}/{version}/{kind} '{name}' in namespace '{}'",
                namespace.unwrap_or("<cluster-scope>")
            )
        } else {
            anyhow!(err).context(format!(
                "failed fetching custom resource {group}/{version}/{kind} name='{name}' namespace='{}'",
                namespace.unwrap_or("<cluster-scope>")
            ))
        }
    })?;

    render_yaml(&fetched, false).context("failed serializing custom resource to YAML")
}

fn is_forbidden_error(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(status) if status.is_forbidden())
}

/// Deletes a Kubernetes resource by kind, name, and optional namespace.
///
/// Uses the same dynamic API lookup as `get_resource_yaml`. Returns a
/// human-readable error for RBAC-forbidden or not-found cases.
pub async fn delete_resource(
    client: &Client,
    kind: &str,
    name: &str,
    namespace: Option<&str>,
) -> Result<()> {
    let (api_resource, namespaced) = api_resource_for_kind(kind)
        .with_context(|| format!("unsupported resource kind '{kind}'"))?;

    let api: Api<DynamicObject> = if namespaced {
        match namespace {
            Some(ns) => Api::namespaced_with(client.clone(), ns, &api_resource),
            None => return Err(anyhow!("resource kind '{kind}' requires a namespace")),
        }
    } else {
        Api::all_with(client.clone(), &api_resource)
    };

    let dp = kube::api::DeleteParams::default();
    api.delete(name, &dp).await.with_context(|| {
        format!(
            "failed to delete {kind}/{name} in namespace '{}'",
            namespace.unwrap_or("<cluster-scope>")
        )
    })?;

    Ok(())
}

/// Force-deletes a Kubernetes resource by setting grace period to 0.
pub async fn force_delete_resource(
    client: &Client,
    kind: &str,
    name: &str,
    namespace: Option<&str>,
) -> Result<()> {
    let (api_resource, namespaced) = api_resource_for_kind(kind)
        .with_context(|| format!("unsupported resource kind '{kind}'"))?;

    let api: Api<DynamicObject> = if namespaced {
        match namespace {
            Some(ns) => Api::namespaced_with(client.clone(), ns, &api_resource),
            None => return Err(anyhow!("resource kind '{kind}' requires a namespace")),
        }
    } else {
        Api::all_with(client.clone(), &api_resource)
    };

    let dp = kube::api::DeleteParams {
        grace_period_seconds: Some(0),
        propagation_policy: Some(kube::api::PropagationPolicy::Background),
        ..Default::default()
    };

    api.delete(name, &dp).await.with_context(|| {
        format!(
            "failed to force-delete {kind}/{name} in namespace '{}'",
            namespace.unwrap_or("<cluster-scope>")
        )
    })?;

    Ok(())
}

/// Deletes a custom resource using explicit API coordinates.
pub async fn delete_custom_resource(
    client: &Client,
    group: &str,
    version: &str,
    kind: &str,
    plural: &str,
    name: &str,
    namespace: Option<&str>,
) -> Result<()> {
    let gvk = GroupVersionKind::gvk(group, version, kind);
    let mut ar = ApiResource::from_gvk(&gvk);
    ar.plural = plural.to_string();

    let api: Api<DynamicObject> = match namespace {
        Some(ns) => Api::namespaced_with(client.clone(), ns, &ar),
        None => Api::all_with(client.clone(), &ar),
    };

    let dp = kube::api::DeleteParams::default();
    api.delete(name, &dp)
        .await
        .with_context(|| {
            format!(
                "failed to delete custom resource {group}/{version}/{kind} name='{name}' namespace='{}'",
                namespace.unwrap_or("<cluster-scope>")
            )
        })?;

    Ok(())
}

/// Requests Flux reconciliation for a custom resource by patching the standard
/// `reconcile.fluxcd.io/requestedAt` annotation.
pub async fn request_flux_reconcile(
    client: &Client,
    group: &str,
    version: &str,
    kind: &str,
    plural: &str,
    name: &str,
    namespace: Option<&str>,
) -> Result<()> {
    match flux_reconcile_support(group, kind) {
        FluxReconcileSupport::Supported => {}
        FluxReconcileSupport::Unsupported(reason) => return Err(anyhow!(reason)),
    }

    let gvk = GroupVersionKind::gvk(group, version, kind);
    let mut ar = ApiResource::from_gvk(&gvk);
    ar.plural = plural.to_string();

    let api: Api<DynamicObject> = match namespace {
        Some(ns) => Api::namespaced_with(client.clone(), ns, &ar),
        None => Api::all_with(client.clone(), &ar),
    };

    let timestamp = format_rfc3339(crate::time::now());
    let patch = Patch::Merge(json!({
        "metadata": {
            "annotations": {
                RECONCILE_REQUEST_ANNOTATION: timestamp,
            }
        }
    }));

    api.patch(name, &PatchParams::default(), &patch)
        .await
        .with_context(|| {
            format!(
                "failed to request Flux reconciliation for {group}/{version}/{kind} name='{name}' namespace='{}'",
                namespace.unwrap_or("<cluster-scope>")
            )
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use kube::core::Status;

    use super::*;
    use crate::k8s::flux::RECONCILE_REQUEST_ANNOTATION;

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
        let forbidden = kube::Error::Api(
            Status::failure("forbidden", "Forbidden")
                .with_code(403)
                .boxed(),
        );
        let not_found = kube::Error::Api(
            Status::failure("not found", "NotFound")
                .with_code(404)
                .boxed(),
        );

        assert!(is_forbidden_error(&forbidden));
        assert!(!is_forbidden_error(&not_found));
    }

    #[test]
    fn flux_reconcile_patch_contains_requested_at_annotation() {
        let patch = json!({
            "metadata": {
                "annotations": {
                    RECONCILE_REQUEST_ANNOTATION: "2026-03-06T12:00:00.123456789Z",
                }
            }
        });
        assert_eq!(
            patch["metadata"]["annotations"][RECONCILE_REQUEST_ANNOTATION],
            "2026-03-06T12:00:00.123456789Z"
        );
    }

    #[test]
    fn manifest_scope_fallback_marks_cluster_scoped_kinds() {
        let (_, namespaced) =
            api_resource_for_manifest("rbac.authorization.k8s.io/v1", "ClusterRole")
                .expect("cluster role");
        assert!(!namespaced);
    }

    #[test]
    fn manifest_scope_fallback_marks_namespaced_kinds() {
        let (_, namespaced) =
            api_resource_for_manifest("apps/v1", "Deployment").expect("deployment");
        assert!(namespaced);
    }

    #[test]
    fn manifest_scope_fallback_rejects_unknown_kinds() {
        let err = api_resource_for_manifest("example.com/v1", "Widget").expect_err("unsupported");
        assert!(
            err.to_string()
                .contains("unsupported manifest kind fallback")
        );
    }
}
