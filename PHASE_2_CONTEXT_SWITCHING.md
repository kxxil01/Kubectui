# Phase 2: Multi-Cluster Context Switching
**Planned for:** Next Sprint (v0.2.0)  
**Status:** Design Document

---

## 🎯 Feature Overview

Enable KubecTUI to work like `kubectx` — easily switch between Kubernetes contexts and clusters without restarting the app.

---

## 📊 Current Behavior (v0.1.0)

```bash
# Must use environment variable or kubectl to switch
export KUBECONFIG=~/.kube/config
kubectl config use-context prod-cluster
./target/release/kubectui

# App connects to that specific cluster
# To switch clusters, must:
# 1. Exit app (q)
# 2. Run: kubectl config use-context other-cluster
# 3. Restart app
```

---

## 🚀 Proposed Behavior (v0.2.0)

### Startup Flow
```
┌─────────────────────────────────────────────────┐
│ KubecTUI - Startup                              │
├─────────────────────────────────────────────────┤
│                                                 │
│ 1. Scan kubeconfig (~/.kube/config + $env)     │
│ 2. List all available contexts                  │
│ 3. Show context selector menu (if multiple)    │
│ 4. User selects context (↑↓ + Enter)           │
│ 5. Load cluster + display dashboard            │
│                                                 │
└─────────────────────────────────────────────────┘
```

### Runtime Context Switching
```
While app is running:
- Press 'C' key → Show context switcher menu
- Select new context → Reconnect automatically
- Update dashboard with new cluster data
- No restart required
```

---

## 💻 Implementation Details

### 1. Kubeconfig Parsing (Enhanced)

**Current (v0.1.0):**
```rust
let client = Client::try_default().await?;  // Uses default context only
```

**Proposed (v0.2.0):**
```rust
pub struct KubeContext {
    pub name: String,
    pub cluster: String,
    pub user: String,
    pub namespace: String,
}

pub async fn list_available_contexts() -> Result<Vec<KubeContext>> {
    // Parse kubeconfig
    // Extract all contexts (not just active one)
    // Return list of available contexts
}

pub async fn switch_context(context_name: &str) -> Result<Client> {
    // Switch to specific context
    // Create new client for that cluster
    // Return client
}
```

### 2. UI Changes

**New Screen: Context Selector**
```
┌────────────────────────────────────────────┐
│ AVAILABLE CONTEXTS (5 total)               │
├────────────────────────────────────────────┤
│ > kind-kubectui-dev         (local)        │
│   minikube                  (local)        │
│   docker-desktop            (local)        │
│   staging-eu-west-1         (AWS EKS)     │
│   prod-us-east-1            (AWS EKS)     │
├────────────────────────────────────────────┤
│ Current: kind-kubectui-dev                 │
│ ⌘ [↑↓] Navigate | [Enter] Select | [q] Exit
└────────────────────────────────────────────┘
```

**New Keyboard Shortcut:**
```
While in dashboard/any view:
- Press 'C' → Open context switcher
- Select new context
- Auto-reconnect to new cluster
- Dashboard updates automatically
```

### 3. State Management

```rust
pub struct GlobalState {
    pub current_context: String,      // NEW
    pub available_contexts: Vec<String>,  // NEW
    pub cluster_snapshot: ClusterSnapshot,
    pub data_phase: DataPhase,
    // ... existing fields
}

pub enum AppView {
    ContextSelector,  // NEW
    Dashboard,
    Nodes,
    Pods,
    Services,
    Deployments,
    DetailView(ResourceRef),
}
```

### 4. Connection Lifecycle

```rust
pub async fn on_context_change(new_context: &str) {
    // Step 1: Save current state (optional)
    // Step 2: Disconnect from old cluster
    // Step 3: Create new client for new context
    // Step 4: Update GlobalState.current_context
    // Step 5: Trigger refresh
    // Step 6: Return to previous view in new cluster
}
```

---

## 🎯 User Workflows

### Scenario 1: Start with Context Menu
```
$ ./target/release/kubectui

→ Shows context selector
→ User selects "prod-us-east-1"
→ App connects and shows prod cluster dashboard
→ User can navigate normally
```

### Scenario 2: Switch Mid-Session
```
User is viewing staging cluster
→ Presses 'C'
→ Context menu appears
→ User selects "prod-us-east-1"
→ Dashboard automatically updates to prod
→ Previous view (e.g., Nodes tab) maintained if possible
```

### Scenario 3: Rapid Context Switching
```
User in prod → Press 'C' → select staging
Staging dashboard appears
User in staging → Press 'C' → select minikube
Local cluster dashboard appears
(No restarts required)
```

---

## 🔧 Implementation Phases

### Phase 2.1: Context Listing
- [ ] Parse kubeconfig for all contexts
- [ ] Display context menu on startup
- [ ] Allow single context selection
- [ ] Connect to selected context
- [ ] **Effort:** 2-3 hours

### Phase 2.2: Runtime Switching
- [ ] Add 'C' key handler
- [ ] Implement context switcher overlay
- [ ] Handle reconnection logic
- [ ] Update dashboard smoothly
- [ ] **Effort:** 2-3 hours

### Phase 2.3: Context Metadata
- [ ] Show cluster type (local/cloud/on-prem)
- [ ] Show namespace (if set in context)
- [ ] Show user info
- [ ] Cache recently used contexts
- [ ] **Effort:** 1-2 hours

---

## 🎯 Feature Parity with kubectx

| Feature | kubectx | KubecTUI v0.2 (Planned) |
|---------|---------|-------------------------|
| List contexts | ✅ | ✅ |
| Switch contexts | ✅ | ✅ |
| Show current context | ✅ | ✅ |
| Rename context | ✅ | (Phase 3) |
| Delete context | ✅ | (Phase 3) |
| Preview cluster info | ❌ | ✅ |
| Switch + visualize | ❌ | ✅ |

---

## 📝 Configuration Options

### Environment Variables
```bash
# Specify custom kubeconfig location
export KUBECONFIG=~/.kube/config:/etc/kube/admin.conf

# Default context (skip menu on startup)
export KUBECTUI_DEFAULT_CONTEXT=prod-us-east-1

# Show context menu always (even single context)
export KUBECTUI_ALWAYS_SHOW_CONTEXT_MENU=1
```

### Config File (~/.kubectui/config)
```yaml
# kubectui config
default_context: prod-us-east-1
context_order:
  - prod-us-east-1
  - staging-eu-west-1
  - kind-kubectui-dev
kubeconfig_paths:
  - ~/.kube/config
  - ~/.kube/prod-config
  - /etc/kubernetes/admin.conf
```

---

## 🚀 Integration with Existing Features

### Works with All Views
```
Feature                 Status
─────────────────────────────────
Dashboard               ✅ Updates on switch
Nodes list              ✅ Reloads nodes
Pods list               ✅ Shows new cluster pods
Services list           ✅ New services loaded
Deployments list        ✅ New deployments loaded
Detail view             ✅ Closes on switch (safe)
Search/filter           ✅ Resets for new cluster
```

---

## 🎯 Success Criteria

- [ ] Users can select from multiple contexts on startup
- [ ] Context switching works mid-session (press 'C')
- [ ] Dashboard updates automatically on context change
- [ ] No crashes or data corruption on switch
- [ ] Performance <500ms for context switch
- [ ] Supports kubectl config merge (multiple kubeconfig files)
- [ ] Works with local (KIND, minikube) and cloud clusters (EKS, GKE, AKS)

---

## 🔮 Future Enhancements (Phase 3+)

- [ ] Context bookmarks (favorite clusters)
- [ ] Quick switch (e.g., '1' for first context)
- [ ] Context-specific settings (colors, refresh rate)
- [ ] Shell command history per context
- [ ] Context aliases (e.g., `prod` → `prod-us-east-1`)
- [ ] Auto-reconnect on network failure
- [ ] Multi-cluster dashboard (view multiple clusters side-by-side)

---

## 📚 Related Issues

- GitHub Issues: Multi-cluster support (coming soon)
- Dependency: No new dependencies needed
- Breaking Changes: None (backward compatible)

---

## 🎉 Example Use Cases

### Use Case 1: DevOps Engineer
```
Morning routine:
1. Start kubectui
2. Select staging-eu-west-1
3. Check deployments
4. Press 'C' → switch to prod-us-east-1
5. Verify prod cluster health
6. Monitor both clusters throughout day
```

### Use Case 2: Platform Team
```
On-call rotation:
1. Start kubectui with $KUBECTUI_DEFAULT_CONTEXT=prod
2. Handle incidents
3. Need to check staging → Press 'C' → switch
4. Compare with prod → 'C' → switch back
5. All without restarting app
```

### Use Case 3: Multi-Cloud Setup
```
Manage AWS + GCP + Azure:
1. Start kubectui
2. List shows contexts from all clouds
3. Switch freely between providers
4. Compare resources across clouds
```

---

**Status:** ✅ **READY FOR PHASE 2 IMPLEMENTATION**

*This feature will make KubecTUI the go-to tool for multi-cluster management.*
