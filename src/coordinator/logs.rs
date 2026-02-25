//! Background task for streaming pod logs in real-time.

use futures::io::AsyncBufReadExt;
use k8s_openapi::api::core::v1::Pod;
use kube::{Api, api::LogParams};
use std::sync::Arc;
use tokio::sync::mpsc;

use super::{LogStreamStatus, UpdateMessage};
use crate::k8s::client::K8sClient;
use crate::k8s::logs::PodRef;

/// Stream logs for a pod container.
pub async fn stream_logs(
    client: Arc<K8sClient>,
    pod_ref: PodRef,
    container_name: String,
    follow: bool,
    update_tx: mpsc::UnboundedSender<UpdateMessage>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let _ = update_tx.send(UpdateMessage::LogStreamStatus {
        pod_name: pod_ref.name.clone(),
        container_name: container_name.clone(),
        status: LogStreamStatus::Started,
    });

    let result = stream_logs_internal(
        &client,
        &pod_ref,
        &container_name,
        follow,
        &update_tx,
        &mut cancel_rx,
    )
    .await;

    match result {
        Ok(_) => {
            let _ = update_tx.send(UpdateMessage::LogStreamStatus {
                pod_name: pod_ref.name.clone(),
                container_name: container_name.clone(),
                status: LogStreamStatus::Ended,
            });
        }
        Err(e) => {
            let _ = update_tx.send(UpdateMessage::LogStreamStatus {
                pod_name: pod_ref.name.clone(),
                container_name: container_name.clone(),
                status: LogStreamStatus::Error(e.to_string()),
            });
        }
    }
}

async fn stream_logs_internal(
    client: &Arc<K8sClient>,
    pod_ref: &PodRef,
    container_name: &str,
    follow: bool,
    update_tx: &mpsc::UnboundedSender<UpdateMessage>,
    cancel_rx: &mut tokio::sync::oneshot::Receiver<()>,
) -> anyhow::Result<()> {
    let pods_api: Api<Pod> = Api::namespaced(client.get_client(), &pod_ref.namespace);

    let params = LogParams {
        container: Some(container_name.to_string()),
        follow,
        tail_lines: if follow { Some(100) } else { Some(500) },
        timestamps: false,
        ..Default::default()
    };

    if follow {
        // Use streaming API for follow mode
        let log_stream = pods_api.log_stream(&pod_ref.name, &params).await?;
        let mut lines = log_stream.lines();

        loop {
            tokio::select! {
                line_result = futures::StreamExt::next(&mut lines) => {
                    match line_result {
                        Some(Ok(line)) => {
                            if !line.is_empty() {
                                let msg = UpdateMessage::LogUpdate {
                                    pod_name: pod_ref.name.clone(),
                                    container_name: container_name.to_string(),
                                    line,
                                };
                                if update_tx.send(msg).is_err() {
                                    return Ok(());
                                }
                            }
                        }
                        Some(Err(e)) => return Err(anyhow::anyhow!("{e}")),
                        None => return Ok(()), // stream ended
                    }
                }
                _ = &mut *cancel_rx => {
                    let _ = update_tx.send(UpdateMessage::LogStreamStatus {
                        pod_name: pod_ref.name.clone(),
                        container_name: container_name.to_string(),
                        status: LogStreamStatus::Cancelled,
                    });
                    return Ok(());
                }
            }
        }
    } else {
        // Fetch all logs at once (non-follow mode)
        let raw = pods_api.logs(&pod_ref.name, &params).await?;
        for line in raw.lines() {
            if update_tx.send(UpdateMessage::LogUpdate {
                pod_name: pod_ref.name.clone(),
                container_name: container_name.to_string(),
                line: line.to_string(),
            }).is_err() {
                return Ok(());
            }
        }
        Ok(())
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
        let (tx, mut rx) = mpsc::unbounded_channel();

        let msg = UpdateMessage::LogStreamStatus {
            pod_name: "test-pod".to_string(),
            container_name: "test-container".to_string(),
            status: LogStreamStatus::Started,
        };

        tx.send(msg).unwrap();

        if let Some(UpdateMessage::LogStreamStatus { pod_name, container_name, status }) = rx.recv().await {
            assert_eq!(pod_name, "test-pod");
            assert_eq!(container_name, "test-container");
            assert_eq!(status, LogStreamStatus::Started);
        } else {
            panic!("Expected LogStreamStatus message");
        }
    }

    #[tokio::test]
    async fn test_log_update_message() {
        let (tx, mut rx) = mpsc::unbounded_channel();

        let msg = UpdateMessage::LogUpdate {
            pod_name: "test-pod".to_string(),
            container_name: "test-container".to_string(),
            line: "test log line".to_string(),
        };

        tx.send(msg).unwrap();

        if let Some(UpdateMessage::LogUpdate { pod_name, container_name, line }) = rx.recv().await {
            assert_eq!(pod_name, "test-pod");
            assert_eq!(container_name, "test-container");
            assert_eq!(line, "test log line");
        } else {
            panic!("Expected LogUpdate message");
        }
    }
}
