//! Shared DTOs exchanged between the Kubernetes client, state, and UI layers.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Lightweight node view used by state management and rendering.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeInfo {
    pub name: String,
    pub ready: bool,
    pub kubelet_version: String,
    pub os_image: String,
    pub role: String,
    pub cpu_allocatable: Option<String>,
    pub memory_allocatable: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub memory_pressure: bool,
    pub disk_pressure: bool,
    pub pid_pressure: bool,
    pub network_unavailable: bool,
}

/// Lightweight pod view used by state management and rendering.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PodInfo {
    pub name: String,
    pub namespace: String,
    pub status: String,
    pub node: Option<String>,
    pub pod_ip: Option<String>,
    pub restarts: i32,
    pub created_at: Option<DateTime<Utc>>,
    pub labels: Vec<(String, String)>,
    pub waiting_reasons: Vec<String>,
}

/// Lightweight service view used by list and detail pages.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServiceInfo {
    pub name: String,
    pub namespace: String,
    pub service_type: String,
    pub type_: String,
    pub cluster_ip: Option<String>,
    pub ports: Vec<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub age: Option<Duration>,
}

/// Lightweight deployment view used by list and detail pages.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeploymentInfo {
    pub name: String,
    pub namespace: String,
    pub desired_replicas: i32,
    pub ready_replicas: i32,
    pub available_replicas: i32,
    pub updated_replicas: i32,
    pub created_at: Option<DateTime<Utc>>,
    pub ready: String,
    pub updated: i32,
    pub available: i32,
    pub age: Option<Duration>,
    pub image: Option<String>,
}

/// Cluster metadata shown in dashboard/context widgets.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClusterInfo {
    pub context: Option<String>,
    pub server: String,
    pub git_version: Option<String>,
    pub platform: Option<String>,
    pub node_count: usize,
    pub ready_nodes: usize,
    pub pod_count: usize,
}

/// Dashboard alert severity used for color coding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertSeverity {
    /// Critical cluster issue.
    Error,
    /// Warning state requiring attention.
    Warning,
    /// Informational healthy state.
    Info,
}

/// Dashboard alert item rendered in the top-alerts panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertItem {
    pub severity: AlertSeverity,
    pub title: String,
    pub message: String,
}
