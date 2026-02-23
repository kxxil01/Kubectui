//! Shared DTOs exchanged between the Kubernetes client, state, and UI layers.

use std::{collections::BTreeMap, time::Duration};

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
    pub selector: String,
    pub update_strategy: String,
    pub labels: BTreeMap<String, String>,
    pub status_message: String,
    pub image: Option<String>,
    pub age: Option<Duration>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Legacy lightweight snapshot used by daemonset integration tests.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClusterSnapshot {
    pub daemonsets: Vec<DaemonSetInfo>,
}

/// Shared RBAC policy rule payload used by Role and ClusterRole DTOs.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RbacRule {
    pub verbs: Vec<String>,
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub resource_names: Vec<String>,
    pub non_resource_urls: Vec<String>,
}

/// Lightweight ServiceAccount view used by list and detail pages.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServiceAccountInfo {
    pub name: String,
    pub namespace: String,
    pub secrets_count: usize,
    pub image_pull_secrets_count: usize,
    pub automount_service_account_token: Option<bool>,
    pub age: Option<Duration>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Lightweight Role view used by list and detail pages.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoleInfo {
    pub name: String,
    pub namespace: String,
    pub rules: Vec<RbacRule>,
    pub age: Option<Duration>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Subject entry used by RoleBinding and ClusterRoleBinding DTOs.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RoleBindingSubject {
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
    pub api_group: Option<String>,
}

/// Lightweight RoleBinding view used by list and detail pages.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoleBindingInfo {
    pub name: String,
    pub namespace: String,
    pub role_ref_kind: String,
    pub role_ref_name: String,
    pub subjects: Vec<RoleBindingSubject>,
    pub age: Option<Duration>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Lightweight ClusterRole view used by list and detail pages.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClusterRoleInfo {
    pub name: String,
    pub rules: Vec<RbacRule>,
    pub age: Option<Duration>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Lightweight ClusterRoleBinding view used by list and detail pages.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClusterRoleBindingInfo {
    pub name: String,
    pub role_ref_kind: String,
    pub role_ref_name: String,
    pub subjects: Vec<RoleBindingSubject>,
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

/// Lightweight ResourceQuota view used by governance lists and detail sections.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourceQuotaInfo {
    pub name: String,
    pub namespace: String,
    pub hard: BTreeMap<String, String>,
    pub used: BTreeMap<String, String>,
    pub percent_used: BTreeMap<String, f64>,
    pub age: Option<Duration>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Lightweight LimitRange view used by governance lists and detail sections.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LimitRangeInfo {
    pub name: String,
    pub namespace: String,
    pub limits: Vec<LimitSpec>,
    pub age: Option<Duration>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Flattened limit item from LimitRange spec.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LimitSpec {
    pub type_: String,
    pub min: BTreeMap<String, String>,
    pub max: BTreeMap<String, String>,
    pub default: BTreeMap<String, String>,
    pub default_request: BTreeMap<String, String>,
    pub max_limit_request_ratio: BTreeMap<String, String>,
}

/// Lightweight PodDisruptionBudget view used by governance lists/detail pages.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PodDisruptionBudgetInfo {
    pub name: String,
    pub namespace: String,
    pub min_available: Option<String>,
    pub max_unavailable: Option<String>,
    pub current_healthy: i32,
    pub desired_healthy: i32,
    pub disruptions_allowed: i32,
    pub expected_pods: i32,
    pub age: Option<Duration>,
    pub created_at: Option<DateTime<Utc>>,
}

/// CustomResourceDefinition metadata for extension browsing.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomResourceDefinitionInfo {
    pub name: String,
    pub group: String,
    pub version: String,
    pub kind: String,
    pub plural: String,
    pub scope: String,
    pub instances: usize,
}

/// Lightweight custom resource instance view.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomResourceInfo {
    pub name: String,
    pub namespace: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub age: Option<Duration>,
}

/// Container-level usage inside PodMetrics.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ContainerMetrics {
    pub name: String,
    pub cpu: String,
    pub memory: String,
}

/// Pod usage metrics from metrics.k8s.io.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PodMetricsInfo {
    pub name: String,
    pub namespace: String,
    pub timestamp: Option<String>,
    pub window: Option<String>,
    pub containers: Vec<ContainerMetrics>,
}

/// Node usage metrics from metrics.k8s.io.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NodeMetricsInfo {
    pub name: String,
    pub timestamp: Option<String>,
    pub window: Option<String>,
    pub cpu: String,
    pub memory: String,
}

impl PodMetricsInfo {
    /// Parses a PodMetrics payload from dynamic JSON data.
    pub fn from_json(name: String, namespace: String, value: &serde_json::Value) -> Option<Self> {
        let containers = value
            .get("containers")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|c| {
                        Some(ContainerMetrics {
                            name: c.get("name")?.as_str()?.to_string(),
                            cpu: c
                                .get("usage")?
                                .get("cpu")?
                                .as_str()
                                .unwrap_or("unknown")
                                .to_string(),
                            memory: c
                                .get("usage")?
                                .get("memory")?
                                .as_str()
                                .unwrap_or("unknown")
                                .to_string(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Some(Self {
            name,
            namespace,
            timestamp: value
                .get("timestamp")
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            window: value
                .get("window")
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            containers,
        })
    }
}

impl NodeMetricsInfo {
    /// Parses a NodeMetrics payload from dynamic JSON data.
    pub fn from_json(name: String, value: &serde_json::Value) -> Option<Self> {
        let usage = value.get("usage")?;
        Some(Self {
            name,
            timestamp: value
                .get("timestamp")
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            window: value
                .get("window")
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            cpu: usage
                .get("cpu")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            memory: usage
                .get("memory")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
        })
    }
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{NodeMetricsInfo, PodMetricsInfo};

    #[test]
    fn pod_metrics_parsing_extracts_container_usage() {
        let payload = json!({
            "timestamp": "2026-02-23T10:10:00Z",
            "window": "30s",
            "containers": [
                {"name": "app", "usage": {"cpu": "25m", "memory": "120Mi"}},
                {"name": "sidecar", "usage": {"cpu": "5m", "memory": "40Mi"}}
            ]
        });

        let parsed =
            PodMetricsInfo::from_json("demo-pod".to_string(), "default".to_string(), &payload)
                .expect("valid pod metrics payload");

        assert_eq!(parsed.name, "demo-pod");
        assert_eq!(parsed.namespace, "default");
        assert_eq!(parsed.containers.len(), 2);
        assert_eq!(parsed.containers[0].cpu, "25m");
        assert_eq!(parsed.containers[1].memory, "40Mi");
    }

    #[test]
    fn node_metrics_parsing_reads_usage_fields() {
        let payload = json!({
            "timestamp": "2026-02-23T10:10:00Z",
            "window": "30s",
            "usage": {"cpu": "240m", "memory": "1024Mi"}
        });

        let parsed = NodeMetricsInfo::from_json("worker-1".to_string(), &payload)
            .expect("valid node metrics payload");

        assert_eq!(parsed.name, "worker-1");
        assert_eq!(parsed.cpu, "240m");
        assert_eq!(parsed.memory, "1024Mi");
    }

    #[test]
    fn node_metrics_parsing_requires_usage() {
        let payload = json!({"timestamp": "2026-02-23T10:10:00Z"});
        assert!(NodeMetricsInfo::from_json("worker-1".to_string(), &payload).is_none());
    }
}
