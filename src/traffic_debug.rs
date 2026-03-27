//! Snapshot-only service and traffic debugging analysis.

use std::collections::BTreeSet;

use crate::{
    app::ResourceRef,
    k8s::{
        dtos::{
            EndpointInfo, GatewayBackendRefInfo, GatewayInfo, GatewayParentRefInfo, GrpcRouteInfo,
            HttpRouteInfo, IngressInfo, IngressRouteInfo, PodInfo, ServiceInfo, ServicePortInfo,
        },
        gateway_semantics::{gateway_parent_attachment_allowed, reference_grant_allows_backend},
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
        ResourceRef::CustomResource {
            name,
            namespace,
            group,
            kind,
            ..
        } if group == "gateway.networking.k8s.io" && kind == "Gateway" => {
            let namespace = namespace
                .as_deref()
                .ok_or_else(|| format!("Gateway '{name}' is missing namespace context."))?;
            let gateway = find_gateway(snapshot, name, namespace)?;
            Ok(analyze_gateway(gateway, snapshot, tunnels))
        }
        ResourceRef::CustomResource {
            name,
            namespace,
            group,
            kind,
            ..
        } if group == "gateway.networking.k8s.io" && kind == "HTTPRoute" => {
            let namespace = namespace
                .as_deref()
                .ok_or_else(|| format!("HTTPRoute '{name}' is missing namespace context."))?;
            let route = find_http_route(snapshot, name, namespace)?;
            Ok(analyze_http_route(route, snapshot, tunnels))
        }
        ResourceRef::CustomResource {
            name,
            namespace,
            group,
            kind,
            ..
        } if group == "gateway.networking.k8s.io" && kind == "GRPCRoute" => {
            let namespace = namespace
                .as_deref()
                .ok_or_else(|| format!("GRPCRoute '{name}' is missing namespace context."))?;
            let route = find_grpc_route(snapshot, name, namespace)?;
            Ok(analyze_grpc_route(route, snapshot, tunnels))
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
            "Traffic debugging is available for Services, Endpoints, Ingresses, Gateways, HTTPRoutes, GRPCRoutes, and Pods."
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
    let ingress_route_refs =
        ingress_routes_for_service(snapshot, &service.namespace, &service.name);
    let gateway_route_refs = gateway_routes_for_service(snapshot, service);
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
            "Traffic entrypoints: {} ingress route(s), {} gateway route(s). Port-forward tunnels to backend pods: {}.",
            ingress_route_refs.len(),
            gateway_route_refs.len(),
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
    if !ingress_route_refs.is_empty() {
        tree.push(section(
            "Ingress routes",
            ingress_route_refs
                .iter()
                .take(MAX_RENDERED_BACKENDS)
                .map(route_ref_node)
                .collect(),
        ));
    }
    if !gateway_route_refs.is_empty() {
        tree.push(section(
            "Gateway routes",
            gateway_route_refs
                .iter()
                .take(MAX_RENDERED_BACKENDS)
                .map(gateway_service_route_node)
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

fn analyze_gateway(
    gateway: &GatewayInfo,
    snapshot: &ClusterSnapshot,
    tunnels: &TunnelRegistry,
) -> TrafficDebugAnalysis {
    let route_refs = gateway_route_refs(snapshot, gateway);
    let backend_services = route_refs
        .iter()
        .flat_map(|route| {
            route.resolution_set.iter().filter_map(|resolution| {
                resolution
                    .service
                    .map(|service| (service.namespace.as_str(), service.name.as_str()))
            })
        })
        .collect::<BTreeSet<_>>();
    let backend_pods = route_refs
        .iter()
        .flat_map(|route| {
            route
                .resolution_set
                .iter()
                .flat_map(|resolution| resolution.backends.iter())
                .map(|pod| (pod.namespace.as_str(), pod.name.as_str()))
        })
        .collect::<BTreeSet<_>>();
    let tunnel_refs = tunnel_refs_for_gateway_routes(&route_refs, tunnels);
    let blocked_cross_namespace = route_refs
        .iter()
        .filter(|route| route.cross_namespace_parent_blocked)
        .count();

    let mut summary_lines = vec![
        format!(
            "Gateway {}/{} class={} listeners={} attached routes={}.",
            gateway.namespace,
            gateway.name,
            gateway.gateway_class_name,
            gateway.listeners.len(),
            route_refs.len()
        ),
        format!(
            "Backend reachability intent: {} backend service(s), {} backend pod(s), {} active tunnel(s).",
            backend_services.len(),
            backend_pods.len(),
            tunnel_refs.len()
        ),
        format!(
            "Published addresses: {}.",
            if gateway.addresses.is_empty() {
                "none yet".to_string()
            } else {
                gateway.addresses.join(", ")
            }
        ),
    ];
    if blocked_cross_namespace > 0 {
        summary_lines.push(format!(
            "{blocked_cross_namespace} attached route(s) cross namespaces without listener allowedRoutes coverage. Control-plane attachment may still be rejected."
        ));
    }

    let mut tree = vec![section("Listeners", gateway_listener_nodes(gateway))];
    tree.push(section(
        "Attached routes",
        if route_refs.is_empty() {
            vec![leaf(
                "No HTTPRoute or GRPCRoute currently targets this Gateway.",
            )]
        } else {
            route_refs
                .iter()
                .take(MAX_RENDERED_BACKENDS)
                .map(gateway_route_node)
                .collect()
        },
    ));
    tree.push(section(
        "Port-forward diagnostics",
        tunnel_nodes(
            &tunnel_refs,
            backend_pods.len(),
            "No active port-forward targets a backend pod behind this Gateway.",
        ),
    ));

    TrafficDebugAnalysis {
        summary_lines,
        tree,
    }
}

fn analyze_http_route(
    route: &HttpRouteInfo,
    snapshot: &ClusterSnapshot,
    tunnels: &TunnelRegistry,
) -> TrafficDebugAnalysis {
    analyze_gateway_route(
        GatewayRouteAnalysisInput {
            route_kind: "HTTPRoute",
            route_version: route.version.as_str(),
            route_name: route.name.as_str(),
            namespace: route.namespace.as_str(),
            hostnames: route.hostnames.as_slice(),
            parent_refs: route.parent_refs.as_slice(),
            backend_refs: route.backend_refs.as_slice(),
        },
        snapshot,
        tunnels,
    )
}

fn analyze_grpc_route(
    route: &GrpcRouteInfo,
    snapshot: &ClusterSnapshot,
    tunnels: &TunnelRegistry,
) -> TrafficDebugAnalysis {
    analyze_gateway_route(
        GatewayRouteAnalysisInput {
            route_kind: "GRPCRoute",
            route_version: route.version.as_str(),
            route_name: route.name.as_str(),
            namespace: route.namespace.as_str(),
            hostnames: route.hostnames.as_slice(),
            parent_refs: route.parent_refs.as_slice(),
            backend_refs: route.backend_refs.as_slice(),
        },
        snapshot,
        tunnels,
    )
}

fn analyze_gateway_route(
    route: GatewayRouteAnalysisInput<'_>,
    snapshot: &ClusterSnapshot,
    tunnels: &TunnelRegistry,
) -> TrafficDebugAnalysis {
    let parent_gateways = route_parent_gateways(snapshot, route.namespace, route.parent_refs);
    let backend_resolutions = route
        .backend_refs
        .iter()
        .map(|backend| {
            resolve_gateway_backend(snapshot, route.namespace, route.route_kind, backend)
        })
        .collect::<Vec<_>>();
    let resolved_services = backend_resolutions
        .iter()
        .filter(|resolution| resolution.service.is_some())
        .count();
    let blocked_cross_namespace = backend_resolutions
        .iter()
        .filter(|resolution| resolution.cross_namespace && !resolution.reference_grant_allowed)
        .count();
    let total_backends = backend_resolutions
        .iter()
        .map(|resolution| resolution.backends.len())
        .sum::<usize>();
    let tunnel_refs = tunnel_refs_for_gateway_backend_resolutions(&backend_resolutions, tunnels);

    let mut summary_lines = vec![
        format!(
            "{} {}/{} attaches to {} parent gateway(s) and references {} backend(s).",
            route.route_kind,
            route.namespace,
            route.route_name,
            parent_gateways.len(),
            route.backend_refs.len()
        ),
        format!(
            "Backend resolution: {resolved_services}/{} backend service(s) resolved, {} backend pod(s), {} active tunnel(s).",
            route.backend_refs.len(),
            total_backends,
            tunnel_refs.len()
        ),
        format!(
            "Hostnames: {}.",
            if route.hostnames.is_empty() {
                "<match-all>".to_string()
            } else {
                route.hostnames.join(", ")
            }
        ),
    ];
    if blocked_cross_namespace > 0 {
        summary_lines.push(format!(
            "{blocked_cross_namespace} backend reference(s) cross namespaces without a matching ReferenceGrant."
        ));
    }

    let mut tree = vec![section(
        "Parent gateways",
        if parent_gateways.is_empty() {
            vec![leaf(
                "No parent Gateway currently resolves from this route.",
            )]
        } else {
            parent_gateways
                .iter()
                .map(parent_gateway_node)
                .collect::<Vec<_>>()
        },
    )];
    tree.push(section(
        "Backend trace",
        if backend_resolutions.is_empty() {
            vec![leaf("No backendRefs are declared on this route.")]
        } else {
            backend_resolutions
                .iter()
                .take(MAX_RENDERED_BACKENDS)
                .map(|resolution| gateway_backend_node(route.route_kind, resolution))
                .collect()
        },
    ));
    tree.push(section(
        "Port-forward diagnostics",
        tunnel_nodes(
            &tunnel_refs,
            total_backends,
            "No active port-forward targets any backend pod on this route.",
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

fn gateway_listener_nodes(gateway: &GatewayInfo) -> Vec<RelationNode> {
    if gateway.listeners.is_empty() {
        return vec![leaf("No listeners are declared on this Gateway.")];
    }
    gateway
        .listeners
        .iter()
        .map(|listener| {
            let mut children = vec![
                leaf(&format!(
                    "Protocol {} port {}",
                    listener.protocol, listener.port
                )),
                leaf(&format!(
                    "Allowed routes {}",
                    listener
                        .allowed_routes_from
                        .as_deref()
                        .unwrap_or("Same (default)")
                )),
                leaf(&format!("Attached routes {}", listener.attached_routes)),
            ];
            if let Some(hostname) = &listener.hostname {
                children.push(leaf(&format!("Hostname {}", hostname)));
            }
            RelationNode {
                resource: Some(ResourceRef::CustomResource {
                    name: gateway.name.clone(),
                    namespace: Some(gateway.namespace.clone()),
                    group: "gateway.networking.k8s.io".to_string(),
                    version: gateway.version.clone(),
                    kind: "Gateway".to_string(),
                    plural: "gateways".to_string(),
                }),
                label: format!("Listener {}", listener.name),
                status: listener.ready.map(|ready| {
                    if ready {
                        "Programmed".to_string()
                    } else {
                        "Pending".to_string()
                    }
                }),
                namespace: Some(gateway.namespace.clone()),
                relation: RelationKind::Backend,
                not_found: false,
                children,
            }
        })
        .collect()
}

fn gateway_route_refs<'a>(
    snapshot: &'a ClusterSnapshot,
    gateway: &'a GatewayInfo,
) -> Vec<GatewayRouteRef<'a>> {
    let mut refs = snapshot
        .http_routes
        .iter()
        .filter_map(|route| {
            gateway_route_ref_for(
                GatewayRouteAnalysisInput {
                    route_kind: "HTTPRoute",
                    route_version: route.version.as_str(),
                    route_name: route.name.as_str(),
                    namespace: route.namespace.as_str(),
                    hostnames: route.hostnames.as_slice(),
                    parent_refs: route.parent_refs.as_slice(),
                    backend_refs: route.backend_refs.as_slice(),
                },
                gateway,
                snapshot,
            )
        })
        .collect::<Vec<_>>();
    refs.extend(snapshot.grpc_routes.iter().filter_map(|route| {
        gateway_route_ref_for(
            GatewayRouteAnalysisInput {
                route_kind: "GRPCRoute",
                route_version: route.version.as_str(),
                route_name: route.name.as_str(),
                namespace: route.namespace.as_str(),
                hostnames: route.hostnames.as_slice(),
                parent_refs: route.parent_refs.as_slice(),
                backend_refs: route.backend_refs.as_slice(),
            },
            gateway,
            snapshot,
        )
    }));
    refs
}

fn gateway_route_ref_for<'a>(
    route: GatewayRouteAnalysisInput<'a>,
    gateway: &'a GatewayInfo,
    snapshot: &'a ClusterSnapshot,
) -> Option<GatewayRouteRef<'a>> {
    let matching_parent_refs = route
        .parent_refs
        .iter()
        .filter(|parent| {
            let parent_namespace = parent.namespace.as_deref().unwrap_or(route.namespace);
            parent.kind == "Gateway"
                && parent.name == gateway.name
                && parent_namespace == gateway.namespace
        })
        .collect::<Vec<_>>();
    if matching_parent_refs.is_empty() {
        return None;
    }
    let parent_ref = matching_parent_refs
        .iter()
        .copied()
        .find(|parent| gateway_parent_attachment_allowed(gateway, route.namespace, parent))
        .unwrap_or(matching_parent_refs[0]);
    let cross_namespace_parent_blocked = route.namespace != gateway.namespace
        && matching_parent_refs
            .iter()
            .all(|parent| !gateway_parent_attachment_allowed(gateway, route.namespace, parent));
    let resolutions = route
        .backend_refs
        .iter()
        .map(|backend| {
            resolve_gateway_backend(snapshot, route.namespace, route.route_kind, backend)
        })
        .collect();
    Some(GatewayRouteRef {
        route_kind: route.route_kind,
        route_version: route.route_version,
        route_name: route.route_name,
        route_namespace: route.namespace,
        hostnames: route.hostnames,
        parent_ref,
        resolution_set: resolutions,
        cross_namespace_parent_blocked,
    })
}

fn route_parent_gateways<'a>(
    snapshot: &'a ClusterSnapshot,
    route_namespace: &str,
    parent_refs: &'a [GatewayParentRefInfo],
) -> Vec<GatewayParentGatewayRef<'a>> {
    parent_refs
        .iter()
        .filter(|parent| parent.kind == "Gateway")
        .map(|parent| {
            let namespace = parent.namespace.as_deref().unwrap_or(route_namespace);
            let gateway = snapshot
                .gateways
                .iter()
                .find(|gateway| gateway.name == parent.name && gateway.namespace == namespace);
            GatewayParentGatewayRef {
                parent,
                namespace: namespace.to_string(),
                gateway,
            }
        })
        .collect()
}

fn parent_gateway_node(parent: &GatewayParentGatewayRef<'_>) -> RelationNode {
    let mut children = Vec::new();
    if let Some(section_name) = &parent.parent.section_name {
        children.push(leaf(&format!("Listener section {}", section_name)));
    }
    if let Some(gateway) = parent.gateway {
        children.push(leaf(&format!(
            "Addresses {}",
            if gateway.addresses.is_empty() {
                "none".to_string()
            } else {
                gateway.addresses.join(", ")
            }
        )));
    } else {
        children.push(leaf("Gateway is missing from the current snapshot."));
    }

    let mut node = leaf_with_resource(
        &format!("Gateway {}", parent.parent.name),
        parent.gateway.map(|gateway| ResourceRef::CustomResource {
            name: gateway.name.clone(),
            namespace: Some(gateway.namespace.clone()),
            group: "gateway.networking.k8s.io".to_string(),
            version: gateway.version.clone(),
            kind: "Gateway".to_string(),
            plural: "gateways".to_string(),
        }),
        parent
            .gateway
            .map(|gateway| gateway.gateway_class_name.clone()),
        Some(parent.namespace.clone()),
    );
    node.children = children;
    node
}

fn resolve_gateway_backend<'a>(
    snapshot: &'a ClusterSnapshot,
    route_namespace: &str,
    route_kind: &str,
    backend_ref: &'a GatewayBackendRefInfo,
) -> GatewayBackendResolution<'a> {
    let target_namespace = backend_ref.namespace.as_deref().unwrap_or(route_namespace);
    let cross_namespace = target_namespace != route_namespace;
    let reference_grant_allowed = !cross_namespace
        || reference_grant_allows_backend(
            snapshot.reference_grants.as_slice(),
            route_namespace,
            route_kind,
            backend_ref,
        );
    let blocked_cross_namespace = cross_namespace && !reference_grant_allowed;
    let service = if backend_ref.kind == "Service" {
        (!blocked_cross_namespace)
            .then(|| {
                snapshot.services.iter().find(|service| {
                    service.namespace == target_namespace && service.name == backend_ref.name
                })
            })
            .flatten()
    } else {
        None
    };
    let endpoint = service.and_then(|service| {
        snapshot.endpoints.iter().find(|endpoint| {
            endpoint.namespace == service.namespace && endpoint.name == service.name
        })
    });
    let backends = service
        .map(|service| service_backends(service, snapshot))
        .unwrap_or_default();

    GatewayBackendResolution {
        backend_ref,
        target_namespace: target_namespace.to_string(),
        cross_namespace,
        reference_grant_allowed,
        blocked_cross_namespace,
        service,
        endpoint,
        backends,
    }
}

fn gateway_route_node(route_ref: &GatewayRouteRef<'_>) -> RelationNode {
    let mut children = vec![leaf(&format!(
        "Hostnames {}",
        if route_ref.hostnames.is_empty() {
            "<match-all>".to_string()
        } else {
            route_ref.hostnames.join(", ")
        }
    ))];
    children.push(leaf(&format!(
        "Parent Gateway {}{}",
        route_ref.parent_ref.name,
        route_ref
            .parent_ref
            .section_name
            .as_deref()
            .map(|section| format!(" section {}", section))
            .unwrap_or_default()
    )));
    if route_ref.cross_namespace_parent_blocked {
        children.push(leaf(
            "Cross-namespace attachment may be rejected because the listener does not advertise allowedRoutes for other namespaces.",
        ));
    }
    let backend_nodes = route_ref
        .resolution_set
        .iter()
        .map(|resolution| gateway_backend_node(route_ref.route_kind, resolution))
        .collect::<Vec<_>>();
    children.push(section("Backend refs", backend_nodes));

    RelationNode {
        resource: Some(ResourceRef::CustomResource {
            name: route_ref.route_name.to_string(),
            namespace: Some(route_ref.route_namespace.to_string()),
            group: "gateway.networking.k8s.io".to_string(),
            version: route_ref.route_version.to_string(),
            kind: route_ref.route_kind.to_string(),
            plural: if route_ref.route_kind == "HTTPRoute" {
                "httproutes".to_string()
            } else {
                "grpcroutes".to_string()
            },
        }),
        label: format!("{} {}", route_ref.route_kind, route_ref.route_name),
        status: Some(format!("{} backend ref(s)", route_ref.resolution_set.len())),
        namespace: Some(route_ref.route_namespace.to_string()),
        relation: RelationKind::Backend,
        not_found: false,
        children,
    }
}

fn gateway_service_route_node(route_ref: &GatewayServiceRouteRef<'_>) -> RelationNode {
    let mut children = vec![leaf(&format!(
        "Hostnames {}",
        if route_ref.hostnames.is_empty() {
            "<match-all>".to_string()
        } else {
            route_ref.hostnames.join(", ")
        }
    ))];
    if route_ref.parent_gateways.is_empty() {
        children.push(leaf(
            "No parent Gateway currently resolves from this route.",
        ));
    } else {
        children.extend(route_ref.parent_gateways.iter().map(parent_gateway_node));
    }
    if route_ref.blocked_cross_namespace_backend {
        children.push(leaf(
            "This Service backendRef crosses namespaces without a matching ReferenceGrant.",
        ));
    } else {
        children.push(leaf("This route currently targets the selected Service."));
    }

    RelationNode {
        resource: Some(ResourceRef::CustomResource {
            name: route_ref.route_name.to_string(),
            namespace: Some(route_ref.route_namespace.to_string()),
            group: "gateway.networking.k8s.io".to_string(),
            version: route_ref.route_version.to_string(),
            kind: route_ref.route_kind.to_string(),
            plural: gateway_route_plural(route_ref.route_kind).to_string(),
        }),
        label: format!("{} {}", route_ref.route_kind, route_ref.route_name),
        status: Some(if route_ref.blocked_cross_namespace_backend {
            "selected Service blocked by missing ReferenceGrant".to_string()
        } else {
            format!("{} parent gateway(s)", route_ref.parent_gateways.len())
        }),
        namespace: Some(route_ref.route_namespace.to_string()),
        relation: RelationKind::Backend,
        not_found: false,
        children,
    }
}

fn gateway_route_plural(route_kind: &str) -> &'static str {
    if route_kind == "HTTPRoute" {
        "httproutes"
    } else {
        "grpcroutes"
    }
}

fn gateway_backend_node(
    route_kind: &str,
    resolution: &GatewayBackendResolution<'_>,
) -> RelationNode {
    let mut children = Vec::new();
    if resolution.cross_namespace {
        children.push(leaf(&format!(
            "Cross-namespace target from {route_kind} namespace to {}.",
            resolution.target_namespace
        )));
        children.push(leaf(&format!(
            "ReferenceGrant {}.",
            if resolution.reference_grant_allowed {
                "present"
            } else {
                "missing"
            }
        )));
    }
    if resolution.blocked_cross_namespace {
        children.push(leaf(
            "Cross-namespace backend is not resolved because no matching ReferenceGrant allows it.",
        ));
    }
    if let Some(endpoint) = resolution.endpoint {
        children.push(leaf(&format!(
            "Endpoints {} address(es)",
            endpoint.addresses.len()
        )));
    }
    if resolution.service.is_none() && !resolution.blocked_cross_namespace {
        children.push(leaf(
            "Referenced backend Service does not exist in the current snapshot.",
        ));
    } else if resolution.service.is_some() && resolution.backends.is_empty() {
        children.push(leaf("Backend Service resolved, but no backend pod matches its selector in the current snapshot."));
    } else if resolution.service.is_some() {
        let mut backend_nodes = resolution
            .backends
            .iter()
            .take(MAX_RENDERED_BACKENDS)
            .map(|pod| {
                if let Some(service) = resolution.service {
                    pod_backend_node(pod, service.port_mappings.as_slice())
                } else {
                    pod_backend_node(pod, &[])
                }
            })
            .collect::<Vec<_>>();
        if resolution.backends.len() > MAX_RENDERED_BACKENDS {
            backend_nodes.push(leaf(&format!(
                "... {} additional backend pod(s) omitted",
                resolution.backends.len() - MAX_RENDERED_BACKENDS
            )));
        }
        children.push(section("Backend pods", backend_nodes));
    }

    let label = format!(
        "{} {}{}",
        resolution.backend_ref.kind,
        resolution.backend_ref.name,
        resolution
            .backend_ref
            .port
            .map(|port| format!(":{port}"))
            .unwrap_or_default()
    );
    let mut node = leaf_with_resource(
        &label,
        resolution
            .service
            .map(|service| ResourceRef::Service(service.name.clone(), service.namespace.clone())),
        resolution.service.map(|service| service.type_.clone()),
        Some(resolution.target_namespace.clone()),
    );
    node.children = children;
    node
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

fn tunnel_refs_for_gateway_routes<'a>(
    route_refs: &[GatewayRouteRef<'a>],
    tunnels: &'a TunnelRegistry,
) -> Vec<PortForwardTunnelInfo> {
    let backend_names = route_refs
        .iter()
        .flat_map(|route| {
            route.resolution_set.iter().flat_map(|resolution| {
                resolution
                    .backends
                    .iter()
                    .map(|pod| (pod.namespace.as_str(), pod.name.as_str()))
            })
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

fn tunnel_refs_for_gateway_backend_resolutions<'a>(
    resolutions: &[GatewayBackendResolution<'a>],
    tunnels: &'a TunnelRegistry,
) -> Vec<PortForwardTunnelInfo> {
    let backend_names = resolutions
        .iter()
        .flat_map(|resolution| {
            resolution
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

fn gateway_routes_for_service<'a>(
    snapshot: &'a ClusterSnapshot,
    service: &'a ServiceInfo,
) -> Vec<GatewayServiceRouteRef<'a>> {
    let mut refs = snapshot
        .http_routes
        .iter()
        .filter_map(|route| {
            gateway_service_route_ref(
                GatewayRouteAnalysisInput {
                    route_kind: "HTTPRoute",
                    route_version: route.version.as_str(),
                    route_name: route.name.as_str(),
                    namespace: route.namespace.as_str(),
                    hostnames: route.hostnames.as_slice(),
                    parent_refs: route.parent_refs.as_slice(),
                    backend_refs: route.backend_refs.as_slice(),
                },
                service,
                snapshot,
            )
        })
        .collect::<Vec<_>>();
    refs.extend(snapshot.grpc_routes.iter().filter_map(|route| {
        gateway_service_route_ref(
            GatewayRouteAnalysisInput {
                route_kind: "GRPCRoute",
                route_version: route.version.as_str(),
                route_name: route.name.as_str(),
                namespace: route.namespace.as_str(),
                hostnames: route.hostnames.as_slice(),
                parent_refs: route.parent_refs.as_slice(),
                backend_refs: route.backend_refs.as_slice(),
            },
            service,
            snapshot,
        )
    }));
    refs
}

fn gateway_service_route_ref<'a>(
    route: GatewayRouteAnalysisInput<'a>,
    service: &'a ServiceInfo,
    snapshot: &'a ClusterSnapshot,
) -> Option<GatewayServiceRouteRef<'a>> {
    let matching_backend = route.backend_refs.iter().find(|backend| {
        backend.kind == "Service"
            && backend.name == service.name
            && backend.namespace.as_deref().unwrap_or(route.namespace) == service.namespace
    })?;
    let resolution = resolve_gateway_backend(
        snapshot,
        route.namespace,
        route.route_kind,
        matching_backend,
    );
    Some(GatewayServiceRouteRef {
        route_kind: route.route_kind,
        route_version: route.route_version,
        route_name: route.route_name,
        route_namespace: route.namespace,
        hostnames: route.hostnames,
        parent_gateways: route_parent_gateways(snapshot, route.namespace, route.parent_refs),
        blocked_cross_namespace_backend: resolution.blocked_cross_namespace,
    })
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

fn find_gateway<'a>(
    snapshot: &'a ClusterSnapshot,
    name: &str,
    namespace: &str,
) -> Result<&'a GatewayInfo, String> {
    snapshot
        .gateways
        .iter()
        .find(|gateway| gateway.name == name && gateway.namespace == namespace)
        .ok_or_else(|| format!("Gateway '{namespace}/{name}' is no longer in the snapshot."))
}

fn find_http_route<'a>(
    snapshot: &'a ClusterSnapshot,
    name: &str,
    namespace: &str,
) -> Result<&'a HttpRouteInfo, String> {
    snapshot
        .http_routes
        .iter()
        .find(|route| route.name == name && route.namespace == namespace)
        .ok_or_else(|| format!("HTTPRoute '{namespace}/{name}' is no longer in the snapshot."))
}

fn find_grpc_route<'a>(
    snapshot: &'a ClusterSnapshot,
    name: &str,
    namespace: &str,
) -> Result<&'a GrpcRouteInfo, String> {
    snapshot
        .grpc_routes
        .iter()
        .find(|route| route.name == name && route.namespace == namespace)
        .ok_or_else(|| format!("GRPCRoute '{namespace}/{name}' is no longer in the snapshot."))
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

#[derive(Debug, Clone)]
struct GatewayRouteRef<'a> {
    route_kind: &'static str,
    route_version: &'a str,
    route_name: &'a str,
    route_namespace: &'a str,
    hostnames: &'a [String],
    parent_ref: &'a GatewayParentRefInfo,
    resolution_set: Vec<GatewayBackendResolution<'a>>,
    cross_namespace_parent_blocked: bool,
}

#[derive(Debug, Clone)]
struct GatewayParentGatewayRef<'a> {
    parent: &'a GatewayParentRefInfo,
    namespace: String,
    gateway: Option<&'a GatewayInfo>,
}

#[derive(Debug, Clone)]
struct GatewayBackendResolution<'a> {
    backend_ref: &'a GatewayBackendRefInfo,
    target_namespace: String,
    cross_namespace: bool,
    reference_grant_allowed: bool,
    blocked_cross_namespace: bool,
    service: Option<&'a ServiceInfo>,
    endpoint: Option<&'a EndpointInfo>,
    backends: Vec<&'a PodInfo>,
}

#[derive(Debug, Clone)]
struct GatewayServiceRouteRef<'a> {
    route_kind: &'static str,
    route_version: &'a str,
    route_name: &'a str,
    route_namespace: &'a str,
    hostnames: &'a [String],
    parent_gateways: Vec<GatewayParentGatewayRef<'a>>,
    blocked_cross_namespace_backend: bool,
}

#[derive(Debug, Clone, Copy)]
struct GatewayRouteAnalysisInput<'a> {
    route_kind: &'static str,
    route_version: &'a str,
    route_name: &'a str,
    namespace: &'a str,
    hostnames: &'a [String],
    parent_refs: &'a [GatewayParentRefInfo],
    backend_refs: &'a [GatewayBackendRefInfo],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        k8s::{
            dtos::{
                ContainerPortInfo, GatewayBackendRefInfo, GatewayInfo, GatewayListenerInfo,
                GatewayParentRefInfo, HttpRouteInfo, IngressInfo, IngressRouteInfo,
                LabelSelectorInfo, ReferenceGrantFromInfo, ReferenceGrantInfo,
                ReferenceGrantToInfo,
            },
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

    fn gateway() -> GatewayInfo {
        GatewayInfo {
            name: "edge".to_string(),
            namespace: "shared".to_string(),
            version: "v1beta1".to_string(),
            gateway_class_name: "istio".to_string(),
            listeners: vec![GatewayListenerInfo {
                name: "http".to_string(),
                protocol: "HTTP".to_string(),
                port: 80,
                hostname: Some("app.example.test".to_string()),
                attached_routes: 1,
                ready: Some(true),
                allowed_routes_from: Some("Same".to_string()),
                allowed_routes_selector: None,
            }],
            ..GatewayInfo::default()
        }
    }

    fn http_route(namespace: &str) -> HttpRouteInfo {
        HttpRouteInfo {
            name: "frontend".to_string(),
            namespace: namespace.to_string(),
            version: "v1beta1".to_string(),
            hostnames: vec!["app.example.test".to_string()],
            parent_refs: vec![GatewayParentRefInfo {
                group: "gateway.networking.k8s.io".to_string(),
                kind: "Gateway".to_string(),
                namespace: Some("shared".to_string()),
                name: "edge".to_string(),
                section_name: Some("http".to_string()),
            }],
            backend_refs: vec![GatewayBackendRefInfo {
                group: "".to_string(),
                kind: "Service".to_string(),
                namespace: Some("backend".to_string()),
                name: "api".to_string(),
                port: Some(80),
            }],
            ..HttpRouteInfo::default()
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

    #[test]
    fn gateway_route_analysis_blocks_cross_namespace_backend_without_reference_grant() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.gateways.push(gateway());
        snapshot.http_routes.push(http_route("apps"));
        snapshot.services.push(ServiceInfo {
            name: "api".to_string(),
            namespace: "backend".to_string(),
            type_: "ClusterIP".to_string(),
            selector: BTreeMap::from([("app".to_string(), "api".to_string())]),
            ..ServiceInfo::default()
        });
        snapshot.pods.push(pod("api-0", "backend", "10.42.0.8"));

        let analysis = analyze_resource(
            &ResourceRef::CustomResource {
                name: "frontend".to_string(),
                namespace: Some("apps".to_string()),
                group: "gateway.networking.k8s.io".to_string(),
                version: "v1beta1".to_string(),
                kind: "HTTPRoute".to_string(),
                plural: "httproutes".to_string(),
            },
            &snapshot,
            &TunnelRegistry::new(),
        )
        .unwrap();

        assert!(
            analysis
                .summary_lines
                .iter()
                .any(|line| line.contains("without a matching ReferenceGrant"))
        );
        let backend_section = analysis
            .tree
            .iter()
            .find(|node| node.label == "Backend trace")
            .unwrap();
        let backend_node = &backend_section.children[0];
        assert_eq!(backend_node.label, "Service api:80");
        assert!(
            backend_node
                .children
                .iter()
                .any(|child| child.label.contains("no matching ReferenceGrant"))
        );
        assert!(
            backend_node
                .children
                .iter()
                .all(|child| child.label != "Backend pods")
        );
    }

    #[test]
    fn gateway_route_refs_preserve_route_version_and_reference_grant_status() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.gateways.push(gateway());
        snapshot.http_routes.push(http_route("apps"));
        snapshot.reference_grants.push(ReferenceGrantInfo {
            name: "grant".to_string(),
            namespace: "backend".to_string(),
            version: "v1alpha2".to_string(),
            from: vec![ReferenceGrantFromInfo {
                group: "gateway.networking.k8s.io".to_string(),
                kind: "HTTPRoute".to_string(),
                namespace: "apps".to_string(),
            }],
            to: vec![ReferenceGrantToInfo {
                group: "".to_string(),
                kind: "Service".to_string(),
                name: Some("api".to_string()),
            }],
            ..ReferenceGrantInfo::default()
        });

        let route_refs = gateway_route_refs(&snapshot, &snapshot.gateways[0]);
        assert_eq!(route_refs.len(), 1);
        assert_eq!(route_refs[0].route_version, "v1beta1");
        assert!(route_refs[0].resolution_set[0].reference_grant_allowed);
        assert!(!route_refs[0].resolution_set[0].blocked_cross_namespace);
    }

    #[test]
    fn gateway_route_refs_allow_cross_namespace_parent_without_section_when_any_listener_allows_it()
    {
        let mut snapshot = ClusterSnapshot::default();
        let mut gateway = gateway();
        gateway.listeners[0].allowed_routes_from = Some("All".to_string());
        snapshot.gateways.push(gateway);

        let mut route = http_route("apps");
        route.parent_refs[0].section_name = None;
        snapshot.http_routes.push(route);

        let route_refs = gateway_route_refs(&snapshot, &snapshot.gateways[0]);
        assert_eq!(route_refs.len(), 1);
        assert!(!route_refs[0].cross_namespace_parent_blocked);
    }

    #[test]
    fn gateway_route_refs_treat_selector_policy_as_non_widening_without_namespace_metadata() {
        let mut snapshot = ClusterSnapshot::default();
        let mut gateway = gateway();
        gateway.listeners[0].allowed_routes_from = Some("Selector".to_string());
        gateway.listeners[0].allowed_routes_selector = Some(LabelSelectorInfo {
            match_labels: BTreeMap::from([("team".to_string(), "edge".to_string())]),
            match_expressions: Vec::new(),
        });
        snapshot.gateways.push(gateway);
        snapshot.http_routes.push(http_route("apps"));

        let route_refs = gateway_route_refs(&snapshot, &snapshot.gateways[0]);
        assert_eq!(route_refs.len(), 1);
        assert!(route_refs[0].cross_namespace_parent_blocked);
    }

    #[test]
    fn gateway_route_refs_prefer_allowed_parent_when_same_gateway_is_referenced_twice() {
        let mut snapshot = ClusterSnapshot::default();
        let mut gateway = gateway();
        gateway.listeners.push(GatewayListenerInfo {
            name: "public".to_string(),
            protocol: "HTTP".to_string(),
            port: 8080,
            hostname: None,
            allowed_routes_from: Some("All".to_string()),
            allowed_routes_selector: None,
            attached_routes: 1,
            ready: Some(true),
        });
        snapshot.gateways.push(gateway);

        let mut route = http_route("apps");
        route.parent_refs = vec![
            GatewayParentRefInfo {
                section_name: Some("http".to_string()),
                ..route.parent_refs[0].clone()
            },
            GatewayParentRefInfo {
                section_name: Some("public".to_string()),
                ..route.parent_refs[0].clone()
            },
        ];
        snapshot.http_routes.push(route);

        let route_refs = gateway_route_refs(&snapshot, &snapshot.gateways[0]);
        assert_eq!(route_refs.len(), 1);
        assert_eq!(
            route_refs[0].parent_ref.section_name.as_deref(),
            Some("public")
        );
        assert!(!route_refs[0].cross_namespace_parent_blocked);
    }

    #[test]
    fn service_analysis_includes_gateway_routes_for_selected_service() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.services.push(ServiceInfo {
            name: "api".to_string(),
            namespace: "backend".to_string(),
            type_: "ClusterIP".to_string(),
            selector: BTreeMap::from([("app".to_string(), "api".to_string())]),
            ..ServiceInfo::default()
        });
        snapshot.gateways.push(gateway());
        snapshot.http_routes.push(http_route("apps"));

        let analysis = analyze_resource(
            &ResourceRef::Service("api".to_string(), "backend".to_string()),
            &snapshot,
            &TunnelRegistry::new(),
        )
        .unwrap();

        assert!(
            analysis
                .summary_lines
                .iter()
                .any(|line| line.contains("1 gateway route(s)"))
        );
        assert!(
            analysis
                .tree
                .iter()
                .any(|section| section.label == "Gateway routes")
        );
    }
}
