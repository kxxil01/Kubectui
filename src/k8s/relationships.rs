//! Core data model and tree flattening for the Relationship Explorer.

use std::collections::HashSet;

use crate::app::{AppView, ResourceRef};
use crate::policy::RelationshipCapability;

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
}
