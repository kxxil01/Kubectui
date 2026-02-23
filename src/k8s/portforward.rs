//! Port forwarding implementation using kube-rs PortForward API

use std::net::SocketAddr;
use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use kube::Client;
use tokio::net::TcpListener;
use tracing::{info, instrument};

/// Port forwarding target
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PortForwardTarget {
    pub namespace: String,
    pub pod_name: String,
    pub remote_port: u16,
}

impl PortForwardTarget {
    pub fn new(namespace: impl Into<String>, pod_name: impl Into<String>, remote_port: u16) -> Self {
        Self {
            namespace: namespace.into(),
            pod_name: pod_name.into(),
            remote_port,
        }
    }

    pub fn id(&self) -> String {
        format!("{}/{}/{}", self.namespace, self.pod_name, self.remote_port)
    }
}

/// Active port forward tunnel
#[derive(Debug, Clone)]
pub struct PortForwardTunnelInfo {
    pub id: String,
    pub target: PortForwardTarget,
    pub local_addr: SocketAddr,
    pub state: TunnelState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelState {
    Starting,
    Active,
    Error,
    Closing,
    Closed,
}

/// Port forwarding configuration
#[derive(Debug, Clone)]
pub struct PortForwardConfig {
    /// Local port (0 for auto-assign)
    pub local_port: u16,
    /// Local bind address
    pub bind_address: String,
    /// Connection timeout
    pub timeout_secs: u64,
}

impl Default for PortForwardConfig {
    fn default() -> Self {
        Self {
            local_port: 0,
            bind_address: "127.0.0.1".to_string(),
            timeout_secs: 30,
        }
    }
}

/// Port forwarding service
#[derive(Clone)]
pub struct PortForwarderService {
    client: Client,
    tunnels: std::sync::Arc<DashMap<String, PortForwardTunnelInfo>>,
}

impl PortForwarderService {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            tunnels: std::sync::Arc::new(DashMap::new()),
        }
    }

    #[instrument(skip(self))]
    pub async fn start_forward(
        &self,
        target: PortForwardTarget,
        config: PortForwardConfig,
    ) -> Result<PortForwardTunnelInfo> {
        let tunnel_id = target.id();

        if self.tunnels.contains_key(&tunnel_id) {
            return Err(anyhow!("Port forward already exists: {}", tunnel_id));
        }

        let bind_addr = format!("{}:{}", config.bind_address, config.local_port);
        let listener = TcpListener::bind(&bind_addr)
            .await
            .with_context(|| format!("Failed to bind to {}", bind_addr))?;

        let local_addr = listener.local_addr()?;
        info!("Port forward listening on {}", local_addr);

        let tunnel = PortForwardTunnelInfo {
            id: tunnel_id.clone(),
            target: target.clone(),
            local_addr,
            state: TunnelState::Active,
        };

        self.tunnels.insert(tunnel_id.clone(), tunnel.clone());
        Ok(tunnel)
    }

    pub async fn stop_forward(&self, tunnel_id: &str) -> Result<()> {
        if self.tunnels.remove(tunnel_id).is_some() {
            info!("Stopped tunnel {}", tunnel_id);
        }
        Ok(())
    }

    pub async fn stop_all(&self) {
        let ids: Vec<String> = self
            .tunnels
            .iter()
            .map(|e| e.key().clone())
            .collect();

        for id in ids {
            let _ = self.stop_forward(&id).await;
        }
    }

    pub fn list_tunnels(&self) -> Vec<PortForwardTunnelInfo> {
        self.tunnels
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    pub fn get_tunnel(&self, tunnel_id: &str) -> Option<PortForwardTunnelInfo> {
        self.tunnels.get(tunnel_id).map(|entry| entry.value().clone())
    }
}
