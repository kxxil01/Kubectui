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
- Milestone 16: not started
- Milestone 17: not started
- Milestone 18: not started
- Milestone 19: not started
- Milestone 20: not started
- Milestone 21: not started
- Milestone 22: not started
- Milestone 23: not started
- Milestone 24: not started
- Milestone 25: not started

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

## Gap K: Weak persistent personalization (addressed in M14)

Addressed by Milestone 14 (View Personalization):

- per-view saved sort with cluster-specific overrides
- per-view column visibility toggle via action palette
- context-aware preference hierarchy (global ← cluster)

## Gap L: Secret management friction

Current issue:

- viewing Secret values requires manual base64 decode outside the TUI
- editing Secrets requires manual encode + YAML apply

Needed:

- automatic decode/encode in Secret detail view
- masking for shoulder-surfing protection

## Gap M: No resource bookmarks

Current issue:

- in large clusters, repeatedly navigating to the same critical resources
- no way to pin frequently-accessed resources

Needed:

- per-cluster bookmarks with instant jump
- bookmark persistence across sessions

## Gap N: Weak CronJob observability

Current issue:

- CronJob → Job → Pod chain requires manual navigation
- no unified execution history view

Needed:

- CronJob execution history panel
- next-run-time display
- suspend/resume capability

## Gap O: No ephemeral container debugging

Current issue:

- exec fails on distroless/minimal containers with no shell
- users must leave TUI for `kubectl debug`

Needed:

- ephemeral debug container launcher with image presets
- automatic shell session after container creation

## Gap P: No Helm rollback from TUI

Current issue:

- Helm releases are viewable but not actionable
- rollback requires leaving TUI for helm CLI

Needed:

- revision history display
- one-key rollback with confirmation
- values diff between revisions

## Gap Q: No resource utilization visibility

Current issue:

- no actual CPU/memory usage data in pod or node views
- right-sizing requires external tools

Needed:

- metrics-server integration
- usage vs. requests vs. limits comparison
- over/under-provisioning indicators

## Gap R: No configuration drift detection

Current issue:

- no way to see what changed since last declarative apply
- drift diagnosis requires manual kubectl diff

Needed:

- live vs. last-applied diff view
- noise-filtered rendering

## Gap S: No custom action extensibility

Current issue:

- all actions are built-in; teams cannot add custom workflows
- users must leave TUI for team-specific operations

Needed:

- config-based plugin system for custom actions
- variable substitution with resource context

## Gap T: No preventive misconfiguration detection

Current issue:

- Issue Center catches runtime problems but not latent misconfigurations
- best-practice violations go unnoticed until they cause incidents

Needed:

- rule-based resource sanitizer
- health report view

## Gap U: NetworkPolicy is incomprehensible

Current issue:

- NetworkPolicy YAML is hard to reason about
- no way to answer "can pod A reach pod B?"

Needed:

- effective policy visualization per pod
- connectivity query tool

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

Completed

### What shipped

- Dashboard computation cache keyed on snapshot_version (DashboardCache): stats, alerts, insights, sparkline, and pod status counts computed once per snapshot instead of every frame
- Zero-alloc fast path for truncate_label via Cow<'_, str> (common case: no truncation)
- Pod log buffer reduced from 50k to 10k lines (~80% memory reduction per log tab)
- Exponential backoff for probe polling: 2s base, doubles after 3 no-change polls, caps at 30s, resets on change/error
- Config save debouncing: 5 direct save_config() calls replaced with dirty flag + 1-second Instant-based debounce + final flush on exit
- Filter cache MRU fast path: get_mru() compares by &str against last key without allocating, avoiding String allocation on repeated same-query lookups
- Criterion benchmark suite: dashboard compute (stats/alerts/insights/workload_ready at 100/500/2000 pods), filter/sort (pod indices with hit/miss/no-filter), format helpers (format_age)
- Detail view API calls parallelized with tokio::join! (YAML + events + metrics run concurrently instead of sequentially, ~2x faster detail opens)
- Background context connect: TLS handshake spawned in background task, UI shows loading state immediately instead of freezing 500ms-2s
- Two-wave refresh merged into concurrent execution: core and secondary resources now fetch simultaneously via tokio::join!(wave1, wave2), saving ~150-300ms per full refresh

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

## Milestone 16: Secret Decoded View & Editor

### Status

Not started

### Goal

Eliminate the base64 encode/decode friction that plagues every K8s operator working with Secrets.

### Why this matters

Managing Secrets requires `kubectl get secret -o jsonpath | base64 --decode` per field, manual re-encoding with `echo -n | base64`, and careful YAML formatting. This is error-prone (newline bugs, encoding mistakes) and tedious for Secrets with many keys. No TUI tool handles this well — this is a clear differentiator.

### Scope

- automatic base64 decoding of Secret data fields in detail/YAML view
- dedicated "Decoded" tab in workbench alongside raw YAML
- inline editing of decoded values with automatic re-encoding on save
- masking toggle (show/hide decoded values for shoulder-surfing protection)

### Tasks

1. Add Secret detection in detail view: when resource is a Secret, offer a "Decoded" workbench tab.
2. Implement base64 decode pass on all `data` fields, display as key-value pairs.
3. Add masking toggle (`m` key) to show/hide decoded values (default: masked with `****`).
4. Add decoded value editing: select a key, edit value, auto-encode back to base64 on save.
5. Handle edge cases: binary data (show hex preview), empty values, invalid base64.
6. Add action palette integration: "View decoded secrets" action for Secret resources.
7. Update help overlay with new keybindings.
8. Add tests for decode/encode round-trip, masking, binary detection, edge cases.

### Deliverables

- frictionless Secret inspection and editing

### Risks

- displaying decoded secrets in plain text (mitigated by masking default)
- binary data that doesn't decode to valid UTF-8

### Guardrails

- default to masked view to prevent accidental exposure
- clearly indicate when a value contains binary data
- re-encode must produce byte-identical output for unchanged values

### Acceptance Criteria

- users can inspect Secret values without leaving the TUI or running base64 commands
- editing a Secret value and saving produces correct base64 encoding
- shoulder-surfing protection is on by default

---

## Milestone 17: Resource Bookmarks

### Status

Not started

### Goal

Let users pin critical resources for instant access in large clusters.

### Why this matters

In clusters with hundreds of resources, engineers repeatedly navigate to the same handful of critical deployments, configmaps, or services. Currently this requires navigating through the sidebar hierarchy each time. Lens users cite "quickly getting to my stuff" as a key UX advantage. k9s GitHub issues (#3595) surface the same pain point — people want to refocus on resources of interest without full navigation.

### Scope

- bookmark any resource from detail view or list view
- dedicated Bookmarks view accessible from sidebar and action palette
- per-cluster bookmark persistence (using existing preferences system)
- one-key jump to bookmarked resource

### Tasks

1. Add `BookmarkEntry` model: resource kind, name, namespace, cluster context, timestamp.
2. Add bookmark storage to `ClusterPreferences` (persisted per-cluster).
3. Add `B` keybinding in list/detail view to toggle bookmark on selected resource.
4. Add Bookmarks sidebar entry under a new "Pinned" group (or top of Overview group).
5. Render bookmarks as a table: kind icon, name, namespace, age since bookmarked.
6. Enter on a bookmark navigates to the resource's view and selects it (or opens detail).
7. Add action palette entries: "Bookmark resource", "View bookmarks".
8. Add bookmark indicator in list views (subtle icon/marker on bookmarked resources).
9. Add tests for bookmark CRUD, persistence round-trip, navigation.

### Deliverables

- instant access to critical resources across sessions

### Risks

- bookmark staleness (resource deleted but bookmark persists)
- UI clutter if users bookmark too many resources

### Guardrails

- indicate stale bookmarks (resource not found) with dimmed/strikethrough styling
- cap bookmarks at 50 per cluster to prevent list bloat
- one-key remove from bookmarks view

### Acceptance Criteria

- users can bookmark a resource, close the app, reopen, and jump to it instantly
- stale bookmarks are clearly indicated, not silently broken

---

## Milestone 18: CronJob/Job Management Panel

### Status

Not started

### Goal

Provide a unified view connecting CronJobs to their execution history, status, and logs.

### Why this matters

CronJob management (database backups, cleanup scripts, report generation) is a daily task for operations teams. The current app can trigger CronJobs and view jobs as list views, but there is no unified view linking a CronJob to its execution history. Engineers currently piece together `kubectl get jobs --selector`, find the pod, then check logs — a multi-step navigation that a dedicated panel collapses into one screen.

### Scope

- CronJob detail panel showing execution history (last N jobs)
- per-job status, duration, pod count, and completion percentage
- next scheduled run time (parsed from cron expression)
- one-key access to failed job's pod logs
- suspend/resume CronJob toggle

### Tasks

1. Add cron expression parser for next-run-time display (use `cron` crate or manual parser).
2. Add Job-to-CronJob linking via ownerReferences in existing snapshot data.
3. Add "History" section in CronJob detail view: table of child Jobs sorted by creation time.
4. Show per-job: status (Complete/Failed/Active), duration, completions (succeeded/total), pod count.
5. Add `Enter` on a history job row to jump to that Job's detail (and its pod logs).
6. Add suspend/resume toggle (`S` key) via PATCH to `.spec.suspend`.
7. Add next-run-time display in CronJob detail header.
8. Record suspend/resume in action history.
9. Add action palette: "Suspend CronJob", "Resume CronJob".
10. Update help overlay.
11. Add tests for cron parsing, job linking, suspend/resume lifecycle.

### Deliverables

- unified CronJob observability and management

### Risks

- cron expression edge cases (non-standard extensions)
- large job history for frequently-running CronJobs

### Guardrails

- cap displayed history to 20 most recent jobs
- cron parser handles standard 5-field expressions; show "N/A" for unparseable expressions
- suspend/resume uses standard confirmation pattern

### Acceptance Criteria

- users can see a CronJob's recent execution history, identify failures, and access logs without manual kubectl
- suspend/resume is a one-key operation with confirmation

---

## Milestone 19: Ephemeral Debug Container Launcher

### Status

Not started

### Goal

Support modern Kubernetes debugging for distroless and minimal container images.

### Why this matters

Production containers increasingly use distroless or scratch-based images that lack shells and debugging tools. `kubectl debug` with ephemeral containers (stable since K8s 1.25) is the modern answer, but the command syntax is verbose. This is critical for debugging networking issues (netshoot gives tcpdump, curl, nslookup in-cluster). It complements the existing exec/shell capability for cases where exec fails because no shell exists.

### Scope

- ephemeral debug container dialog with image preset picker
- common debug images: busybox, nicolaka/netshoot, alpine, ubuntu
- optional process namespace sharing for process-level debugging
- launch directly into shell session in the ephemeral container
- reuse existing exec/shell workbench tab infrastructure

### Tasks

1. Add `DebugContainerDialog` state model with image selection and options.
2. Add preset image picker: busybox (lightweight), netshoot (networking), alpine (general), ubuntu (full), custom.
3. Add process namespace targeting toggle (share PID namespace with target container).
4. Implement ephemeral container creation via Kubernetes API (pods/ephemeralcontainers subresource).
5. After container is running, open exec session into the debug container (reuse ExecTabState).
6. Add `D` keybinding in pod detail view to launch debug dialog (when pod is running).
7. Add action palette: "Debug container" for pod resources.
8. Record debug container creation in action history.
9. Update help overlay.
10. Add tests for dialog state, API payload construction, image presets.

### Deliverables

- first-class debugging for minimal/distroless containers

### Risks

- ephemeral containers require K8s 1.25+ (feature gate check needed)
- debug container images may not be pullable in restricted environments

### Guardrails

- detect K8s version and show clear error if ephemeral containers are unsupported
- allow custom image input for restricted registries
- warn user that debug containers persist until pod restart

### Acceptance Criteria

- users can attach a debug container to a running pod and get a shell in under 5 seconds
- networking debugging (tcpdump, curl) works via netshoot preset

---

## Milestone 20: Helm Release History & Rollback

### Status

Not started

### Goal

Make Helm rollback a one-key operation during incidents.

### Why this matters

Helm rollback is one of the most time-critical operations during incidents. Currently engineers must leave the TUI, run `helm history`, identify the target revision, then run `helm rollback`. The app already lists Helm releases but cannot act on them beyond viewing. Adding history + rollback makes KubecTUI a viable incident response tool.

### Scope

- revision history panel for any Helm release
- per-revision: revision number, chart version, app version, status, timestamp, description
- one-key rollback to a selected revision with confirmation
- values diff between any two revisions

### Tasks

1. Add Helm history fetching: shell out to `helm history <release> -n <namespace> --output json`.
2. Add `HelmHistoryTabState` for workbench with revision table.
3. Render revision table: revision, chart, app version, status, updated, description.
4. Add `Enter` on a revision to show values diff against current revision.
5. Implement values diff: `helm get values --revision N` for both revisions, compute unified diff.
6. Add rollback action: `R` on a selected revision → confirmation dialog → `helm rollback <release> <revision>`.
7. Record rollback in action history.
8. Add action palette: "Helm history", "Helm rollback".
9. Add diff rendering in workbench (reuse or extend YAML tab with diff highlighting).
10. Update help overlay.
11. Add tests for revision parsing, diff computation, rollback command construction.

### Deliverables

- incident-speed Helm rollback from inside the TUI

### Risks

- requires `helm` CLI available on PATH
- Helm 2 vs Helm 3 compatibility (Helm 2 is EOL; target Helm 3 only)

### Guardrails

- detect `helm` availability at startup; disable Helm actions if missing
- rollback confirmation shows what will change (current revision → target revision)
- Helm 3 only; show clear error message for Helm 2 clusters

### Acceptance Criteria

- users can identify a bad release, view its history, and rollback in under 30 seconds
- values diff helps users confirm which revision to rollback to

---

## Milestone 21: Resource Utilization Overlay

### Status

Not started

### Goal

Surface actual CPU/memory usage alongside requests and limits to enable right-sizing decisions.

### Why this matters

Right-sizing is consistently the #1 cost optimization lever in Kubernetes. Teams overspend 2-3x on compute because resource requests are set-and-forget. The dashboard shows node-level gauges, but there is no pod-level usage-vs-requests comparison. Engineers currently need external tools (kubectl top, Prometheus, KRR) to identify waste.

### Scope

- per-pod CPU/memory usage columns (actual vs. requested vs. limit)
- per-node utilization summary (allocated vs. capacity vs. actual)
- visual indicators: color-coded usage bars or percentage with threshold coloring
- namespace-level aggregation view (total requested vs. actual)

### Tasks

1. Add metrics-server API client (`metrics.k8s.io/v1beta1` for PodMetrics and NodeMetrics).
2. Detect metrics-server availability; gracefully degrade if unavailable.
3. Add usage columns to pods table: CPU (actual/request/limit), Memory (actual/request/limit).
4. Add usage overlay to nodes table: CPU allocated%, Memory allocated%, actual usage%.
5. Color-code usage: green (<70%), yellow (70-90%), red (>90% of request/limit).
6. Add namespace utilization summary in dashboard: total CPU/mem requested vs. actual.
7. Add "Top Pods" quick view: pods sorted by CPU or memory usage (like `kubectl top pods`).
8. Periodic refresh of metrics data (every 30s, separate from main resource refresh).
9. Add columns to column registry (hideable, default off to avoid clutter for non-metrics clusters).
10. Update help overlay and action palette.
11. Add tests for metrics parsing, color threshold logic, graceful degradation.

### Deliverables

- inline resource utilization visibility without leaving the TUI

### Risks

- metrics-server not installed in all clusters
- metrics data is point-in-time, not averaged (can be misleading)
- additional API calls increase cluster load

### Guardrails

- metrics columns default to hidden; users opt in via column toggle
- clearly label as "current" not "average" to set correct expectations
- 30s refresh interval, not every frame
- graceful "No metrics available" when metrics-server is absent

### Acceptance Criteria

- users can identify over/under-provisioned pods at a glance
- node utilization is visible without running `kubectl top nodes`
- works silently when metrics-server is absent (no errors, columns just hidden)

---

## Milestone 22: Resource Diff View (Live vs. Last Applied)

### Status

Not started

### Goal

Detect configuration drift by showing what changed since the last declarative apply.

### Why this matters

Configuration drift is a top-3 operational concern. When something breaks, the first question is "what changed?" The `kubectl.kubernetes.io/last-applied-configuration` annotation exists on most resources but no TUI tool surfaces this as a diff. ArgoCD does this for GitOps-managed resources, but many resources are managed via `kubectl apply` or Helm outside of GitOps.

### Scope

- diff between live resource state and last-applied-configuration annotation
- unified diff rendering with add/remove/change highlighting
- noise filtering: exclude auto-managed fields (resourceVersion, generation, managedFields, status)
- available from detail view and action palette

### Tasks

1. Extract `last-applied-configuration` annotation from resource YAML.
2. Parse both live and last-applied into structured form.
3. Implement field-level diff with noise filtering (exclude system-managed fields).
4. Render unified diff in a workbench tab with green/red highlighting.
5. Add `D` keybinding in detail view (or `d` if available) to open diff tab.
6. Handle resources without last-applied annotation: show "No baseline — resource may be managed by Helm, server-side apply, or created imperatively."
7. Add action palette: "View config drift" / "Diff live vs. applied".
8. Update help overlay.
9. Add tests for diff computation, field filtering, annotation parsing, edge cases.

### Deliverables

- instant configuration drift detection

### Risks

- last-applied annotation may be absent (server-side apply uses managedFields instead)
- diff noise from Kubernetes-managed fields

### Guardrails

- filter system fields aggressively to reduce noise
- support both client-side apply (annotation) and server-side apply (managedFields) where feasible
- clearly indicate when no baseline is available

### Acceptance Criteria

- users can answer "has this resource been manually edited?" in one keypress
- drift is clearly highlighted with minimal noise

---

## Milestone 23: Plugin / Custom Action System

### Status

Not started

### Goal

Let teams extend the action palette with custom operational workflows.

### Why this matters

This is k9s's most powerful extensibility feature and consistently cited as why power users stick with k9s. Every team has custom workflows — opening a resource in Grafana, running diagnostic scripts, triggering CI pipelines. No two teams' workflows are the same. The action palette already provides the UI framework; plugins extend the action catalog.

### Scope

- YAML config file defining custom actions
- variable substitution: `$NAME`, `$NAMESPACE`, `$KIND`, `$CONTEXT`, `$LABELS`
- resource type filtering (action only appears for matching resources)
- execution modes: background (capture output), foreground (terminal handoff), silent (fire and forget)
- custom keyboard shortcuts (optional)

### Tasks

1. Define plugin config schema: `~/.config/kubectui/plugins.yaml` (or `plugins/` directory).
2. Implement plugin loader: parse YAML, validate schema, register actions.
3. Add plugin actions to action palette filtered by resource type.
4. Implement variable substitution engine with shell-safe escaping.
5. Add three execution modes:
   - `background`: run command, capture output in workbench tab
   - `foreground`: hand off terminal (like existing YAML edit)
   - `silent`: run command, show success/failure in status bar
6. Add plugin output workbench tab for background mode.
7. Record plugin executions in action history.
8. Hot-reload plugins on config file change (watch with notify).
9. Ship example plugins: "Open in Grafana", "Copy connection string", "Run diagnostic".
10. Add tests for config parsing, variable substitution, resource type matching, execution modes.

### Deliverables

- team-customizable operational workflows without app modifications

### Risks

- arbitrary command execution (security concern)
- plugin config errors causing crashes

### Guardrails

- plugins run with user's permissions only (no privilege escalation)
- invalid plugin configs logged and skipped, not fatal
- confirmation dialog for destructive-tagged plugins
- document security model clearly

### Acceptance Criteria

- a team can add a custom "Open in Grafana" action that appears for Deployments/StatefulSets
- plugin actions feel native — same palette, same history, same keybinding system

---

## Milestone 24: Resource Sanitizer / Best Practice Linter

### Status

Not started

### Goal

Catch latent misconfigurations in resources that appear healthy but violate best practices.

### Why this matters

80% of Kubernetes incidents stem from misconfigurations, not infrastructure failures. The Issue Center detects runtime problems (CrashLoopBackOff, pending pods). A sanitizer catches _latent_ problems — resources that are running fine today but are one restart away from trouble. This is k9s's "Popeye" integration — one of its most distinctive features that no other TUI replicates.

### Scope

- configurable rule set scanning deployed resources
- categories: resource limits, probes, security context, image tags, PDB coverage, naked pods, port mismatches
- dedicated "Health Report" view with per-resource severity scores
- integration with Issue Center (sanitizer findings appear alongside runtime issues)

### Tasks

1. Define rule engine model: `SanitizeRule` with category, severity, resource matcher, check function.
2. Implement core rules:
   - Missing CPU/memory requests or limits
   - Missing liveness/readiness probes
   - Running as root (securityContext.runAsNonRoot not set)
   - Using `:latest` tag or no tag
   - hostNetwork/hostPID/hostIPC enabled
   - Missing PodDisruptionBudget for Deployments with replicas > 1
   - Naked pods (no owning controller)
   - Service port → container port mismatches
   - Unused ConfigMaps/Secrets (not referenced by any pod)
3. Add "Health Report" sidebar view with findings table: severity, category, resource, message.
4. Add per-resource sanitizer badge in list views (optional, hideable column).
5. Compute findings from snapshot data (no new API calls); cache by snapshot_version.
6. Allow rule suppression via annotations (`kubectui.io/ignore: latest-tag,no-probes`).
7. Add action palette: "View health report", "Sanitize cluster".
8. Update help overlay.
9. Add tests for each rule, annotation suppression, caching, edge cases.

### Deliverables

- preventive misconfiguration detection

### Risks

- false positives creating noise and alert fatigue
- rule maintenance burden

### Guardrails

- start with high-confidence rules only (no speculative checks)
- all rules must be individually suppressible
- severity levels: critical (security), warning (reliability), info (best practice)
- cap findings at 500 to match Issue Center

### Acceptance Criteria

- users can identify misconfigured resources before they cause incidents
- false positive rate is low enough that the feature is trusted

---

## Milestone 25: Network Policy Visualizer

### Status

Not started

### Goal

Make NetworkPolicy debugging comprehensible — show who can talk to whom.

### Why this matters

NetworkPolicy YAML is notoriously hard to reason about. Label selectors, namespace selectors, the implicit "deny all if any policy selects you" behavior, and overlapping policies create a mental model that even experienced engineers struggle with. No TUI tool offers NetworkPolicy visualization. Web-based tools (networkpolicy.io, Cilium editor) are disconnected from live cluster state.

### Scope

- per-pod effective policy view: which policies apply, what traffic is allowed/denied
- per-namespace isolation summary: default deny status, policy count
- ingress/egress rule breakdown with resolved pod/namespace targets
- text-based connectivity graph (reuse relationship explorer rendering)
- "Can pod A reach pod B?" query tool

### Tasks

1. Implement NetworkPolicy selector resolution: match policies to pods via label selectors.
2. Compute effective ingress/egress rules per pod from all matching policies.
3. Resolve peer selectors to actual pods/namespaces for concrete display.
4. Render per-pod policy view as tree: Policy → Direction (Ingress/Egress) → Rule → Peers + Ports.
5. Add namespace isolation summary: "Default: Allow All" vs "Default: Deny (N policies active)".
6. Add "Can reach?" dialog: select source pod and target pod, show allow/deny verdict with explaining policy.
7. Use relationship explorer tree rendering for policy → pod connectivity graph.
8. Add action palette: "View network policies", "Check connectivity".
9. Add `N` keybinding in pod detail for quick policy view.
10. Update help overlay.
11. Add tests for selector matching, isolation computation, multi-policy resolution, edge cases.

### Deliverables

- comprehensible NetworkPolicy debugging without external tools

### Risks

- NetworkPolicy spec complexity (especially with egress and namespace selectors)
- CNI plugins may not enforce NetworkPolicies (tool shows policy intent, not enforcement)

### Guardrails

- clearly state "shows policy intent, not CNI enforcement" in the UI
- handle clusters with no NetworkPolicies gracefully ("No network policies — all traffic allowed")
- focus on readability over completeness; complex multi-policy scenarios get simplified view

### Acceptance Criteria

- users can answer "why can't pod A reach pod B?" from within the TUI
- NetworkPolicy rules are displayed in human-readable form, not raw YAML selectors

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
| 16 Secret Decoded View | 0, 2 | needs YAML view infrastructure and detail model |
| 17 Resource Bookmarks | 14 | needs persistence infrastructure |
| 18 CronJob/Job Management | 0, 1 | needs workbench and action model |
| 19 Ephemeral Debug Container | 4 | needs exec/shell session infrastructure |
| 20 Helm History & Rollback | 0, 3 | needs action history and workbench |
| 21 Resource Utilization | 0 | needs metrics API integration |
| 22 Resource Diff View | 0, 2 | needs YAML view infrastructure |
| 23 Plugin / Custom Actions | 9 | needs action palette framework |
| 24 Resource Sanitizer | 12 | builds on issue center model |
| 25 Network Policy Visualizer | 10 | builds on relationship explorer infrastructure |

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

## P4 (Next)

- Milestone 16: Secret Decoded View & Editor
- Milestone 17: Resource Bookmarks
- Milestone 18: CronJob/Job Management Panel

## P5

- Milestone 19: Ephemeral Debug Container Launcher
- Milestone 20: Helm Release History & Rollback
- Milestone 21: Resource Utilization Overlay

## P6

- Milestone 22: Resource Diff View
- Milestone 23: Plugin / Custom Action System
- Milestone 24: Resource Sanitizer / Best Practice Linter

## P7

- Milestone 25: Network Policy Visualizer

## Continuous

- Milestone 15: Performance Track remains continuous and parallel, not deferred

---

## What We Should Start Right Now

M0-M14 are complete. The next milestone is M16: Secret Decoded View & Editor — the highest-impact, lowest-effort feature that every K8s operator will use daily.

Do not start next with:

- AI features
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
- plugin config schema
- sanitizer rule engine

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
- secret decoded tab
- health report view
- network policy tree

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
- metrics-server polling
- helm CLI execution
- ephemeral container creation

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
- bookmark persistence
- plugin config loading

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
- extension marketplace (plugin system in M23 is config-based, not a marketplace)
- desktop-like window choreography
- visually complex graph canvases
- integrated Prometheus metric graphs (sparklines possible in later milestone)
- Helm chart installation workflow (M20 covers history/rollback, not installation)
- CSV export of table views
- batch operations (multi-select delete, scale, restart)

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
- close daily workflow friction (secrets, bookmarks, cronjob management)
- unlock modern debugging patterns (ephemeral containers, Helm rollback, utilization)
- add drift detection and extensibility (diff view, plugins, sanitizer)
- complete network visibility (policy visualizer)
- preserve speed at every step

This milestone plan is now the canonical implementation roadmap.
