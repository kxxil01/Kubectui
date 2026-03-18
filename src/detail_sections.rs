//! Detail metadata and section builders for the resource detail overlay.

use std::collections::BTreeMap;

use crate::app::{DetailMetadata, ResourceRef};
use crate::cronjob::CRONJOB_NEXT_RUN_TIMEZONE_FALLBACK;
use crate::k8s::dtos::FluxResourceInfo;
use crate::state::ClusterSnapshot;
use crate::time::{AppTimestamp, format_local, format_rfc3339, format_utc};

fn find_flux_resource<'a>(
    snapshot: &'a ClusterSnapshot,
    name: &str,
    namespace: &Option<String>,
    group: &str,
    kind: &str,
) -> Option<&'a FluxResourceInfo> {
    snapshot
        .flux_resources
        .iter()
        .find(|f| f.name == name && f.namespace == *namespace && f.group == group && f.kind == kind)
}

fn format_detail_time(ts: Option<AppTimestamp>) -> String {
    ts.map(|value| format_local(value, "%Y-%m-%d %H:%M:%S"))
        .unwrap_or_else(|| "N/A".to_string())
}

fn cronjob_timezone_label(timezone: Option<&str>) -> String {
    timezone
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            format!("controller default ({CRONJOB_NEXT_RUN_TIMEZONE_FALLBACK} estimate)")
        })
}

pub fn metadata_for_resource(snapshot: &ClusterSnapshot, resource: &ResourceRef) -> DetailMetadata {
    match resource {
        ResourceRef::Node(name) => {
            if let Some(node) = snapshot.nodes.iter().find(|n| &n.name == name) {
                DetailMetadata {
                    name: node.name.clone(),
                    namespace: None,
                    status: Some(if node.ready { "Ready" } else { "NotReady" }.to_string()),
                    node_unschedulable: Some(node.unschedulable),
                    node: Some(node.name.clone()),
                    ip: None,
                    created: None,
                    labels: Vec::new(),
                    ..DetailMetadata::default()
                }
            } else {
                DetailMetadata {
                    name: name.clone(),
                    ..DetailMetadata::default()
                }
            }
        }
        ResourceRef::Pod(name, ns) => {
            if let Some(pod) = snapshot
                .pods
                .iter()
                .find(|p| &p.name == name && &p.namespace == ns)
            {
                DetailMetadata {
                    name: pod.name.clone(),
                    namespace: Some(pod.namespace.clone()),
                    status: Some(pod.status.clone()),
                    node: pod.node.clone(),
                    ip: pod.pod_ip.clone(),
                    created: pod.created_at.map(format_rfc3339),
                    labels: pod.labels.clone(),
                    annotations: pod.annotations.clone(),
                    owner_references: pod.owner_references.clone(),
                    ..DetailMetadata::default()
                }
            } else {
                DetailMetadata {
                    name: name.clone(),
                    namespace: Some(ns.clone()),
                    ..DetailMetadata::default()
                }
            }
        }
        ResourceRef::Service(name, ns) => DetailMetadata {
            name: name.clone(),
            namespace: Some(ns.clone()),
            status: Some("Active".to_string()),
            ..DetailMetadata::default()
        },
        ResourceRef::Deployment(name, ns) => {
            let status = snapshot
                .deployments
                .iter()
                .find(|d| &d.name == name && &d.namespace == ns)
                .map(|d| format!("Ready {}/{}", d.ready_replicas, d.desired_replicas));

            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::StatefulSet(name, ns) => {
            let status = snapshot
                .statefulsets
                .iter()
                .find(|ss| &ss.name == name && &ss.namespace == ns)
                .map(|ss| format!("Ready {}/{}", ss.ready_replicas, ss.desired_replicas));

            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::ResourceQuota(name, ns) => {
            let status = snapshot
                .resource_quotas
                .iter()
                .find(|rq| &rq.name == name && &rq.namespace == ns)
                .map(|rq| {
                    let max_pct = rq
                        .percent_used
                        .values()
                        .fold(0.0_f64, |acc, value| acc.max(*value));
                    format!("Max usage {:.0}%", max_pct)
                });

            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::LimitRange(name, ns) => {
            let status = snapshot
                .limit_ranges
                .iter()
                .find(|lr| &lr.name == name && &lr.namespace == ns)
                .map(|lr| format!("{} limit specs", lr.limits.len()));

            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::PodDisruptionBudget(name, ns) => {
            let status = snapshot
                .pod_disruption_budgets
                .iter()
                .find(|pdb| &pdb.name == name && &pdb.namespace == ns)
                .map(|pdb| format!("Healthy {}/{}", pdb.current_healthy, pdb.desired_healthy));

            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::DaemonSet(name, ns) => {
            let status = snapshot
                .daemonsets
                .iter()
                .find(|ds| &ds.name == name && &ds.namespace == ns)
                .map(|ds| format!("Ready {}/{}", ds.ready_count, ds.desired_count));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::ReplicaSet(name, ns) => {
            let status = snapshot
                .replicasets
                .iter()
                .find(|rs| &rs.name == name && &rs.namespace == ns)
                .map(|rs| format!("Ready {}/{}", rs.ready, rs.desired));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::ReplicationController(name, ns) => {
            let status = snapshot
                .replication_controllers
                .iter()
                .find(|rc| &rc.name == name && &rc.namespace == ns)
                .map(|rc| format!("Ready {}/{}", rc.ready, rc.desired));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::Job(name, ns) => {
            let status = snapshot
                .jobs
                .iter()
                .find(|j| &j.name == name && &j.namespace == ns)
                .map(|j| j.status.clone());
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::CronJob(name, ns) => {
            let status = snapshot
                .cronjobs
                .iter()
                .find(|cj| &cj.name == name && &cj.namespace == ns)
                .map(|cj| {
                    if cj.suspend {
                        "Paused".to_string()
                    } else if cj.active_jobs > 0 {
                        format!("Active · {} running", cj.active_jobs)
                    } else {
                        "Active".to_string()
                    }
                });
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                cronjob_suspended: snapshot
                    .cronjobs
                    .iter()
                    .find(|cj| &cj.name == name && &cj.namespace == ns)
                    .map(|cj| cj.suspend),
                ..DetailMetadata::default()
            }
        }
        ResourceRef::Endpoint(name, ns) => DetailMetadata {
            name: name.clone(),
            namespace: Some(ns.clone()),
            status: Some("Active".to_string()),
            ..DetailMetadata::default()
        },
        ResourceRef::Ingress(name, ns) => {
            let status = snapshot
                .ingresses
                .iter()
                .find(|i| &i.name == name && &i.namespace == ns)
                .and_then(|i| i.address.clone());
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::IngressClass(name) => DetailMetadata {
            name: name.clone(),
            namespace: None,
            status: Some("Active".to_string()),
            ..DetailMetadata::default()
        },
        ResourceRef::NetworkPolicy(name, ns) => DetailMetadata {
            name: name.clone(),
            namespace: Some(ns.clone()),
            status: Some("Active".to_string()),
            ..DetailMetadata::default()
        },
        ResourceRef::ConfigMap(name, ns) => {
            let status = snapshot
                .config_maps
                .iter()
                .find(|cm| &cm.name == name && &cm.namespace == ns)
                .map(|cm| format!("{} keys", cm.data_count));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::Secret(name, ns) => {
            let status = snapshot
                .secrets
                .iter()
                .find(|s| &s.name == name && &s.namespace == ns)
                .map(|s| format!("{} ({} keys)", s.type_, s.data_count));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::Hpa(name, ns) => {
            let status = snapshot
                .hpas
                .iter()
                .find(|h| &h.name == name && &h.namespace == ns)
                .map(|h| format!("{}/{} replicas", h.current_replicas, h.max_replicas));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::PriorityClass(name) => {
            let status = snapshot
                .priority_classes
                .iter()
                .find(|pc| &pc.name == name)
                .map(|pc| format!("value: {}", pc.value));
            DetailMetadata {
                name: name.clone(),
                namespace: None,
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::Pvc(name, ns) => {
            let status = snapshot
                .pvcs
                .iter()
                .find(|pvc| &pvc.name == name && &pvc.namespace == ns)
                .map(|pvc| pvc.status.clone());
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::Pv(name) => {
            let status = snapshot
                .pvs
                .iter()
                .find(|pv| &pv.name == name)
                .map(|pv| pv.status.clone());
            DetailMetadata {
                name: name.clone(),
                namespace: None,
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::StorageClass(name) => DetailMetadata {
            name: name.clone(),
            namespace: None,
            status: Some("Active".to_string()),
            ..DetailMetadata::default()
        },
        ResourceRef::Namespace(name) => {
            let status = snapshot
                .namespace_list
                .iter()
                .find(|ns| &ns.name == name)
                .map(|ns| ns.status.clone());
            DetailMetadata {
                name: name.clone(),
                namespace: None,
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::Event(name, ns) => {
            let status = snapshot
                .events
                .iter()
                .find(|ev| &ev.name == name && &ev.namespace == ns)
                .map(|ev| ev.reason.clone());
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::ServiceAccount(name, ns) => DetailMetadata {
            name: name.clone(),
            namespace: Some(ns.clone()),
            status: Some("Active".to_string()),
            ..DetailMetadata::default()
        },
        ResourceRef::Role(name, ns) => {
            let status = snapshot
                .roles
                .iter()
                .find(|r| &r.name == name && &r.namespace == ns)
                .map(|r| format!("{} rules", r.rules.len()));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::RoleBinding(name, ns) => {
            let status = snapshot
                .role_bindings
                .iter()
                .find(|rb| &rb.name == name && &rb.namespace == ns)
                .map(|rb| format!("-> {}/{}", rb.role_ref_kind, rb.role_ref_name));
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::ClusterRole(name) => {
            let status = snapshot
                .cluster_roles
                .iter()
                .find(|cr| &cr.name == name)
                .map(|cr| format!("{} rules", cr.rules.len()));
            DetailMetadata {
                name: name.clone(),
                namespace: None,
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::ClusterRoleBinding(name) => {
            let status = snapshot
                .cluster_role_bindings
                .iter()
                .find(|crb| &crb.name == name)
                .map(|crb| format!("-> {}/{}", crb.role_ref_kind, crb.role_ref_name));
            DetailMetadata {
                name: name.clone(),
                namespace: None,
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::HelmRelease(name, ns) => {
            let status = snapshot
                .helm_releases
                .iter()
                .find(|r| &r.name == name && &r.namespace == ns)
                .map(|r| r.status.clone());
            DetailMetadata {
                name: name.clone(),
                namespace: Some(ns.clone()),
                status,
                ..DetailMetadata::default()
            }
        }
        ResourceRef::CustomResource {
            name,
            namespace,
            kind,
            group,
            ..
        } => {
            if let Some(flux) = find_flux_resource(snapshot, name, namespace, group, kind) {
                DetailMetadata {
                    name: name.clone(),
                    namespace: namespace.clone(),
                    status: Some(flux.status.clone()),
                    created: flux.created_at.map(format_rfc3339),
                    flux_reconcile_enabled: !flux.suspended,
                    ..DetailMetadata::default()
                }
            } else {
                DetailMetadata {
                    name: name.clone(),
                    namespace: namespace.clone(),
                    status: Some(format!("{kind}.{group}")),
                    ..DetailMetadata::default()
                }
            }
        }
    }
}

pub fn sections_for_resource(snapshot: &ClusterSnapshot, resource: &ResourceRef) -> Vec<String> {
    match resource {
        ResourceRef::Node(name) => snapshot
            .nodes
            .iter()
            .find(|n| &n.name == name)
            .map(|node| {
                vec![
                    format!("Kubelet: {}", node.kubelet_version),
                    format!("OS Image: {}", node.os_image),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Pod(name, ns) => snapshot
            .pods
            .iter()
            .find(|p| &p.name == name && &p.namespace == ns)
            .map(|pod| {
                vec![
                    "CONTAINERS".to_string(),
                    format!("- restarts: {}", pod.restarts),
                    format!("- node: {}", pod.node.as_deref().unwrap_or("n/a")),
                    format!("- pod IP: {}", pod.pod_ip.as_deref().unwrap_or("n/a")),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Service(name, ns) => snapshot
            .services
            .iter()
            .find(|s| &s.name == name && &s.namespace == ns)
            .map(|svc| {
                vec![
                    "PORTS".to_string(),
                    if svc.ports.is_empty() {
                        "- none".to_string()
                    } else {
                        format!("- {}", svc.ports.join(", "))
                    },
                    format!("type: {}", svc.type_),
                    format!(
                        "cluster IP: {}",
                        svc.cluster_ip.as_deref().unwrap_or("None")
                    ),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Deployment(name, ns) => snapshot
            .deployments
            .iter()
            .find(|d| &d.name == name && &d.namespace == ns)
            .map(|dep| {
                vec![
                    "REPLICAS".to_string(),
                    format!("desired: {}", dep.desired_replicas),
                    format!("ready: {}", dep.ready_replicas),
                    format!("available: {}", dep.available_replicas),
                    format!("updated: {}", dep.updated_replicas),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::StatefulSet(name, ns) => snapshot
            .statefulsets
            .iter()
            .find(|ss| &ss.name == name && &ss.namespace == ns)
            .map(|ss| {
                vec![
                    "REPLICAS".to_string(),
                    format!("desired: {}", ss.desired_replicas),
                    format!("ready: {}", ss.ready_replicas),
                    format!("service: {}", ss.service_name),
                    format!("pod management: {}", ss.pod_management_policy),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::ResourceQuota(name, ns) => snapshot
            .resource_quotas
            .iter()
            .find(|rq| &rq.name == name && &rq.namespace == ns)
            .map(|rq| {
                let mut lines = vec!["QUOTAS".to_string()];
                for (key, hard) in rq.hard.iter().take(12) {
                    let used = rq.used.get(key).cloned().unwrap_or_else(|| "-".to_string());
                    let pct = rq
                        .percent_used
                        .get(key)
                        .map(|v| format!(" ({v:.0}%)"))
                        .unwrap_or_default();
                    lines.push(format!("{key}: {used}/{hard}{pct}"));
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::LimitRange(name, ns) => snapshot
            .limit_ranges
            .iter()
            .find(|lr| &lr.name == name && &lr.namespace == ns)
            .map(|lr| {
                let mut lines = vec!["LIMIT SPECS".to_string()];
                for spec in lr.limits.iter().take(8) {
                    lines.push(format!("type: {}", spec.type_));
                    if !spec.default.is_empty() {
                        lines.push(format!("  default: {}", map_to_kv(&spec.default)));
                    }
                    if !spec.default_request.is_empty() {
                        lines.push(format!(
                            "  defaultRequest: {}",
                            map_to_kv(&spec.default_request)
                        ));
                    }
                    if !spec.min.is_empty() {
                        lines.push(format!("  min: {}", map_to_kv(&spec.min)));
                    }
                    if !spec.max.is_empty() {
                        lines.push(format!("  max: {}", map_to_kv(&spec.max)));
                    }
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::PodDisruptionBudget(name, ns) => snapshot
            .pod_disruption_budgets
            .iter()
            .find(|pdb| &pdb.name == name && &pdb.namespace == ns)
            .map(|pdb| {
                vec![
                    "AVAILABILITY".to_string(),
                    format!(
                        "minAvailable: {}",
                        pdb.min_available.as_deref().unwrap_or("-")
                    ),
                    format!(
                        "maxUnavailable: {}",
                        pdb.max_unavailable.as_deref().unwrap_or("-")
                    ),
                    format!("currentHealthy: {}", pdb.current_healthy),
                    format!("desiredHealthy: {}", pdb.desired_healthy),
                    format!("disruptionsAllowed: {}", pdb.disruptions_allowed),
                    format!("expectedPods: {}", pdb.expected_pods),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::DaemonSet(name, ns) => snapshot
            .daemonsets
            .iter()
            .find(|ds| &ds.name == name && &ds.namespace == ns)
            .map(|ds| {
                vec![
                    "STATUS".to_string(),
                    format!("desired: {}", ds.desired_count),
                    format!("ready: {}", ds.ready_count),
                    format!("unavailable: {}", ds.unavailable_count),
                    format!("updateStrategy: {}", ds.update_strategy),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::ReplicaSet(name, ns) => snapshot
            .replicasets
            .iter()
            .find(|rs| &rs.name == name && &rs.namespace == ns)
            .map(|rs| {
                vec![
                    "REPLICAS".to_string(),
                    format!("desired: {}", rs.desired),
                    format!("ready: {}", rs.ready),
                    format!("available: {}", rs.available),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::ReplicationController(name, ns) => snapshot
            .replication_controllers
            .iter()
            .find(|rc| &rc.name == name && &rc.namespace == ns)
            .map(|rc| {
                vec![
                    "REPLICAS".to_string(),
                    format!("desired: {}", rc.desired),
                    format!("ready: {}", rc.ready),
                    format!("available: {}", rc.available),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Job(name, ns) => snapshot
            .jobs
            .iter()
            .find(|j| &j.name == name && &j.namespace == ns)
            .map(|j| {
                vec![
                    "JOB STATUS".to_string(),
                    format!("status: {}", j.status),
                    format!("completions: {}", j.completions),
                    format!("parallelism: {}", j.parallelism),
                    format!("active: {}", j.active_pods),
                    format!("failed: {}", j.failed_pods),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::CronJob(name, ns) => snapshot
            .cronjobs
            .iter()
            .find(|cj| &cj.name == name && &cj.namespace == ns)
            .map(|cj| {
                vec![
                    "SCHEDULE".to_string(),
                    format!("schedule: {}", cj.schedule),
                    format!(
                        "timezone: {}",
                        cronjob_timezone_label(cj.timezone.as_deref())
                    ),
                    format!("state: {}", if cj.suspend { "paused" } else { "active" }),
                    format!("active: {}", cj.active_jobs),
                    format!(
                        "nextSchedule: {}",
                        format_detail_time(cj.next_schedule_time)
                    ),
                    format!(
                        "lastSchedule: {}",
                        format_detail_time(cj.last_schedule_time)
                    ),
                    format!(
                        "lastSuccess: {}",
                        format_detail_time(cj.last_successful_time)
                    ),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Endpoint(name, ns) => snapshot
            .endpoints
            .iter()
            .find(|e| &e.name == name && &e.namespace == ns)
            .map(|e| {
                let mut lines = vec!["ADDRESSES".to_string()];
                for addr in e.addresses.iter().take(10) {
                    lines.push(format!("- {addr}"));
                }
                if !e.ports.is_empty() {
                    lines.push("PORTS".to_string());
                    for port in e.ports.iter().take(10) {
                        lines.push(format!("- {port}"));
                    }
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::Ingress(name, ns) => snapshot
            .ingresses
            .iter()
            .find(|i| &i.name == name && &i.namespace == ns)
            .map(|i| {
                let mut lines = vec!["RULES".to_string()];
                for host in i.hosts.iter().take(10) {
                    lines.push(format!("- {host}"));
                }
                if let Some(addr) = &i.address {
                    lines.push(format!("address: {addr}"));
                }
                if let Some(class) = &i.class {
                    lines.push(format!("class: {class}"));
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::IngressClass(name) => snapshot
            .ingress_classes
            .iter()
            .find(|ic| &ic.name == name)
            .map(|ic| {
                vec![
                    format!("controller: {}", ic.controller),
                    format!("default: {}", ic.is_default),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::NetworkPolicy(name, ns) => snapshot
            .network_policies
            .iter()
            .find(|np| &np.name == name && &np.namespace == ns)
            .map(|np| {
                vec![
                    format!("podSelector: {}", np.pod_selector),
                    format!("ingressRules: {}", np.ingress_rules),
                    format!("egressRules: {}", np.egress_rules),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::ConfigMap(name, ns) => snapshot
            .config_maps
            .iter()
            .find(|cm| &cm.name == name && &cm.namespace == ns)
            .map(|cm| vec![format!("keys: {}", cm.data_count)])
            .unwrap_or_default(),
        ResourceRef::Secret(name, ns) => snapshot
            .secrets
            .iter()
            .find(|s| &s.name == name && &s.namespace == ns)
            .map(|s| {
                vec![
                    format!("type: {}", s.type_),
                    format!("keys: {}", s.data_count),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Hpa(name, ns) => snapshot
            .hpas
            .iter()
            .find(|h| &h.name == name && &h.namespace == ns)
            .map(|h| {
                vec![
                    format!("reference: {}", h.reference),
                    format!("minReplicas: {}", h.min_replicas.unwrap_or(1)),
                    format!("maxReplicas: {}", h.max_replicas),
                    format!("currentReplicas: {}", h.current_replicas),
                    format!("desiredReplicas: {}", h.desired_replicas),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::PriorityClass(name) => snapshot
            .priority_classes
            .iter()
            .find(|pc| &pc.name == name)
            .map(|pc| {
                vec![
                    format!("value: {}", pc.value),
                    format!("globalDefault: {}", pc.global_default),
                    format!("description: {}", pc.description),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Pvc(name, ns) => snapshot
            .pvcs
            .iter()
            .find(|pvc| &pvc.name == name && &pvc.namespace == ns)
            .map(|pvc| {
                vec![
                    format!("status: {}", pvc.status),
                    format!("capacity: {}", pvc.capacity.as_deref().unwrap_or("-")),
                    format!("accessModes: {}", pvc.access_modes.join(", ")),
                    format!(
                        "storageClass: {}",
                        pvc.storage_class.as_deref().unwrap_or("-")
                    ),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Pv(name) => snapshot
            .pvs
            .iter()
            .find(|pv| &pv.name == name)
            .map(|pv| {
                vec![
                    format!("status: {}", pv.status),
                    format!("capacity: {}", pv.capacity.as_deref().unwrap_or("-")),
                    format!("accessModes: {}", pv.access_modes.join(", ")),
                    format!("reclaimPolicy: {}", pv.reclaim_policy),
                    format!("claim: {}", pv.claim.as_deref().unwrap_or("-")),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::StorageClass(name) => snapshot
            .storage_classes
            .iter()
            .find(|sc| &sc.name == name)
            .map(|sc| {
                vec![
                    format!("provisioner: {}", sc.provisioner),
                    format!(
                        "reclaimPolicy: {}",
                        sc.reclaim_policy.as_deref().unwrap_or("-")
                    ),
                    format!(
                        "volumeBindingMode: {}",
                        sc.volume_binding_mode.as_deref().unwrap_or("-")
                    ),
                    format!("allowVolumeExpansion: {}", sc.allow_volume_expansion),
                    format!("default: {}", sc.is_default),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Namespace(name) => snapshot
            .namespace_list
            .iter()
            .find(|ns| &ns.name == name)
            .map(|ns| vec![format!("status: {}", ns.status)])
            .unwrap_or_default(),
        ResourceRef::Event(name, ns) => snapshot
            .events
            .iter()
            .find(|ev| &ev.name == name && &ev.namespace == ns)
            .map(|ev| {
                vec![
                    format!("reason: {}", ev.reason),
                    format!("type: {}", ev.type_),
                    format!("count: {}", ev.count),
                    format!("object: {}", ev.involved_object),
                    format!("message: {}", ev.message),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::ServiceAccount(name, ns) => snapshot
            .service_accounts
            .iter()
            .find(|sa| &sa.name == name && &sa.namespace == ns)
            .map(|sa| {
                vec![
                    format!("secrets: {}", sa.secrets_count),
                    format!("imagePullSecrets: {}", sa.image_pull_secrets_count),
                    format!(
                        "automountToken: {}",
                        sa.automount_service_account_token
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "unset".to_string())
                    ),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::Role(name, ns) => snapshot
            .roles
            .iter()
            .find(|r| &r.name == name && &r.namespace == ns)
            .map(|r| {
                let mut lines = vec![format!("rules: {}", r.rules.len())];
                for rule in r.rules.iter().take(5) {
                    lines.push(format!(
                        "  {} on {}",
                        rule.verbs.join(","),
                        rule.resources.join(",")
                    ));
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::RoleBinding(name, ns) => snapshot
            .role_bindings
            .iter()
            .find(|rb| &rb.name == name && &rb.namespace == ns)
            .map(|rb| {
                let mut lines = vec![
                    format!("roleRef: {}/{}", rb.role_ref_kind, rb.role_ref_name),
                    format!("subjects: {}", rb.subjects.len()),
                ];
                for subj in rb.subjects.iter().take(5) {
                    lines.push(format!("  {} {}", subj.kind, subj.name));
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::ClusterRole(name) => snapshot
            .cluster_roles
            .iter()
            .find(|cr| &cr.name == name)
            .map(|cr| {
                let mut lines = vec![format!("rules: {}", cr.rules.len())];
                for rule in cr.rules.iter().take(5) {
                    lines.push(format!(
                        "  {} on {}",
                        rule.verbs.join(","),
                        rule.resources.join(",")
                    ));
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::ClusterRoleBinding(name) => snapshot
            .cluster_role_bindings
            .iter()
            .find(|crb| &crb.name == name)
            .map(|crb| {
                let mut lines = vec![
                    format!("roleRef: {}/{}", crb.role_ref_kind, crb.role_ref_name),
                    format!("subjects: {}", crb.subjects.len()),
                ];
                for subj in crb.subjects.iter().take(5) {
                    lines.push(format!("  {} {}", subj.kind, subj.name));
                }
                lines
            })
            .unwrap_or_default(),
        ResourceRef::HelmRelease(name, ns) => snapshot
            .helm_releases
            .iter()
            .find(|r| &r.name == name && &r.namespace == ns)
            .map(|r| {
                let updated = r
                    .updated
                    .map(|ts| format_utc(ts, "%Y-%m-%d %H:%M:%S"))
                    .unwrap_or_else(|| "-".to_string());
                vec![
                    "HELM RELEASE".to_string(),
                    format!("chart: {}", r.chart),
                    format!("chartVersion: {}", r.chart_version),
                    format!("appVersion: {}", r.app_version),
                    format!("revision: {}", r.revision),
                    format!("status: {}", r.status),
                    format!("updated: {updated}"),
                ]
            })
            .unwrap_or_default(),
        ResourceRef::CustomResource {
            name,
            namespace,
            kind,
            group,
            version,
            ..
        } => {
            if let Some(flux) = find_flux_resource(snapshot, name, namespace, group, kind) {
                flux_detail_sections(flux)
            } else {
                vec![
                    "CUSTOM RESOURCE".to_string(),
                    format!("kind: {kind}"),
                    format!("apiVersion: {group}/{version}"),
                ]
            }
        }
    }
}

fn flux_detail_sections(flux: &FluxResourceInfo) -> Vec<String> {
    let mut lines = Vec::new();

    lines.push("RECONCILIATION".to_string());
    if let Some(ref sr) = flux.source_ref {
        lines.push(format!("source: {sr}"));
    }
    if let Some(ref interval) = flux.interval {
        lines.push(format!("interval: {interval}"));
    }
    if let Some(ref timeout) = flux.timeout {
        lines.push(format!("timeout: {timeout}"));
    }
    if let Some(ts) = flux.last_reconcile_time {
        lines.push(format!(
            "last reconcile: {}",
            format_utc(ts, "%Y-%m-%d %H:%M:%S UTC")
        ));
    }
    if flux.suspended {
        lines.push("suspended: true".to_string());
    }

    if flux.last_applied_revision.is_some() || flux.last_attempted_revision.is_some() {
        lines.push(String::new());
        lines.push("REVISIONS".to_string());
        if let Some(ref rev) = flux.last_applied_revision {
            let display = if rev.len() > 48 {
                &rev[..rev.floor_char_boundary(48)]
            } else {
                rev
            };
            lines.push(format!("applied: {display}"));
        }
        if let Some(ref rev) = flux.last_attempted_revision {
            let display = if rev.len() > 48 {
                &rev[..rev.floor_char_boundary(48)]
            } else {
                rev
            };
            lines.push(format!("attempted: {display}"));
        }
    }

    if flux.generation.is_some() || flux.observed_generation.is_some() {
        lines.push(String::new());
        lines.push("GENERATION".to_string());
        let cur = flux
            .generation
            .map(|g| g.to_string())
            .unwrap_or_else(|| "-".to_string());
        let obs = flux
            .observed_generation
            .map(|g| g.to_string())
            .unwrap_or_else(|| "-".to_string());
        let sync = if flux.observed_generation == flux.generation {
            "in sync"
        } else {
            "OUTDATED"
        };
        lines.push(format!("current: {cur}  observed: {obs}  ({sync})"));
    }

    if let Some(ref artifact) = flux.artifact {
        lines.push(String::new());
        lines.push("ARTIFACT".to_string());
        lines.push(artifact.clone());
    }

    if !flux.conditions.is_empty() {
        lines.push(String::new());
        lines.push("CONDITIONS".to_string());
        for cond in &flux.conditions {
            let ts = cond
                .timestamp
                .map(|t| format_utc(t, "%H:%M:%S"))
                .unwrap_or_else(|| "-".to_string());
            let reason = cond.reason.as_deref().unwrap_or("-");
            let msg = cond.message.as_deref().unwrap_or("");
            let msg_display = if msg.len() > 60 {
                let end = msg.floor_char_boundary(57);
                format!("{}...", &msg[..end])
            } else {
                msg.to_string()
            };
            lines.push(format!(
                "  {}={} ({reason}) [{ts}] {msg_display}",
                cond.type_, cond.status
            ));
        }
    }

    lines
}

fn map_to_kv(map: &BTreeMap<String, String>) -> String {
    map.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(", ")
}
