# KubecTUI

A fast, keyboard-driven terminal UI for Kubernetes. Browse resources, stream logs, port-forward, scale workloads, inspect probes, and trigger rolling restarts — all without leaving your terminal.

![Rust](https://img.shields.io/badge/rust-1.93.1+-orange)
![License](https://img.shields.io/badge/license-MIT-blue)

---

## Features

- **35 resource views** — Pods, Deployments, StatefulSets, DaemonSets, Jobs, CronJobs, Services, Endpoints, Ingresses, ConfigMaps, Secrets, HPAs, PVCs, PVs, StorageClasses, RBAC, Events, Namespaces, and more
- **Custom Resource Definitions** — browse CRDs, drill into instances, view full YAML via dynamic API
- **Real-time log streaming** with follow mode, line-number display, and multi-container picker
- **Port-forwarding** via kube-rs — no `kubectl` binary required
- **Scale deployments** directly from the detail view
- **Rollout restart** for Deployments, StatefulSets, and DaemonSets
- **YAML editing** — press `e` to open resource YAML in `$EDITOR`, apply changes on save
- **Resource deletion** — press `d` to delete any resource with a confirmation prompt
- **Health probe inspector** — view liveness/readiness configs per container
- **Helm release browser** — reads Helm v3 releases from cluster secrets
- **Helm repository viewer** — reads local Helm repo config from filesystem
- **5 color themes** — Dark (default), Nord, Dracula, Catppuccin Mocha, Light — cycle with `T`, persist in config
- **Multi-context switching** at startup and runtime
- **Namespace filtering** across all views
- **Fuzzy search** on every resource list
- **Command palette** for quick navigation

---

## Installation

### Prerequisites

- Rust 1.93.1+
- A kubeconfig at `~/.kube/config` (or `KUBECONFIG` env var)

### Build from source

```bash
git clone https://github.com/kxxil01/Kubectui.git
cd Kubectui
cargo build --release
./target/release/kubectui
```

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

### Sidebar

| Key | Action |
|-----|--------|
| `j` / `k` | Move sidebar cursor |
| `Enter` | Expand/collapse group or navigate to view |

> Tip: focus switches automatically to the content list when you select a view. Press `Esc` to return focus to the sidebar.

### Content list

| Key | Action |
|-----|--------|
| `j` / `k` | Select next/previous row |
| `Enter` | Open detail view for selected resource |
| `/` | Enter search mode — type to filter the list |
| `Esc` (in search) | Clear search and exit search mode |
| `r` | Refresh all resource data |

### Global

| Key | Action |
|-----|--------|
| `~` | Open namespace picker |
| `c` | Open context switcher |
| `:` | Open command palette |
| `T` | Cycle color theme (dark → nord → dracula → catppuccin) |
| `q` | Quit (asks for confirmation) |
| `Esc` | Cancel quit confirmation / close overlays |

---

## Detail View

Press `Enter` on any resource to open its detail view. The detail view shows metadata, status, resource-specific info, metrics (where available), and a YAML preview.

### Detail view keybindings

| Key | Action | Applies to |
|-----|--------|------------|
| `l` / `L` | Open log viewer | Pods |
| `f` | Open port-forward dialog | Pods |
| `s` | Open scale dialog | Deployments, StatefulSets |
| `p` | Open probe inspector | Pods |
| `R` | Rollout restart | Deployments, StatefulSets, DaemonSets |
| `e` | Edit YAML in `$EDITOR` | All (when YAML is loaded) |
| `d` | Delete resource (with confirmation) | All |
| `Esc` | Close detail view | All |

---

## Log Viewer

Open with `l` from a Pod detail view.

| Key | Action |
|-----|--------|
| `j` / `↓` | Scroll down one line |
| `k` / `↑` | Scroll up one line |
| `g` | Jump to top |
| `G` | Jump to bottom |
| `f` | Toggle follow mode (auto-scroll to new lines) |
| `Esc` | Close log viewer |

> Tip: follow mode streams new log lines in real time. Toggle it off with `f` to freely scroll history.

---

## Port-Forward Dialog

Open with `f` from a Pod detail view. Pre-fills namespace and pod name automatically.

| Key | Action |
|-----|--------|
| `Tab` | Move to next field |
| `Shift+Tab` | Move to previous field |
| `Enter` | Create tunnel |
| `F2` | Switch to tunnel list view |
| `Esc` | Close dialog |

**In tunnel list view:**

| Key | Action |
|-----|--------|
| `j` / `↓` | Select next tunnel |
| `k` / `↑` | Select previous tunnel |
| `d` | Stop selected tunnel |
| `r` | Refresh tunnel list |
| `F1` | Switch back to create form |
| `Esc` | Close dialog |

> Tip: set local port to `0` to auto-assign a free port.

---

## Scale Dialog

Open with `s` from a Deployment or StatefulSet detail view.

| Key | Action |
|-----|--------|
| `0`–`9` | Type desired replica count |
| `+` | Increment by 1 |
| `-` | Decrement by 1 |
| `Backspace` | Delete last digit |
| `Enter` | Apply scale |
| `Esc` | Cancel |

> Tip: a warning appears if you change replicas by more than 10 at once. Valid range is 0–100.

---

## Probe Inspector

Open with `p` from a Pod detail view. Shows liveness and readiness probe configs for each container.

| Key | Action |
|-----|--------|
| `j` / `↓` | Select next container |
| `k` / `↑` | Select previous container |
| `Space` | Expand/collapse container probe details |
| `Esc` | Close inspector |

---

## Rollout Restart

Press `R` (shift+r) from a Deployment, StatefulSet, or DaemonSet detail view. Triggers a rolling restart by patching the pod template annotation — equivalent to `kubectl rollout restart`. No confirmation prompt; takes effect immediately.

---

## Namespace Picker

Press `~` from anywhere to open the namespace picker.

| Key | Action |
|-----|--------|
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `Enter` | Select namespace |
| `Esc` | Cancel |

Select `all` to show resources across all namespaces.

---

## Context Switcher

Press `c` from the main view (or select at startup if multiple contexts exist).

| Key | Action |
|-----|--------|
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `Enter` | Switch to selected context |
| `Esc` | Cancel |

---

## Command Palette

Press `:` to open. Type any resource name to jump directly to that view.

| Key | Action |
|-----|--------|
| Type | Filter views by name |
| `↑` / `↓` | Navigate results |
| `Enter` | Navigate to selected view |
| `Esc` | Close |

---

## Themes

KubecTUI ships with 5 color themes. Press `T` (shift+t) from the main view to cycle through them. The current theme is shown in the status bar.

| Theme | Description |
|-------|-------------|
| `dark` | GitHub-inspired deep dark (default) |
| `nord` | Arctic, north-bluish color palette |
| `dracula` | Dark with vibrant purple/pink accents |
| `catppuccin` | Warm dark with pastel accents (Mocha variant) |
| `light` | Clean, high-contrast light background |

Your theme choice is saved automatically to `~/.config/kubectui/config.json` and restored on next launch.

### CLI flag

```bash
kubectui --theme nord
```

Valid values: `dark`, `nord`, `dracula`, `catppuccin`, `mocha`, `light`.

---

## Tips

- **Quick jump**: use `:` + type `dep` to jump straight to Deployments, `pod` for Pods, etc.
- **Search is live**: `/` filters the current list as you type — no need to press Enter.
- **Ctrl+U** clears the search query while in search mode.
- **Helm releases**: navigate to Helm → Releases to see all Helm v3 releases in the cluster. No Helm CLI needed.
- **Metrics**: CPU/memory metrics in the detail view require `metrics-server` to be installed in the cluster. If unavailable, a note is shown instead of an error.
- **YAML view**: every resource detail view includes a YAML preview at the bottom — useful for quick inspection without switching to a terminal.
- **Restart vs Scale**: use `R` for a rolling restart (zero-downtime pod replacement), use `s` to change the replica count.
- **Auto-refresh**: cluster data refreshes automatically every 30 seconds. Customize via `refresh_interval_secs` in `~/.kube/kubectui-config.json` (set to `0` to disable).

---

## License

MIT
