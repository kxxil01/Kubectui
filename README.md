# KubecTUI

A fast, keyboard-driven terminal UI for Kubernetes. Browse resources, inspect timelines and relationships, stream logs, exec into pods, port-forward, scale workloads, manage CronJobs, and perform day-2 operations without leaving your terminal.

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
- **Bottom workbench** with 9 persistent tab types — Action History, YAML, Decoded Secret, Timeline, Pod Logs, Workload Logs, Exec, Port-Forward, and Relations
- **Action palette** (`:`) — unified fuzzy search for navigation, column toggles, and context-aware resource actions
- **Permission-aware UX** — list/detail actions are filtered by resource capability and available RBAC; forbidden list/metrics/discovery reads degrade gracefully instead of poisoning the UI
- **Pod exec/shell** — terminal sessions with container picker and shell fallback order (bash → sh → busybox)
- **Real-time log streaming** with follow mode, previous logs, timestamp toggle, search/highlight, and multi-container picker
- **Workload-level logs** — aggregate logs across all pods of a Deployment, StatefulSet, DaemonSet, ReplicaSet, ReplicationController, or Job with per-pod/container/text filtering
- **Port-forwarding** via kube-rs — no `kubectl` binary required
- **Scale deployments** and StatefulSets directly from the detail view or action palette
- **Rollout restart** for Deployments, StatefulSets, and DaemonSets
- **YAML editing** — press `e` to open supported resource YAML in `$EDITOR`, apply changes on save
- **Resource deletion** — press `d` to delete supported resources with confirmation, `F` for force delete when available
- **CronJob management panel** — next-run/last-run state, capped Job execution history, selected-run log access, manual trigger, and pause/resume on `S`
- **Decoded Secret inspection/editing** — open decoded Secret values in the workbench, edit inline, and save with automatic base64 re-encode
- **Health probe inspector** — view liveness, readiness, and startup probe configs per container
- **Relationship explorer** — jump owner chains, service backends, ingress backends, storage bindings, RBAC bindings, and Flux lineage from `w`
- **Issue Center** — problem-first view across cluster failures and degraded resources
- **Bookmarks** — persistent per-context bookmarks with a dedicated Bookmarks view and `B` toggle
- **Node operations** — cordon, uncordon, and drain with confirmation and action-history tracking
- **Clipboard integration** — `Ctrl+y` copies resource name, `Y` copies namespace/name, `y` in logs copies content
- **Log export** — `S` in log tabs saves buffer to file
- **Action history** with pending/success/error tracking and resource jump-back
- **Helm release browser** — reads Helm v3 releases from cluster secrets
- **FluxCD support** — browse all Flux resources, trigger reconcile, deep reconciliation detail (full conditions, revisions, generation sync, source ref, Stalled detection), Reconcile column with relative time, generation mismatch indicator
- **5 color themes** — Dark (default), Nord, Dracula, Catppuccin Mocha, Light — cycle with `T`, persist in config
- **Multi-context switching** at startup and runtime
- **Namespace filtering** across all views
- **Fuzzy search** (`/`) on every resource list
- **Dashboard** with cluster health gauges, alerts, and workload summaries
- **Network resilience** — connection health indicator (●/◐/○), graceful backoff on API failures, staleness indicator, manual refresh bypass, error truncation
- **UI/UX polish** — loading spinners, sort direction colors, persistent search bar with result count, YAML syntax highlighting, toast notifications, detail metadata expand/collapse
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
| `y` | Open YAML in workbench | All resources with YAML access |
| `o` | Open decoded Secret tab | Secrets |
| `v` | Open timeline/events in workbench | Supported namespaced resources |
| `l` / `L` | Open log viewer | Pods and supported workloads; CronJobs use the selected history row |
| `x` | Open exec/shell session | Pods |
| `f` | Open port-forward dialog | Pods |
| `p` | Open probe inspector | Pods |
| `s` | Open scale dialog | Deployments, StatefulSets |
| `R` | Rollout restart | Deployments, StatefulSets, DaemonSets |
| `R` | Flux reconcile | FluxCD resources |
| `e` | Edit YAML in `$EDITOR` | Supported resources when YAML is loaded |
| `d` | Delete resource (with confirmation) | Supported resources |
| `F` | Force delete (in delete confirmation) | Supported delete targets |
| `T` | Trigger CronJob as a new Job | CronJobs |
| `S` | Pause/resume CronJob | CronJobs |
| `w` | Open relations tab | Relationship-capable resources |
| `B` | Toggle bookmark | List/detail resource contexts |
| `m` | Toggle metadata expand/collapse | All |
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
> Tip: permission-gated actions are also filtered by your current RBAC when KubecTUI can determine it.

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
- **RBAC-aware behavior**: if your kubeconfig cannot list or mutate a resource type, KubecTUI now prefers hiding or disabling that action instead of failing after the UI pivots
- **Search is live**: `/` filters the current list as you type — no need to press Enter
- **Ctrl+U** clears the search query while in search mode
- **Previous logs**: press `P` in a pod log tab to view logs from crashed/restarted containers
- **Workload logs**: press `l` on a Deployment/StatefulSet to aggregate logs from all its pods
- **All Containers**: in pod logs picker, select "All Containers" to stream all container logs together
- **CronJobs**: use `j`/`k` in CronJob detail to select a historical Job, `Enter` to jump into that Job, `l` for its logs, `T` to trigger, and `S` to pause/resume
- **Helm releases**: navigate to Helm → Releases to see all Helm v3 releases in the cluster
- **Metrics**: CPU/memory metrics require `metrics-server` to be installed in the cluster
- **Restart vs Scale**: use `R` for a rolling restart (zero-downtime), use `s` to change replica count
- **Auto-refresh**: cluster data refreshes every 30 seconds. Customize via `refresh_interval_secs` in config

---

## License

MIT
