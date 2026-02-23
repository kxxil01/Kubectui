# Phase 2: Multi-Cluster Context Switching - Implementation Guide
**Status:** Ready for Implementation  
**Dependencies:** ZERO new external dependencies  
**Estimated Effort:** 10-12 hours  
**Target Release:** v0.2.0

---

## 🎯 Overview

Add native multi-cluster context switching to KubecTUI using only Rust + existing dependencies.

**Key Promise:** No external tools (no kubectx), fully self-contained, single binary.

---

## 📋 Architecture

```
Dependencies Available (Already Included):
├── serde_yaml      ✅ Parse kubeconfig
├── kube-rs         ✅ Native context switching
├── tokio           ✅ Async runtime
└── anyhow          ✅ Error handling

New Dependencies Required: ZERO
```

---

## 🔧 Concrete Implementation

### File 1: `src/k8s/context.rs` (NEW)

```rust
//! Kubernetes context management for multi-cluster support
//! No external dependencies - uses kube-rs native APIs

use anyhow::{anyhow, Context, Result};
use kube::config::KubeConfigLoader;
use serde_yaml;
use std::fs;
use std::path::PathBuf;

/// Represents a Kubernetes context from kubeconfig
#[derive(Clone, Debug, PartialEq)]
pub struct KubeContext {
    pub name: String,
    pub cluster: String,
    pub user: String,
    pub namespace: String,
}

/// List all available Kubernetes contexts from kubeconfig
///
/// Parses ~/.kube/config (or $KUBECONFIG) and returns all available contexts.
/// This is fully self-contained - no external tools needed.
pub async fn list_available_contexts() -> Result<Vec<KubeContext>> {
    let kubeconfig_path = get_kubeconfig_path()?;
    let kubeconfig_content = fs::read_to_string(&kubeconfig_path)
        .context(format!("Failed to read kubeconfig: {:?}", kubeconfig_path))?;
    
    // Parse YAML using serde_yaml (already a dependency)
    let config = serde_yaml::from_str::<serde_yaml::Value>(&kubeconfig_content)
        .context("Failed to parse kubeconfig YAML")?;
    
    let mut contexts = Vec::new();
    
    // Extract all contexts from "contexts" field
    if let Some(ctx_list) = config["contexts"].as_sequence() {
        for ctx in ctx_list {
            if let Some(name) = ctx["name"].as_str() {
                let cluster = ctx["context"]["cluster"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                let user = ctx["context"]["user"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                let namespace = ctx["context"]["namespace"]
                    .as_str()
                    .unwrap_or("default")
                    .to_string();
                
                contexts.push(KubeContext {
                    name: name.to_string(),
                    cluster,
                    user,
                    namespace,
                });
            }
        }
    }
    
    Ok(contexts)
}

/// Get the current active context from kubeconfig
pub async fn get_current_context() -> Result<String> {
    let kubeconfig_path = get_kubeconfig_path()?;
    let kubeconfig_content = fs::read_to_string(&kubeconfig_path)?;
    let config = serde_yaml::from_str::<serde_yaml::Value>(&kubeconfig_content)?;
    
    config["current-context"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("No current context set in kubeconfig"))
}

/// Switch to a specific Kubernetes context
///
/// This uses kube-rs built-in context switching.
/// No external dependencies or subprocess calls needed.
pub async fn connect_to_context(context_name: &str) -> Result<kube::Client> {
    let kubeconfig_path = get_kubeconfig_path()?;
    
    // kube-rs has native context switching support (public API)
    let kubeconfig = KubeConfigLoader::new_from_file(kubeconfig_path)?
        .with_context(context_name)  // ← Native kube-rs method
        .load()
        .context(format!("Failed to load context '{}'", context_name))?;
    
    let client = kube::Client::try_from(kubeconfig)?;
    Ok(client)
}

/// Get kubeconfig path respecting standard K8s conventions
fn get_kubeconfig_path() -> Result<PathBuf> {
    // 1. Check $KUBECONFIG
    if let Ok(path) = std::env::var("KUBECONFIG") {
        return Ok(PathBuf::from(path));
    }
    
    // 2. Check ~/.kube/config
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .context("Cannot determine home directory")?;
    
    let default_path = PathBuf::from(home).join(".kube/config");
    if default_path.exists() {
        return Ok(default_path);
    }
    
    Err(anyhow!(
        "No kubeconfig found. Check $KUBECONFIG or ~/.kube/config"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_get_kubeconfig_path() {
        let result = get_kubeconfig_path();
        assert!(result.is_ok(), "Should find kubeconfig");
    }
    
    #[tokio::test]
    async fn test_list_available_contexts() {
        let contexts = list_available_contexts().await;
        assert!(contexts.is_ok(), "Should list contexts");
        
        let contexts = contexts.unwrap();
        assert!(!contexts.is_empty(), "Should have at least one context");
    }
    
    #[tokio::test]
    async fn test_get_current_context() {
        let current = get_current_context().await;
        assert!(current.is_ok(), "Should get current context");
    }
    
    #[tokio::test]
    async fn test_connect_to_context_valid() {
        // Get first available context
        let contexts = list_available_contexts().await.unwrap();
        if !contexts.is_empty() {
            let result = connect_to_context(&contexts[0].name).await;
            assert!(result.is_ok(), "Should connect to valid context");
        }
    }
    
    #[tokio::test]
    async fn test_connect_to_context_invalid() {
        let result = connect_to_context("nonexistent-cluster-xyz").await;
        assert!(result.is_err(), "Should fail for nonexistent context");
    }
}
```

### File 2: Update `src/state/mod.rs`

Add context management to GlobalState:

```rust
// Add to GlobalState struct
pub struct GlobalState {
    pub current_context: String,  // NEW
    pub available_contexts: Vec<KubeContext>,  // NEW
    pub client: Client,
    pub cluster_snapshot: ClusterSnapshot,
    pub data_phase: DataPhase,
    pub last_error: Option<String>,
}

// Add context switching method
impl GlobalState {
    /// Load all available contexts on startup
    pub async fn load_contexts() -> Result<Vec<KubeContext>> {
        use crate::k8s::context;
        context::list_available_contexts().await
    }
    
    /// Switch to a different context
    pub async fn switch_context(&mut self, context_name: &str) -> Result<()> {
        use crate::k8s::context;
        
        // Connect to new context
        self.client = context::connect_to_context(context_name).await?;
        self.current_context = context_name.to_string();
        
        // Clear old data
        self.cluster_snapshot = ClusterSnapshot::default();
        self.data_phase = DataPhase::Idle;
        
        // Refresh with new context
        self.refresh().await?;
        
        Ok(())
    }
}
```

### File 3: Update `src/app.rs`

Add context switcher view and keyboard handling:

```rust
// Add new view variant
pub enum AppView {
    ContextSelector,  // NEW
    Dashboard,
    Nodes,
    Pods,
    Services,
    Deployments,
    DetailView(ResourceRef),
}

// Add context selection state
pub struct ContextSelectorState {
    pub contexts: Vec<KubeContext>,
    pub selected_idx: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl AppState {
    /// Handle 'C' key to open context switcher
    pub fn handle_context_key(&mut self) {
        self.view = AppView::ContextSelector;
    }
    
    /// Select context and switch
    pub fn select_context(&mut self, idx: usize) {
        if idx < self.available_contexts.len() {
            let context = self.available_contexts[idx].clone();
            // This will be handled async in main event loop
            self.pending_context_switch = Some(context);
        }
    }
}
```

### File 4: Update `src/ui/mod.rs`

Add rendering for context selector:

```rust
pub fn render_context_selector(
    frame: &mut Frame,
    contexts: &[KubeContext],
    selected: usize,
    current: &str,
) {
    let area = frame.size();
    
    // Create centered block
    let block = Block::default()
        .title("Select Kubernetes Context")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded);
    
    let list_area = Rect {
        x: area.width / 4,
        y: area.height / 4,
        width: area.width / 2,
        height: area.height / 2,
    };
    
    let mut lines = vec![];
    for (idx, ctx) in contexts.iter().enumerate() {
        let is_selected = idx == selected;
        let is_current = ctx.name == current;
        
        let marker = if is_selected { "> " } else { "  " };
        let current_marker = if is_current { " (current)" } else { "" };
        let line = format!("{}{}{}", marker, ctx.name, current_marker);
        
        let style = if is_selected {
            Style::default().fg(Color::Cyan).bold()
        } else if is_current {
            Style::default().fg(Color::Green)
        } else {
            Style::default()
        };
        
        lines.push(Line::from(Span::styled(line, style)));
    }
    
    let paragraph = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Center);
    
    frame.render_widget(paragraph, list_area);
}
```

### File 5: Update `src/main.rs`

Add event handling for context switching:

```rust
// In main event loop
if let Event::Key(key) = event::read()? {
    match key.code {
        KeyCode::Char('c') | KeyCode::Char('C') => {
            // Open context selector
            app.view = AppView::ContextSelector;
            app.context_selector_state = ContextSelectorState {
                contexts: global_state.available_contexts.clone(),
                selected_idx: 0,
                loading: false,
                error: None,
            };
        }
        KeyCode::Down if app.view == AppView::ContextSelector => {
            app.context_selector_state.selected_idx = 
                (app.context_selector_state.selected_idx + 1) 
                % app.context_selector_state.contexts.len();
        }
        KeyCode::Up if app.view == AppView::ContextSelector => {
            if app.context_selector_state.selected_idx == 0 {
                app.context_selector_state.selected_idx = 
                    app.context_selector_state.contexts.len() - 1;
            } else {
                app.context_selector_state.selected_idx -= 1;
            }
        }
        KeyCode::Enter if app.view == AppView::ContextSelector => {
            let idx = app.context_selector_state.selected_idx;
            let context_name = app.context_selector_state.contexts[idx].name.clone();
            
            // Switch context async
            let client = global_state.client.clone();
            tokio::spawn(async move {
                if let Err(e) = global_state.switch_context(&context_name).await {
                    eprintln!("Failed to switch context: {}", e);
                }
            });
            
            app.view = AppView::Dashboard;
        }
        KeyCode::Esc if app.view == AppView::ContextSelector => {
            app.view = AppView::Dashboard;
        }
        _ => {}
    }
}
```

---

## 📊 Build Dependencies

**Current Cargo.toml already has:**
```toml
serde_yaml = "0.9"        # ✅ For parsing kubeconfig
kube = { version = "0.92", features = ["client"] }  # ✅ Includes context support
anyhow = "1"              # ✅ Error handling
tokio = { version = "1", features = ["full"] }  # ✅ Async runtime
```

**New dependencies needed:** NONE ✅

---

## 🧪 Testing Strategy

### Unit Tests (in context.rs)
```rust
#[tokio::test]
async fn test_list_contexts() { /* ... */ }

#[tokio::test]
async fn test_current_context() { /* ... */ }

#[tokio::test]
async fn test_switch_context() { /* ... */ }
```

### Integration Tests (new file: tests/context_switching.rs)
```rust
#[tokio::test]
async fn test_switch_and_list_pods() {
    // Switch context
    // List resources
    // Verify they match new cluster
}
```

---

## 🎯 Implementation Steps

### Step 1: Core Context Module (2-3 hours)
- [ ] Create `src/k8s/context.rs`
- [ ] Implement `list_available_contexts()`
- [ ] Implement `connect_to_context()`
- [ ] Add unit tests
- [ ] Verify builds cleanly

### Step 2: State Integration (2-3 hours)
- [ ] Update `src/state/mod.rs`
- [ ] Add `current_context` field
- [ ] Implement `switch_context()` method
- [ ] Test state transitions

### Step 3: UI Components (2-3 hours)
- [ ] Add `ContextSelector` view
- [ ] Implement context renderer
- [ ] Add selection state
- [ ] Test rendering

### Step 4: Keyboard & Events (2 hours)
- [ ] Add 'C' key handler
- [ ] Implement selection navigation (↑↓)
- [ ] Wire Enter to switch
- [ ] Wire Esc to cancel

### Step 5: Integration & Testing (1-2 hours)
- [ ] Full integration test
- [ ] Test with KIND cluster (switch contexts)
- [ ] Test with real kubeconfig (multiple contexts)
- [ ] Verify performance (<500ms switch)

---

## ✅ Success Criteria

- [ ] App loads with context selector (if multiple contexts)
- [ ] User can select context with ↑↓ + Enter
- [ ] Context switch triggers auto-refresh
- [ ] Dashboard updates with new cluster data
- [ ] Press 'C' opens context switcher mid-session
- [ ] Switch <500ms (including API call)
- [ ] No crashes or panics
- [ ] All tests passing
- [ ] Zero new dependencies

---

## 🚀 Build Plan

**Option 1: Single Agent (12 hours)**
```
Codex Agent → Implement all 5 steps sequentially
Result: Complete Phase 2 in one session
```

**Option 2: Two Parallel Agents (6-8 hours)**
```
Codex Agent 1 → Core + State (steps 1-2)
Codex Agent 2 → UI + Events (steps 3-4)
Then: Integration + Testing (step 5)
Result: Faster parallel build
```

---

## 📋 Deliverables

### Code
- ✅ `src/k8s/context.rs` (150 lines)
- ✅ Updated `src/state/mod.rs` (50 lines)
- ✅ Updated `src/app.rs` (80 lines)
- ✅ Updated `src/ui/mod.rs` (60 lines)
- ✅ Updated `src/main.rs` (100 lines)
- ✅ `tests/context_switching.rs` (100 lines)

### Tests
- ✅ 6+ unit tests
- ✅ 4+ integration tests
- ✅ All passing

### Documentation
- ✅ Code comments
- ✅ Usage guide update
- ✅ Keybinding documentation

---

## 🎊 Phase 2.0 Release Checklist

- [ ] All code compiles cleanly
- [ ] All tests passing (100%)
- [ ] Zero warnings (clippy -D warnings)
- [ ] Context switching works smoothly
- [ ] Performance <500ms per switch
- [ ] Works with KIND cluster
- [ ] Works with real kubeconfig (multiple contexts)
- [ ] README updated with 'C' key documentation
- [ ] Git commits clean and descriptive
- [ ] GitHub pushed
- [ ] v0.2.0 tagged and released

---

## 📊 Effort Estimate

| Task | Est. Time |
|------|-----------|
| Core context module | 3h |
| State integration | 2h |
| UI components | 3h |
| Keyboard/events | 2h |
| Testing & validation | 2h |
| **Total** | **12h** |

---

## 🎁 What Users Get

```
v0.2.0 Release:
✅ Built-in context selector (no kubectx needed!)
✅ Press 'C' to switch contexts mid-session
✅ No app restart required
✅ Single binary - self-contained
✅ Works with any kubeconfig
✅ Fast (<500ms switches)
✅ All existing features work with any context
```

---

**Ready to implement? This is everything needed for Phase 2 MVP.** 🚀
