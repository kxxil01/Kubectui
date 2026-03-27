# Changelog

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
