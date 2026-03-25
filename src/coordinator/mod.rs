//! Real-time update coordinator for managing background polling and streaming tasks.
//!
//! This module provides the UpdateCoordinator which manages:
//! - Probe polling (2s intervals)
//! - Log streaming (follow mode)
//! - Tunnel status monitoring
//! - Event streaming
//!
//! The coordinator spawns background tokio tasks and sends updates through mpsc channels
//! to the main event loop for rendering.

pub mod logs;
pub mod probes;

use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::mpsc;

use crate::k8s::client::K8sClient;
use crate::k8s::logs::PodRef;
use crate::k8s::probes::ContainerProbes;

/// Update types sent from background tasks to the main event loop.
#[derive(Debug, Clone)]
pub enum UpdateMessage {
    /// Probe status update for a pod
    ProbeUpdate {
        pod_name: String,
        namespace: String,
        probes: Vec<(String, ContainerProbes)>,
    },
    /// Log line update
    LogUpdate {
        pod_name: String,
        namespace: String,
        container_name: String,
        line: String,
    },
    /// Log streaming error or end
    LogStreamStatus {
        pod_name: String,
        namespace: String,
        container_name: String,
        status: LogStreamStatus,
    },
    /// Probe polling error
    ProbeError {
        pod_name: String,
        namespace: String,
        error: String,
    },
}

/// Status of a log stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogStreamStatus {
    Started,
    Ended,
    Error(String),
    Cancelled,
}

/// Reference to a running background task that can be cancelled.
#[derive(Debug, Clone)]
pub struct TaskHandle {
    cancel_tx: Arc<RwLock<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl TaskHandle {
    /// Create a new task handle with a cancellation channel.
    pub fn new(cancel_tx: tokio::sync::oneshot::Sender<()>) -> Self {
        Self {
            cancel_tx: Arc::new(RwLock::new(Some(cancel_tx))),
        }
    }

    /// Cancel the task.
    pub async fn cancel(&self) {
        let mut guard = self.cancel_tx.write().await;
        if let Some(tx) = guard.take() {
            let _ = tx.send(());
        }
    }
}

/// Coordinates all real-time updates from K8s API.
///
/// Manages background tasks for:
/// - Polling probe status every 2 seconds
/// - Streaming pod logs
/// - Monitoring tunnel status
/// - Watching events
#[derive(Clone)]
pub struct UpdateCoordinator {
    client: Arc<K8sClient>,
    /// Channel to send updates to the main event loop
    update_tx: mpsc::Sender<UpdateMessage>,
    /// Active probe polling tasks (keyed by pod_ref: "namespace/name")
    probe_tasks: Arc<RwLock<HashMap<String, TaskHandle>>>,
    /// Active log streaming tasks (keyed by pod_ref + container)
    log_tasks: Arc<RwLock<HashMap<String, TaskHandle>>>,
}

impl UpdateCoordinator {
    /// Create a new UpdateCoordinator.
    pub fn new(client: K8sClient, update_tx: mpsc::Sender<UpdateMessage>) -> Self {
        Self {
            client: Arc::new(client),
            update_tx,
            probe_tasks: Arc::new(RwLock::new(HashMap::new())),
            log_tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start polling probes for a specific pod.
    pub async fn start_probe_polling(&self, pod_name: String, namespace: String) -> Result<()> {
        let key = format!("{}/{}", namespace, pod_name);

        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let handle = TaskHandle::new(cancel_tx);

        // Atomically check-and-insert under a single write lock to avoid TOCTOU races.
        {
            let mut tasks = self.probe_tasks.write().await;
            if tasks.contains_key(&key) {
                return Ok(());
            }
            tasks.insert(key.clone(), handle);
        }

        let coordinator = self.clone();
        let client = self.client.clone();
        let pod_name_clone = pod_name.clone();
        let namespace_clone = namespace.clone();
        let key_clone = key.clone();

        tokio::spawn(async move {
            let fut = std::panic::AssertUnwindSafe(probes::poll_probes_loop(
                client,
                pod_name_clone,
                namespace_clone.clone(),
                coordinator.update_tx.clone(),
                cancel_rx,
            ));
            if futures::FutureExt::catch_unwind(fut).await.is_err() {
                tracing::error!("probe polling task panicked for {key_clone}");
            }

            // Clean up the task handle
            let mut tasks = coordinator.probe_tasks.write().await;
            tasks.remove(&key_clone);
        });

        Ok(())
    }

    /// Stop polling probes for a specific pod.
    pub async fn stop_probe_polling(&self, pod_name: &str, namespace: &str) -> Result<()> {
        let key = format!("{}/{}", namespace, pod_name);

        let mut tasks = self.probe_tasks.write().await;
        if let Some(handle) = tasks.remove(&key) {
            handle.cancel().await;
        }

        Ok(())
    }

    /// Start streaming logs for a pod container.
    pub async fn start_log_streaming(
        &self,
        pod_name: String,
        namespace: String,
        container_name: String,
        follow: bool,
        previous: bool,
        timestamps: bool,
    ) -> Result<()> {
        let key = format!("{}/{}/{}", namespace, pod_name, container_name);

        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let handle = TaskHandle::new(cancel_tx);

        // Atomically check-and-insert under a single write lock to avoid TOCTOU races.
        {
            let mut tasks = self.log_tasks.write().await;
            if tasks.contains_key(&key) {
                return Ok(());
            }
            tasks.insert(key.clone(), handle);
        }

        let coordinator = self.clone();
        let client = self.client.clone();
        let pod_name_clone = pod_name.clone();
        let namespace_clone = namespace.clone();
        let container_name_clone = container_name.clone();
        let key_clone = key.clone();

        tokio::spawn(async move {
            let fut = std::panic::AssertUnwindSafe(logs::stream_logs(
                client,
                PodRef::new(pod_name_clone, namespace_clone.clone()),
                container_name_clone.clone(),
                follow,
                previous,
                timestamps,
                coordinator.update_tx.clone(),
                cancel_rx,
            ));
            if futures::FutureExt::catch_unwind(fut).await.is_err() {
                tracing::error!("log streaming task panicked for {key_clone}");
            }

            // Clean up the task handle
            let mut tasks = coordinator.log_tasks.write().await;
            tasks.remove(&key_clone);
        });

        Ok(())
    }

    /// Stop streaming logs for a specific pod container.
    pub async fn stop_log_streaming(
        &self,
        pod_name: &str,
        namespace: &str,
        container_name: &str,
    ) -> Result<()> {
        let key = format!("{}/{}/{}", namespace, pod_name, container_name);

        let mut tasks = self.log_tasks.write().await;
        if let Some(handle) = tasks.remove(&key) {
            handle.cancel().await;
        }

        Ok(())
    }

    /// Stop all background tasks for cleanup.
    pub async fn shutdown(&self) -> Result<()> {
        // Cancel all probe tasks
        {
            let mut tasks = self.probe_tasks.write().await;
            for (_, handle) in tasks.drain() {
                handle.cancel().await;
            }
        }

        // Cancel all log tasks
        {
            let mut tasks = self.log_tasks.write().await;
            for (_, handle) in tasks.drain() {
                handle.cancel().await;
            }
        }

        Ok(())
    }

    /// Get the number of active probe polling tasks.
    pub async fn active_probe_tasks(&self) -> usize {
        self.probe_tasks.read().await.len()
    }

    /// Get the number of active log streaming tasks.
    pub async fn active_log_tasks(&self) -> usize {
        self.log_tasks.read().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_handle_creation() {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let handle = TaskHandle::new(tx);
        assert!(handle.cancel_tx.blocking_read().is_some());
    }

    #[tokio::test]
    async fn test_coordinator_creation() {
        let (tx, _rx) = mpsc::channel(4096);
        let client = K8sClient::dummy();

        let coordinator = UpdateCoordinator::new(client, tx);

        assert_eq!(coordinator.active_probe_tasks().await, 0);
        assert_eq!(coordinator.active_log_tasks().await, 0);
    }
}
