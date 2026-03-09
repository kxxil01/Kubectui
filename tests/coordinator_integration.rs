//! Integration tests for the UpdateCoordinator with KIND cluster.

#[cfg(test)]
mod integration_tests {
    use kubectui::coordinator::UpdateCoordinator;
    use kubectui::k8s::client::K8sClient;
    use std::time::Duration;
    use tokio::sync::mpsc;

    /// Test that we can connect to the KIND cluster
    #[tokio::test]
    #[ignore] // Only run when KIND cluster is available
    async fn test_connect_to_kind_cluster() {
        let client = K8sClient::connect().await;
        assert!(client.is_ok(), "Failed to connect to K8s cluster");
    }

    /// Test probe polling on a real pod (requires KIND cluster with probes-test pod)
    #[tokio::test]
    #[ignore] // Only run when KIND cluster is available
    async fn test_probe_polling_real_pod() {
        let client = K8sClient::connect()
            .await
            .expect("Failed to connect to K8s cluster");

        let (update_tx, mut update_rx) = mpsc::channel(4096);
        let coordinator = UpdateCoordinator::new(client, update_tx);

        // Start polling for a pod
        coordinator
            .start_probe_polling("probes-test".to_string(), "default".to_string())
            .await
            .expect("Failed to start probe polling");

        // Wait a bit for the first poll
        tokio::time::sleep(Duration::from_secs(3)).await;

        // Should have received a probe update or error
        let mut received_update = false;
        while let Ok(Some(msg)) =
            tokio::time::timeout(Duration::from_millis(100), update_rx.recv()).await
        {
            use kubectui::coordinator::UpdateMessage;
            match msg {
                UpdateMessage::ProbeUpdate { .. } => {
                    received_update = true;
                    break;
                }
                UpdateMessage::ProbeError { .. } => {
                    // This is also acceptable (pod might not exist)
                    received_update = true;
                    break;
                }
                _ => {}
            }
        }

        coordinator.shutdown().await.ok();
        assert!(received_update, "No probe update received after polling");
    }

    /// Test that multiple pods can be polled concurrently
    #[tokio::test]
    #[ignore] // Only run when KIND cluster is available
    async fn test_concurrent_probe_polling() {
        let client = K8sClient::connect()
            .await
            .expect("Failed to connect to K8s cluster");

        let (update_tx, _update_rx) = mpsc::channel(4096);
        let coordinator = UpdateCoordinator::new(client, update_tx);

        // Start polling for multiple pods
        coordinator
            .start_probe_polling("pod-1".to_string(), "default".to_string())
            .await
            .ok();
        coordinator
            .start_probe_polling("pod-2".to_string(), "default".to_string())
            .await
            .ok();
        coordinator
            .start_probe_polling("pod-3".to_string(), "default".to_string())
            .await
            .ok();

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Should have started 3 tasks (even if they fail to find the pods)
        let active = coordinator.active_probe_tasks().await;
        assert!(active > 0, "No probe tasks started");

        coordinator.shutdown().await.ok();

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            coordinator.active_probe_tasks().await,
            0,
            "Tasks not cleaned up"
        );
    }

    /// Test log streaming on a real pod
    #[tokio::test]
    #[ignore] // Only run when KIND cluster is available
    async fn test_log_streaming_real_pod() {
        let client = K8sClient::connect()
            .await
            .expect("Failed to connect to K8s cluster");

        let (update_tx, mut update_rx) = mpsc::channel(4096);
        let coordinator = UpdateCoordinator::new(client, update_tx);

        // Start streaming logs
        coordinator
            .start_log_streaming(
                "nginx".to_string(),
                "default".to_string(),
                "nginx".to_string(),
                false, // Don't follow, just get recent logs
                false,
                false,
            )
            .await
            .ok();

        // Wait for logs to be streamed
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Should have received log updates or stream status
        let mut message_count = 0;
        while message_count < 10 {
            if let Ok(Some(msg)) =
                tokio::time::timeout(Duration::from_millis(200), update_rx.recv()).await
            {
                use kubectui::coordinator::UpdateMessage;
                match msg {
                    UpdateMessage::LogUpdate { .. } | UpdateMessage::LogStreamStatus { .. } => {
                        message_count += 1;
                    }
                    _ => {}
                }
            } else {
                break;
            }
        }

        coordinator.shutdown().await.ok();
        // Either we got logs or got stream status (pod might not exist but that's ok)
        assert!(message_count > 0, "No messages received from log streaming");
    }

    /// Test that stopping probe polling actually stops the task
    #[tokio::test]
    #[ignore] // Only run when KIND cluster is available
    async fn test_stop_probe_polling() {
        let client = K8sClient::connect()
            .await
            .expect("Failed to connect to K8s cluster");

        let (update_tx, _update_rx) = mpsc::channel(4096);
        let coordinator = UpdateCoordinator::new(client, update_tx);

        // Start polling
        coordinator
            .start_probe_polling("test-pod".to_string(), "default".to_string())
            .await
            .ok();

        tokio::time::sleep(Duration::from_millis(100)).await;
        let tasks_before = coordinator.active_probe_tasks().await;

        // Stop polling
        coordinator
            .stop_probe_polling("test-pod", "default")
            .await
            .ok();

        tokio::time::sleep(Duration::from_millis(200)).await;
        let tasks_after = coordinator.active_probe_tasks().await;

        assert!(tasks_before > 0, "No tasks running before stop");
        assert_eq!(tasks_after, 0, "Tasks not stopped properly");

        coordinator.shutdown().await.ok();
    }

    /// Test memory cleanup on shutdown
    #[tokio::test]
    #[ignore] // Only run when KIND cluster is available
    async fn test_memory_cleanup_on_shutdown() {
        let client = K8sClient::connect()
            .await
            .expect("Failed to connect to K8s cluster");

        let (update_tx, _update_rx) = mpsc::channel(4096);
        let coordinator = UpdateCoordinator::new(client, update_tx);

        // Start many tasks
        for i in 0..10 {
            coordinator
                .start_probe_polling(format!("pod-{}", i), "default".to_string())
                .await
                .ok();

            coordinator
                .start_log_streaming(
                    format!("pod-{}", i),
                    "default".to_string(),
                    format!("container-{}", i),
                    false,
                    false,
                    false,
                )
                .await
                .ok();
        }

        tokio::time::sleep(Duration::from_millis(200)).await;

        let probe_tasks_before = coordinator.active_probe_tasks().await;
        let log_tasks_before = coordinator.active_log_tasks().await;

        // Shutdown
        coordinator.shutdown().await.ok();

        tokio::time::sleep(Duration::from_millis(200)).await;

        let probe_tasks_after = coordinator.active_probe_tasks().await;
        let log_tasks_after = coordinator.active_log_tasks().await;

        println!(
            "Probe tasks before: {}, after: {}",
            probe_tasks_before, probe_tasks_after
        );
        println!(
            "Log tasks before: {}, after: {}",
            log_tasks_before, log_tasks_after
        );

        assert!(probe_tasks_before > 0, "No probe tasks started");
        assert!(log_tasks_before > 0, "No log tasks started");
        assert_eq!(probe_tasks_after, 0, "Probe tasks not cleaned up");
        assert_eq!(log_tasks_after, 0, "Log tasks not cleaned up");
    }
}
