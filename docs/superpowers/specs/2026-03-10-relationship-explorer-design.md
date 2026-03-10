# Milestone 10: Relationship Explorer — Design Spec

## Goal

Let users navigate the cluster by dependency and ownership chains, not just by resource kind. A new workbench tab renders an indented, expand/collapse tree of related resources for any supported resource.

## Decisions

- **Surface**: Workbench tab (follows YAML/Events/Logs pattern)
- **Tree layout**: Indented tree with expand/collapse (file-explorer style)
- **Navigation**: Browse tree + Enter jumps to resource detail
- **Scope**: All 6 relationship chains (owner, service, ingress, storage, Flux, RBAC — RBAC is an addition beyond plan.md's 5 chains, leveraging the existing policy layer)
- **Data source**: Snapshot-first, API fallback for missing resources
- **Entry point**: `w` key in detail view + "Relations" in action palette

## Data Model

### RelationNode

```rust
// src/k8s/relationships.rs (new file)

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationNode {
    pub resource: Option<ResourceRef>,  // None for section headers and "(not found)" placeholders
    pub label: String,                  // "Deployment nginx-deployment"
    pub status: Option<String>,         // "Ready", "Running", "3/3"
    pub namespace: Option<String>,
    pub relation: RelationKind,
    pub children: Vec<RelationNode>,
    pub not_found: bool,               // true for unresolvable references (rendered dimmed)
}

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
```

The tree is resolved once asynchronously and stored as `Vec<RelationNode>` (multiple roots, one per chain type). The UI flattens it into visible lines based on expand/collapse state.

### FlatNode (for rendering)

Tree sizes are bounded (typically <100 nodes for any real cluster resource), so cloning is acceptable for simplicity.

```rust
pub struct FlatNode {
    pub depth: usize,
    pub tree_index: usize,       // stable index for expand/collapse tracking
    pub node: RelationNode,      // cloned — tree sizes are small enough
    pub is_last_child: bool,     // for └ vs ├ rendering
    pub parent_is_last: Vec<bool>, // for │ vs space continuation lines
    pub has_children: bool,
    pub expanded: bool,
}
```

### Tree indexing scheme

Each `RelationNode` gets a unique `tree_index` assigned during tree construction via depth-first pre-order traversal. This index is stable across expand/collapse operations and is used as the key in the `expanded: HashSet<usize>` set. The flattening function walks the tree, skipping children of collapsed nodes, and emits `FlatNode` entries for visible nodes only.

## Workbench Tab State

```rust
// in workbench.rs

pub struct RelationsTabState {
    pub resource: ResourceRef,
    pub tree: Vec<RelationNode>,
    pub cursor: usize,              // index into flattened visible lines
    pub expanded: HashSet<usize>,   // tree node indices that are expanded
    pub loading: bool,
    pub error: Option<String>,
}
```

Infrastructure additions:
- `WorkbenchTabKind::Relations` — title: `"Relations"`
- `WorkbenchTabKey::Relations(ResourceRef)` — one tab per resource, reuses on re-open
- `WorkbenchTabState::Relations(RelationsTabState)` variant
- Tab title format: `"Relations default/nginx-deployment"`
- Closed by `close_resource_tabs()` on context/namespace switch

## Policy Integration

### New DetailAction

Add `DetailAction::ViewRelationships` as the 13th action:
- Key hint: `[w]`
- Label: `"Relations"`
- Position: after `Trigger` in `DetailAction::ORDER`

### supports_detail_action

`ResourceRef::supports_detail_action(ViewRelationships)` returns true when the resource type has non-empty `relationship_capabilities()`. This covers:
- Workloads: Deployment, StatefulSet, DaemonSet, ReplicaSet, ReplicationController, Job, CronJob, Pod
- Network: Service, Endpoints, Ingress, IngressClass
- Storage: PVC, PV, StorageClass
- FluxCD: all Flux views (CustomResource with matching group/kind)
- RBAC: ServiceAccount, ClusterRole, Role, ClusterRoleBinding, RoleBinding

Implementation: add a `ViewRelationships` match arm in `supports_detail_action` that calls a helper `resource_has_relationships(resource) -> bool`. This helper maps ResourceRef to AppView via `resource_to_view()` and checks `!view.relationship_capabilities().is_empty()`.

### DetailViewState::supports_action update

Add `ViewRelationships` to the `requires_clear_surface` match in `DetailViewState::supports_action()`. This ensures the Relations action is blocked during loading, scale dialog, probe panel, and delete confirmation — consistent with all other detail actions.

### Action Palette

Add "Relations" entry with aliases: `["relations", "relationships", "related", "web", "tree", "deps"]`

## Resolver Architecture

Six resolver functions in `src/k8s/relationships.rs`, all taking `(&ResourceRef, &crate::state::ClusterSnapshot, &Client)` and returning `Vec<RelationNode>`:

Note: `ClusterSnapshot` here refers to `crate::state::ClusterSnapshot` (the real runtime snapshot in `src/state/mod.rs`), NOT the legacy test DTO `crate::k8s::dtos::ClusterSnapshot`.

### resolve_owner_chain

- **Input**: Any workload resource (Pod, ReplicaSet, Job, etc.)
- **Strategy**: Walk `owner_references` upward from snapshot DTOs. For each owner, find the matching resource in the snapshot by kind+name+namespace. If not found, API fallback via `get()`.
- **Downward**: Also find resources in snapshot whose `owner_references` point to the root resource.
- **Requires**: Enhancing `OwnerRefInfo` to include `uid` field for accurate matching. Add `owner_references` to `ReplicaSetInfo`, `JobInfo`, and other workload DTOs that currently lack it.

### resolve_service_backends

- **Input**: Service or Endpoints
- **Strategy**: For Service — fetch service via API (need selector, which isn't in `ServiceInfo` DTO), match pods from snapshot by label selector. Also find corresponding Endpoints object in snapshot.
- **For Endpoints**: Find the parent Service by name, then resolve pods.
- **Requires**: Either adding `selector` to `ServiceInfo` DTO or fetching the Service object via API.

### resolve_ingress_backends

- **Input**: Ingress
- **Strategy**: Fetch Ingress via API, parse `spec.rules[].http.paths[].backend.service.name` → find matching Services in snapshot.
- **For IngressClass**: Find all Ingresses using this class.

### resolve_storage_bindings

- **Input**: PVC, PV, or StorageClass
- **Strategy**: PVC → follow `volume` field to PV in snapshot → follow `storage_class` field to StorageClass. Reverse: StorageClass → find PVs with matching class → find PVCs bound to those PVs.
- **Data available**: `PvcInfo.volume`, `PvcInfo.storage_class`, `PvInfo.claim`, `PvInfo.storage_class` — all already in DTOs.

### resolve_flux_lineage

- **Input**: Any Flux resource
- **Strategy**: Fetch Flux resource via API, parse `spec.sourceRef` for sources. Find downstream Flux resources that reference this source. Uses existing `FluxResourceInfo` data where available.

### resolve_rbac_bindings

- **Input**: ServiceAccount, Role, ClusterRole, RoleBinding, ClusterRoleBinding
- **Strategy**: For ServiceAccount — scan `RoleBindingInfo` and `ClusterRoleBindingInfo` subjects in snapshot for matching SA name/namespace → follow `role_ref_name`/`role_ref_kind` to find the Role/ClusterRole.
- **For Role/ClusterRole**: Find bindings that reference this role.
- **Data available**: `RoleBindingInfo.subjects`, `RoleBindingInfo.role_ref_name`, etc. — all in DTOs.

### Resolution orchestration

```rust
pub async fn resolve_relationships(
    resource: &ResourceRef,
    snapshot: &ClusterSnapshot,
    client: &Client,
) -> Result<Vec<RelationNode>> {
    // Determine which capabilities apply
    let view = resource_to_view(resource);
    let capabilities = view.relationship_capabilities();

    let mut sections = Vec::new();
    for cap in capabilities {
        let nodes = match cap {
            OwnerChain => resolve_owner_chain(resource, snapshot, client).await?,
            ServiceBackends => resolve_service_backends(resource, snapshot, client).await?,
            IngressBackends => resolve_ingress_backends(resource, snapshot, client).await?,
            StorageBindings => resolve_storage_bindings(resource, snapshot, client).await?,
            FluxLineage => resolve_flux_lineage(resource, snapshot, client).await?,
            RbacBindings => resolve_rbac_bindings(resource, snapshot, client).await?,
        };
        if !nodes.is_empty() {
            sections.push(RelationNode {
                resource: None,
                label: cap.section_title().to_string(),
                status: None,
                namespace: None,
                relation: RelationKind::SectionHeader,
                children: nodes,
            });
        }
    }
    Ok(sections)
}
```

### DTO enhancements needed

1. `OwnerRefInfo`: add `uid: String` field
2. `ReplicaSetInfo`: add `owner_references: Vec<OwnerRefInfo>`
3. `JobInfo`: add `owner_references: Vec<OwnerRefInfo>`
4. `ServiceInfo`: add `selector: BTreeMap<String, String>` (for pod matching without API call)
5. `IngressInfo`: add `backend_services: Vec<(String, String)>` — list of `(service_name, port)` pairs parsed from rules

These are additive changes to existing DTOs — no breaking changes.

### resource_to_view helper

Maps `ResourceRef` variant to `AppView` for capability lookup:
```rust
fn resource_to_view(resource: &ResourceRef) -> Option<AppView> {
    match resource {
        ResourceRef::Pod(..) => Some(AppView::Pods),
        ResourceRef::Deployment(..) => Some(AppView::Deployments),
        ResourceRef::Service(..) => Some(AppView::Services),
        ResourceRef::StatefulSet(..) => Some(AppView::StatefulSets),
        ResourceRef::DaemonSet(..) => Some(AppView::DaemonSets),
        ResourceRef::ReplicaSet(..) => Some(AppView::ReplicaSets),
        ResourceRef::ReplicationController(..) => Some(AppView::ReplicationControllers),
        ResourceRef::Job(..) => Some(AppView::Jobs),
        ResourceRef::CronJob(..) => Some(AppView::CronJobs),
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
        // Flux CRs: map by group to the appropriate FluxCD* view
        ResourceRef::CustomResource { group, .. }
            if group.ends_with(".fluxcd.io") => Some(AppView::FluxCDAll),
        // All other resources (Node, ConfigMap, Secret, Namespace, etc.): no relationships
        _ => None,
    }
}
```

Returns `Option<AppView>` — `None` means the resource has no relationship support. Flux CustomResources are mapped to `FluxCDAll` which has `RELATIONSHIPS_FLUX` capability.

## Tree Rendering

### Visual layout

```
── Owner Chain ──────────────────────────────
▼ Deployment nginx-deployment        default   Ready
  ▼ ReplicaSet nginx-deployment-7fb9 default   3/3
    ├ Pod nginx-7fb9-abc12                     Running
    ├ Pod nginx-7fb9-def34                     Running
    └ Pod nginx-7fb9-ghi56                     Running
  ▶ ReplicaSet nginx-deployment-5d4b default   0/0
── Service Backends ─────────────────────────
▼ Service nginx-svc                  default   ClusterIP
  ├ Pod nginx-7fb9-abc12                       Running
  ├ Pod nginx-7fb9-def34                       Running
  └ Pod nginx-7fb9-ghi56                       Running
```

### Rendering rules

- Tree connectors: `├` for non-last children, `└` for last child, `│` for continuation lines
- Expand/collapse markers: `▼` expanded, `▶` collapsed (only for nodes with children)
- 2-space indent per depth level
- Resource kind: accent color
- Resource name: foreground
- Namespace: dimmed (only shown when different from root resource's namespace)
- Status: green (healthy), yellow (degraded), red (error)
- Root resource: bold
- Section headers: styled like existing `── Actions ──` in the action palette
- Cursor: highlighted row (reverse video or similar)

### Interaction keys

When workbench is focused on a Relations tab:
- `j`/`k` or Up/Down: move cursor through visible (expanded) lines
- `l`/Right: expand node under cursor
- `h`/Left: collapse node under cursor (or move cursor to parent if leaf)
- `Enter`: jump to resource — opens detail view for that resource. No-op on section headers and "(not found)" placeholder nodes.
- `g`: scroll to top
- `G`: scroll to bottom
- `Esc`: return focus from workbench to previous focus area (existing workbench behavior, not Relations-specific)

## Async Data Flow

1. User presses `w` in detail view → `AppAction::OpenRelationships`
2. `main.rs` handler:
   - Creates `RelationsTabState { resource, loading: true, tree: vec![], .. }`
   - Opens workbench tab via `workbench.open_tab()`
   - Sets `focus = Focus::Workbench`
   - Spawns async task: `resolve_relationships(resource, snapshot, client)`
3. Result delivered back to main loop via existing channel pattern
4. Tab state updated: `loading = false`, `tree = result` (or `error = Some(msg)`)

Single async resolve, single result delivery. No streaming.

## Error Handling

- Individual chain resolver failures are captured per-section — other sections still display
- If all resolvers fail, show error in tab: "Failed to resolve relationships: {message}"
- Missing/broken references produce empty children, not errors (e.g., owner reference pointing to deleted resource shows "ReplicaSet foo (not found)" in dimmed style)
- API fallback timeout: 5 seconds per resolver, then skip with partial results

## Testing Strategy

### Unit tests in `relationships.rs`

- Owner chain resolution from pods with known ownerReferences in mock snapshot
- Downward ownership: deployment → find replicasets that reference it
- Service selector matching against pod labels
- Storage binding chain PVC → PV → StorageClass from snapshot fields
- RBAC: ServiceAccount → RoleBinding → Role chain
- Missing/broken references produce "(not found)" nodes, not panics
- Empty snapshot returns empty tree

### Unit tests in `workbench.rs`

- `RelationsTabState` creation and field defaults
- Tab key deduplication (same resource reopens, doesn't create second tab)
- `close_resource_tabs()` includes Relations tabs

### Unit tests for tree flattening

- Expand/collapse state produces correct visible lines
- Cursor bounds clamping after collapse
- Tree connector generation (├/└/│) with nested depths

### Policy tests

- `ViewRelationships` available for all 6 capability groups
- `ViewRelationships` unavailable for Dashboard, ConfigMaps, Secrets, etc.
- `DetailViewState::supports_action(ViewRelationships)` respects loading/overlay guards

## Files Changed

### New files
- `src/k8s/relationships.rs` — RelationNode, RelationKind, all 6 resolvers, orchestrator, tests

### Modified files
- `src/k8s/mod.rs` — add `pub mod relationships;`
- `src/k8s/dtos.rs` — enhance OwnerRefInfo (add uid), add owner_references to ReplicaSetInfo/JobInfo, add selector to ServiceInfo, add backend_services to IngressInfo
- `src/k8s/client.rs` — populate new DTO fields during resource fetching
- `src/workbench.rs` — RelationsTabState, WorkbenchTabKind/Key/State variants
- `src/policy.rs` — DetailAction::ViewRelationships, supports_detail_action, ORDER update
- `src/app.rs` — AppAction::OpenRelationships, `w` keybinding in detail view
- `src/events/input.rs` — handle AppAction::OpenRelationships (close palette, return true)
- `src/main.rs` — spawn relationship resolver, handle result, open tab
- `src/ui/components/command_palette.rs` — add Relations action alias
- `src/ui/views/workbench.rs` (or new file) — render Relations tab content
- `src/ui/components/help_overlay.rs` — add `w` keybinding entry

## Not In Scope

- Bidirectional tree (showing "selected by" upward + "owns" downward from a single root) — v1 shows the natural chain direction per capability
- Real-time tree updates (tree is resolved once on open, not live-refreshed)
- Custom resource relationships (only built-in K8s + Flux + RBAC)
- Visual graph canvas
