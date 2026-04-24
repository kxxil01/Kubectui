# Flux UI/UX Regression Tasks

Use this backlog one item at a time. Keep each patch small, tested, and tied to one observable regression.

## Done

- [x] Preserve Flux selected resource identity across watch/refresh reorder.
- [x] Regression for reorder after watch/apply-style update.
- [x] Regression for delete-before-selection update.
- [x] Flux detail stays aligned with selected list resource after watch reorder.
- [x] Flux detail closes when the selected Flux resource is deleted.
- [x] Flux selected delete falls back to nearest neighbor for first/middle/last rows.
- [x] Flux active search preserves matching selection and falls back with status when hidden.
- [x] Flux name/age sort paths preserve selected identity after reorder.
- [x] Flux pending reconcile completion stays aligned after watch reorder/stale refresh.
- [x] Flux secondary pane focus/scroll stays stable across reorder and resets on resource change.
- [x] Flux visual stability covered for far reorder and repeated watch churn.
- [x] Non-Flux watched resource tables preserve selected identity after reorder.
- [x] Non-Flux watched resource details close when selected resource is deleted.

## Next Tasks

### 1. Detail Pane Alignment

- [x] Add regression: Flux detail open, watch reorder preserves selected list resource and detail resource alignment.
- [x] Decide deleted-selected behavior with detail open: close detail vs move to fallback resource.
- [x] Patch canonical detail/list sync path if stale detail survives.
- [x] Verify with focused test and full gate.

Done when:
- List highlight and detail title/resource always refer to same resource after reorder.
- Deleted selected resource has explicit, tested behavior.

### 2. Active Search Fallback

- [x] Add regression: selected Flux resource remains visible after reorder with active search.
- [x] Add regression: selected Flux resource stops matching search after update.
- [x] Decide UX message for selection disappearing from filtered results.
- [x] Patch fallback if raw clamp feels wrong or loses nearest identity.

Done when:
- Search-visible resources preserve identity.
- Search-hidden selected resource fallback is predictable and documented by tests.

### 3. Sort Stability

- [x] Add regression: Flux name sort reorder preserves selected identity.
- [x] Add regression: Flux age sort reorder preserves selected identity.
- [x] Check viewport window keeps selected row visible after sort-driven moves.
- [x] Patch if sort-specific cache/filtered indices bypass identity restore.

Done when:
- Same selected resource remains highlighted under name/age sort after refresh/watch reorder.

### 4. Delete Selected Resource

- [x] Add regression: deleting selected first row selects next row.
- [x] Add regression: deleting selected middle row selects nearest next row.
- [x] Add regression: deleting selected last row selects previous row.
- [x] Patch fallback from raw clamp to nearest-neighbor policy if needed.

Done when:
- Delete behavior is deterministic and matches user expectation.
- No arbitrary jump to unrelated row when better neighbor exists.

### 5. Pending Reconcile Race

- [x] Add regression: pending Flux reconcile, watch reorder, verification completion.
- [x] Add regression: pending Flux reconcile, refresh result after watch changed same target.
- [x] Ensure action history, status message, selected list row, and detail resource agree.

Done when:
- Reconcile status never appears to complete for a different highlighted resource.

### 6. Secondary Pane Focus

- [x] Add regression: Flux secondary pane focused, watch reorder preserves focus mode.
- [x] Add regression: detail scroll not reset when same selected resource moves rows.
- [x] Add regression: detail scroll resets only when selected resource changes.

Done when:
- `;`, `j/k`, page keys, and Enter still route predictably after watch reorder.

### 7. Visual Stability Audit

- [x] Regression: reorder selected row outside visible window.
- [x] Regression: repeated watch churn while Flux view active.
- [x] Capture any flicker/viewport snap symptoms as tests where practical.

Done when:
- Selected row remains visible.
- No visible highlight flash to wrong resource.

### 8. Generalize Beyond Flux

- [x] Audit Pods watch update selection behavior.
- [x] Audit Deployments watch update selection behavior.
- [x] Audit Services/Jobs/Namespaces if first two expose same raw-index risk.
- [x] Decide whether to generalize helper for all resource tables or keep Flux-only.

Done when:
- Same class of regression is either fixed canonically or explicitly scoped out with evidence.

### 9. Watched Detail Alignment

- [x] Add regression: Pod detail closes when selected Pod disappears after watch update.
- [x] Add regression: Deployment detail closes when selected Deployment disappears after watch update.
- [x] Add regression: Pod detail remains open when same selected Pod only reorders.
- [x] Generalize stale detail cleanup beyond Flux for active resource-table views.

Done when:
- List highlight and detail pane never refer to different active-view resources after watch fallback.

## Suggested Order

1. Detail pane alignment.
2. Delete selected resource fallback.
3. Active search fallback.
4. Sort stability.
5. Pending reconcile race.
6. Secondary pane focus.
7. Visual audit.
8. Non-Flux watched tables.

## Patch Rules

- One task per PR.
- Add failing regression first when practical.
- Use existing selection/filtering helpers, not duplicate per-view logic.
- Keep event-loop changes narrow.
- No screenshots or logs containing secrets.
