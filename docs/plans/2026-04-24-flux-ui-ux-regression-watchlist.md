# Flux UI/UX Regression Watchlist

Date: 2026-04-24

Scope: Flux table behavior under watch-driven updates, apply/reconcile refreshes, delete updates, filtering, sorting, and split-pane/detail UX.

Security note: do not copy credentials from local notes, IDE selections, terminal output, kube configs, or secret manifests into docs, tests, fixtures, logs, screenshots, or PR bodies.

## Current Fix Landed

- Flux table selection now preserves selected resource identity across watch or refresh reorders when possible.
- Canonical helper: `preserve_flux_selection_identity_after_snapshot_change`.
- Event-loop hooks:
  - Flux refresh completion.
  - Flux watch update application.
- Regression covered:
  - Selected Kustomization moves from row 1 to row 0 after reorder.
  - Row above selected Kustomization disappears after delete-like update.

## Regression Areas To Watch

### 1. Selection vs Detail Pane

Risk:
- List selection may preserve identity while open detail pane still points at old resource.
- If selected resource disappears, detail pane may stay open on stale data.

Check:
- Open Flux detail.
- Trigger watch reorder.
- Trigger delete of row above selected.
- Trigger delete of selected resource.
- Confirm list highlight, detail title, detail metadata, and footer actions agree.

Expected:
- Reorder keeps same selected resource and detail stays aligned.
- Deleted selected resource closes detail or moves to explicit nearest fallback with detail updated.

### 2. Search/Filter Active During Reorder

Risk:
- Selected resource can leave filtered result set.
- Current fallback clamps raw row index.
- UX may feel like jump without context.

Check:
- Apply query matching selected resource.
- Watch update changes name/status/message so selected no longer matches query.
- Delete row above selected while filter active.
- Reorder while filter active.

Expected:
- If selected still matches, identity preserved.
- If selected no longer matches, selection moves predictably.
- Consider status/toast if selected resource left filtered results.

### 3. Sort Active During Reorder

Risk:
- Age/name sort plus watch refresh can reorder rows often.
- Selection must follow identity, not raw row index.

Check:
- Enable name sort ascending and descending.
- Enable age sort ascending and descending.
- Trigger apply/reconcile update that changes observed fields.
- Trigger watch reorder.

Expected:
- Selected identity preserved whenever same resource remains visible.
- Viewport keeps selected row visible.

### 4. Delete Semantics

Risk:
- Deleting selected resource currently falls back to clamped index.
- Better UX may choose nearest neighbor by previous position.

Check:
- Delete selected first row.
- Delete selected middle row.
- Delete selected last row.
- Delete row above selected.
- Delete row below selected.

Expected:
- Delete row above selected keeps same selected identity.
- Delete selected resource chooses nearest stable neighbor.
- User can predict where highlight lands.

### 5. Flux Reconcile Pending State

Risk:
- Pending reconcile verification may complete while selected row reorders.
- Status/action history can update while list and detail are moving.

Check:
- Select Flux resource.
- Start reconcile.
- Watch update reorders selected resource.
- Refresh result lands after watch update.

Expected:
- Highlight remains on reconciled resource.
- Status bar message matches same resource.
- Action history entry target remains correct.

### 6. Split-Pane Focus And Scroll

Risk:
- Selection restore can fight secondary pane scroll/focus state.
- `;`, `j/k`, page keys, and Enter must stay consistent.

Check:
- Flux split view with list focus.
- Toggle secondary focus.
- Scroll detail pane.
- Trigger reorder/delete.
- Press `j/k`, `Enter`, page keys.

Expected:
- List focus still controls row selection.
- Secondary focus still controls detail scroll.
- Selection restore does not reset scroll unless selected resource changes.

### 7. Visual Stability

Risk:
- Correct identity preservation can still look bad if highlight jumps or viewport snaps.

Check:
- Reorder selected row from bottom to top.
- Reorder selected row outside current visible window.
- Refresh repeatedly during active watch churn.

Expected:
- Selected row remains visible.
- Highlight does not flicker to wrong resource.
- Status/loading affordance does not obscure selection.

### 8. Other Watched Tables

Risk:
- Same raw-index selection jump can exist outside Flux.
- Pods, Deployments, Services, Jobs, and Namespaces receive watch updates.

Check:
- Audit selected-resource preservation in watched resource tables.
- Start with Pods and Deployments due highest usage.

Expected:
- Either canonical identity preservation exists across watched tables, or documented reason Flux-only is enough.

## Manual Test Matrix

| Scenario | View | State | Update | Expected |
| --- | --- | --- | --- | --- |
| Reorder selected | Flux Kustomizations | No search, default sort | Watch reorder | Same resource selected |
| Reorder selected | Flux All | Name sort | Refresh reorder | Same resource selected |
| Reorder selected | Flux HelmReleases | Age sort | Apply/reconcile refresh | Same resource selected |
| Delete above | Flux Kustomizations | No search | Watch delete row above | Same resource selected, index changes |
| Delete selected | Flux Kustomizations | No search | Watch delete selected | Nearest stable fallback |
| Filter hides selected | Flux All | Active search | Status/name/message change | Predictable fallback and optional status |
| Detail open | Flux Kustomizations | Detail pane open | Watch reorder | Detail matches selected resource |
| Pending reconcile | Flux Kustomizations | Reconcile pending | Watch then refresh | History/status/list agree |
| Secondary focus | Flux All | Detail pane focused | Watch reorder | Focus/scroll stable |

## Verification Commands

Run focused first:

```bash
cargo test flux_selection_identity_survives_watch_reorder_and_delete_updates -- --nocapture
```

Then standard gate:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Run render profiling only if render/windowing/hot view code changes:

```bash
cargo test --test performance profile_render_path_and_emit_reports -- --ignored --nocapture
```
