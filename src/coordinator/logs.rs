//! Background task for streaming pod logs in real-time.

use futures::io::AsyncBufReadExt;
use k8s_openapi::api::core::v1::Pod;
use kube::{Api, api::LogParams};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use super::{LogStreamStatus, UpdateMessage};
use crate::k8s::client::K8sClient;
use crate::k8s::logs::PodRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamOutcome {
    Ended,
    Cancelled,
}

/// Stream logs for a pod container.
#[allow(clippy::too_many_arguments)]
pub async fn stream_logs(
    client: Arc<K8sClient>,
    pod_ref: PodRef,
    container_name: String,
    follow: bool,
    previous: bool,
    timestamps: bool,
    update_tx: mpsc::Sender<UpdateMessage>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let _ = update_tx
        .send(UpdateMessage::LogStreamStatus {
            pod_name: pod_ref.name.clone(),
            namespace: pod_ref.namespace.clone(),
            container_name: container_name.clone(),
            status: LogStreamStatus::Started,
        })
        .await;

    let result = stream_logs_internal(
        &client,
        &pod_ref,
        &container_name,
        follow,
        previous,
        timestamps,
        &update_tx,
        &mut cancel_rx,
    )
    .await;

    match result {
        Ok(StreamOutcome::Ended) => {
            let _ = update_tx
                .send(UpdateMessage::LogStreamStatus {
                    pod_name: pod_ref.name.clone(),
                    namespace: pod_ref.namespace.clone(),
                    container_name: container_name.clone(),
                    status: LogStreamStatus::Ended,
                })
                .await;
        }
        Ok(StreamOutcome::Cancelled) => {
            let _ = update_tx
                .send(UpdateMessage::LogStreamStatus {
                    pod_name: pod_ref.name.clone(),
                    namespace: pod_ref.namespace.clone(),
                    container_name: container_name.clone(),
                    status: LogStreamStatus::Cancelled,
                })
                .await;
        }
        Err(e) => {
            let _ = update_tx
                .send(UpdateMessage::LogStreamStatus {
                    pod_name: pod_ref.name.clone(),
                    namespace: pod_ref.namespace.clone(),
                    container_name: container_name.clone(),
                    status: LogStreamStatus::Error(e.to_string()),
                })
                .await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn stream_logs_internal(
    client: &Arc<K8sClient>,
    pod_ref: &PodRef,
    container_name: &str,
    follow: bool,
    previous: bool,
    timestamps: bool,
    update_tx: &mpsc::Sender<UpdateMessage>,
    cancel_rx: &mut tokio::sync::oneshot::Receiver<()>,
) -> anyhow::Result<StreamOutcome> {
    let pods_api: Api<Pod> = Api::namespaced(client.get_client(), &pod_ref.namespace);

    let follow = if previous { false } else { follow };
    let params = LogParams {
        container: Some(container_name.to_string()),
        follow,
        previous,
        tail_lines: if follow && !previous {
            Some(100)
        } else {
            Some(500)
        },
        timestamps,
        ..Default::default()
    };

    if follow {
        // Use streaming API for follow mode
        let log_stream = pods_api.log_stream(&pod_ref.name, &params).await?;
        let mut lines = log_stream.lines();
        let idle_timeout = tokio::time::sleep(Duration::from_secs(300));
        tokio::pin!(idle_timeout);

        loop {
            tokio::select! {
                line_result = futures::StreamExt::next(&mut lines) => {
                    match line_result {
                        Some(Ok(line)) => {
                            if !line.is_empty() {
                                let msg = UpdateMessage::LogUpdate {
                                    pod_name: pod_ref.name.clone(),
                                    namespace: pod_ref.namespace.clone(),
                                    container_name: container_name.to_string(),
                                    line,
                                };
                                if update_tx.send(msg).await.is_err() {
                                    return Ok(StreamOutcome::Ended);
                                }
                            }
                        }
                        Some(Err(e)) => return Err(anyhow::anyhow!("{e}")),
                        None => return Ok(StreamOutcome::Ended), // stream ended
                    }
                    idle_timeout.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(300));
                }
                _ = &mut idle_timeout => {
                    return Ok(StreamOutcome::Ended);
                }
                _ = &mut *cancel_rx => {
                    return Ok(StreamOutcome::Cancelled);
                }
            }
        }
    } else {
        // Fetch all logs at once (non-follow mode)
        let raw = tokio::time::timeout(
            Duration::from_secs(30),
            pods_api.logs(&pod_ref.name, &params),
        )
        .await
        .map_err(|_| anyhow::anyhow!("log fetch timed out after 30s"))??;
        for line in raw.lines() {
            if update_tx
                .send(UpdateMessage::LogUpdate {
                    pod_name: pod_ref.name.clone(),
                    namespace: pod_ref.namespace.clone(),
                    container_name: container_name.to_string(),
                    line: line.to_string(),
                })
                .await
                .is_err()
            {
                return Ok(StreamOutcome::Ended);
            }
        }
        Ok(StreamOutcome::Ended)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pod_ref_creation() {
        let pod_ref = PodRef::new("test-pod".to_string(), "default".to_string());
        assert_eq!(pod_ref.name, "test-pod");
        assert_eq!(pod_ref.namespace, "default");
    }

    #[tokio::test]
    async fn test_log_stream_status_message() {
        let (tx, mut rx) = mpsc::channel(4096);

        let msg = UpdateMessage::LogStreamStatus {
            pod_name: "test-pod".to_string(),
            namespace: "default".to_string(),
            container_name: "test-container".to_string(),
            status: LogStreamStatus::Started,
        };

        tx.send(msg).await.unwrap();

        if let Some(UpdateMessage::LogStreamStatus {
            pod_name,
            namespace,
            container_name,
            status,
        }) = rx.recv().await
        {
            assert_eq!(pod_name, "test-pod");
            assert_eq!(namespace, "default");
            assert_eq!(container_name, "test-container");
            assert_eq!(status, LogStreamStatus::Started);
        } else {
            panic!("Expected LogStreamStatus message");
        }
    }

    #[tokio::test]
    async fn test_log_update_message() {
        let (tx, mut rx) = mpsc::channel(4096);

        let msg = UpdateMessage::LogUpdate {
            pod_name: "test-pod".to_string(),
            namespace: "default".to_string(),
            container_name: "test-container".to_string(),
            line: "test log line".to_string(),
        };

        tx.send(msg).await.unwrap();

        if let Some(UpdateMessage::LogUpdate {
            pod_name,
            namespace,
            container_name,
            line,
        }) = rx.recv().await
        {
            assert_eq!(pod_name, "test-pod");
            assert_eq!(namespace, "default");
            assert_eq!(container_name, "test-container");
            assert_eq!(line, "test log line");
        } else {
            panic!("Expected LogUpdate message");
        }
    }
}
