//! Background task for streaming pod logs in real-time.

use std::sync::Arc;
use tokio::sync::mpsc;
use kube::Api;
use k8s_openapi::api::core::v1::Pod;

use crate::k8s::client::K8sClient;
use crate::k8s::logs::PodRef;
use super::{UpdateMessage, LogStreamStatus};

/// Stream logs for a pod container.
///
/// This task continuously reads logs from a pod container and sends them
/// to the main event loop. It supports following logs (tail mode) or reading
/// a fixed number of recent lines.
///
/// # Arguments
///
/// * `client` - K8s client for API calls
/// * `pod_ref` - Pod reference (name and namespace)
/// * `container_name` - Container name within the pod
/// * `follow` - If true, follow new logs; if false, read recent logs only
/// * `update_tx` - Channel to send log lines and status
/// * `mut cancel_rx` - Receiver for cancellation signal
pub async fn stream_logs(
    client: Arc<K8sClient>,
    pod_ref: PodRef,
    container_name: String,
    follow: bool,
    update_tx: mpsc::UnboundedSender<UpdateMessage>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    // Send status: starting
    let _ = update_tx.send(UpdateMessage::LogStreamStatus {
        pod_name: pod_ref.name.clone(),
        container_name: container_name.clone(),
        status: LogStreamStatus::Started,
    });

    // Attempt to stream logs
    match stream_logs_internal(&client, &pod_ref, &container_name, follow, &update_tx, &mut cancel_rx).await {
        Ok(_) => {
            // Send status: ended normally
            let _ = update_tx.send(UpdateMessage::LogStreamStatus {
                pod_name: pod_ref.name.clone(),
                container_name: container_name.clone(),
                status: LogStreamStatus::Ended,
            });
        }
        Err(e) => {
            // Send status: error
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
    _follow: bool,
    update_tx: &mpsc::UnboundedSender<UpdateMessage>,
    cancel_rx: &mut tokio::sync::oneshot::Receiver<()>,
) -> anyhow::Result<()> {
    // Verify pod exists
    let pods_api: Api<Pod> = Api::namespaced(client.get_client(), &pod_ref.namespace);
    let _pod = pods_api.get(&pod_ref.name).await?;

    // TODO: Implement actual log streaming using kube-rs log API
    // For now, we just send a placeholder log line to demonstrate the framework
    
    // Simulate log streaming with placeholder data
    let mut poll_interval = tokio::time::interval(std::time::Duration::from_secs(1));
    
    loop {
        tokio::select! {
            _ = poll_interval.tick() => {
                // Send a sample log line (in real implementation, this would come from kube-rs)
                let msg = UpdateMessage::LogUpdate {
                    pod_name: pod_ref.name.clone(),
                    container_name: container_name.to_string(),
                    line: format!("Log: {} - {}", chrono::Utc::now(), container_name),
                };
                if update_tx.send(msg).is_err() {
                    // Channel closed, exit task
                    break;
                }
            }
            _ = cancel_rx => {
                // Cancellation signal received
                let _ = update_tx.send(UpdateMessage::LogStreamStatus {
                    pod_name: pod_ref.name.clone(),
                    container_name: container_name.to_string(),
                    status: LogStreamStatus::Cancelled,
                });
                break;
            }
        }
    }

    Ok(())
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
