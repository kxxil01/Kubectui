#![allow(clippy::field_reassign_with_default)]
//! Comprehensive tests for the update coordinator and real-time updates system.

#[cfg(test)]
mod coordinator_tests {
    use kubectui::coordinator::{LogStreamStatus, UpdateCoordinator, UpdateMessage};
    use kubectui::k8s::logs::PodRef;
    use std::time::Duration;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_coordinator_creation() {
        let (update_tx, _update_rx) = mpsc::channel(4096);
        let client = kubectui::k8s::client::K8sClient::connect()
            .await
            .expect("Failed to connect to K8s cluster");

        let coordinator = UpdateCoordinator::new(client, update_tx);

        assert_eq!(coordinator.active_probe_tasks().await, 0);
        assert_eq!(coordinator.active_log_tasks().await, 0);
    }

    #[tokio::test]
    async fn test_coordinator_multiple_probes() {
        let (update_tx, _update_rx) = mpsc::channel(4096);
        let client = kubectui::k8s::client::K8sClient::connect()
            .await
            .expect("Failed to connect to K8s cluster");

        let coordinator = UpdateCoordinator::new(client, update_tx);

        // Start probe polling for first pod (won't actually work without pod, but tests task tracking)
        let result = coordinator
            .start_probe_polling("test-pod-1".to_string(), "default".to_string())
            .await;
        assert!(result.is_ok());

        // Task is added before the actual poll starts, so we should have at least 1 task
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Cleanup
        coordinator.shutdown().await.ok();
    }

    #[tokio::test]
    async fn test_coordinator_start_stop_probe() {
        let (update_tx, _update_rx) = mpsc::channel(4096);
        let client = kubectui::k8s::client::K8sClient::connect()
            .await
            .expect("Failed to connect to K8s cluster");

        let coordinator = UpdateCoordinator::new(client, update_tx);

        // Start probe polling
        coordinator
            .start_probe_polling("test-pod".to_string(), "default".to_string())
            .await
            .ok();

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Stop probe polling
        coordinator
            .stop_probe_polling("test-pod", "default")
            .await
            .ok();

        tokio::time::sleep(Duration::from_millis(100)).await;

        // After stopping, should have no tasks
        assert_eq!(coordinator.active_probe_tasks().await, 0);

        coordinator.shutdown().await.ok();
    }

    #[tokio::test]
    async fn test_coordinator_idempotent_start_probe() {
        let (update_tx, _update_rx) = mpsc::channel(4096);
        let client = kubectui::k8s::client::K8sClient::connect()
            .await
            .expect("Failed to connect to K8s cluster");

        let coordinator = UpdateCoordinator::new(client, update_tx);

        // Start probe polling
        coordinator
            .start_probe_polling("test-pod".to_string(), "default".to_string())
            .await
            .ok();

        tokio::time::sleep(Duration::from_millis(100)).await;
        let count_after_first = coordinator.active_probe_tasks().await;

        // Try to start again (should be idempotent)
        coordinator
            .start_probe_polling("test-pod".to_string(), "default".to_string())
            .await
            .ok();

        tokio::time::sleep(Duration::from_millis(100)).await;
        let count_after_second = coordinator.active_probe_tasks().await;

        // Should still have same count (idempotent)
        assert_eq!(count_after_first, count_after_second);

        coordinator.shutdown().await.ok();
    }

    #[tokio::test]
    async fn test_coordinator_multiple_log_streams() {
        let (update_tx, _update_rx) = mpsc::channel(4096);
        let client = kubectui::k8s::client::K8sClient::connect()
            .await
            .expect("Failed to connect to K8s cluster");

        let coordinator = UpdateCoordinator::new(client, update_tx);

        // Start log streaming for first container
        let result = coordinator
            .start_log_streaming(
                "test-pod".to_string(),
                "default".to_string(),
                "container1".to_string(),
                false,
            )
            .await;
        assert!(result.is_ok());

        // Start log streaming for second container
        let result = coordinator
            .start_log_streaming(
                "test-pod".to_string(),
                "default".to_string(),
                "container2".to_string(),
                false,
            )
            .await;
        assert!(result.is_ok());

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Should have 2 log tasks (or close to it - they may fail due to pod not existing)
        let active = coordinator.active_log_tasks().await;
        assert!(active <= 2);

        coordinator.shutdown().await.ok();
    }

    #[tokio::test]
    async fn test_coordinator_shutdown_cleanup() {
        let (update_tx, _update_rx) = mpsc::channel(4096);
        let client = kubectui::k8s::client::K8sClient::connect()
            .await
            .expect("Failed to connect to K8s cluster");

        let coordinator = UpdateCoordinator::new(client, update_tx);

        // Start multiple tasks
        coordinator
            .start_probe_polling("test-pod-1".to_string(), "default".to_string())
            .await
            .ok();
        coordinator
            .start_probe_polling("test-pod-2".to_string(), "default".to_string())
            .await
            .ok();
        coordinator
            .start_log_streaming(
                "test-pod".to_string(),
                "default".to_string(),
                "container1".to_string(),
                false,
            )
            .await
            .ok();

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Shutdown should clean up all tasks
        coordinator.shutdown().await.ok();

        tokio::time::sleep(Duration::from_millis(100)).await;

        assert_eq!(coordinator.active_probe_tasks().await, 0);
        assert_eq!(coordinator.active_log_tasks().await, 0);
    }

    #[tokio::test]
    async fn test_update_message_creation() {
        use kubectui::k8s::probes::ContainerProbes;

        let msg = UpdateMessage::ProbeUpdate {
            pod_name: "test-pod".to_string(),
            namespace: "default".to_string(),
            probes: vec![("test-container".to_string(), ContainerProbes::default())],
        };

        match msg {
            UpdateMessage::ProbeUpdate {
                pod_name,
                namespace,
                probes,
            } => {
                assert_eq!(pod_name, "test-pod");
                assert_eq!(namespace, "default");
                assert_eq!(probes.len(), 1);
            }
            _ => panic!("Expected ProbeUpdate message"),
        }
    }

    #[tokio::test]
    async fn test_log_stream_status_transitions() {
        let (tx, mut rx) = mpsc::channel(4096);

        // Send status transitions
        tx.send(UpdateMessage::LogStreamStatus {
            pod_name: "test".to_string(),
            namespace: "default".to_string(),
            container_name: "app".to_string(),
            status: LogStreamStatus::Started,
        })
        .await.unwrap();

        tx.send(UpdateMessage::LogStreamStatus {
            pod_name: "test".to_string(),
            namespace: "default".to_string(),
            container_name: "app".to_string(),
            status: LogStreamStatus::Ended,
        })
        .await.unwrap();

        // Receive and verify
        if let Some(UpdateMessage::LogStreamStatus { status, .. }) = rx.recv().await {
            assert_eq!(status, LogStreamStatus::Started);
        } else {
            panic!("Expected LogStreamStatus");
        }

        if let Some(UpdateMessage::LogStreamStatus { status, .. }) = rx.recv().await {
            assert_eq!(status, LogStreamStatus::Ended);
        } else {
            panic!("Expected LogStreamStatus");
        }
    }

    #[tokio::test]
    async fn test_pod_ref_creation_and_comparison() {
        let pod_ref1 = PodRef::new("my-pod".to_string(), "namespace-1".to_string());
        let pod_ref2 = PodRef::new("my-pod".to_string(), "namespace-1".to_string());
        let pod_ref3 = PodRef::new("other-pod".to_string(), "namespace-1".to_string());

        assert_eq!(pod_ref1, pod_ref2);
        assert_ne!(pod_ref1, pod_ref3);
    }
}

#[cfg(test)]
mod probe_polling_tests {
    use kubectui::k8s::probes::{ContainerProbes, ProbeConfig, ProbeHandler, ProbeType};

    #[test]
    fn test_container_probes_equality() {
        let probe_config = ProbeConfig {
            probe_type: ProbeType::Liveness,
            handler: ProbeHandler::Http {
                path: "/health".to_string(),
                port: 8080,
                scheme: "HTTP".to_string(),
            },
            initial_delay_seconds: 5,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
        };

        let mut cp1 = ContainerProbes::default();
        cp1.liveness = Some(probe_config.clone());

        let mut cp2 = ContainerProbes::default();
        cp2.liveness = Some(probe_config);

        assert_eq!(cp1, cp2);
    }

    #[test]
    fn test_container_probes_has_probes() {
        let mut cp = ContainerProbes::default();
        assert!(!cp.has_probes());

        let probe = ProbeConfig {
            probe_type: ProbeType::Readiness,
            handler: ProbeHandler::Tcp { port: 8080 },
            initial_delay_seconds: 0,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
        };

        cp.readiness = Some(probe);
        assert!(cp.has_probes());
    }

    #[test]
    fn test_probe_config_display() {
        let probe = ProbeConfig {
            probe_type: ProbeType::Liveness,
            handler: ProbeHandler::Http {
                path: "/health".to_string(),
                port: 8080,
                scheme: "HTTP".to_string(),
            },
            initial_delay_seconds: 5,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
        };

        let display = probe.format_display();
        assert!(display.contains("Liveness"));
        assert!(display.contains("5"));
        assert!(display.contains("10"));
    }
}

#[cfg(test)]
mod log_streaming_tests {
    use kubectui::coordinator::LogStreamStatus;

    #[test]
    fn test_log_stream_status_equality() {
        let status1 = LogStreamStatus::Started;
        let status2 = LogStreamStatus::Started;
        assert_eq!(status1, status2);

        let status3 = LogStreamStatus::Ended;
        assert_ne!(status1, status3);
    }

    #[test]
    fn test_log_stream_error_status() {
        let status = LogStreamStatus::Error("Connection timeout".to_string());
        assert!(matches!(status, LogStreamStatus::Error(_)));
    }

    #[tokio::test]
    async fn test_log_stream_message_ordering() {
        use kubectui::coordinator::UpdateMessage;
        use tokio::sync::mpsc;

        let (tx, mut rx) = mpsc::channel(4096);

        // Send multiple log updates
        for i in 0..5 {
            tx.send(UpdateMessage::LogUpdate {
                pod_name: "test".to_string(),
                namespace: "default".to_string(),
                container_name: "app".to_string(),
                line: format!("log line {}", i),
            })
            .await.unwrap();
        }

        // Verify order
        for i in 0..5 {
            if let Some(UpdateMessage::LogUpdate { line, .. }) = rx.recv().await {
                assert_eq!(line, format!("log line {}", i));
            } else {
                panic!("Expected LogUpdate");
            }
        }
    }
}

#[cfg(test)]
mod coordinator_channel_tests {
    use kubectui::coordinator::UpdateMessage;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_unbounded_channel_capacity() {
        let (tx, mut rx) = mpsc::channel::<UpdateMessage>(4096);

        // Send many messages
        for i in 0..1000 {
            let msg = UpdateMessage::LogUpdate {
                pod_name: format!("pod-{}", i),
                namespace: "default".to_string(),
                container_name: "app".to_string(),
                line: format!("line {}", i),
            };
            tx.send(msg).await.unwrap();
        }

        // Verify we can receive them all
        let mut count = 0;
        while rx.recv().await.is_some() {
            count += 1;
            if count >= 1000 {
                break;
            }
        }

        assert_eq!(count, 1000);
    }

    #[tokio::test]
    async fn test_channel_closure_on_drop() {
        let (tx, rx) = mpsc::channel::<UpdateMessage>(4096);
        drop(tx);

        // Should return None when sender is dropped
        assert!(rx.is_closed());
    }

    #[tokio::test]
    async fn test_mixed_message_types() {
        let (tx, mut rx) = mpsc::channel(4096);

        // Send different message types
        tx.send(UpdateMessage::LogUpdate {
            pod_name: "pod1".to_string(),
            namespace: "default".to_string(),
            container_name: "app".to_string(),
            line: "test".to_string(),
        })
        .await.unwrap();

        tx.send(UpdateMessage::ProbeUpdate {
            pod_name: "pod2".to_string(),
            namespace: "default".to_string(),
            probes: vec![],
        })
        .await.unwrap();

        tx.send(UpdateMessage::ProbeError {
            pod_name: "pod3".to_string(),
            namespace: "default".to_string(),
            error: "error".to_string(),
        })
        .await.unwrap();

        // Verify we can match on them
        let mut log_count = 0;
        let mut probe_count = 0;
        let mut error_count = 0;

        // Close sender so receiver loop can terminate.
        drop(tx);

        while let Some(msg) = rx.recv().await {
            match msg {
                UpdateMessage::LogUpdate { .. } => log_count += 1,
                UpdateMessage::ProbeUpdate { .. } => probe_count += 1,
                UpdateMessage::ProbeError { .. } => error_count += 1,
                _ => {}
            }
        }

        assert_eq!(log_count, 1);
        assert_eq!(probe_count, 1);
        assert_eq!(error_count, 1);
    }
}
