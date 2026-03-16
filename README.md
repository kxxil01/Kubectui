# KubecTUI

A keyboard-driven terminal UI for Kubernetes operations.

![Rust](https://img.shields.io/badge/rust-1.85+-orange)
![License](https://img.shields.io/badge/license-MIT-blue)
[![CI](https://github.com/kxxil01/Kubectui/actions/workflows/ci.yml/badge.svg)](https://github.com/kxxil01/Kubectui/actions/workflows/ci.yml)

## Install

```bash
# Homebrew
brew install kxxil01/tap/kubectui

# From source
git clone https://github.com/kxxil01/Kubectui.git
cd Kubectui && cargo build --release
./target/release/kubectui
```

Requires a kubeconfig at `~/.kube/config` (or `KUBECONFIG` env var).

## What it does

**Browse** 46 resource views across 9 sidebar groups — Pods, Deployments, Services, Secrets, RBAC, FluxCD, Helm, CRDs, and more. The sidebar auto-collapses to show only the active group.

**Operate** without leaving the terminal — exec into pods, stream logs, port-forward, scale, rollout restart, delete, edit YAML in `$EDITOR`, trigger CronJobs, cordon/drain nodes, and reconcile Flux resources.

**Inspect** with a bottom workbench that holds persistent tabs — YAML, decoded Secrets, timelines, pod/workload logs, exec sessions, port-forwards, and relationship graphs.

**Monitor** via a dashboard with health gauges, CPU/memory utilization, namespace breakdown, top consumers, and alerts. Node metrics display in human-readable units (`4.2Gi/13.0Gi`). Live watch streams keep 10 core resources updated in real-time.

## Key features

| Category | Details |
|----------|---------|
| **Resources** | 46 views, CRD instances, Helm v3 releases, FluxCD with deep reconciliation detail |
| **Actions** | Exec, logs, port-forward, scale, restart, delete, YAML edit, CronJob trigger/pause |
| **Workbench** | 9 tab types: History, YAML, Decoded Secret, Timeline, Pod Logs, Workload Logs, Exec, Port-Forward, Relations |
| **Dashboard** | 5 gauges, overcommitment panel, namespace utilization, top-5 CPU/memory pods, alerts |
| **Search** | `/` filters any list live. `:` opens the action palette for fuzzy navigation + resource actions |
| **RBAC-aware** | Actions filtered by resource capability and cluster permissions; forbidden reads degrade gracefully |
| **Themes** | Dark, Nord, Dracula, Catppuccin Mocha, Light — cycle with `T` |
| **Resilience** | Connection health indicator, backoff on failures, staleness display, watch auto-reconnect |

## Keybindings

Press `?` in-app for the full list. Summary:

### Navigation

| Key | Action |
|-----|--------|
| `j`/`k` or `↑`/`↓` | Move up/down |
| `Enter` | Select item / open detail |
| `Tab` / `Shift+Tab` | Next / previous view |
| `Esc` | Close current overlay or detail |

### Global

| Key | Action |
|-----|--------|
| `/` | Search (filters as you type) |
| `:` | Action palette (fuzzy search for views + actions) |
| `~` | Namespace picker (type to filter, arrows to navigate) |
| `c` | Context switcher (type to filter, arrows to navigate) |
| `r` | Refresh data |
| `T` | Cycle theme |
| `b` | Toggle workbench |
| `H` | Action history |
| `Ctrl+y` / `Y` | Copy name / namespace+name |
| `q` | Quit |

### Resource actions

Press `Enter` on a resource, then:

| Key | Action | Resources |
|-----|--------|-----------|
| `y` | YAML | All |
| `l` / `L` | Logs | Pods, workloads, CronJobs |
| `x` | Exec/shell | Pods |
| `f` | Port-forward | Pods |
| `s` | Scale | Deployments, StatefulSets |
| `R` | Rollout restart / Flux reconcile | Workloads / FluxCD |
| `d` | Delete | Supported resources |
| `e` | Edit in `$EDITOR` | When YAML is loaded |
| `w` | Relationships | Relationship-capable resources |
| `B` | Bookmark | All |
| `o` | Decoded view | Secrets |
| `p` | Probe inspector | Pods |
| `T` / `S` | Trigger / Pause | CronJobs |

### Workbench

| Key | Action |
|-----|--------|
| `[` / `]` | Switch tabs |
| `Ctrl+W` | Close tab |
| `z` | Maximize |
| `Ctrl+↑`/`↓` | Resize |

### Log viewer

| Key | Action |
|-----|--------|
| `g` / `G` | Top / bottom |
| `f` | Follow mode |
| `P` | Previous logs (crashed containers) |
| `t` | Timestamps |
| `/` then `n`/`N` | Search + next/prev match |
| `y` / `S` | Copy / export to file |

## CLI

```
kubectui [OPTIONS]

  --theme <name>          dark | nord | dracula | catppuccin | light
  --profile-render        Enable render profiling
  --profile-output <dir>  Profile output directory
  -V, --version           Version
  -h, --help              Help
```

## Tips

- `:` discovers all available actions for the current resource — no need to memorize keys
- `/` search is live — filters as you type, `Ctrl+U` clears
- Sidebar groups auto-collapse — only the active group is expanded. `Enter` on a header temporarily expands it
- `l` on a Deployment/StatefulSet aggregates logs from all its pods
- `P` in log viewer shows previous logs from crashed containers
- CPU/memory metrics require `metrics-server` in the cluster
- Data auto-refreshes every 30s (configurable via `refresh_interval_secs` in config)

## License

MIT
