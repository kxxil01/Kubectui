# KubecTUI

A fast, keyboard-driven terminal UI for Kubernetes. Browse resources, stream logs, exec into pods, port-forward, scale workloads, inspect probes, and trigger rolling restarts — all without leaving your terminal.

![Rust](https://img.shields.io/badge/rust-1.85+-orange)
![License](https://img.shields.io/badge/license-MIT-blue)
[![CI](https://github.com/kxxil01/Kubectui/actions/workflows/ci.yml/badge.svg)](https://github.com/kxxil01/Kubectui/actions/workflows/ci.yml)

---

## Installation

### Homebrew (macOS / Linux)

```bash
brew install kxxil01/tap/kubectui
```

### Build from source

```bash
git clone https://github.com/kxxil01/Kubectui.git
cd Kubectui
cargo build --release
./target/release/kubectui
```

### Prerequisites

- A kubeconfig at `~/.kube/config` (or `KUBECONFIG` env var)

---

## Features

- **46 resource views** across 9 sidebar groups — Pods, Deployments, StatefulSets, DaemonSets, Jobs, CronJobs, Services, Endpoints, Ingresses, ConfigMaps, Secrets, HPAs, PVCs, PVs, StorageClasses, RBAC, Events, Namespaces, FluxCD resources, and more
- **Custom Resource Definitions** — browse CRDs, drill into instances, view full YAML via dynamic API
- **Bottom workbench** with 7 persistent tab types — YAML, Events, Pod Logs, Workload Logs, Exec, Port-Forward, Action History
- **Action palette** (`:`) — unified fuzzy search for navigation and context-aware resource actions (logs, exec, scale, restart, delete, etc.)
- **Pod exec/shell** — terminal sessions with container picker and shell fallback order (bash → sh → busybox)
- **Real-time log streaming** with follow mode, previous logs, timestamp toggle, search/highlight, and multi-container picker
- **Workload-level logs** — aggregate logs across all pods of a Deployment, StatefulSet, or DaemonSet with per-pod/container/text filtering
- **Port-forwarding** via kube-rs — no `kubectl` binary required
- **Scale deployments** and StatefulSets directly from the detail view or action palette
- **Rollout restart** for Deployments, StatefulSets, and DaemonSets
- **YAML editing** — press `e` to open resource YAML in `$EDITOR`, apply changes on save
- **Resource deletion** — press `d` to delete with confirmation, `F` for force delete (stuck finalizers)
- **CronJob manual trigger** — press `T` to create a Job from CronJob spec
- **Health probe inspector** — view liveness, readiness, and startup probe configs per container
- **Clipboard integration** — `Ctrl+y` copies resource name, `Y` copies namespace/name, `y` in logs copies content
- **Log export** — `S` in log tabs saves buffer to file
- **Action history** with pending/success/error tracking and resource jump-back
- **Helm release browser** — reads Helm v3 releases from cluster secrets
- **FluxCD support** — browse all Flux resources, trigger reconcile from detail or palette
- **5 color themes** — Dark (default), Nord, Dracula, Catppuccin Mocha, Light — cycle with `T`, persist in config
- **Multi-context switching** at startup and runtime
- **Namespace filtering** across all views
- **Fuzzy search** (`/`) on every resource list
- **Dashboard** with cluster health gauges, alerts, and workload summaries
- **Configuration persistence** — namespace, theme, workbench state, refresh interval

---

## Keybindings

### Navigation

| Key | Action |
|-----|--------|
| `j` / `↓` | Move down (sidebar or list) |
| `k` / `↑` | Move up (sidebar or list) |
| `Enter` | Activate sidebar item / open resource detail |
| `Tab` | Next resource view |
| `Shift+Tab` | Previous resource view |
| `Esc` | Close overlay → close detail → return to sidebar |

### Global

| Key | Action |
|-----|--------|
| `?` | Open help overlay (all keybindings) |
| `~` | Open namespace picker |
| `c` | Open context switcher |
| `:` | Open action palette (navigate + resource actions) |
| `/` | Enter search mode — type to filter the list |
| `r` | Refresh all resource data |
| `Ctrl+y` | Copy resource name to clipboard |
| `Y` | Copy namespace/name to clipboard |
| `T` | Cycle color theme |
| `b` | Toggle workbench |
| `H` | Open action history |
| `q` | Quit (asks for confirmation) |

### Detail View

Press `Enter` on any resource to open its detail view.

| Key | Action | Applies to |
|-----|--------|------------|
| `y` | Open YAML viewer | All |
| `v` | Open events viewer | All |
| `l` / `L` | Open log viewer | Pods, Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs |
| `x` | Open exec/shell session | Pods |
| `f` | Open port-forward dialog | Pods |
| `p` | Open probe inspector | Pods |
| `s` | Open scale dialog | Deployments, StatefulSets |
| `R` | Rollout restart | Deployments, StatefulSets, DaemonSets |
| `R` | Flux reconcile | FluxCD resources |
| `e` | Edit YAML in `$EDITOR` | All (when YAML is loaded) |
| `d` | Delete resource (with confirmation) | All |
| `F` | Force delete (in delete confirmation) | All |
| `T` | Trigger CronJob as a new Job | CronJobs |
| `:` | Open action palette | All |
| `Esc` | Close detail view | All |

### Action Palette

Press `:` from anywhere to open. Shows context-aware resource actions when a resource is selected or detail is open.

| Key | Action |
|-----|--------|
| Type | Filter by action name or view name |
| `↑` / `↓` | Navigate results |
| `Enter` | Execute action or navigate to view |
| `Esc` | Close |

> Tip: type `scl` to find Scale, `lg` for Logs, `dep` for Deployments, etc. Actions are filtered by what the current resource supports.

### Workbench

| Key | Action |
|-----|--------|
| `b` | Toggle workbench open/close |
| `[` / `]` | Previous / next workbench tab |
| `Ctrl+W` | Close active workbench tab |
| `z` | Toggle maximize workbench |
| `Ctrl+↑` / `Ctrl+↓` | Resize workbench |

### Log Viewer

| Key | Action |
|-----|--------|
| `j` / `↓` | Scroll down one line |
| `k` / `↑` | Scroll up one line |
| `g` | Jump to top |
| `G` | Jump to bottom |
| `f` | Toggle follow mode |
| `P` | Toggle previous logs (crashed containers) |
| `t` | Toggle timestamps |
| `/` | Search within logs |
| `n` / `N` | Next / previous search match |
| `y` | Copy log content to clipboard |
| `S` | Export logs to file |
| `Esc` | Close log viewer |

### Port-Forward Dialog

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Move between fields |
| `Enter` | Create tunnel |
| `F2` | Switch to tunnel list view |
| `Esc` | Close dialog |

### Scale Dialog

| Key | Action |
|-----|--------|
| `0`–`9` | Type desired replica count |
| `+` / `-` | Increment / decrement by 1 |
| `Backspace` | Delete last digit |
| `Enter` | Apply scale |
| `Esc` | Cancel |

### Probe Inspector

| Key | Action |
|-----|--------|
| `j` / `↓` | Select next container |
| `k` / `↑` | Select previous container |
| `Space` | Expand/collapse container probe details |
| `Esc` | Close inspector |

---

## CLI Options

```
kubectui [OPTIONS]

  --theme <name>          Set color theme (dark, nord, dracula, catppuccin, light)
  --profile-render        Enable render profiling (frame timings + folded stacks)
  --profile-output <dir>  Profile output directory (default: target/profiles)
  --version, -V           Show version
  --help, -h              Show help
```

---

## Tips

- **Action palette**: use `:` to discover all available actions for the current resource — no need to memorize shortcuts
- **Quick jump**: `:` + type `dep` to jump to Deployments, `pod` for Pods, `svc` for Services
- **Search is live**: `/` filters the current list as you type — no need to press Enter
- **Ctrl+U** clears the search query while in search mode
- **Previous logs**: press `P` in a pod log tab to view logs from crashed/restarted containers
- **Workload logs**: press `l` on a Deployment/StatefulSet to aggregate logs from all its pods
- **All Containers**: in pod logs picker, select "All Containers" to stream all container logs together
- **Helm releases**: navigate to Helm → Releases to see all Helm v3 releases in the cluster
- **Metrics**: CPU/memory metrics require `metrics-server` to be installed in the cluster
- **Restart vs Scale**: use `R` for a rolling restart (zero-downtime), use `s` to change replica count
- **Auto-refresh**: cluster data refreshes every 30 seconds. Customize via `refresh_interval_secs` in config

---

## License

MIT
