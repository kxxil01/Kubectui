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
- Milestone 4: next

Completion notes:

- Milestone 0 shipped the canonical policy layer and aligned UI hints, keybindings, and runtime action guards.
- Milestone 1 shipped the bottom workbench foundation with persisted open state, persisted height, tab management, focus handling, and layout integration.
- Milestone 2 moved YAML, events, logs, and port-forward sessions onto the workbench-backed path and removed the old duplicate detail-only inspection path.
- Milestone 3 added a canonical action history model, recorded pending/success/error mutation state centrally, and exposed the verification surface in the workbench with jump-back to affected resources where possible.

Verification status for completed milestones:

- local build/test/performance gates are passing
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

- broad resource coverage
- workload-first loading and background hydration
- per-view loading state
- responsive layouts
- shared sorting across workloads and more non-workload views
- bottom workbench with persistent tabs
- workbench-backed YAML, events, logs, and port-forward sessions
- logs, port-forwarding, scaling, restart, YAML apply, delete
- Flux reconcile
- standardized mutation lifecycle
- optimistic updates for safe operations
- action history and recent mutation verification
- command palette
- context and namespace switching
- CRD browsing

This means the next phase should focus on workflow depth, discoverability, and operator quality of life.

---

## Gaps To Close

These are the main gaps between the current app and a strong operator workspace.

## Gap A: No timeline and correlation surface

Current issue:

- users can verify recent actions, but not yet in a timeline-oriented or correlated way
- action history exists, but it is still separate from event chronology

Needed:

- event/action correlation
- timeline-oriented verification
- faster understanding of what changed after an action

## Gap B: Weak multi-resource workflows

Current issue:

- many operations are single-resource only

Needed:

- multi-pod logs
- workload-level debugging flows
- relationship-driven navigation

## Gap C: No integrated exec / shell workflow

Current issue:

- users still need to leave the app for common pod shell operations

Needed:

- pod exec/session support
- terminal-like workbench surface

## Gap D: Weak relationship navigation

Current issue:

- resources are browsable by kind, but not by dependency chain

Needed:

- service -> endpoint -> pod
- deployment -> replicaset -> pod
- ingress -> service -> pod
- PVC -> PV -> StorageClass
- Flux source -> downstream resource chain

## Gap E: Weak issue-centered workflow

Current issue:

- users must think in resources more than problems

Needed:

- issue center
- grouped problem categories
- action-oriented issue drilldown

## Gap F: Weak persistent personalization

Current issue:

- users cannot fully shape and retain their workspace

Needed:

- per-view saved sort
- per-view columns
- persisted workbench state
- context-aware preferences

## Gap G: Missing high-value node operations

Current issue:

- inspection exists, operations do not

Needed:

- cordon
- uncordon
- drain

## Gap H: Weak action verification timeline

Current issue:

- users can trigger actions and inspect resources, but there is no timeline-oriented verification surface yet

Needed:

- events near the action
- action history
- resource-level verification surfaces

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

### Why this exists

The app has already grown quickly. Before adding a workbench and more operational features, the app should have one clear source of truth for shared policies.

### Scope

- audit current cross-view behavior
- identify legacy or duplicate state paths
- centralize capability policies where needed
- document and normalize existing global behaviors

### Tasks

1. Audit current canonical policies:
   - shared sort capabilities
   - loading behavior
   - mutation lifecycle
   - detail open/close behavior
2. Add missing capability tables where needed:
   - action capability table
   - persistence capability table
   - relationship capability table
3. Remove or mark duplicate flows that would conflict with workbench migration.
4. Make footer hints and action availability fully consistent with centralized policies.

### Deliverables

- centralized policy helpers
- reduced ad hoc branching
- documented constraints for future milestones

### Risks

- widening scope too much into refactoring with no visible user value

### Guardrail

Keep this milestone tightly scoped to policy cleanup, not feature invention.

### Acceptance Criteria

- one canonical policy path exists for core cross-view behavior
- no obvious conflicting UI paths remain for the features being migrated next

---

## Milestone 1: Workbench Foundation

### Status

Completed

### Goal

Create a persistent bottom workbench that becomes the home for long-lived operational surfaces.

### Why this comes first

Workbench is the enabling layer for:

- logs without losing list context
- YAML without losing list context
- events per resource
- action history
- pod exec
- future long-lived operational tabs

Without this, later features will create more modal and overlay duplication.

### Scope

- bottom workbench layout
- open/close behavior
- tab strip
- active tab switching
- persisted height/open state
- canonical state model

### Tasks

1. Add `WorkbenchState` to canonical app state.
2. Add `WorkbenchTabKind`.
3. Add `WorkbenchTab`.
4. Add reducer/input flows for:
   - open tab
   - close tab
   - focus next/previous tab
   - resize workbench
   - toggle workbench open/closed
5. Add `ratatui` layout support for:
   - sidebar
   - main content
   - bottom workbench
   - footer
6. Add canonical workbench rendering with a real empty state and dynamic tab content.
7. Persist:
   - workbench open state
   - workbench height
8. Add tests for:
   - tab creation
   - tab close
   - tab focus
   - layout behavior
   - persistence restore

### Out of Scope

- pod exec
- multi-pod logs
- relationship explorer
- issue center

### Deliverables

- visible bottom workbench
- reusable tab state model
- no performance regression in standard list views

### Risks

- introducing render churn across the whole app
- adding duplicate focus models
- making keyboard navigation confusing

### Guardrails

- inactive tabs must be cheap
- workbench open/close must not rebuild unrelated state
- focus behavior must be explicit and testable

### Acceptance Criteria

- users can open and close the workbench reliably
- workbench height is adjustable and persisted
- switching tabs is immediate
- no visible lag is introduced in core list navigation

---

## Milestone 2: Migrate Existing Operational Surfaces Into Workbench

### Status

Completed

### Goal

Move existing high-value inspection tools into the workbench so the app stops relying on scattered blocking experiences.

### Scope

- logs
- YAML
- events
- port-forward sessions

### Tasks

1. Move single-pod logs into a workbench tab.
2. Move YAML inspection into a workbench tab.
3. Add resource-scoped events tab support.
4. Add a port-forward session tab or session list tab.
5. Make open-from-detail and open-from-list flow into the same workbench-backed path.
6. Reduce or remove duplicate long-term overlay behavior where appropriate.
7. Add tests for:
   - opening from list
   - opening from detail
   - reopening existing tab vs spawning duplicate tab
   - closing tab while list remains interactive

### Deliverables

- logs in workbench
- YAML in workbench
- events in workbench
- port-forward session visibility in workbench

### Risks

- duplicated code paths between modal and workbench implementations
- log buffering affecting render performance
- too many tabs creating memory pressure

### Guardrails

- bounded buffers only
- tab reuse policy must be explicit
- expensive content should be cached or isolated from frame-by-frame rebuilds

### Acceptance Criteria

- users can keep browsing while logs or YAML remain visible
- events are accessible near the selected resource
- the workbench becomes the canonical place for ongoing operational inspection

---

## Milestone 3: Action History and Verification Surface

### Status

Completed

### Goal

Give users a first-class place to understand what happened after they triggered an action.

### Scope

- action history tab
- pending/success/error status
- timestamps
- resource jump-back

### Tasks

1. Add `ActionHistoryEntry` model.
2. Record all mutating actions centrally:
   - delete
   - scale
   - restart
   - reconcile
   - YAML apply
3. Render action history in workbench.
4. Link history entries back to resources where possible.
5. Show progress transitions where the action model supports them.
6. Add tests for:
   - entry recording
   - entry lifecycle transitions
   - clearing stale entries policy

### Deliverables

- workbench action history tab
- visible mutation verification path

### Risks

- action history becoming noisy or spammy
- duplicate status reporting between footer and history

### Guardrails

- footer is for current status
- workbench history is for recent history
- do not overload one mechanism with both roles

### Acceptance Criteria

- after a mutation, users can see a durable record of what happened
- users can jump from a recent action back to the affected resource

---

## Milestone 4: Pod Exec / Shell

### Goal

Add a terminal-native exec workflow for pods and containers.

### Scope

- exec into selected pod
- container selection
- shell fallback order
- shell session hosted in workbench

### Tasks

1. Add exec session state model.
2. Support container selection for multi-container pods.
3. Attempt shells in order:
   - `/bin/bash`
   - `/bin/sh`
   - `/busybox/sh`
4. Render exec session in a terminal-oriented workbench tab.
5. Handle errors clearly:
   - pod not running
   - container not ready
   - exec forbidden
   - shell missing
6. Add tests for:
   - state transitions
   - unsupported pod state
   - failure messaging

### Deliverables

- exec workflow available from pods
- shell sessions live in workbench

### Risks

- terminal emulation complexity
- input routing conflicts with app navigation

### Guardrails

- isolate session I/O from the main render path
- keep scrollback bounded
- make focus rules explicit

### Acceptance Criteria

- users can open a pod shell without leaving KubecTUI
- failure states are obvious and actionable

---

## Milestone 5: Multi-Pod and Workload-Level Logs

### Goal

Make workload debugging first-class.

### Scope

- aggregate logs by workload
- label lines by pod and container
- filters and follow mode

### Tasks

1. Add workload log session model.
2. Resolve workload -> pod set reliably.
3. Stream logs concurrently from selected pods.
4. Prefix lines with source identity.
5. Add filters:
   - pod
   - container
   - text
6. Add follow/pause controls.
7. Handle partial failure gracefully.
8. Add tests for:
   - multiplex ordering policy
   - partial stream failure
   - session stop/cleanup

### Deliverables

- workload-level logs for deployments, statefulsets, daemonsets, and similar selectors where safe

### Risks

- high memory use
- line flood overwhelming render path
- pod churn while streaming

### Guardrails

- bounded line buffers
- capped active streams if needed
- explicit refresh/re-resolve behavior when pod membership changes

### Acceptance Criteria

- users can inspect workload logs without opening each pod individually
- partial failures do not kill the entire experience

---

## Milestone 6: Action Palette v2

### Goal

Turn the command palette into the main discoverability and action surface.

### Scope

- verb-driven actions
- context-aware availability
- fuzzy search over actions and targets

### Tasks

1. Expand command palette into action palette.
2. Add action catalog:
   - logs
   - exec
   - scale
   - restart
   - reconcile
   - delete
   - YAML
   - related resources
   - switch namespace/context
   - toggle columns
3. Show disabled actions with reasons where useful.
4. Make action palette aware of current selection and current view.
5. Add tests for:
   - action availability
   - disabled reasoning
   - fuzzy action matching

### Deliverables

- action palette as a primary user-facing control surface

### Risks

- action list becoming too noisy
- conflicting shortcuts and palette actions

### Guardrails

- palette should enhance discoverability, not duplicate random internal actions
- every direct action shortcut should also map to a palette entry

### Acceptance Criteria

- users can find most core workflows from the palette
- disabled actions explain why they are unavailable

---

## Milestone 7: Relationship Explorer

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

## Milestone 8: Issue Center

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

## Milestone 9: View Personalization and Workspace Persistence

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

## Milestone 10: Node Operations

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

## Milestone 11: Timeline and Event Correlation

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

## Milestone 12: Performance and Scale Track

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
| 6 Action Palette v2 | 0 | needs capability tables and stable action model |
| 7 Relationship Explorer | 0, 1 | benefits from centralized capability policy and workbench |
| 8 Issue Center | 0 | needs canonical issue model |
| 9 Persistence | 1, 2, 6 | should persist stable user workflows, not temporary ones |
| 10 Node Operations | 0, 3 | depends on action lifecycle and verification surfaces |
| 11 Timeline | 2, 3 | depends on workbench events/history |
| 12 Performance Track | all | parallel quality track across all milestones |

---

## Implementation Priority

This is the execution order.

## P0

- Milestone 0
- Milestone 1
- Milestone 2
- Milestone 3

## P1

- Milestone 4
- Milestone 5
- Milestone 6
- Milestone 7

## P2

- Milestone 8
- Milestone 9
- Milestone 10
- Milestone 11

## P3

- Milestone 12 remains continuous and parallel, not deferred

---

## What We Should Start Right Now

Start with Milestone 4.

That means the next real implementation work should be:

1. add the canonical exec/session state model
2. host pod/container shell sessions in the workbench
3. define focus and input routing for live terminal sessions
4. handle shell fallback, permission errors, and unsupported pod/container states
5. keep session buffers bounded and isolated from the normal render path

Do not start next with:

- AI features
- plugin systems
- batch actions
- new one-off resource views
- visual graph experiments
- node drain first
- milestone 5+ before milestone 4 is complete

Those are later priorities.

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

Rules:

- design first
- test state transitions
- keep one canonical source of truth

## Type B: UI Layout Tasks

These change rendering and navigation surfaces.

Examples:

- workbench bottom pane
- tab strip
- timeline view
- issue center rendering

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

Rules:

- no ambiguous focus
- every input path must have obvious exit behavior

## Type D: Async Workflow Tasks

These involve background tasks or streaming data.

Examples:

- logs
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

They should not distract from the milestone order above.

---

## External Product References

These are workflow references, not UI templates:

- Lens docs: [https://docs.k8slens.dev/](https://docs.k8slens.dev/)
- Lens repo: [https://github.com/lensapp/lens](https://github.com/lensapp/lens)
- Freelens repo: [https://github.com/freelensapp/freelens](https://github.com/freelensapp/freelens)
- Lens extensions repo: [https://github.com/lensapp/lens-extensions](https://github.com/lensapp/lens-extensions)

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

- build the workbench
- migrate long-lived operational flows into it
- strengthen action verification
- add exec, workload logs, relationships, and issues
- preserve speed at every step

This milestone plan is now the canonical implementation roadmap.
