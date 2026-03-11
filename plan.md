# KubecTUI Milestone Plan

## Status

This file is the canonical source of truth for product and implementation priority.

When this document conflicts with older notes, ad hoc ideas, or chat suggestions, this document wins.

This plan does not aim to copy Lens, OpenLens, or Freelens visually. It adopts the workflow patterns that make those tools easy to learn, then translates them into a terminal-native Rust + `ratatui` application.

## Implementation Progress

Current milestone status:

- Milestone 0: completed
- Milestone 1: completed
- Milestone 2: completed
- Milestone 3: completed
- Milestone 4: completed
- Milestone 5: completed
- Milestone 6: completed
- Milestone 7: completed
- Milestone 8: completed
- Milestone 9: completed
- Milestone 10: completed
- Milestone 11: completed
- Milestone 12: completed
- Milestone 13: completed
- Milestone 14: completed

Completion notes:

- Milestone 0 shipped the canonical policy layer and aligned UI hints, keybindings, and runtime action guards.
- Milestone 1 shipped the bottom workbench foundation with persisted open state, persisted height, tab management, focus handling, and layout integration.
- Milestone 2 moved YAML, events, logs, and port-forward sessions onto the workbench-backed path and removed the old duplicate detail-only inspection path.
- Milestone 3 added a canonical action history model, recorded pending/success/error mutation state centrally, and exposed the verification surface in the workbench with jump-back to affected resources where possible.
- Milestone 4 added pod exec/shell sessions hosted in the workbench with container selection, shell fallback order, bounded scrollback, and error handling.
- Milestone 5 added multi-pod and workload-level logs with per-pod/container/text filtering, follow mode, "All Containers" picker in pod logs, and workload log aggregation from deployments/statefulsets/daemonsets.

- Milestone 10 shipped the Relationship Explorer: workbench-hosted Relations tab with 6 resolver chains (owner, service backends, ingress backends, storage bindings, RBAC bindings, Flux lineage), indented expand/collapse tree with connectors, `w` keybinding from detail view, action palette integration, and Enter-to-jump navigation to related resources.

- Milestone 12 shipped the Issue Center: a problem-first view with 11 detection categories (CrashLoopBackOff, ImagePullFailure, pending/failed pods, node not-ready/pressure, degraded workloads, storage issues, Flux reconcile failures, services with no endpoints, failed jobs). Issues are computed from snapshot data (no new API calls), cached by snapshot_version, sorted by severity, capped at 500. Searchable table with Enter→jump-to-detail, dashboard health summary shows issue count.

- Milestone 11 shipped Node Operations: cordon (`c`), uncordon (`u`), and drain (`D`) for Kubernetes nodes. Cordon/uncordon execute immediately with optimistic cache updates. Drain shows a confirmation dialog with force-drain option (`F`). Node list status column now shows SchedulingDisabled for cordoned nodes. All three operations are available via the action palette and recorded in action history.

Post-milestone fixes and improvements (shipped after M5):

- Deep audit: 23 fixes across 35 files (UTF-8 safe truncation, time-based backoff, TOCTOU race fix, non-blocking extension fetch, temp file security, magic number cleanup)
- Age sorting fix: None values always sort last regardless of sort direction
- Delete confirmation UX: accepts D/y/Enter (not just Shift+D), widened dialog, updated footer hints
- Workbench maximize: `z` toggles fullscreen mode, Esc restores, MAX_WORKBENCH_HEIGHT bumped from 20 to 40
- All Containers: pod logs picker shows "All Containers" when 2+ containers, opens WorkloadLogs tab

Verification status for completed milestones:

- 626 tests passing, zero clippy warnings, fmt clean, dev+release builds passing
- remaining validation gap is live-cluster smoke behavior under real kube context and RBAC

---

## Mission

KubecTUI should become a fast, reliable, keyboard-first Kubernetes operations workspace.

The app should feel:

- fast on first load
- predictable across views
- easy to learn for users coming from Lens-like tools
- strong at day-2 operations, not just resource browsing

The app should not become:

- a GUI imitation inside a terminal
- a pile of unrelated tables
- a modal-heavy workflow maze

---

## Product Rules

These rules apply to every milestone.

## 1. TUI-native first

Adopt workflow patterns, not GUI widgets.

Good translations:

- dock -> bottom workbench
- command palette -> action palette
- resource graph -> text/tree relationship explorer
- issues panel -> issue center
- right-panel detail -> modal detail overlay

Bad translations:

- draggable floating windows
- graph canvases that do not fit the terminal
- mouse-first interaction
- visually dense desktop metaphors

## 2. Performance is a feature

Every feature must have a performance budget.

No milestone should introduce:

- blocking network work in render path
- unbounded buffers
- repeated full-data recomputation every frame
- refresh storms after actions
- visible lag in hot views

## 3. One canonical behavior model

Shared app behavior should not be reimplemented per view.

Canonical policies should exist for:

- sorting
- loading
- mutation lifecycle
- action availability
- relationship availability
- persistence behavior

## 4. Empty is not loading

All views must explicitly distinguish:

- loading
- refreshing
- ready but empty
- error

## 5. Actions must be verifiable

After a mutation, the user must be able to tell:

- what happened
- whether it succeeded
- where to look next

---

## Current Strengths

KubecTUI already has:

- 46 resource views across 9 sidebar groups (Overview, Workloads, Network, Config, Storage, Helm, FluxCD, Access Control, Custom Resources)
- workload-first loading and background hydration
- per-view loading state with explicit loading/refreshing/ready/empty/error states
- responsive layouts with sidebar + content + workbench split
- shared sorting across workloads and pods with ascending/descending toggle
- bottom workbench with 8 persistent tab types (ActionHistory, YAML, Timeline, PodLogs, WorkloadLogs, Exec, PortForward, Relations)
- workbench maximize (`z` to fullscreen, Esc to restore), resizable height (8-40 lines)
- pod logs with container picker, "All Containers" option, follow mode
- workload-level log aggregation with pod/container/text filtering
- pod exec/shell sessions with container selection and shell fallback
- port-forwarding with session management
- scaling dialog (deployments/statefulsets)
- rollout restart
- delete with multi-key confirmation (D/y/Enter)
- YAML edit (opens in $EDITOR, applies on save)
- Flux reconcile
- action history with pending/success/error tracking and resource jump-back
- action palette (`:`) for navigating to any of 46 views AND executing context-aware resource actions (logs, exec, scale, restart, delete, etc.)
- context and namespace switching
- CRD browsing with dynamic instance viewing
- search mode (`/`) with case-insensitive substring matching
- 5 themes (Dark, Nord, Dracula, Catppuccin, Light)
- probe panel (liveness, readiness, startup probe inspection)
- dashboard with cluster health gauges, alerts, and workload summaries
- configuration persistence (namespace, theme, workbench state, refresh interval)

This means the next phase should focus on discoverability, operator quality of life, and closing the UX gaps that Lens/Freelens users expect.

---

## Lens/Freelens UX Research

This section documents the key UX patterns from Lens and Freelens that inform our gap analysis and milestone priorities. These are workflow references, not UI templates.

### What Lens Gets Right

**Discoverability**: Users can discover all features without reading docs. Every action is visible in context menus, keyboard shortcuts are listed, and a help system exists.

**Log viewing**: Previous logs for crashed containers, search/highlight within logs, timestamp toggle, log level filtering, line count limits, download/export to file.

**Resource information density**: Pod tables show IP and Node columns. Detail views show labels, annotations, environment variables, owner references, and container resource limits.

**Status communication**: Color-coded pod status, ready fraction indicators (1/2), node condition badges, deployment health inference (healthy/degraded/failed).

**Sidebar context**: Resource counts next to categories (e.g., "Pods (47)"), always-visible cluster context and namespace.

**Node operations**: Cordon, uncordon, drain — critical for cluster operators.

**Relationship navigation**: Owner references link resources. Service -> Endpoints -> Pod chains are navigable.

**Data operations**: Copy to clipboard, CSV export, inline editing of replica counts.

### What We Should NOT Copy

- Resizable/reorderable columns via mouse drag
- Right-click context menus (no mouse in TUI)
- Desktop-style tab management (Cmd+number)
- Integrated Prometheus metric graphs (too complex for TUI; sparklines possible later)
- Hotbar/catalog model (desktop-specific)
- Extension marketplace
- Monaco YAML editor

---

## Gaps To Close

These are the main gaps between the current app and a strong operator workspace, updated with Lens/Freelens research findings.

## Gap A: Weak discoverability

Current issue:

- no help screen or keybinding reference
- users from Lens cannot discover shortcuts without reading source code
- inline footer hints only cover detail view actions, not global keys

Needed:

- `?` help overlay listing all keybindings organized by context
- searchable keybinding reference
- contextual hints that adapt to current focus mode

## Gap B: Incomplete log workflow

Current issue:

- no previous logs for crashed/restarted containers
- no search/highlight within pod logs
- no timestamp toggle
- no log export to file
- pod logs lack the text filter that workload logs have

Needed:

- previous logs toggle (`--previous` flag)
- search with highlight in all log views
- timestamp prefix toggle
- export/save to file

## Gap C: Missing resource information

Current issue:

- pod table lacks IP and Node columns
- detail view does not show labels, annotations, or environment variables
- no owner reference links between resources

Needed:

- Pod IP and Node columns in pods table
- labels/annotations section in detail view
- owner reference display with jump-to navigation

## Gap D: No clipboard integration

Current issue:

- no way to copy pod names, IPs, YAML snippets, or log lines

Needed:

- yank/copy to system clipboard for selected values

## Gap E: Missing node operations

Current issue:

- node inspection exists but no operational actions

Needed:

- cordon / uncordon
- drain with strong confirmation and progress feedback

## Gap F: Missing resource actions

Current issue:

- no force delete for stuck resources (finalizers)
- no manual CronJob trigger
- no inline replica editing (currently modal only)

Needed:

- force delete option
- CronJob manual trigger
- streamlined scaling UX

## Gap G: Weak sidebar context

Current issue:

- sidebar shows group names but no resource counts
- current context/namespace is in header but not prominently actionable

Needed:

- resource counts per sidebar group or view
- clear context/namespace indicator at all times

## Gap H: No timeline and correlation surface

Current issue:

- action history exists but is separate from event chronology
- no timeline-oriented verification

Needed:

- event/action correlation
- timeline-oriented verification
- faster understanding of what changed after an action

## Gap I: Weak relationship navigation (addressed in M10)

Addressed by Milestone 10 (Relationship Explorer):

- deployment -> replicaset -> pod (owner chain)
- service -> endpoint -> pod (service backends)
- ingress -> service -> pod (ingress backends)
- PVC -> PV -> StorageClass (storage bindings)
- SA -> RoleBinding -> Role / ClusterRoleBinding -> ClusterRole (RBAC bindings)
- Flux source -> downstream resource chain (Flux lineage)

## Gap J: Weak issue-centered workflow

Current issue:

- users must think in resources more than problems

Needed:

- issue center with grouped problem categories
- action-oriented issue drilldown

## Gap K: Weak persistent personalization

Current issue:

- users cannot fully shape and retain their workspace

Needed:

- per-view saved sort
- per-view columns
- context-aware preferences

---

## Milestone Strategy

The milestones below are ordered deliberately. Each milestone unlocks the next one and reduces the chance of duplicate paths or UI inconsistency.

The rule is:

- do not skip foundational milestones
- do not build later workflows on temporary UI paths

---

## Milestone 0: Foundation Audit and Canonical Policy Cleanup

### Status

Completed

### Goal

Stabilize the core interaction and policy model before adding major new workflows.

---

## Milestone 1: Workbench Foundation

### Status

Completed

### Goal

Create a persistent bottom workbench that becomes the home for long-lived operational surfaces.

---

## Milestone 2: Migrate Existing Operational Surfaces Into Workbench

### Status

Completed

### Goal

Move existing high-value inspection tools into the workbench so the app stops relying on scattered blocking experiences.

---

## Milestone 3: Action History and Verification Surface

### Status

Completed

### Goal

Give users a first-class place to understand what happened after they triggered an action.

---

## Milestone 4: Pod Exec / Shell

### Status

Completed

### Goal

Add a terminal-native exec workflow for pods and containers.

### What shipped

- exec session state model with container selection
- shell fallback order (/bin/bash -> /bin/sh -> /busybox/sh)
- workbench-hosted exec tab with input routing
- bounded scrollback (5,000 lines)
- error handling for pod not running, container not ready, exec forbidden, shell missing

---

## Milestone 5: Multi-Pod and Workload-Level Logs

### Status

Completed

### Goal

Make workload debugging first-class.

### What shipped

- workload log session model with concurrent pod streaming
- per-pod, per-container, and text filtering
- follow/pause controls
- "All Containers" picker in pod logs (switches to WorkloadLogs tab)
- bounded line buffer (5,000 lines)
- partial failure handling

---

## Milestone 6: Discoverability and Operator Quality of Life

### Status

Completed

### What shipped

- `?` help overlay with keybinding reference (6 sections, scrollable)
- Previous logs toggle (`P` key, `--previous` flag for crashed containers)
- Log search and highlight (`/` to search, `n`/`N` for next/prev match, highlighted matches)
- Timestamp toggle (`t` key, re-fetches with `--timestamps`)
- Sidebar resource counts (e.g., "Pods (12)")
- Pod IP column in pods table

### Goal

Close the most impactful UX gaps that Lens/Freelens users expect. Make the app learnable without docs and improve day-2 debugging workflows.

### Why this comes next

Users from Lens will evaluate the app in the first 5 minutes. If they cannot discover shortcuts, view previous logs for crashed pods, or search within logs, they will leave. These are table-stakes features, not advanced workflows.

### Scope

- help overlay
- previous logs
- log search and highlight
- timestamp toggle in logs
- sidebar resource counts
- Pod IP and Node columns

### Tasks

1. Add `?` help overlay that shows all keybindings organized by context (Global, Detail, Workbench, Logs, Exec, Port-Forward). Dismissible with `?` or `Esc`.
2. Add previous logs toggle in pod logs tab:
   - `P` key to toggle `--previous` flag
   - status line shows "previous" indicator
   - re-fetches log stream with previous flag
3. Add search within logs:
   - `/` in pod logs tab enters search mode (workload logs already have text filter)
   - highlight matching text in log lines
   - `n`/`N` for next/previous match
4. Add timestamp toggle:
   - `t` in log tabs to toggle timestamp prefix
   - fetch logs with `--timestamps` flag
5. Add resource counts to sidebar groups:
   - show count of loaded resources next to each view name (e.g., "Pods 47")
   - counts update on data refresh
   - do not count resources that have not been loaded yet (show nothing, not 0)
6. Add Pod IP and Node columns to pods table:
   - IP column shows pod IP
   - Node column shows node name (truncated if needed)
7. Add tests for:
   - help overlay state toggle
   - previous logs flag propagation
   - log search matching and cursor movement
   - timestamp toggle state
   - sidebar count rendering
   - pod table column additions

### Deliverables

- discoverable help system
- previous logs for crashed container debugging
- searchable log output
- timestamp-aware log viewing
- sidebar with resource counts
- richer pod table

### Risks

- help overlay becoming stale if shortcuts change
- previous logs re-fetch causing brief flash
- log search performance on large buffers

### Guardrails

- help overlay should be auto-generated from a keybinding registry, not hand-maintained
- previous logs uses same bounded buffer
- log search uses simple substring match, not regex (keep it fast)

### Acceptance Criteria

- a new user can press `?` and learn all shortcuts within 30 seconds
- a user debugging a CrashLoopBackOff pod can view previous logs without leaving the app
- a user can search for an error string within logs and jump between matches
- sidebar shows at-a-glance resource counts

---

## Milestone 7: Clipboard and Data Export

### Status

Completed

### What shipped

- OSC 52 clipboard module (terminal-native, no platform-specific code)
- `Ctrl+y` copies resource name, `Y` copies namespace/name
- `y` in log tabs copies all log content to clipboard
- `S` in log tabs exports log buffer to file (`/tmp/kubectui-logs-{label}-{timestamp}.log`)
- Status bar feedback on copy/export success
- Help overlay updated with all new keybindings

### Goal

Let users get data out of the TUI without manual transcription.

### Scope

- copy to system clipboard
- log export to file

### Tasks

1. Add yank/copy to clipboard:
   - `y` on a selected resource row copies resource name
   - `Y` copies full resource identifier (namespace/name)
   - in log views, yank copies visible log content
   - uses OSC 52 escape sequence for terminal clipboard access
2. Add log export:
   - `S` in log tabs saves current buffer to file
   - default path: `/tmp/kubectui-logs-{resource}-{timestamp}.log`
   - status message confirms save path
3. Add tests for clipboard content generation and file write

### Deliverables

- clipboard integration
- log file export

### Acceptance Criteria

- users can paste resource names and log content into other tools

---

## Milestone 8: Enhanced Resource Detail

### Status

Completed

### What shipped

- Annotations displayed in detail metadata panel (up to 5, values truncated at 60 chars)
- Owner references shown as OWNERS section in detail view (Kind/Name)
- Multi-line labels display (up to 5 in metadata panel)
- Force delete option: `F` key in delete confirmation dialog uses grace_period_seconds=0
- CronJob manual trigger: `T` key creates a Job from CronJob spec, recorded in action history
- Help overlay updated with `F` (force delete) and `T` (trigger CronJob) entries
- Container environment variables deferred (YAML view already provides this data)

### Goal

Bring detail view information density closer to Lens without visual clutter.

### Scope

- labels and annotations display
- owner reference links
- force delete option
- CronJob manual trigger

### Acceptance Criteria

- users can inspect labels, annotations, and ownership without kubectl
- stuck resources can be force-deleted from the TUI
- CronJobs can be manually triggered from detail view

---

## Milestone 9: Action Palette v2

### Status

Completed

### What shipped

- Unified action palette (`:`) mixing navigation and context-aware resource actions
- `PaletteEntry` enum with `Navigate(AppView)` and `Action(DetailAction)` variants
- 12 action aliases filtered by `ResourceRef::supports_detail_action()` — unavailable actions hidden
- Section headers (`── Actions ──` / `── Navigate ──`) with fuzzy matching across both
- Context resolution from detail view or highlighted list row at palette open time
- `AppAction::PaletteAction` with detail-open-then-act pattern for actions needing detail view
- Deferred action dispatch with cleanup on Esc/error (no stale action race)
- Palette opens from both list view and detail view
- 21 palette-specific tests covering action availability, ordering, filtering, execute dispatch

### Goal

Turn the command palette into the main discoverability and action surface.

### Acceptance Criteria

- users can find most core workflows from the palette
- unavailable actions are hidden based on resource type

---

## Milestone 10: Relationship Explorer

### Status

Completed

### What shipped

- RelationNode tree model with ResourceRef-typed references and 6 resolver chains
- Owner chain resolver with cycle detection (MAX_OWNER_CHAIN_DEPTH=20) and NotFound handling
- Service backends resolver (Service → Endpoints → Pods via selector matching)
- Ingress backends resolver (Ingress → Service → Pods with IngressClass display)
- Storage bindings resolver (PVC ↔ PV ↔ StorageClass with status display)
- RBAC bindings resolver (ServiceAccount → RoleBinding/ClusterRoleBinding → Role/ClusterRole)
- Flux lineage resolver (HelmRelease → source/downstream matching by URL and name-prefix)
- Workbench-hosted Relations tab with indented expand/collapse tree and Unicode connectors
- `w` keybinding from detail view, action palette integration (`View Relations`)
- Enter-to-jump navigation opening detail for related resources
- Snapshot-based resolution (no additional API calls)
- 506 tests passing including relationship-specific coverage

### Goal

Let users move through the cluster by dependency and ownership, not just by resource kind.

### Scope

- resource relationship tab or panel
- text/tree representation
- jumpable related resources

### Tasks

1. Add relationship capability policy per view/resource.
2. Add relationship node and edge model.
3. Implement first relation sets:
   - deployment -> replicaset -> pod
   - service -> endpoints -> pod
   - ingress -> service -> pod
   - PVC -> PV -> storageclass
   - Flux source -> downstream resources
4. Render relationships as an expandable tree/list.
5. Add jump-to-related-resource behavior.
6. Add tests for:
   - relation discovery
   - broken/missing relation handling
   - navigation behavior

### Deliverables

- relationship explorer available from supported resources

### Risks

- attempting to build a visual graph that does not fit TUI constraints
- over-fetching related resources synchronously

### Guardrails

- keep relation rendering text-native
- reuse already-loaded data where possible
- background hydrate missing relation sets if needed

### Acceptance Criteria

- users can answer "what depends on this?" and "what does this point to?" directly inside the app

---

## Milestone 11: Node Operations

### Status

Completed

### Goal

Support high-value node lifecycle actions safely.

### Scope

- cordon
- uncordon
- drain

### Tasks

1. Add node action capability policies.
2. Implement cordon.
3. Implement uncordon.
4. Implement drain with:
   - clear confirmation
   - strong warning text
   - visible progress/error feedback
5. Record actions in action history.
6. Add tests for:
   - action availability
   - mutation lifecycle
   - error handling

### Deliverables

- safe node operations from inside KubecTUI

### Risks

- accidental destructive operations
- insufficiently clear UX around drain consequences

### Guardrails

- drain must be treated as a high-risk action
- confirmation must be stronger than ordinary mutations

### Acceptance Criteria

- node operations are powerful but deliberate and safe

---

## Milestone 12: Issue Center

### Status

Completed

### Goal

Make problem-centered operations first-class.

### Scope

- dedicated issues view
- grouped problem categories
- drilldown to affected resources

### Tasks

1. Add `IssueRecord` model.
2. Define categories:
   - crash loop
   - image pull failure
   - pending scheduling
   - node pressure
   - probe failure
   - storage bind failure
   - ingress/backend mismatch
   - Flux reconcile failure
3. Add issue aggregation and deduplication.
4. Render issue list with severity and affected count.
5. Link issue rows to affected resources.
6. Surface top issues on dashboard.
7. Add tests for:
   - issue grouping
   - severity ordering
   - deduplication

### Deliverables

- issue center
- dashboard issue summary

### Risks

- issue noise
- weak prioritization making the feature less useful

### Guardrails

- prioritize actionability
- deduplicate aggressively
- focus on operator-relevant problems, not raw event spam

### Acceptance Criteria

- users can start from problems instead of resource kinds
- issue drilldown is fast and actionable

---

## Milestone 13: Timeline and Event Correlation

### Status

Completed

### What shipped

- Unified per-resource Timeline tab (upgraded from Events tab) merging K8s events with action history entries
- Chronological sorting with timestamps displayed on every line (HH:MM:SS in local timezone)
- 5-minute correlation window: K8s events following a user action are visually marked with `~` prefix in accent2 color
- User actions rendered with `>>>` prefix in accent color with status badges (OK/PENDING/ERROR)
- Timeline auto-rebuilds when action history changes, scoped to the affected resource's tab only
- Sort tiebreaker: Actions sort before Events at equal timestamps for deterministic correlation
- Scroll bounds clamped after rebuild to prevent stale scroll positions
- Render optimization: only visible window lines built per frame (O(visible) not O(n))
- `truncate_message` returns `Cow<str>` for zero-alloc fast path, handles edge cases (max_chars < 4, Unicode)
- Events capped at MAX_TIMELINE_EVENTS=200 per tab to bound memory
- 20 timeline tests + 10 truncate_message tests covering merge, sort, correlation (within/outside/boundary/overlapping/same-timestamp), filtering, status variants, edge cases

### Goal

Show users what happened over time around a resource or action.

### Scope

- resource event timeline
- recent mutation result timeline
- correlation between actions and events where possible

### Tasks

1. Extend workbench events/history model for timeline use.
2. Correlate actions with recent events when possible.
3. Improve event rendering around selected resources.
4. Add tests for ordering and correlation behavior.

### Deliverables

- stronger post-action verification workflow

### Risks

- weak correlation producing misleading timelines

### Guardrails

- prefer explicit event ordering over speculative correlation

### Acceptance Criteria

- users can understand what changed after an action with less guesswork

---

## Milestone 14: View Personalization and Workspace Persistence

### Status

Completed

### What shipped

- ViewPreferences model with sort_column, sort_ascending, hidden_columns, shown_columns, column_order
- Field-level preference merge (defaults ← global ← cluster) with shown_columns un-hide mechanism
- Cluster-aware preference routing: writes go to cluster-specific prefs when active, else global
- Column registry (ColumnDef) for 23 views with hideable/non-hideable flags, title-case labels
- Column toggle via action palette (`:` then search "columns", checkbox-style `[x]`/`[ ]` toggle)
- Sort persistence: sort preferences saved per-view and restored on view switch
- Sort clear targets most-specific level only (cluster if present, else global)
- Dynamic column rendering for Pods, Deployments, Nodes (column-driven headers, rows, constraints)
- Nav group collapse persistence across sessions
- Config dirty flag with batched saves in event loop
- Context name tracking on context switch for per-cluster preference resolution
- Backward-compatible JSON config expansion (all new fields use `#[serde(default)]`)
- Help overlay updated with sort keybindings for non-pod views
- 24 new tests across preferences, columns, sort persistence, config round-trip

### Goal

Make the app remember how each user works.

### Scope

- persisted workbench state
- per-view sort
- per-view columns
- optional per-context preferences

### Tasks

1. Add `ViewPreference` model.
2. Add `ViewColumnConfig`.
3. Persist:
   - sort
   - visible columns
   - workbench size
   - selected workbench tab where safe
4. Define scope rules:
   - global
   - cluster-specific
   - namespace-specific only if clearly justified
5. Add UI flows for column toggling.
6. Add tests for:
   - persistence load/save
   - invalid config fallback
   - context isolation

### Deliverables

- remembered workspace preferences

### Risks

- preference leakage across unrelated clusters
- configuration complexity

### Guardrails

- keep persistence rules simple and explicit
- prefer stable defaults over over-customization

### Acceptance Criteria

- users can reopen the app without rebuilding their workspace every time

---

## Milestone 15: Performance and Scale Track

### Status

Continuous (parallel to all milestones)

### Goal

Keep the app fast as the workbench and workflow surface grow.

### Scope

- virtualization/windowing
- derived row caching
- bounded async/session resources
- render-path validation

### Tasks

1. Implement shared row windowing for large tables.
2. Add per-view formatted row caches.
3. Use coarse invalidation for time-sensitive cells like age.
4. Bound buffers for:
   - logs
   - exec
   - actions
   - events
5. Add or improve instrumentation for:
   - render duration
   - sort/filter duration
   - refresh queue depth
   - active session count
6. Run performance validation on every major render-affecting milestone.

### Deliverables

- maintained or improved render responsiveness under larger workloads

### Risks

- feature work outrunning performance discipline

### Guardrails

- no render-path change lands without measurement

### Acceptance Criteria

- hot views remain responsive
- long-running sessions remain stable
- workbench does not degrade baseline navigation quality

---

## Milestone Dependencies

These dependencies are strict unless there is a compelling reason to revise the plan.

| Milestone | Depends On | Reason |
|---|---|---|
| 0 Foundation Audit | none | establishes canonical policy model |
| 1 Workbench Foundation | 0 | avoids duplicate architecture |
| 2 Workbench Migration | 1 | needs workbench state/layout |
| 3 Action History | 1, 2 | depends on workbench and canonical action flow |
| 4 Pod Exec | 1, 2 | needs workbench terminal/session surface |
| 5 Multi-Pod Logs | 1, 2 | needs workbench and log session model |
| 6 Discoverability & QoL | 0-5 | builds on complete operational surface |
| 7 Clipboard & Export | 6 | extends data accessibility |
| 8 Enhanced Detail | 0 | needs canonical policy model |
| 9 Action Palette v2 | 0, 6 | needs capability tables and help system |
| 10 Relationship Explorer | 0, 1 | benefits from centralized capability policy and workbench |
| 11 Node Operations | 0, 3 | depends on action lifecycle and verification surfaces |
| 12 Issue Center | 0 | needs canonical issue model |
| 13 Timeline | 2, 3 | depends on workbench events/history |
| 14 Persistence | 1, 2, 9 | should persist stable user workflows, not temporary ones |
| 15 Performance Track | all | parallel quality track across all milestones |

---

## Implementation Priority

This is the execution order.

## P0 (Completed)

- Milestone 0: Foundation Audit
- Milestone 1: Workbench Foundation
- Milestone 2: Workbench Migration
- Milestone 3: Action History
- Milestone 4: Pod Exec
- Milestone 5: Multi-Pod Logs

## P1 (Completed)

- Milestone 6: Discoverability & Operator QoL
- Milestone 7: Clipboard & Export
- Milestone 8: Enhanced Detail
- Milestone 9: Action Palette v2

## P2 (Completed)
- Milestone 10: Relationship Explorer ✅
- Milestone 11: Node Operations ✅

## P3 (Completed)

- Milestone 12: Issue Center ✅
- Milestone 13: Timeline & Correlation ✅
- Milestone 14: View Personalization ✅

## P4

- Milestone 15: Performance Track remains continuous and parallel, not deferred

---

## What We Should Start Right Now

M0-M14 are complete. The next milestone is M15: Performance Track (virtualization, caching, instrumentation).

Do not start next with:

- AI features
- plugin systems
- batch actions
- visual graph experiments that don't fit TUI constraints
- desktop-style interaction patterns

---

## Milestone Task Classification

This section classifies work so implementation stays manageable and careful.

## Type A: Architecture Tasks

These change core state or policy models.

Examples:

- workbench state model
- action history model
- issue model
- capability tables
- persistence model
- keybinding registry

Rules:

- design first
- test state transitions
- keep one canonical source of truth

## Type B: UI Layout Tasks

These change rendering and navigation surfaces.

Examples:

- workbench bottom pane
- tab strip
- help overlay
- timeline view
- issue center rendering
- sidebar resource counts

Rules:

- preserve keyboard clarity
- preserve responsiveness
- keep focus ownership explicit

## Type C: Interaction Tasks

These change user input flows.

Examples:

- tab switching
- workbench focus
- action palette behavior
- exec input routing
- log search navigation

Rules:

- no ambiguous focus
- every input path must have obvious exit behavior

## Type D: Async Workflow Tasks

These involve background tasks or streaming data.

Examples:

- logs (including previous logs)
- exec
- reconcile/status follow-up
- event correlation

Rules:

- bounded resources
- explicit lifecycle
- visible status

## Type E: Persistence Tasks

These change saved configuration or remembered state.

Examples:

- workbench height
- sort persistence
- column persistence

Rules:

- stable format
- safe fallback on invalid config
- no hidden cross-context leakage

---

## Quality Gate For Every Milestone

Every milestone that changes code should pass:

1. `cargo fmt --all`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test --all-targets --all-features`
4. `cargo test --test performance profile_render_path_and_emit_reports -- --ignored --nocapture`

When hot render paths change materially, also run the median comparison workflow.

---

## Definition of Done

A milestone is done only when:

- the feature is implemented on the canonical path
- duplicate temporary paths are removed or intentionally scoped
- tests cover the important state transitions
- render-path performance remains acceptable
- the UX is consistent with the rest of the app
- the user can discover and verify the workflow without guesswork

---

## Future / Not Now

These are valid ideas, but they are not current milestone priorities:

- AI assistant features
- extension marketplace or plugin framework
- desktop-like window choreography
- advanced diff/merge editor beyond practical YAML workflows
- visually complex graph canvases
- integrated Prometheus metric graphs (sparklines possible in later milestone)
- Helm chart installation workflow
- CSV export of table views

They should not distract from the milestone order above.

---

## External Product References

These are workflow references, not UI templates:

- Lens docs: [https://docs.k8slens.dev/](https://docs.k8slens.dev/)
- Lens repo: [https://github.com/lensapp/lens](https://github.com/lensapp/lens)
- Freelens repo: [https://github.com/freelensapp/freelens](https://github.com/freelensapp/freelens)

Use them to learn:

- what workflows matter
- what operators expect
- how discoverability is handled

Do not use them as justification to clone desktop interaction patterns into a terminal.

---

## Final Direction

KubecTUI should evolve from:

- a fast Kubernetes browser

into:

- a fast terminal-native Kubernetes operations workspace

The correct path is:

- build the workbench (done)
- migrate long-lived operational flows into it (done)
- strengthen action verification (done)
- add exec, workload logs (done)
- close discoverability and QoL gaps (done)
- add clipboard and export (done)
- add enhanced detail, action palette (done)
- add node operations, issues (done)
- add timeline and event correlation (done)
- add view personalization and workspace persistence (done)
- preserve speed at every step

This milestone plan is now the canonical implementation roadmap.
