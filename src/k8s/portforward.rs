//! Port forwarding implementation using kube-rs PortForward API

use anyhow::{Context, Result, anyhow};
use dashmap::DashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tracing::{info, instrument};

pub use crate::k8s::portforward_errors::PortForwardError;

/// Port forwarding target
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PortForwardTarget {
    pub namespace: String,
    pub pod_name: String,
    pub remote_port: u16,
}

impl PortForwardTarget {
    pub fn new(
        namespace: impl Into<String>,
        pod_name: impl Into<String>,
        remote_port: u16,
    ) -> Self {
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

/// Handle to a background tunnel task
#[derive(Debug)]
pub struct TunnelHandle {
    pub task: JoinHandle<()>,
    pub info: PortForwardTunnelInfo,
}

/// Port forwarding configuration
#[derive(Debug, Clone, PartialEq, Eq)]
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
    k8s_client: Arc<crate::k8s::client::K8sClient>,
    tunnels: Arc<DashMap<String, PortForwardTunnelInfo>>,
    handles: Arc<DashMap<String, TunnelHandle>>,
}

impl PortForwarderService {
    pub fn new(k8s_client: Arc<crate::k8s::client::K8sClient>) -> Self {
        Self {
            k8s_client,
            tunnels: Arc::new(DashMap::new()),
            handles: Arc::new(DashMap::new()),
        }
    }

    /// Create and start a port forward tunnel asynchronously.
    /// Returns tunnel ID immediately, continues in background.
    #[instrument(skip(self))]
    pub async fn create_tunnel_async(
        &self,
        target: PortForwardTarget,
        config: PortForwardConfig,
    ) -> Result<String, PortForwardError> {
        let tunnel_id = target.id();

        // Check if already exists
        if self.tunnels.contains_key(&tunnel_id) {
            return Err(PortForwardError::ConnectionFailed {
                pod_name: target.pod_name.clone(),
                retryable: false,
                message: format!("Tunnel already exists: {}", tunnel_id),
            });
        }

        // Create tunnel via K8s API (validates pod exists and port is exposed)
        let tunnel_info = self
            .k8s_client
            .create_port_forward(&target, &config)
            .await?;

        // Register tunnel
        self.tunnels.insert(tunnel_id.clone(), tunnel_info.clone());

        // Bind local TCP listener
        let bind_addr = format!("{}:{}", config.bind_address, tunnel_info.local_addr.port());
        let listener = tokio::net::TcpListener::bind(&bind_addr)
            .await
            .map_err(|e| PortForwardError::ConnectionFailed {
                pod_name: target.pod_name.clone(),
                retryable: true,
                message: format!("Failed to bind {bind_addr}: {e}"),
            })?;

        // Spawn background task to maintain tunnel lifecycle
        let _tunnels = Arc::clone(&self.tunnels);
        let id = tunnel_id.clone();
        let local_addr_clone = tunnel_info.local_addr;
        let client_for_task = Arc::clone(&self.k8s_client);
        let pod_name_for_task = target.pod_name.clone();
        let namespace_for_task = target.namespace.clone();
        let remote_port_for_task = target.remote_port;
        let task = tokio::spawn(async move {
            // Real port-forward: accept TCP connections and proxy through kube API
            info!("Tunnel {} accepting connections on {}", id, local_addr_clone);
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let client_clone = Arc::clone(&client_for_task);
                        let pod_name_clone = pod_name_for_task.clone();
                        let namespace_clone = namespace_for_task.clone();
                        let remote_port_clone = remote_port_for_task;
                        tokio::spawn(async move {
                            if let Err(e) = proxy_connection(
                                stream,
                                client_clone,
                                &pod_name_clone,
                                &namespace_clone,
                                remote_port_clone,
                            )
                            .await
                            {
                                tracing::warn!("port-forward proxy error: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!("port-forward accept error: {e}");
                        break;
                    }
                }
            }
        });

        // Store the handle
        self.handles.insert(
            tunnel_id.clone(),
            TunnelHandle {
                task,
                info: tunnel_info,
            },
        );

        Ok(tunnel_id)
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
        // Cancel the background task
        if let Some((_, handle)) = self.handles.remove(tunnel_id) {
            handle.task.abort();
        }

        if self.tunnels.remove(tunnel_id).is_some() {
            info!("Stopped tunnel {}", tunnel_id);
        }
        Ok(())
    }

    pub async fn stop_all(&self) {
        let ids: Vec<String> = self.tunnels.iter().map(|e| e.key().clone()).collect();

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
        self.tunnels
            .get(tunnel_id)
            .map(|entry| entry.value().clone())
    }
}

/// Proxies a single TCP connection through the kube-rs PortForward API.
async fn proxy_connection(
    mut local_stream: tokio::net::TcpStream,
    client: Arc<crate::k8s::client::K8sClient>,
    pod_name: &str,
    namespace: &str,
    remote_port: u16,
) -> anyhow::Result<()> {
    use k8s_openapi::api::core::v1::Pod;
    use kube::Api;

    let pods: Api<Pod> = Api::namespaced(client.get_client(), namespace);
    let mut pf = pods.portforward(pod_name, &[remote_port]).await?;

    let mut port_stream = pf
        .take_stream(remote_port)
        .ok_or_else(|| anyhow!("no stream for port {remote_port}"))?;

    let (mut local_read, mut local_write) = local_stream.split();
    let (mut remote_read, mut remote_write) = tokio::io::split(&mut port_stream);

    let client_to_pod = tokio::io::copy(&mut local_read, &mut remote_write);
    let pod_to_client = tokio::io::copy(&mut remote_read, &mut local_write);

    tokio::select! {
        r = client_to_pod => { r?; }
        r = pod_to_client => { r?; }
    }

    // Gracefully close the port-forward stream
    remote_write.shutdown().await.ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn service() -> PortForwarderService {
        let client = Arc::new(
            crate::k8s::client::K8sClient::connect()
                .await
                .expect("kind cluster should be available for tests"),
        );
        PortForwarderService::new(client)
    }

    #[tokio::test]
    async fn start_forward_binds_ephemeral_port() {
        let svc = service().await;
        let target = PortForwardTarget::new("default", "pod-a", 8080);

        let tunnel = svc
            .start_forward(target.clone(), PortForwardConfig::default())
            .await
            .expect("bind should succeed on random local port");

        assert_eq!(tunnel.target, target);
        assert!(tunnel.local_addr.port() > 0);
        assert_eq!(tunnel.state, TunnelState::Active);
    }

    #[tokio::test]
    async fn start_forward_rejects_duplicate_id() {
        let svc = service().await;
        let target = PortForwardTarget::new("default", "pod-b", 9090);

        svc.start_forward(target.clone(), PortForwardConfig::default())
            .await
            .expect("first tunnel should succeed");

        let err = svc
            .start_forward(target.clone(), PortForwardConfig::default())
            .await
            .expect_err("duplicate should fail");

        assert!(format!("{err:#}").contains("already exists"));
    }

    #[tokio::test]
    async fn get_list_and_stop_tunnel_round_trip() {
        let svc = service().await;
        let target = PortForwardTarget::new("default", "pod-c", 7070);

        let tunnel = svc
            .start_forward(target.clone(), PortForwardConfig::default())
            .await
            .expect("tunnel should start");

        let list = svc.list_tunnels();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, target.id());

        let fetched = svc
            .get_tunnel(&target.id())
            .expect("tunnel should be queryable");
        assert_eq!(fetched.id, tunnel.id);

        svc.stop_forward(&target.id())
            .await
            .expect("stop should succeed");
        assert!(svc.get_tunnel(&target.id()).is_none());
    }

    #[tokio::test]
    async fn stop_all_clears_multiple_tunnels() {
        let svc = service().await;

        for (pod, port) in [("pod-d", 8081), ("pod-e", 8082), ("pod-f", 8083)] {
            svc.start_forward(
                PortForwardTarget::new("default", pod, port),
                PortForwardConfig::default(),
            )
            .await
            .expect("tunnel creation should succeed");
        }

        assert_eq!(svc.list_tunnels().len(), 3);
        svc.stop_all().await;
        assert!(svc.list_tunnels().is_empty());
    }

    #[tokio::test]
    async fn create_tunnel_async_returns_pod_not_found_for_missing_pod() {
        let svc = service().await;
        let target = PortForwardTarget::new("default", "pod-that-does-not-exist", 8080);

        let err = svc
            .create_tunnel_async(target, PortForwardConfig::default())
            .await
            .expect_err("missing pod should fail");

        match err {
            PortForwardError::PodNotFound { .. } => {}
            other => panic!("expected PodNotFound, got {other:?}"),
        }
    }
}
