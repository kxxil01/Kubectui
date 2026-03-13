# Phase 8: Watch-Backed Core Resource Caches

## Goal

Reduce steady-state polling cost and improve time-to-freshness for the highest-value views by replacing repeated full-list polling with watch-backed caches for core resources, while preserving the current `ClusterSnapshot` contract and the current truthful load-state behavior.

This phase starts only after the scope-driven polling model is stable. It must not reintroduce duplicate data paths, stale cross-context updates, or UI state that lies about resource readiness.

## Non-Goals

- Do not replace every poll with a watch.
- Do not move metrics to watches first.
- Do not add per-view caches or alternate render data models.
- Do not thread watch-specific state through UI render code.
- Do not change Helm, Flux, RBAC, CRDs, or local Helm repository loading in the first rollout.
- Do not weaken the existing polling path until the watch path is proven.

## Target Outcomes

- Lower `api_calls_per_refresh` for watched views versus current Phase 0 baselines.
- Faster propagation of live cluster changes for watched resources.
- No regressions in:
  - context switching
  - namespace switching
  - load-state truthfulness
  - selection consistency
  - issue detection
  - connection health reporting

## Canonical Architecture

Phase 8 must extend the existing state pipeline, not bypass it.

The single source of truth remains:

- [src/state/mod.rs](/Users/ilham/Developer/tools/Kubectui/src/state/mod.rs)

The watch path must feed the same `ClusterSnapshot` model already used by polling.

Recommended new module layout:

- [src/state/watch.rs](/Users/ilham/Developer/tools/Kubectui/src/state/watch.rs)
- [src/state/mod.rs](/Users/ilham/Developer/tools/Kubectui/src/state/mod.rs)
- [src/k8s/client.rs](/Users/ilham/Developer/tools/Kubectui/src/k8s/client.rs)

Recommended responsibilities:

- `src/state/watch.rs`
  - watch session lifecycle
  - reconnect and relist policy
  - per-resource in-memory stores
  - generation guards for context and namespace
  - event application and snapshot publication triggers
- `src/state/mod.rs`
  - owns `ClusterSnapshot`
  - merges watch-backed resource stores into published snapshots
  - preserves load-state and scope semantics
  - preserves optimistic mutation behavior
- `src/k8s/client.rs`
  - direct Kubernetes watch/list integration
  - no app-level orchestration

## Initial Watch Scope

Ship watches only for these resources first:

1. Pods
2. Deployments
3. ReplicaSets
4. StatefulSets
5. DaemonSets
6. Services
7. Nodes

Reasoning:

- These dominate operator attention and steady-state refresh traffic.
- They power the most frequently visited views.
- They affect Dashboard and Issues enough to produce real UX wins.

Keep these polled in Phase 8 initial rollout:

- metrics
- Helm releases
- Flux resources
- RBAC and security resources
- CRDs and extension instances
- events
- storage
- governance/config resources

## Data Model Design

Introduce one watch manager with typed stores, not per-view caches.

Suggested top-level types:

```rust
pub struct WatchManager {
    session: WatchSessionKey,
    stores: WatchedResourceStores,
    status: WatchStatusMap,
}

pub struct WatchSessionKey {
    pub context_generation: u64,
    pub cluster_context: Option<String>,
    pub namespace: Option<String>,
}

pub struct WatchedResourceStores {
    pub pods: ResourceStore<PodInfo>,
    pub deployments: ResourceStore<DeploymentInfo>,
    pub replicasets: ResourceStore<ReplicaSetInfo>,
    pub statefulsets: ResourceStore<StatefulSetInfo>,
    pub daemonsets: ResourceStore<DaemonSetInfo>,
    pub services: ResourceStore<ServiceInfo>,
    pub nodes: ResourceStore<NodeInfo>,
}

pub struct ResourceStore<T> {
    pub items: Vec<T>,
    pub revision: u64,
    pub readiness: StoreReadiness,
    pub last_error: Option<String>,
}

pub enum StoreReadiness {
    Idle,
    Listing,
    Watching,
    Resyncing,
    Error,
}
```

Key rules:

- `ResourceStore<T>` remains typed and direct.
- `Vec<T>` remains the publish format for snapshot compatibility.
- No trait-object wrapper around stores.
- No generic adapter layer around UI views.

## Snapshot Integration Rules

The published snapshot must still look like one coherent cluster state.

Watch-backed resources should update snapshot fields directly:

- `pods`
- `deployments`
- `replicasets`
- `statefulsets`
- `daemonsets`
- `services`
- `nodes`

These derived snapshot fields must be recomputed on watch updates:

- `services_count`
- `namespaces_count`
- `issue_count`
- dashboard summaries affected by watched resources
- view load states for watched views

These fields must not be recomputed from watch path unless their source also changed:

- `pod_metrics`
- `node_metrics`
- `helm_releases`
- `flux_resources`
- `cluster_info` except where it is derived from watched nodes/pods and cached version data

## Refresh Strategy After Watches

Do not delete polling. Narrow it.

The new model should be:

- watched resources:
  - list once on startup or session reset
  - watch for live updates
  - periodic resync only for drift protection
- polled resources:
  - keep current scope-driven polling

Updated semantics:

- `RefreshScope` remains the user-facing and scheduler-facing contract.
- For a watched scope, manual refresh should trigger relist/resync, not a full traditional poll.
- For a non-watched scope, manual refresh continues to poll.
- Mixed-scope refresh remains legal:
  - watched buckets resync
  - non-watched buckets poll

## Context And Namespace Invariants

This is the most important correctness boundary.

Every watch event must be rejected unless it matches the current session key:

- `context_generation`
- cluster context identity
- namespace scope

Required behavior:

1. On context switch:
   - cancel all existing watch tasks
   - discard their pending events
   - create a fresh watch session key
   - relist watched resources for the new context
2. On namespace switch:
   - cancel namespace-scoped watches
   - retain cluster-scoped node watches only if context is unchanged
   - relist namespace-scoped watched resources
3. On stale event arrival:
   - drop silently
   - do not mutate snapshot
   - do not mutate load state

## Kubernetes Watch Implementation

Use direct Kubernetes watch support in [src/k8s/client.rs](/Users/ilham/Developer/tools/Kubectui/src/k8s/client.rs).

Recommended client API shape:

```rust
pub async fn list_pods(&self, namespace: Option<&str>) -> Result<Vec<PodInfo>>;
pub async fn watch_pods(
    &self,
    namespace: Option<&str>,
    resource_version: String,
) -> Result<impl Stream<Item = Result<WatchEvent<PodInfo>>>>;
```

Apply the same pattern for:

- deployments
- replicasets
- statefulsets
- daemonsets
- services
- nodes

Implementation rules:

- keep mapping from Kubernetes API objects to `*Info` DTOs in one place
- preserve current namespace scoping
- preserve current error formatting quality
- prefer server-provided resource versions for resumable watches

## Event Processing Model

Recommended processing pipeline per watched resource:

1. Initial list
2. Build typed `Vec<T>`
3. Capture latest `resource_version`
4. Publish store as ready
5. Start watch stream from that version
6. Apply `Added`, `Modified`, `Deleted`
7. Update store revision
8. Notify state layer to publish a new snapshot

Recommended mutation strategy:

- convert store into a map only inside watch application logic if needed
- publish sorted `Vec<T>` back into snapshot
- keep sorting and identity rules consistent with current polling output

Do not publish raw unordered map iteration output to the UI.

## Failure Handling

Required failure classes:

1. Initial list fails
2. Watch stream drops
3. Resource version expires
4. Authorization denied
5. Namespace disappears
6. Client disconnect or timeout

Required behavior:

- initial list failure:
  - keep previous snapshot data if available
  - mark watched bucket degraded
  - preserve current connection-health semantics
- watch stream drops:
  - enter reconnect backoff
  - do not clear existing data
- resource version expired:
  - perform fresh relist
  - restart watch
- authorization denied:
  - surface actionable error
  - stop retry loop until next manual refresh or context change

Recommended retry policy:

- bounded exponential backoff with jitter
- immediate relist on resource version expiration
- hard reset on context or namespace change

## Manual Refresh Semantics

Manual refresh must stay predictable.

For watched scopes:

- `Pods` refresh:
  - relist pods
  - restart pod watch if needed
  - optionally also refresh metrics if current profile requires it
- `Dashboard` refresh:
  - relist watched core-overview resources
  - poll metrics
- `Issues` refresh:
  - relist watched core-overview resources
  - poll remaining diagnostic buckets

For non-watched scopes:

- behavior remains current scope-driven polling

## Auto-Refresh Semantics

Once core watches are enabled, auto-refresh should be narrowed.

Recommended model:

- watched scopes:
  - no periodic full relist by default
  - only periodic low-frequency resync for drift protection
- non-watched scopes:
  - keep existing periodic polling
- dashboard metrics:
  - keep periodic polling

This avoids rebuilding full snapshots from repeated core list calls.

## UI And UX Rules

No watch-specific UI concepts should leak into most views.

The UI should continue to show:

- `Loading`
- `Refreshing`
- `Ready`
- unknown counts when data is not yet loaded

Possible small additions:

- optional subtle status in Dashboard or status bar when live watch sync is reconnecting

Do not:

- add per-view watch badges everywhere
- expose raw Kubernetes watch errors in table titles
- show fake zeroes during watch reconnect

## Interaction With Optimistic Mutations

Current optimistic flows must still work:

- optimistic delete
- optimistic scale
- restart-triggered refresh
- cordon/uncordon/drain follow-up refreshes

Rules:

- optimistic update applies immediately to snapshot as today
- subsequent watch event must reconcile to actual cluster state
- duplicate event must not double-apply
- delete events must remove items cleanly even if optimistic removal already happened

## Performance Validation

Use existing Phase 0 baselines as the before-state.

Add new ignored performance test coverage for watch mode:

- time from resource mutation to visible snapshot update
- relist cost after reconnect
- steady-state API-call count under idle cluster
- memory growth under long-running watch session

Required comparisons:

1. Current polling baseline
2. Watch mode steady-state
3. Watch reconnect/resync path

Metrics to report:

- `api_calls_per_refresh`
- `time_to_primary_ready`
- `time_to_background_ready`
- `time_to_live_update`
- reconnect recovery time

## Test Plan

### Unit tests

Add focused unit tests for:

- session-key stale event rejection
- watch event add/modify/delete application
- resync on resource version expiration
- namespace switch cancellation behavior
- load-state transitions for watched scopes
- optimistic mutation followed by watch reconciliation

### State tests

Extend [src/state/mod.rs](/Users/ilham/Developer/tools/Kubectui/src/state/mod.rs) tests for:

- watched scope manual refresh triggers relist instead of broad poll
- watched and non-watched mixed refresh remains correct
- connection health ignores healthy retained watch state when the relist actually fails

### Integration tests

Add integration-style tests around:

- live update reflected in `Pods`
- service add/remove reflected in `Services`
- deployment replica change reflected in `Deployments`
- context switch under in-flight watch events

### Failure-path tests

Must cover:

- initial watch bootstrap failure
- watch drop and reconnect
- forbidden watch
- resource version expired
- namespace deletion

## Rollout Plan

### Step 1: Scaffolding

- add `src/state/watch.rs`
- add `WatchSessionKey`, `ResourceStore<T>`, `WatchManager`
- add feature flag or runtime gate for watch-backed mode

### Step 2: Pod watch

- implement list + watch for pods
- wire `Pods` view to watched store
- keep metrics polling unchanged
- validate optimistic pod delete interactions

### Step 3: Deployment family

- add deployments
- add replicasets
- verify owner-chain and issue-center behavior remain correct

### Step 4: Stateful and daemon workloads

- add statefulsets
- add daemonsets
- ensure workload-ready aggregates remain correct

### Step 5: Services

- add services
- verify network diagnostic interactions do not regress when endpoints remain polled

### Step 6: Nodes

- add nodes
- keep node metrics polled
- verify dashboard and issue-center mixed watch+poll composition

### Step 7: Auto-refresh narrowing

- reduce broad poll frequency for watched scopes
- keep periodic resync
- compare idle API-call baseline before and after

### Step 8: Default-on decision

Enable by default only if all are true:

- full test suite green
- profiling shows real API-call reduction
- no stale-context leaks
- reconnect behavior is stable
- no material render regressions

## Acceptance Criteria

Phase 8 is done only when:

- watched resources update live without manual refresh
- watched resources no longer pay repeated steady-state full-list polling
- mixed watch+poll refresh semantics remain deterministic
- UI load states remain truthful
- context and namespace changes are safe
- performance baselines show improvement or neutral behavior with lower API load
- no duplicate implementations were introduced

## Exit Criteria For Future Expansion

Only after the initial watched core set is stable should you evaluate expanding watch coverage to:

- namespaces
- endpoints
- ingresses

Do not expand to metrics, Helm, Flux, or RBAC until the first watched core set has been stable in practice.
