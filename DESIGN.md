# KubecTUI - Kubernetes Cluster Explorer
## Product Design & Implementation Plan

---

## 🎯 Vision

A **fast**, **beautiful**, **intuitive** terminal UI for exploring and managing Kubernetes clusters. No bloat, no friction, just pure productivity.

**Core Promise:** See your entire K8s cluster at a glance. Navigate. Debug. Deploy. All in the terminal.

---

## 📋 Feature Roadmap

### Phase 1: MVP (Week 1-2)
**Goal:** Cluster overview + resource browsing

- [ ] **Dashboard View**
  - Cluster name, API server, Kubernetes version
  - Node count, Pod count, Service count
  - Health status (ready nodes, running pods)
  - CPU/Memory summary (if metrics-server available)

- [ ] **Resource Browsers**
  - Nodes (list, sort by name/status/capacity)
  - Pods (namespace-filtered, list by status)
  - Services (list, show port mappings)
  - Deployments (list, show replicas)

- [ ] **Navigation**
  - Tab-based UI (Dashboard | Nodes | Pods | Services | Deployments)
  - Arrow keys to navigate lists
  - Search/filter within each tab
  - Enter to drill down into resource details

- [ ] **Detail View**
  - YAML preview (syntax highlighted)
  - Resource metadata (name, namespace, age, labels)
  - Event logs (if available)

### Phase 2: Intelligence (Week 3-4)
**Goal:** Smart diagnostics & debugging

- [ ] **Smart Filtering**
  - Filter by namespace, label selector, status
  - Saved filter profiles

- [ ] **Health Indicators**
  - Pod crash detection
  - Node pressure warnings (MemoryPressure, DiskPressure)
  - Resource saturation alerts
  - CrashLoopBackOff indicators

- [ ] **Quick Actions**
  - View pod logs (tail + follow)
  - Port forward (interactive tunnel setup)
  - Describe resource (kubectl describe output)
  - Delete resource (with confirmation)

- [ ] **Metrics Dashboard**
  - CPU/Memory usage per node
  - Pod resource requests vs actual usage
  - Network I/O if available

### Phase 3: Power Features (Week 5+)
**Goal:** Advanced cluster management

- [ ] **Config Management**
  - ConfigMap/Secret viewer
  - Inline editing (with validation)
  - Rollback support

- [ ] **Deployment Tools**
  - Rolling update visualization
  - Canary/Blue-Green prep helpers
  - Pod replica scaler

- [ ] **Advanced Diagnostics**
  - Network policies explorer
  - RBAC viewer
  - PVC/Storage status
  - StatefulSet state tracking

- [ ] **Cluster Health Report**
  - Generate diagnostic bundle
  - Export to JSON/YAML
  - Health scoring

---

## 🏗️ Architecture

### Directory Structure
```
kubectui/
├── src/
│   ├── main.rs              # Entry point
│   ├── app.rs               # Main app state machine
│   ├── ui/
│   │   ├── mod.rs
│   │   ├── dashboard.rs     # Dashboard view
│   │   ├── resource_list.rs # Generic list widget
│   │   ├── detail.rs        # Detail view
│   │   └── components.rs    # Reusable widgets
│   ├── k8s/
│   │   ├── mod.rs
│   │   ├── client.rs        # Kube client wrapper
│   │   ├── resources.rs     # Resource structs
│   │   ├── metrics.rs       # Metrics fetching
│   │   └── cache.rs         # Client-side cache
│   ├── state/
│   │   ├── mod.rs
│   │   ├── app_state.rs     # Global state
│   │   ├── filters.rs       # Filter logic
│   │   └── events.rs        # Event handling
│   └── styles/
│       └── theme.rs         # Colors + styling
├── Cargo.toml
└── DESIGN.md (this file)
```

### Key Components

#### 1. **App State Machine** (`app.rs`)
```rust
pub enum AppView {
    Dashboard,
    Nodes,
    Pods,
    Services,
    Deployments,
    DetailView(Resource),
    Logs(PodRef),
}

pub struct AppState {
    view: AppView,
    resources: ResourceCache,
    selected_idx: usize,
    search_query: String,
    error: Option<String>,
}
```

#### 2. **K8s Client Wrapper** (`k8s/client.rs`)
- Async wrapper around kube-rs
- Auto-reconnect on disconnection
- Caching layer (TTL-based, configurable)
- Metrics polling (optional)

#### 3. **Resource Cache** (`k8s/cache.rs`)
- In-memory cache for Nodes, Pods, Services, etc.
- Background syncer (watch API)
- Fast lookup without API thrashing

#### 4. **UI Layer** (ratatui-based)
- **Stateless rendering** — redraw on state changes
- **Color scheme:** Dark theme (solarized or custom)
- **Typography:** Clean monospace, readable at small sizes

---

## 🎨 UI Design

### Color Palette (Inspired by Kubernetes branding)
```
Background:       #0d1117 (dark)
Text:             #e6edf3 (light)
Accent:           #58a6ff (kubernetes blue)
Success:          #3fb950 (green)
Warning:          #d29922 (orange)
Error:            #f85149 (red)
Border:           #30363d (subtle gray)
```

### Layout (Dashboard View)
```
┌──────────────────────────────────────────────────────────┐
│ KubecTUI v0.1.0 | Cluster: prod-us-east-1 (v1.30.0)     │
├──────────────────────────────────────────────────────────┤
│ [Dashboard] [Nodes] [Pods] [Services] [Deployments]      │
├──────────────────────────────────────────────────────────┤
│ 📊 CLUSTER STATUS                                        │
│                                                          │
│  Nodes:        12/12 Ready (↑ 5 using 45% CPU)          │
│  Pods:         342 Running (8 Pending, 2 Failed)        │
│  Services:     28 (24 ClusterIP, 4 LoadBalancer)        │
│  Namespaces:   8 Active                                 │
│                                                          │
│  Memory:       ████████████░░░░░░░░ 67% (128GB/192GB)   │
│  CPU:          ██████░░░░░░░░░░░░░░ 31% (12.4/40 cores) │
│                                                          │
│ ⚠️  ALERTS                                               │
│  • Node-5: MemoryPressure detected                       │
│  • Pod crash loop: payment-svc-3 (10 restarts)          │
│  • PVC: db-storage-pvc 92% full                         │
│                                                          │
├──────────────────────────────────────────────────────────┤
│ ⌘ Cmd: [Tab] Navigate | [/] Search | [d] Details | [q] Quit
└──────────────────────────────────────────────────────────┘
```

### List View (Pods Tab)
```
┌──────────────────────────────────────────────────────────┐
│ PODS (Filter: all namespaces, Status: all)               │
├─ Name ─────────────────────┬─ NS ──┬─ Status ┬─ Restarts┤
│ > nginx-deployment-5d4f6   │ prod  │ Running │ 0        │
│   api-gateway-7f8c2        │ prod  │ Running │ 0        │
│   redis-cache-0            │ prod  │ Running │ 12 ⚠️   │
│   payment-svc-xyz          │ prod  │ Failed  │ 5 🔴    │
│   db-migration-job         │ infra │ Pending │ -        │
│                                                          │
│ Search: [________________________]                       │
│ Filter: [namespace=prod] [status=running]               │
│                                                          │
│ ⌘ Cmd: [↑↓] Navigate | [Enter] Details | [l] Logs      │
└──────────────────────────────────────────────────────────┘
```

### Detail View (Pod Details)
```
┌──────────────────────────────────────────────────────────┐
│ POD DETAILS: nginx-deployment-5d4f6                      │
├──────────────────────────────────────────────────────────┤
│ Namespace:    prod                                       │
│ Status:       Running (Ready 1/1)                        │
│ Node:         worker-3 (10.0.2.15)                       │
│ IP:           172.17.0.5                                │
│ Created:      2h 34m ago                                │
│ Labels:       app=web, version=v2.1                     │
│ Annotations:  custom-key: custom-value                  │
│                                                          │
│ CONTAINERS (1)                                          │
│ ├ Name:       nginx                                     │
│ │ Image:      nginx:1.27@sha256:abc123...              │
│ │ State:      Running (started 2h ago)                 │
│ │ CPU/Mem:    50m / 128Mi (request: 100m/256Mi)       │
│ │ Ports:      80/TCP                                   │
│ │ Mounts:     /etc/config → ConfigMap: nginx-conf     │
│ │             /data → PVC: app-data                    │
│                                                          │
│ EVENTS (recent)                                         │
│ ✓ Successfully pulled image                            │
│ ✓ Created container                                     │
│ ✓ Started container                                     │
│                                                          │
│ [View YAML] [Port Forward] [Shell] [Delete] [Logs]     │
│ ⌘ Cmd: [ESC] Back | [Spacebar] Actions | [q] Quit     │
└──────────────────────────────────────────────────────────┘
```

---

## ⚡ Performance Goals

| Operation | Target | Method |
|-----------|--------|--------|
| Initial load | < 2s | Async client, early results |
| Tab switch | < 500ms | Cached data, fast sort |
| Search | < 300ms | In-memory filter (no API call) |
| Detail view | < 1s | Pre-fetch on hover/selection |
| Real-time updates | < 5s | Background watch API sync |
| Memory usage | < 50MB | Smart cache eviction |
| CPU (idle) | < 2% | Event-driven rendering |

---

## 🔧 Technology Stack

| Layer | Tech | Why |
|-------|------|-----|
| **UI Framework** | ratatui 0.30 | Fast, proven, extensible |
| **K8s Client** | kube-rs 0.92 | Async, type-safe, official |
| **Runtime** | tokio 1.x | Battle-tested async |
| **Styling** | ratatui-core | Theming, colors, layouts |
| **Serialization** | serde + serde_json | JSON ↔ struct |
| **CLI Args** | clap 4 | Standard, ergonomic |
| **Error Handling** | anyhow | Simple, contextual errors |

---

## 📝 Implementation Strategy

### Phase 1 Tasks (MVP - 1 week)
1. **Day 1-2:** Set up app state machine + basic UI structure
2. **Day 3-4:** K8s client wrapper + resource fetching
3. **Day 5:** Dashboard view + metrics summary
4. **Day 6-7:** List views (Nodes, Pods, Services) + navigation

### Iteration Strategy
- Build one view at a time (start with Dashboard)
- Test locally against KIND cluster (lightweight)
- Benchmark and optimize as we go
- Get feedback early and often

### Testing Approach
- **Unit tests:** State machine, filters, cache logic
- **Integration tests:** Against KIND/minikube cluster
- **Manual testing:** Real cluster (dry-run first)

---

## 🚀 Launch Checklist

- [ ] Core app compiles and runs
- [ ] Can connect to cluster (kubeconfig)
- [ ] Dashboard loads and updates
- [ ] All four list views (Nodes, Pods, Services, Deps)
- [ ] Detail view with YAML
- [ ] Search/filter working
- [ ] Keyboard shortcuts documented
- [ ] README with install + usage
- [ ] v0.1.0 release

---

## 📚 Future Ideas

- **Multi-cluster support** (context switching)
- **Custom dashboards** (user-defined widgets)
- **Helm integration** (release viewer/deployer)
- **ArgoCD integration** (sync status)
- **Slack/PagerDuty alerts** (notification plugin)
- **Shell completion** (zsh, bash, fish)
- **Dark/Light theme toggle**
- **Config file support** (.kubectui.yaml)
- **Session save/restore** (remember filters)
- **Export to Prometheus** (cluster metrics)

---

## 🎯 Success Metrics

- **Performance:** All operations < 1s
- **Usability:** New user can navigate in < 5 min
- **Stability:** 0 crashes per 8h session
- **Resource efficiency:** < 50MB memory, < 5% idle CPU
- **Coverage:** 80% of kubectl get/describe workflows supported

---

**Status:** Ready for Phase 1 implementation  
**Last Updated:** 2026-02-23  
**Owner:** KiBOT + Tuan Ilham
