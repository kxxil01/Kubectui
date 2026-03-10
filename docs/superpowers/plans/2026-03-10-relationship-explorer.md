# Relationship Explorer Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a workbench-hosted relationship explorer that renders an indented, expandable tree of related Kubernetes resources.

**Architecture:** New `Relations` workbench tab opened via `w` in detail view or action palette. Relationships resolved async from in-memory snapshot with API fallback. Six chain types: owner, service, ingress, storage, Flux, RBAC. Tree rendered with expand/collapse, Enter jumps to related resource.

**Tech Stack:** Rust, ratatui, kube-rs, tokio async channels

**Spec:** `docs/superpowers/specs/2026-03-10-relationship-explorer-design.md`

---

## File Structure

### New files
- `src/k8s/relationships.rs` — RelationNode model, RelationKind enum, tree flattening, 6 resolver functions, orchestrator, resource_to_view helper, tests
- `src/ui/views/relations.rs` — Relations tab rendering (tree drawing, cursor, connectors)

### Modified files
- `src/k8s/mod.rs` — add `pub mod relationships;`
- `src/k8s/dtos.rs` — enhance OwnerRefInfo (+uid), add owner_references to ReplicaSetInfo/JobInfo, add selector to ServiceInfo, add backend_services to IngressInfo
- `src/k8s/client.rs` — populate new DTO fields during fetching
- `src/workbench.rs` — RelationsTabState, WorkbenchTabKind/Key/State variants
- `src/policy.rs` — DetailAction::ViewRelationships, supports_detail_action, ORDER array
- `src/app.rs` — AppAction::OpenRelationships, `w` keybinding
- `src/events/input.rs` — handle AppAction::OpenRelationships
- `src/main.rs` — relations channel, spawn resolver, receive result, open tab handler
- `src/ui/components/command_palette.rs` — add Relations alias to ACTION_ALIASES
- `src/ui/components/help_overlay.rs` — add `w` keybinding
- `src/ui/components/workbench.rs` — add Relations render dispatch arm
- `src/ui/views/mod.rs` — add `pub mod relations;`

---

## Chunk 1: Foundation (Data Model + Tab Infrastructure)

### Task 1: Enhance OwnerRefInfo DTO

**Files:**
- Modify: `src/k8s/dtos.rs` — OwnerRefInfo struct (lines 26-30)
- Modify: `src/k8s/client.rs` — PodInfo owner_references population (lines 271-280)

- [ ] **Step 1: Add uid field to OwnerRefInfo**

In `src/k8s/dtos.rs`, add `uid` to the struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct OwnerRefInfo {
    pub kind: String,
    pub name: String,
    pub uid: String,
}
```

- [ ] **Step 2: Update PodInfo owner_references population to include uid**

In `src/k8s/client.rs`, find the `owner_references` population (around line 271) and add the uid field:

```rust
owner_references: pod
    .metadata
    .owner_references
    .unwrap_or_default()
    .into_iter()
    .map(|oref| crate::k8s::dtos::OwnerRefInfo {
        kind: oref.kind,
        name: oref.name,
        uid: oref.uid,
    })
    .collect(),
```

- [ ] **Step 3: Run tests to verify no regressions**

Run: `cargo test --all-targets --all-features`
Expected: All existing tests pass (uid defaults to empty string in test fixtures via Default).

- [ ] **Step 4: Commit**

```
git add src/k8s/dtos.rs src/k8s/client.rs
git commit -m "refactor(dtos): add uid to OwnerRefInfo for relationship resolution"
```

---

### Task 2: Add owner_references to workload DTOs

**Files:**
- Modify: `src/k8s/dtos.rs` — ReplicaSetInfo, JobInfo structs
- Modify: `src/k8s/client.rs` — fetch_replicasets, fetch_jobs population code

- [ ] **Step 1: Add owner_references field to ReplicaSetInfo**

In `src/k8s/dtos.rs`, add to `ReplicaSetInfo` (after `created_at`):

```rust
pub owner_references: Vec<OwnerRefInfo>,
```

- [ ] **Step 2: Add owner_references field to JobInfo**

In `src/k8s/dtos.rs`, add to `JobInfo` (after `created_at`):

```rust
pub owner_references: Vec<OwnerRefInfo>,
```

- [ ] **Step 3: Populate owner_references in fetch_replicasets**

In `src/k8s/client.rs`, find where `ReplicaSetInfo` is constructed and add:

```rust
owner_references: rs
    .metadata
    .owner_references
    .unwrap_or_default()
    .into_iter()
    .map(|oref| crate::k8s::dtos::OwnerRefInfo {
        kind: oref.kind,
        name: oref.name,
        uid: oref.uid,
    })
    .collect(),
```

- [ ] **Step 4: Populate owner_references in fetch_jobs**

Same pattern for `JobInfo` construction in the jobs fetch function.

- [ ] **Step 5: Run tests**

Run: `cargo test --all-targets --all-features`
Expected: PASS

- [ ] **Step 6: Commit**

```
git add src/k8s/dtos.rs src/k8s/client.rs
git commit -m "refactor(dtos): add owner_references to ReplicaSetInfo and JobInfo"
```

---

### Task 3: Add selector to ServiceInfo and backend_services to IngressInfo

**Files:**
- Modify: `src/k8s/dtos.rs` — ServiceInfo, IngressInfo structs
- Modify: `src/k8s/client.rs` — fetch_services, fetch_ingresses

- [ ] **Step 1: Add selector field to ServiceInfo**

In `src/k8s/dtos.rs`, add to `ServiceInfo` (after `ports`):

```rust
pub selector: std::collections::BTreeMap<String, String>,
```

- [ ] **Step 2: Populate selector in fetch_services**

In `src/k8s/client.rs`, find the ServiceInfo construction (around line 337) and add:

```rust
selector: svc
    .spec
    .as_ref()
    .and_then(|spec| spec.selector.clone())
    .unwrap_or_default(),
```

- [ ] **Step 3: Add backend_services field to IngressInfo**

In `src/k8s/dtos.rs`, add to `IngressInfo` (after `ports`):

```rust
pub backend_services: Vec<(String, String)>,
```

This stores `(service_name, port)` pairs extracted from ingress rules.

- [ ] **Step 4: Populate backend_services in fetch_ingresses**

In `src/k8s/client.rs`, find the IngressInfo construction (around line 1229). Before it, extract backends:

```rust
let backend_services: Vec<(String, String)> = ing
    .spec
    .as_ref()
    .map(|spec| {
        let mut backends = Vec::new();
        if let Some(default_backend) = &spec.default_backend {
            if let Some(svc) = &default_backend.service {
                let port = svc.port.as_ref().map(|p| {
                    p.name.clone().unwrap_or_else(|| {
                        p.number.map(|n| n.to_string()).unwrap_or_default()
                    })
                }).unwrap_or_default();
                backends.push((svc.name.clone(), port));
            }
        }
        for rule in spec.rules.as_deref().unwrap_or_default() {
            if let Some(http) = &rule.http {
                for path in &http.paths {
                    if let Some(svc) = &path.backend.service {
                        let port = svc.port.as_ref().map(|p| {
                            p.name.clone().unwrap_or_else(|| {
                                p.number.map(|n| n.to_string()).unwrap_or_default()
                            })
                        }).unwrap_or_default();
                        backends.push((svc.name.clone(), port));
                    }
                }
            }
        }
        backends.sort();
        backends.dedup();
        backends
    })
    .unwrap_or_default();
```

Then add `backend_services,` to the IngressInfo construction.

- [ ] **Step 5: Run tests**

Run: `cargo test --all-targets --all-features`
Expected: PASS

- [ ] **Step 6: Commit**

```
git add src/k8s/dtos.rs src/k8s/client.rs
git commit -m "refactor(dtos): add selector to ServiceInfo, backend_services to IngressInfo"
```

---

### Task 4: Add RelationNode model and tree flattening

**Files:**
- Create: `src/k8s/relationships.rs`
- Modify: `src/k8s/mod.rs`

- [ ] **Step 1: Write tests for RelationNode and tree flattening**

Create `src/k8s/relationships.rs` with the data model and tests first:

```rust
//! Relationship resolution for the relationship explorer workbench tab.

use std::collections::HashSet;

use crate::app::{AppView, ResourceRef};
use crate::policy::RelationshipCapability;

/// A node in the relationship tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationNode {
    pub resource: Option<ResourceRef>,
    pub label: String,
    pub status: Option<String>,
    pub namespace: Option<String>,
    pub relation: RelationKind,
    pub not_found: bool,
    pub children: Vec<RelationNode>,
}

/// How a node relates to its parent in the tree.
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

/// Flattened tree node for rendering.
#[derive(Debug, Clone)]
pub struct FlatNode {
    pub depth: usize,
    pub tree_index: usize,
    pub resource: Option<ResourceRef>,
    pub label: String,
    pub status: Option<String>,
    pub namespace: Option<String>,
    pub relation: RelationKind,
    pub not_found: bool,
    pub is_last_child: bool,
    pub parent_is_last: Vec<bool>,
    pub has_children: bool,
    pub expanded: bool,
}

/// Flatten the tree into visible lines based on expand/collapse state.
pub fn flatten_tree(
    nodes: &[RelationNode],
    expanded: &HashSet<usize>,
) -> Vec<FlatNode> {
    let mut result = Vec::new();
    let mut counter = 0;
    for (i, node) in nodes.iter().enumerate() {
        let is_last = i == nodes.len() - 1;
        flatten_node(node, 0, is_last, &[], expanded, &mut counter, &mut result);
    }
    result
}

fn flatten_node(
    node: &RelationNode,
    depth: usize,
    is_last_child: bool,
    parent_is_last: &[bool],
    expanded: &HashSet<usize>,
    counter: &mut usize,
    result: &mut Vec<FlatNode>,
) {
    let index = *counter;
    *counter += 1;
    let has_children = !node.children.is_empty();
    let is_expanded = expanded.contains(&index);

    result.push(FlatNode {
        depth,
        tree_index: index,
        resource: node.resource.clone(),
        label: node.label.clone(),
        status: node.status.clone(),
        namespace: node.namespace.clone(),
        relation: node.relation,
        not_found: node.not_found,
        is_last_child,
        parent_is_last: parent_is_last.to_vec(),
        has_children,
        expanded: is_expanded,
    });

    if has_children && is_expanded {
        let mut new_parent_is_last = parent_is_last.to_vec();
        new_parent_is_last.push(is_last_child);
        for (i, child) in node.children.iter().enumerate() {
            let child_is_last = i == node.children.len() - 1;
            flatten_node(
                child,
                depth + 1,
                child_is_last,
                &new_parent_is_last,
                expanded,
                counter,
                result,
            );
        }
    } else if has_children {
        // Still need to count collapsed children for stable indexing
        fn skip_count(node: &RelationNode, counter: &mut usize) {
            for child in &node.children {
                *counter += 1;
                skip_count(child, counter);
            }
        }
        skip_count(node, counter);
    }
}

/// Map a ResourceRef to the AppView used for capability lookup.
pub fn resource_to_view(resource: &ResourceRef) -> Option<AppView> {
    match resource {
        ResourceRef::Pod(..) => Some(AppView::Pods),
        ResourceRef::Deployment(..) => Some(AppView::Deployments),
        ResourceRef::StatefulSet(..) => Some(AppView::StatefulSets),
        ResourceRef::DaemonSet(..) => Some(AppView::DaemonSets),
        ResourceRef::ReplicaSet(..) => Some(AppView::ReplicaSets),
        ResourceRef::ReplicationController(..) => Some(AppView::ReplicationControllers),
        ResourceRef::Job(..) => Some(AppView::Jobs),
        ResourceRef::CronJob(..) => Some(AppView::CronJobs),
        ResourceRef::Service(..) => Some(AppView::Services),
        ResourceRef::Endpoints(..) => Some(AppView::Endpoints),
        ResourceRef::Ingress(..) => Some(AppView::Ingresses),
        ResourceRef::IngressClass(..) => Some(AppView::IngressClasses),
        ResourceRef::PersistentVolumeClaim(..) => Some(AppView::PersistentVolumeClaims),
        ResourceRef::PersistentVolume(..) => Some(AppView::PersistentVolumes),
        ResourceRef::StorageClass(..) => Some(AppView::StorageClasses),
        ResourceRef::ServiceAccount(..) => Some(AppView::ServiceAccounts),
        ResourceRef::ClusterRole(..) => Some(AppView::ClusterRoles),
        ResourceRef::Role(..) => Some(AppView::Roles),
        ResourceRef::ClusterRoleBinding(..) => Some(AppView::ClusterRoleBindings),
        ResourceRef::RoleBinding(..) => Some(AppView::RoleBindings),
        ResourceRef::CustomResource { group, .. }
            if group.ends_with(".fluxcd.io") => Some(AppView::FluxCDAll),
        _ => None,
    }
}

/// Check whether a resource has any relationship capabilities.
pub fn resource_has_relationships(resource: &ResourceRef) -> bool {
    resource_to_view(resource)
        .map(|view| !view.relationship_capabilities().is_empty())
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

    fn leaf(name: &str) -> RelationNode {
        RelationNode {
            resource: Some(ResourceRef::Pod(name.to_string(), "ns".to_string())),
            label: format!("Pod {name}"),
            status: Some("Running".to_string()),
            namespace: Some("ns".to_string()),
            relation: RelationKind::Owned,
            not_found: false,
            children: vec![],
        }
    }

    fn parent(name: &str, children: Vec<RelationNode>) -> RelationNode {
        RelationNode {
            resource: Some(ResourceRef::Deployment(name.to_string(), "ns".to_string())),
            label: format!("Deployment {name}"),
            status: Some("Ready".to_string()),
            namespace: Some("ns".to_string()),
            relation: RelationKind::Root,
            not_found: false,
            children,
        }
    }

    #[test]
    fn flatten_empty_tree() {
        let flat = flatten_tree(&[], &HashSet::new());
        assert!(flat.is_empty());
    }

    #[test]
    fn flatten_single_node() {
        let tree = vec![leaf("pod-0")];
        let flat = flatten_tree(&tree, &HashSet::new());
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].label, "Pod pod-0");
        assert_eq!(flat[0].depth, 0);
        assert!(!flat[0].has_children);
    }

    #[test]
    fn flatten_expanded_parent_shows_children() {
        let tree = vec![parent("deploy", vec![leaf("pod-0"), leaf("pod-1")])];
        let mut expanded = HashSet::new();
        expanded.insert(0); // parent index
        let flat = flatten_tree(&tree, &expanded);
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[0].label, "Deployment deploy");
        assert_eq!(flat[0].depth, 0);
        assert!(flat[0].has_children);
        assert!(flat[0].expanded);
        assert_eq!(flat[1].label, "Pod pod-0");
        assert_eq!(flat[1].depth, 1);
        assert!(!flat[1].is_last_child);
        assert_eq!(flat[2].label, "Pod pod-1");
        assert_eq!(flat[2].depth, 1);
        assert!(flat[2].is_last_child);
    }

    #[test]
    fn flatten_collapsed_parent_hides_children() {
        let tree = vec![parent("deploy", vec![leaf("pod-0"), leaf("pod-1")])];
        let flat = flatten_tree(&tree, &HashSet::new());
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].label, "Deployment deploy");
        assert!(!flat[0].expanded);
    }

    #[test]
    fn stable_indices_across_collapse() {
        // Two parents, each with children
        let tree = vec![
            parent("d1", vec![leaf("p1"), leaf("p2")]),
            parent("d2", vec![leaf("p3")]),
        ];
        // Expand only second parent (index 3: d1=0, p1=1, p2=2, d2=3)
        let mut expanded = HashSet::new();
        expanded.insert(3);
        let flat = flatten_tree(&tree, &expanded);
        // d1 (collapsed), d2 (expanded), p3
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[0].tree_index, 0); // d1
        assert_eq!(flat[1].tree_index, 3); // d2
        assert_eq!(flat[2].tree_index, 4); // p3
    }

    #[test]
    fn resource_to_view_maps_core_types() {
        assert_eq!(
            resource_to_view(&ResourceRef::Pod("p".into(), "ns".into())),
            Some(AppView::Pods)
        );
        assert_eq!(
            resource_to_view(&ResourceRef::Service("s".into(), "ns".into())),
            Some(AppView::Services)
        );
        assert_eq!(
            resource_to_view(&ResourceRef::Node("n".into())),
            None
        );
    }

    #[test]
    fn resource_has_relationships_for_supported_types() {
        assert!(resource_has_relationships(&ResourceRef::Deployment("d".into(), "ns".into())));
        assert!(resource_has_relationships(&ResourceRef::Service("s".into(), "ns".into())));
        assert!(resource_has_relationships(&ResourceRef::PersistentVolumeClaim("p".into(), "ns".into())));
        assert!(!resource_has_relationships(&ResourceRef::Node("n".into())));
        assert!(!resource_has_relationships(&ResourceRef::ConfigMap("c".into(), "ns".into())));
    }

    #[test]
    fn parent_is_last_tracks_ancestors() {
        let tree = vec![parent("d1", vec![
            parent("rs1", vec![leaf("pod-0")]),
        ])];
        let mut expanded = HashSet::new();
        expanded.insert(0); // d1
        expanded.insert(1); // rs1
        let flat = flatten_tree(&tree, &expanded);
        assert_eq!(flat.len(), 3);
        // pod-0's parent_is_last should track both ancestor levels
        assert_eq!(flat[2].parent_is_last, vec![true, true]);
    }
}
```

- [ ] **Step 2: Add mod declaration**

In `src/k8s/mod.rs`, add:

```rust
pub mod relationships;
```

- [ ] **Step 3: Run tests**

Run: `cargo test --all-targets --all-features`
Expected: All tests pass including new relationship tests.

- [ ] **Step 4: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No warnings.

- [ ] **Step 5: Commit**

```
git add src/k8s/relationships.rs src/k8s/mod.rs
git commit -m "feat(relationships): add RelationNode model and tree flattening"
```

---

### Task 5: Add RelationsTabState and workbench infrastructure

**Files:**
- Modify: `src/workbench.rs` — add Relations tab kind, key, state

- [ ] **Step 1: Add Relations variants to WorkbenchTabKind, WorkbenchTabKey, WorkbenchTabState**

In `src/workbench.rs`:

Add to `WorkbenchTabKind` enum:
```rust
Relations,
```

Add to `WorkbenchTabKind::title()`:
```rust
WorkbenchTabKind::Relations => "Relations",
```

Add to `WorkbenchTabKey` enum:
```rust
Relations(ResourceRef),
```

Add the state struct (after `PortForwardTabState`):
```rust
#[derive(Debug, Clone)]
pub struct RelationsTabState {
    pub resource: ResourceRef,
    pub tree: Vec<crate::k8s::relationships::RelationNode>,
    pub cursor: usize,
    pub expanded: std::collections::HashSet<usize>,
    pub loading: bool,
    pub error: Option<String>,
}

impl RelationsTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            tree: Vec::new(),
            cursor: 0,
            expanded: std::collections::HashSet::new(),
            loading: true,
            error: None,
        }
    }
}
```

Add to `WorkbenchTabState` enum:
```rust
Relations(RelationsTabState),
```

Add to `WorkbenchTabState::kind()`:
```rust
Self::Relations(_) => WorkbenchTabKind::Relations,
```

Add to `WorkbenchTabState::key()`:
```rust
Self::Relations(tab) => WorkbenchTabKey::Relations(tab.resource.clone()),
```

Add to `WorkbenchTabState::title()`:
```rust
Self::Relations(tab) => format!("Relations {}", resource_title(&tab.resource)),
```

- [ ] **Step 2: Write test for Relations tab deduplication**

Add to `#[cfg(test)] mod tests` in workbench.rs:

```rust
#[test]
fn relations_tab_deduplicates_by_resource() {
    let mut state = WorkbenchState::default();
    let first = state.open_tab(WorkbenchTabState::Relations(RelationsTabState::new(
        pod("pod-0"),
    )));
    let second = state.open_tab(WorkbenchTabState::Relations(RelationsTabState::new(
        pod("pod-0"),
    )));
    assert_eq!(first, second);
    assert_eq!(state.tabs.len(), 1);
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --all-targets --all-features`
Expected: PASS

- [ ] **Step 4: Commit**

```
git add src/workbench.rs
git commit -m "feat(workbench): add Relations tab kind, key, and state"
```

---

### Task 6: Add DetailAction::ViewRelationships to policy layer

**Files:**
- Modify: `src/policy.rs`

- [ ] **Step 1: Add ViewRelationships variant to DetailAction**

In `src/policy.rs`, add to `DetailAction` enum (after `Trigger`):
```rust
ViewRelationships,
```

Add to `DetailAction::ORDER` array (increase size to 13):
```rust
pub const ORDER: [DetailAction; 13] = [
    // ... existing 12 ...
    DetailAction::ViewRelationships,
];
```

Add to `DetailAction::key_hint()`:
```rust
DetailAction::ViewRelationships => "[w]",
```

Add to `DetailAction::label()`:
```rust
DetailAction::ViewRelationships => "Relations",
```

- [ ] **Step 2: Add supports_detail_action match arm**

In `ResourceRef::supports_detail_action()`, add:

```rust
DetailAction::ViewRelationships => {
    crate::k8s::relationships::resource_has_relationships(self)
}
```

- [ ] **Step 3: Add ViewRelationships to requires_clear_surface in DetailViewState::supports_action**

In `DetailViewState::supports_action()`, add `DetailAction::ViewRelationships` to the `requires_clear_surface` match (it should match alongside all other actions that need a clear surface).

- [ ] **Step 4: Write tests**

Add to `mod tests` in policy.rs:

```rust
#[test]
fn view_relationships_available_for_relationship_capable_resources() {
    let pod = ResourceRef::Pod("pod-0".to_string(), "ns".to_string());
    assert!(pod.supports_detail_action(DetailAction::ViewRelationships));

    let deploy = ResourceRef::Deployment("api".to_string(), "ns".to_string());
    assert!(deploy.supports_detail_action(DetailAction::ViewRelationships));

    let svc = ResourceRef::Service("svc".to_string(), "ns".to_string());
    assert!(svc.supports_detail_action(DetailAction::ViewRelationships));

    let pvc = ResourceRef::PersistentVolumeClaim("pvc".to_string(), "ns".to_string());
    assert!(pvc.supports_detail_action(DetailAction::ViewRelationships));
}

#[test]
fn view_relationships_unavailable_for_non_relationship_resources() {
    let node = ResourceRef::Node("node-0".to_string());
    assert!(!node.supports_detail_action(DetailAction::ViewRelationships));

    let cm = ResourceRef::ConfigMap("cm".to_string(), "ns".to_string());
    assert!(!cm.supports_detail_action(DetailAction::ViewRelationships));
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --all-targets --all-features`
Expected: PASS

- [ ] **Step 6: Commit**

```
git add src/policy.rs
git commit -m "feat(policy): add DetailAction::ViewRelationships with capability check"
```

---

## Chunk 2: Wiring (App Actions + Keybinding + Palette + Event Loop)

### Task 7: Add AppAction::OpenRelationships and keybinding

**Files:**
- Modify: `src/app.rs` — AppAction enum, handle_key_event
- Modify: `src/events/input.rs` — apply_action

- [ ] **Step 1: Add AppAction variant**

In `src/app.rs`, add to `AppAction` enum (after `PaletteAction`):
```rust
OpenRelationships,
```

- [ ] **Step 2: Add `w` keybinding in handle_key_event**

In `src/app.rs` `handle_key_event()`, add after the Trigger (`T`) key handler and before the `Tab` handler, following the same pattern as other detail-view actions:

```rust
KeyCode::Char('w')
    if self
        .detail_view
        .as_ref()
        .is_some_and(|detail| detail.supports_action(DetailAction::ViewRelationships)) =>
{
    AppAction::OpenRelationships
}
```

- [ ] **Step 3: Handle in apply_action**

In `src/events/input.rs`, add to the `apply_action` match:
```rust
AppAction::OpenRelationships => true,
```

- [ ] **Step 4: Write test in events/input.rs**

```rust
#[test]
fn test_apply_action_open_relationships_returns_true() {
    let mut app = AppState::default();
    assert!(apply_action(AppAction::OpenRelationships, &mut app));
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --all-targets --all-features`
Expected: PASS

- [ ] **Step 6: Commit**

```
git add src/app.rs src/events/input.rs
git commit -m "feat(app): add AppAction::OpenRelationships with w keybinding"
```

---

### Task 8: Add Relations to action palette and help overlay

**Files:**
- Modify: `src/ui/components/command_palette.rs` — ACTION_ALIASES
- Modify: `src/ui/components/help_overlay.rs` — Detail View section

- [ ] **Step 1: Add Relations aliases to ACTION_ALIASES**

In `src/ui/components/command_palette.rs`, add to `ACTION_ALIASES` array (after the Trigger entry):

```rust
(
    DetailAction::ViewRelationships,
    &["relations", "relationships", "related", "web", "tree", "deps"],
),
```

- [ ] **Step 2: Add w keybinding to help overlay**

In `src/ui/components/help_overlay.rs`, in the "Detail View" section, add after the Trigger entry:

```rust
("w", "View relations"),
```

- [ ] **Step 3: Run tests**

Run: `cargo test --all-targets --all-features`
Expected: PASS (existing palette tests should still pass since filtering is dynamic)

- [ ] **Step 4: Commit**

```
git add src/ui/components/command_palette.rs src/ui/components/help_overlay.rs
git commit -m "feat(palette): add Relations action alias and help overlay entry"
```

---

### Task 9: Wire main.rs event loop — channel, handler, and result receiver

**Files:**
- Modify: `src/main.rs`

This is the most complex task. It adds:
1. A channel for relationship resolution results
2. The `AppAction::OpenRelationships` handler that opens the tab and spawns async resolution
3. A `tokio::select!` arm that receives results and updates the tab

- [ ] **Step 1: Add channel and result type**

Near the other channel declarations (around line 997), add:

```rust
let (relations_tx, mut relations_rx) =
    tokio::sync::mpsc::channel::<(ResourceRef, Result<Vec<crate::k8s::relationships::RelationNode>, String>)>(16);
```

- [ ] **Step 2: Add OpenRelationships handler**

In the action dispatch block, find where `OpenResourceYaml` is handled (around line 2298). Add the `OpenRelationships` handler following the same pattern:

```rust
AppAction::OpenRelationships => {
    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(&app, &cached_snapshot));
    let Some(resource) = resource else {
        app.set_error("No resource selected for relationship exploration.".to_string());
        continue;
    };
    app.detail_view = None;
    app.workbench.open_tab(WorkbenchTabState::Relations(
        RelationsTabState::new(resource.clone()),
    ));
    app.focus = Focus::Workbench;

    let tx = relations_tx.clone();
    let client_clone = client.clone(); // K8sClient, cheap Arc-like clone
    // Note: cloning the full snapshot is expensive for large clusters.
    // Acceptable for v1 since resolution is one-shot. Consider Arc<ClusterSnapshot> later.
    let snapshot_clone = cached_snapshot.clone();
    let requested_resource = resource.clone();
    tokio::spawn(async move {
        let result = crate::k8s::relationships::resolve_relationships(
            &requested_resource,
            &snapshot_clone,
            &client_clone,
        )
        .await
        .map_err(|err| format!("{err:#}"));
        let _ = tx.send((requested_resource, result)).await;
    });
}
```

- [ ] **Step 3: Add PaletteAction handler for ViewRelationships**

In the PaletteAction match block (where DetailAction variants are mapped to AppAction), add:

```rust
DetailAction::ViewRelationships => AppAction::OpenRelationships,
```

Note: Check the actual match syntax in the PaletteAction handler — some arms may use `Option` wrapping while others return directly. Follow the existing pattern exactly.

- [ ] **Step 4: Add tokio::select! result receiver arm**

In the `tokio::select!` block, add after the existing result receivers (e.g., after `probe_rx`):

```rust
result = relations_rx.recv() => {
    if let Some((requested_resource, result)) = result {
        let tab_key = WorkbenchTabKey::Relations(requested_resource.clone());
        if let Some(tab) = app.workbench.find_tab_mut(&tab_key) {
            if let WorkbenchTabState::Relations(ref mut state) = tab.state {
                state.loading = false;
                match result {
                    Ok(tree) => {
                        // Auto-expand top-level nodes
                        let mut expanded = std::collections::HashSet::new();
                        let mut counter = 0;
                        for section in &tree {
                            expanded.insert(counter);
                            counter += 1;
                            for child in &section.children {
                                expanded.insert(counter);
                                counter += 1;
                                fn count_children(n: &crate::k8s::relationships::RelationNode, c: &mut usize) {
                                    for ch in &n.children { *c += 1; count_children(ch, c); }
                                }
                                count_children(child, &mut counter);
                            }
                        }
                        state.expanded = expanded;
                        state.tree = tree;
                    }
                    Err(err) => {
                        state.error = Some(err);
                    }
                }
            }
        }
        needs_redraw = true;
    }
}
```

- [ ] **Step 5: Run tests and clippy**

Run: `cargo test --all-targets --all-features && cargo clippy --all-targets --all-features -- -D warnings`
Expected: PASS

- [ ] **Step 6: Commit**

```
git add src/main.rs
git commit -m "feat(main): wire relationship resolution channel and event handlers"
```

---

## Chunk 3: Resolvers

### Task 10: Implement resolve_owner_chain

**Files:**
- Modify: `src/k8s/relationships.rs`

- [ ] **Step 1: Write tests for owner chain resolution**

Add to the tests module in `relationships.rs`:

```rust
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

    // Should produce: Deployment -> ReplicaSet -> Pod
    assert!(!result.is_empty());
    // Root should be the deployment
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

    // Should still produce a tree but with not_found marker
    assert!(!result.is_empty());
    assert!(result[0].not_found);
}
```

- [ ] **Step 2: Implement resolve_owner_chain_from_snapshot**

This is the synchronous, snapshot-only version. Add to `relationships.rs`:

```rust
use crate::state::ClusterSnapshot;

/// Resolve owner chain from snapshot data only (no API calls).
pub fn resolve_owner_chain_from_snapshot(
    resource: &ResourceRef,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    // Walk upward from resource via owner_references
    let owner_refs = get_owner_refs(resource, snapshot);
    if owner_refs.is_empty() {
        // Try downward: find resources owned by this resource
        return find_owned_resources(resource, snapshot);
    }

    // Build chain upward, then reverse to show top-down
    let mut chain = vec![make_node_for_resource(resource, snapshot, RelationKind::Root)];
    let mut current_refs = owner_refs;
    let mut visited = std::collections::HashSet::new();
    visited.insert(resource.name().to_string());

    while !current_refs.is_empty() {
        let oref = &current_refs[0]; // Follow first (controller) owner
        if visited.contains(&oref.name) {
            break; // Cycle protection
        }
        visited.insert(oref.name.clone());

        if let Some((owner_resource, next_refs)) = find_resource_by_owner_ref(oref, resource, snapshot) {
            let node = make_node_for_resource(&owner_resource, snapshot, RelationKind::Owner);
            chain.push(node);
            current_refs = next_refs;
        } else {
            // Owner not found in snapshot
            chain.push(RelationNode {
                resource: None,
                label: format!("{} {}", oref.kind, oref.name),
                status: None,
                namespace: resource.namespace().map(|s| s.to_string()),
                relation: RelationKind::Owner,
                not_found: true,
                children: vec![],
            });
            break;
        }
    }

    // Reverse chain so top-level owner is root, nest children
    chain.reverse();
    // Also find downward-owned resources and attach to root
    let owned = find_owned_resources(resource, snapshot);

    // Build nested tree from chain
    let mut result = chain.remove(0);
    let mut current = &mut result;
    for node in chain {
        current.children.push(node);
        let last = current.children.len() - 1;
        current = &mut current.children[last];
    }
    // Attach owned resources as children of the original resource node
    for owned_node in owned {
        current.children.push(owned_node);
    }

    vec![result]
}
```

The helper functions `get_owner_refs`, `find_resource_by_owner_ref`, `find_owned_resources`, and `make_node_for_resource` need to be implemented. These look up owner references from each DTO type in the snapshot, find matching resources by kind+name+namespace, and create RelationNode entries. These are internal helpers — keep them private.

- [ ] **Step 3: Run tests**

Run: `cargo test --all-targets --all-features`
Expected: PASS

- [ ] **Step 4: Commit**

```
git add src/k8s/relationships.rs
git commit -m "feat(relationships): implement owner chain resolver from snapshot"
```

---

### Task 11: Implement resolve_service_backends

**Files:**
- Modify: `src/k8s/relationships.rs`

- [ ] **Step 1: Write tests**

```rust
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
    snapshot.pods = vec![
        PodInfo {
            name: "nginx-pod-1".into(),
            namespace: "default".into(),
            status: "Running".into(),
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

    assert_eq!(result.len(), 1); // service node
    assert_eq!(result[0].label, "Service nginx-svc");
    assert_eq!(result[0].children.len(), 1); // only nginx-pod-1 matches
    assert_eq!(result[0].children[0].label, "Pod nginx-pod-1");
}
```

- [ ] **Step 2: Implement resolve_service_backends_from_snapshot**

Match pods from snapshot where all service selector labels are present in pod labels (same namespace).

- [ ] **Step 3: Run tests**

Run: `cargo test --all-targets --all-features`
Expected: PASS

- [ ] **Step 4: Commit**

```
git add src/k8s/relationships.rs
git commit -m "feat(relationships): implement service backends resolver"
```

---

### Task 12: Implement resolve_ingress_backends, resolve_storage_bindings

**Files:**
- Modify: `src/k8s/relationships.rs`

- [ ] **Step 1: Write tests for ingress backends**

Test that IngressInfo.backend_services are matched to services in snapshot.

- [ ] **Step 2: Implement resolve_ingress_backends_from_snapshot**

For Ingress: match `backend_services` service names to ServiceInfo in snapshot (same namespace). For IngressClass: find all Ingresses with matching class.

- [ ] **Step 3: Write tests for storage bindings**

Test PVC → PV → StorageClass chain using PvcInfo.volume, PvInfo.storage_class.

- [ ] **Step 4: Implement resolve_storage_bindings_from_snapshot**

For PVC: follow `volume` → find PV by name → follow `storage_class` → find StorageClass by name.
For PV: follow `claim` → find PVC, follow `storage_class` → find StorageClass.
For StorageClass: find PVs with matching `storage_class`, then their bound PVCs.

- [ ] **Step 5: Run tests**

Run: `cargo test --all-targets --all-features`
Expected: PASS

- [ ] **Step 6: Commit**

```
git add src/k8s/relationships.rs
git commit -m "feat(relationships): implement ingress and storage resolvers"
```

---

### Task 13: Implement resolve_flux_lineage and resolve_rbac_bindings

**Files:**
- Modify: `src/k8s/relationships.rs`

- [ ] **Step 1: Write tests for RBAC bindings**

Test ServiceAccount → scan RoleBindingInfo subjects → find matching Role.

- [ ] **Step 2: Implement resolve_rbac_bindings_from_snapshot**

For ServiceAccount: scan `snapshot.role_bindings` and `snapshot.cluster_role_bindings` for subjects matching SA name/namespace. Follow `role_ref_name` and `role_ref_kind` to find Role or ClusterRole.
For Role/ClusterRole: find bindings that reference it.
For RoleBinding/ClusterRoleBinding: show the role it references and the subjects.

- [ ] **Step 3: Write tests for Flux lineage**

Test that Flux resources can find related Flux resources by name/kind matching.

- [ ] **Step 4: Implement resolve_flux_lineage_from_snapshot**

For Flux resources: match FluxResourceInfo entries in snapshot by cross-referencing names and kinds. Note: `FluxResourceInfo` has `source_url` but no structured `sourceRef` (kind/name). Use best-effort name matching between Flux resources of different kinds (e.g., HelmRelease → HelmRepository, Kustomization → GitRepository). This is intentionally limited in v1 — adding `source_ref_kind`/`source_ref_name` to `FluxResourceInfo` DTO is a follow-up enhancement.

- [ ] **Step 5: Run tests**

Run: `cargo test --all-targets --all-features`
Expected: PASS

- [ ] **Step 6: Commit**

```
git add src/k8s/relationships.rs
git commit -m "feat(relationships): implement RBAC and Flux lineage resolvers"
```

---

### Task 14: Implement async resolve_relationships orchestrator

**Files:**
- Modify: `src/k8s/relationships.rs`

- [ ] **Step 1: Implement the orchestrator function**

```rust
/// Resolve all relationships for a resource, using snapshot data with API fallback.
pub async fn resolve_relationships(
    resource: &ResourceRef,
    snapshot: &crate::state::ClusterSnapshot,
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
```

Note: v1 uses snapshot-only resolution. API fallback can be added later without changing the interface since `_client` is already in the signature.

- [ ] **Step 2: Run tests**

Run: `cargo test --all-targets --all-features`
Expected: PASS

- [ ] **Step 3: Commit**

```
git add src/k8s/relationships.rs
git commit -m "feat(relationships): add async resolve_relationships orchestrator"
```

---

## Chunk 4: Rendering + Input Handling

### Task 15: Implement relations tab rendering

**Files:**
- Create: `src/ui/views/relations.rs`
- Modify: `src/ui/views/mod.rs`
- Modify: `src/ui/components/workbench.rs`

- [ ] **Step 1: Create the render function**

Create `src/ui/views/relations.rs`:

```rust
//! Rendering for the Relations workbench tab.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::k8s::relationships::{flatten_tree, FlatNode, RelationKind};
use crate::ui::theme::Theme;
use crate::workbench::RelationsTabState;

pub fn render_relations_tab(frame: &mut Frame, area: Rect, tab: &RelationsTabState, theme: &Theme) {
    if tab.loading {
        let text = Paragraph::new("Loading relationships...")
            .style(Style::default().fg(theme.fg_dim));
        frame.render_widget(text, area);
        return;
    }

    if let Some(err) = &tab.error {
        let text = Paragraph::new(format!("Error: {err}"))
            .style(Style::default().fg(theme.error));
        frame.render_widget(text, area);
        return;
    }

    if tab.tree.is_empty() {
        let text = Paragraph::new("No relationships found.")
            .style(Style::default().fg(theme.fg_dim));
        frame.render_widget(text, area);
        return;
    }

    let flat = flatten_tree(&tab.tree, &tab.expanded);
    let visible_height = area.height as usize;

    // Scroll so cursor is visible
    let scroll_offset = if flat.is_empty() {
        0
    } else {
        let cursor = tab.cursor.min(flat.len().saturating_sub(1));
        if cursor < visible_height / 2 {
            0
        } else {
            cursor.saturating_sub(visible_height / 2)
        }
    };

    let mut lines = Vec::new();
    for (i, node) in flat.iter().enumerate().skip(scroll_offset).take(visible_height) {
        let line = render_flat_node(node, i == tab.cursor, theme);
        lines.push(line);
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

fn render_flat_node(node: &FlatNode, is_cursor: bool, theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();

    if node.relation == RelationKind::SectionHeader {
        // Section header: "── Owner Chain ──"
        let header = format!("── {} ", node.label);
        let padding = "─".repeat(60usize.saturating_sub(header.len()));
        spans.push(Span::styled(
            format!("{header}{padding}"),
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        ));
    } else {
        // Indent with tree connectors
        for (depth_idx, &parent_last) in node.parent_is_last.iter().enumerate() {
            if depth_idx == 0 && node.depth == 0 {
                continue;
            }
            if parent_last {
                spans.push(Span::raw("  "));
            } else {
                spans.push(Span::styled("│ ", Style::default().fg(theme.fg_dim)));
            }
        }

        // Connector or expand/collapse marker
        if node.depth > 0 {
            let connector = if node.is_last_child { "└ " } else { "├ " };
            spans.push(Span::styled(connector, Style::default().fg(theme.fg_dim)));
        }

        if node.has_children {
            let marker = if node.expanded { "▼ " } else { "▶ " };
            spans.push(Span::styled(marker, Style::default().fg(theme.fg_dim)));
        } else if node.depth == 0 {
            spans.push(Span::raw("  "));
        }

        // Kind label
        let (kind_part, name_part) = node.label.split_once(' ').unwrap_or(("", &node.label));

        let kind_style = if node.not_found {
            Style::default().fg(theme.fg_dim)
        } else {
            Style::default().fg(theme.accent)
        };
        spans.push(Span::styled(format!("{kind_part} "), kind_style));

        // Resource name
        let name_style = if node.not_found {
            Style::default().fg(theme.fg_dim)
        } else {
            Style::default().fg(theme.fg)
        };
        spans.push(Span::styled(name_part.to_string(), name_style));

        // Namespace (dimmed)
        if let Some(ns) = &node.namespace {
            spans.push(Span::styled(format!(" {ns}"), Style::default().fg(theme.fg_dim)));
        }

        // Status
        if let Some(status) = &node.status {
            let status_color = match status.as_str() {
                "Running" | "Ready" | "Bound" | "Active" => theme.success,
                "Pending" | "Waiting" | "Terminating" => theme.warning,
                "Failed" | "Error" | "CrashLoopBackOff" => theme.error,
                _ => theme.fg_dim,
            };
            spans.push(Span::styled(format!(" {status}"), Style::default().fg(status_color)));
        }

        if node.not_found {
            spans.push(Span::styled(" (not found)", Style::default().fg(theme.fg_dim)));
        }
    }

    let mut line = Line::from(spans);
    if is_cursor {
        // Apply cursor highlight to entire line
        line = line.style(Style::default().bg(theme.selection_bg).fg(theme.selection_fg));
    }
    line
}
```

- [ ] **Step 2: Add mod declaration**

In `src/ui/views/mod.rs`, add:
```rust
pub mod relations;
```

- [ ] **Step 3: Add render dispatch in workbench.rs**

In `src/ui/components/workbench.rs`, in the render match block (around line 87), add:

```rust
WorkbenchTabState::Relations(tab) => {
    crate::ui::views::relations::render_relations_tab(frame, inner, tab, &app.theme())
}
```

Note: check how `theme` is accessed in the existing render functions and follow the same pattern. It may be `app.theme` or passed as a parameter. Also verify the actual field names in `src/ui/theme.rs` — the rendering code references `theme.selection_bg`, `theme.selection_fg`, `theme.accent`, `theme.success`, `theme.warning`, `theme.error`, `theme.fg`, `theme.fg_dim` which need to match the actual Theme struct fields.

- [ ] **Step 4: Run tests and clippy**

Run: `cargo test --all-targets --all-features && cargo clippy --all-targets --all-features -- -D warnings`
Expected: PASS

- [ ] **Step 5: Commit**

```
git add src/ui/views/relations.rs src/ui/views/mod.rs src/ui/components/workbench.rs
git commit -m "feat(ui): implement relations tab rendering with tree connectors"
```

---

### Task 16: Implement relations tab input handling

**Files:**
- Modify: `src/app.rs` — workbench key handling for Relations tab

- [ ] **Step 1: Find the workbench key handling section**

In `src/app.rs`, find where `WorkbenchTabState` variants are matched for key handling (this is in the workbench-focused section of `handle_key_event`). Add a match arm for Relations:

```rust
WorkbenchTabState::Relations(tab) => {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
            if !flat.is_empty() {
                tab.cursor = (tab.cursor + 1).min(flat.len().saturating_sub(1));
            }
            AppAction::None
        }
        KeyCode::Char('k') | KeyCode::Up => {
            tab.cursor = tab.cursor.saturating_sub(1);
            AppAction::None
        }
        KeyCode::Char('g') => {
            tab.cursor = 0;
            AppAction::None
        }
        KeyCode::Char('G') => {
            let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
            tab.cursor = flat.len().saturating_sub(1);
            AppAction::None
        }
        KeyCode::Char('l') | KeyCode::Right => {
            // Expand node under cursor
            let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
            if let Some(node) = flat.get(tab.cursor) {
                if node.has_children && !node.expanded {
                    tab.expanded.insert(node.tree_index);
                }
            }
            AppAction::None
        }
        KeyCode::Char('h') | KeyCode::Left => {
            // Collapse node under cursor, or move to parent
            let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
            if let Some(node) = flat.get(tab.cursor) {
                if node.expanded {
                    tab.expanded.remove(&node.tree_index);
                } else if tab.cursor > 0 {
                    // Move to parent: find previous node at lower depth
                    for i in (0..tab.cursor).rev() {
                        if flat[i].depth < node.depth {
                            tab.cursor = i;
                            break;
                        }
                    }
                }
            }
            AppAction::None
        }
        KeyCode::Enter => {
            let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
            if let Some(node) = flat.get(tab.cursor) {
                if let Some(resource) = &node.resource {
                    if !node.not_found && node.relation != crate::k8s::relationships::RelationKind::SectionHeader {
                        return AppAction::OpenDetail(resource.clone());
                    }
                }
            }
            AppAction::None
        }
        _ => AppAction::None,
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test --all-targets --all-features`
Expected: PASS

- [ ] **Step 3: Commit**

```
git add src/app.rs
git commit -m "feat(app): add relations tab keyboard navigation (j/k/h/l/Enter)"
```

---

## Chunk 5: Quality Gate

### Task 17: Final quality gate and cleanup

**Files:** All modified files

- [ ] **Step 1: Run formatter**

Run: `cargo fmt --all`

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Fix any warnings.

- [ ] **Step 3: Run all tests**

Run: `cargo test --all-targets --all-features`
Expected: All tests pass.

- [ ] **Step 4: Verify test count increased**

Check that test count is higher than 480 (previous count after M9).

- [ ] **Step 5: Commit any cleanup**

```
git add -A
git commit -m "chore: M10 quality gate cleanup"
```

---

### Task 18: Update plan.md and help overlay verification

**Files:**
- Modify: `plan.md`
- Modify: `src/ui/components/help_overlay.rs` (verify)

- [ ] **Step 1: Update plan.md milestone 10 status**

Change M10 status from "Planned" to "Completed". Add "What shipped" section listing all features.

- [ ] **Step 2: Update implementation progress section**

Add `- Milestone 10: completed` to the status list.

- [ ] **Step 3: Update test count**

Update the verification status line with the new test count.

- [ ] **Step 4: Commit**

```
git add plan.md
git commit -m "docs: mark M10 Relationship Explorer complete"
```
