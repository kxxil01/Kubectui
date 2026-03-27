//! Shared DTO conversion functions for Kubernetes API objects.
//!
//! These conversions are used by both the polling path (`client.rs`) and
//! the watch path (`state/watch.rs`) to produce identical typed DTOs.

use crate::time::{AppTimestamp, age_duration_from_timestamp, now};
use std::collections::BTreeMap;

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{
    Endpoints, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Pod, PodSpec,
    ReplicationController, Service, ServiceAccount,
};
use k8s_openapi::api::networking::v1::{Ingress, IngressClass, NetworkPolicy};
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use k8s_openapi::api::rbac::v1::{
    ClusterRole, ClusterRoleBinding, PolicyRule, Role, RoleBinding, Subject,
};
use k8s_openapi::api::scheduling::v1::PriorityClass;
use k8s_openapi::api::storage::v1::StorageClass;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::core::PartialObjectMeta;

use crate::cronjob::cronjob_next_schedule_time;
use crate::k8s::dtos::{
    ClusterRoleBindingInfo, ClusterRoleInfo, ConfigMapInfo, ContainerPortInfo, CronJobInfo,
    DaemonSetInfo, DeploymentInfo, EndpointInfo, HpaInfo, IngressClassInfo, IngressInfo,
    IngressRouteInfo, JobInfo, K8sEventInfo, LabelSelectorInfo, LabelSelectorRequirementInfo,
    LimitRangeInfo, LimitSpec, NamespaceInfo, NetworkPolicyInfo, NetworkPolicyPeerInfo,
    NetworkPolicyPortInfo, NetworkPolicyRuleInfo, NodeInfo, OwnerRefInfo, PodDisruptionBudgetInfo,
    PodInfo, PriorityClassInfo, PvInfo, PvcInfo, RbacRule, ReplicaSetInfo,
    ReplicationControllerInfo, ResourceQuotaInfo, RoleBindingInfo, RoleBindingSubject, RoleInfo,
    SecretInfo, ServiceAccountInfo, ServiceInfo, ServicePortInfo, StatefulSetInfo,
    StorageClassInfo,
};
use crate::state::alerts::{format_mib, format_millicores, parse_mib, parse_millicores};

/// Common metadata fields extracted from any Kubernetes resource.
pub(crate) struct CommonMetadata {
    pub(crate) name: String,
    pub(crate) namespace: String,
    pub(crate) created_at: Option<AppTimestamp>,
    pub(crate) age: Option<std::time::Duration>,
}

/// Converts a Kubernetes `jiff::Timestamp` into the app's canonical timestamp.
pub(crate) fn app_timestamp_from_k8s_timestamp(
    timestamp: &k8s_openapi::jiff::Timestamp,
) -> Option<AppTimestamp> {
    Some(*timestamp)
}

/// Computes a non-negative age from a creation timestamp relative to `now`.
pub(crate) fn age_from_created_at(
    created_at: Option<AppTimestamp>,
    now: AppTimestamp,
) -> Option<std::time::Duration> {
    age_duration_from_timestamp(created_at, now)
}

/// Extracts standard metadata fields shared by all Kubernetes resources.
pub(crate) fn extract_common_metadata(
    meta: &k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta,
) -> CommonMetadata {
    let now = now();
    let created_at = meta
        .creation_timestamp
        .as_ref()
        .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0));
    CommonMetadata {
        name: meta.name.clone().unwrap_or_else(|| "<unknown>".to_string()),
        namespace: meta
            .namespace
            .clone()
            .unwrap_or_else(|| "default".to_string()),
        created_at,
        age: age_from_created_at(created_at, now),
    }
}

fn label_selector_to_info(
    selector: Option<&k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector>,
) -> LabelSelectorInfo {
    let Some(selector) = selector else {
        return LabelSelectorInfo::default();
    };

    LabelSelectorInfo {
        match_labels: selector.match_labels.clone().unwrap_or_default(),
        match_expressions: selector
            .match_expressions
            .as_ref()
            .map(|requirements| {
                requirements
                    .iter()
                    .map(|req| LabelSelectorRequirementInfo {
                        key: req.key.clone(),
                        operator: req.operator.clone(),
                        values: req.values.clone().unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn pod_spec_images(spec: Option<&PodSpec>) -> Vec<String> {
    spec.map(|spec| {
        spec.containers
            .iter()
            .filter_map(|container| container.image.clone())
            .collect()
    })
    .unwrap_or_default()
}

fn collect_pod_spec_resource_refs(spec: Option<&PodSpec>) -> (Vec<String>, Vec<String>) {
    let mut referenced_config_maps = BTreeMap::<String, ()>::new();
    let mut referenced_secrets = BTreeMap::<String, ()>::new();
    let Some(spec) = spec else {
        return (Vec::new(), Vec::new());
    };

    for volume in spec.volumes.as_deref().unwrap_or_default() {
        if let Some(config_map) = &volume.config_map {
            referenced_config_maps.insert(config_map.name.clone(), ());
        }
        if let Some(secret) = &volume.secret
            && let Some(name) = &secret.secret_name
        {
            referenced_secrets.insert(name.clone(), ());
        }
    }
    for image_pull_secret in spec.image_pull_secrets.as_deref().unwrap_or_default() {
        referenced_secrets.insert(image_pull_secret.name.clone(), ());
    }
    for container in spec
        .containers
        .iter()
        .chain(spec.init_containers.as_deref().unwrap_or_default().iter())
    {
        for env_from in container.env_from.as_deref().unwrap_or_default() {
            if let Some(config_map_ref) = &env_from.config_map_ref {
                referenced_config_maps.insert(config_map_ref.name.clone(), ());
            }
            if let Some(secret_ref) = &env_from.secret_ref {
                referenced_secrets.insert(secret_ref.name.clone(), ());
            }
        }
        for env in container.env.as_deref().unwrap_or_default() {
            if let Some(value_from) = &env.value_from {
                if let Some(config_map_key_ref) = &value_from.config_map_key_ref {
                    referenced_config_maps.insert(config_map_key_ref.name.clone(), ());
                }
                if let Some(secret_key_ref) = &value_from.secret_key_ref {
                    referenced_secrets.insert(secret_key_ref.name.clone(), ());
                }
            }
        }
    }

    (
        referenced_config_maps.into_keys().collect(),
        referenced_secrets.into_keys().collect(),
    )
}

fn template_labels(meta: Option<&ObjectMeta>) -> BTreeMap<String, String> {
    meta.and_then(|meta| meta.labels.clone())
        .unwrap_or_default()
}

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

    let spec = pod.spec.as_ref();
    let pod_run_as_non_root = spec
        .and_then(|spec| spec.security_context.as_ref())
        .and_then(|context| context.run_as_non_root);
    let run_as_non_root_configured = !containers.is_empty()
        && containers.iter().all(|container| {
            container
                .security_context
                .as_ref()
                .and_then(|context| context.run_as_non_root)
                .or(pod_run_as_non_root)
                == Some(true)
        });

    let missing_liveness_probes = containers
        .iter()
        .filter(|container| container.liveness_probe.is_none())
        .count();
    let missing_readiness_probes = containers
        .iter()
        .filter(|container| container.readiness_probe.is_none())
        .count();

    let mut container_ports = containers
        .iter()
        .flat_map(|container| container.ports.as_deref().unwrap_or_default().iter())
        .map(|port| ContainerPortInfo {
            name: port.name.clone(),
            container_port: port.container_port,
            protocol: port.protocol.clone().unwrap_or_else(|| "TCP".to_string()),
        })
        .collect::<Vec<_>>();
    container_ports.sort_unstable_by(|left, right| {
        left.protocol
            .cmp(&right.protocol)
            .then_with(|| left.container_port.cmp(&right.container_port))
            .then_with(|| left.name.cmp(&right.name))
    });
    container_ports.dedup();

    let (referenced_config_maps, referenced_secrets) = collect_pod_spec_resource_refs(spec);

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
        created_at: pod
            .metadata
            .creation_timestamp
            .as_ref()
            .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0)),
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
        container_images: containers
            .iter()
            .filter_map(|container| container.image.clone())
            .collect(),
        container_ports,
        missing_liveness_probes,
        missing_readiness_probes,
        run_as_non_root_configured,
        host_network: spec.and_then(|spec| spec.host_network).unwrap_or(false),
        host_pid: spec.and_then(|spec| spec.host_pid).unwrap_or(false),
        host_ipc: spec.and_then(|spec| spec.host_ipc).unwrap_or(false),
        referenced_config_maps,
        referenced_secrets,
    }
}

/// Converts a raw Kubernetes `Deployment` into a [`DeploymentInfo`] DTO.
pub fn deployment_to_info(dep: Deployment) -> DeploymentInfo {
    let now = now();
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
    let created_at = dep
        .metadata
        .creation_timestamp
        .as_ref()
        .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0));
    let pod_spec = dep
        .spec
        .as_ref()
        .and_then(|spec| spec.template.spec.as_ref());
    let image = extract_image_from_pod_spec(pod_spec);
    let (referenced_config_maps, referenced_secrets) = collect_pod_spec_resource_refs(pod_spec);
    let pod_template_labels = dep
        .spec
        .as_ref()
        .and_then(|spec| spec.template.metadata.as_ref())
        .and_then(|meta| meta.labels.clone())
        .unwrap_or_default();

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
        age: age_from_created_at(created_at, now),
        image,
        images: pod_spec_images(pod_spec),
        selector: dep
            .spec
            .as_ref()
            .map(|spec| label_selector_to_info(Some(&spec.selector)))
            .unwrap_or_default(),
        pod_template_labels,
        referenced_config_maps,
        referenced_secrets,
        annotations: dep
            .metadata
            .annotations
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect(),
    }
}

/// Converts a raw Kubernetes `ReplicaSet` into a [`ReplicaSetInfo`] DTO.
pub fn replicaset_to_info(rs: ReplicaSet) -> ReplicaSetInfo {
    let now = now();
    let spec = rs.spec.as_ref();
    let status = rs.status.as_ref();
    let created_at = rs
        .metadata
        .creation_timestamp
        .as_ref()
        .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0));
    let pod_spec = spec
        .and_then(|s| s.template.as_ref())
        .and_then(|t| t.spec.as_ref());
    let (referenced_config_maps, referenced_secrets) = collect_pod_spec_resource_refs(pod_spec);

    ReplicaSetInfo {
        name: rs.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
        namespace: rs
            .metadata
            .namespace
            .unwrap_or_else(|| "default".to_string()),
        desired: spec.and_then(|s| s.replicas).unwrap_or(0),
        ready: status.and_then(|s| s.ready_replicas).unwrap_or(0),
        available: status.and_then(|s| s.available_replicas).unwrap_or(0),
        image: extract_image_from_pod_spec(pod_spec),
        referenced_config_maps,
        referenced_secrets,
        age: age_from_created_at(created_at, now),
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
    let now = now();
    let spec = ss.spec.as_ref();
    let status = ss.status.as_ref();
    let created_at = ss
        .metadata
        .creation_timestamp
        .as_ref()
        .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0));
    let pod_template_labels =
        template_labels(spec.and_then(|statefulset| statefulset.template.metadata.as_ref()));
    let pod_spec = spec.and_then(|s| s.template.spec.as_ref());
    let (referenced_config_maps, referenced_secrets) = collect_pod_spec_resource_refs(pod_spec);

    StatefulSetInfo {
        name: ss.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
        namespace: ss
            .metadata
            .namespace
            .unwrap_or_else(|| "default".to_string()),
        desired_replicas: spec.and_then(|s| s.replicas).unwrap_or(1),
        ready_replicas: status.and_then(|s| s.ready_replicas).unwrap_or(0),
        service_name: spec
            .and_then(|s| s.service_name.clone())
            .unwrap_or_else(|| "<none>".to_string()),
        pod_management_policy: spec
            .and_then(|s| s.pod_management_policy.clone())
            .unwrap_or_else(|| "OrderedReady".to_string()),
        pod_template_labels,
        image: extract_image_from_pod_spec(pod_spec),
        referenced_config_maps,
        referenced_secrets,
        age: age_from_created_at(created_at, now),
        created_at,
    }
}

/// Converts a raw Kubernetes `DaemonSet` into a [`DaemonSetInfo`] DTO.
pub fn daemonset_to_info(ds: DaemonSet) -> DaemonSetInfo {
    let now = now();
    let spec = ds.spec.as_ref();
    let status = ds.status.as_ref();
    let created_at = ds
        .metadata
        .creation_timestamp
        .as_ref()
        .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0));

    let desired_count = status.map(|s| s.desired_number_scheduled).unwrap_or(0);
    let ready_count = status.map(|s| s.number_ready).unwrap_or(0);
    let unavailable_count = status.and_then(|s| s.number_unavailable).unwrap_or(0);
    let pod_template_labels =
        template_labels(spec.and_then(|daemonset| daemonset.template.metadata.as_ref()));
    let pod_spec = spec.and_then(|s| s.template.spec.as_ref());
    let (referenced_config_maps, referenced_secrets) = collect_pod_spec_resource_refs(pod_spec);

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
        pod_template_labels,
        status_message: if unavailable_count == 0 {
            "Ready".to_string()
        } else {
            format!("{unavailable_count} pods unavailable")
        },
        image: extract_image_from_pod_spec(pod_spec),
        referenced_config_maps,
        referenced_secrets,
        age: age_from_created_at(created_at, now),
        created_at,
    }
}

/// Converts a raw Kubernetes `Service` into a [`ServiceInfo`] DTO.
pub fn service_to_info(svc: Service) -> ServiceInfo {
    let now = now();
    let port_mappings = svc
        .spec
        .as_ref()
        .and_then(|spec| spec.ports.as_ref())
        .map(|ports| {
            ports
                .iter()
                .map(|port| ServicePortInfo {
                    port: port.port,
                    protocol: port.protocol.clone().unwrap_or_else(|| "TCP".to_string()),
                    target_port_name: match &port.target_port {
                        Some(IntOrString::String(name)) => Some(name.clone()),
                        _ => None,
                    },
                    target_port_number: match &port.target_port {
                        Some(IntOrString::Int(number)) => Some(*number),
                        _ => None,
                    },
                })
                .collect()
        })
        .unwrap_or_default();
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

    let created_at = svc
        .metadata
        .creation_timestamp
        .as_ref()
        .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0));

    ServiceInfo {
        name: svc.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
        namespace: svc
            .metadata
            .namespace
            .unwrap_or_else(|| "default".to_string()),
        type_: service_type,
        cluster_ip: svc.spec.as_ref().and_then(|spec| spec.cluster_ip.clone()),
        external_name: svc
            .spec
            .as_ref()
            .and_then(|spec| spec.external_name.clone()),
        labels: svc.metadata.labels.clone().unwrap_or_default(),
        ports,
        selector: svc
            .spec
            .as_ref()
            .and_then(|spec| spec.selector.clone())
            .unwrap_or_default(),
        port_mappings,
        annotations: svc
            .metadata
            .annotations
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect(),
        created_at,
        age: age_from_created_at(created_at, now),
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
        created_at: node
            .metadata
            .creation_timestamp
            .as_ref()
            .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0)),
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
    let now = now();
    let spec = rc.spec.as_ref();
    let status = rc.status.as_ref();
    let created_at = rc
        .metadata
        .creation_timestamp
        .as_ref()
        .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0));
    let pod_spec = spec
        .and_then(|s| s.template.as_ref())
        .and_then(|t| t.spec.as_ref());
    let (referenced_config_maps, referenced_secrets) = collect_pod_spec_resource_refs(pod_spec);
    ReplicationControllerInfo {
        name: rc.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
        namespace: rc
            .metadata
            .namespace
            .unwrap_or_else(|| "default".to_string()),
        desired: spec.and_then(|s| s.replicas).unwrap_or(0),
        ready: status.and_then(|s| s.ready_replicas).unwrap_or(0),
        available: status.and_then(|s| s.available_replicas).unwrap_or(0),
        image: extract_image_from_pod_spec(pod_spec),
        referenced_config_maps,
        referenced_secrets,
        age: age_from_created_at(created_at, now),
        created_at,
    }
}

/// Converts a raw Kubernetes `Job` into a [`JobInfo`] DTO.
pub fn job_to_info(job: Job) -> JobInfo {
    let now = now();
    let spec = job.spec.as_ref();
    let status = job.status.as_ref();

    let succeeded = status.and_then(|s| s.succeeded).unwrap_or(0);
    let failed = status.and_then(|s| s.failed).unwrap_or(0);
    let active = status.and_then(|s| s.active).unwrap_or(0);
    let desired_completions = spec.and_then(|s| s.completions).unwrap_or(1);
    let parallelism = spec.and_then(|s| s.parallelism).unwrap_or(1);
    let start_time = status
        .and_then(|s| s.start_time.as_ref())
        .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0));
    let completion_time = status
        .and_then(|s| s.completion_time.as_ref())
        .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0));
    let created_at = job
        .metadata
        .creation_timestamp
        .as_ref()
        .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0));
    let pod_template_labels = template_labels(spec.and_then(|job| job.template.metadata.as_ref()));
    let pod_spec = spec.and_then(|s| s.template.spec.as_ref());
    let (referenced_config_maps, referenced_secrets) = collect_pod_spec_resource_refs(pod_spec);

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
        pod_template_labels,
        referenced_config_maps,
        referenced_secrets,
        age: age_from_created_at(created_at, now),
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
    let now = now();
    let spec = cj.spec.as_ref();
    let status = cj.status.as_ref();
    let created_at = cj
        .metadata
        .creation_timestamp
        .as_ref()
        .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0));
    let schedule = spec
        .map(|s| s.schedule.clone())
        .unwrap_or_else(|| "<none>".to_string());
    let timezone = spec.and_then(|s| s.time_zone.clone());
    let suspend = spec.and_then(|s| s.suspend).unwrap_or(false);
    let pod_template_labels = template_labels(
        spec.and_then(|cronjob| cronjob.job_template.spec.as_ref())
            .and_then(|job_spec| job_spec.template.metadata.as_ref()),
    );
    let pod_spec = spec
        .and_then(|s| s.job_template.spec.as_ref())
        .and_then(|job_spec| job_spec.template.spec.as_ref());
    let (referenced_config_maps, referenced_secrets) = collect_pod_spec_resource_refs(pod_spec);

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
            .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0)),
        next_schedule_time: cronjob_next_schedule_time(
            &schedule,
            timezone.as_deref(),
            suspend,
            now,
        ),
        last_successful_time: status
            .and_then(|s| s.last_successful_time.as_ref())
            .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0)),
        suspend,
        active_jobs: status
            .and_then(|s| s.active.as_ref())
            .map(|v| v.len() as i32)
            .unwrap_or(0),
        pod_template_labels,
        referenced_config_maps,
        referenced_secrets,
        age: age_from_created_at(created_at, now),
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
    start_time: Option<AppTimestamp>,
    completion_time: Option<AppTimestamp>,
) -> Option<String> {
    let start = start_time?;
    let end = completion_time.unwrap_or_else(now);
    let secs = end.as_second().saturating_sub(start.as_second());
    let mins = secs / 60;
    let rem_secs = secs % 60;

    if mins > 0 {
        Some(format!("{mins}m{rem_secs}s"))
    } else {
        Some(format!("{rem_secs}s"))
    }
}

/// Converts a raw `ServiceAccount` into a [`ServiceAccountInfo`] DTO.
pub fn service_account_to_info(sa: ServiceAccount) -> ServiceAccountInfo {
    let m = extract_common_metadata(&sa.metadata);
    ServiceAccountInfo {
        name: m.name,
        namespace: m.namespace,
        secrets_count: sa.secrets.as_ref().map_or(0, |v| v.len()),
        secret_names: sa
            .secrets
            .as_ref()
            .map(|refs| {
                refs.iter()
                    .filter_map(|secret| secret.name.clone())
                    .collect()
            })
            .unwrap_or_default(),
        image_pull_secrets_count: sa.image_pull_secrets.as_ref().map_or(0, |v| v.len()),
        image_pull_secret_names: sa
            .image_pull_secrets
            .as_ref()
            .map(|refs| refs.iter().map(|secret| secret.name.clone()).collect())
            .unwrap_or_default(),
        automount_service_account_token: sa.automount_service_account_token,
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `Role` into a [`RoleInfo`] DTO.
pub fn role_to_info(role: Role) -> RoleInfo {
    let m = extract_common_metadata(&role.metadata);
    RoleInfo {
        name: m.name,
        namespace: m.namespace,
        rules: role
            .rules
            .as_ref()
            .map(|rules| rules.iter().map(rule_from_policy_rule).collect())
            .unwrap_or_default(),
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `RoleBinding` into a [`RoleBindingInfo`] DTO.
pub fn role_binding_to_info(rb: RoleBinding) -> RoleBindingInfo {
    let m = extract_common_metadata(&rb.metadata);
    let role_ref = rb.role_ref;
    RoleBindingInfo {
        name: m.name,
        namespace: m.namespace,
        role_ref_kind: role_ref.kind,
        role_ref_name: role_ref.name,
        subjects: rb
            .subjects
            .as_ref()
            .map(|subjects| subjects.iter().map(subject_from_k8s).collect())
            .unwrap_or_default(),
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `ClusterRole` into a [`ClusterRoleInfo`] DTO.
pub fn cluster_role_to_info(cr: ClusterRole) -> ClusterRoleInfo {
    let m = extract_common_metadata(&cr.metadata);
    ClusterRoleInfo {
        name: m.name,
        rules: cr
            .rules
            .as_ref()
            .map(|rules| rules.iter().map(rule_from_policy_rule).collect())
            .unwrap_or_default(),
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `ClusterRoleBinding` into a [`ClusterRoleBindingInfo`] DTO.
pub fn cluster_role_binding_to_info(crb: ClusterRoleBinding) -> ClusterRoleBindingInfo {
    let m = extract_common_metadata(&crb.metadata);
    let role_ref = crb.role_ref;
    ClusterRoleBindingInfo {
        name: m.name,
        role_ref_kind: role_ref.kind,
        role_ref_name: role_ref.name,
        subjects: crb
            .subjects
            .as_ref()
            .map(|subjects| subjects.iter().map(subject_from_k8s).collect())
            .unwrap_or_default(),
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `ResourceQuota` into a [`ResourceQuotaInfo`] DTO.
pub fn resource_quota_to_info(
    quota: k8s_openapi::api::core::v1::ResourceQuota,
) -> ResourceQuotaInfo {
    let m = extract_common_metadata(&quota.metadata);
    let hard = quota
        .status
        .as_ref()
        .and_then(|status| status.hard.as_ref())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, v.0))
        .collect::<BTreeMap<_, _>>();

    let used = quota
        .status
        .as_ref()
        .and_then(|status| status.used.as_ref())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, v.0))
        .collect::<BTreeMap<_, _>>();

    let percent_used = quota_percent_used(&hard, &used);

    ResourceQuotaInfo {
        name: m.name,
        namespace: m.namespace,
        hard,
        used,
        percent_used,
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `LimitRange` into a [`LimitRangeInfo`] DTO.
pub fn limit_range_to_info(range: k8s_openapi::api::core::v1::LimitRange) -> LimitRangeInfo {
    let m = extract_common_metadata(&range.metadata);
    let limits = range
        .spec
        .as_ref()
        .map(|spec| {
            spec.limits
                .iter()
                .map(|item| LimitSpec {
                    type_: item.type_.clone(),
                    min: quantity_map_to_string_map(item.min.clone()),
                    max: quantity_map_to_string_map(item.max.clone()),
                    default: quantity_map_to_string_map(item.default.clone()),
                    default_request: quantity_map_to_string_map(item.default_request.clone()),
                    max_limit_request_ratio: quantity_map_to_string_map(
                        item.max_limit_request_ratio.clone(),
                    ),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    LimitRangeInfo {
        name: m.name,
        namespace: m.namespace,
        limits,
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `PodDisruptionBudget` into a [`PodDisruptionBudgetInfo`] DTO.
pub fn pdb_to_info(pdb: PodDisruptionBudget) -> PodDisruptionBudgetInfo {
    let m = extract_common_metadata(&pdb.metadata);
    let spec = pdb.spec.as_ref();
    let status = pdb.status.as_ref();

    PodDisruptionBudgetInfo {
        name: m.name,
        namespace: m.namespace,
        min_available: spec
            .and_then(|s| s.min_available.as_ref())
            .map(int_or_string_to_string),
        max_unavailable: spec
            .and_then(|s| s.max_unavailable.as_ref())
            .map(int_or_string_to_string),
        current_healthy: status.map(|s| s.current_healthy).unwrap_or(0),
        desired_healthy: status.map(|s| s.desired_healthy).unwrap_or(0),
        disruptions_allowed: status.map(|s| s.disruptions_allowed).unwrap_or(0),
        expected_pods: status.map(|s| s.expected_pods).unwrap_or(0),
        selector: spec.map(|spec| label_selector_to_info(spec.selector.as_ref())),
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `Endpoints` into an [`EndpointInfo`] DTO.
pub fn endpoint_to_info(ep: Endpoints) -> EndpointInfo {
    let m = extract_common_metadata(&ep.metadata);
    let mut addresses = Vec::new();
    let mut ports = Vec::new();
    if let Some(subsets) = ep.subsets {
        for subset in &subsets {
            if let Some(addrs) = &subset.addresses {
                for addr in addrs {
                    addresses.push(addr.ip.clone());
                }
            }
            if let Some(ps) = &subset.ports {
                for p in ps {
                    ports.push(format!(
                        "{}/{}",
                        p.port,
                        p.protocol.as_deref().unwrap_or("TCP")
                    ));
                }
            }
        }
    }
    EndpointInfo {
        name: m.name,
        namespace: m.namespace,
        addresses,
        ports,
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `Ingress` into an [`IngressInfo`] DTO.
pub fn ingress_to_info(ing: Ingress) -> IngressInfo {
    let m = extract_common_metadata(&ing.metadata);
    let class = ing.spec.as_ref().and_then(|s| s.ingress_class_name.clone());
    let hosts: Vec<String> = ing
        .spec
        .as_ref()
        .and_then(|s| s.rules.as_ref())
        .map(|rules| rules.iter().filter_map(|r| r.host.clone()).collect())
        .unwrap_or_default();
    let address = ing
        .status
        .as_ref()
        .and_then(|s| s.load_balancer.as_ref())
        .and_then(|lb| lb.ingress.as_ref())
        .and_then(|ingresses| ingresses.first())
        .and_then(|i| i.ip.clone().or_else(|| i.hostname.clone()));
    let routes: Vec<IngressRouteInfo> = ing
        .spec
        .as_ref()
        .map(|spec| {
            let mut routes = Vec::new();
            if let Some(default_backend) = &spec.default_backend
                && let Some(svc) = &default_backend.service
            {
                let port = svc
                    .port
                    .as_ref()
                    .map(|p| {
                        p.name
                            .clone()
                            .unwrap_or_else(|| p.number.map(|n| n.to_string()).unwrap_or_default())
                    })
                    .unwrap_or_default();
                routes.push(IngressRouteInfo {
                    host: None,
                    path: None,
                    service_name: svc.name.clone(),
                    service_port: port,
                });
            }
            for rule in spec.rules.as_deref().unwrap_or_default() {
                if let Some(http) = &rule.http {
                    for path in &http.paths {
                        if let Some(svc) = &path.backend.service {
                            let port = svc
                                .port
                                .as_ref()
                                .map(|p| {
                                    p.name.clone().unwrap_or_else(|| {
                                        p.number.map(|n| n.to_string()).unwrap_or_default()
                                    })
                                })
                                .unwrap_or_default();
                            routes.push(IngressRouteInfo {
                                host: rule.host.clone(),
                                path: path.path.clone(),
                                service_name: svc.name.clone(),
                                service_port: port,
                            });
                        }
                    }
                }
            }
            routes
        })
        .unwrap_or_default();
    let mut backend_services: Vec<(String, String)> = routes
        .iter()
        .map(|route| (route.service_name.clone(), route.service_port.clone()))
        .collect();
    backend_services.sort();
    backend_services.dedup();
    IngressInfo {
        name: m.name,
        namespace: m.namespace,
        class,
        hosts,
        address,
        labels: ing.metadata.labels.clone().unwrap_or_default(),
        ports: vec!["80".to_string(), "443".to_string()],
        backend_services,
        routes,
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `IngressClass` into an [`IngressClassInfo`] DTO.
pub fn ingress_class_to_info(ic: IngressClass) -> IngressClassInfo {
    let is_default = ic
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get("ingressclass.kubernetes.io/is-default-class"))
        .map(|v| v == "true")
        .unwrap_or(false);
    let m = extract_common_metadata(&ic.metadata);
    IngressClassInfo {
        name: m.name,
        controller: ic
            .spec
            .as_ref()
            .map(|s| s.controller.clone().unwrap_or_default())
            .unwrap_or_default(),
        is_default,
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `NetworkPolicy` into a [`NetworkPolicyInfo`] DTO.
pub fn network_policy_to_info(np: NetworkPolicy) -> NetworkPolicyInfo {
    let m = extract_common_metadata(&np.metadata);
    let spec = np.spec.as_ref();
    let pod_selector_spec = spec
        .map(|spec| label_selector_to_info(spec.pod_selector.as_ref()))
        .unwrap_or_default();
    let pod_selector = if pod_selector_spec.match_labels.is_empty()
        && pod_selector_spec.match_expressions.is_empty()
    {
        "<all>".to_string()
    } else {
        pod_selector_spec
            .match_labels
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    let ingress: Vec<NetworkPolicyRuleInfo> = spec
        .and_then(|spec| spec.ingress.as_ref())
        .map(|rules| {
            rules
                .iter()
                .map(|rule| NetworkPolicyRuleInfo {
                    peers: rule
                        .from
                        .as_ref()
                        .map(|peers| {
                            peers
                                .iter()
                                .map(|peer| NetworkPolicyPeerInfo {
                                    pod_selector: peer
                                        .pod_selector
                                        .as_ref()
                                        .map(|selector| label_selector_to_info(Some(selector))),
                                    namespace_selector: peer
                                        .namespace_selector
                                        .as_ref()
                                        .map(|selector| label_selector_to_info(Some(selector))),
                                    ip_block_cidr: peer
                                        .ip_block
                                        .as_ref()
                                        .map(|ip_block| ip_block.cidr.clone()),
                                    ip_block_except: peer
                                        .ip_block
                                        .as_ref()
                                        .and_then(|ip_block| ip_block.except.clone())
                                        .unwrap_or_default(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    ports: rule
                        .ports
                        .as_ref()
                        .map(|ports| {
                            ports
                                .iter()
                                .map(|port| NetworkPolicyPortInfo {
                                    protocol: port.protocol.clone(),
                                    port_name: match &port.port {
                                        Some(IntOrString::String(name)) => Some(name.clone()),
                                        _ => None,
                                    },
                                    port_number: match &port.port {
                                        Some(IntOrString::Int(number)) => Some(*number),
                                        _ => None,
                                    },
                                    end_port: port.end_port,
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    let egress: Vec<NetworkPolicyRuleInfo> = spec
        .and_then(|spec| spec.egress.as_ref())
        .map(|rules| {
            rules
                .iter()
                .map(|rule| NetworkPolicyRuleInfo {
                    peers: rule
                        .to
                        .as_ref()
                        .map(|peers| {
                            peers
                                .iter()
                                .map(|peer| NetworkPolicyPeerInfo {
                                    pod_selector: peer
                                        .pod_selector
                                        .as_ref()
                                        .map(|selector| label_selector_to_info(Some(selector))),
                                    namespace_selector: peer
                                        .namespace_selector
                                        .as_ref()
                                        .map(|selector| label_selector_to_info(Some(selector))),
                                    ip_block_cidr: peer
                                        .ip_block
                                        .as_ref()
                                        .map(|ip_block| ip_block.cidr.clone()),
                                    ip_block_except: peer
                                        .ip_block
                                        .as_ref()
                                        .and_then(|ip_block| ip_block.except.clone())
                                        .unwrap_or_default(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    ports: rule
                        .ports
                        .as_ref()
                        .map(|ports| {
                            ports
                                .iter()
                                .map(|port| NetworkPolicyPortInfo {
                                    protocol: port.protocol.clone(),
                                    port_name: match &port.port {
                                        Some(IntOrString::String(name)) => Some(name.clone()),
                                        _ => None,
                                    },
                                    port_number: match &port.port {
                                        Some(IntOrString::Int(number)) => Some(*number),
                                        _ => None,
                                    },
                                    end_port: port.end_port,
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    let ingress_rules = ingress.len();
    let egress_rules = egress.len();
    NetworkPolicyInfo {
        name: m.name,
        namespace: m.namespace,
        pod_selector,
        pod_selector_spec,
        policy_types: spec
            .and_then(|spec| spec.policy_types.clone())
            .unwrap_or_default(),
        ingress,
        egress,
        ingress_rules,
        egress_rules,
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `ConfigMap` into a [`ConfigMapInfo`] DTO.
pub fn config_map_to_info(cm: k8s_openapi::api::core::v1::ConfigMap) -> ConfigMapInfo {
    let m = extract_common_metadata(&cm.metadata);
    let data_count = cm.data.as_ref().map(|d| d.len()).unwrap_or(0)
        + cm.binary_data.as_ref().map(|d| d.len()).unwrap_or(0);
    ConfigMapInfo {
        name: m.name,
        namespace: m.namespace,
        data_count,
        annotations: cm
            .metadata
            .annotations
            .unwrap_or_default()
            .into_iter()
            .collect(),
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `Secret` into a [`SecretInfo`] DTO.
pub fn secret_to_info(s: k8s_openapi::api::core::v1::Secret) -> SecretInfo {
    let m = extract_common_metadata(&s.metadata);
    let data_count = s.data.as_ref().map(|d| d.len()).unwrap_or(0);
    SecretInfo {
        name: m.name,
        namespace: m.namespace,
        type_: s.type_.unwrap_or_else(|| "Opaque".to_string()),
        data_count,
        annotations: s
            .metadata
            .annotations
            .unwrap_or_default()
            .into_iter()
            .collect(),
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `HorizontalPodAutoscaler` into an [`HpaInfo`] DTO.
pub fn hpa_to_info(hpa: HorizontalPodAutoscaler) -> HpaInfo {
    let m = extract_common_metadata(&hpa.metadata);
    let spec = hpa.spec.as_ref();
    let status = hpa.status.as_ref();
    let reference = spec
        .map(|s| format!("{}/{}", s.scale_target_ref.kind, s.scale_target_ref.name))
        .unwrap_or_default();
    HpaInfo {
        name: m.name,
        namespace: m.namespace,
        reference,
        min_replicas: spec.and_then(|s| s.min_replicas),
        max_replicas: spec.map(|s| s.max_replicas).unwrap_or(0),
        current_replicas: status.and_then(|s| s.current_replicas).unwrap_or(0),
        desired_replicas: status.map(|s| s.desired_replicas).unwrap_or(0),
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `PersistentVolumeClaim` into a [`PvcInfo`] DTO.
pub fn pvc_to_info(pvc: PersistentVolumeClaim) -> PvcInfo {
    let m = extract_common_metadata(&pvc.metadata);
    let spec = pvc.spec.as_ref();
    let status = pvc.status.as_ref();
    let access_modes = spec
        .and_then(|s| s.access_modes.as_ref())
        .map(|modes| modes.to_vec())
        .unwrap_or_default();
    let capacity = status
        .and_then(|s| s.capacity.as_ref())
        .and_then(|c| c.get("storage"))
        .map(|q| q.0.clone());
    PvcInfo {
        name: m.name,
        namespace: m.namespace,
        status: status
            .and_then(|s| s.phase.clone())
            .unwrap_or_else(|| "Unknown".to_string()),
        volume: spec.and_then(|s| s.volume_name.clone()),
        capacity,
        access_modes,
        storage_class: spec.and_then(|s| s.storage_class_name.clone()),
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `PersistentVolume` into a [`PvInfo`] DTO.
pub fn pv_to_info(pv: PersistentVolume) -> PvInfo {
    let m = extract_common_metadata(&pv.metadata);
    let spec = pv.spec.as_ref();
    let access_modes = spec
        .and_then(|s| s.access_modes.as_ref())
        .map(|modes| modes.to_vec())
        .unwrap_or_default();
    let capacity = spec
        .and_then(|s| s.capacity.as_ref())
        .and_then(|c| c.get("storage"))
        .map(|q| q.0.clone());
    let claim = spec.and_then(|s| s.claim_ref.as_ref()).map(|cr| {
        format!(
            "{}/{}",
            cr.namespace.as_deref().unwrap_or(""),
            cr.name.as_deref().unwrap_or("")
        )
    });
    PvInfo {
        name: m.name,
        capacity,
        access_modes,
        reclaim_policy: spec
            .and_then(|s| s.persistent_volume_reclaim_policy.clone())
            .unwrap_or_else(|| "Retain".to_string()),
        status: pv
            .status
            .as_ref()
            .and_then(|s| s.phase.clone())
            .unwrap_or_else(|| "Unknown".to_string()),
        claim,
        storage_class: spec.and_then(|s| s.storage_class_name.clone()),
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `StorageClass` into a [`StorageClassInfo`] DTO.
pub fn storage_class_to_info(sc: StorageClass) -> StorageClassInfo {
    let is_default = sc
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get("storageclass.kubernetes.io/is-default-class"))
        .map(|v| v == "true")
        .unwrap_or(false);
    let m = extract_common_metadata(&sc.metadata);
    StorageClassInfo {
        name: m.name,
        provisioner: sc.provisioner,
        reclaim_policy: sc.reclaim_policy,
        volume_binding_mode: sc.volume_binding_mode,
        allow_volume_expansion: sc.allow_volume_expansion.unwrap_or(false),
        is_default,
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts a raw `Namespace` into a [`NamespaceInfo`] DTO.
pub fn namespace_to_info(ns: Namespace) -> NamespaceInfo {
    let m = extract_common_metadata(&ns.metadata);
    NamespaceInfo {
        name: m.name,
        status: namespace_status_from_metadata(&ns.metadata),
        labels: ns.metadata.labels.clone().unwrap_or_default(),
        age: m.age,
        created_at: m.created_at,
    }
}

/// Converts namespace metadata into the canonical lightweight [`NamespaceInfo`] DTO.
pub fn namespace_metadata_to_info(ns: PartialObjectMeta<Namespace>) -> NamespaceInfo {
    let m = extract_common_metadata(&ns.metadata);
    NamespaceInfo {
        name: m.name,
        status: namespace_status_from_metadata(&ns.metadata),
        labels: ns.metadata.labels.clone().unwrap_or_default(),
        age: m.age,
        created_at: m.created_at,
    }
}

fn namespace_status_from_metadata(meta: &ObjectMeta) -> String {
    if meta.deletion_timestamp.is_some() {
        "Terminating".to_string()
    } else {
        "Active".to_string()
    }
}

/// Converts a raw `Event` into a [`K8sEventInfo`] DTO.
pub fn event_to_info(ev: k8s_openapi::api::core::v1::Event) -> K8sEventInfo {
    let m = extract_common_metadata(&ev.metadata);
    let last_seen = ev
        .last_timestamp
        .as_ref()
        .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0));
    let involved = format!(
        "{}/{}",
        ev.involved_object.kind.as_deref().unwrap_or(""),
        ev.involved_object.name.as_deref().unwrap_or("")
    );
    K8sEventInfo {
        name: m.name,
        namespace: m.namespace,
        reason: ev.reason.unwrap_or_default(),
        message: ev.message.unwrap_or_default(),
        type_: ev.type_.unwrap_or_else(|| "Normal".to_string()),
        count: ev.count.unwrap_or(1),
        involved_object: involved,
        last_seen,
        age: m.age,
    }
}

/// Converts a raw `PriorityClass` into a [`PriorityClassInfo`] DTO.
pub fn priority_class_to_info(pc: PriorityClass) -> PriorityClassInfo {
    let m = extract_common_metadata(&pc.metadata);
    PriorityClassInfo {
        name: m.name,
        value: pc.value,
        global_default: pc.global_default.unwrap_or(false),
        description: pc.description.unwrap_or_default(),
        age: m.age,
        created_at: m.created_at,
    }
}

pub(crate) fn rule_from_policy_rule(rule: &PolicyRule) -> RbacRule {
    RbacRule {
        verbs: rule.verbs.clone(),
        api_groups: rule.api_groups.clone().unwrap_or_default(),
        resources: rule.resources.clone().unwrap_or_default(),
        resource_names: rule.resource_names.clone().unwrap_or_default(),
        non_resource_urls: rule.non_resource_urls.clone().unwrap_or_default(),
    }
}

pub(crate) fn subject_from_k8s(subject: &Subject) -> RoleBindingSubject {
    RoleBindingSubject {
        kind: subject.kind.clone(),
        name: subject.name.clone(),
        namespace: subject.namespace.clone(),
        api_group: subject.api_group.clone(),
    }
}

pub(crate) fn quota_percent_used(
    hard: &BTreeMap<String, String>,
    used: &BTreeMap<String, String>,
) -> BTreeMap<String, f64> {
    hard.iter()
        .filter_map(|(key, hard_value)| {
            let used_value = used.get(key)?;
            let used_num = parse_k8s_quantity(used_value)?;
            let hard_num = parse_k8s_quantity(hard_value)?;
            if hard_num <= 0.0 {
                return None;
            }
            Some((key.clone(), (used_num / hard_num) * 100.0))
        })
        .collect()
}

pub(crate) fn quantity_map_to_string_map(
    value: Option<BTreeMap<String, k8s_openapi::apimachinery::pkg::api::resource::Quantity>>,
) -> BTreeMap<String, String> {
    value
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, v.0))
        .collect()
}

pub(crate) fn int_or_string_to_string(value: &IntOrString) -> String {
    match value {
        IntOrString::Int(v) => v.to_string(),
        IntOrString::String(v) => v.clone(),
    }
}

pub(crate) fn parse_k8s_quantity(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let factors = [
        ("Ki", 1024.0),
        ("Mi", 1024.0_f64.powi(2)),
        ("Gi", 1024.0_f64.powi(3)),
        ("Ti", 1024.0_f64.powi(4)),
        ("Pi", 1024.0_f64.powi(5)),
        ("Ei", 1024.0_f64.powi(6)),
        ("n", 1e-9),
        ("u", 1e-6),
        ("m", 1e-3),
        ("K", 1e3),
        ("M", 1e6),
        ("G", 1e9),
        ("T", 1e12),
        ("P", 1e15),
        ("E", 1e18),
    ];

    for (suffix, factor) in factors {
        if let Some(number) = raw.strip_suffix(suffix) {
            let value = number.trim().parse::<f64>().ok()?;
            return Some(value * factor);
        }
    }

    raw.parse::<f64>().ok()
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
    use kube::core::PartialObjectMeta;
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
    fn namespace_to_info_uses_metadata_status_rules() {
        let info = namespace_to_info(Namespace {
            metadata: ObjectMeta {
                name: Some("default".to_string()),
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(info.name, "default");
        assert_eq!(info.status, "Active");
    }

    #[test]
    fn namespace_metadata_to_info_marks_terminating_namespaces() {
        let info = namespace_metadata_to_info(PartialObjectMeta::<Namespace> {
            metadata: ObjectMeta {
                name: Some("staging".to_string()),
                deletion_timestamp: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(
                    now(),
                )),
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(info.name, "staging");
        assert_eq!(info.status, "Terminating");
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
