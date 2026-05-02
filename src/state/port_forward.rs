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
        let selected_id = self.selected().map(|tunnel| tunnel.id.clone());
        self.tunnels.clear();
        for tunnel in tunnels {
            self.tunnels.insert(tunnel.id.clone(), tunnel);
        }
        self.rebuild_order();
        self.selected_index = selected_id
            .and_then(|id| {
                self.tunnel_ids
                    .iter()
                    .position(|candidate| candidate == &id)
            })
            .unwrap_or(self.selected_index);
        self.clamp_selected_index();
    }

    /// Add a tunnel
    pub fn add_tunnel(&mut self, tunnel: PortForwardTunnelInfo) {
        self.tunnels.insert(tunnel.id.clone(), tunnel);
        self.rebuild_order();
        self.clamp_selected_index();
    }

    /// Remove a tunnel
    pub fn remove_tunnel(&mut self, tunnel_id: &str) {
        self.tunnels.remove(tunnel_id);
        self.rebuild_order();
        self.clamp_selected_index();
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

    /// Returns tunnels in stable display order.
    pub fn ordered_tunnels(&self) -> Vec<&PortForwardTunnelInfo> {
        self.tunnel_ids
            .iter()
            .filter_map(|id| self.tunnels.get(id))
            .collect()
    }

    pub fn ordered_tunnels_matching(&self, search: &str) -> Vec<&PortForwardTunnelInfo> {
        self.ordered_tunnels()
            .into_iter()
            .filter(|tunnel| tunnel_matches_search(tunnel, search))
            .collect()
    }

    pub fn update_tunnels_preserving_filtered_selection(
        &mut self,
        tunnels: Vec<PortForwardTunnelInfo>,
        selected_index: usize,
        search: &str,
    ) -> usize {
        let selected_id = self
            .ordered_tunnels_matching(search)
            .get(selected_index)
            .map(|tunnel| tunnel.id.clone());
        self.update_tunnels(tunnels);
        selected_id
            .and_then(|id| {
                self.ordered_tunnels_matching(search)
                    .iter()
                    .position(|tunnel| tunnel.id == id)
            })
            .unwrap_or_else(|| {
                selected_index.min(
                    self.ordered_tunnels_matching(search)
                        .len()
                        .saturating_sub(1),
                )
            })
    }

    /// Returns the selected index.
    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    fn clamp_selected_index(&mut self) {
        if self.tunnel_ids.is_empty() {
            self.selected_index = 0;
        } else if self.selected_index >= self.tunnel_ids.len() {
            self.selected_index = self.tunnel_ids.len() - 1;
        }
    }

    fn rebuild_order(&mut self) {
        let mut ordered = self.tunnels.values().collect::<Vec<_>>();
        ordered.sort_by(|left, right| {
            (
                left.target.namespace.as_str(),
                left.target.pod_name.as_str(),
                left.local_addr.port(),
                left.target.remote_port,
                left.id.as_str(),
            )
                .cmp(&(
                    right.target.namespace.as_str(),
                    right.target.pod_name.as_str(),
                    right.local_addr.port(),
                    right.target.remote_port,
                    right.id.as_str(),
                ))
        });
        self.tunnel_ids = ordered
            .into_iter()
            .map(|tunnel| tunnel.id.clone())
            .collect();
    }
}

fn tunnel_matches_search(tunnel: &PortForwardTunnelInfo, search: &str) -> bool {
    search.is_empty()
        || contains_ascii_case_insensitive(&tunnel.target.pod_name, search)
        || contains_ascii_case_insensitive(&tunnel.target.namespace, search)
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
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
        let tunnels = vec![create_test_tunnel("test-1"), create_test_tunnel("test-2")];

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

    #[test]
    fn selected_index_is_clamped_after_removal() {
        let mut registry = TunnelRegistry::new();
        registry.add_tunnel(create_test_tunnel("test-1"));
        registry.add_tunnel(create_test_tunnel("test-2"));

        registry.select_next();
        assert_eq!(registry.selected().map(|t| t.id.as_str()), Some("test-2"));

        registry.remove_tunnel("test-2");
        assert_eq!(registry.selected().map(|t| t.id.as_str()), Some("test-1"));
        assert_eq!(registry.selected_index(), 0);
    }

    #[test]
    fn ordered_tunnels_sort_deterministically() {
        let mut registry = TunnelRegistry::new();
        registry.add_tunnel(PortForwardTunnelInfo {
            id: "b".to_string(),
            target: PortForwardTarget::new("ops", "pod-b", 8081),
            local_addr: SocketAddr::from_str("127.0.0.1:8081").unwrap(),
            state: TunnelState::Active,
        });
        registry.add_tunnel(PortForwardTunnelInfo {
            id: "a".to_string(),
            target: PortForwardTarget::new("default", "pod-a", 8080),
            local_addr: SocketAddr::from_str("127.0.0.1:8080").unwrap(),
            state: TunnelState::Active,
        });

        let ordered = registry
            .ordered_tunnels()
            .into_iter()
            .map(|tunnel| tunnel.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ordered, vec!["a", "b"]);
    }

    #[test]
    fn update_tunnels_preserves_selected_tunnel_identity() {
        let mut registry = TunnelRegistry::new();
        registry.update_tunnels(vec![
            PortForwardTunnelInfo {
                id: "alpha".to_string(),
                target: PortForwardTarget::new("team-b", "api", 8080),
                local_addr: SocketAddr::from_str("127.0.0.1:9000").unwrap(),
                state: TunnelState::Active,
            },
            PortForwardTunnelInfo {
                id: "beta".to_string(),
                target: PortForwardTarget::new("team-a", "api", 8080),
                local_addr: SocketAddr::from_str("127.0.0.1:9001").unwrap(),
                state: TunnelState::Active,
            },
        ]);
        registry.select_next();
        assert_eq!(
            registry.selected().map(|tunnel| tunnel.id.as_str()),
            Some("alpha")
        );

        registry.update_tunnels(vec![
            PortForwardTunnelInfo {
                id: "alpha".to_string(),
                target: PortForwardTarget::new("team-a", "api", 8080),
                local_addr: SocketAddr::from_str("127.0.0.1:9000").unwrap(),
                state: TunnelState::Active,
            },
            PortForwardTunnelInfo {
                id: "beta".to_string(),
                target: PortForwardTarget::new("team-b", "api", 8080),
                local_addr: SocketAddr::from_str("127.0.0.1:9001").unwrap(),
                state: TunnelState::Active,
            },
        ]);

        assert_eq!(
            registry.selected().map(|tunnel| tunnel.id.as_str()),
            Some("alpha")
        );
    }

    #[test]
    fn update_tunnels_preserves_filtered_selected_tunnel_identity() {
        let mut registry = TunnelRegistry::new();
        registry.update_tunnels(vec![
            PortForwardTunnelInfo {
                id: "target".to_string(),
                target: PortForwardTarget::new("team-b", "api", 8080),
                local_addr: SocketAddr::from_str("127.0.0.1:9000").unwrap(),
                state: TunnelState::Active,
            },
            PortForwardTunnelInfo {
                id: "other".to_string(),
                target: PortForwardTarget::new("team-b", "worker", 8080),
                local_addr: SocketAddr::from_str("127.0.0.1:9001").unwrap(),
                state: TunnelState::Active,
            },
        ]);

        let selected = registry.update_tunnels_preserving_filtered_selection(
            vec![
                PortForwardTunnelInfo {
                    id: "inserted-before".to_string(),
                    target: PortForwardTarget::new("team-a", "api", 8080),
                    local_addr: SocketAddr::from_str("127.0.0.1:8999").unwrap(),
                    state: TunnelState::Active,
                },
                PortForwardTunnelInfo {
                    id: "target".to_string(),
                    target: PortForwardTarget::new("team-b", "api", 8080),
                    local_addr: SocketAddr::from_str("127.0.0.1:9000").unwrap(),
                    state: TunnelState::Active,
                },
                PortForwardTunnelInfo {
                    id: "other".to_string(),
                    target: PortForwardTarget::new("team-b", "worker", 8080),
                    local_addr: SocketAddr::from_str("127.0.0.1:9001").unwrap(),
                    state: TunnelState::Active,
                },
            ],
            0,
            "api",
        );

        let visible = registry.ordered_tunnels_matching("api");
        assert_eq!(visible[selected].id, "target");
    }

    #[test]
    fn update_tunnels_clamps_filtered_selection_when_selected_tunnel_disappears() {
        let mut registry = TunnelRegistry::new();
        registry.update_tunnels(vec![
            PortForwardTunnelInfo {
                id: "alpha".to_string(),
                target: PortForwardTarget::new("team-a", "api", 8080),
                local_addr: SocketAddr::from_str("127.0.0.1:9000").unwrap(),
                state: TunnelState::Active,
            },
            PortForwardTunnelInfo {
                id: "beta".to_string(),
                target: PortForwardTarget::new("team-b", "api", 8080),
                local_addr: SocketAddr::from_str("127.0.0.1:9001").unwrap(),
                state: TunnelState::Active,
            },
        ]);

        let selected = registry.update_tunnels_preserving_filtered_selection(
            vec![PortForwardTunnelInfo {
                id: "alpha".to_string(),
                target: PortForwardTarget::new("team-a", "api", 8080),
                local_addr: SocketAddr::from_str("127.0.0.1:9000").unwrap(),
                state: TunnelState::Active,
            }],
            1,
            "api",
        );

        assert_eq!(selected, 0);
        assert_eq!(
            registry.ordered_tunnels_matching("api")[selected].id,
            "alpha"
        );
    }
}
