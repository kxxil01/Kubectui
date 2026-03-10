# M11: Node Operations — Design Spec

## Goal

Add cordon, uncordon, and drain operations for Kubernetes nodes, enabling cluster operators to manage node lifecycle directly from KubecTUI.

## Actions

### Cordon (`c` key)

- No confirmation dialog (reversible, non-destructive)
- Patches node `spec.unschedulable = true`
- Immediate execution on keypress
- Recorded in action history

### Uncordon (`u` key)

- No confirmation dialog (reversible, non-destructive)
- Patches node `spec.unschedulable = false`
- Immediate execution on keypress
- Recorded in action history

### Drain (`D` key)

- Confirmation dialog (same pattern as delete)
- Dialog text: "Drain Node 'name'? This will evict all pods from this node."
- Footer: `[D]/[y]/[Enter]` Drain | `[F]` Force drain | `[Esc]` Cancel
- Force drain: ignores pod disruption budgets
- Evicts all pods except DaemonSet-managed and mirror pods
- Default timeout: 300s, grace period: 30s
- Recorded in action history

## Policy

- `DetailAction::Cordon`, `DetailAction::Uncordon`, `DetailAction::Drain` added to enum
- Only `ResourceRef::Node(_)` supports these three actions
- Action palette entries: "Cordon Node", "Uncordon Node", "Drain Node"

## K8s Client Methods

### `cordon_node(name: &str)`

JSON merge patch: `{"spec": {"unschedulable": true}}`

### `uncordon_node(name: &str)`

JSON merge patch: `{"spec": {"unschedulable": false}}`

### `drain_node(name: &str, timeout_secs: u64, grace_period_secs: u64, force: bool)`

1. List pods on the node via field selector `spec.nodeName={name}`
2. Filter out: DaemonSet-owned pods, mirror pods (annotation `kubernetes.io/config.mirror`)
3. Evict each pod via Eviction API (`policy/v1`)
4. If `force`: delete pods that fail eviction (PDB violation)
5. Wait for pods to terminate, with timeout

## NodeInfo DTO Update

Add `unschedulable: bool` field to `NodeInfo`. Extracted from `node.spec.unschedulable`.

## Node List Visual

Status column updated:
- `● Ready` (green) — schedulable and ready
- `● Ready SchedulingDisabled` (yellow warning color) — cordoned
- `✗ NotReady` (red) — not ready
- `✗ NotReady SchedulingDisabled` (red) — not ready and cordoned

## Async Channel

One new channel: `node_ops_tx` / `node_ops_rx` (capacity 16).

Result struct: `NodeOpsAsyncResult { request_id, action_history_id, context_generation, origin_view, resource, op_kind, result }`

Where `op_kind` distinguishes Cordon/Uncordon/Drain for result messaging.

## App State Changes

- `AppAction::CordonNode`, `AppAction::UncordonNode`, `AppAction::DrainNode`, `AppAction::ForceDrainNode`
- `DetailViewState.confirm_drain: bool` — drain confirmation modal state
- `ActionKind::Cordon`, `ActionKind::Uncordon`, `ActionKind::Drain`

## Optimistic Update

After cordon/uncordon succeeds:
- Update the node's `unschedulable` field in the cached snapshot
- Trigger origin view refresh for immediate visual feedback

## Help & Discoverability

- Help overlay: "Node Actions" section with `c`, `u`, `D`
- Action palette: "Cordon Node", "Uncordon Node", "Drain Node" (Node resources only)

## Files Modified

| File | Change |
|------|--------|
| `src/policy.rs` | Add Cordon/Uncordon/Drain to DetailAction, Node capability |
| `src/action_history.rs` | Add ActionKind variants |
| `src/k8s/dtos.rs` | Add `unschedulable` to NodeInfo |
| `src/k8s/client.rs` | Add cordon/uncordon/drain methods |
| `src/app.rs` | Add AppAction variants, key handling, confirm_drain state |
| `src/main.rs` | Add channel, dispatch handlers, result handler |
| `src/ui/views/detail.rs` | Render drain confirmation dialog |
| `src/ui/views/nodes.rs` | Show SchedulingDisabled in status column |
| `src/ui/components/help_overlay.rs` | Add Node Actions section |
| `src/ui/components/command_palette.rs` | Add palette entries |

## Testing

- Policy: Node supports Cordon/Uncordon/Drain, other resources do not
- Action history: new ActionKind variants serialize/display correctly
- Key handling: `c`/`u`/`D` dispatch correct AppActions from Node detail view
- Drain confirmation: `D`/`y`/Enter confirm, `F` force, `Esc` cancel
- Node list rendering: SchedulingDisabled indicator for cordoned nodes
- Palette: entries visible only for Node resources
