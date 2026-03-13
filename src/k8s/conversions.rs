//! Shared DTO conversion functions for Kubernetes API objects.
//!
//! These conversions are used by both the polling path (`client.rs`) and
//! the watch path (`state/watch.rs`) to produce identical typed DTOs.

use chrono::Utc;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Node, Pod, PodSpec, ReplicationController, Service};

use crate::cronjob::cronjob_next_schedule_time;
use crate::k8s::dtos::{
    CronJobInfo, DaemonSetInfo, DeploymentInfo, JobInfo, NodeInfo, OwnerRefInfo, PodInfo,
    ReplicaSetInfo, ReplicationControllerInfo, ServiceInfo, StatefulSetInfo,
};
use crate::state::alerts::{format_mib, format_millicores, parse_mib, parse_millicores};

/// Extracts the first container image from a pod spec.
pub fn extract_image_from_pod_spec(pod_spec: Option<&PodSpec>) -> Option<String> {
    pod_spec
        .and_then(|spec| spec.containers.first())
        .and_then(|container| container.image.clone())
}

/// Converts a raw Kubernetes `Pod` object into a lightweight [`PodInfo`] DTO.
pub fn pod_to_info(pod: Pod) -> PodInfo {
    let container_statuses = pod
        .status
        .as_ref()
        .and_then(|status| status.container_statuses.as_ref())
        .cloned()
        .unwrap_or_default();

    let waiting_reasons = container_statuses
        .iter()
        .filter_map(|status| status.state.as_ref())
        .filter_map(|state| state.waiting.as_ref())
        .filter_map(|waiting| waiting.reason.clone())
        .collect::<Vec<_>>();

    let restarts = container_statuses.iter().map(|s| s.restart_count).sum();

    let containers = pod
        .spec
        .as_ref()
        .map(|spec| spec.containers.as_slice())
        .unwrap_or_default();

    let (cpu_request, memory_request, cpu_limit, memory_limit) = {
        let mut cpu_req_m: u64 = 0;
        let mut mem_req_mib: u64 = 0;
        let mut cpu_lim_m: u64 = 0;
        let mut mem_lim_mib: u64 = 0;
        let mut has_cpu_req = false;
        let mut has_mem_req = false;
        let mut has_cpu_lim = false;
        let mut has_mem_lim = false;
        for c in containers {
            if let Some(req) = c.resources.as_ref().and_then(|r| r.requests.as_ref()) {
                if let Some(cpu) = req.get("cpu") {
                    cpu_req_m += parse_millicores(&cpu.0);
                    has_cpu_req = true;
                }
                if let Some(mem) = req.get("memory") {
                    mem_req_mib += parse_mib(&mem.0);
                    has_mem_req = true;
                }
            }
            if let Some(lim) = c.resources.as_ref().and_then(|r| r.limits.as_ref()) {
                if let Some(cpu) = lim.get("cpu") {
                    cpu_lim_m += parse_millicores(&cpu.0);
                    has_cpu_lim = true;
                }
                if let Some(mem) = lim.get("memory") {
                    mem_lim_mib += parse_mib(&mem.0);
                    has_mem_lim = true;
                }
            }
        }
        (
            has_cpu_req.then(|| format_millicores(cpu_req_m)),
            has_mem_req.then(|| format_mib(mem_req_mib)),
            has_cpu_lim.then(|| format_millicores(cpu_lim_m)),
            has_mem_lim.then(|| format_mib(mem_lim_mib)),
        )
    };

    PodInfo {
        name: pod.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
        namespace: pod
            .metadata
            .namespace
            .unwrap_or_else(|| "default".to_string()),
        status: pod
            .status
            .as_ref()
            .and_then(|status| status.phase.clone())
            .unwrap_or_else(|| "Unknown".to_string()),
        node: pod.spec.as_ref().and_then(|spec| spec.node_name.clone()),
        pod_ip: pod.status.as_ref().and_then(|status| status.pod_ip.clone()),
        restarts,
        created_at: pod.metadata.creation_timestamp.as_ref().map(|ts| ts.0),
        labels: pod
            .metadata
            .labels
            .unwrap_or_default()
            .into_iter()
            .collect(),
        annotations: pod
            .metadata
            .annotations
            .unwrap_or_default()
            .into_iter()
            .collect(),
        owner_references: pod
            .metadata
            .owner_references
            .unwrap_or_default()
            .into_iter()
            .map(|oref| OwnerRefInfo {
                kind: oref.kind,
                name: oref.name,
                uid: oref.uid,
            })
            .collect(),
        waiting_reasons,
        cpu_request,
        memory_request,
        cpu_limit,
        memory_limit,
    }
}

/// Converts a raw Kubernetes `Deployment` into a [`DeploymentInfo`] DTO.
pub fn deployment_to_info(dep: Deployment) -> DeploymentInfo {
    let now = Utc::now();
    let desired_replicas = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
    let ready_replicas = dep
        .status
        .as_ref()
        .and_then(|s| s.ready_replicas)
        .unwrap_or(0);
    let available_replicas = dep
        .status
        .as_ref()
        .and_then(|s| s.available_replicas)
        .unwrap_or(0);
    let updated_replicas = dep
        .status
        .as_ref()
        .and_then(|s| s.updated_replicas)
        .unwrap_or(0);
    let created_at = dep.metadata.creation_timestamp.as_ref().map(|ts| ts.0);
    let image = extract_image_from_pod_spec(
        dep.spec
            .as_ref()
            .and_then(|spec| spec.template.spec.as_ref()),
    );

    DeploymentInfo {
        name: dep.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
        namespace: dep
            .metadata
            .namespace
            .unwrap_or_else(|| "default".to_string()),
        desired_replicas,
        ready_replicas,
        available_replicas,
        updated_replicas,
        created_at,
        ready: format!("{ready_replicas}/{desired_replicas}"),
        age: created_at.and_then(|ts| (now - ts).to_std().ok()),
        image,
    }
}

/// Converts a raw Kubernetes `ReplicaSet` into a [`ReplicaSetInfo`] DTO.
pub fn replicaset_to_info(rs: ReplicaSet) -> ReplicaSetInfo {
    let now = Utc::now();
    let spec = rs.spec.as_ref();
    let status = rs.status.as_ref();
    let created_at = rs.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

    ReplicaSetInfo {
        name: rs.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
        namespace: rs
            .metadata
            .namespace
            .unwrap_or_else(|| "default".to_string()),
        desired: spec.and_then(|s| s.replicas).unwrap_or(0),
        ready: status.and_then(|s| s.ready_replicas).unwrap_or(0),
        available: status.and_then(|s| s.available_replicas).unwrap_or(0),
        image: extract_image_from_pod_spec(
            spec.and_then(|s| s.template.as_ref())
                .and_then(|t| t.spec.as_ref()),
        ),
        age: created_at.and_then(|ts| (now - ts).to_std().ok()),
        created_at,
        owner_references: rs
            .metadata
            .owner_references
            .unwrap_or_default()
            .into_iter()
            .map(|oref| OwnerRefInfo {
                kind: oref.kind,
                name: oref.name,
                uid: oref.uid,
            })
            .collect(),
    }
}

/// Converts a raw Kubernetes `StatefulSet` into a [`StatefulSetInfo`] DTO.
pub fn statefulset_to_info(ss: StatefulSet) -> StatefulSetInfo {
    let now = Utc::now();
    let spec = ss.spec.as_ref();
    let status = ss.status.as_ref();
    let created_at = ss.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

    StatefulSetInfo {
        name: ss.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
        namespace: ss
            .metadata
            .namespace
            .unwrap_or_else(|| "default".to_string()),
        desired_replicas: spec.and_then(|s| s.replicas).unwrap_or(1),
        ready_replicas: status.and_then(|s| s.ready_replicas).unwrap_or(0),
        service_name: spec
            .map(|s| s.service_name.clone())
            .unwrap_or_else(|| "<none>".to_string()),
        pod_management_policy: spec
            .and_then(|s| s.pod_management_policy.clone())
            .unwrap_or_else(|| "OrderedReady".to_string()),
        image: extract_image_from_pod_spec(spec.and_then(|s| s.template.spec.as_ref())),
        age: created_at.and_then(|ts| (now - ts).to_std().ok()),
        created_at,
    }
}

/// Converts a raw Kubernetes `DaemonSet` into a [`DaemonSetInfo`] DTO.
pub fn daemonset_to_info(ds: DaemonSet) -> DaemonSetInfo {
    let now = Utc::now();
    let spec = ds.spec.as_ref();
    let status = ds.status.as_ref();
    let created_at = ds.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

    let desired_count = status.map(|s| s.desired_number_scheduled).unwrap_or(0);
    let ready_count = status.map(|s| s.number_ready).unwrap_or(0);
    let unavailable_count = status.and_then(|s| s.number_unavailable).unwrap_or(0);

    DaemonSetInfo {
        name: ds.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
        namespace: ds
            .metadata
            .namespace
            .unwrap_or_else(|| "default".to_string()),
        desired_count,
        ready_count,
        unavailable_count,
        selector: spec
            .and_then(|s| s.selector.match_labels.as_ref())
            .map(|labels| {
                labels
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_else(|| "<none>".to_string()),
        update_strategy: spec
            .and_then(|s| s.update_strategy.as_ref())
            .and_then(|us| us.type_.clone())
            .unwrap_or_else(|| "RollingUpdate".to_string()),
        labels: ds
            .metadata
            .labels
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect(),
        status_message: if unavailable_count == 0 {
            "Ready".to_string()
        } else {
            format!("{unavailable_count} pods unavailable")
        },
        image: extract_image_from_pod_spec(spec.and_then(|s| s.template.spec.as_ref())),
        age: created_at.and_then(|ts| (now - ts).to_std().ok()),
        created_at,
    }
}

/// Converts a raw Kubernetes `Service` into a [`ServiceInfo`] DTO.
pub fn service_to_info(svc: Service) -> ServiceInfo {
    let now = Utc::now();
    let ports = svc
        .spec
        .as_ref()
        .and_then(|spec| spec.ports.as_ref())
        .map(|ports| {
            ports
                .iter()
                .map(|p| {
                    format!(
                        "{}/{}",
                        p.port,
                        p.protocol.clone().unwrap_or_else(|| "TCP".to_string())
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    let service_type = svc
        .spec
        .as_ref()
        .and_then(|spec| spec.type_.clone())
        .unwrap_or_else(|| "ClusterIP".to_string());

    let created_at = svc.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

    ServiceInfo {
        name: svc.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
        namespace: svc
            .metadata
            .namespace
            .unwrap_or_else(|| "default".to_string()),
        type_: service_type,
        cluster_ip: svc.spec.as_ref().and_then(|spec| spec.cluster_ip.clone()),
        ports,
        selector: svc
            .spec
            .as_ref()
            .and_then(|spec| spec.selector.clone())
            .unwrap_or_default(),
        created_at,
        age: created_at.and_then(|ts| (now - ts).to_std().ok()),
    }
}

/// Checks whether a node condition of the given type has status `"True"`.
pub fn node_condition_true(node: &Node, condition_type: &str) -> bool {
    node.status
        .as_ref()
        .and_then(|status| status.conditions.as_ref())
        .and_then(|conditions| {
            conditions
                .iter()
                .find(|condition| condition.type_ == condition_type)
        })
        .is_some_and(|condition| condition.status == "True")
}

/// Extracts the role of a node from its labels.
pub fn node_role(node: &Node) -> String {
    let labels = node.metadata.labels.as_ref();

    let is_control_plane = labels.is_some_and(|labels| {
        labels.contains_key("node-role.kubernetes.io/control-plane")
            || labels.contains_key("node-role.kubernetes.io/master")
    });

    if is_control_plane {
        "master".to_string()
    } else {
        "worker".to_string()
    }
}

/// Converts a raw Kubernetes `Node` into a [`NodeInfo`] DTO.
pub fn node_to_info(node: Node) -> NodeInfo {
    let alloc = node
        .status
        .as_ref()
        .and_then(|status| status.allocatable.as_ref());
    let name = node
        .metadata
        .name
        .as_ref()
        .cloned()
        .unwrap_or_else(|| "<unknown>".to_string());

    NodeInfo {
        name,
        ready: node_condition_true(&node, "Ready"),
        kubelet_version: node
            .status
            .as_ref()
            .and_then(|status| status.node_info.as_ref())
            .map(|info| info.kubelet_version.clone())
            .unwrap_or_else(|| "unknown".to_string()),
        os_image: node
            .status
            .as_ref()
            .and_then(|status| status.node_info.as_ref())
            .map(|info| info.os_image.clone())
            .unwrap_or_else(|| "unknown".to_string()),
        role: node_role(&node),
        cpu_allocatable: alloc.and_then(|a| a.get("cpu").map(|q| q.0.clone())),
        memory_allocatable: alloc.and_then(|a| a.get("memory").map(|q| q.0.clone())),
        created_at: node.metadata.creation_timestamp.as_ref().map(|ts| ts.0),
        memory_pressure: node_condition_true(&node, "MemoryPressure"),
        disk_pressure: node_condition_true(&node, "DiskPressure"),
        pid_pressure: node_condition_true(&node, "PIDPressure"),
        network_unavailable: node_condition_true(&node, "NetworkUnavailable"),
        unschedulable: node
            .spec
            .as_ref()
            .and_then(|s| s.unschedulable)
            .unwrap_or(false),
    }
}

/// Converts a raw Kubernetes `ReplicationController` into a [`ReplicationControllerInfo`] DTO.
pub fn replication_controller_to_info(rc: ReplicationController) -> ReplicationControllerInfo {
    let now = Utc::now();
    let spec = rc.spec.as_ref();
    let status = rc.status.as_ref();
    let created_at = rc.metadata.creation_timestamp.as_ref().map(|ts| ts.0);
    ReplicationControllerInfo {
        name: rc.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
        namespace: rc
            .metadata
            .namespace
            .unwrap_or_else(|| "default".to_string()),
        desired: spec.and_then(|s| s.replicas).unwrap_or(0),
        ready: status.and_then(|s| s.ready_replicas).unwrap_or(0),
        available: status.and_then(|s| s.available_replicas).unwrap_or(0),
        image: extract_image_from_pod_spec(
            spec.and_then(|s| s.template.as_ref())
                .and_then(|t| t.spec.as_ref()),
        ),
        age: created_at.and_then(|ts| (now - ts).to_std().ok()),
        created_at,
    }
}

/// Converts a raw Kubernetes `Job` into a [`JobInfo`] DTO.
pub fn job_to_info(job: Job) -> JobInfo {
    let now = Utc::now();
    let spec = job.spec.as_ref();
    let status = job.status.as_ref();

    let succeeded = status.and_then(|s| s.succeeded).unwrap_or(0);
    let failed = status.and_then(|s| s.failed).unwrap_or(0);
    let active = status.and_then(|s| s.active).unwrap_or(0);
    let desired_completions = spec.and_then(|s| s.completions).unwrap_or(1);
    let parallelism = spec.and_then(|s| s.parallelism).unwrap_or(1);
    let start_time = status.and_then(|s| s.start_time.as_ref()).map(|ts| ts.0);
    let completion_time = status
        .and_then(|s| s.completion_time.as_ref())
        .map(|ts| ts.0);
    let created_at = job.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

    JobInfo {
        name: job.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
        namespace: job
            .metadata
            .namespace
            .unwrap_or_else(|| "default".to_string()),
        status: job_status_from_counts(succeeded, failed, active),
        completions: format_job_completions(succeeded, desired_completions),
        duration: format_job_duration(start_time, completion_time),
        desired_completions,
        succeeded_pods: succeeded,
        parallelism,
        active_pods: active,
        failed_pods: failed,
        age: created_at.and_then(|ts| (now - ts).to_std().ok()),
        created_at,
        owner_references: job
            .metadata
            .owner_references
            .unwrap_or_default()
            .into_iter()
            .map(|oref| OwnerRefInfo {
                kind: oref.kind,
                name: oref.name,
                uid: oref.uid,
            })
            .collect(),
    }
}

/// Converts a raw Kubernetes `CronJob` into a [`CronJobInfo`] DTO.
pub fn cronjob_to_info(cj: CronJob) -> CronJobInfo {
    let now = Utc::now();
    let spec = cj.spec.as_ref();
    let status = cj.status.as_ref();
    let created_at = cj.metadata.creation_timestamp.as_ref().map(|ts| ts.0);
    let schedule = spec
        .map(|s| s.schedule.clone())
        .unwrap_or_else(|| "<none>".to_string());
    let timezone = spec.and_then(|s| s.time_zone.clone());
    let suspend = spec.and_then(|s| s.suspend).unwrap_or(false);

    CronJobInfo {
        name: cj.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
        namespace: cj
            .metadata
            .namespace
            .unwrap_or_else(|| "default".to_string()),
        schedule: schedule.clone(),
        timezone: timezone.clone(),
        last_schedule_time: status
            .and_then(|s| s.last_schedule_time.as_ref())
            .map(|ts| ts.0),
        next_schedule_time: cronjob_next_schedule_time(
            &schedule,
            timezone.as_deref(),
            suspend,
            now,
        ),
        last_successful_time: status
            .and_then(|s| s.last_successful_time.as_ref())
            .map(|ts| ts.0),
        suspend,
        active_jobs: status
            .and_then(|s| s.active.as_ref())
            .map(|v| v.len() as i32)
            .unwrap_or(0),
        age: created_at.and_then(|ts| (now - ts).to_std().ok()),
        created_at,
    }
}

pub(crate) fn job_status_from_counts(succeeded: i32, failed: i32, active: i32) -> String {
    if succeeded > 0 && active == 0 {
        "Succeeded".to_string()
    } else if failed > 0 {
        "Failed".to_string()
    } else if active > 0 {
        "Running".to_string()
    } else {
        "Pending".to_string()
    }
}

pub(crate) fn format_job_completions(succeeded: i32, parallelism: i32) -> String {
    format!("{}/{}", succeeded.max(0), parallelism.max(1))
}

pub(crate) fn format_job_duration(
    start_time: Option<chrono::DateTime<Utc>>,
    completion_time: Option<chrono::DateTime<Utc>>,
) -> Option<String> {
    let start = start_time?;
    let end = completion_time.unwrap_or_else(Utc::now);
    let delta = end.signed_duration_since(start);

    let secs = delta.num_seconds().max(0);
    let mins = secs / 60;
    let rem_secs = secs % 60;

    if mins > 0 {
        Some(format!("{mins}m{rem_secs}s"))
    } else {
        Some(format!("{rem_secs}s"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        Container, ContainerState, ContainerStateWaiting, ContainerStatus, PodSpec, PodStatus,
        ResourceRequirements,
    };
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::collections::BTreeMap;

    fn minimal_pod(name: &str, namespace: &str) -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn minimal_pod_defaults() {
        let info = pod_to_info(minimal_pod("my-pod", "default"));
        assert_eq!(info.name, "my-pod");
        assert_eq!(info.namespace, "default");
        assert_eq!(info.status, "Unknown");
        assert_eq!(info.restarts, 0);
        assert!(info.node.is_none());
        assert!(info.pod_ip.is_none());
        assert!(info.labels.is_empty());
        assert!(info.annotations.is_empty());
        assert!(info.owner_references.is_empty());
        assert!(info.waiting_reasons.is_empty());
        assert!(info.cpu_request.is_none());
        assert!(info.memory_request.is_none());
        assert!(info.cpu_limit.is_none());
        assert!(info.memory_limit.is_none());
    }

    #[test]
    fn missing_name_uses_unknown() {
        let pod = Pod {
            metadata: ObjectMeta {
                namespace: Some("ns".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let info = pod_to_info(pod);
        assert_eq!(info.name, "<unknown>");
    }

    #[test]
    fn missing_namespace_uses_default() {
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("p".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let info = pod_to_info(pod);
        assert_eq!(info.namespace, "default");
    }

    #[test]
    fn extracts_status_phase() {
        let mut pod = minimal_pod("p", "ns");
        pod.status = Some(PodStatus {
            phase: Some("Running".to_string()),
            ..Default::default()
        });
        let info = pod_to_info(pod);
        assert_eq!(info.status, "Running");
    }

    #[test]
    fn extracts_node_and_pod_ip() {
        let mut pod = minimal_pod("p", "ns");
        pod.spec = Some(PodSpec {
            node_name: Some("node-1".to_string()),
            containers: vec![],
            ..Default::default()
        });
        pod.status = Some(PodStatus {
            pod_ip: Some("10.0.0.5".to_string()),
            ..Default::default()
        });
        let info = pod_to_info(pod);
        assert_eq!(info.node.as_deref(), Some("node-1"));
        assert_eq!(info.pod_ip.as_deref(), Some("10.0.0.5"));
    }

    #[test]
    fn sums_restarts_across_containers() {
        let mut pod = minimal_pod("p", "ns");
        pod.status = Some(PodStatus {
            container_statuses: Some(vec![
                ContainerStatus {
                    name: "a".to_string(),
                    restart_count: 3,
                    ready: true,
                    image: String::new(),
                    image_id: String::new(),
                    ..Default::default()
                },
                ContainerStatus {
                    name: "b".to_string(),
                    restart_count: 2,
                    ready: true,
                    image: String::new(),
                    image_id: String::new(),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        });
        let info = pod_to_info(pod);
        assert_eq!(info.restarts, 5);
    }

    #[test]
    fn collects_waiting_reasons() {
        let mut pod = minimal_pod("p", "ns");
        pod.status = Some(PodStatus {
            container_statuses: Some(vec![ContainerStatus {
                name: "c".to_string(),
                restart_count: 0,
                ready: false,
                image: String::new(),
                image_id: String::new(),
                state: Some(ContainerState {
                    waiting: Some(ContainerStateWaiting {
                        reason: Some("CrashLoopBackOff".to_string()),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ..Default::default()
        });
        let info = pod_to_info(pod);
        assert_eq!(info.waiting_reasons, vec!["CrashLoopBackOff"]);
    }

    #[test]
    fn parses_resource_requests_and_limits() {
        let mut pod = minimal_pod("p", "ns");
        let mut requests = BTreeMap::new();
        requests.insert("cpu".to_string(), Quantity("250m".to_string()));
        requests.insert("memory".to_string(), Quantity("128Mi".to_string()));
        let mut limits = BTreeMap::new();
        limits.insert("cpu".to_string(), Quantity("1".to_string()));
        limits.insert("memory".to_string(), Quantity("512Mi".to_string()));
        pod.spec = Some(PodSpec {
            containers: vec![Container {
                name: "app".to_string(),
                resources: Some(ResourceRequirements {
                    requests: Some(requests),
                    limits: Some(limits),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        });
        let info = pod_to_info(pod);
        assert!(info.cpu_request.is_some());
        assert!(info.memory_request.is_some());
        assert!(info.cpu_limit.is_some());
        assert!(info.memory_limit.is_some());
    }

    #[test]
    fn extracts_labels_and_annotations() {
        let mut pod = minimal_pod("p", "ns");
        let mut labels = BTreeMap::new();
        labels.insert("app".to_string(), "web".to_string());
        let mut annotations = BTreeMap::new();
        annotations.insert("note".to_string(), "test".to_string());
        pod.metadata.labels = Some(labels);
        pod.metadata.annotations = Some(annotations);
        let info = pod_to_info(pod);
        assert_eq!(info.labels, vec![("app".to_string(), "web".to_string())]);
        assert_eq!(
            info.annotations,
            vec![("note".to_string(), "test".to_string())]
        );
    }

    // ── deployment_to_info tests ──

    #[test]
    fn deployment_minimal_defaults() {
        let dep = Deployment::default();
        let info = deployment_to_info(dep);
        assert_eq!(info.name, "<unknown>");
        assert_eq!(info.namespace, "default");
        assert_eq!(info.desired_replicas, 1);
        assert_eq!(info.ready_replicas, 0);
        assert_eq!(info.ready, "0/1");
        assert!(info.image.is_none());
    }

    #[test]
    fn deployment_extracts_replicas() {
        use k8s_openapi::api::apps::v1::{DeploymentSpec, DeploymentStatus};
        let dep = Deployment {
            metadata: ObjectMeta {
                name: Some("web".to_string()),
                namespace: Some("prod".to_string()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: Some(3),
                ..Default::default()
            }),
            status: Some(DeploymentStatus {
                ready_replicas: Some(2),
                available_replicas: Some(2),
                updated_replicas: Some(3),
                ..Default::default()
            }),
        };
        let info = deployment_to_info(dep);
        assert_eq!(info.name, "web");
        assert_eq!(info.namespace, "prod");
        assert_eq!(info.desired_replicas, 3);
        assert_eq!(info.ready_replicas, 2);
        assert_eq!(info.available_replicas, 2);
        assert_eq!(info.updated_replicas, 3);
        assert_eq!(info.ready, "2/3");
    }

    // ── replicaset_to_info tests ──

    #[test]
    fn replicaset_minimal_defaults() {
        let rs = ReplicaSet::default();
        let info = replicaset_to_info(rs);
        assert_eq!(info.name, "<unknown>");
        assert_eq!(info.namespace, "default");
        assert_eq!(info.desired, 0);
        assert_eq!(info.ready, 0);
        assert!(info.owner_references.is_empty());
    }

    #[test]
    fn replicaset_extracts_owner_references() {
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
        let rs = ReplicaSet {
            metadata: ObjectMeta {
                name: Some("web-abc".to_string()),
                namespace: Some("ns".to_string()),
                owner_references: Some(vec![OwnerReference {
                    kind: "Deployment".to_string(),
                    name: "web".to_string(),
                    uid: "uid-123".to_string(),
                    api_version: "apps/v1".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            },
            ..Default::default()
        };
        let info = replicaset_to_info(rs);
        assert_eq!(info.owner_references.len(), 1);
        assert_eq!(info.owner_references[0].kind, "Deployment");
        assert_eq!(info.owner_references[0].name, "web");
    }

    // ── extract_image_from_pod_spec tests ──

    #[test]
    fn extract_image_none_spec() {
        assert!(extract_image_from_pod_spec(None).is_none());
    }

    #[test]
    fn extract_image_from_container() {
        let spec = PodSpec {
            containers: vec![Container {
                name: "app".to_string(),
                image: Some("nginx:latest".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            extract_image_from_pod_spec(Some(&spec)),
            Some("nginx:latest".to_string())
        );
    }
}
