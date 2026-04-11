# Review Scope Audit

## Goal
- Exhaustively review Kubectui UI and interaction paths for correctness, stability, responsiveness, and predictable behavior.
- Keep one canonical implementation per behavior.
- Prefer fixing root causes in shared render/state/input paths over patching individual views.

## Review Principles
- Review by bug class, not random file order.
- Audit primary codepath first, then specialized surfaces.
- Prefer deterministic state transitions.
- Clamp all cursor/selection/scroll state after data changes.
- Preserve user intent across refresh, filter, sort, and re-render.
- Ensure narrow terminals degrade gracefully without hidden or unreachable content.
- Treat all mixed-height content as visual-row problems, not logical-line problems.

## Global Scope
- `src/ui/`
- `src/app/`
- `src/workbench.rs`
- `src/state/`
- `src/action/`
- `src/events/`
- render helpers, shared widgets, dialogs, overlays, detail views, list views, workbench panes

## Review Buckets

### 1. Selection State Invariants
- selected row/index never points past current data
- selected entity identity preserved when data refreshes
- selection stays valid after sort/filter/query change
- selection stays valid after deletion/removal
- selection does not silently jump because of unstable ordering
- map-backed collections use deterministic order before UI selection/windowing

Files to inspect:
- `src/state/`
- `src/workbench.rs`
- `src/app/detail_state.rs`
- `src/ui/views/*`
- `src/ui/components/*`

### 2. Scroll State Invariants
- scroll offset clamps after content shrink
- scroll offset preserves visible context when content grows
- page/line scroll uses actual visual rows where wrapping exists
- scrollbar total/position matches rendered content
- selected row remains visible after navigation
- detail panes keep focused/selected content visible

Files to inspect:
- `src/ui/mod.rs`
- `src/ui/components/workbench.rs`
- `src/ui/views/detail.rs`
- overlays, pickers, probe panels, help, runbook, AI, relations, history panes

### 3. Ordering Determinism
- no UI depends on raw `HashMap` or `DashMap` iteration order
- ordering source is explicit and stable
- refreshes preserve order semantics
- compact/list/detail variants use same ordering rules

Targets:
- port-forwarding
- watch/event-fed resources
- action history projections
- timeline merges
- any `values().collect()` render path

### 4. Narrow Width / Height Behavior
- no right-edge crop for actions, hints, status banners, titles, current-context labels
- no fixed-height row silently hides wrapped content
- dynamic headers/footers reserve real height
- compact mode remains readable on smallest supported terminal
- one-line content clamps intentionally when wrap would break layout

Targets:
- dialogs
- pickers
- workbench headers/filters
- detail footers and confirm dialogs
- summary cards and dashboard panels

### 5. Input Rendering / Cursor Follow
- long input keeps cursor visible
- prefix/suffix/cursor rendering never overflows width
- focused fields show true cursor position, not tail-only approximation
- editing paths share one canonical helper
- widget behavior identical across dialogs and workbench

Targets:
- search bars
- command palette
- namespace/context pickers
- secret edit
- exec prompt
- log search/jump
- dialog form fields

### 6. Mixed-Height Text Rendering
- wrapped paragraphs use visual-row scroll math
- no logical-line slicing followed by wrap
- long descriptions/help/error text always reachable
- progress/scroll feedback remains accurate on wrapped content

Targets:
- help overlay
- probe panel
- runbook detail
- AI analysis
- detail metadata panels
- debug dialog notes/errors
- template/edit forms

### 7. Dialog / Modal Lifecycle
- open/close/reset behavior consistent
- focus order deterministic
- compact and full modes preserve same business rules
- errors/warnings/pending states do not leave stale focus/scroll state
- dialog refresh does not lose user selection unless required

Targets:
- scale dialog
- port-forward dialog
- node debug dialog
- debug container dialog
- resource template dialog
- confirm dialogs in detail/workbench

### 8. Filter / Search / Sort Correctness
- filtered index caches stay aligned with selected row
- sort toggles preserve stable tie-breakers
- search reset or escape returns cursor/selection predictably
- filtered selection follows visible data, not stale raw index
- empty-result transitions handled cleanly

Targets:
- all table views
- workbench filtered panes
- command palette
- projects/governance/security split views

### 9. Split-Pane Detail Surfaces
- primary/detail panes share one detail-scroll model where appropriate
- lower detail panes do not clip long subjects/rules/notes
- table/detail selection linkage remains stable
- compact stacked fallback preserves both primary list and detail usefulness

Targets:
- security views
- governance center
- projects
- cronjob history
- detail metadata/metrics panes

### 10. Workbench-Specific Behavior
- tab focus, maximize, active tab, and close behavior remain stable
- specialized panes preserve selection by identity when backing data updates
- logs, exec, rollout, Helm, relations, traffic, connectivity, access review all clamp correctly
- mixed summary/header strips reserve actual height
- container pickers and revision lists keep cursor visible

Targets:
- `src/workbench.rs`
- `src/ui/components/workbench.rs`
- `src/app/input.rs`

### 11. Dashboard / Summary Integrity
- fixed-height cards do not wrap-spill or silently clip critical content
- alerts, insights, top consumers, namespace utilization, saturation panes scroll or clamp intentionally
- narrow dashboard remains stable and informative

Targets:
- `src/ui/views/dashboard.rs`

### 12. Error / Empty / Pending States
- empty states never break table windowing
- loading states do not leave stale selection or scroll
- error banners/messages remain visible in narrow layouts
- pending states preserve invariant state and recover cleanly

Targets:
- all dialogs
- detail surfaces
- workbench panes
- resource tables

### 13. Keyboard Routing / Focus Safety
- shortcuts route by active focus only
- text-input focus blocks conflicting shortcuts
- global shortcuts do not override modal-local editing
- focus changes reset only the state that should reset
- `Esc`, `Tab`, page keys, and movement keys are consistent

Targets:
- `src/app/input.rs`
- dialog handlers
- picker handlers
- workbench handlers

### 14. Render Performance Regressions
- fixes do not rebuild full off-screen data unnecessarily
- viewport/window helpers remain canonical
- dynamic wrapped headers do not introduce pathological allocation
- no duplicate render paths for same behavior

Targets:
- shared render helpers
- table windowing helpers
- scroll metric helpers
- workbench pane render functions

## Review Order
1. Shared helpers
2. Input and selection state
3. Ordering determinism
4. Scroll/clamp correctness
5. Dialogs and overlays
6. Split-pane detail views
7. Workbench specialized panes
8. Dashboard and summaries
9. Final narrow-terminal smoke sweep
10. Performance regression check

## Per-Pass Checklist
- find shared/root cause first
- patch canonical path
- add regression test
- run:
  - `cargo fmt --all`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - targeted tests
  - `cargo test --test performance profile_render_path_and_emit_reports -- --ignored --nocapture`

## Exit Criteria
- no confirmed remaining issue in current bug class
- all touched paths have regression coverage
- no stale selection, stale scroll, unstable order, or unreachable wrapped content on audited surfaces
- shared helpers remain single source of truth
