//! Shared DTO conversion functions for Kubernetes API objects.
//!
//! These conversions are used by both the polling path (`client.rs`) and
//! the watch path (`state/watch.rs`) to produce identical typed DTOs.

use k8s_openapi::api::core::v1::Pod;

use crate::k8s::dtos::{OwnerRefInfo, PodInfo};
use crate::state::alerts::{format_mib, format_millicores, parse_mib, parse_millicores};

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
