//! Core data model and tree flattening for the Relationship Explorer.

use std::collections::HashSet;

use crate::app::{AppView, ResourceRef};
use crate::k8s::dtos::OwnerRefInfo;
use crate::policy::RelationshipCapability;
use crate::state::ClusterSnapshot;

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
fn count_descendants(nodes: &[RelationNode], counter: &mut usize) {
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

/// Get the namespace for a ResourceRef (None for cluster-scoped).
fn resource_namespace(resource: &ResourceRef) -> Option<&str> {
    match resource {
        ResourceRef::Pod(_, ns)
        | ResourceRef::Deployment(_, ns)
        | ResourceRef::StatefulSet(_, ns)
        | ResourceRef::DaemonSet(_, ns)
        | ResourceRef::ReplicaSet(_, ns)
        | ResourceRef::ReplicationController(_, ns)
        | ResourceRef::Job(_, ns)
        | ResourceRef::CronJob(_, ns)
        | ResourceRef::Service(_, ns)
        | ResourceRef::Endpoint(_, ns)
        | ResourceRef::Ingress(_, ns)
        | ResourceRef::Pvc(_, ns)
        | ResourceRef::ServiceAccount(_, ns)
        | ResourceRef::Role(_, ns)
        | ResourceRef::RoleBinding(_, ns) => Some(ns.as_str()),
        ResourceRef::Pv(name)
        | ResourceRef::StorageClass(name)
        | ResourceRef::IngressClass(name)
        | ResourceRef::ClusterRole(name)
        | ResourceRef::ClusterRoleBinding(name)
        | ResourceRef::Node(name) => {
            let _ = name;
            None
        }
        ResourceRef::CustomResource { namespace, .. } => namespace.as_deref(),
        _ => None,
    }
}

/// Get the name for a ResourceRef.
fn resource_name(resource: &ResourceRef) -> &str {
    match resource {
        ResourceRef::Pod(name, _)
        | ResourceRef::Deployment(name, _)
        | ResourceRef::StatefulSet(name, _)
        | ResourceRef::DaemonSet(name, _)
        | ResourceRef::ReplicaSet(name, _)
        | ResourceRef::ReplicationController(name, _)
        | ResourceRef::Job(name, _)
        | ResourceRef::CronJob(name, _)
        | ResourceRef::Service(name, _)
        | ResourceRef::Endpoint(name, _)
        | ResourceRef::Ingress(name, _)
        | ResourceRef::Pvc(name, _)
        | ResourceRef::Pv(name)
        | ResourceRef::StorageClass(name)
        | ResourceRef::IngressClass(name)
        | ResourceRef::ServiceAccount(name, _)
        | ResourceRef::ClusterRole(name)
        | ResourceRef::Role(name, _)
        | ResourceRef::ClusterRoleBinding(name)
        | ResourceRef::RoleBinding(name, _)
        | ResourceRef::Node(name) => name.as_str(),
        ResourceRef::CustomResource { name, .. } => name.as_str(),
        _ => "",
    }
}

/// Build a human-readable kind string for a ResourceRef.
fn resource_kind_label(resource: &ResourceRef) -> &str {
    match resource {
        ResourceRef::Pod(_, _) => "Pod",
        ResourceRef::Deployment(_, _) => "Deployment",
        ResourceRef::StatefulSet(_, _) => "StatefulSet",
        ResourceRef::DaemonSet(_, _) => "DaemonSet",
        ResourceRef::ReplicaSet(_, _) => "ReplicaSet",
        ResourceRef::ReplicationController(_, _) => "ReplicationController",
        ResourceRef::Job(_, _) => "Job",
        ResourceRef::CronJob(_, _) => "CronJob",
        ResourceRef::Service(_, _) => "Service",
        ResourceRef::Endpoint(_, _) => "Endpoint",
        ResourceRef::Ingress(_, _) => "Ingress",
        ResourceRef::IngressClass(_) => "IngressClass",
        ResourceRef::Pvc(_, _) => "PersistentVolumeClaim",
        ResourceRef::Pv(_) => "PersistentVolume",
        ResourceRef::StorageClass(_) => "StorageClass",
        ResourceRef::ServiceAccount(_, _) => "ServiceAccount",
        ResourceRef::ClusterRole(_) => "ClusterRole",
        ResourceRef::Role(_, _) => "Role",
        ResourceRef::ClusterRoleBinding(_) => "ClusterRoleBinding",
        ResourceRef::RoleBinding(_, _) => "RoleBinding",
        ResourceRef::Node(_) => "Node",
        ResourceRef::CustomResource { kind, .. } => kind.as_str(),
        _ => "Resource",
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
            .map(|_c| String::new()),
        _ => None,
    }
}

/// Create a RelationNode for a known ResourceRef.
fn make_node(
    resource: ResourceRef,
    snapshot: &ClusterSnapshot,
    relation: RelationKind,
) -> RelationNode {
    let label = format!(
        "{} {}",
        resource_kind_label(&resource),
        resource_name(&resource)
    );
    let status = resource_status(&resource, snapshot);
    let namespace = resource_namespace(&resource).map(|s| s.to_string());
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
    let name = resource_name(resource);
    let kind = resource_kind_label(resource);
    let ns = resource_namespace(resource).unwrap_or("");

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

/// Walk owner references upward from a resource, returning the chain
/// top-down (root owner first). Also finds resources owned by the target.
pub fn resolve_owner_chain_from_snapshot(
    resource: &ResourceRef,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    let ns = match resource_namespace(resource) {
        Some(ns) => ns.to_string(),
        None => return vec![],
    };

    // Walk up the owner chain, collecting (resource, owner_refs) pairs.
    // We stop when there are no more owner refs or we can't find the owner.
    let mut chain: Vec<ResourceRef> = vec![resource.clone()];
    let mut current = resource.clone();

    loop {
        let owner_refs = get_owner_refs(&current, snapshot);
        if owner_refs.is_empty() {
            break;
        }
        // Take the first owner ref (typical case)
        let oref = &owner_refs[0];
        match find_resource_for_owner_ref(oref, &ns, snapshot) {
            Some(parent_ref) => {
                chain.push(parent_ref.clone());
                current = parent_ref;
            }
            None => {
                // Owner not found in snapshot — represent as not_found
                let placeholder =
                    ResourceRef::Pod(format!("__not_found__{}", oref.name), ns.clone());
                // We use a sentinel to signal not_found; handled below
                let _ = placeholder;
                // Push a dummy entry to mark not_found at the top
                chain.push(ResourceRef::Pod(
                    format!("__not_found__{}__{}", oref.kind, oref.name),
                    ns.clone(),
                ));
                break;
            }
        }
    }

    // chain is bottom-up; reverse to top-down
    chain.reverse();

    // Build the tree top-down with nesting
    // The top of chain owns the next, etc.
    // We also append owned resources (downward) to the target resource.

    // Build owned children for the original resource
    let target_owned: Vec<RelationNode> = find_owned_resources(resource, snapshot)
        .into_iter()
        .map(|r| make_node(r, snapshot, RelationKind::Owned))
        .collect();

    // Build the nested tree top-down with nesting.
    fn build_chain_tree(
        chain: &[ResourceRef],
        snapshot: &ClusterSnapshot,
        target: &ResourceRef,
        target_owned: Vec<RelationNode>,
    ) -> Vec<RelationNode> {
        if chain.is_empty() {
            return vec![];
        }

        let first = &chain[0];

        // Check if this is a not_found sentinel
        let is_not_found =
            matches!(first, ResourceRef::Pod(name, _) if name.starts_with("__not_found__"));

        if is_not_found {
            // Extract kind and name from sentinel
            let sentinel_name = resource_name(first);
            let parts: Vec<&str> = sentinel_name
                .strip_prefix("__not_found__")
                .unwrap_or("")
                .splitn(3, "__")
                .collect();
            let (kind, name) = if parts.len() >= 2 {
                (parts[0], parts[1])
            } else {
                ("Unknown", sentinel_name)
            };
            let oref = OwnerRefInfo {
                kind: kind.to_string(),
                name: name.to_string(),
                uid: String::new(),
            };
            return vec![make_not_found_node(&oref, RelationKind::Owner)];
        }

        let is_target = first == target;

        let mut node = if is_target {
            let mut n = make_node(first.clone(), snapshot, RelationKind::Root);
            n.children = target_owned;
            n
        } else {
            make_node(first.clone(), snapshot, RelationKind::Owner)
        };

        if chain.len() > 1 && !is_target {
            let rest = &chain[1..];
            let children = build_chain_tree(rest, snapshot, target, vec![]);
            node.children = children;
        }

        vec![node]
    }

    build_chain_tree(&chain, snapshot, resource, target_owned)
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

}
