//! Core data model and tree flattening for the Relationship Explorer.

use std::collections::HashSet;

use crate::app::{AppView, ResourceRef};
use crate::k8s::dtos::OwnerRefInfo;
use crate::k8s::gateway_semantics::{
    gateway_parent_attachment_allowed, reference_grant_allows_backend,
    select_gateway_parent_attachment,
};
use crate::policy::RelationshipCapability;
use crate::state::ClusterSnapshot;

/// Safety limit for owner chain traversal to prevent infinite loops from
/// circular ownerReferences in corrupt snapshots.
const MAX_OWNER_CHAIN_DEPTH: usize = 20;

/// A node in the relationship tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationNode {
    /// None for section headers.
    pub resource: Option<ResourceRef>,
    /// Display label, e.g. "Deployment nginx-deployment".
    pub label: String,
    /// e.g. "Ready", "Running", "3/3".
    pub status: Option<String>,
    pub namespace: Option<String>,
    pub relation: RelationKind,
    /// True for unresolvable references.
    pub not_found: bool,
    pub children: Vec<RelationNode>,
}

/// How a node relates to the root resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    Root,
    Owner,
    Owned,
    SelectedBy,
    Backend,
    Bound,
    FluxSource,
    RbacBinding,
    SectionHeader,
}

/// A flattened, render-ready node produced by [`flatten_tree`].
#[derive(Debug, Clone)]
pub struct FlatNode {
    pub depth: usize,
    /// Stable index assigned by depth-first pre-order traversal.
    pub tree_index: usize,
    pub resource: Option<ResourceRef>,
    pub label: String,
    pub status: Option<String>,
    pub namespace: Option<String>,
    pub relation: RelationKind,
    pub not_found: bool,
    pub is_last_child: bool,
    /// Whether each ancestor in the path is the last child at its level.
    pub parent_is_last: Vec<bool>,
    pub has_children: bool,
    pub expanded: bool,
}

/// Flatten `nodes` into visible lines based on the `expanded` set.
///
/// Each node receives a stable `tree_index` via depth-first pre-order
/// traversal. Collapsed parents skip rendering their children but still
/// consume indices so that toggling one subtree does not shift sibling
/// indices.
pub fn flatten_tree(nodes: &[RelationNode], expanded: &HashSet<usize>) -> Vec<FlatNode> {
    let mut result = Vec::new();
    let mut counter = 0usize;
    flatten_recursive(nodes, expanded, 0, &[], &mut counter, &mut result);
    result
}

fn flatten_recursive(
    nodes: &[RelationNode],
    expanded: &HashSet<usize>,
    depth: usize,
    parent_is_last: &[bool],
    counter: &mut usize,
    result: &mut Vec<FlatNode>,
) {
    let last_idx = nodes.len().saturating_sub(1);
    for (i, node) in nodes.iter().enumerate() {
        let tree_index = *counter;
        *counter += 1;

        let is_last = i == last_idx;
        let is_expanded = expanded.contains(&tree_index);
        let has_children = !node.children.is_empty();

        result.push(FlatNode {
            depth,
            tree_index,
            resource: node.resource.clone(),
            label: node.label.clone(),
            status: node.status.clone(),
            namespace: node.namespace.clone(),
            relation: node.relation,
            not_found: node.not_found,
            is_last_child: is_last,
            parent_is_last: parent_is_last.to_vec(),
            has_children,
            expanded: is_expanded,
        });

        // Build the ancestor chain for children.
        let mut child_parent_is_last = parent_is_last.to_vec();
        child_parent_is_last.push(is_last);

        if is_expanded {
            // Render children.
            flatten_recursive(
                &node.children,
                expanded,
                depth + 1,
                &child_parent_is_last,
                counter,
                result,
            );
        } else {
            // Still count all descendant indices for stability.
            count_descendants(&node.children, counter);
        }
    }
}

/// Advance `counter` by the number of descendants without emitting anything.
pub fn count_descendants(nodes: &[RelationNode], counter: &mut usize) {
    for node in nodes {
        *counter += 1;
        count_descendants(&node.children, counter);
    }
}

/// Map a [`ResourceRef`] to the [`AppView`] that owns it, for capability lookup.
///
/// Returns `None` for resources that have no relationship support.
pub fn resource_to_view(resource: &ResourceRef) -> Option<AppView> {
    match resource {
        ResourceRef::Pod(_, _) => Some(AppView::Pods),
        ResourceRef::Deployment(_, _) => Some(AppView::Deployments),
        ResourceRef::StatefulSet(_, _) => Some(AppView::StatefulSets),
        ResourceRef::DaemonSet(_, _) => Some(AppView::DaemonSets),
        ResourceRef::ReplicaSet(_, _) => Some(AppView::ReplicaSets),
        ResourceRef::ReplicationController(_, _) => Some(AppView::ReplicationControllers),
        ResourceRef::Job(_, _) => Some(AppView::Jobs),
        ResourceRef::CronJob(_, _) => Some(AppView::CronJobs),
        ResourceRef::Service(_, _) => Some(AppView::Services),
        ResourceRef::Endpoint(_, _) => Some(AppView::Endpoints),
        ResourceRef::Ingress(_, _) => Some(AppView::Ingresses),
        ResourceRef::IngressClass(_) => Some(AppView::IngressClasses),
        ResourceRef::CustomResource { group, kind, .. }
            if group == "gateway.networking.k8s.io" && kind == "GatewayClass" =>
        {
            Some(AppView::GatewayClasses)
        }
        ResourceRef::CustomResource { group, kind, .. }
            if group == "gateway.networking.k8s.io" && kind == "Gateway" =>
        {
            Some(AppView::Gateways)
        }
        ResourceRef::CustomResource { group, kind, .. }
            if group == "gateway.networking.k8s.io" && kind == "HTTPRoute" =>
        {
            Some(AppView::HttpRoutes)
        }
        ResourceRef::CustomResource { group, kind, .. }
            if group == "gateway.networking.k8s.io" && kind == "GRPCRoute" =>
        {
            Some(AppView::GrpcRoutes)
        }
        ResourceRef::CustomResource { group, kind, .. }
            if group == "gateway.networking.k8s.io" && kind == "ReferenceGrant" =>
        {
            Some(AppView::ReferenceGrants)
        }
        ResourceRef::Pvc(_, _) => Some(AppView::PersistentVolumeClaims),
        ResourceRef::Pv(_) => Some(AppView::PersistentVolumes),
        ResourceRef::StorageClass(_) => Some(AppView::StorageClasses),
        ResourceRef::ServiceAccount(_, _) => Some(AppView::ServiceAccounts),
        ResourceRef::ClusterRole(_) => Some(AppView::ClusterRoles),
        ResourceRef::Role(_, _) => Some(AppView::Roles),
        ResourceRef::ClusterRoleBinding(_) => Some(AppView::ClusterRoleBindings),
        ResourceRef::RoleBinding(_, _) => Some(AppView::RoleBindings),
        ResourceRef::CustomResource { group, .. } if group.ends_with(".fluxcd.io") => {
            Some(AppView::FluxCDAll)
        }
        // No relationship support for these resource types.
        ResourceRef::Node(_)
        | ResourceRef::ConfigMap(_, _)
        | ResourceRef::Secret(_, _)
        | ResourceRef::Namespace(_)
        | ResourceRef::NetworkPolicy(_, _)
        | ResourceRef::ResourceQuota(_, _)
        | ResourceRef::LimitRange(_, _)
        | ResourceRef::PodDisruptionBudget(_, _)
        | ResourceRef::Hpa(_, _)
        | ResourceRef::PriorityClass(_)
        | ResourceRef::Event(_, _)
        | ResourceRef::HelmRelease(_, _)
        | ResourceRef::CustomResource { .. } => None,
    }
}

/// Returns `true` when `resource` maps to a view with at least one
/// relationship capability.
pub fn resource_has_relationships(resource: &ResourceRef) -> bool {
    resource_to_view(resource)
        .map(|v| !v.relationship_capabilities().is_empty())
        .unwrap_or(false)
}

impl RelationshipCapability {
    pub const fn section_title(self) -> &'static str {
        match self {
            RelationshipCapability::OwnerChain => "Owner Chain",
            RelationshipCapability::ServiceBackends => "Service Backends",
            RelationshipCapability::IngressBackends => "Ingress Backends",
            RelationshipCapability::GatewayRoutes => "Gateway Topology",
            RelationshipCapability::StorageBindings => "Storage Bindings",
            RelationshipCapability::FluxLineage => "Flux Lineage",
            RelationshipCapability::RbacBindings => "RBAC Bindings",
        }
    }
}

// ---------------------------------------------------------------------------
// Task 10: Owner chain resolver
// ---------------------------------------------------------------------------

/// Get owner references for any supported resource type.
fn get_owner_refs(resource: &ResourceRef, snapshot: &ClusterSnapshot) -> Vec<OwnerRefInfo> {
    match resource {
        ResourceRef::Pod(name, ns) => snapshot
            .pods
            .iter()
            .find(|p| &p.name == name && &p.namespace == ns)
            .map(|p| p.owner_references.clone())
            .unwrap_or_default(),
        ResourceRef::ReplicaSet(name, ns) => snapshot
            .replicasets
            .iter()
            .find(|r| &r.name == name && &r.namespace == ns)
            .map(|r| r.owner_references.clone())
            .unwrap_or_default(),
        ResourceRef::Job(name, ns) => snapshot
            .jobs
            .iter()
            .find(|j| &j.name == name && &j.namespace == ns)
            .map(|j| j.owner_references.clone())
            .unwrap_or_default(),
        // All other types don't have owner_references in the DTO.
        _ => vec![],
    }
}

/// Retrieve a resource's status string from the snapshot.
fn resource_status(resource: &ResourceRef, snapshot: &ClusterSnapshot) -> Option<String> {
    match resource {
        ResourceRef::Pod(name, ns) => snapshot
            .pods
            .iter()
            .find(|p| &p.name == name && &p.namespace == ns)
            .map(|p| p.status.clone()),
        ResourceRef::Deployment(name, ns) => snapshot
            .deployments
            .iter()
            .find(|d| &d.name == name && &d.namespace == ns)
            .map(|d| d.ready.clone()),
        ResourceRef::ReplicaSet(name, ns) => snapshot
            .replicasets
            .iter()
            .find(|r| &r.name == name && &r.namespace == ns)
            .map(|r| format!("{}/{}", r.ready, r.desired)),
        ResourceRef::Job(name, ns) => snapshot
            .jobs
            .iter()
            .find(|j| &j.name == name && &j.namespace == ns)
            .map(|j| j.status.clone()),
        ResourceRef::StatefulSet(name, ns) => snapshot
            .statefulsets
            .iter()
            .find(|s| &s.name == name && &s.namespace == ns)
            .map(|s| format!("{}/{}", s.ready_replicas, s.desired_replicas)),
        ResourceRef::DaemonSet(name, ns) => snapshot
            .daemonsets
            .iter()
            .find(|d| &d.name == name && &d.namespace == ns)
            .map(|d| d.status_message.clone()),
        ResourceRef::CronJob(name, ns) => snapshot
            .cronjobs
            .iter()
            .find(|c| &c.name == name && &c.namespace == ns)
            .map(|c| {
                if c.suspend {
                    "Suspended".to_string()
                } else {
                    format!("{} active", c.active_jobs)
                }
            }),
        _ => None,
    }
}

/// Create a RelationNode for a known ResourceRef.
fn make_node(
    resource: ResourceRef,
    snapshot: &ClusterSnapshot,
    relation: RelationKind,
) -> RelationNode {
    let label = format!("{} {}", resource.kind(), resource.name());
    let status = resource_status(&resource, snapshot);
    let namespace = resource.namespace().map(|s| s.to_string());
    RelationNode {
        resource: Some(resource),
        label,
        status,
        namespace,
        relation,
        not_found: false,
        children: vec![],
    }
}

/// Create a not_found placeholder node for an unresolvable owner reference.
fn make_not_found_node(oref: &OwnerRefInfo, relation: RelationKind) -> RelationNode {
    RelationNode {
        resource: None,
        label: format!("{} {}", oref.kind, oref.name),
        status: None,
        namespace: None,
        relation,
        not_found: true,
        children: vec![],
    }
}

/// Find the ResourceRef for an owner reference in the snapshot (same namespace).
fn find_resource_for_owner_ref(
    oref: &OwnerRefInfo,
    namespace: &str,
    snapshot: &ClusterSnapshot,
) -> Option<ResourceRef> {
    match oref.kind.as_str() {
        "ReplicaSet" => snapshot
            .replicasets
            .iter()
            .find(|r| r.name == oref.name && r.namespace == namespace)
            .map(|r| ResourceRef::ReplicaSet(r.name.clone(), r.namespace.clone())),
        "Deployment" => snapshot
            .deployments
            .iter()
            .find(|d| d.name == oref.name && d.namespace == namespace)
            .map(|d| ResourceRef::Deployment(d.name.clone(), d.namespace.clone())),
        "StatefulSet" => snapshot
            .statefulsets
            .iter()
            .find(|s| s.name == oref.name && s.namespace == namespace)
            .map(|s| ResourceRef::StatefulSet(s.name.clone(), s.namespace.clone())),
        "DaemonSet" => snapshot
            .daemonsets
            .iter()
            .find(|d| d.name == oref.name && d.namespace == namespace)
            .map(|d| ResourceRef::DaemonSet(d.name.clone(), d.namespace.clone())),
        "Job" => snapshot
            .jobs
            .iter()
            .find(|j| j.name == oref.name && j.namespace == namespace)
            .map(|j| ResourceRef::Job(j.name.clone(), j.namespace.clone())),
        "CronJob" => snapshot
            .cronjobs
            .iter()
            .find(|c| c.name == oref.name && c.namespace == namespace)
            .map(|c| ResourceRef::CronJob(c.name.clone(), c.namespace.clone())),
        "ReplicationController" => snapshot
            .replication_controllers
            .iter()
            .find(|r| r.name == oref.name && r.namespace == namespace)
            .map(|r| ResourceRef::ReplicationController(r.name.clone(), r.namespace.clone())),
        _ => None,
    }
}

/// Find all resources in the snapshot that are owned by `resource`.
fn find_owned_resources(resource: &ResourceRef, snapshot: &ClusterSnapshot) -> Vec<ResourceRef> {
    let name = resource.name();
    let kind = resource.kind();
    let ns = resource.namespace().unwrap_or("");

    let mut owned = Vec::new();

    // Check pods
    for pod in &snapshot.pods {
        if pod.namespace == ns {
            for oref in &pod.owner_references {
                if oref.name == name && oref.kind == kind {
                    owned.push(ResourceRef::Pod(pod.name.clone(), pod.namespace.clone()));
                }
            }
        }
    }

    // Check replicasets
    for rs in &snapshot.replicasets {
        if rs.namespace == ns {
            for oref in &rs.owner_references {
                if oref.name == name && oref.kind == kind {
                    owned.push(ResourceRef::ReplicaSet(
                        rs.name.clone(),
                        rs.namespace.clone(),
                    ));
                }
            }
        }
    }

    // Check jobs
    for job in &snapshot.jobs {
        if job.namespace == ns {
            for oref in &job.owner_references {
                if oref.name == name && oref.kind == kind {
                    owned.push(ResourceRef::Job(job.name.clone(), job.namespace.clone()));
                }
            }
        }
    }

    owned
}

/// Represents an entry in the owner chain walk — either a resolved resource
/// or an unresolvable owner reference.
enum ChainEntry {
    Resolved(ResourceRef),
    NotFound(OwnerRefInfo),
}

/// Walk owner references upward from a resource, returning the chain
/// top-down (root owner first). Also finds resources owned by the target.
pub fn resolve_owner_chain_from_snapshot(
    resource: &ResourceRef,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    let ns = match resource.namespace() {
        Some(ns) => ns.to_string(),
        None => return vec![],
    };

    // Walk up the owner chain, collecting entries.
    // We stop when there are no more owner refs, we can't find the owner,
    // or we hit the depth limit (cycle protection).
    let mut chain: Vec<ChainEntry> = vec![ChainEntry::Resolved(resource.clone())];
    let mut current = resource.clone();

    for _ in 0..MAX_OWNER_CHAIN_DEPTH {
        let owner_refs = get_owner_refs(&current, snapshot);
        if owner_refs.is_empty() {
            break;
        }
        // Take the first owner ref (typical case)
        let oref = &owner_refs[0];
        match find_resource_for_owner_ref(oref, &ns, snapshot) {
            Some(parent_ref) => {
                // Cycle detection: check if we've already visited this resource
                let already_seen = chain
                    .iter()
                    .any(|entry| matches!(entry, ChainEntry::Resolved(r) if r == &parent_ref));
                if already_seen {
                    break;
                }
                chain.push(ChainEntry::Resolved(parent_ref.clone()));
                current = parent_ref;
            }
            None => {
                chain.push(ChainEntry::NotFound(oref.clone()));
                break;
            }
        }
    }

    // chain is bottom-up; reverse to top-down
    chain.reverse();

    // Build owned children for the original resource
    let target_owned: Vec<RelationNode> = find_owned_resources(resource, snapshot)
        .into_iter()
        .map(|r| make_node(r, snapshot, RelationKind::Owned))
        .collect();

    // Build the nested tree top-down.
    fn build_chain_tree(
        chain: &[ChainEntry],
        snapshot: &ClusterSnapshot,
        target: &ResourceRef,
        target_owned: Vec<RelationNode>,
    ) -> Vec<RelationNode> {
        if chain.is_empty() {
            return vec![];
        }

        match &chain[0] {
            ChainEntry::NotFound(oref) => {
                let mut n = make_not_found_node(oref, RelationKind::Owner);
                if chain.len() > 1 {
                    n.children = build_chain_tree(&chain[1..], snapshot, target, target_owned);
                }
                vec![n]
            }
            ChainEntry::Resolved(res) => {
                let is_target = res == target;

                let node = if is_target {
                    let mut n = make_node(res.clone(), snapshot, RelationKind::Root);
                    n.children = target_owned;
                    n
                } else {
                    let mut n = make_node(res.clone(), snapshot, RelationKind::Owner);
                    if chain.len() > 1 {
                        n.children = build_chain_tree(&chain[1..], snapshot, target, target_owned);
                    }
                    n
                };

                vec![node]
            }
        }
    }

    build_chain_tree(&chain, snapshot, resource, target_owned)
}

// ---------------------------------------------------------------------------
// Task 11: Service backends resolver
// ---------------------------------------------------------------------------

/// Match pods to a service by its selector labels (same namespace, all labels must match).
pub fn resolve_service_backends_from_snapshot(
    resource: &ResourceRef,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    match resource {
        ResourceRef::Service(name, ns) => {
            let Some(svc) = snapshot
                .services
                .iter()
                .find(|s| &s.name == name && &s.namespace == ns)
            else {
                return vec![];
            };

            let selector_pods = if svc.selector.is_empty() {
                Vec::new()
            } else {
                pods_matching_selector(&svc.selector, ns, snapshot)
            };
            let Some(endpoint_node) = endpoint_node_for_service(name, ns, snapshot, selector_pods)
            else {
                return vec![];
            };

            let svc_node = RelationNode {
                resource: Some(ResourceRef::Service(name.clone(), ns.clone())),
                label: format!("Service {name}"),
                status: None,
                namespace: Some(ns.clone()),
                relation: RelationKind::Root,
                not_found: false,
                children: vec![endpoint_node],
            };
            vec![svc_node]
        }
        ResourceRef::Endpoint(name, ns) => {
            let Some(endpoint) = snapshot
                .endpoints
                .iter()
                .find(|ep| &ep.name == name && &ep.namespace == ns)
            else {
                return vec![];
            };

            let mut backend_pods =
                pods_matching_endpoint_addresses(&endpoint.addresses, ns, snapshot);
            if backend_pods.is_empty()
                && let Some(service) = snapshot
                    .services
                    .iter()
                    .find(|svc| &svc.name == name && &svc.namespace == ns)
                && !service.selector.is_empty()
            {
                backend_pods = pods_matching_selector(&service.selector, ns, snapshot);
            }

            vec![RelationNode {
                resource: Some(ResourceRef::Endpoint(name.clone(), ns.clone())),
                label: format!("Endpoints {name}"),
                status: Some(format!("{} addresses", endpoint.addresses.len())),
                namespace: Some(ns.clone()),
                relation: RelationKind::Root,
                not_found: false,
                children: backend_pods,
            }]
        }
        _ => vec![],
    }
}

/// Return RelationNodes for pods whose labels match all entries in `selector`.
fn pods_matching_selector(
    selector: &std::collections::BTreeMap<String, String>,
    namespace: &str,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    snapshot
        .pods
        .iter()
        .filter(|pod| {
            pod.namespace == namespace
                && selector
                    .iter()
                    .all(|(k, v)| pod.labels.iter().any(|(pk, pv)| pk == k && pv == v))
        })
        .map(|pod| RelationNode {
            resource: Some(ResourceRef::Pod(pod.name.clone(), pod.namespace.clone())),
            label: format!("Pod {}", pod.name),
            status: Some(pod.status.clone()),
            namespace: Some(pod.namespace.clone()),
            relation: RelationKind::Backend,
            not_found: false,
            children: vec![],
        })
        .collect()
}

fn pods_matching_endpoint_addresses(
    addresses: &[String],
    namespace: &str,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    snapshot
        .pods
        .iter()
        .filter(|pod| {
            pod.namespace == namespace
                && pod
                    .pod_ip
                    .as_ref()
                    .is_some_and(|ip| addresses.iter().any(|address| address == ip))
        })
        .map(|pod| RelationNode {
            resource: Some(ResourceRef::Pod(pod.name.clone(), pod.namespace.clone())),
            label: format!("Pod {}", pod.name),
            status: Some(pod.status.clone()),
            namespace: Some(pod.namespace.clone()),
            relation: RelationKind::Backend,
            not_found: false,
            children: vec![],
        })
        .collect()
}

fn endpoint_node_for_service(
    name: &str,
    namespace: &str,
    snapshot: &ClusterSnapshot,
    fallback_pods: Vec<RelationNode>,
) -> Option<RelationNode> {
    let endpoint = snapshot
        .endpoints
        .iter()
        .find(|ep| ep.name == name && ep.namespace == namespace);
    let endpoint_pods = endpoint
        .map(|ep| pods_matching_endpoint_addresses(&ep.addresses, namespace, snapshot))
        .unwrap_or_default();
    let children = if endpoint_pods.is_empty() {
        fallback_pods
    } else {
        endpoint_pods
    };

    if endpoint.is_none() && children.is_empty() {
        return None;
    }

    Some(RelationNode {
        resource: endpoint.map(|_| ResourceRef::Endpoint(name.to_string(), namespace.to_string())),
        label: format!("Endpoints {name}"),
        status: endpoint.map(|ep| format!("{} addresses", ep.addresses.len())),
        namespace: Some(namespace.to_string()),
        relation: RelationKind::Backend,
        not_found: endpoint.is_none(),
        children,
    })
}

// ---------------------------------------------------------------------------
// Task 12: Ingress backends + storage bindings resolvers
// ---------------------------------------------------------------------------

/// Resolve ingress backend services (and their pods) or find ingresses for an IngressClass.
pub fn resolve_ingress_backends_from_snapshot(
    resource: &ResourceRef,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    match resource {
        ResourceRef::Ingress(name, ns) => {
            let Some(ing) = snapshot
                .ingresses
                .iter()
                .find(|i| &i.name == name && &i.namespace == ns)
            else {
                return vec![];
            };

            let service_nodes: Vec<RelationNode> = ing
                .backend_services
                .iter()
                .map(|(svc_name, _port)| {
                    // Try to find the service in snapshot
                    if let Some(svc) = snapshot
                        .services
                        .iter()
                        .find(|s| &s.name == svc_name && &s.namespace == ns)
                    {
                        let pod_children = pods_matching_selector(&svc.selector, ns, snapshot);
                        RelationNode {
                            resource: Some(ResourceRef::Service(svc_name.clone(), ns.clone())),
                            label: format!("Service {svc_name}"),
                            status: None,
                            namespace: Some(ns.clone()),
                            relation: RelationKind::Backend,
                            not_found: false,
                            children: pod_children,
                        }
                    } else {
                        RelationNode {
                            resource: None,
                            label: format!("Service {svc_name}"),
                            status: None,
                            namespace: Some(ns.clone()),
                            relation: RelationKind::Backend,
                            not_found: true,
                            children: vec![],
                        }
                    }
                })
                .collect();

            if service_nodes.is_empty() {
                return vec![];
            }

            let ing_node = RelationNode {
                resource: Some(ResourceRef::Ingress(name.clone(), ns.clone())),
                label: format!("Ingress {name}"),
                status: None,
                namespace: Some(ns.clone()),
                relation: RelationKind::Root,
                not_found: false,
                children: service_nodes,
            };
            vec![ing_node]
        }
        ResourceRef::IngressClass(class_name) => {
            // Find all ingresses using this class
            let ingress_nodes: Vec<RelationNode> = snapshot
                .ingresses
                .iter()
                .filter(|i| i.class.as_deref() == Some(class_name.as_str()))
                .map(|i| RelationNode {
                    resource: Some(ResourceRef::Ingress(i.name.clone(), i.namespace.clone())),
                    label: format!("Ingress {}", i.name),
                    status: None,
                    namespace: Some(i.namespace.clone()),
                    relation: RelationKind::Owned,
                    not_found: false,
                    children: vec![],
                })
                .collect();

            if ingress_nodes.is_empty() {
                return vec![];
            }

            let class_node = RelationNode {
                resource: Some(ResourceRef::IngressClass(class_name.clone())),
                label: format!("IngressClass {class_name}"),
                status: None,
                namespace: None,
                relation: RelationKind::Root,
                not_found: false,
                children: ingress_nodes,
            };
            vec![class_node]
        }
        _ => vec![],
    }
}

pub fn resolve_gateway_topology_from_snapshot(
    resource: &ResourceRef,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    match resource {
        ResourceRef::CustomResource {
            name,
            group,
            kind,
            version,
            ..
        } if group == "gateway.networking.k8s.io" && kind == "GatewayClass" => {
            let gateways = snapshot
                .gateways
                .iter()
                .filter(|gateway| gateway.gateway_class_name == *name)
                .map(|gateway| RelationNode {
                    resource: Some(gateway_custom_resource_ref(
                        &gateway.name,
                        Some(&gateway.namespace),
                        &gateway.version,
                        "Gateway",
                        "gateways",
                    )),
                    label: format!("Gateway {}", gateway.name),
                    status: Some(format!("{} listener(s)", gateway.listeners.len())),
                    namespace: Some(gateway.namespace.clone()),
                    relation: RelationKind::Owned,
                    not_found: false,
                    children: Vec::new(),
                })
                .collect::<Vec<_>>();
            if gateways.is_empty() {
                return Vec::new();
            }
            vec![RelationNode {
                resource: Some(gateway_custom_resource_ref(
                    name,
                    None,
                    version,
                    "GatewayClass",
                    "gatewayclasses",
                )),
                label: format!("GatewayClass {name}"),
                status: None,
                namespace: None,
                relation: RelationKind::Root,
                not_found: false,
                children: gateways,
            }]
        }
        ResourceRef::CustomResource {
            name,
            namespace,
            group,
            version,
            kind,
            ..
        } if group == "gateway.networking.k8s.io" && kind == "Gateway" => {
            let Some(namespace) = namespace.as_deref() else {
                return Vec::new();
            };
            let Some(gateway) = snapshot
                .gateways
                .iter()
                .find(|gateway| gateway.name == *name && gateway.namespace == namespace)
            else {
                return Vec::new();
            };
            let mut children = Vec::new();
            for route in &snapshot.http_routes {
                let matching_parent_refs = route
                    .parent_refs
                    .iter()
                    .filter(|parent| {
                        let parent_namespace =
                            parent.namespace.as_deref().unwrap_or(&route.namespace);
                        parent.kind == "Gateway"
                            && parent.name == gateway.name
                            && parent_namespace == gateway.namespace
                    })
                    .collect::<Vec<_>>();
                if let Some((_, blocked)) = select_gateway_parent_attachment(
                    gateway,
                    &route.namespace,
                    &matching_parent_refs,
                ) {
                    let mut node = gateway_route_relation_node(
                        "HTTPRoute",
                        &route.name,
                        &route.namespace,
                        &route.version,
                        "httproutes",
                        route.backend_refs.as_slice(),
                        snapshot,
                    );
                    if blocked {
                        node.status = Some(format!(
                            "{}; cross-namespace attachment may be rejected",
                            node.status.unwrap_or_default()
                        ));
                    }
                    children.push(node);
                }
            }
            for route in &snapshot.grpc_routes {
                let matching_parent_refs = route
                    .parent_refs
                    .iter()
                    .filter(|parent| {
                        let parent_namespace =
                            parent.namespace.as_deref().unwrap_or(&route.namespace);
                        parent.kind == "Gateway"
                            && parent.name == gateway.name
                            && parent_namespace == gateway.namespace
                    })
                    .collect::<Vec<_>>();
                if let Some((_, blocked)) = select_gateway_parent_attachment(
                    gateway,
                    &route.namespace,
                    &matching_parent_refs,
                ) {
                    let mut node = gateway_route_relation_node(
                        "GRPCRoute",
                        &route.name,
                        &route.namespace,
                        &route.version,
                        "grpcroutes",
                        route.backend_refs.as_slice(),
                        snapshot,
                    );
                    if blocked {
                        node.status = Some(format!(
                            "{}; cross-namespace attachment may be rejected",
                            node.status.unwrap_or_default()
                        ));
                    }
                    children.push(node);
                }
            }
            vec![RelationNode {
                resource: Some(gateway_custom_resource_ref(
                    &gateway.name,
                    Some(&gateway.namespace),
                    version,
                    "Gateway",
                    "gateways",
                )),
                label: format!("Gateway {}", gateway.name),
                status: Some(format!("{} listener(s)", gateway.listeners.len())),
                namespace: Some(gateway.namespace.clone()),
                relation: RelationKind::Root,
                not_found: false,
                children,
            }]
        }
        ResourceRef::CustomResource {
            name,
            namespace,
            group,
            version,
            kind,
            ..
        } if group == "gateway.networking.k8s.io"
            && matches!(kind.as_str(), "HTTPRoute" | "GRPCRoute") =>
        {
            let Some(namespace) = namespace.as_deref() else {
                return Vec::new();
            };
            let (parent_refs, backend_refs, plural) = if kind == "HTTPRoute" {
                let Some(route) = snapshot
                    .http_routes
                    .iter()
                    .find(|route| route.name == *name && route.namespace == namespace)
                else {
                    return Vec::new();
                };
                (
                    route.parent_refs.as_slice(),
                    route.backend_refs.as_slice(),
                    "httproutes",
                )
            } else {
                let Some(route) = snapshot
                    .grpc_routes
                    .iter()
                    .find(|route| route.name == *name && route.namespace == namespace)
                else {
                    return Vec::new();
                };
                (
                    route.parent_refs.as_slice(),
                    route.backend_refs.as_slice(),
                    "grpcroutes",
                )
            };

            let mut children = parent_refs
                .iter()
                .filter(|parent| parent.kind == "Gateway")
                .map(|parent| {
                    let parent_namespace = parent.namespace.as_deref().unwrap_or(namespace);
                    let gateway = snapshot.gateways.iter().find(|gateway| {
                        gateway.name == parent.name && gateway.namespace == parent_namespace
                    });
                    RelationNode {
                        resource: gateway.map(|gateway| {
                            gateway_custom_resource_ref(
                                &gateway.name,
                                Some(&gateway.namespace),
                                &gateway.version,
                                "Gateway",
                                "gateways",
                            )
                        }),
                        label: format!("Gateway {}", parent.name),
                        status: Some(
                            match (
                                parent.section_name.as_deref(),
                                gateway.is_some_and(|gateway| {
                                    !gateway_parent_attachment_allowed(gateway, namespace, parent)
                                }),
                            ) {
                                (Some(section), true) => {
                                    format!("{section}; cross-namespace attachment may be rejected")
                                }
                                (Some(section), false) => section.to_string(),
                                (None, true) => {
                                    "cross-namespace attachment may be rejected".to_string()
                                }
                                (None, false) => String::new(),
                            },
                        )
                        .filter(|status| !status.is_empty()),
                        namespace: Some(parent_namespace.to_string()),
                        relation: RelationKind::SelectedBy,
                        not_found: gateway.is_none(),
                        children: Vec::new(),
                    }
                })
                .collect::<Vec<_>>();
            children.extend(
                backend_refs.iter().map(|backend| {
                    gateway_backend_relation_node(kind, namespace, backend, snapshot)
                }),
            );
            vec![RelationNode {
                resource: Some(gateway_custom_resource_ref(
                    name,
                    Some(namespace),
                    version,
                    kind,
                    plural,
                )),
                label: format!("{kind} {name}"),
                status: Some(format!("{} backend ref(s)", backend_refs.len())),
                namespace: Some(namespace.to_string()),
                relation: RelationKind::Root,
                not_found: false,
                children,
            }]
        }
        ResourceRef::CustomResource {
            name,
            namespace,
            group,
            version,
            kind,
            ..
        } if group == "gateway.networking.k8s.io" && kind == "ReferenceGrant" => {
            let Some(namespace) = namespace.as_deref() else {
                return Vec::new();
            };
            let Some(grant) = snapshot
                .reference_grants
                .iter()
                .find(|grant| grant.name == *name && grant.namespace == namespace)
            else {
                return Vec::new();
            };
            let mut children = grant
                .from
                .iter()
                .map(|entry| RelationNode {
                    resource: None,
                    label: format!("From {} {} in {}", entry.group, entry.kind, entry.namespace),
                    status: None,
                    namespace: Some(entry.namespace.clone()),
                    relation: RelationKind::SelectedBy,
                    not_found: false,
                    children: Vec::new(),
                })
                .collect::<Vec<_>>();
            children.extend(grant.to.iter().map(|entry| RelationNode {
                resource: None,
                label: format!(
                    "To {} {}{}",
                    entry.group,
                    entry.kind,
                    entry
                        .name
                        .as_deref()
                        .map(|name| format!(" {name}"))
                        .unwrap_or_default()
                ),
                status: None,
                namespace: Some(grant.namespace.clone()),
                relation: RelationKind::Backend,
                not_found: false,
                children: Vec::new(),
            }));
            vec![RelationNode {
                resource: Some(gateway_custom_resource_ref(
                    &grant.name,
                    Some(&grant.namespace),
                    version,
                    "ReferenceGrant",
                    "referencegrants",
                )),
                label: format!("ReferenceGrant {}", grant.name),
                status: Some(format!("{} from, {} to", grant.from.len(), grant.to.len())),
                namespace: Some(grant.namespace.clone()),
                relation: RelationKind::Root,
                not_found: false,
                children,
            }]
        }
        _ => Vec::new(),
    }
}

fn gateway_custom_resource_ref(
    name: &str,
    namespace: Option<&str>,
    version: &str,
    kind: &str,
    plural: &str,
) -> ResourceRef {
    ResourceRef::CustomResource {
        name: name.to_string(),
        namespace: namespace.map(ToOwned::to_owned),
        group: "gateway.networking.k8s.io".to_string(),
        version: version.to_string(),
        kind: kind.to_string(),
        plural: plural.to_string(),
    }
}

fn gateway_route_relation_node(
    kind: &str,
    name: &str,
    namespace: &str,
    version: &str,
    plural: &str,
    backend_refs: &[crate::k8s::dtos::GatewayBackendRefInfo],
    snapshot: &ClusterSnapshot,
) -> RelationNode {
    RelationNode {
        resource: Some(gateway_custom_resource_ref(
            name,
            Some(namespace),
            version,
            kind,
            plural,
        )),
        label: format!("{kind} {name}"),
        status: Some(format!("{} backend ref(s)", backend_refs.len())),
        namespace: Some(namespace.to_string()),
        relation: RelationKind::Owned,
        not_found: false,
        children: backend_refs
            .iter()
            .map(|backend| gateway_backend_relation_node(kind, namespace, backend, snapshot))
            .collect(),
    }
}

fn gateway_backend_relation_node(
    route_kind: &str,
    route_namespace: &str,
    backend: &crate::k8s::dtos::GatewayBackendRefInfo,
    snapshot: &ClusterSnapshot,
) -> RelationNode {
    let target_namespace = backend.namespace.as_deref().unwrap_or(route_namespace);
    let cross_namespace = target_namespace != route_namespace;
    let reference_grant_allowed = !cross_namespace
        || reference_grant_allows_backend(
            snapshot.reference_grants.as_slice(),
            route_namespace,
            route_kind,
            backend,
        );
    let blocked_cross_namespace = cross_namespace && !reference_grant_allowed;
    let service = if backend.kind == "Service" {
        (!blocked_cross_namespace)
            .then(|| {
                snapshot.services.iter().find(|service| {
                    service.name == backend.name && service.namespace == target_namespace
                })
            })
            .flatten()
    } else {
        None
    };
    let pods = service
        .map(|service| {
            if service.selector.is_empty() {
                let endpoint = snapshot.endpoints.iter().find(|endpoint| {
                    endpoint.name == service.name && endpoint.namespace == service.namespace
                });
                endpoint
                    .map(|endpoint| {
                        pods_matching_endpoint_addresses(
                            endpoint.addresses.as_slice(),
                            target_namespace,
                            snapshot,
                        )
                    })
                    .unwrap_or_default()
            } else {
                pods_matching_selector(&service.selector, target_namespace, snapshot)
            }
        })
        .unwrap_or_default();

    RelationNode {
        resource: service
            .map(|service| ResourceRef::Service(service.name.clone(), service.namespace.clone())),
        label: format!("{} {}", backend.kind, backend.name),
        status: Some(if blocked_cross_namespace {
            format!("cross-namespace from {route_kind} blocked by missing ReferenceGrant")
        } else if cross_namespace {
            format!("cross-namespace from {route_kind}")
        } else {
            format!("{} pod(s)", pods.len())
        }),
        namespace: Some(target_namespace.to_string()),
        relation: RelationKind::Backend,
        not_found: service.is_none() && !blocked_cross_namespace,
        children: pods,
    }
}

/// Resolve storage binding chains: PVC → PV → StorageClass (and inverses).
pub fn resolve_storage_bindings_from_snapshot(
    resource: &ResourceRef,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    match resource {
        ResourceRef::Pvc(name, ns) => {
            let Some(pvc) = snapshot
                .pvcs
                .iter()
                .find(|p| &p.name == name && &p.namespace == ns)
            else {
                return vec![];
            };

            // Find bound PV
            let pv_node = pvc.volume.as_ref().and_then(|vol_name| {
                let pv = snapshot.pvs.iter().find(|p| &p.name == vol_name);
                let sc_child = pv
                    .and_then(|p| p.storage_class.as_ref())
                    .and_then(|sc_name| {
                        snapshot
                            .storage_classes
                            .iter()
                            .find(|sc| &sc.name == sc_name)
                            .map(|sc| RelationNode {
                                resource: Some(ResourceRef::StorageClass(sc.name.clone())),
                                label: format!("StorageClass {}", sc.name),
                                status: None,
                                namespace: None,
                                relation: RelationKind::Bound,
                                not_found: false,
                                children: vec![],
                            })
                    });

                pv.map(|p| RelationNode {
                    resource: Some(ResourceRef::Pv(p.name.clone())),
                    label: format!("PersistentVolume {}", p.name),
                    status: Some(p.status.clone()),
                    namespace: None,
                    relation: RelationKind::Bound,
                    not_found: false,
                    children: sc_child.into_iter().collect(),
                })
            });

            let children: Vec<RelationNode> = pv_node.into_iter().collect();
            if children.is_empty() {
                return vec![];
            }

            vec![RelationNode {
                resource: Some(ResourceRef::Pvc(name.clone(), ns.clone())),
                label: format!("PersistentVolumeClaim {name}"),
                status: Some(pvc.status.clone()),
                namespace: Some(ns.clone()),
                relation: RelationKind::Root,
                not_found: false,
                children,
            }]
        }
        ResourceRef::Pv(name) => {
            let Some(pv) = snapshot.pvs.iter().find(|p| &p.name == name) else {
                return vec![];
            };

            let mut children = Vec::new();

            // Find bound PVC
            if let Some(claim) = &pv.claim {
                // claim is typically "namespace/name"
                let (pvc_ns, pvc_name) = claim.split_once('/').unwrap_or(("", claim.as_str()));
                let found = snapshot
                    .pvcs
                    .iter()
                    .find(|p| p.name == pvc_name && (pvc_ns.is_empty() || p.namespace == pvc_ns));
                let pvc_node = if let Some(p) = found {
                    RelationNode {
                        resource: Some(ResourceRef::Pvc(p.name.clone(), p.namespace.clone())),
                        label: format!("PersistentVolumeClaim {}", p.name),
                        status: Some(p.status.clone()),
                        namespace: Some(p.namespace.clone()),
                        relation: RelationKind::Bound,
                        not_found: false,
                        children: vec![],
                    }
                } else {
                    RelationNode {
                        resource: None,
                        label: format!("PersistentVolumeClaim {pvc_name}"),
                        status: None,
                        namespace: None,
                        relation: RelationKind::Bound,
                        not_found: true,
                        children: vec![],
                    }
                };
                children.push(pvc_node);
            }

            // Find StorageClass
            if let Some(sc_name) = &pv.storage_class {
                let sc_node = if let Some(sc) =
                    snapshot.storage_classes.iter().find(|s| &s.name == sc_name)
                {
                    RelationNode {
                        resource: Some(ResourceRef::StorageClass(sc.name.clone())),
                        label: format!("StorageClass {}", sc.name),
                        status: None,
                        namespace: None,
                        relation: RelationKind::Bound,
                        not_found: false,
                        children: vec![],
                    }
                } else {
                    RelationNode {
                        resource: None,
                        label: format!("StorageClass {sc_name}"),
                        status: None,
                        namespace: None,
                        relation: RelationKind::Bound,
                        not_found: true,
                        children: vec![],
                    }
                };
                children.push(sc_node);
            }

            if children.is_empty() {
                return vec![];
            }

            vec![RelationNode {
                resource: Some(ResourceRef::Pv(name.clone())),
                label: format!("PersistentVolume {name}"),
                status: Some(pv.status.clone()),
                namespace: None,
                relation: RelationKind::Root,
                not_found: false,
                children,
            }]
        }
        ResourceRef::StorageClass(name) => {
            // Find all PVs using this storage class
            let pv_nodes: Vec<RelationNode> = snapshot
                .pvs
                .iter()
                .filter(|p| p.storage_class.as_deref() == Some(name.as_str()))
                .map(|p| RelationNode {
                    resource: Some(ResourceRef::Pv(p.name.clone())),
                    label: format!("PersistentVolume {}", p.name),
                    status: Some(p.status.clone()),
                    namespace: None,
                    relation: RelationKind::Bound,
                    not_found: false,
                    children: vec![],
                })
                .collect();

            // Also find PVCs using this storage class
            let pvc_nodes: Vec<RelationNode> = snapshot
                .pvcs
                .iter()
                .filter(|p| p.storage_class.as_deref() == Some(name.as_str()))
                .map(|p| RelationNode {
                    resource: Some(ResourceRef::Pvc(p.name.clone(), p.namespace.clone())),
                    label: format!("PersistentVolumeClaim {}", p.name),
                    status: Some(p.status.clone()),
                    namespace: Some(p.namespace.clone()),
                    relation: RelationKind::Bound,
                    not_found: false,
                    children: vec![],
                })
                .collect();

            let mut children = pv_nodes;
            children.extend(pvc_nodes);

            if children.is_empty() {
                return vec![];
            }

            vec![RelationNode {
                resource: Some(ResourceRef::StorageClass(name.clone())),
                label: format!("StorageClass {name}"),
                status: None,
                namespace: None,
                relation: RelationKind::Root,
                not_found: false,
                children,
            }]
        }
        _ => vec![],
    }
}

// ---------------------------------------------------------------------------
// Task 13: RBAC bindings + Flux lineage resolvers
// ---------------------------------------------------------------------------

/// Resolve RBAC binding relationships for ServiceAccount, Role/ClusterRole, RoleBinding/ClusterRoleBinding.
pub fn resolve_rbac_bindings_from_snapshot(
    resource: &ResourceRef,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    match resource {
        ResourceRef::ServiceAccount(sa_name, sa_ns) => {
            resolve_rbac_for_service_account(sa_name, sa_ns, snapshot)
        }
        ResourceRef::Role(role_name, role_ns) => {
            resolve_rbac_for_role(role_name, role_ns, snapshot)
        }
        ResourceRef::ClusterRole(role_name) => resolve_rbac_for_cluster_role(role_name, snapshot),
        ResourceRef::RoleBinding(binding_name, binding_ns) => {
            resolve_rbac_for_role_binding(binding_name, binding_ns, snapshot)
        }
        ResourceRef::ClusterRoleBinding(binding_name) => {
            resolve_rbac_for_cluster_role_binding(binding_name, snapshot)
        }
        _ => vec![],
    }
}

fn resolve_rbac_for_service_account(
    sa_name: &str,
    sa_ns: &str,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    let mut binding_nodes = Vec::new();

    // Check RoleBindings
    for rb in &snapshot.role_bindings {
        if rb.namespace != sa_ns {
            continue;
        }
        let matches = rb.subjects.iter().any(|s| {
            s.kind == "ServiceAccount"
                && s.name == sa_name
                && s.namespace.as_deref().unwrap_or(sa_ns) == sa_ns
        });
        if !matches {
            continue;
        }

        // Find the role it references
        let role_child = make_role_ref_node(
            &rb.role_ref_kind,
            &rb.role_ref_name,
            &rb.namespace,
            snapshot,
        );

        let rb_node = RelationNode {
            resource: Some(ResourceRef::RoleBinding(
                rb.name.clone(),
                rb.namespace.clone(),
            )),
            label: format!("RoleBinding {}", rb.name),
            status: None,
            namespace: Some(rb.namespace.clone()),
            relation: RelationKind::RbacBinding,
            not_found: false,
            children: vec![role_child],
        };
        binding_nodes.push(rb_node);
    }

    // Check ClusterRoleBindings
    for crb in &snapshot.cluster_role_bindings {
        let matches = crb.subjects.iter().any(|s| {
            s.kind == "ServiceAccount" && s.name == sa_name && s.namespace.as_deref() == Some(sa_ns)
        });
        if !matches {
            continue;
        }

        let role_child = make_role_ref_node(&crb.role_ref_kind, &crb.role_ref_name, "", snapshot);

        let crb_node = RelationNode {
            resource: Some(ResourceRef::ClusterRoleBinding(crb.name.clone())),
            label: format!("ClusterRoleBinding {}", crb.name),
            status: None,
            namespace: None,
            relation: RelationKind::RbacBinding,
            not_found: false,
            children: vec![role_child],
        };
        binding_nodes.push(crb_node);
    }

    if binding_nodes.is_empty() {
        return vec![];
    }

    vec![RelationNode {
        resource: Some(ResourceRef::ServiceAccount(
            sa_name.to_string(),
            sa_ns.to_string(),
        )),
        label: format!("ServiceAccount {sa_name}"),
        status: None,
        namespace: Some(sa_ns.to_string()),
        relation: RelationKind::Root,
        not_found: false,
        children: binding_nodes,
    }]
}

fn make_role_ref_node(
    role_ref_kind: &str,
    role_ref_name: &str,
    namespace: &str,
    snapshot: &ClusterSnapshot,
) -> RelationNode {
    match role_ref_kind {
        "Role" => {
            let found = snapshot
                .roles
                .iter()
                .find(|r| r.name == role_ref_name && r.namespace == namespace);
            if let Some(r) = found {
                RelationNode {
                    resource: Some(ResourceRef::Role(r.name.clone(), r.namespace.clone())),
                    label: format!("Role {}", r.name),
                    status: None,
                    namespace: Some(r.namespace.clone()),
                    relation: RelationKind::RbacBinding,
                    not_found: false,
                    children: vec![],
                }
            } else {
                RelationNode {
                    resource: None,
                    label: format!("Role {role_ref_name}"),
                    status: None,
                    namespace: None,
                    relation: RelationKind::RbacBinding,
                    not_found: true,
                    children: vec![],
                }
            }
        }
        "ClusterRole" => {
            let found = snapshot
                .cluster_roles
                .iter()
                .find(|r| r.name == role_ref_name);
            if let Some(r) = found {
                RelationNode {
                    resource: Some(ResourceRef::ClusterRole(r.name.clone())),
                    label: format!("ClusterRole {}", r.name),
                    status: None,
                    namespace: None,
                    relation: RelationKind::RbacBinding,
                    not_found: false,
                    children: vec![],
                }
            } else {
                RelationNode {
                    resource: None,
                    label: format!("ClusterRole {role_ref_name}"),
                    status: None,
                    namespace: None,
                    relation: RelationKind::RbacBinding,
                    not_found: true,
                    children: vec![],
                }
            }
        }
        _ => RelationNode {
            resource: None,
            label: format!("{role_ref_kind} {role_ref_name}"),
            status: None,
            namespace: None,
            relation: RelationKind::RbacBinding,
            not_found: true,
            children: vec![],
        },
    }
}

fn resolve_rbac_for_role(
    role_name: &str,
    role_ns: &str,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    let binding_nodes: Vec<RelationNode> = snapshot
        .role_bindings
        .iter()
        .filter(|rb| {
            rb.namespace == role_ns && rb.role_ref_kind == "Role" && rb.role_ref_name == role_name
        })
        .map(|rb| {
            let subject_children: Vec<RelationNode> = rb
                .subjects
                .iter()
                .map(|s| RelationNode {
                    resource: if s.kind == "ServiceAccount" {
                        Some(ResourceRef::ServiceAccount(
                            s.name.clone(),
                            s.namespace.clone().unwrap_or_else(|| rb.namespace.clone()),
                        ))
                    } else {
                        None
                    },
                    label: format!("{} {}", s.kind, s.name),
                    status: None,
                    namespace: s.namespace.clone(),
                    relation: RelationKind::RbacBinding,
                    not_found: false,
                    children: vec![],
                })
                .collect();
            RelationNode {
                resource: Some(ResourceRef::RoleBinding(
                    rb.name.clone(),
                    rb.namespace.clone(),
                )),
                label: format!("RoleBinding {}", rb.name),
                status: None,
                namespace: Some(rb.namespace.clone()),
                relation: RelationKind::RbacBinding,
                not_found: false,
                children: subject_children,
            }
        })
        .collect();

    if binding_nodes.is_empty() {
        return vec![];
    }

    vec![RelationNode {
        resource: Some(ResourceRef::Role(
            role_name.to_string(),
            role_ns.to_string(),
        )),
        label: format!("Role {role_name}"),
        status: None,
        namespace: Some(role_ns.to_string()),
        relation: RelationKind::Root,
        not_found: false,
        children: binding_nodes,
    }]
}

fn resolve_rbac_for_cluster_role(role_name: &str, snapshot: &ClusterSnapshot) -> Vec<RelationNode> {
    let mut binding_nodes: Vec<RelationNode> = Vec::new();

    // RoleBindings that reference this ClusterRole
    for rb in &snapshot.role_bindings {
        if rb.role_ref_kind == "ClusterRole" && rb.role_ref_name == role_name {
            let subject_children: Vec<RelationNode> = rb
                .subjects
                .iter()
                .map(|s| RelationNode {
                    resource: if s.kind == "ServiceAccount" {
                        Some(ResourceRef::ServiceAccount(
                            s.name.clone(),
                            s.namespace.clone().unwrap_or_else(|| rb.namespace.clone()),
                        ))
                    } else {
                        None
                    },
                    label: format!("{} {}", s.kind, s.name),
                    status: None,
                    namespace: s.namespace.clone(),
                    relation: RelationKind::RbacBinding,
                    not_found: false,
                    children: vec![],
                })
                .collect();
            binding_nodes.push(RelationNode {
                resource: Some(ResourceRef::RoleBinding(
                    rb.name.clone(),
                    rb.namespace.clone(),
                )),
                label: format!("RoleBinding {}", rb.name),
                status: None,
                namespace: Some(rb.namespace.clone()),
                relation: RelationKind::RbacBinding,
                not_found: false,
                children: subject_children,
            });
        }
    }

    // ClusterRoleBindings that reference this ClusterRole
    for crb in &snapshot.cluster_role_bindings {
        if crb.role_ref_kind == "ClusterRole" && crb.role_ref_name == role_name {
            let subject_children: Vec<RelationNode> = crb
                .subjects
                .iter()
                .map(|s| RelationNode {
                    resource: if s.kind == "ServiceAccount" {
                        s.namespace
                            .as_ref()
                            .map(|ns| ResourceRef::ServiceAccount(s.name.clone(), ns.clone()))
                    } else {
                        None
                    },
                    label: format!("{} {}", s.kind, s.name),
                    status: None,
                    namespace: s.namespace.clone(),
                    relation: RelationKind::RbacBinding,
                    not_found: false,
                    children: vec![],
                })
                .collect();
            binding_nodes.push(RelationNode {
                resource: Some(ResourceRef::ClusterRoleBinding(crb.name.clone())),
                label: format!("ClusterRoleBinding {}", crb.name),
                status: None,
                namespace: None,
                relation: RelationKind::RbacBinding,
                not_found: false,
                children: subject_children,
            });
        }
    }

    if binding_nodes.is_empty() {
        return vec![];
    }

    vec![RelationNode {
        resource: Some(ResourceRef::ClusterRole(role_name.to_string())),
        label: format!("ClusterRole {role_name}"),
        status: None,
        namespace: None,
        relation: RelationKind::Root,
        not_found: false,
        children: binding_nodes,
    }]
}

fn resolve_rbac_for_role_binding(
    binding_name: &str,
    binding_ns: &str,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    let Some(rb) = snapshot
        .role_bindings
        .iter()
        .find(|r| r.name == binding_name && r.namespace == binding_ns)
    else {
        return vec![];
    };

    let role_child = make_role_ref_node(&rb.role_ref_kind, &rb.role_ref_name, binding_ns, snapshot);

    let subject_children: Vec<RelationNode> = rb
        .subjects
        .iter()
        .map(|s| RelationNode {
            resource: if s.kind == "ServiceAccount" {
                Some(ResourceRef::ServiceAccount(
                    s.name.clone(),
                    s.namespace
                        .clone()
                        .unwrap_or_else(|| binding_ns.to_string()),
                ))
            } else {
                None
            },
            label: format!("{} {}", s.kind, s.name),
            status: None,
            namespace: s.namespace.clone(),
            relation: RelationKind::RbacBinding,
            not_found: false,
            children: vec![],
        })
        .collect();

    vec![RelationNode {
        resource: Some(ResourceRef::RoleBinding(
            binding_name.to_string(),
            binding_ns.to_string(),
        )),
        label: format!("RoleBinding {binding_name}"),
        status: None,
        namespace: Some(binding_ns.to_string()),
        relation: RelationKind::Root,
        not_found: false,
        children: std::iter::once(role_child)
            .chain(subject_children)
            .collect(),
    }]
}

fn resolve_rbac_for_cluster_role_binding(
    binding_name: &str,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    let Some(crb) = snapshot
        .cluster_role_bindings
        .iter()
        .find(|r| r.name == binding_name)
    else {
        return vec![];
    };

    let role_child = make_role_ref_node(&crb.role_ref_kind, &crb.role_ref_name, "", snapshot);

    let subject_children: Vec<RelationNode> = crb
        .subjects
        .iter()
        .map(|s| RelationNode {
            resource: if s.kind == "ServiceAccount" {
                s.namespace
                    .as_ref()
                    .map(|ns| ResourceRef::ServiceAccount(s.name.clone(), ns.clone()))
            } else {
                None
            },
            label: format!("{} {}", s.kind, s.name),
            status: None,
            namespace: s.namespace.clone(),
            relation: RelationKind::RbacBinding,
            not_found: false,
            children: vec![],
        })
        .collect();

    vec![RelationNode {
        resource: Some(ResourceRef::ClusterRoleBinding(binding_name.to_string())),
        label: format!("ClusterRoleBinding {binding_name}"),
        status: None,
        namespace: None,
        relation: RelationKind::Root,
        not_found: false,
        children: std::iter::once(role_child)
            .chain(subject_children)
            .collect(),
    }]
}

/// Resolve Flux lineage: find Flux resources that are owners/owned or share
/// the same source reference within the namespace. Only shows genuinely
/// related resources rather than every Flux resource in the namespace.
pub fn resolve_flux_lineage_from_snapshot(
    resource: &ResourceRef,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    let (name, namespace, kind, group) = match resource {
        ResourceRef::CustomResource {
            name,
            namespace,
            kind,
            group,
            ..
        } => (
            name.as_str(),
            namespace.as_deref(),
            kind.as_str(),
            group.as_str(),
        ),
        _ => return vec![],
    };

    let resource_ns = namespace.unwrap_or("");

    // Find this resource's source_url for matching.
    let self_info = snapshot
        .flux_resources
        .iter()
        .find(|f| f.name == name && f.namespace.as_deref().unwrap_or("") == resource_ns);

    let self_source_url = self_info.and_then(|f| f.source_url.as_deref());

    // Collect related Flux resources:
    // 1. Resources sharing the same source_url (implies same source dependency)
    // 2. Same namespace, filtering out unrelated resources
    let related: Vec<RelationNode> = snapshot
        .flux_resources
        .iter()
        .filter(|f| {
            let f_ns = f.namespace.as_deref().unwrap_or("");
            // Must be same namespace (strict match)
            if f_ns != resource_ns {
                return false;
            }
            // Not the resource itself
            if f.name == name && f.kind == kind && f.group == group && f_ns == resource_ns {
                return false;
            }
            // Match by shared source_url
            if let (Some(self_url), Some(f_url)) = (self_source_url, f.source_url.as_deref())
                && self_url == f_url
            {
                return true;
            }
            // Match by name prefix with separator (e.g. "my-app" matches "my-app-source")
            if f.name.starts_with(&format!("{name}-")) || name.starts_with(&format!("{}-", f.name))
            {
                return true;
            }
            false
        })
        .map(|f| RelationNode {
            resource: Some(ResourceRef::CustomResource {
                group: f.group.clone(),
                kind: f.kind.clone(),
                plural: f.plural.clone(),
                version: f.version.clone(),
                name: f.name.clone(),
                namespace: f.namespace.clone(),
            }),
            label: format!("{} {}", f.kind, f.name),
            status: Some(f.status.clone()),
            namespace: f.namespace.clone(),
            relation: RelationKind::FluxSource,
            not_found: false,
            children: vec![],
        })
        .collect();

    if related.is_empty() {
        return vec![];
    }

    vec![RelationNode {
        resource: Some(resource.clone()),
        label: format!(
            "{} {name}",
            match resource {
                ResourceRef::CustomResource { kind, .. } => kind.as_str(),
                _ => "Resource",
            }
        ),
        status: None,
        namespace: namespace.map(|s| s.to_string()),
        relation: RelationKind::Root,
        not_found: false,
        children: related,
    }]
}

// ---------------------------------------------------------------------------
// Task 14: Async orchestrator
// ---------------------------------------------------------------------------

/// Resolve all relationship sections for a resource using snapshot data.
pub async fn resolve_relationships(
    resource: &ResourceRef,
    snapshot: &ClusterSnapshot,
    _client: &crate::k8s::client::K8sClient,
) -> anyhow::Result<Vec<RelationNode>> {
    let Some(view) = resource_to_view(resource) else {
        return Ok(Vec::new());
    };
    let capabilities = view.relationship_capabilities();
    let mut sections = Vec::new();
    for cap in capabilities {
        let nodes = match cap {
            RelationshipCapability::OwnerChain => {
                resolve_owner_chain_from_snapshot(resource, snapshot)
            }
            RelationshipCapability::ServiceBackends => {
                resolve_service_backends_from_snapshot(resource, snapshot)
            }
            RelationshipCapability::IngressBackends => {
                resolve_ingress_backends_from_snapshot(resource, snapshot)
            }
            RelationshipCapability::GatewayRoutes => {
                resolve_gateway_topology_from_snapshot(resource, snapshot)
            }
            RelationshipCapability::StorageBindings => {
                resolve_storage_bindings_from_snapshot(resource, snapshot)
            }
            RelationshipCapability::FluxLineage => {
                resolve_flux_lineage_from_snapshot(resource, snapshot)
            }
            RelationshipCapability::RbacBindings => {
                resolve_rbac_bindings_from_snapshot(resource, snapshot)
            }
        };
        if !nodes.is_empty() {
            sections.push(RelationNode {
                resource: None,
                label: cap.section_title().to_string(),
                status: None,
                namespace: None,
                relation: RelationKind::SectionHeader,
                not_found: false,
                children: nodes,
            });
        }
    }
    Ok(sections)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(label: &str) -> RelationNode {
        RelationNode {
            resource: None,
            label: label.to_string(),
            status: None,
            namespace: None,
            relation: RelationKind::Owned,
            not_found: false,
            children: vec![],
        }
    }

    fn parent(label: &str, children: Vec<RelationNode>) -> RelationNode {
        RelationNode {
            resource: None,
            label: label.to_string(),
            status: None,
            namespace: None,
            relation: RelationKind::Owner,
            not_found: false,
            children,
        }
    }

    #[test]
    fn flatten_empty_tree() {
        let result = flatten_tree(&[], &HashSet::new());
        assert!(result.is_empty());
    }

    #[test]
    fn flatten_single_node() {
        let nodes = vec![leaf("root")];
        let result = flatten_tree(&nodes, &HashSet::new());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].depth, 0);
        assert_eq!(result[0].tree_index, 0);
        assert_eq!(result[0].label, "root");
        assert!(!result[0].has_children);
    }

    #[test]
    fn flatten_expanded_parent_shows_children() {
        let nodes = vec![parent("p", vec![leaf("c1"), leaf("c2")]), leaf("sibling")];
        let mut expanded = HashSet::new();
        expanded.insert(0); // expand "p"

        let result = flatten_tree(&nodes, &expanded);
        // p, c1, c2, sibling
        assert_eq!(result.len(), 4);

        assert_eq!(result[0].label, "p");
        assert_eq!(result[0].depth, 0);
        assert_eq!(result[0].tree_index, 0);
        assert!(result[0].has_children);
        assert!(result[0].expanded);

        assert_eq!(result[1].label, "c1");
        assert_eq!(result[1].depth, 1);
        assert_eq!(result[1].tree_index, 1);
        assert!(!result[1].is_last_child);

        assert_eq!(result[2].label, "c2");
        assert_eq!(result[2].depth, 1);
        assert_eq!(result[2].tree_index, 2);
        assert!(result[2].is_last_child);

        assert_eq!(result[3].label, "sibling");
        assert_eq!(result[3].depth, 0);
        assert_eq!(result[3].tree_index, 3);
    }

    #[test]
    fn flatten_collapsed_parent_hides_children() {
        let nodes = vec![parent("p", vec![leaf("c1"), leaf("c2")])];
        let result = flatten_tree(&nodes, &HashSet::new());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "p");
        assert!(result[0].has_children);
        assert!(!result[0].expanded);
    }

    #[test]
    fn stable_indices_across_collapse() {
        // Two parents each with one child.
        let nodes = vec![
            parent("p1", vec![leaf("c1")]),
            parent("p2", vec![leaf("c2")]),
        ];

        // Expand only p2 (index 2).
        let mut expanded = HashSet::new();
        expanded.insert(2);

        let result = flatten_tree(&nodes, &expanded);
        // p1 (collapsed, idx=0), p2 (idx=2, expanded), c2 (idx=3)
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].tree_index, 0); // p1
        assert_eq!(result[1].tree_index, 2); // p2
        assert_eq!(result[2].tree_index, 3); // c2
        assert!(result[1].expanded);
    }

    #[test]
    fn resource_to_view_maps_core_types() {
        assert_eq!(
            resource_to_view(&ResourceRef::Pod("p".into(), "ns".into())),
            Some(AppView::Pods)
        );
        assert_eq!(
            resource_to_view(&ResourceRef::Service("svc".into(), "ns".into())),
            Some(AppView::Services)
        );
        assert_eq!(resource_to_view(&ResourceRef::Node("n".into())), None);
    }

    #[test]
    fn resource_has_relationships_for_supported_types() {
        assert!(resource_has_relationships(&ResourceRef::Deployment(
            "d".into(),
            "ns".into()
        )));
        assert!(resource_has_relationships(&ResourceRef::Service(
            "s".into(),
            "ns".into()
        )));
        assert!(resource_has_relationships(&ResourceRef::Pvc(
            "claim".into(),
            "ns".into()
        )));
        assert!(!resource_has_relationships(&ResourceRef::Node("n".into())));
        assert!(!resource_has_relationships(&ResourceRef::ConfigMap(
            "cm".into(),
            "ns".into()
        )));
    }

    #[test]
    fn parent_is_last_tracks_ancestors() {
        // grandparent → parent → child
        let nodes = vec![parent("gp", vec![parent("p", vec![leaf("c")])])];
        let mut expanded = HashSet::new();
        expanded.insert(0); // expand gp
        expanded.insert(1); // expand p

        let result = flatten_tree(&nodes, &expanded);
        // gp (idx=0), p (idx=1), c (idx=2)
        assert_eq!(result.len(), 3);

        // gp: depth=0, parent_is_last empty
        assert_eq!(result[0].depth, 0);
        assert!(result[0].parent_is_last.is_empty());

        // p: depth=1, parent_is_last=[true] (gp is last at its level)
        assert_eq!(result[1].depth, 1);
        assert_eq!(result[1].parent_is_last, vec![true]);

        // c: depth=2, parent_is_last=[true, true]
        assert_eq!(result[2].depth, 2);
        assert_eq!(result[2].parent_is_last, vec![true, true]);
    }

    // ---------------------------------------------------------------------------
    // Task 10 tests: Owner chain
    // ---------------------------------------------------------------------------

    #[test]
    fn resolve_owner_chain_pod_to_replicaset_to_deployment() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.pods = vec![PodInfo {
            name: "pod-0".into(),
            namespace: "default".into(),
            status: "Running".into(),
            owner_references: vec![OwnerRefInfo {
                kind: "ReplicaSet".into(),
                name: "rs-abc".into(),
                uid: "uid-rs".into(),
            }],
            ..Default::default()
        }];
        snapshot.replicasets = vec![ReplicaSetInfo {
            name: "rs-abc".into(),
            namespace: "default".into(),
            desired: 3,
            ready: 3,
            owner_references: vec![OwnerRefInfo {
                kind: "Deployment".into(),
                name: "deploy-1".into(),
                uid: "uid-deploy".into(),
            }],
            ..Default::default()
        }];
        snapshot.deployments = vec![DeploymentInfo {
            name: "deploy-1".into(),
            namespace: "default".into(),
            ready: "3/3".into(),
            ..Default::default()
        }];

        let resource = ResourceRef::Pod("pod-0".into(), "default".into());
        let result = resolve_owner_chain_from_snapshot(&resource, &snapshot);

        assert!(!result.is_empty());
        assert_eq!(result[0].label, "Deployment deploy-1");
        assert_eq!(result[0].children.len(), 1);
        assert_eq!(result[0].children[0].label, "ReplicaSet rs-abc");
    }

    #[test]
    fn resolve_owner_chain_includes_owned_children_of_target() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        // ReplicaSet owned by Deployment, and the ReplicaSet owns a Pod.
        // The target (ReplicaSet) should show its owned Pod even when it has an owner.
        let mut snapshot = ClusterSnapshot::default();
        snapshot.replicasets = vec![ReplicaSetInfo {
            name: "rs-abc".into(),
            namespace: "default".into(),
            desired: 3,
            ready: 3,
            owner_references: vec![OwnerRefInfo {
                kind: "Deployment".into(),
                name: "deploy-1".into(),
                uid: "uid-deploy".into(),
            }],
            ..Default::default()
        }];
        snapshot.deployments = vec![DeploymentInfo {
            name: "deploy-1".into(),
            namespace: "default".into(),
            ready: "3/3".into(),
            ..Default::default()
        }];
        snapshot.pods = vec![PodInfo {
            name: "pod-0".into(),
            namespace: "default".into(),
            status: "Running".into(),
            owner_references: vec![OwnerRefInfo {
                kind: "ReplicaSet".into(),
                name: "rs-abc".into(),
                uid: "uid-rs".into(),
            }],
            ..Default::default()
        }];

        let resource = ResourceRef::ReplicaSet("rs-abc".into(), "default".into());
        let result = resolve_owner_chain_from_snapshot(&resource, &snapshot);

        // Tree: Deployment → ReplicaSet (target) → Pod (owned)
        assert_eq!(result[0].label, "Deployment deploy-1");
        let rs_node = &result[0].children[0];
        assert_eq!(rs_node.label, "ReplicaSet rs-abc");
        assert_eq!(
            rs_node.children.len(),
            1,
            "target should include owned children"
        );
        assert_eq!(rs_node.children[0].label, "Pod pod-0");
    }

    #[test]
    fn resolve_owner_chain_missing_owner_shows_not_found() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.pods = vec![PodInfo {
            name: "orphan-pod".into(),
            namespace: "default".into(),
            owner_references: vec![OwnerRefInfo {
                kind: "ReplicaSet".into(),
                name: "deleted-rs".into(),
                uid: "uid-gone".into(),
            }],
            ..Default::default()
        }];

        let resource = ResourceRef::Pod("orphan-pod".into(), "default".into());
        let result = resolve_owner_chain_from_snapshot(&resource, &snapshot);

        assert!(!result.is_empty());
        assert!(result[0].not_found);
    }

    #[test]
    fn resolve_owner_chain_handles_cycle_without_infinite_loop() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        // Create a pathological cycle: rs-a owns rs-b owns rs-a
        let mut snapshot = ClusterSnapshot::default();
        snapshot.replicasets = vec![
            ReplicaSetInfo {
                name: "rs-a".into(),
                namespace: "default".into(),
                desired: 1,
                ready: 1,
                owner_references: vec![OwnerRefInfo {
                    kind: "ReplicaSet".into(),
                    name: "rs-b".into(),
                    uid: "uid-b".into(),
                }],
                ..Default::default()
            },
            ReplicaSetInfo {
                name: "rs-b".into(),
                namespace: "default".into(),
                desired: 1,
                ready: 1,
                owner_references: vec![OwnerRefInfo {
                    kind: "ReplicaSet".into(),
                    name: "rs-a".into(),
                    uid: "uid-a".into(),
                }],
                ..Default::default()
            },
        ];

        let resource = ResourceRef::ReplicaSet("rs-a".into(), "default".into());
        // This must terminate (not hang forever)
        let result = resolve_owner_chain_from_snapshot(&resource, &snapshot);
        // Should produce a tree with at most the two resources (no infinite chain)
        assert!(!result.is_empty());
    }

    // ---------------------------------------------------------------------------
    // Task 11 tests: Service backends
    // ---------------------------------------------------------------------------

    #[test]
    fn resolve_service_backends_matches_pods_by_selector() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.services = vec![ServiceInfo {
            name: "nginx-svc".into(),
            namespace: "default".into(),
            type_: "ClusterIP".into(),
            selector: [("app".to_string(), "nginx".to_string())].into(),
            ..Default::default()
        }];
        snapshot.endpoints = vec![EndpointInfo {
            name: "nginx-svc".into(),
            namespace: "default".into(),
            addresses: vec!["10.0.0.10".into()],
            ..Default::default()
        }];
        snapshot.pods = vec![
            PodInfo {
                name: "nginx-pod-1".into(),
                namespace: "default".into(),
                status: "Running".into(),
                pod_ip: Some("10.0.0.10".into()),
                labels: vec![("app".into(), "nginx".into())],
                ..Default::default()
            },
            PodInfo {
                name: "other-pod".into(),
                namespace: "default".into(),
                status: "Running".into(),
                labels: vec![("app".into(), "other".into())],
                ..Default::default()
            },
        ];

        let resource = ResourceRef::Service("nginx-svc".into(), "default".into());
        let result = resolve_service_backends_from_snapshot(&resource, &snapshot);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "Service nginx-svc");
        assert_eq!(result[0].children.len(), 1);
        assert_eq!(result[0].children[0].label, "Endpoints nginx-svc");
        assert_eq!(result[0].children[0].children.len(), 1);
        assert_eq!(result[0].children[0].children[0].label, "Pod nginx-pod-1");
    }

    #[test]
    fn resolve_service_backends_empty_selector_returns_nothing() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.services = vec![ServiceInfo {
            name: "headless".into(),
            namespace: "default".into(),
            type_: "ClusterIP".into(),
            selector: std::collections::BTreeMap::new(),
            ..Default::default()
        }];

        let resource = ResourceRef::Service("headless".into(), "default".into());
        let result = resolve_service_backends_from_snapshot(&resource, &snapshot);
        assert!(result.is_empty());
    }

    #[test]
    fn resolve_service_backends_selectorless_service_keeps_manual_endpoints_visible() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.services = vec![ServiceInfo {
            name: "manual".into(),
            namespace: "default".into(),
            type_: "ClusterIP".into(),
            selector: std::collections::BTreeMap::new(),
            ..Default::default()
        }];
        snapshot.endpoints = vec![EndpointInfo {
            name: "manual".into(),
            namespace: "default".into(),
            addresses: vec!["10.0.0.20".into()],
            ..Default::default()
        }];

        let resource = ResourceRef::Service("manual".into(), "default".into());
        let result = resolve_service_backends_from_snapshot(&resource, &snapshot);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].children.len(), 1);
        assert_eq!(result[0].children[0].label, "Endpoints manual");
    }

    #[test]
    fn resolve_endpoint_resource_returns_endpoint_root() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.endpoints = vec![EndpointInfo {
            name: "api".into(),
            namespace: "default".into(),
            addresses: vec!["10.0.0.30".into()],
            ..Default::default()
        }];
        snapshot.pods = vec![PodInfo {
            name: "api-pod".into(),
            namespace: "default".into(),
            status: "Running".into(),
            pod_ip: Some("10.0.0.30".into()),
            ..Default::default()
        }];

        let resource = ResourceRef::Endpoint("api".into(), "default".into());
        let result = resolve_service_backends_from_snapshot(&resource, &snapshot);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "Endpoints api");
        assert_eq!(result[0].children.len(), 1);
        assert_eq!(result[0].children[0].label, "Pod api-pod");
    }

    // ---------------------------------------------------------------------------
    // Task 12 tests: Ingress backends + storage bindings
    // ---------------------------------------------------------------------------

    #[test]
    fn resolve_ingress_backends_matches_services() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.ingresses = vec![IngressInfo {
            name: "my-ingress".into(),
            namespace: "default".into(),
            backend_services: vec![("web-svc".to_string(), "80".to_string())],
            ..Default::default()
        }];
        snapshot.services = vec![ServiceInfo {
            name: "web-svc".into(),
            namespace: "default".into(),
            selector: [("app".to_string(), "web".to_string())].into(),
            ..Default::default()
        }];

        let resource = ResourceRef::Ingress("my-ingress".into(), "default".into());
        let result = resolve_ingress_backends_from_snapshot(&resource, &snapshot);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "Ingress my-ingress");
        assert_eq!(result[0].children.len(), 1);
        assert_eq!(result[0].children[0].label, "Service web-svc");
        assert!(!result[0].children[0].not_found);
    }

    #[test]
    fn resolve_ingress_backends_missing_service_not_found() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.ingresses = vec![IngressInfo {
            name: "my-ingress".into(),
            namespace: "default".into(),
            backend_services: vec![("missing-svc".to_string(), "80".to_string())],
            ..Default::default()
        }];

        let resource = ResourceRef::Ingress("my-ingress".into(), "default".into());
        let result = resolve_ingress_backends_from_snapshot(&resource, &snapshot);

        assert_eq!(result.len(), 1);
        assert!(result[0].children[0].not_found);
    }

    #[test]
    fn resolve_ingress_class_finds_ingresses() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.ingresses = vec![
            IngressInfo {
                name: "ing-1".into(),
                namespace: "default".into(),
                class: Some("nginx".into()),
                ..Default::default()
            },
            IngressInfo {
                name: "ing-2".into(),
                namespace: "prod".into(),
                class: Some("traefik".into()),
                ..Default::default()
            },
        ];

        let resource = ResourceRef::IngressClass("nginx".into());
        let result = resolve_ingress_backends_from_snapshot(&resource, &snapshot);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "IngressClass nginx");
        assert_eq!(result[0].children.len(), 1);
        assert_eq!(result[0].children[0].label, "Ingress ing-1");
    }

    #[test]
    fn resolve_gateway_route_blocks_cross_namespace_backend_without_reference_grant() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.http_routes = vec![HttpRouteInfo {
            name: "frontend".into(),
            namespace: "apps".into(),
            version: "v1beta1".into(),
            backend_refs: vec![GatewayBackendRefInfo {
                group: "".into(),
                kind: "Service".into(),
                namespace: Some("backend".into()),
                name: "api".into(),
                port: Some(80),
            }],
            ..Default::default()
        }];
        snapshot.services = vec![ServiceInfo {
            name: "api".into(),
            namespace: "backend".into(),
            selector: [("app".to_string(), "api".to_string())].into(),
            ..Default::default()
        }];

        let resource = ResourceRef::CustomResource {
            group: "gateway.networking.k8s.io".into(),
            kind: "HTTPRoute".into(),
            plural: "httproutes".into(),
            version: "v1beta1".into(),
            name: "frontend".into(),
            namespace: Some("apps".into()),
        };
        let result = resolve_gateway_topology_from_snapshot(&resource, &snapshot);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].children.len(), 1);
        assert_eq!(
            result[0].children[0].status.as_deref(),
            Some("cross-namespace from HTTPRoute blocked by missing ReferenceGrant")
        );
        assert!(result[0].children[0].resource.is_none());
        assert!(!result[0].children[0].not_found);
    }

    #[test]
    fn resolve_gateway_route_preserves_route_version() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.http_routes = vec![HttpRouteInfo {
            name: "frontend".into(),
            namespace: "apps".into(),
            version: "v1beta1".into(),
            ..Default::default()
        }];

        let resource = ResourceRef::CustomResource {
            group: "gateway.networking.k8s.io".into(),
            kind: "HTTPRoute".into(),
            plural: "httproutes".into(),
            version: "v1beta1".into(),
            name: "frontend".into(),
            namespace: Some("apps".into()),
        };
        let result = resolve_gateway_topology_from_snapshot(&resource, &snapshot);

        assert_eq!(result.len(), 1);
        match result[0].resource.as_ref() {
            Some(ResourceRef::CustomResource { version, .. }) => assert_eq!(version, "v1beta1"),
            other => panic!("unexpected resource: {other:?}"),
        }
    }

    #[test]
    fn resolve_gateway_root_deduplicates_routes_with_multiple_parent_refs_to_same_gateway() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.gateways = vec![GatewayInfo {
            name: "edge".into(),
            namespace: "shared".into(),
            version: "v1".into(),
            listeners: vec![
                GatewayListenerInfo {
                    name: "http".into(),
                    protocol: "HTTP".into(),
                    port: 80,
                    ..Default::default()
                },
                GatewayListenerInfo {
                    name: "public".into(),
                    protocol: "HTTP".into(),
                    port: 8080,
                    allowed_routes_from: Some("All".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }];
        snapshot.http_routes = vec![HttpRouteInfo {
            name: "frontend".into(),
            namespace: "apps".into(),
            version: "v1".into(),
            parent_refs: vec![
                GatewayParentRefInfo {
                    group: "gateway.networking.k8s.io".into(),
                    kind: "Gateway".into(),
                    name: "edge".into(),
                    namespace: Some("shared".into()),
                    section_name: Some("http".into()),
                },
                GatewayParentRefInfo {
                    group: "gateway.networking.k8s.io".into(),
                    kind: "Gateway".into(),
                    name: "edge".into(),
                    namespace: Some("shared".into()),
                    section_name: Some("public".into()),
                },
            ],
            ..Default::default()
        }];

        let resource = ResourceRef::CustomResource {
            group: "gateway.networking.k8s.io".into(),
            kind: "Gateway".into(),
            plural: "gateways".into(),
            version: "v1".into(),
            name: "edge".into(),
            namespace: Some("shared".into()),
        };
        let result = resolve_gateway_topology_from_snapshot(&resource, &snapshot);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].children.len(), 1);
        assert_eq!(result[0].children[0].label, "HTTPRoute frontend");
    }

    #[test]
    fn resolve_gateway_root_marks_cross_namespace_attachment_warning() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.gateways = vec![GatewayInfo {
            name: "edge".into(),
            namespace: "shared".into(),
            version: "v1".into(),
            listeners: vec![GatewayListenerInfo {
                name: "http".into(),
                protocol: "HTTP".into(),
                port: 80,
                allowed_routes_from: Some("Selector".into()),
                ..Default::default()
            }],
            ..Default::default()
        }];
        snapshot.http_routes = vec![HttpRouteInfo {
            name: "frontend".into(),
            namespace: "apps".into(),
            version: "v1".into(),
            parent_refs: vec![GatewayParentRefInfo {
                group: "gateway.networking.k8s.io".into(),
                kind: "Gateway".into(),
                name: "edge".into(),
                namespace: Some("shared".into()),
                section_name: Some("http".into()),
            }],
            ..Default::default()
        }];

        let resource = ResourceRef::CustomResource {
            group: "gateway.networking.k8s.io".into(),
            kind: "Gateway".into(),
            plural: "gateways".into(),
            version: "v1".into(),
            name: "edge".into(),
            namespace: Some("shared".into()),
        };
        let result = resolve_gateway_topology_from_snapshot(&resource, &snapshot);

        assert_eq!(result.len(), 1);
        assert!(
            result[0].children[0]
                .status
                .as_deref()
                .is_some_and(|status| status.contains("attachment may be rejected"))
        );
    }

    #[test]
    fn resolve_gateway_root_prefers_allowed_parent_ref_status_when_same_gateway_is_referenced_twice()
     {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.gateways = vec![GatewayInfo {
            name: "edge".into(),
            namespace: "shared".into(),
            version: "v1".into(),
            listeners: vec![
                GatewayListenerInfo {
                    name: "private".into(),
                    protocol: "HTTP".into(),
                    port: 80,
                    allowed_routes_from: Some("Same".into()),
                    ..Default::default()
                },
                GatewayListenerInfo {
                    name: "public".into(),
                    protocol: "HTTP".into(),
                    port: 8080,
                    allowed_routes_from: Some("All".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }];
        snapshot.http_routes = vec![HttpRouteInfo {
            name: "frontend".into(),
            namespace: "apps".into(),
            version: "v1".into(),
            parent_refs: vec![
                GatewayParentRefInfo {
                    group: "gateway.networking.k8s.io".into(),
                    kind: "Gateway".into(),
                    name: "edge".into(),
                    namespace: Some("shared".into()),
                    section_name: Some("private".into()),
                },
                GatewayParentRefInfo {
                    group: "gateway.networking.k8s.io".into(),
                    kind: "Gateway".into(),
                    name: "edge".into(),
                    namespace: Some("shared".into()),
                    section_name: Some("public".into()),
                },
            ],
            ..Default::default()
        }];

        let resource = ResourceRef::CustomResource {
            group: "gateway.networking.k8s.io".into(),
            kind: "Gateway".into(),
            plural: "gateways".into(),
            version: "v1".into(),
            name: "edge".into(),
            namespace: Some("shared".into()),
        };
        let result = resolve_gateway_topology_from_snapshot(&resource, &snapshot);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].children.len(), 1);
        assert!(
            result[0].children[0]
                .status
                .as_deref()
                .is_none_or(|status| !status.contains("attachment may be rejected"))
        );
    }

    #[test]
    fn resolve_pvc_to_pv_to_storage_class() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.pvcs = vec![PvcInfo {
            name: "data-pvc".into(),
            namespace: "default".into(),
            status: "Bound".into(),
            volume: Some("pv-001".into()),
            storage_class: Some("fast".into()),
            ..Default::default()
        }];
        snapshot.pvs = vec![PvInfo {
            name: "pv-001".into(),
            status: "Bound".into(),
            storage_class: Some("fast".into()),
            claim: Some("default/data-pvc".into()),
            ..Default::default()
        }];
        snapshot.storage_classes = vec![StorageClassInfo {
            name: "fast".into(),
            provisioner: "disk.csi.k8s.io".into(),
            ..Default::default()
        }];

        let resource = ResourceRef::Pvc("data-pvc".into(), "default".into());
        let result = resolve_storage_bindings_from_snapshot(&resource, &snapshot);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "PersistentVolumeClaim data-pvc");
        assert_eq!(result[0].children.len(), 1);
        assert_eq!(result[0].children[0].label, "PersistentVolume pv-001");
        assert_eq!(result[0].children[0].children.len(), 1);
        assert_eq!(result[0].children[0].children[0].label, "StorageClass fast");
    }

    // ---------------------------------------------------------------------------
    // Task 13 tests: RBAC and Flux
    // ---------------------------------------------------------------------------

    #[test]
    fn resolve_rbac_service_account_to_binding_to_role() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.service_accounts = vec![ServiceAccountInfo {
            name: "my-sa".into(),
            namespace: "default".into(),
            ..Default::default()
        }];
        snapshot.roles = vec![RoleInfo {
            name: "my-role".into(),
            namespace: "default".into(),
            ..Default::default()
        }];
        snapshot.role_bindings = vec![RoleBindingInfo {
            name: "my-rb".into(),
            namespace: "default".into(),
            role_ref_kind: "Role".into(),
            role_ref_name: "my-role".into(),
            subjects: vec![RoleBindingSubject {
                kind: "ServiceAccount".into(),
                name: "my-sa".into(),
                namespace: Some("default".into()),
                ..Default::default()
            }],
            ..Default::default()
        }];

        let resource = ResourceRef::ServiceAccount("my-sa".into(), "default".into());
        let result = resolve_rbac_bindings_from_snapshot(&resource, &snapshot);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "ServiceAccount my-sa");
        assert_eq!(result[0].children.len(), 1);
        assert_eq!(result[0].children[0].label, "RoleBinding my-rb");
        assert_eq!(result[0].children[0].children.len(), 1);
        assert_eq!(result[0].children[0].children[0].label, "Role my-role");
    }

    #[test]
    fn resolve_rbac_cluster_role_binding_shows_role_and_subjects() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.cluster_roles = vec![ClusterRoleInfo {
            name: "admin".into(),
            ..Default::default()
        }];
        snapshot.cluster_role_bindings = vec![ClusterRoleBindingInfo {
            name: "admin-crb".into(),
            role_ref_kind: "ClusterRole".into(),
            role_ref_name: "admin".into(),
            subjects: vec![RoleBindingSubject {
                kind: "ServiceAccount".into(),
                name: "ops-sa".into(),
                namespace: Some("ops".into()),
                ..Default::default()
            }],
            ..Default::default()
        }];

        let resource = ResourceRef::ClusterRoleBinding("admin-crb".into());
        let result = resolve_rbac_bindings_from_snapshot(&resource, &snapshot);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "ClusterRoleBinding admin-crb");
        // First child = ClusterRole, second child = subject
        assert!(!result[0].children.is_empty());
        assert_eq!(result[0].children[0].label, "ClusterRole admin");
        assert!(!result[0].children[0].not_found);
    }

    #[test]
    fn resolve_flux_lineage_returns_related_resources_by_source_url() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.flux_resources = vec![
            FluxResourceInfo {
                name: "my-app".into(),
                namespace: Some("flux-system".into()),
                kind: "Kustomization".into(),
                group: "kustomize.toolkit.fluxcd.io".into(),
                plural: "kustomizations".into(),
                status: "Ready".into(),
                source_url: Some("https://github.com/org/repo".into()),
                ..Default::default()
            },
            FluxResourceInfo {
                name: "my-repo".into(),
                namespace: Some("flux-system".into()),
                kind: "GitRepository".into(),
                group: "source.toolkit.fluxcd.io".into(),
                plural: "gitrepositories".into(),
                status: "Ready".into(),
                source_url: Some("https://github.com/org/repo".into()),
                ..Default::default()
            },
            FluxResourceInfo {
                name: "unrelated".into(),
                namespace: Some("flux-system".into()),
                kind: "HelmRelease".into(),
                group: "helm.toolkit.fluxcd.io".into(),
                plural: "helmreleases".into(),
                status: "Ready".into(),
                source_url: Some("https://charts.example.com".into()),
                ..Default::default()
            },
            FluxResourceInfo {
                name: "other-ns-resource".into(),
                namespace: Some("other".into()),
                kind: "HelmRelease".into(),
                group: "helm.toolkit.fluxcd.io".into(),
                plural: "helmreleases".into(),
                status: "Ready".into(),
                source_url: Some("https://github.com/org/repo".into()),
                ..Default::default()
            },
        ];

        let resource = ResourceRef::CustomResource {
            group: "kustomize.toolkit.fluxcd.io".into(),
            kind: "Kustomization".into(),
            plural: "kustomizations".into(),
            version: "v1".into(),
            name: "my-app".into(),
            namespace: Some("flux-system".into()),
        };
        let result = resolve_flux_lineage_from_snapshot(&resource, &snapshot);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "Kustomization my-app");
        // Should find my-repo (same source_url, same ns), not unrelated or other-ns-resource
        assert_eq!(result[0].children.len(), 1);
        assert_eq!(result[0].children[0].label, "GitRepository my-repo");
    }

    #[test]
    fn resolve_flux_lineage_excludes_different_namespace() {
        use crate::k8s::dtos::*;
        use crate::state::ClusterSnapshot;

        let mut snapshot = ClusterSnapshot::default();
        snapshot.flux_resources = vec![
            FluxResourceInfo {
                name: "my-app".into(),
                namespace: Some("flux-system".into()),
                kind: "Kustomization".into(),
                group: "kustomize.toolkit.fluxcd.io".into(),
                plural: "kustomizations".into(),
                status: "Ready".into(),
                ..Default::default()
            },
            FluxResourceInfo {
                name: "my-app".into(),
                namespace: Some("other".into()),
                kind: "Kustomization".into(),
                group: "kustomize.toolkit.fluxcd.io".into(),
                plural: "kustomizations".into(),
                status: "Ready".into(),
                ..Default::default()
            },
        ];

        let resource = ResourceRef::CustomResource {
            group: "kustomize.toolkit.fluxcd.io".into(),
            kind: "Kustomization".into(),
            plural: "kustomizations".into(),
            version: "v1".into(),
            name: "my-app".into(),
            namespace: Some("flux-system".into()),
        };
        let result = resolve_flux_lineage_from_snapshot(&resource, &snapshot);
        // No same-ns related resources that match by source_url or name prefix
        assert!(result.is_empty());
    }
}
