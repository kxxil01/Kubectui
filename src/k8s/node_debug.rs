//! Node debug pod creation and lifecycle helpers.

use anyhow::{Context, Result, anyhow};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    Api,
    api::{DeleteParams, PostParams},
};

use crate::k8s::{client::K8sClient, exec::DebugImagePreset};

const DEBUG_POD_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);
const DEBUG_POD_READY_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
const DEBUG_CONTAINER_NAME: &str = "debugger";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum NodeDebugProfile {
    #[default]
    General,
    Sysadmin,
}

impl NodeDebugProfile {
    pub const ALL: [Self; 2] = [Self::General, Self::Sysadmin];

    pub const fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Sysadmin => "Sysadmin",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::General => "host namespaces + /host mount",
            Self::Sysadmin => "privileged host debug shell",
        }
    }

    pub const fn is_privileged(self) -> bool {
        matches!(self, Self::Sysadmin)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeDebugLaunchRequest {
    pub node_name: String,
    pub namespace: String,
    pub image: String,
    pub profile: NodeDebugProfile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeDebugLaunchResult {
    pub node_name: String,
    pub namespace: String,
    pub pod_name: String,
    pub image: String,
    pub profile: NodeDebugProfile,
    pub container_name: String,
}

pub async fn launch_node_debug_pod(
    client: &K8sClient,
    request: &NodeDebugLaunchRequest,
) -> Result<NodeDebugLaunchResult> {
    let pods: Api<Pod> = Api::namespaced(client.get_client(), &request.namespace);
    let pod = build_debug_pod(request)?;
    let created = pods
        .create(&PostParams::default(), &pod)
        .await
        .with_context(|| {
            format!(
                "failed to create node debug pod for Node '{}' in namespace '{}'",
                request.node_name, request.namespace
            )
        })?;
    let pod_name = created
        .metadata
        .name
        .clone()
        .context("node debug pod missing name")?;
    if let Err(wait_err) = wait_for_node_debug_pod_ready(&pods, &pod_name).await {
        let cleanup_err = delete_node_debug_pod(client, &request.namespace, &pod_name)
            .await
            .err();
        return match cleanup_err {
            Some(cleanup_err) => Err(wait_err).context(format!(
                "node debug pod '{pod_name}' failed before shell attach, and cleanup in namespace '{}' also failed: {cleanup_err:#}",
                request.namespace
            )),
            None => Err(wait_err).context(format!(
                "node debug pod '{pod_name}' failed before shell attach and was cleaned up"
            )),
        };
    }

    Ok(NodeDebugLaunchResult {
        node_name: request.node_name.clone(),
        namespace: request.namespace.clone(),
        pod_name,
        image: request.image.clone(),
        profile: request.profile,
        container_name: DEBUG_CONTAINER_NAME.to_string(),
    })
}

pub async fn delete_node_debug_pod(
    client: &K8sClient,
    namespace: &str,
    pod_name: &str,
) -> Result<()> {
    let pods: Api<Pod> = Api::namespaced(client.get_client(), namespace);
    let params = DeleteParams {
        grace_period_seconds: Some(0),
        ..DeleteParams::default()
    };
    match pods.delete(pod_name, &params).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(response)) if response.code == 404 => Ok(()),
        Err(err) => Err(err).with_context(|| {
            format!(
                "failed to delete node debug pod '{}' in namespace '{}'",
                pod_name, namespace
            )
        }),
    }
}

fn build_debug_pod(request: &NodeDebugLaunchRequest) -> Result<Pod> {
    let prefix = debug_generate_name_prefix(&request.node_name);
    let privileged = request.profile.is_privileged();
    serde_json::from_value(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {
            "generateName": prefix,
            "labels": {
                "app.kubernetes.io/managed-by": "kubectui",
                "kubectui.io/node-debug": "true",
                "kubectui.io/source-node": request.node_name,
            },
            "annotations": {
                "kubectui.io/node-debug-profile": request.profile.label().to_ascii_lowercase(),
            },
        },
        "spec": {
            "automountServiceAccountToken": false,
            "containers": [{
                "name": DEBUG_CONTAINER_NAME,
                "image": request.image,
                "command": ["/bin/sh", "-c", "trap 'exit 0' TERM INT; while true; do sleep 3600; done"],
                "stdin": true,
                "tty": true,
                "securityContext": {
                    "privileged": privileged,
                    "runAsUser": 0,
                    "allowPrivilegeEscalation": privileged,
                },
                "volumeMounts": [{
                    "name": "host-root",
                    "mountPath": "/host",
                }],
            }],
            "dnsPolicy": "ClusterFirstWithHostNet",
            "hostIPC": true,
            "hostNetwork": true,
            "hostPID": true,
            "nodeName": request.node_name,
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": 0,
            "tolerations": [
                {"operator": "Exists", "effect": "NoSchedule"},
                {"operator": "Exists", "effect": "NoExecute"},
                {"operator": "Exists", "effect": "PreferNoSchedule"}
            ],
            "volumes": [{
                "name": "host-root",
                "hostPath": {
                    "path": "/",
                    "type": "Directory"
                }
            }]
        }
    }))
    .context("failed to build node debug pod manifest")
}

async fn wait_for_node_debug_pod_ready(pods: &Api<Pod>, pod_name: &str) -> Result<()> {
    let deadline = tokio::time::Instant::now() + DEBUG_POD_READY_TIMEOUT;
    loop {
        let pod = pods
            .get(pod_name)
            .await
            .with_context(|| format!("failed to inspect node debug pod '{pod_name}'"))?;

        if container_state_is_running(&pod) {
            return Ok(());
        }

        if let Some(reason) = failure_reason(&pod) {
            return Err(anyhow!(
                "node debug pod '{}' failed before shell attach: {}",
                pod_name,
                reason
            ));
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for node debug pod '{}' to become ready",
                pod_name
            ));
        }

        tokio::time::sleep(DEBUG_POD_READY_POLL_INTERVAL).await;
    }
}

fn container_state_is_running(pod: &Pod) -> bool {
    pod.status
        .as_ref()
        .and_then(|status| status.container_statuses.as_ref())
        .and_then(|statuses| {
            statuses
                .iter()
                .find(|status| status.name == DEBUG_CONTAINER_NAME)
        })
        .and_then(|status| status.state.as_ref())
        .and_then(|state| state.running.as_ref())
        .is_some()
}

fn failure_reason(pod: &Pod) -> Option<String> {
    if let Some(status) = pod.status.as_ref() {
        if matches!(status.phase.as_deref(), Some("Failed" | "Succeeded")) {
            return Some(status.message.clone().unwrap_or_else(|| {
                status
                    .reason
                    .clone()
                    .unwrap_or_else(|| "pod terminated".to_string())
            }));
        }

        if let Some(container_statuses) = status.container_statuses.as_ref()
            && let Some(container) = container_statuses
                .iter()
                .find(|container| container.name == DEBUG_CONTAINER_NAME)
            && let Some(state) = container.state.as_ref()
        {
            if let Some(waiting) = state.waiting.as_ref()
                && waiting.reason.as_deref() != Some("ContainerCreating")
            {
                return Some(
                    waiting
                        .message
                        .clone()
                        .or_else(|| waiting.reason.clone())
                        .unwrap_or_else(|| "container is waiting".to_string()),
                );
            }
            if let Some(terminated) = state.terminated.as_ref() {
                return Some(
                    terminated
                        .message
                        .clone()
                        .or_else(|| terminated.reason.clone())
                        .unwrap_or_else(|| {
                            format!("container exited with code {}", terminated.exit_code)
                        }),
                );
            }
        }
    }

    None
}

fn debug_generate_name_prefix(node_name: &str) -> String {
    let mut sanitized = node_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if sanitized.is_empty() {
        sanitized = "node".to_string();
    }
    sanitized.truncate(24);
    format!("kubectui-node-debug-{sanitized}-")
}

pub fn default_debug_image(preset: DebugImagePreset, custom_image: &str) -> Option<String> {
    match preset {
        DebugImagePreset::Custom => {
            let trimmed = custom_image.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        _ => preset.default_image().map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sysadmin_profile_is_privileged() {
        assert!(NodeDebugProfile::Sysadmin.is_privileged());
        assert!(!NodeDebugProfile::General.is_privileged());
    }

    #[test]
    fn build_manifest_sets_host_mounts_and_privilege() {
        let manifest = build_debug_pod(&NodeDebugLaunchRequest {
            node_name: "node-0.example".to_string(),
            namespace: "default".to_string(),
            image: "busybox:1.37".to_string(),
            profile: NodeDebugProfile::Sysadmin,
        })
        .expect("manifest");
        let spec = manifest.spec.expect("spec");
        assert_eq!(spec.node_name.as_deref(), Some("node-0.example"));
        assert_eq!(spec.host_network, Some(true));
        assert_eq!(spec.host_pid, Some(true));
        assert_eq!(spec.host_ipc, Some(true));
        assert_eq!(spec.dns_policy.as_deref(), Some("ClusterFirstWithHostNet"));
        let container = spec.containers.first().expect("container");
        assert_eq!(container.name, DEBUG_CONTAINER_NAME);
        assert_eq!(
            container
                .security_context
                .as_ref()
                .and_then(|ctx| ctx.privileged),
            Some(true)
        );
        assert_eq!(
            container
                .volume_mounts
                .as_ref()
                .and_then(|mounts| mounts.first())
                .map(|mount| mount.mount_path.as_str()),
            Some("/host")
        );
    }

    #[test]
    fn default_debug_image_uses_preset_or_custom_value() {
        assert_eq!(
            default_debug_image(DebugImagePreset::Busybox, ""),
            Some("busybox:1.37".to_string())
        );
        assert_eq!(
            default_debug_image(DebugImagePreset::Custom, " ghcr.io/acme/debug:1 "),
            Some("ghcr.io/acme/debug:1".to_string())
        );
        assert_eq!(default_debug_image(DebugImagePreset::Custom, " "), None);
    }

    #[test]
    fn generate_name_prefix_is_bounded_and_dns_friendly() {
        let prefix = debug_generate_name_prefix("ip-10-0-0-1.eu-west-1.compute.internal");
        assert!(prefix.starts_with("kubectui-node-debug-"));
        assert!(prefix.ends_with('-'));
        assert!(prefix.len() <= 63);
        assert!(!prefix.contains('.'));
    }
}
