//! Port forwarding integration tests

use std::net::SocketAddr;
use std::str::FromStr;

#[tokio::test]
async fn test_port_forward_target_id() {
    let target = kubectui::k8s::portforward::PortForwardTarget::new("default", "test-pod", 8080);
    assert_eq!(target.id(), "default/test-pod/8080");
}

#[tokio::test]
async fn test_tunnel_info_creation() {
    let target = kubectui::k8s::portforward::PortForwardTarget::new("default", "test-pod", 8080);
    let local_addr = SocketAddr::from_str("127.0.0.1:8080").unwrap();

    let tunnel = kubectui::k8s::portforward::PortForwardTunnelInfo {
        id: target.id(),
        target: target.clone(),
        local_addr,
        state: kubectui::k8s::portforward::TunnelState::Active,
    };

    assert_eq!(
        tunnel.state,
        kubectui::k8s::portforward::TunnelState::Active
    );
    assert_eq!(tunnel.target.namespace, "default");
}

#[tokio::test]
async fn test_port_forward_config_defaults() {
    let config = kubectui::k8s::portforward::PortForwardConfig::default();
    assert_eq!(config.local_port, 0); // Auto-assign
    assert_eq!(config.bind_address, "127.0.0.1");
    assert_eq!(config.timeout_secs, 30);
}

#[tokio::test]
async fn test_port_in_use_error_message() {
    use kubectui::k8s::portforward_errors::PortForwardError;

    let err = PortForwardError::PortInUse {
        port: 8080,
        process_name: Some("nginx".to_string()),
    };

    let msg = err.to_user_message();
    assert!(msg.contains("8080"));
    assert!(msg.contains("nginx"));
}

#[tokio::test]
async fn test_pod_not_found_error_message() {
    use kubectui::k8s::portforward_errors::PortForwardError;

    let err = PortForwardError::PodNotFound {
        namespace: "default".to_string(),
        pod_name: "missing-pod".to_string(),
    };

    let msg = err.to_user_message();
    assert!(msg.contains("missing-pod"));
    assert!(msg.contains("default"));
}

#[tokio::test]
async fn test_port_not_exposed_error_with_available_ports() {
    use kubectui::k8s::portforward_errors::PortForwardError;

    let err = PortForwardError::PortNotExposed {
        pod_name: "test-pod".to_string(),
        port: 8080,
        available_ports: vec![3000, 5000],
    };

    let msg = err.to_user_message();
    assert!(msg.contains("8080"));
    assert!(msg.contains("test-pod"));
    assert!(msg.contains("3000"));
    assert!(msg.contains("5000"));
}

#[tokio::test]
async fn test_connection_failed_retryable() {
    use kubectui::k8s::portforward_errors::PortForwardError;

    let err = PortForwardError::ConnectionFailed {
        pod_name: "test-pod".to_string(),
        retryable: true,
        message: "temporary network error".to_string(),
    };

    assert!(err.is_retryable());
}

#[tokio::test]
async fn test_timeout_error_retryable() {
    use kubectui::k8s::portforward_errors::PortForwardError;

    let err = PortForwardError::Timeout {
        operation: "create port-forward".to_string(),
        duration_ms: 5000,
    };

    assert!(err.is_retryable());
}

#[tokio::test]
async fn test_error_severity() {
    use kubectui::k8s::portforward_errors::{ErrorSeverity, PortForwardError};

    let pod_not_found = PortForwardError::PodNotFound {
        namespace: "default".to_string(),
        pod_name: "missing".to_string(),
    };
    assert_eq!(pod_not_found.severity(), ErrorSeverity::Error);

    let port_in_use = PortForwardError::PortInUse {
        port: 8080,
        process_name: None,
    };
    assert_eq!(port_in_use.severity(), ErrorSeverity::Warning);
}

#[tokio::test]
async fn test_error_suggested_action() {
    use kubectui::k8s::portforward_errors::PortForwardError;

    let port_in_use = PortForwardError::PortInUse {
        port: 80,
        process_name: None,
    };
    let action = port_in_use.suggested_action();
    assert!(action.is_some());
    assert!(action.unwrap().contains("port 0"));
}

#[tokio::test]
async fn test_tunnel_registry_add_and_remove() {
    use kubectui::k8s::portforward::{PortForwardTarget, PortForwardTunnelInfo, TunnelState};
    use kubectui::state::port_forward::TunnelRegistry;
    use std::str::FromStr;

    let mut registry = TunnelRegistry::new();

    let target = PortForwardTarget::new("default", "test-pod", 8080);
    let tunnel = PortForwardTunnelInfo {
        id: target.id(),
        target: target.clone(),
        local_addr: SocketAddr::from_str("127.0.0.1:8080").unwrap(),
        state: TunnelState::Active,
    };

    registry.add_tunnel(tunnel.clone());
    assert_eq!(registry.len(), 1);
    assert_eq!(registry.active_count(), 1);

    registry.remove_tunnel(&tunnel.id);
    assert_eq!(registry.len(), 0);
    assert_eq!(registry.active_count(), 0);
}

#[tokio::test]
async fn test_tunnel_registry_navigation() {
    use kubectui::k8s::portforward::{PortForwardTarget, PortForwardTunnelInfo, TunnelState};
    use kubectui::state::port_forward::TunnelRegistry;
    use std::str::FromStr;

    let mut registry = TunnelRegistry::new();

    // Add multiple tunnels
    for i in 1..=3 {
        let target = PortForwardTarget::new("default", &format!("pod-{}", i), 8000 + i as u16);
        let tunnel = PortForwardTunnelInfo {
            id: target.id(),
            target: target.clone(),
            local_addr: SocketAddr::from_str(&format!("127.0.0.1:{}", 8000 + i as u16)).unwrap(),
            state: TunnelState::Active,
        };
        registry.add_tunnel(tunnel);
    }

    assert_eq!(registry.len(), 3);

    // Test navigation
    let first = registry.selected().unwrap().clone();
    assert_eq!(first.target.pod_name, "pod-1");

    registry.select_next();
    let second = registry.selected().unwrap().clone();
    assert_eq!(second.target.pod_name, "pod-2");

    registry.select_next();
    let third = registry.selected().unwrap().clone();
    assert_eq!(third.target.pod_name, "pod-3");

    registry.select_prev();
    let back_to_second = registry.selected().unwrap().clone();
    assert_eq!(back_to_second.target.pod_name, "pod-2");
}

#[tokio::test]
async fn test_tunnel_registry_is_empty() {
    use kubectui::state::port_forward::TunnelRegistry;

    let registry = TunnelRegistry::new();
    assert!(registry.is_empty());
}

#[tokio::test]
async fn test_tunnel_registry_update_tunnels() {
    use kubectui::k8s::portforward::{PortForwardTarget, PortForwardTunnelInfo, TunnelState};
    use kubectui::state::port_forward::TunnelRegistry;
    use std::str::FromStr;

    let mut registry = TunnelRegistry::new();

    let tunnels = vec![
        {
            let target = PortForwardTarget::new("default", "pod-1", 8080);
            PortForwardTunnelInfo {
                id: target.id(),
                target: target.clone(),
                local_addr: SocketAddr::from_str("127.0.0.1:8080").unwrap(),
                state: TunnelState::Active,
            }
        },
        {
            let target = PortForwardTarget::new("default", "pod-2", 8081);
            PortForwardTunnelInfo {
                id: target.id(),
                target: target.clone(),
                local_addr: SocketAddr::from_str("127.0.0.1:8081").unwrap(),
                state: TunnelState::Active,
            }
        },
    ];

    registry.update_tunnels(tunnels);
    assert_eq!(registry.len(), 2);
}
