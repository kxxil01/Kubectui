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

/// Lightweight StatefulSet view used by list and detail pages.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StatefulSetInfo {
    pub name: String,
    pub namespace: String,
    pub desired_replicas: i32,
    pub ready_replicas: i32,
    pub service_name: String,
    pub pod_management_policy: String,
    pub image: Option<String>,
    pub age: Option<Duration>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Lightweight DaemonSet view used by list and detail pages.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaemonSetInfo {
    pub name: String,
    pub namespace: String,
    pub desired_count: i32,
    pub ready_count: i32,
    pub unavailable_count: i32,
    pub image: Option<String>,
    pub age: Option<Duration>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Lightweight Job view used by list and detail pages.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JobInfo {
    pub name: String,
    pub namespace: String,
    pub status: String,
    pub completions: String,
    pub duration: Option<String>,
    pub parallelism: i32,
    pub active_pods: i32,
    pub failed_pods: i32,
    pub age: Option<Duration>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Lightweight CronJob view used by list and detail pages.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CronJobInfo {
    pub name: String,
    pub namespace: String,
    pub schedule: String,
    pub timezone: Option<String>,
    pub last_schedule_time: Option<DateTime<Utc>>,
    pub next_schedule_time: Option<DateTime<Utc>>,
    pub last_successful_time: Option<DateTime<Utc>>,
    pub suspend: bool,
    pub active_jobs: i32,
    pub age: Option<Duration>,
    pub created_at: Option<DateTime<Utc>>,
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
    /// Critical condition (red).
    Error,
    /// Warning condition (yellow).
    Warning,
    /// Informational condition (green).
    Info,
}

/// Dashboard alert item displayed in the alert list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertItem {
    pub severity: AlertSeverity,
    pub title: String,
    pub message: String,
}
