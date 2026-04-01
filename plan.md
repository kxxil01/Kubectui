# KubecTUI Milestone Plan

## Status

This file is the canonical source of truth for active product and implementation priority.

When this document conflicts with older notes, ad hoc ideas, or chat suggestions, this document wins.

This plan does not aim to copy Lens, OpenLens, or Freelens visually. It adopts the workflow patterns that make those tools easy to learn, then translates them into a terminal-native Rust + `ratatui` application.

## Implementation Progress

- Roadmap delivery through M39 is complete.
- M40 through M43 plus deferred M38 are the active expansion backlog.
- Phase 8 (watch-backed caches) is complete.
- Historical shipped scope, milestone archive, and completed priority records now live in [CHANGELOG.md](CHANGELOG.md).

Recent shipped follow-ups after roadmap completion:

- 2026-04-01 M39 shipped: the command palette now supports global resource search by kind/name/namespace/labels plus recent-activity switching across workbench tabs, action history, and recent jumps.
- 2026-03-29 render-frame skip pass: the canonical render path now skips unchanged header, sidebar, status, and main-content regions when render inputs are stable, with same-terminal invalidation coverage for theme, icon mode, search, and selection.
- 2026-03-29 perf-gate hardening: `scripts/perf_gate.sh` now falls back to the folded profile for `header` and `status` when those spans drop out of `render-frame-summary.txt`.
- 2026-03-29 empty-state consistency pass: the remaining dashboard and RBAC/security empty states now use the canonical centered empty-state path instead of manual left-padding strings.
- Latest local verification on 2026-04-01: `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features`, and `cargo test --test performance benchmark_search_keystroke_under_5ms -- --ignored --nocapture` all pass.

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

These rules apply to every active milestone.

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

## Current Product Position

KubecTUI already has:

- 46 resource views across the current sidebar groups with responsive layout and shared sorting/filtering
- a persistent workbench with logs, exec, timeline, YAML, decoded secrets, relations, traffic, rollout, and action-history workflows
- palette-driven navigation and action dispatch with RBAC-aware gating
- dashboard, issue center, health report, vulnerability view, governance view, and project/application scopes
- workload, node, Helm, Flux, NetworkPolicy, traffic-debug, and gateway-aware operator tooling
- persisted preferences for namespace, theme, icon mode, columns, sorting, bookmarks, workspaces, and banks
- canonical render-path scale work already shipped: row windowing, derived caches, watch-backed refresh, and frame-skip rendering

The remaining highest-leverage gaps are:

- multi-cluster operator flow is still second-class
- resource/activity finding is still too view-local
- RBAC troubleshooting lacks a first-class access review surface
- the extension substrate is not yet packaged and inspectable as a product feature
- synthetic endpoint validation and local sandbox control are still script- or operator-driven outside the app

---

## Active Roadmap Strategy

The original milestone roadmap is complete through M37. This document now tracks roadmap expansion only.

Active sequencing rule:

- continue `M40 -> M41 -> M42 -> M43 -> M38` unless a materially higher-leverage operator gap appears
- do not rebuild completed surfaces on parallel paths when the canonical workbench/action/detail flow already exists
- keep new work snapshot-based, keyboard-first, and within the current performance budget

Do not start next with:

- autonomous/general AI features outside the shipped M23 extension path
- visual graph experiments that do not fit TUI constraints
- desktop-style interaction patterns
- heavyweight cloud-cost or billing integrations before multi-cluster and RBAC leverage land

---

## Remaining Backlog Priority Order

The next set should optimize for operator workflow leverage first, then cross-cluster clarity, then secure extensibility, while preserving the current performance budget.

### Big Win Priority Order

- M40: RBAC Access Review & Reverse Lookup
- M41: Extension Packaging & Catalog
- M42: Synthetic Service Checks & Benchmarks
- M43: Local Sandbox Cluster Control
- M38: Multi-Cluster Workspaces & Compare

### Why this order

- M40 follows because the product now has broad operational power but still lacks a first-class “what can this subject do / why is this denied” surface that operators expect on shared clusters.
- M41 comes next because the extension substrate already exists; the next leverage is making it installable, inspectable, and shareable without weakening the canonical runtime path.
- M42 and M43 stay ahead of multi-cluster because they add bounded operator leverage without forcing a cross-cutting state-model expansion.
- M38 moves to the end because it is the broadest state and lifecycle expansion left. It should land after the single-cluster findability, authz, extension, and local-validation surfaces are stronger and more stable.

### Next Milestones

#### M40: RBAC Access Review & Reverse Lookup

- Status: not started
- Big win: high
- Why:
  - the app now depends heavily on correct authz behavior and tri-state checks, but it still lacks a direct surface for operators to inspect effective access.
  - this is one of the highest-leverage troubleshooting workflows for shared clusters.
- Scope:
  - “what can I do” for the current identity in namespace or cluster scope
  - reverse lookup from ServiceAccount/User/Group to Roles, ClusterRoles, and Bindings
  - denied-action explanation surface tied back to the canonical detail/palette policy path
  - explicit namespace-vs-cluster auth boundaries
- Guardrails:
  - no speculative auth modeling outside Kubernetes authz APIs and the existing snapshot
  - single canonical source of truth for action requirements must remain `authorization.rs`
  - fail clearly on clusters where self-subject review APIs are unavailable

#### M41: Extension Packaging & Catalog

- Status: not started
- Big win: medium-high
- Why:
  - the extension/runtime substrate is already complete; the missing step is making those extensions easier to package, inspect, install, and share.
  - this compounds the value of M23 without changing the execution model.
- Scope:
  - packaged extension manifests with metadata, versioning, and validation
  - local catalog/registry view for installed extensions and AI actions
  - enable/disable, trust, and config inspection on the canonical extension path
  - import/export workflow for local extension bundles
- Guardrails:
  - no network marketplace in the first pass
  - no alternate extension runtime; reuse the shipped command/AI action substrate
  - strong validation and explicit trust boundaries before execution

#### M42: Synthetic Service Checks & Benchmarks

- Status: not started
- Big win: medium
- Why:
  - KubecTUI already explains traffic intent and routing, but operators still need a fast “does this endpoint actually respond” surface for day-2 checks.
  - K9s-style benchmark and connectivity workflows remain useful when kept bounded and explicit.
- Scope:
  - bounded HTTP/TCP synthetic checks from local operator context
  - response summary, latency, status-code, and basic retry result surfacing
  - optional short benchmark mode for selected services/endpoints
  - action-history and workbench integration on the existing traffic-debug path
- Guardrails:
  - strictly opt-in; no background probes
  - bounded runtime and output
  - no confusion between local synthetic checks and in-cluster dataplane truth

#### M43: Local Sandbox Cluster Control

- Status: not started
- Big win: medium
- Why:
  - the product now ships disposable-cluster smoke tooling, but it remains script-driven rather than operator-facing.
  - local sandbox control would tighten the development and validation loop without touching production workflows.
- Scope:
  - detect and manage local `kind`/lightweight sandbox clusters
  - launch/recreate/delete sandbox clusters from a guarded workflow
  - connect smoke automation and release validation to that local-cluster surface
  - explicit safeguards so local actions never target remote contexts
- Guardrails:
  - local-only in the first pass
  - no remote managed-cluster lifecycle actions
  - fail closed when the current context is not clearly a supported local sandbox

#### M38: Multi-Cluster Workspaces & Compare

- Status: not started
- Big win: highest
- Why:
  - KubecTUI now has strong single-cluster workflows, but operators still need to pivot between prod/staging/dev or between regional clusters quickly.
  - current workspaces and banks are cluster-local jumps, not a first-class multi-cluster operating surface.
  - this is now intentionally sequenced last because it is the broadest remaining state-model and lifecycle change.
- Scope:
  - multi-cluster workspace banks and recent-cluster switching
  - side-by-side compare for selected overview/resource surfaces where data is structurally comparable
  - explicit cluster badges throughout navigation, workbench, and action history
  - cluster-scoped saved jumps without cross-cluster state leakage
- Guardrails:
  - no hidden parallel refresh storm across every kubeconfig context
  - no second state tree separate from the existing snapshot/workbench model
  - compare views must stay bounded, text-first, and keyboard-native

### Recommended Remaining Order

- M39 -> M40 -> M41 -> M42 -> M43 -> M38

### Research Basis

This ordering is informed by the workflows emphasized in active operator tools and docs:

- K9s plugin, XRay, shell, and workflow emphasis
- Lens/Freelens command palette, logs, terminal, overview, and hotbar workflows
- Lens teamwork and access-sharing workflows
- Headlamp multi-cluster, plugin, and activity/task-oriented workflows
- KubecTUI's existing strengths: workbench, action history, Helm rollback, health/sanitizer, network policy analysis

The goal is not parity for its own sake. The goal is to invest next in the workflows that most reduce operator round-trips while staying fast and terminal-native.
