//! Kubernetes exec session support for workbench-hosted pod shells.

use std::{future::Future, pin::Pin, time::Duration};

use anyhow::{Context, Result, anyhow};
use k8s_openapi::{
    api::core::v1::{ContainerState, EphemeralContainer, Pod},
    apimachinery::pkg::apis::meta::v1::Status,
};
use kube::{
    Api,
    api::{AttachParams, AttachedProcess, Patch, PatchParams},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot},
};

use crate::k8s::client::K8sClient;

const SHELL_READY_GRACE_PERIOD_MS: u64 = 250;
const READ_CHUNK_SIZE: usize = 1024;
const DEBUG_CONTAINER_NAME_PREFIX: &str = "kubectui-debug";
const DEBUG_CONTAINER_READY_TIMEOUT: Duration = Duration::from_secs(30);
const DEBUG_CONTAINER_READY_POLL_INTERVAL: Duration = Duration::from_millis(250);

const SHELL_FALLBACKS: &[(&str, &[&str])] = &[
    ("/bin/bash", &["/bin/bash", "-i"]),
    ("/bin/sh", &["/bin/sh", "-i"]),
    ("/busybox/sh", &["/busybox/sh", "-i"]),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum DebugImagePreset {
    #[default]
    Busybox,
    Netshoot,
    Alpine,
    Ubuntu,
    Custom,
}

impl DebugImagePreset {
    pub const ALL: [Self; 5] = [
        Self::Busybox,
        Self::Netshoot,
        Self::Alpine,
        Self::Ubuntu,
        Self::Custom,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Busybox => "Busybox",
            Self::Netshoot => "Netshoot",
            Self::Alpine => "Alpine",
            Self::Ubuntu => "Ubuntu",
            Self::Custom => "Custom",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Busybox => "small shell",
            Self::Netshoot => "network toolbox",
            Self::Alpine => "general shell",
            Self::Ubuntu => "full userspace",
            Self::Custom => "custom registry image",
        }
    }

    pub const fn default_image(self) -> Option<&'static str> {
        match self {
            Self::Busybox => Some("busybox:1.37"),
            Self::Netshoot => Some("nicolaka/netshoot:latest"),
            Self::Alpine => Some("alpine:3.22"),
            Self::Ubuntu => Some("ubuntu:24.04"),
            Self::Custom => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugContainerLaunchRequest {
    pub pod_name: String,
    pub namespace: String,
    pub image: String,
    pub target_container_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugContainerLaunchResult {
    pub pod_name: String,
    pub namespace: String,
    pub container_name: String,
    pub image: String,
}

pub struct ExecSessionHandle {
    pub input_tx: mpsc::Sender<Vec<u8>>,
    pub cancel_tx: oneshot::Sender<()>,
}

#[derive(Debug, Clone)]
pub enum ExecEvent {
    Opened {
        session_id: u64,
        shell: String,
    },
    Output {
        session_id: u64,
        chunk: String,
        is_stderr: bool,
    },
    Exited {
        session_id: u64,
        success: bool,
        message: String,
    },
    Error {
        session_id: u64,
        error: String,
    },
}

type StatusFuture = Pin<Box<dyn Future<Output = Option<Status>> + Send>>;

pub async fn fetch_pod_containers(
    client: &K8sClient,
    pod_name: &str,
    namespace: &str,
) -> Result<Vec<String>> {
    let pods: Api<Pod> = Api::namespaced(client.get_client(), namespace);
    let pod = pods
        .get(pod_name)
        .await
        .with_context(|| format!("failed to fetch Pod '{pod_name}'"))?;
    let containers = pod
        .spec
        .context("pod missing spec")?
        .containers
        .into_iter()
        .map(|container| container.name)
        .collect::<Vec<_>>();
    if containers.is_empty() {
        return Err(anyhow!("pod '{pod_name}' has no containers"));
    }
    Ok(containers)
}

pub async fn launch_debug_container(
    client: &K8sClient,
    request: &DebugContainerLaunchRequest,
) -> Result<DebugContainerLaunchResult> {
    let pods: Api<Pod> = Api::namespaced(client.get_client(), &request.namespace);
    let pod = pods
        .get(&request.pod_name)
        .await
        .with_context(|| format!("failed to fetch Pod '{}'", request.pod_name))?;
    ensure_pod_supports_debug_container(&pod, &request.pod_name)?;
    let container_name = next_debug_container_name(&pod);
    let patch = serde_json::json!({
        "spec": {
            "ephemeralContainers": [
                build_debug_ephemeral_container(
                    &container_name,
                    &request.image,
                    request.target_container_name.as_deref(),
                )
            ]
        }
    });
    let patch_params = PatchParams {
        field_manager: Some("kubectui".to_string()),
        ..Default::default()
    };

    pods.patch_ephemeral_containers(&request.pod_name, &patch_params, &Patch::Strategic(&patch))
        .await
        .with_context(|| {
            format!(
                "failed to create debug container on Pod '{}' in namespace '{}'",
                request.pod_name, request.namespace
            )
        })?;

    wait_for_debug_container_ready(&pods, &request.pod_name, &container_name).await?;

    Ok(DebugContainerLaunchResult {
        pod_name: request.pod_name.clone(),
        namespace: request.namespace.clone(),
        container_name,
        image: request.image.clone(),
    })
}

pub async fn spawn_exec_session(
    client: K8sClient,
    session_id: u64,
    pod_name: String,
    namespace: String,
    container_name: String,
    update_tx: mpsc::Sender<ExecEvent>,
) -> Result<ExecSessionHandle> {
    let (input_tx, input_rx) = mpsc::channel(128);
    let (cancel_tx, cancel_rx) = oneshot::channel();
    tokio::spawn(async move {
        if let Err(err) = run_exec_session(
            client,
            session_id,
            pod_name,
            namespace,
            container_name,
            input_rx,
            cancel_rx,
            update_tx.clone(),
        )
        .await
        {
            let _ = update_tx
                .send(ExecEvent::Error {
                    session_id,
                    error: format!("{err:#}"),
                })
                .await;
        }
    });

    Ok(ExecSessionHandle {
        input_tx,
        cancel_tx,
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_exec_session(
    client: K8sClient,
    session_id: u64,
    pod_name: String,
    namespace: String,
    container_name: String,
    mut input_rx: mpsc::Receiver<Vec<u8>>,
    mut cancel_rx: oneshot::Receiver<()>,
    update_tx: mpsc::Sender<ExecEvent>,
) -> Result<()> {
    let pods: Api<Pod> = Api::namespaced(client.get_client(), &namespace);
    let (mut attached, shell, mut status_future) =
        open_shell_process(&pods, &pod_name, &container_name).await?;

    let _ = update_tx
        .send(ExecEvent::Opened {
            session_id,
            shell: shell.to_string(),
        })
        .await;

    let mut stdin = attached
        .stdin()
        .context("exec session missing stdin writer")?;
    let stdout = attached.stdout();
    let stderr = attached.stderr();
    let stdout_task = stdout.map(|reader| {
        tokio::spawn(pipe_exec_output(
            session_id,
            reader,
            false,
            update_tx.clone(),
        ))
    });
    let stderr_task = stderr.map(|reader| {
        tokio::spawn(pipe_exec_output(
            session_id,
            reader,
            true,
            update_tx.clone(),
        ))
    });

    loop {
        tokio::select! {
            maybe_input = input_rx.recv() => {
                match maybe_input {
                    Some(bytes) => {
                        stdin.write_all(&bytes).await.context("failed writing to exec stdin")?;
                        stdin.flush().await.context("failed flushing exec stdin")?;
                    }
                    None => {
                        attached.abort();
                        break;
                    }
                }
            }
            status = &mut status_future => {
                let (success, message) = match status {
                    Some(status) if status.status.as_deref() == Some("Success") => {
                        (true, format!("Shell exited successfully from {shell}."))
                    }
                    Some(status) => (
                        false,
                        status.message.unwrap_or_else(|| format!("{shell} exited.")),
                    ),
                    None => (false, format!("{shell} session ended without an exit status.")),
                };
                let _ = update_tx
                    .send(ExecEvent::Exited {
                        session_id,
                        success,
                        message,
                    })
                    .await;
                break;
            }
            _ = &mut cancel_rx => {
                attached.abort();
                let _ = update_tx
                    .send(ExecEvent::Exited {
                        session_id,
                        success: false,
                        message: "Exec session cancelled.".to_string(),
                    })
                    .await;
                break;
            }
        }
    }

    if let Some(task) = stdout_task {
        let _ = task.await;
    }
    if let Some(task) = stderr_task {
        let _ = task.await;
    }
    let _ = attached.join().await;
    Ok(())
}

async fn open_shell_process(
    pods: &Api<Pod>,
    pod_name: &str,
    container_name: &str,
) -> Result<(AttachedProcess, &'static str, StatusFuture)> {
    for (shell_label, command) in SHELL_FALLBACKS {
        let attach = AttachParams::default()
            .stdin(true)
            .stdout(true)
            .stderr(true)
            .container(container_name.to_string());
        let mut attached = pods
            .exec(pod_name, command.iter().copied(), &attach)
            .await
            .with_context(|| format!("failed to exec {} in Pod '{pod_name}'", command[0]))?;
        let mut status_future = attached
            .take_status()
            .map(|future| Box::pin(future) as StatusFuture)
            .context("exec session missing status future")?;

        if tokio::time::timeout(
            Duration::from_millis(SHELL_READY_GRACE_PERIOD_MS),
            &mut status_future,
        )
        .await
        .is_err()
        {
            return Ok((attached, shell_label, status_future));
        }
    }

    Err(anyhow!(
        "No supported shell was found. Tried /bin/bash, /bin/sh, and /busybox/sh."
    ))
}

async fn pipe_exec_output(
    session_id: u64,
    mut reader: impl AsyncRead + Unpin,
    is_stderr: bool,
    update_tx: mpsc::Sender<ExecEvent>,
) {
    let mut buf = vec![0u8; READ_CHUNK_SIZE];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                let _ = update_tx
                    .send(ExecEvent::Output {
                        session_id,
                        chunk,
                        is_stderr,
                    })
                    .await;
            }
            Err(err) => {
                let _ = update_tx
                    .send(ExecEvent::Error {
                        session_id,
                        error: format!("exec output read failed: {err}"),
                    })
                    .await;
                break;
            }
        }
    }
}

fn build_debug_ephemeral_container(
    name: &str,
    image: &str,
    target_container_name: Option<&str>,
) -> EphemeralContainer {
    EphemeralContainer {
        name: name.to_string(),
        image: Some(image.to_string()),
        image_pull_policy: Some("IfNotPresent".to_string()),
        command: Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            "trap 'exit 0' TERM INT; while true; do sleep 3600; done".to_string(),
        ]),
        stdin: Some(true),
        tty: Some(true),
        target_container_name: target_container_name.map(str::to_string),
        ..EphemeralContainer::default()
    }
}

fn ensure_pod_supports_debug_container(pod: &Pod, pod_name: &str) -> Result<()> {
    let phase = pod
        .status
        .as_ref()
        .and_then(|status| status.phase.as_deref())
        .unwrap_or("Unknown");
    if phase != "Running" {
        return Err(anyhow!(
            "Pod '{pod_name}' is not running (phase: {phase}). Debug containers can only be attached to running pods."
        ));
    }
    if pod.metadata.deletion_timestamp.is_some() {
        return Err(anyhow!(
            "Pod '{pod_name}' is terminating. Wait for a healthy replacement before launching a debug container."
        ));
    }
    let has_containers = pod
        .spec
        .as_ref()
        .is_some_and(|spec| !spec.containers.is_empty());
    if !has_containers {
        return Err(anyhow!(
            "Pod '{pod_name}' has no regular containers to debug."
        ));
    }
    Ok(())
}

fn next_debug_container_name(pod: &Pod) -> String {
    let mut taken = std::collections::BTreeSet::new();
    if let Some(spec) = &pod.spec {
        taken.extend(
            spec.containers
                .iter()
                .map(|container| container.name.as_str()),
        );
        taken.extend(
            spec.init_containers
                .iter()
                .flatten()
                .map(|container| container.name.as_str()),
        );
        taken.extend(
            spec.ephemeral_containers
                .iter()
                .flatten()
                .map(|container| container.name.as_str()),
        );
    }

    if !taken.contains(DEBUG_CONTAINER_NAME_PREFIX) {
        return DEBUG_CONTAINER_NAME_PREFIX.to_string();
    }

    for suffix in 1..u32::MAX {
        let candidate = format!("{DEBUG_CONTAINER_NAME_PREFIX}-{suffix}");
        if !taken.contains(candidate.as_str()) {
            return candidate;
        }
    }

    format!("{DEBUG_CONTAINER_NAME_PREFIX}-overflow")
}

async fn wait_for_debug_container_ready(
    pods: &Api<Pod>,
    pod_name: &str,
    container_name: &str,
) -> Result<()> {
    let started = tokio::time::Instant::now();
    let mut last_state: Option<String> = None;

    loop {
        let pod = pods
            .get(pod_name)
            .await
            .with_context(|| format!("failed to re-fetch Pod '{pod_name}'"))?;
        if let Some(status) = pod.status.as_ref()
            && let Some(container_status) = status
                .ephemeral_container_statuses
                .as_ref()
                .and_then(|statuses| statuses.iter().find(|status| status.name == container_name))
        {
            if container_status
                .state
                .as_ref()
                .is_some_and(container_state_is_running)
            {
                return Ok(());
            }

            last_state = container_status
                .state
                .as_ref()
                .and_then(describe_debug_container_state);
        }

        if started.elapsed() >= DEBUG_CONTAINER_READY_TIMEOUT {
            let suffix = last_state
                .map(|state| format!(" Last observed state: {state}."))
                .unwrap_or_default();
            return Err(anyhow!(
                "Timed out waiting for debug container '{container_name}' to become runnable.{suffix}"
            ));
        }

        tokio::time::sleep(DEBUG_CONTAINER_READY_POLL_INTERVAL).await;
    }
}

fn container_state_is_running(state: &ContainerState) -> bool {
    state.running.is_some()
}

fn describe_debug_container_state(state: &ContainerState) -> Option<String> {
    if let Some(waiting) = &state.waiting {
        let reason = waiting.reason.as_deref().unwrap_or("Waiting");
        let message = waiting
            .message
            .as_deref()
            .unwrap_or("container is still starting");
        return Some(format!("{reason}: {message}"));
    }
    if let Some(terminated) = &state.terminated {
        let reason = terminated.reason.as_deref().unwrap_or("Terminated");
        let message = terminated
            .message
            .as_deref()
            .unwrap_or("container terminated before a shell could be attached");
        return Some(format!("{reason}: {message}"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{Container, PodSpec, PodStatus};
    use kube::core::ObjectMeta;

    #[test]
    fn shell_fallbacks_are_ordered() {
        assert_eq!(SHELL_FALLBACKS[0].0, "/bin/bash");
        assert_eq!(SHELL_FALLBACKS[1].0, "/bin/sh");
        assert_eq!(SHELL_FALLBACKS[2].0, "/busybox/sh");
    }

    #[test]
    fn debug_presets_have_expected_default_images() {
        assert_eq!(
            DebugImagePreset::Netshoot.default_image(),
            Some("nicolaka/netshoot:latest")
        );
        assert_eq!(DebugImagePreset::Custom.default_image(), None);
    }

    #[test]
    fn build_debug_ephemeral_container_sets_target_when_requested() {
        let container =
            build_debug_ephemeral_container("kubectui-debug", "busybox:1.37", Some("app"));
        assert_eq!(container.image.as_deref(), Some("busybox:1.37"));
        assert_eq!(container.target_container_name.as_deref(), Some("app"));
        assert_eq!(
            container.command.as_deref(),
            Some(
                &[
                    "sh".to_string(),
                    "-c".to_string(),
                    "trap 'exit 0' TERM INT; while true; do sleep 3600; done".to_string(),
                ][..]
            )
        );
    }

    #[test]
    fn next_debug_container_name_skips_existing_names() {
        let pod = Pod {
            metadata: ObjectMeta::default(),
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "app".to_string(),
                    ..Container::default()
                }],
                ephemeral_containers: Some(vec![EphemeralContainer {
                    name: "kubectui-debug".to_string(),
                    ..EphemeralContainer::default()
                }]),
                ..PodSpec::default()
            }),
            status: None,
        };

        assert_eq!(next_debug_container_name(&pod), "kubectui-debug-1");
    }

    #[test]
    fn ensure_pod_supports_debug_container_rejects_non_running_pods() {
        let pod = Pod {
            metadata: ObjectMeta::default(),
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "app".to_string(),
                    ..Container::default()
                }],
                ..PodSpec::default()
            }),
            status: Some(PodStatus {
                phase: Some("Pending".to_string()),
                ..PodStatus::default()
            }),
        };

        let error = ensure_pod_supports_debug_container(&pod, "api-0").expect_err("non-running");
        assert!(error.to_string().contains("is not running"));
    }
}
