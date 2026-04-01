# Changelog

## 2026-04-01

Planning archive sync.

### Archived Roadmap Record

- M0-M5 established the canonical interaction model: bottom workbench, action history, pod exec, and workload-level logs.
- M6-M14 closed core operator UX gaps: help/discoverability, clipboard/export, richer detail surfaces, action palette, relationships, node ops, issue center, timeline, and persisted workspace/view preferences.
- M15-M25 delivered the major operational platform layers: render/scale hardening, decoded Secret editing, bookmarks, CronJob control, ephemeral debug containers, Helm history/rollback, utilization, drift diffing, extensions plus AI hooks, sanitizer/health reporting, and NetworkPolicy analysis.
- M26-M37 completed the broader operator workspace: rollout control, advanced log investigation, hotkeys/workspaces/banks, vulnerability center, resource create/apply templates, service/traffic debugging, node debug shell, release hardening, projects, Gateway API workflows, guided runbooks, and governance/cost rollups.

### Post-Roadmap Follow-Ups

- 2026-04-01 M39: shipped global resource search in the command palette across kind/name/namespace/labels, plus recent-activity switching across workbench tabs, action history, and recent jumps.
- 2026-03-29 render-frame skip pass: unchanged header, sidebar, status, and main-content regions now reuse the canonical render path when inputs are stable, with regression coverage for theme, icon mode, search, and selection invalidation.
- 2026-03-29 perf-gate hardening: `scripts/perf_gate.sh` now falls back to folded profile data when `header` or `status` spans are omitted from `render-frame-summary.txt`.
- 2026-03-29 empty-state consistency cleanup: remaining dashboard and RBAC/security empty states now use the canonical centered empty-state renderer instead of manual padding.

### Validation Archive

- Latest local verification pass on 2026-04-01:
  - `cargo fmt --all`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --all-targets --all-features`
  - `cargo test --test performance benchmark_search_keystroke_under_5ms -- --ignored --nocapture`
- Latest local verification pass on 2026-03-29:
  - `cargo fmt --all`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --all-targets --all-features`
  - `scripts/perf_gate.sh`

## 2026-03-27

Roadmap completion release.

### Highlights

- Added Helm release history, computed-values diff, and rollback in the workbench
- Added rollout control for Deployments, StatefulSets, and DaemonSets with restart, pause/resume, and undo
- Added advanced log investigation with presets, regex mode, time windows, jump-to-time, JSON summaries, and correlation
- Added saved workspaces, banks, and configurable hotkeys
- Added Health Report, sanitizer findings, and a Trivy-backed vulnerability center
- Added NetworkPolicy intent analysis, pod reachability checks, and service / ingress traffic debugging
- Added AI-assisted resource analysis plus specialized workflows for failure explanation, rollout risk, network verdicts, and finding triage
- Added resource drift view, ephemeral debug containers, node debug shell, and built-in apply templates

### Validation

- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- Performance gates stayed positive across the milestone series, including the final M23 phase 3 comparison:
  - baseline `render=252.801ms`
  - candidate `render=251.295ms`

### Notes

- Live-cluster smoke behavior is still the main remaining external validation gap for the newest surfaces.
