//! Integration tests for port forwarding functionality

#[cfg(test)]
mod portforward_tests {
    use kubectui::k8s::portforward::{PortForwardConfig, PortForwardTarget, TunnelState};
    use kubectui::state::port_forward::TunnelRegistry;
    use std::net::SocketAddr;
    use std::str::FromStr;

    fn create_test_tunnel(id: &str) -> kubectui::k8s::portforward::PortForwardTunnelInfo {
        kubectui::k8s::portforward::PortForwardTunnelInfo {
            id: id.to_string(),
            target: PortForwardTarget::new("default", "test-pod", 8080),
            local_addr: SocketAddr::from_str("127.0.0.1:8080").unwrap(),
            state: TunnelState::Active,
        }
    }

    #[test]
    fn test_tunnel_target_id() {
        let target = PortForwardTarget::new("default", "my-pod", 8080);
        assert_eq!(target.id(), "default/my-pod/8080");
    }

    #[test]
    fn test_tunnel_target_clone() {
        let target1 = PortForwardTarget::new("default", "pod-1", 8080);
        let target2 = target1.clone();

        assert_eq!(target1, target2);
        assert_eq!(target1.namespace, target2.namespace);
        assert_eq!(target1.pod_name, target2.pod_name);
        assert_eq!(target1.remote_port, target2.remote_port);
    }

    #[test]
    fn test_portforward_config_defaults() {
        let config = PortForwardConfig::default();
        assert_eq!(config.local_port, 0);
        assert_eq!(config.bind_address, "127.0.0.1");
        assert_eq!(config.timeout_secs, 30);
    }

    #[test]
    fn test_portforward_config_custom() {
        let config = PortForwardConfig {
            local_port: 9000,
            bind_address: "0.0.0.0".to_string(),
            timeout_secs: 60,
        };
        assert_eq!(config.local_port, 9000);
        assert_eq!(config.bind_address, "0.0.0.0");
        assert_eq!(config.timeout_secs, 60);
    }

    #[test]
    fn test_tunnel_registry_add_tunnel() {
        let mut registry = TunnelRegistry::new();
        assert_eq!(registry.len(), 0);

        let tunnel = create_test_tunnel("tunnel-1");
        registry.add_tunnel(tunnel.clone());

        assert_eq!(registry.len(), 1);
        assert_eq!(registry.active_count(), 1);
    }

    #[test]
    fn test_tunnel_registry_remove_tunnel() {
        let mut registry = TunnelRegistry::new();
        let tunnel = create_test_tunnel("tunnel-1");
        registry.add_tunnel(tunnel.clone());
        assert_eq!(registry.len(), 1);

        registry.remove_tunnel("tunnel-1");
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_tunnel_registry_update_tunnels() {
        let mut registry = TunnelRegistry::new();
        let tunnel1 = create_test_tunnel("tunnel-1");
        let tunnel2 = create_test_tunnel("tunnel-2");

        registry.update_tunnels(vec![tunnel1, tunnel2]);
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn test_tunnel_registry_multiple_tunnels() {
        let mut registry = TunnelRegistry::new();

        for i in 1..=5 {
            let tunnel = create_test_tunnel(&format!("tunnel-{}", i));
            registry.add_tunnel(tunnel);
        }

        assert_eq!(registry.len(), 5);
        assert_eq!(registry.active_count(), 5);
    }

    #[test]
    fn test_tunnel_registry_navigation() {
        let mut registry = TunnelRegistry::new();
        registry.add_tunnel(create_test_tunnel("tunnel-1"));
        registry.add_tunnel(create_test_tunnel("tunnel-2"));
        registry.add_tunnel(create_test_tunnel("tunnel-3"));

        // Initially at first tunnel
        let selected = registry.selected().unwrap();
        assert_eq!(selected.id, "tunnel-1");

        // Move next
        registry.select_next();
        let selected = registry.selected().unwrap();
        assert_eq!(selected.id, "tunnel-2");

        // Move next again
        registry.select_next();
        let selected = registry.selected().unwrap();
        assert_eq!(selected.id, "tunnel-3");

        // Wrap around
        registry.select_next();
        let selected = registry.selected().unwrap();
        assert_eq!(selected.id, "tunnel-1");
    }

    #[test]
    fn test_tunnel_registry_navigate_prev() {
        let mut registry = TunnelRegistry::new();
        registry.add_tunnel(create_test_tunnel("tunnel-1"));
        registry.add_tunnel(create_test_tunnel("tunnel-2"));

        // Move to second
        registry.select_next();
        let selected = registry.selected().unwrap();
        assert_eq!(selected.id, "tunnel-2");

        // Move prev back to first
        registry.select_prev();
        let selected = registry.selected().unwrap();
        assert_eq!(selected.id, "tunnel-1");

        // Try to move before first (should stay at first)
        registry.select_prev();
        let selected = registry.selected().unwrap();
        assert_eq!(selected.id, "tunnel-1");
    }

    #[test]
    fn test_tunnel_state_variants() {
        let tunnel = create_test_tunnel("test");
        assert_eq!(tunnel.state, TunnelState::Active);

        let starting = TunnelState::Starting;
        let error = TunnelState::Error;
        let closing = TunnelState::Closing;
        let closed = TunnelState::Closed;

        assert_ne!(starting, TunnelState::Active);
        assert_ne!(error, TunnelState::Active);
        assert_ne!(closing, TunnelState::Active);
        assert_ne!(closed, TunnelState::Active);
    }

    #[test]
    fn test_tunnel_registry_is_empty() {
        let mut registry = TunnelRegistry::new();
        assert!(registry.is_empty());

        let tunnel = create_test_tunnel("tunnel-1");
        registry.add_tunnel(tunnel);
        assert!(!registry.is_empty());

        registry.remove_tunnel("tunnel-1");
        assert!(registry.is_empty());
    }

    #[test]
    fn test_tunnel_registry_clear_via_update() {
        let mut registry = TunnelRegistry::new();
        registry.add_tunnel(create_test_tunnel("tunnel-1"));
        registry.add_tunnel(create_test_tunnel("tunnel-2"));
        assert_eq!(registry.len(), 2);

        // Update with empty list clears all
        registry.update_tunnels(vec![]);
        assert_eq!(registry.len(), 0);
    }
}
