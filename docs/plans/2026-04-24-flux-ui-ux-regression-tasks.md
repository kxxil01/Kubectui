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
- [x] Non-Flux active search fallback preserves identity or closes stale detail.
- [x] Watched active-search fallback status clears after selection recovers.
- [x] Watched active-search empty fallback reports no visible results and clears on recovery.
- [x] Manual search edits clear stale watched-search fallback status without clearing unrelated status.
- [x] Manual search edits reset selection to first filtered result.
- [x] Manual search edits close stale detail panes.
- [x] View, namespace, and resource navigation clear stale watched-search fallback status.
- [x] Workspace restore and context switch clear stale watched-search fallback status.
- [x] Namespace switch closes stale detail panes.
- [x] View navigation closes stale detail panes.
- [x] Search escape with empty query closes stale detail pane.
- [x] Action-history resource jump preserves list/detail alignment.
- [x] Palette and runbook detail actions load the targeted resource detail.
- [x] Palette resource-only actions point detail state at the targeted resource.

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

### 10. Watched Active Search Fallback

- [x] Add regression: selected Pod remains highlighted when visible search results reorder.
- [x] Add regression: selected Pod stops matching search after watch update and falls to visible neighbor.
- [x] Add regression: selected Service stops matching search after watch update and falls to visible neighbor.
- [x] Ensure stale detail closes when active search fallback changes selected resource.

Done when:
- Active search on watched tables never leaves highlight/detail on different resources.

### 11. Watched Search Status Recovery

- [x] Add regression: fallback status clears after selected visible resource matches search again.
- [x] Add regression: unrelated status survives visible selection preservation.
- [x] Clear only the canonical search-fallback status.

Done when:
- Stale search-fallback status cannot linger after selection is back on a visible matching resource.

### 12. Watched Search Empty Result Recovery

- [x] Add regression: selected resource stops matching search and filtered result set becomes empty.
- [x] Add regression: stale empty-result fallback status clears when visible results return.
- [x] Keep detail closed when no row is visible after active-search fallback.

Done when:
- Empty active-search fallback never claims selection moved to a visible row that does not exist.
- Empty-result fallback status does not linger after search results return.

### 13. Manual Search Edit Status Recovery

- [x] Move watched-search fallback status strings to the app state source of truth.
- [x] Add regression: editing search query clears stale watched-search fallback status.
- [x] Add regression: editing search query preserves unrelated status messages.

Done when:
- Search edit/clear cannot leave a stale watch fallback message after user changes filter intent.
- Non-watch status messages still survive search editing.

### 14. Manual Search Edit Selection Reset

- [x] Add regression: editing search query resets selected index to first result.
- [x] Add regression: cursor-only search navigation preserves selected index.
- [x] Reset selected index only when query content changes.

Done when:
- Search narrowing/widening cannot reveal a stale raw selected index as an arbitrary row.
- Cursor movement inside the search box does not disturb table selection.

### 15. Manual Search Edit Detail Alignment

- [x] Add regression: editing search query closes open detail pane.
- [x] Add regression: cursor-only search navigation keeps detail pane open.
- [x] Close detail pane only when query content changes.

Done when:
- Search edit cannot leave list highlight and detail pane pointed at different resources.
- Cursor movement inside the search box does not close detail.

### 16. Navigation Search Status Recovery

- [x] Add regression: view navigation clears watched-search fallback status.
- [x] Add regression: namespace switch clears watched-search fallback status.
- [x] Add regression: resource jump clears watched-search fallback status.
- [x] Preserve unrelated status messages during view navigation.

Done when:
- Clearing search via navigation cannot leave stale watched-search fallback text.
- Non-watch status messages still survive navigation.

### 17. Workspace Search Status Recovery

- [x] Add regression: workspace restore clears watched-search fallback status.
- [x] Add regression: workspace restore preserves unrelated status messages.
- [x] Add regression: context switch UI reset clears watched-search fallback status.

Done when:
- Workspace restore/context switch cannot leave stale watched-search fallback text.
- Non-watch status messages still survive workspace restore.

### 18. Namespace Detail Alignment

- [x] Add regression: namespace switch closes open detail pane.
- [x] Patch canonical namespace mutator.

Done when:
- Switching namespaces cannot leave list highlight and detail pane pointed at different namespace scopes.

### 19. View Navigation Detail Alignment

- [x] Add regression: view navigation closes open detail pane.
- [x] Patch canonical view navigation mutator.

Done when:
- Switching views cannot leave list highlight and detail pane pointed at different resource views.

### 20. Search Escape Empty-Query Alignment

- [x] Add regression: search escape with empty query closes open detail pane.
- [x] Add regression: search escape with empty query clears watched-search fallback status.
- [x] Patch canonical search escape branch.

Done when:
- Exiting empty search cannot reset list selection while leaving detail pane on old resource.

### 21. Action History Resource Jump Alignment

- [x] Add regression: preparing a history/bookmark resource jump selects the target row after navigation reset.
- [x] Patch action-history open-selected path to use canonical resource-target preparation.

Done when:
- Opening a resource from action history cannot show target detail while the list highlight remains on a reset row.

### 22. Palette Target Detail Alignment

- [x] Add regression: palette/runbook detail actions require loading the target resource when another detail pane is open.
- [x] Patch palette and runbook deferred detail-action paths to compare resource identity, not just detail presence.

Done when:
- Palette/runbook actions cannot run a target-specific command against a stale open detail pane for another resource.

### 23. Palette Resource-Only Target Alignment

- [x] Add regression: palette resource-only actions still point detail state at the explicit target resource.
- [x] Patch the palette target-preparation helper so logs/exec/port-forward/bookmark/view actions do not inherit a stale open detail pane.

Done when:
- Palette actions that only need resource identity cannot run against the previously open detail resource.

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
