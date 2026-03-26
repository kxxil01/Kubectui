//! Snapshot-only service and traffic debugging analysis.

use std::collections::BTreeSet;

use crate::{
    app::ResourceRef,
    k8s::{
        dtos::{
            EndpointInfo, IngressInfo, IngressRouteInfo, PodInfo, ServiceInfo, ServicePortInfo,
        },
        portforward::{PortForwardTunnelInfo, TunnelState},
        relationships::{RelationKind, RelationNode},
    },
    network_policy_semantics::{policy_applies_to_ingress, policy_selects_pod},
    state::{ClusterSnapshot, port_forward::TunnelRegistry},
};

const MAX_RENDERED_BACKENDS: usize = 40;
const MAX_RENDERED_TUNNELS: usize = 20;

#[derive(Debug, Clone)]
pub struct TrafficDebugAnalysis {
    pub summary_lines: Vec<String>,
    pub tree: Vec<RelationNode>,
}

pub fn analyze_resource(
    resource: &ResourceRef,
    snapshot: &ClusterSnapshot,
    tunnels: &TunnelRegistry,
) -> Result<TrafficDebugAnalysis, String> {
    match resource {
        ResourceRef::Service(name, namespace) => {
            let service = find_service(snapshot, name, namespace)?;
            Ok(analyze_service(service, snapshot, tunnels))
        }
        ResourceRef::Ingress(name, namespace) => {
            let ingress = find_ingress(snapshot, name, namespace)?;
            Ok(analyze_ingress(ingress, snapshot, tunnels))
        }
        ResourceRef::Endpoint(name, namespace) => {
            let endpoint = find_endpoint(snapshot, name, namespace)?;
            Ok(analyze_endpoint(endpoint, snapshot, tunnels))
        }
        ResourceRef::Pod(name, namespace) => {
            let pod = find_pod(snapshot, name, namespace)?;
            Ok(analyze_pod(pod, snapshot, tunnels))
        }
        _ => Err(
            "Traffic debugging is available for Services, Endpoints, Ingresses, and Pods."
                .to_string(),
        ),
    }
}

fn analyze_service(
    service: &ServiceInfo,
    snapshot: &ClusterSnapshot,
    tunnels: &TunnelRegistry,
) -> TrafficDebugAnalysis {
    let endpoint = snapshot
        .endpoints
        .iter()
        .find(|ep| ep.name == service.name && ep.namespace == service.namespace);
    let backends = service_backends(service, snapshot);
    let route_refs = ingress_routes_for_service(snapshot, &service.namespace, &service.name);
    let tunnel_refs = tunnel_refs_for_service(service, &backends, tunnels);
    let isolated = isolated_ingress_backend_count(&backends, snapshot);

    let resolution_summary = if let Some(external_name) = &service.external_name {
        format!(
            "ExternalName service: resolves to {external_name}; no pod backends are expected in-cluster."
        )
    } else if service.selector.is_empty() {
        format!(
            "Selectorless service: Endpoints publishes {} address(es); {} of them currently map to pod IPs in the snapshot.",
            endpoint.map_or(0, |ep| ep.addresses.len()),
            backends.len()
        )
    } else {
        format!(
            "Backend resolution: selector matches {} pod(s); Endpoints publishes {} address(es).",
            backends.len(),
            endpoint.map_or(0, |ep| ep.addresses.len())
        )
    };

    let mut summary_lines = vec![
        format!(
            "Service {}/{} type={} {}.",
            service.namespace,
            service.name,
            service.type_,
            service
                .cluster_ip
                .as_deref()
                .map(|ip| format!("clusterIP={ip}"))
                .unwrap_or_else(|| "headless or pending ClusterIP".to_string())
        ),
        resolution_summary,
        service_port_summary(service, &backends),
        format!(
            "Ingress routes: {}. Port-forward tunnels to backend pods: {}.",
            route_refs.len(),
            tunnel_refs.len()
        ),
    ];
    if !backends.is_empty() {
        summary_lines.push(format!(
            "Ingress NetworkPolicy isolation on backend pods: {isolated}/{} pod(s). Use [C] on a backend pod for exact policy intent.",
            backends.len()
        ));
    }
    if service.selector.is_empty() && service.external_name.is_none() {
        summary_lines.push(
            "Selectorless service: backend ownership is manual; verify the Endpoints object is curated intentionally."
                .to_string(),
        );
    }

    let mut tree = vec![
        section("DNS hints", service_dns_nodes(service)),
        backends_section_for_service(service, endpoint, &backends),
    ];
    if !route_refs.is_empty() {
        tree.push(section(
            "Ingress routes",
            route_refs
                .iter()
                .take(MAX_RENDERED_BACKENDS)
                .map(route_ref_node)
                .collect(),
        ));
    }
    tree.push(section(
        "Port-forward diagnostics",
        tunnel_nodes(
            &tunnel_refs,
            backends.len(),
            "No active port-forward targets these backend pods.",
        ),
    ));

    TrafficDebugAnalysis {
        summary_lines,
        tree,
    }
}

fn analyze_ingress(
    ingress: &IngressInfo,
    snapshot: &ClusterSnapshot,
    tunnels: &TunnelRegistry,
) -> TrafficDebugAnalysis {
    let route_refs = ingress_route_refs(snapshot, ingress);
    let resolved = route_refs
        .iter()
        .filter(|route| route.service.is_some())
        .count();
    let published_addresses = route_refs
        .iter()
        .filter_map(|route| route.endpoint.map(|ep| ep.addresses.len()))
        .sum::<usize>();
    let tunnel_refs = tunnel_refs_for_route_refs(&route_refs, tunnels);

    let mut summary_lines = vec![
        format!(
            "Ingress {}/{} class={} address={}.",
            ingress.namespace,
            ingress.name,
            ingress.class.as_deref().unwrap_or("default"),
            ingress.address.as_deref().unwrap_or("pending")
        ),
        format!(
            "Route audit: {} route(s), {} backend service(s) currently resolvable, {} published endpoint address(es).",
            ingress.routes.len(),
            resolved,
            published_addresses
        ),
        format!(
            "Host hints: {}. Port-forward tunnels touching backend pods: {}.",
            if ingress.hosts.is_empty() {
                "default backend / wildcard only".to_string()
            } else {
                ingress.hosts.join(", ")
            },
            tunnel_refs.len()
        ),
        "Ingress path tracing here is control-plane intent only; runtime load balancer, DNS, and CNI behavior can still diverge.".to_string(),
    ];
    if ingress.address.is_none() {
        summary_lines.push(
            "Ingress has no published address yet. Check controller provisioning, Service exposure, and external DNS separately."
                .to_string(),
        );
    }

    let mut tree = vec![section("Host and DNS hints", ingress_host_nodes(ingress))];
    tree.push(section(
        "Backend trace",
        route_refs
            .iter()
            .take(MAX_RENDERED_BACKENDS)
            .map(route_trace_node)
            .collect(),
    ));
    tree.push(section(
        "Port-forward diagnostics",
        tunnel_nodes(
            &tunnel_refs,
            route_refs.len(),
            "No active port-forward targets any backend pod on this ingress path.",
        ),
    ));

    TrafficDebugAnalysis {
        summary_lines,
        tree,
    }
}

fn analyze_endpoint(
    endpoint: &EndpointInfo,
    snapshot: &ClusterSnapshot,
    tunnels: &TunnelRegistry,
) -> TrafficDebugAnalysis {
    let service = snapshot
        .services
        .iter()
        .find(|svc| svc.name == endpoint.name && svc.namespace == endpoint.namespace);
    let backends = service
        .map(|svc| service_backends(svc, snapshot))
        .unwrap_or_default();
    let route_refs = ingress_routes_for_service(snapshot, &endpoint.namespace, &endpoint.name);
    let tunnel_refs = service
        .map(|svc| tunnel_refs_for_service(svc, &backends, tunnels))
        .unwrap_or_default();

    let mut summary_lines = vec![
        format!(
            "Endpoints {}/{} publishes {} address(es) across {} port entry/entries.",
            endpoint.namespace,
            endpoint.name,
            endpoint.addresses.len(),
            endpoint.ports.len()
        ),
        match service {
            Some(service) => format!(
                "Bound service: {}/{} type={} selector-pods={}.",
                service.namespace,
                service.name,
                service.type_,
                backends.len()
            ),
            None => "No matching Service object currently exists for this Endpoints resource."
                .to_string(),
        },
        format!(
            "Ingress routes through this endpoint set: {}. Matching port-forward tunnels: {}.",
            route_refs.len(),
            tunnel_refs.len()
        ),
    ];
    if service.is_none() {
        summary_lines.push(
            "Manual Endpoints without a matching Service usually indicate stale control-plane state or out-of-band reconciliation."
                .to_string(),
        );
    }

    let mut tree = vec![section(
        "Published addresses",
        endpoint_address_nodes(endpoint),
    )];
    if let Some(service) = service {
        tree.push(backends_section_for_service(
            service,
            Some(endpoint),
            &backends,
        ));
    }
    if !route_refs.is_empty() {
        tree.push(section(
            "Ingress routes",
            route_refs
                .iter()
                .take(MAX_RENDERED_BACKENDS)
                .map(route_ref_node)
                .collect(),
        ));
    }
    tree.push(section(
        "Port-forward diagnostics",
        tunnel_nodes(
            &tunnel_refs,
            endpoint.addresses.len(),
            "No active port-forward targets the pods behind this endpoint set.",
        ),
    ));

    TrafficDebugAnalysis {
        summary_lines,
        tree,
    }
}

fn analyze_pod(
    pod: &PodInfo,
    snapshot: &ClusterSnapshot,
    tunnels: &TunnelRegistry,
) -> TrafficDebugAnalysis {
    let services = services_selecting_pod(snapshot, pod);
    let route_refs = services
        .iter()
        .flat_map(|service| ingress_routes_for_service(snapshot, &service.namespace, &service.name))
        .collect::<Vec<_>>();
    let tunnel_refs = tunnels
        .ordered_tunnels()
        .into_iter()
        .filter(|tunnel| {
            tunnel.target.namespace == pod.namespace && tunnel.target.pod_name == pod.name
        })
        .take(MAX_RENDERED_TUNNELS)
        .cloned()
        .collect::<Vec<_>>();
    let isolated = pod_isolated_by_ingress_policy(pod, snapshot);

    let mut summary_lines = vec![
        format!(
            "Pod {}/{} is selected by {} Service(s) and reachable from {} Ingress route(s).",
            pod.namespace,
            pod.name,
            services.len(),
            route_refs.len()
        ),
        format!(
            "Port-forward tunnels targeting this pod: {}. Current pod IP: {}.",
            tunnel_refs.len(),
            pod.pod_ip.as_deref().unwrap_or("unknown")
        ),
        format!(
            "Ingress NetworkPolicy isolation: {}.",
            if isolated {
                "yes"
            } else {
                "no isolating ingress policy selects this pod"
            }
        ),
    ];
    if services.is_empty() {
        summary_lines.push(
            "No Service currently selects this pod. Traffic issues here are likely direct pod access, owner/controller drift, or selector mismatch."
                .to_string(),
        );
    }

    let mut tree = vec![section(
        "Selecting services",
        service_nodes_for_pod(&services, snapshot, pod),
    )];
    if !route_refs.is_empty() {
        tree.push(section(
            "Ingress routes",
            route_refs
                .iter()
                .take(MAX_RENDERED_BACKENDS)
                .map(route_ref_node)
                .collect(),
        ));
    }
    tree.push(section(
        "Port-forward diagnostics",
        tunnel_nodes(&tunnel_refs, 1, "No active port-forward targets this pod."),
    ));

    TrafficDebugAnalysis {
        summary_lines,
        tree,
    }
}

fn service_port_summary(service: &ServiceInfo, backends: &[&PodInfo]) -> String {
    if service.port_mappings.is_empty() {
        return "Service port audit: no declared service ports.".to_string();
    }
    let mut matched = 0usize;
    let mut total = 0usize;
    for mapping in &service.port_mappings {
        total += 1;
        if backends
            .iter()
            .any(|pod| service_port_matches_any_backend(mapping, pod))
        {
            matched += 1;
        }
    }
    format!(
        "Service port audit: {matched}/{total} service port mapping(s) resolve to a declared backend container port."
    )
}

fn backends_section_for_service(
    service: &ServiceInfo,
    endpoint: Option<&EndpointInfo>,
    backends: &[&PodInfo],
) -> RelationNode {
    let mut children = Vec::new();
    children.push(leaf(&format!(
        "Selector {}",
        if service.selector.is_empty() {
            "<manual>".to_string()
        } else {
            service
                .selector
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    )));
    if let Some(endpoint) = endpoint {
        children.push(leaf(&format!(
            "Endpoints {} address(es) [{}]",
            endpoint.addresses.len(),
            if endpoint.ports.is_empty() {
                "no ports".to_string()
            } else {
                endpoint.ports.join(", ")
            }
        )));
    }
    if backends.is_empty() {
        children.push(leaf(if service.external_name.is_some() {
            "ExternalName services do not publish pod backends."
        } else if service.selector.is_empty() {
            "No in-cluster pod IP currently matches the published Endpoints addresses."
        } else {
            "No backend pods matched the selector in the current snapshot."
        }));
    } else {
        let mut rendered = backends
            .iter()
            .take(MAX_RENDERED_BACKENDS)
            .map(|pod| pod_backend_node(pod, service.port_mappings.as_slice()))
            .collect::<Vec<_>>();
        if backends.len() > MAX_RENDERED_BACKENDS {
            rendered.push(leaf(&format!(
                "... {} additional backend pod(s) omitted",
                backends.len() - MAX_RENDERED_BACKENDS
            )));
        }
        children.push(section("Backend pods", rendered));
    }
    RelationNode {
        resource: Some(ResourceRef::Service(
            service.name.clone(),
            service.namespace.clone(),
        )),
        label: format!("Service {}", service.name),
        status: Some(format!("{} backend pod(s)", backends.len())),
        namespace: Some(service.namespace.clone()),
        relation: RelationKind::Root,
        not_found: false,
        children,
    }
}

fn service_nodes_for_pod(
    services: &[&ServiceInfo],
    snapshot: &ClusterSnapshot,
    pod: &PodInfo,
) -> Vec<RelationNode> {
    if services.is_empty() {
        return vec![leaf("No Service currently references this pod.")];
    }
    services
        .iter()
        .map(|service| {
            let endpoint = snapshot
                .endpoints
                .iter()
                .find(|ep| ep.name == service.name && ep.namespace == service.namespace);
            let route_count =
                ingress_routes_for_service(snapshot, &service.namespace, &service.name).len();
            RelationNode {
                resource: Some(ResourceRef::Service(
                    service.name.clone(),
                    service.namespace.clone(),
                )),
                label: format!("Service {}", service.name),
                status: Some(format!(
                    "{} route(s), endpoint addresses={}",
                    route_count,
                    endpoint.map_or(0, |ep| ep.addresses.len())
                )),
                namespace: Some(service.namespace.clone()),
                relation: RelationKind::SelectedBy,
                not_found: false,
                children: vec![
                    leaf(&format!(
                        "Ports {}",
                        if service.ports.is_empty() {
                            "none".to_string()
                        } else {
                            service.ports.join(", ")
                        }
                    )),
                    leaf(&format!(
                        "Backend mode {}",
                        if service.selector.is_empty() {
                            "manual / selectorless"
                        } else if service.selector.iter().all(|(key, expected)| {
                            pod.labels
                                .iter()
                                .find(|(label, _)| label == key)
                                .is_some_and(|(_, actual)| actual == expected)
                        }) {
                            "selector satisfied"
                        } else {
                            "selector drift"
                        }
                    )),
                ],
            }
        })
        .collect()
}

fn tunnel_nodes(
    tunnels: &[PortForwardTunnelInfo],
    candidate_count: usize,
    empty_message: &str,
) -> Vec<RelationNode> {
    if tunnels.is_empty() {
        return vec![leaf(empty_message)];
    }
    let mut nodes = tunnels
        .iter()
        .take(MAX_RENDERED_TUNNELS)
        .map(tunnel_node)
        .collect::<Vec<_>>();
    if tunnels.len() > MAX_RENDERED_TUNNELS {
        nodes.push(leaf(&format!(
            "... {} additional tunnel(s) omitted",
            tunnels.len() - MAX_RENDERED_TUNNELS
        )));
    }
    nodes.push(leaf(&format!(
        "Matched against {candidate_count} candidate backend target(s)."
    )));
    nodes
}

fn tunnel_node(tunnel: &PortForwardTunnelInfo) -> RelationNode {
    let state = match tunnel.state {
        TunnelState::Starting => "starting",
        TunnelState::Active => "active",
        TunnelState::Error => "error",
        TunnelState::Closing => "closing",
        TunnelState::Closed => "closed",
    };
    RelationNode {
        resource: Some(ResourceRef::Pod(
            tunnel.target.pod_name.clone(),
            tunnel.target.namespace.clone(),
        )),
        label: format!(
            "Tunnel {}:{} -> {}",
            tunnel.target.pod_name, tunnel.target.remote_port, tunnel.local_addr
        ),
        status: Some(state.to_string()),
        namespace: Some(tunnel.target.namespace.clone()),
        relation: RelationKind::Backend,
        not_found: false,
        children: vec![leaf(
            "If this state stalls, refresh the Port-Forward tab or recreate the tunnel from pod detail.",
        )],
    }
}

fn route_trace_node(route_ref: &IngressRouteRef<'_>) -> RelationNode {
    let route = route_ref.route;
    let route_label = format!(
        "{} {} -> {}:{}",
        route.host.as_deref().unwrap_or("<default-host>"),
        route.path.as_deref().unwrap_or("/*"),
        route.service_name,
        route.service_port
    );
    let mut children = Vec::new();
    match route_ref.service {
        Some(service) => {
            children.push(leaf_with_resource(
                &format!("Service {}", service.name),
                Some(ResourceRef::Service(
                    service.name.clone(),
                    service.namespace.clone(),
                )),
                Some(service.type_.clone()),
                Some(service.namespace.clone()),
            ));
            if let Some(endpoint) = route_ref.endpoint {
                children.push(leaf(&format!(
                    "Endpoints {} address(es) [{}]",
                    endpoint.addresses.len(),
                    if endpoint.ports.is_empty() {
                        "no ports".to_string()
                    } else {
                        endpoint.ports.join(", ")
                    }
                )));
            } else {
                children.push(leaf(
                    "No Endpoints object currently publishes this Service.",
                ));
            }
            if route_ref.backends.is_empty() {
                children.push(leaf(
                    "No backend pod in the snapshot satisfies this Service selector.",
                ));
            } else {
                let mut backend_nodes = route_ref
                    .backends
                    .iter()
                    .take(MAX_RENDERED_BACKENDS)
                    .map(|pod| pod_backend_node(pod, service.port_mappings.as_slice()))
                    .collect::<Vec<_>>();
                if route_ref.backends.len() > MAX_RENDERED_BACKENDS {
                    backend_nodes.push(leaf(&format!(
                        "... {} additional backend pod(s) omitted",
                        route_ref.backends.len() - MAX_RENDERED_BACKENDS
                    )));
                }
                children.push(section("Backend pods", backend_nodes));
            }
        }
        None => {
            children.push(leaf(
                "Referenced Service does not exist in the current snapshot.",
            ));
        }
    }

    RelationNode {
        resource: Some(ResourceRef::Ingress(
            route_ref.ingress.name.clone(),
            route_ref.ingress.namespace.clone(),
        )),
        label: route_label,
        status: Some(
            route_ref
                .service
                .map(|service| service.type_.as_str())
                .unwrap_or("service missing")
                .to_string(),
        ),
        namespace: Some(route_ref.ingress.namespace.clone()),
        relation: RelationKind::Backend,
        not_found: route_ref.service.is_none(),
        children,
    }
}

fn route_ref_node(route_ref: &IngressRouteRef<'_>) -> RelationNode {
    leaf_with_resource(
        &format!(
            "Ingress {} routes {} {} -> {}:{}",
            route_ref.ingress.name,
            route_ref.route.host.as_deref().unwrap_or("<default-host>"),
            route_ref.route.path.as_deref().unwrap_or("/*"),
            route_ref.route.service_name,
            route_ref.route.service_port
        ),
        Some(ResourceRef::Ingress(
            route_ref.ingress.name.clone(),
            route_ref.ingress.namespace.clone(),
        )),
        route_ref
            .service
            .map(|service| format!("service {}", service.type_)),
        Some(route_ref.ingress.namespace.clone()),
    )
}

fn endpoint_address_nodes(endpoint: &EndpointInfo) -> Vec<RelationNode> {
    if endpoint.addresses.is_empty() {
        return vec![leaf("No endpoint addresses are currently published.")];
    }
    endpoint
        .addresses
        .iter()
        .take(MAX_RENDERED_BACKENDS)
        .map(|address| leaf(address))
        .collect()
}

fn ingress_host_nodes(ingress: &IngressInfo) -> Vec<RelationNode> {
    let mut nodes = Vec::new();
    if ingress.hosts.is_empty() {
        nodes.push(leaf(
            "No explicit host rules. This ingress relies on default backend or wildcard matching.",
        ));
    } else {
        nodes.extend(ingress.hosts.iter().map(|host| {
            leaf(&format!(
                "Host {} -> address {}",
                host,
                ingress.address.as_deref().unwrap_or("pending")
            ))
        }));
    }
    if let Some(address) = &ingress.address {
        nodes.push(leaf(&format!(
            "DNS should resolve ingress hosts to {} via your external DNS/controller setup.",
            address
        )));
    }
    nodes
}

fn service_dns_nodes(service: &ServiceInfo) -> Vec<RelationNode> {
    let mut nodes = vec![
        leaf(&format!("Short name {}", service.name)),
        leaf(&format!("Qualified {}.{}", service.name, service.namespace)),
        leaf(&format!(
            "Service DNS {}.{}.svc",
            service.name, service.namespace
        )),
        leaf(&format!(
            "FQDN {}.{}.svc.cluster.local",
            service.name, service.namespace
        )),
    ];
    if let Some(external_name) = &service.external_name {
        nodes.push(leaf(&format!("ExternalName target {}", external_name)));
    }
    nodes
}

fn service_backends<'a>(service: &ServiceInfo, snapshot: &'a ClusterSnapshot) -> Vec<&'a PodInfo> {
    if service.selector.is_empty() {
        return endpoint_backends(service, snapshot);
    }
    snapshot
        .pods
        .iter()
        .filter(|pod| {
            pod.namespace == service.namespace
                && service.selector.iter().all(|(key, expected)| {
                    pod.labels
                        .iter()
                        .find(|(label, _)| label == key)
                        .is_some_and(|(_, actual)| actual == expected)
                })
        })
        .collect()
}

fn endpoint_backends<'a>(service: &ServiceInfo, snapshot: &'a ClusterSnapshot) -> Vec<&'a PodInfo> {
    let Some(endpoint) = snapshot
        .endpoints
        .iter()
        .find(|ep| ep.name == service.name && ep.namespace == service.namespace)
    else {
        return Vec::new();
    };
    let addresses = endpoint.addresses.iter().collect::<BTreeSet<_>>();
    snapshot
        .pods
        .iter()
        .filter(|pod| {
            pod.namespace == service.namespace
                && pod
                    .pod_ip
                    .as_ref()
                    .is_some_and(|pod_ip| addresses.contains(&pod_ip))
        })
        .collect()
}

fn ingress_routes_for_service<'a>(
    snapshot: &'a ClusterSnapshot,
    namespace: &str,
    service_name: &str,
) -> Vec<IngressRouteRef<'a>> {
    snapshot
        .ingresses
        .iter()
        .filter(|ingress| ingress.namespace == namespace)
        .flat_map(|ingress| ingress.routes.iter().map(move |route| (ingress, route)))
        .filter(|(_, route)| route.service_name == service_name)
        .map(|(ingress, route)| build_route_ref(snapshot, ingress, route))
        .collect()
}

fn ingress_route_refs<'a>(
    snapshot: &'a ClusterSnapshot,
    ingress: &'a IngressInfo,
) -> Vec<IngressRouteRef<'a>> {
    ingress
        .routes
        .iter()
        .map(|route| build_route_ref(snapshot, ingress, route))
        .collect()
}

fn build_route_ref<'a>(
    snapshot: &'a ClusterSnapshot,
    ingress: &'a IngressInfo,
    route: &'a IngressRouteInfo,
) -> IngressRouteRef<'a> {
    let service = snapshot
        .services
        .iter()
        .find(|svc| svc.namespace == ingress.namespace && svc.name == route.service_name);
    let endpoint = snapshot
        .endpoints
        .iter()
        .find(|ep| ep.namespace == ingress.namespace && ep.name == route.service_name);
    let backends = service
        .map(|service| service_backends(service, snapshot))
        .unwrap_or_default();
    IngressRouteRef {
        ingress,
        route,
        service,
        endpoint,
        backends,
    }
}

fn services_selecting_pod<'a>(
    snapshot: &'a ClusterSnapshot,
    pod: &PodInfo,
) -> Vec<&'a ServiceInfo> {
    snapshot
        .services
        .iter()
        .filter(|service| {
            if service.namespace != pod.namespace {
                return false;
            }
            if !service.selector.is_empty() {
                return service.selector.iter().all(|(key, expected)| {
                    pod.labels
                        .iter()
                        .find(|(label, _)| label == key)
                        .is_some_and(|(_, actual)| actual == expected)
                });
            }
            snapshot
                .endpoints
                .iter()
                .find(|ep| ep.name == service.name && ep.namespace == service.namespace)
                .is_some_and(|endpoint| {
                    pod.pod_ip.as_ref().is_some_and(|pod_ip| {
                        endpoint.addresses.iter().any(|address| address == pod_ip)
                    })
                })
        })
        .collect()
}

fn tunnel_refs_for_service(
    service: &ServiceInfo,
    backends: &[&PodInfo],
    tunnels: &TunnelRegistry,
) -> Vec<PortForwardTunnelInfo> {
    let backend_names = backends
        .iter()
        .map(|pod| (pod.namespace.as_str(), pod.name.as_str()))
        .collect::<BTreeSet<_>>();
    tunnels
        .ordered_tunnels()
        .into_iter()
        .filter(|tunnel| {
            tunnel.target.namespace == service.namespace
                && backend_names.contains(&(
                    tunnel.target.namespace.as_str(),
                    tunnel.target.pod_name.as_str(),
                ))
        })
        .take(MAX_RENDERED_TUNNELS)
        .cloned()
        .collect()
}

fn tunnel_refs_for_route_refs<'a>(
    route_refs: &[IngressRouteRef<'a>],
    tunnels: &'a TunnelRegistry,
) -> Vec<PortForwardTunnelInfo> {
    let backend_names = route_refs
        .iter()
        .flat_map(|route| {
            route
                .backends
                .iter()
                .map(|pod| (pod.namespace.as_str(), pod.name.as_str()))
        })
        .collect::<BTreeSet<_>>();
    tunnels
        .ordered_tunnels()
        .into_iter()
        .filter(|tunnel| {
            backend_names.contains(&(
                tunnel.target.namespace.as_str(),
                tunnel.target.pod_name.as_str(),
            ))
        })
        .take(MAX_RENDERED_TUNNELS)
        .cloned()
        .collect()
}

fn pod_backend_node(pod: &PodInfo, mappings: &[ServicePortInfo]) -> RelationNode {
    let matched_ports = mappings
        .iter()
        .filter(|mapping| service_port_matches_any_backend(mapping, pod))
        .map(|mapping| format!("{}/{}", mapping.port, mapping.protocol))
        .collect::<Vec<_>>();
    RelationNode {
        resource: Some(ResourceRef::Pod(pod.name.clone(), pod.namespace.clone())),
        label: format!("Pod {}", pod.name),
        status: Some(pod.status.clone()),
        namespace: Some(pod.namespace.clone()),
        relation: RelationKind::Backend,
        not_found: false,
        children: vec![
            leaf(&format!(
                "IP {}",
                pod.pod_ip.as_deref().unwrap_or("unknown")
            )),
            leaf(&format!(
                "Container ports {}",
                if pod.container_ports.is_empty() {
                    "none".to_string()
                } else {
                    pod.container_ports
                        .iter()
                        .map(|port| {
                            port.name
                                .as_ref()
                                .map(|name| format!("{name}:{}", port.container_port))
                                .unwrap_or_else(|| port.container_port.to_string())
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            )),
            leaf(&format!(
                "Service matches {}",
                if matched_ports.is_empty() {
                    "none".to_string()
                } else {
                    matched_ports.join(", ")
                }
            )),
        ],
    }
}

fn isolated_ingress_backend_count(backends: &[&PodInfo], snapshot: &ClusterSnapshot) -> usize {
    backends
        .iter()
        .filter(|pod| pod_isolated_by_ingress_policy(pod, snapshot))
        .count()
}

fn pod_isolated_by_ingress_policy(pod: &PodInfo, snapshot: &ClusterSnapshot) -> bool {
    snapshot
        .network_policies
        .iter()
        .any(|policy| policy_applies_to_ingress(policy) && policy_selects_pod(policy, pod))
}

fn service_port_matches_any_backend(mapping: &ServicePortInfo, pod: &PodInfo) -> bool {
    pod.container_ports.iter().any(|port| {
        let target_port_number = mapping.target_port_number.unwrap_or(mapping.port);
        port.protocol.eq_ignore_ascii_case(&mapping.protocol)
            && (target_port_number == port.container_port
                || mapping.target_port_name.as_ref().is_some_and(|name| {
                    port.name
                        .as_ref()
                        .is_some_and(|port_name| port_name == name)
                }))
    })
}

fn find_service<'a>(
    snapshot: &'a ClusterSnapshot,
    name: &str,
    namespace: &str,
) -> Result<&'a ServiceInfo, String> {
    snapshot
        .services
        .iter()
        .find(|service| service.name == name && service.namespace == namespace)
        .ok_or_else(|| format!("Service '{namespace}/{name}' is no longer in the snapshot."))
}

fn find_ingress<'a>(
    snapshot: &'a ClusterSnapshot,
    name: &str,
    namespace: &str,
) -> Result<&'a IngressInfo, String> {
    snapshot
        .ingresses
        .iter()
        .find(|ingress| ingress.name == name && ingress.namespace == namespace)
        .ok_or_else(|| format!("Ingress '{namespace}/{name}' is no longer in the snapshot."))
}

fn find_endpoint<'a>(
    snapshot: &'a ClusterSnapshot,
    name: &str,
    namespace: &str,
) -> Result<&'a EndpointInfo, String> {
    snapshot
        .endpoints
        .iter()
        .find(|endpoint| endpoint.name == name && endpoint.namespace == namespace)
        .ok_or_else(|| format!("Endpoints '{namespace}/{name}' is no longer in the snapshot."))
}

fn find_pod<'a>(
    snapshot: &'a ClusterSnapshot,
    name: &str,
    namespace: &str,
) -> Result<&'a PodInfo, String> {
    snapshot
        .pods
        .iter()
        .find(|pod| pod.name == name && pod.namespace == namespace)
        .ok_or_else(|| format!("Pod '{namespace}/{name}' is no longer in the snapshot."))
}

fn section(label: &str, children: Vec<RelationNode>) -> RelationNode {
    RelationNode {
        resource: None,
        label: label.to_string(),
        status: None,
        namespace: None,
        relation: RelationKind::SectionHeader,
        not_found: false,
        children,
    }
}

fn leaf(label: &str) -> RelationNode {
    leaf_with_resource(label, None, None, None)
}

fn leaf_with_resource(
    label: &str,
    resource: Option<ResourceRef>,
    status: Option<String>,
    namespace: Option<String>,
) -> RelationNode {
    RelationNode {
        resource,
        label: label.to_string(),
        status,
        namespace,
        relation: RelationKind::Backend,
        not_found: false,
        children: Vec::new(),
    }
}

#[derive(Debug, Clone)]
struct IngressRouteRef<'a> {
    ingress: &'a IngressInfo,
    route: &'a IngressRouteInfo,
    service: Option<&'a ServiceInfo>,
    endpoint: Option<&'a EndpointInfo>,
    backends: Vec<&'a PodInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        k8s::{
            dtos::{ContainerPortInfo, IngressInfo, IngressRouteInfo},
            portforward::{PortForwardTarget, PortForwardTunnelInfo, TunnelState},
        },
        state::ClusterSnapshot,
    };
    use std::{collections::BTreeMap, net::SocketAddr, str::FromStr};

    fn pod(name: &str, namespace: &str, ip: &str) -> PodInfo {
        PodInfo {
            name: name.to_string(),
            namespace: namespace.to_string(),
            pod_ip: Some(ip.to_string()),
            status: "Running".to_string(),
            labels: vec![("app".to_string(), "api".to_string())],
            container_ports: vec![ContainerPortInfo {
                name: Some("http".to_string()),
                container_port: 8080,
                protocol: "TCP".to_string(),
            }],
            ..PodInfo::default()
        }
    }

    fn service(name: &str) -> ServiceInfo {
        ServiceInfo {
            name: name.to_string(),
            namespace: "demo".to_string(),
            type_: "ClusterIP".to_string(),
            cluster_ip: Some("10.0.0.20".to_string()),
            selector: BTreeMap::from([("app".to_string(), "api".to_string())]),
            port_mappings: vec![ServicePortInfo {
                port: 80,
                protocol: "TCP".to_string(),
                target_port_name: Some("http".to_string()),
                target_port_number: None,
            }],
            ports: vec!["80/TCP".to_string()],
            ..ServiceInfo::default()
        }
    }

    fn selectorless_service(name: &str) -> ServiceInfo {
        ServiceInfo {
            name: name.to_string(),
            namespace: "demo".to_string(),
            type_: "ClusterIP".to_string(),
            cluster_ip: Some("10.0.0.30".to_string()),
            port_mappings: vec![ServicePortInfo {
                port: 8080,
                protocol: "TCP".to_string(),
                target_port_name: None,
                target_port_number: None,
            }],
            ports: vec!["8080/TCP".to_string()],
            ..ServiceInfo::default()
        }
    }

    fn endpoint(name: &str) -> EndpointInfo {
        EndpointInfo {
            name: name.to_string(),
            namespace: "demo".to_string(),
            addresses: vec!["10.42.0.8".to_string()],
            ports: vec!["8080/TCP".to_string()],
            ..EndpointInfo::default()
        }
    }

    fn ingress(name: &str, service_name: &str) -> IngressInfo {
        IngressInfo {
            name: name.to_string(),
            namespace: "demo".to_string(),
            class: Some("nginx".to_string()),
            hosts: vec!["api.example.test".to_string()],
            address: Some("1.2.3.4".to_string()),
            backend_services: vec![(service_name.to_string(), "80".to_string())],
            routes: vec![IngressRouteInfo {
                host: Some("api.example.test".to_string()),
                path: Some("/".to_string()),
                service_name: service_name.to_string(),
                service_port: "80".to_string(),
            }],
            ..IngressInfo::default()
        }
    }

    fn tunnel(namespace: &str, pod_name: &str) -> PortForwardTunnelInfo {
        PortForwardTunnelInfo {
            id: format!("{namespace}/{pod_name}/8080"),
            target: PortForwardTarget::new(namespace, pod_name, 8080),
            local_addr: SocketAddr::from_str("127.0.0.1:18080").unwrap(),
            state: TunnelState::Active,
        }
    }

    #[test]
    fn service_analysis_includes_dns_and_tunnels() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.services.push(service("api"));
        snapshot.endpoints.push(endpoint("api"));
        snapshot.pods.push(pod("api-0", "demo", "10.42.0.8"));
        snapshot.ingresses.push(ingress("edge", "api"));
        let mut tunnels = TunnelRegistry::new();
        tunnels.add_tunnel(tunnel("demo", "api-0"));

        let analysis = analyze_resource(
            &ResourceRef::Service("api".to_string(), "demo".to_string()),
            &snapshot,
            &tunnels,
        )
        .unwrap();

        assert!(analysis.summary_lines[0].contains("Service demo/api"));
        assert!(
            analysis
                .tree
                .iter()
                .any(|section| section.label == "DNS hints")
        );
        assert!(
            analysis
                .tree
                .iter()
                .any(|section| section.label == "Port-forward diagnostics")
        );
    }

    #[test]
    fn ingress_analysis_reports_missing_service() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.ingresses.push(ingress("edge", "missing"));

        let analysis = analyze_resource(
            &ResourceRef::Ingress("edge".to_string(), "demo".to_string()),
            &snapshot,
            &TunnelRegistry::new(),
        )
        .unwrap();

        assert!(analysis.summary_lines[1].contains("0 backend service(s) currently resolvable"));
        let backend = analysis
            .tree
            .iter()
            .find(|section| section.label == "Backend trace")
            .unwrap();
        assert!(backend.children[0].not_found);
    }

    #[test]
    fn endpoint_analysis_flags_missing_service() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.endpoints.push(endpoint("manual"));

        let analysis = analyze_resource(
            &ResourceRef::Endpoint("manual".to_string(), "demo".to_string()),
            &snapshot,
            &TunnelRegistry::new(),
        )
        .unwrap();

        assert!(analysis.summary_lines[1].contains("No matching Service object"));
    }

    #[test]
    fn pod_analysis_includes_selecting_services() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.services.push(service("api"));
        snapshot.pods.push(pod("api-0", "demo", "10.42.0.8"));

        let analysis = analyze_resource(
            &ResourceRef::Pod("api-0".to_string(), "demo".to_string()),
            &snapshot,
            &TunnelRegistry::new(),
        )
        .unwrap();

        assert!(analysis.summary_lines[0].contains("selected by 1 Service"));
        assert!(
            analysis
                .tree
                .iter()
                .any(|section| section.label == "Selecting services")
        );
    }

    #[test]
    fn unsupported_resource_returns_error() {
        let err = analyze_resource(
            &ResourceRef::ConfigMap("cfg".to_string(), "demo".to_string()),
            &ClusterSnapshot::default(),
            &TunnelRegistry::new(),
        )
        .unwrap_err();

        assert!(err.contains("Traffic debugging"));
    }

    #[test]
    fn service_port_matching_defaults_target_port_to_service_port() {
        let pod = PodInfo {
            container_ports: vec![ContainerPortInfo {
                name: None,
                container_port: 8080,
                protocol: "TCP".to_string(),
            }],
            ..PodInfo::default()
        };
        let mapping = ServicePortInfo {
            port: 8080,
            protocol: "TCP".to_string(),
            target_port_name: None,
            target_port_number: None,
        };

        assert!(service_port_matches_any_backend(&mapping, &pod));
    }

    #[test]
    fn pod_analysis_includes_selectorless_endpoint_backed_services() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.services.push(selectorless_service("manual"));
        snapshot.endpoints.push(endpoint("manual"));
        snapshot.pods.push(pod("api-0", "demo", "10.42.0.8"));

        let analysis = analyze_resource(
            &ResourceRef::Pod("api-0".to_string(), "demo".to_string()),
            &snapshot,
            &TunnelRegistry::new(),
        )
        .unwrap();

        assert!(analysis.summary_lines[0].contains("selected by 1 Service"));
    }
}
