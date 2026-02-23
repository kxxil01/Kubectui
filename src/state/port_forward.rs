//! Active tunnel registry with lifecycle management

use std::collections::HashMap;

use crate::k8s::portforward::{PortForwardTunnelInfo, TunnelState};

/// Registry of active port forward tunnels
#[derive(Debug, Default, Clone)]
pub struct TunnelRegistry {
    /// Active tunnels by ID
    tunnels: HashMap<String, PortForwardTunnelInfo>,
    /// Ordered list of tunnel IDs (for consistent ordering)
    tunnel_ids: Vec<String>,
    /// Selected tunnel index (for UI)
    selected_index: usize,
}

impl TunnelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update tunnels from service
    pub fn update_tunnels(&mut self, tunnels: Vec<PortForwardTunnelInfo>) {
        self.tunnels.clear();
        self.tunnel_ids.clear();
        for tunnel in tunnels {
            self.tunnel_ids.push(tunnel.id.clone());
            self.tunnels.insert(tunnel.id.clone(), tunnel);
        }
    }

    /// Add a tunnel
    pub fn add_tunnel(&mut self, tunnel: PortForwardTunnelInfo) {
        self.tunnel_ids.push(tunnel.id.clone());
        self.tunnels.insert(tunnel.id.clone(), tunnel);
    }

    /// Remove a tunnel
    pub fn remove_tunnel(&mut self, tunnel_id: &str) {
        self.tunnels.remove(tunnel_id);
        self.tunnel_ids.retain(|id| id != tunnel_id);
    }

    /// Get selected tunnel
    pub fn selected(&self) -> Option<&PortForwardTunnelInfo> {
        if self.tunnel_ids.is_empty() {
            return None;
        }
        let id = self.tunnel_ids.get(self.selected_index)?;
        self.tunnels.get(id)
    }

    /// Navigation
    pub fn select_next(&mut self) {
        if !self.tunnel_ids.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.tunnel_ids.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.tunnel_ids.is_empty() {
            self.selected_index = self.selected_index.saturating_sub(1);
        }
    }

    /// Getters
    pub fn tunnels(&self) -> &HashMap<String, PortForwardTunnelInfo> {
        &self.tunnels
    }

    pub fn active_count(&self) -> usize {
        self.tunnels
            .values()
            .filter(|t| t.state == TunnelState::Active)
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.tunnels.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tunnels.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::portforward::{PortForwardTarget, TunnelState};
    use std::net::SocketAddr;
    use std::str::FromStr;

    fn create_test_tunnel(id: &str) -> PortForwardTunnelInfo {
        PortForwardTunnelInfo {
            id: id.to_string(),
            target: PortForwardTarget::new("default", "test-pod", 8080),
            local_addr: SocketAddr::from_str("127.0.0.1:8080").unwrap(),
            state: TunnelState::Active,
        }
    }

    #[test]
    fn test_add_and_remove_tunnels() {
        let mut registry = TunnelRegistry::new();
        let tunnel = create_test_tunnel("test-1");

        registry.add_tunnel(tunnel.clone());
        assert_eq!(registry.len(), 1);

        registry.remove_tunnel("test-1");
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_update_tunnels() {
        let mut registry = TunnelRegistry::new();
        let tunnels = vec![
            create_test_tunnel("test-1"),
            create_test_tunnel("test-2"),
        ];

        registry.update_tunnels(tunnels);
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn test_select_navigation() {
        let mut registry = TunnelRegistry::new();
        registry.add_tunnel(create_test_tunnel("test-1"));
        registry.add_tunnel(create_test_tunnel("test-2"));

        registry.select_next();
        let selected = registry.selected().unwrap();
        assert_eq!(selected.id, "test-2");

        registry.select_prev();
        let selected = registry.selected().unwrap();
        assert_eq!(selected.id, "test-1");
    }

    #[test]
    fn test_active_count() {
        let mut registry = TunnelRegistry::new();
        registry.add_tunnel(create_test_tunnel("test-1"));
        registry.add_tunnel(create_test_tunnel("test-2"));

        assert_eq!(registry.active_count(), 2);
    }
}
