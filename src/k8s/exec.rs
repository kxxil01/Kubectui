//! Kubernetes exec session support for workbench-hosted pod shells.

use std::{future::Future, pin::Pin, time::Duration};

use anyhow::{Context, Result, anyhow};
use k8s_openapi::{api::core::v1::Pod, apimachinery::pkg::apis::meta::v1::Status};
use kube::{
    Api,
    api::{AttachParams, AttachedProcess},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot},
};

use crate::k8s::client::K8sClient;

const SHELL_READY_GRACE_PERIOD_MS: u64 = 250;
const READ_CHUNK_SIZE: usize = 1024;

const SHELL_FALLBACKS: &[(&str, &[&str])] = &[
    ("/bin/bash", &["/bin/bash", "-i"]),
    ("/bin/sh", &["/bin/sh", "-i"]),
    ("/busybox/sh", &["/busybox/sh", "-i"]),
];

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_fallbacks_are_ordered() {
        assert_eq!(SHELL_FALLBACKS[0].0, "/bin/bash");
        assert_eq!(SHELL_FALLBACKS[1].0, "/bin/sh");
        assert_eq!(SHELL_FALLBACKS[2].0, "/busybox/sh");
    }
}
