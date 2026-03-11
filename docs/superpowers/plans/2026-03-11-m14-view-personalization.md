# M14: View Personalization Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make KubecTUI remember sort preferences, column visibility, workbench state, and nav collapse state — with global defaults and per-cluster overrides.

**Architecture:** New `src/preferences.rs` module defines `ViewPreferences`, `UserPreferences`, `ClusterPreferences` types and a `resolve_preferences()` function. New `src/columns.rs` module defines `ColumnDef` and per-view column registries. `AppConfig` expands with optional preference fields (backward compatible). Column toggling exposed via action palette `:columns` command. Save triggers on user preference changes.

**Tech Stack:** Rust 2024, serde/serde_json, ratatui Constraint

---

## Chunk 1: Core Preferences & Persistence

### Task 1: Create preferences module with types and resolution

**Files:**
- Create: `src/preferences.rs`
- Modify: `src/lib.rs` — add `pub mod preferences;`

- [ ] **Step 1: Write tests for preference resolution**

In `src/preferences.rs`, add `#[cfg(test)] mod tests` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_view_prefs() {
        let vp = ViewPreferences::default();
        assert!(vp.sort_column.is_none());
        assert!(vp.sort_ascending);
        assert!(vp.hidden_columns.is_empty());
        assert!(vp.column_order.is_none());
    }

    #[test]
    fn resolve_global_only() {
        let mut global = UserPreferences::default();
        global.views.insert("pods".into(), ViewPreferences {
            sort_column: Some("age".into()),
            sort_ascending: false,
            ..Default::default()
        });
        let result = resolve_view_preferences("pods", &Some(global), &None, None);
        assert_eq!(result.sort_column.as_deref(), Some("age"));
        assert!(!result.sort_ascending);
    }

    #[test]
    fn resolve_cluster_overrides_global() {
        let mut global = UserPreferences::default();
        global.views.insert("pods".into(), ViewPreferences {
            sort_column: Some("age".into()),
            sort_ascending: false,
            hidden_columns: vec!["namespace".into()],
            ..Default::default()
        });
        let mut cluster = ClusterPreferences::default();
        cluster.views.insert("pods".into(), ViewPreferences {
            sort_column: Some("status".into()),
            ..Default::default()
        });
        let mut clusters = HashMap::new();
        clusters.insert("prod".into(), cluster);
        let result = resolve_view_preferences("pods", &Some(global), &Some(clusters), Some("prod"));
        assert_eq!(result.sort_column.as_deref(), Some("status"));
        // cluster override replaces entire ViewPreferences, not field-merge
        assert!(result.sort_ascending); // default, not inherited from global
    }

    #[test]
    fn resolve_unknown_cluster_falls_through() {
        let mut global = UserPreferences::default();
        global.views.insert("pods".into(), ViewPreferences {
            sort_column: Some("age".into()),
            ..Default::default()
        });
        let result = resolve_view_preferences("pods", &Some(global), &None, Some("unknown"));
        assert_eq!(result.sort_column.as_deref(), Some("age"));
    }

    #[test]
    fn resolve_no_prefs_returns_default() {
        let result = resolve_view_preferences("pods", &None, &None, None);
        assert!(result.sort_column.is_none());
        assert!(result.sort_ascending);
    }

    #[test]
    fn serde_round_trip() {
        let vp = ViewPreferences {
            sort_column: Some("restarts".into()),
            sort_ascending: false,
            hidden_columns: vec!["namespace".into(), "image".into()],
            column_order: Some(vec!["name".into(), "status".into(), "age".into()]),
        };
        let json = serde_json::to_string(&vp).unwrap();
        let back: ViewPreferences = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sort_column, vp.sort_column);
        assert_eq!(back.sort_ascending, vp.sort_ascending);
        assert_eq!(back.hidden_columns, vp.hidden_columns);
        assert_eq!(back.column_order, vp.column_order);
    }

    #[test]
    fn deserialize_empty_json_uses_defaults() {
        let vp: ViewPreferences = serde_json::from_str("{}").unwrap();
        assert!(vp.sort_column.is_none());
        assert!(vp.sort_ascending);
        assert!(vp.hidden_columns.is_empty());
        assert!(vp.column_order.is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib preferences::tests -- --nocapture`
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Implement preference types and resolution**

Create `src/preferences.rs`:

```rust
//! User preference types for view personalization.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_true() -> bool {
    true
}

/// Per-view sort + column preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ViewPreferences {
    /// Column ID to sort by (e.g. "age", "status", "restarts").
    #[serde(default)]
    pub sort_column: Option<String>,
    /// Sort direction. `true` = ascending (default).
    #[serde(default = "default_true")]
    pub sort_ascending: bool,
    /// Column IDs to hide.
    #[serde(default)]
    pub hidden_columns: Vec<String>,
    /// Custom column ordering. `None` = default order.
    #[serde(default)]
    pub column_order: Option<Vec<String>>,
}

/// Global user preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserPreferences {
    #[serde(default)]
    pub views: HashMap<String, ViewPreferences>,
}

/// Per-cluster preference overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClusterPreferences {
    #[serde(default)]
    pub views: HashMap<String, ViewPreferences>,
}

/// Resolves the effective preferences for a view by checking
/// cluster-specific → global → defaults.
pub fn resolve_view_preferences(
    view_key: &str,
    global: &Option<UserPreferences>,
    clusters: &Option<HashMap<String, ClusterPreferences>>,
    current_context: Option<&str>,
) -> ViewPreferences {
    // Try cluster-specific first
    if let Some(ctx) = current_context
        && let Some(clusters) = clusters
        && let Some(cluster) = clusters.get(ctx)
        && let Some(prefs) = cluster.views.get(view_key)
    {
        return prefs.clone();
    }
    // Fall through to global
    if let Some(global) = global
        && let Some(prefs) = global.views.get(view_key)
    {
        return prefs.clone();
    }
    ViewPreferences::default()
}
```

- [ ] **Step 4: Add `pub mod preferences;` to `src/lib.rs`**

Add the line alongside existing module declarations.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib preferences::tests -- --nocapture`
Expected: All 7 tests PASS

- [ ] **Step 6: Run quality gate**

Run: `cargo fmt --all && cargo clippy --all-targets --all-features -- -D warnings`
Expected: clean

- [ ] **Step 7: Commit**

```bash
git add src/preferences.rs src/lib.rs
git commit -m "feat(prefs): add ViewPreferences types and resolution logic"
```

---

### Task 2: Expand AppConfig with preference fields

**Files:**
- Modify: `src/app.rs` — AppConfig struct (~line 1508), load_config_from_path (~line 3131), save_config_to_path (~line 3159)

- [ ] **Step 1: Write test for config round-trip with preferences**

Add to `src/app.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn config_round_trip_with_preferences() {
    use crate::preferences::{UserPreferences, ViewPreferences, ClusterPreferences};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");

    let mut app = AppState::default();
    // Set up preferences
    let mut global = UserPreferences::default();
    global.views.insert("pods".into(), ViewPreferences {
        sort_column: Some("restarts".into()),
        sort_ascending: false,
        hidden_columns: vec!["namespace".into()],
        column_order: None,
    });
    app.preferences = Some(global);

    let mut cluster_prefs = ClusterPreferences::default();
    cluster_prefs.views.insert("pods".into(), ViewPreferences {
        sort_column: Some("status".into()),
        ..Default::default()
    });
    let mut clusters = std::collections::HashMap::new();
    clusters.insert("prod".into(), cluster_prefs);
    app.cluster_preferences = Some(clusters);

    app.collapsed_groups.insert(NavGroup::FluxCD);
    app.collapsed_groups.insert(NavGroup::AccessControl);

    save_config_to_path(&app, &path);
    let loaded = load_config_from_path(&path);

    // Verify preferences survived
    let prefs = loaded.preferences.as_ref().unwrap();
    let pod_prefs = prefs.views.get("pods").unwrap();
    assert_eq!(pod_prefs.sort_column.as_deref(), Some("restarts"));
    assert!(!pod_prefs.sort_ascending);
    assert_eq!(pod_prefs.hidden_columns, vec!["namespace"]);

    let clusters = loaded.cluster_preferences.as_ref().unwrap();
    let prod = clusters.get("prod").unwrap();
    let prod_pods = prod.views.get("pods").unwrap();
    assert_eq!(prod_pods.sort_column.as_deref(), Some("status"));

    // Verify collapsed groups survived
    assert!(loaded.collapsed_groups.contains(&NavGroup::FluxCD));
    assert!(loaded.collapsed_groups.contains(&NavGroup::AccessControl));
}

#[test]
fn config_backward_compat_no_prefs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    // Write old-style config without preference fields
    std::fs::write(&path, r#"{"namespace":"default","workbench_open":true,"workbench_height":14}"#).unwrap();
    let loaded = load_config_from_path(&path);
    assert!(loaded.preferences.is_none());
    assert!(loaded.cluster_preferences.is_none());
    assert!(loaded.collapsed_groups.is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib app::tests::config_round_trip_with_preferences -- --nocapture`
Expected: FAIL — `preferences` field doesn't exist on AppState

- [ ] **Step 3: Expand AppConfig and AppState**

In `src/app.rs`, modify `AppConfig` (~line 1508):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    namespace: String,
    #[serde(default)]
    theme: Option<String>,
    #[serde(default = "default_refresh_interval")]
    refresh_interval_secs: u64,
    #[serde(default)]
    workbench_open: bool,
    #[serde(default = "default_workbench_height")]
    workbench_height: u16,
    #[serde(default)]
    collapsed_nav_groups: Vec<String>,
    #[serde(default)]
    preferences: Option<UserPreferences>,
    #[serde(default)]
    clusters: Option<HashMap<String, ClusterPreferences>>,
}
```

Add to `AppState` fields (~line 1468, before workbench):

```rust
    /// Global user preferences for view sort/column customization.
    pub preferences: Option<UserPreferences>,
    /// Per-cluster preference overrides, keyed by kube context name.
    pub cluster_preferences: Option<HashMap<String, ClusterPreferences>>,
```

Update `AppState::default()` (~line 1499):

```rust
    preferences: None,
    cluster_preferences: None,
```

- [ ] **Step 4: Update load_config_from_path**

In `load_config_from_path` (~line 3131), after workbench setup add:

```rust
    app.preferences = cfg.preferences;
    app.cluster_preferences = cfg.clusters;
    if let Some(groups) = &cfg.collapsed_nav_groups {
        for name in groups {
            if let Some(g) = nav_group_from_str(name) {
                app.collapsed_groups.insert(g);
            }
        }
    }
```

Add helper function:

```rust
fn nav_group_from_str(s: &str) -> Option<NavGroup> {
    match s {
        "overview" => Some(NavGroup::Overview),
        "workloads" => Some(NavGroup::Workloads),
        "network" => Some(NavGroup::Network),
        "config" => Some(NavGroup::Config),
        "storage" => Some(NavGroup::Storage),
        "helm" => Some(NavGroup::Helm),
        "flux" | "fluxcd" => Some(NavGroup::FluxCD),
        "access_control" | "rbac" => Some(NavGroup::AccessControl),
        "custom_resources" | "extensions" => Some(NavGroup::CustomResources),
        _ => None,
    }
}

fn nav_group_to_str(g: NavGroup) -> &'static str {
    match g {
        NavGroup::Overview => "overview",
        NavGroup::Workloads => "workloads",
        NavGroup::Network => "network",
        NavGroup::Config => "config",
        NavGroup::Storage => "storage",
        NavGroup::Helm => "helm",
        NavGroup::FluxCD => "flux",
        NavGroup::AccessControl => "access_control",
        NavGroup::CustomResources => "custom_resources",
    }
}
```

- [ ] **Step 5: Update save_config_to_path**

In `save_config_to_path` (~line 3159), update AppConfig construction:

```rust
    let collapsed: Vec<String> = app.collapsed_groups
        .iter()
        .map(|g| nav_group_to_str(*g).to_string())
        .collect();
    let cfg = AppConfig {
        namespace: app.current_namespace.clone(),
        theme: Some(theme_name.to_string()),
        refresh_interval_secs: app.refresh_interval_secs,
        workbench_open: app.workbench.open,
        workbench_height: app.workbench.height,
        collapsed_nav_groups: collapsed,
        preferences: app.preferences.clone(),
        clusters: app.cluster_preferences.clone(),
    };
```

- [ ] **Step 6: Add imports**

At top of `src/app.rs`, add to existing `crate::` imports:

```rust
use crate::preferences::{UserPreferences, ClusterPreferences};
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test --lib app::tests -- --nocapture`
Expected: Both new tests PASS, existing tests still PASS

- [ ] **Step 8: Run quality gate**

Run: `cargo fmt --all && cargo clippy --all-targets --all-features -- -D warnings`
Expected: clean

- [ ] **Step 9: Commit**

```bash
git add src/app.rs
git commit -m "feat(prefs): expand AppConfig with preferences and collapsed groups"
```

---

## Chunk 2: Column Definition System

### Task 3: Create column registry module

**Files:**
- Create: `src/columns.rs`
- Modify: `src/lib.rs` — add `pub mod columns;`

- [ ] **Step 1: Write tests for column resolution**

In `src/columns.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::ViewPreferences;

    const TEST_COLS: &[ColumnDef] = &[
        ColumnDef { id: "name", label: "Name", default_width: Constraint::Min(20), hideable: false, default_visible: true },
        ColumnDef { id: "namespace", label: "Namespace", default_width: Constraint::Length(18), hideable: true, default_visible: true },
        ColumnDef { id: "status", label: "Status", default_width: Constraint::Length(12), hideable: true, default_visible: true },
        ColumnDef { id: "age", label: "Age", default_width: Constraint::Length(9), hideable: true, default_visible: true },
        ColumnDef { id: "image", label: "Image", default_width: Constraint::Length(30), hideable: true, default_visible: false },
    ];

    #[test]
    fn default_visible_columns() {
        let prefs = ViewPreferences::default();
        let visible = resolve_columns(TEST_COLS, &prefs);
        let ids: Vec<&str> = visible.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["name", "namespace", "status", "age"]);
    }

    #[test]
    fn hidden_columns_removed() {
        let prefs = ViewPreferences {
            hidden_columns: vec!["namespace".into()],
            ..Default::default()
        };
        let visible = resolve_columns(TEST_COLS, &prefs);
        let ids: Vec<&str> = visible.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["name", "status", "age"]);
    }

    #[test]
    fn non_hideable_column_cannot_be_hidden() {
        let prefs = ViewPreferences {
            hidden_columns: vec!["name".into()],
            ..Default::default()
        };
        let visible = resolve_columns(TEST_COLS, &prefs);
        assert!(visible.iter().any(|c| c.id == "name"));
    }

    #[test]
    fn column_order_applied() {
        let prefs = ViewPreferences {
            column_order: Some(vec!["age".into(), "name".into(), "namespace".into(), "status".into()]),
            ..Default::default()
        };
        let visible = resolve_columns(TEST_COLS, &prefs);
        let ids: Vec<&str> = visible.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["age", "name", "namespace", "status"]);
    }

    #[test]
    fn column_order_with_unknown_ids_skipped() {
        let prefs = ViewPreferences {
            column_order: Some(vec!["age".into(), "unknown".into(), "name".into()]),
            ..Default::default()
        };
        let visible = resolve_columns(TEST_COLS, &prefs);
        let ids: Vec<&str> = visible.iter().map(|c| c.id).collect();
        // age, name from order, then namespace, status (remaining default-visible, in original order)
        assert_eq!(ids, vec!["age", "name", "namespace", "status"]);
    }

    #[test]
    fn hidden_default_invisible_shown_when_not_hidden() {
        // "image" is default_visible=false, so it's hidden by default
        let prefs = ViewPreferences::default();
        let visible = resolve_columns(TEST_COLS, &prefs);
        assert!(!visible.iter().any(|c| c.id == "image"));
    }

    #[test]
    fn constraints_from_visible() {
        let prefs = ViewPreferences::default();
        let visible = resolve_columns(TEST_COLS, &prefs);
        let constraints = visible_constraints(&visible);
        assert_eq!(constraints.len(), 4);
    }

    #[test]
    fn view_key_for_known_view() {
        use crate::app::AppView;
        assert_eq!(view_key(AppView::Pods), "pods");
        assert_eq!(view_key(AppView::Deployments), "deployments");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib columns::tests -- --nocapture`
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Implement column registry**

Create `src/columns.rs`:

```rust
//! Column definition registry and resolution for table views.

use ratatui::layout::Constraint;

use crate::app::AppView;
use crate::preferences::ViewPreferences;

/// Describes a single table column.
#[derive(Debug, Clone, Copy)]
pub struct ColumnDef {
    /// Stable identifier used in preferences (e.g. "name", "age").
    pub id: &'static str,
    /// Display header text.
    pub label: &'static str,
    /// Default width constraint.
    pub default_width: Constraint,
    /// If false, this column cannot be hidden (e.g. "name").
    pub hideable: bool,
    /// Whether this column is visible by default.
    pub default_visible: bool,
}

/// Returns the preference key for a given AppView.
pub fn view_key(view: AppView) -> &'static str {
    match view {
        AppView::Dashboard => "dashboard",
        AppView::Issues => "issues",
        AppView::Nodes => "nodes",
        AppView::Namespaces => "namespaces",
        AppView::Events => "events",
        AppView::Pods => "pods",
        AppView::Deployments => "deployments",
        AppView::StatefulSets => "statefulsets",
        AppView::DaemonSets => "daemonsets",
        AppView::ReplicaSets => "replicasets",
        AppView::ReplicationControllers => "replicationcontrollers",
        AppView::Jobs => "jobs",
        AppView::CronJobs => "cronjobs",
        AppView::Services => "services",
        AppView::Endpoints => "endpoints",
        AppView::Ingresses => "ingresses",
        AppView::IngressClasses => "ingressclasses",
        AppView::NetworkPolicies => "networkpolicies",
        AppView::PortForwarding => "portforwarding",
        AppView::ConfigMaps => "configmaps",
        AppView::Secrets => "secrets",
        AppView::ResourceQuotas => "resourcequotas",
        AppView::LimitRanges => "limitranges",
        AppView::HPAs => "hpas",
        AppView::PodDisruptionBudgets => "poddisruptionbudgets",
        AppView::PriorityClasses => "priorityclasses",
        AppView::PersistentVolumeClaims => "pvcs",
        AppView::PersistentVolumes => "pvs",
        AppView::StorageClasses => "storageclasses",
        AppView::HelmCharts => "helmcharts",
        AppView::HelmReleases => "helmreleases",
        AppView::FluxCDAll => "flux_all",
        AppView::FluxCDAlertProviders => "flux_alertproviders",
        AppView::FluxCDAlerts => "flux_alerts",
        AppView::FluxCDArtifacts => "flux_artifacts",
        AppView::FluxCDHelmReleases => "flux_helmreleases",
        AppView::FluxCDHelmRepositories => "flux_helmrepositories",
        AppView::FluxCDImages => "flux_images",
        AppView::FluxCDKustomizations => "flux_kustomizations",
        AppView::FluxCDReceivers => "flux_receivers",
        AppView::FluxCDSources => "flux_sources",
        AppView::ServiceAccounts => "serviceaccounts",
        AppView::ClusterRoles => "clusterroles",
        AppView::Roles => "roles",
        AppView::ClusterRoleBindings => "clusterrolebindings",
        AppView::RoleBindings => "rolebindings",
        AppView::Extensions => "extensions",
    }
}

// ── Per-view column registries ──────────────────────────────────────

pub const POD_COLUMNS: &[ColumnDef] = &[
    ColumnDef { id: "name", label: "Name", default_width: Constraint::Min(28), hideable: false, default_visible: true },
    ColumnDef { id: "namespace", label: "Namespace", default_width: Constraint::Length(18), hideable: true, default_visible: true },
    ColumnDef { id: "status", label: "Status", default_width: Constraint::Length(20), hideable: true, default_visible: true },
    ColumnDef { id: "ready", label: "Ready", default_width: Constraint::Length(8), hideable: true, default_visible: true },
    ColumnDef { id: "age", label: "Age", default_width: Constraint::Length(9), hideable: true, default_visible: true },
    ColumnDef { id: "restarts", label: "Restarts", default_width: Constraint::Length(10), hideable: true, default_visible: true },
    ColumnDef { id: "node", label: "Node", default_width: Constraint::Length(22), hideable: true, default_visible: false },
    ColumnDef { id: "ip", label: "IP", default_width: Constraint::Length(16), hideable: true, default_visible: false },
];

pub const DEPLOYMENT_COLUMNS: &[ColumnDef] = &[
    ColumnDef { id: "name", label: "Name", default_width: Constraint::Min(28), hideable: false, default_visible: true },
    ColumnDef { id: "namespace", label: "Namespace", default_width: Constraint::Length(18), hideable: true, default_visible: true },
    ColumnDef { id: "ready", label: "Ready", default_width: Constraint::Length(10), hideable: true, default_visible: true },
    ColumnDef { id: "updated", label: "Up-to-date", default_width: Constraint::Length(12), hideable: true, default_visible: true },
    ColumnDef { id: "available", label: "Available", default_width: Constraint::Length(11), hideable: true, default_visible: true },
    ColumnDef { id: "age", label: "Age", default_width: Constraint::Length(9), hideable: true, default_visible: true },
    ColumnDef { id: "image", label: "Image", default_width: Constraint::Length(34), hideable: true, default_visible: true },
];

pub const NODE_COLUMNS: &[ColumnDef] = &[
    ColumnDef { id: "name", label: "Name", default_width: Constraint::Min(24), hideable: false, default_visible: true },
    ColumnDef { id: "status", label: "Status", default_width: Constraint::Length(22), hideable: true, default_visible: true },
    ColumnDef { id: "roles", label: "Roles", default_width: Constraint::Length(16), hideable: true, default_visible: true },
    ColumnDef { id: "ready", label: "Ready", default_width: Constraint::Length(8), hideable: true, default_visible: true },
    ColumnDef { id: "age", label: "Age", default_width: Constraint::Length(9), hideable: true, default_visible: true },
    ColumnDef { id: "cpu", label: "CPU", default_width: Constraint::Length(8), hideable: true, default_visible: true },
    ColumnDef { id: "memory", label: "Memory", default_width: Constraint::Length(10), hideable: true, default_visible: true },
];

pub const SERVICE_COLUMNS: &[ColumnDef] = &[
    ColumnDef { id: "name", label: "Name", default_width: Constraint::Min(24), hideable: false, default_visible: true },
    ColumnDef { id: "namespace", label: "Namespace", default_width: Constraint::Length(18), hideable: true, default_visible: true },
    ColumnDef { id: "type", label: "Type", default_width: Constraint::Length(14), hideable: true, default_visible: true },
    ColumnDef { id: "cluster_ip", label: "Cluster IP", default_width: Constraint::Length(16), hideable: true, default_visible: true },
    ColumnDef { id: "ports", label: "Ports", default_width: Constraint::Length(26), hideable: true, default_visible: true },
    ColumnDef { id: "age", label: "Age", default_width: Constraint::Length(9), hideable: true, default_visible: true },
];

pub const STATEFULSET_COLUMNS: &[ColumnDef] = &[
    ColumnDef { id: "name", label: "Name", default_width: Constraint::Min(28), hideable: false, default_visible: true },
    ColumnDef { id: "namespace", label: "Namespace", default_width: Constraint::Length(18), hideable: true, default_visible: true },
    ColumnDef { id: "ready", label: "Ready", default_width: Constraint::Length(10), hideable: true, default_visible: true },
    ColumnDef { id: "age", label: "Age", default_width: Constraint::Length(9), hideable: true, default_visible: true },
    ColumnDef { id: "image", label: "Image", default_width: Constraint::Length(34), hideable: true, default_visible: true },
];

pub const DAEMONSET_COLUMNS: &[ColumnDef] = &[
    ColumnDef { id: "name", label: "Name", default_width: Constraint::Min(28), hideable: false, default_visible: true },
    ColumnDef { id: "namespace", label: "Namespace", default_width: Constraint::Length(18), hideable: true, default_visible: true },
    ColumnDef { id: "desired", label: "Desired", default_width: Constraint::Length(9), hideable: true, default_visible: true },
    ColumnDef { id: "current", label: "Current", default_width: Constraint::Length(9), hideable: true, default_visible: true },
    ColumnDef { id: "ready", label: "Ready", default_width: Constraint::Length(9), hideable: true, default_visible: true },
    ColumnDef { id: "age", label: "Age", default_width: Constraint::Length(9), hideable: true, default_visible: true },
];

pub const JOB_COLUMNS: &[ColumnDef] = &[
    ColumnDef { id: "name", label: "Name", default_width: Constraint::Min(28), hideable: false, default_visible: true },
    ColumnDef { id: "namespace", label: "Namespace", default_width: Constraint::Length(18), hideable: true, default_visible: true },
    ColumnDef { id: "completions", label: "Completions", default_width: Constraint::Length(14), hideable: true, default_visible: true },
    ColumnDef { id: "duration", label: "Duration", default_width: Constraint::Length(11), hideable: true, default_visible: true },
    ColumnDef { id: "age", label: "Age", default_width: Constraint::Length(9), hideable: true, default_visible: true },
];

pub const CRONJOB_COLUMNS: &[ColumnDef] = &[
    ColumnDef { id: "name", label: "Name", default_width: Constraint::Min(28), hideable: false, default_visible: true },
    ColumnDef { id: "namespace", label: "Namespace", default_width: Constraint::Length(18), hideable: true, default_visible: true },
    ColumnDef { id: "schedule", label: "Schedule", default_width: Constraint::Length(18), hideable: true, default_visible: true },
    ColumnDef { id: "suspend", label: "Suspend", default_width: Constraint::Length(9), hideable: true, default_visible: true },
    ColumnDef { id: "active", label: "Active", default_width: Constraint::Length(8), hideable: true, default_visible: true },
    ColumnDef { id: "last_schedule", label: "Last Schedule", default_width: Constraint::Length(15), hideable: true, default_visible: true },
    ColumnDef { id: "age", label: "Age", default_width: Constraint::Length(9), hideable: true, default_visible: true },
];

/// Returns the column registry for a view, or `None` for views without table columns.
pub fn columns_for_view(view: AppView) -> Option<&'static [ColumnDef]> {
    match view {
        AppView::Pods => Some(POD_COLUMNS),
        AppView::Deployments => Some(DEPLOYMENT_COLUMNS),
        AppView::Nodes => Some(NODE_COLUMNS),
        AppView::Services => Some(SERVICE_COLUMNS),
        AppView::StatefulSets => Some(STATEFULSET_COLUMNS),
        AppView::DaemonSets => Some(DAEMONSET_COLUMNS),
        AppView::Jobs => Some(JOB_COLUMNS),
        AppView::CronJobs => Some(CRONJOB_COLUMNS),
        // Remaining views use columns_for_view returning None for now;
        // registries can be added incrementally.
        _ => None,
    }
}

/// Resolves the visible columns for a view given user preferences.
///
/// 1. Start with all columns where `default_visible` is true
/// 2. Remove columns listed in `hidden_columns` (skip non-hideable)
/// 3. Apply `column_order` if set (unknown IDs skipped, remaining appended)
pub fn resolve_columns(registry: &[ColumnDef], prefs: &ViewPreferences) -> Vec<ColumnDef> {
    // Start with default-visible columns
    let mut visible: Vec<ColumnDef> = registry
        .iter()
        .filter(|c| c.default_visible)
        .copied()
        .collect();

    // Remove hidden columns (respect hideable flag)
    if !prefs.hidden_columns.is_empty() {
        visible.retain(|c| !c.hideable || !prefs.hidden_columns.contains(&c.id.to_string()));
    }

    // Apply custom ordering if set
    if let Some(order) = &prefs.column_order {
        let mut ordered = Vec::with_capacity(visible.len());
        for id in order {
            if let Some(pos) = visible.iter().position(|c| c.id == id.as_str()) {
                ordered.push(visible.remove(pos));
            }
        }
        // Append remaining columns not in the order list
        ordered.extend(visible);
        return ordered;
    }

    visible
}

/// Builds a `Vec<Constraint>` from the resolved visible columns.
pub fn visible_constraints(columns: &[ColumnDef]) -> Vec<Constraint> {
    columns.iter().map(|c| c.default_width).collect()
}
```

- [ ] **Step 4: Add `pub mod columns;` to `src/lib.rs`**

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib columns::tests -- --nocapture`
Expected: All 8 tests PASS

- [ ] **Step 6: Run quality gate**

Run: `cargo fmt --all && cargo clippy --all-targets --all-features -- -D warnings`
Expected: clean

- [ ] **Step 7: Commit**

```bash
git add src/columns.rs src/lib.rs
git commit -m "feat(columns): add column registry and resolution for table views"
```

---

## Chunk 3: Sort Persistence Wiring

### Task 4: Wire persisted sort preferences to existing sort system

**Files:**
- Modify: `src/app.rs` — add methods to apply/save sort preferences, update sort toggle handlers

This task connects the `ViewPreferences.sort_column` to the existing `WorkloadSortState`/`PodSortState` system. When the app loads, persisted sort preferences are applied. When the user toggles sort, preferences are updated and saved.

- [ ] **Step 1: Write tests for sort preference application**

Add to `src/app.rs` tests:

```rust
#[test]
fn apply_sort_from_preferences_pods() {
    use crate::preferences::{UserPreferences, ViewPreferences};
    let mut app = AppState::default();
    let mut global = UserPreferences::default();
    global.views.insert("pods".into(), ViewPreferences {
        sort_column: Some("restarts".into()),
        sort_ascending: false,
        ..Default::default()
    });
    app.preferences = Some(global);
    app.apply_sort_from_preferences("pods");
    let sort = app.pod_sort.unwrap();
    assert_eq!(sort.column, PodSortColumn::Restarts);
    assert!(sort.descending);
}

#[test]
fn apply_sort_from_preferences_workload() {
    use crate::preferences::{UserPreferences, ViewPreferences};
    let mut app = AppState::default();
    let mut global = UserPreferences::default();
    global.views.insert("deployments".into(), ViewPreferences {
        sort_column: Some("age".into()),
        sort_ascending: true,
        ..Default::default()
    });
    app.preferences = Some(global);
    app.apply_sort_from_preferences("deployments");
    let sort = app.workload_sort.unwrap();
    assert_eq!(sort.column, WorkloadSortColumn::Age);
    assert!(!sort.descending); // ascending = !descending
}

#[test]
fn apply_sort_invalid_column_ignored() {
    use crate::preferences::{UserPreferences, ViewPreferences};
    let mut app = AppState::default();
    let mut global = UserPreferences::default();
    global.views.insert("pods".into(), ViewPreferences {
        sort_column: Some("nonexistent".into()),
        ..Default::default()
    });
    app.preferences = Some(global);
    app.apply_sort_from_preferences("pods");
    assert!(app.pod_sort.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib app::tests::apply_sort -- --nocapture`
Expected: FAIL — method doesn't exist

- [ ] **Step 3: Implement sort preference application**

Add to `AppState` impl block in `src/app.rs`:

```rust
    /// Applies persisted sort preferences for the given view key.
    pub fn apply_sort_from_preferences(&mut self, view_key: &str) {
        let prefs = crate::preferences::resolve_view_preferences(
            view_key,
            &self.preferences,
            &self.cluster_preferences,
            self.current_context_name.as_deref(),
        );
        let Some(col_id) = &prefs.sort_column else {
            return;
        };
        let descending = !prefs.sort_ascending;

        match view_key {
            "pods" => {
                let column = match col_id.as_str() {
                    "name" => PodSortColumn::Name,
                    "age" => PodSortColumn::Age,
                    "status" => PodSortColumn::Status,
                    "restarts" => PodSortColumn::Restarts,
                    _ => return,
                };
                self.pod_sort = Some(PodSortState::new(column, descending));
            }
            _ => {
                let column = match col_id.as_str() {
                    "name" => WorkloadSortColumn::Name,
                    "age" => WorkloadSortColumn::Age,
                    _ => return,
                };
                self.workload_sort = Some(WorkloadSortState::new(column, descending));
            }
        }
    }

    /// Saves the current sort state for the given view key into preferences.
    pub fn save_sort_to_preferences(&mut self, view_key: &str) {
        let (sort_column, sort_ascending) = match view_key {
            "pods" => match self.pod_sort {
                Some(s) => (Some(match s.column {
                    PodSortColumn::Name => "name",
                    PodSortColumn::Age => "age",
                    PodSortColumn::Status => "status",
                    PodSortColumn::Restarts => "restarts",
                }), !s.descending),
                None => (None, true),
            },
            _ => match self.workload_sort {
                Some(s) => (Some(match s.column {
                    WorkloadSortColumn::Name => "name",
                    WorkloadSortColumn::Age => "age",
                }), !s.descending),
                None => (None, true),
            },
        };

        let global = self.preferences.get_or_insert_with(Default::default);
        if let Some(col) = sort_column {
            let vp = global.views.entry(view_key.to_string()).or_default();
            vp.sort_column = Some(col.to_string());
            vp.sort_ascending = sort_ascending;
        } else {
            // Sort cleared — remove sort_column from preferences
            if let Some(vp) = global.views.get_mut(view_key) {
                vp.sort_column = None;
            }
        }
    }
```

Also add a `current_context_name` field to `AppState`:

```rust
    /// Active kube context name (for per-cluster preferences).
    pub current_context_name: Option<String>,
```

And in `Default::default()`: `current_context_name: None,`

- [ ] **Step 4: Wire sort toggle to save preferences**

In the sort toggle/clear methods (~lines 1840-1868), after each toggle/clear, add a call to save:

```rust
    fn set_or_toggle_pod_sort(&mut self, column: PodSortColumn) {
        self.selected_idx = 0;
        // ... existing toggle logic ...
        self.save_sort_to_preferences("pods");
    }

    fn clear_pod_sort(&mut self) {
        self.pod_sort = None;
        self.save_sort_to_preferences("pods");
    }

    fn set_or_toggle_workload_sort(&mut self, column: WorkloadSortColumn) {
        self.selected_idx = 0;
        // ... existing toggle logic ...
        let view_key = crate::columns::view_key(self.view);
        self.save_sort_to_preferences(view_key);
    }

    fn clear_workload_sort(&mut self) {
        self.workload_sort = None;
        let view_key = crate::columns::view_key(self.view);
        self.save_sort_to_preferences(view_key);
    }
```

- [ ] **Step 5: Apply persisted sort on view change**

Find the view-change handler in `src/app.rs` (the method that sets `self.view`). After setting the view, call:

```rust
    let view_key = crate::columns::view_key(self.view);
    self.apply_sort_from_preferences(view_key);
```

- [ ] **Step 6: Set context name on load/context-switch**

In `load_config_from_path`, after loading, read current context:

```rust
    app.current_context_name = kube::config::Kubeconfig::read()
        .ok()
        .and_then(|cfg| cfg.current_context);
```

In the context switch handler in `src/main.rs` (wherever `SelectContext` is handled), update `app.current_context_name`.

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test --lib app::tests -- --nocapture`
Expected: All new + existing tests PASS

- [ ] **Step 8: Run quality gate**

Run: `cargo fmt --all && cargo clippy --all-targets --all-features -- -D warnings`
Expected: clean

- [ ] **Step 9: Commit**

```bash
git add src/app.rs src/main.rs
git commit -m "feat(prefs): wire sort persistence to preferences system"
```

---

## Chunk 4: Column Toggle Action Palette Command

### Task 5: Add `:columns` command to action palette

**Files:**
- Modify: `src/ui/components/command_palette.rs` — add ViewCommand variant, column toggle entries
- Modify: `src/app.rs` — add column toggle state and handler

This task adds a new `CommandPaletteAction::ToggleColumn(String)` variant. When the user types "columns" or "toggle" in the palette while on a table view, the palette shows available columns as toggle entries.

- [ ] **Step 1: Add ToggleColumn variant to CommandPaletteAction**

In `src/ui/components/command_palette.rs`:

```rust
pub enum CommandPaletteAction {
    None,
    Navigate(AppView),
    Execute(DetailAction, ResourceRef),
    ToggleColumn(String),  // NEW: column ID to toggle
    Close,
}
```

Add new `PaletteEntry` variant:

```rust
pub enum PaletteEntry {
    Navigate(AppView),
    Action(DetailAction),
    ColumnToggle { id: String, label: String, visible: bool },  // NEW
}
```

- [ ] **Step 2: Add column entries to filtered()**

In the `filtered()` method, after actions section and before navigation section, add column toggles when query starts with "col" or is "columns" or "toggle":

```rust
    // Column toggles (when query matches)
    if let Some(columns_info) = &self.columns_info {
        let q_lower = self.query.to_ascii_lowercase();
        let show_columns = q_lower.is_empty()
            || fuzzy_match("columns", &q_lower)
            || fuzzy_match("toggle", &q_lower)
            || columns_info.iter().any(|(_, label, _)| fuzzy_match(&label.to_ascii_lowercase(), &q_lower));

        if show_columns {
            for (id, label, visible) in columns_info {
                if q_lower.is_empty()
                    || fuzzy_match("columns", &q_lower)
                    || fuzzy_match("toggle", &q_lower)
                    || fuzzy_match(&label.to_ascii_lowercase(), &q_lower)
                {
                    result.push(PaletteEntry::ColumnToggle {
                        id: id.clone(),
                        label: label.clone(),
                        visible: *visible,
                    });
                }
            }
        }
    }
```

Add `columns_info` field to `CommandPalette`:

```rust
pub struct CommandPalette {
    query: String,
    selected_index: usize,
    is_open: bool,
    cached_filtered: RefCell<Option<Vec<PaletteEntry>>>,
    resource_context: Option<ResourceActionContext>,
    columns_info: Option<Vec<(String, String, bool)>>,  // NEW: (id, label, visible)
}
```

Add method to set column info:

```rust
    pub fn set_columns_info(&mut self, info: Option<Vec<(String, String, bool)>>) {
        self.columns_info = info;
        self.cached_filtered.borrow_mut().take();
    }
```

- [ ] **Step 3: Handle ToggleColumn in app.rs**

In the command palette action handler in `src/app.rs`, add:

```rust
    CommandPaletteAction::ToggleColumn(column_id) => {
        let view_key = crate::columns::view_key(self.view);
        let global = self.preferences.get_or_insert_with(Default::default);
        let vp = global.views.entry(view_key.to_string()).or_default();
        if let Some(pos) = vp.hidden_columns.iter().position(|c| c == &column_id) {
            vp.hidden_columns.remove(pos); // unhide
        } else {
            vp.hidden_columns.push(column_id); // hide
        }
        // Don't close palette — user may want to toggle multiple columns
        self.command_palette.cached_filtered.borrow_mut().take();
    }
```

- [ ] **Step 4: Populate column info when opening palette**

When the command palette opens, pass current view's column info:

```rust
    if let Some(registry) = crate::columns::columns_for_view(self.view) {
        let prefs = crate::preferences::resolve_view_preferences(
            &crate::columns::view_key(self.view),
            &self.preferences,
            &self.cluster_preferences,
            self.current_context_name.as_deref(),
        );
        let info: Vec<(String, String, bool)> = registry
            .iter()
            .filter(|c| c.hideable)
            .map(|c| {
                let visible = c.default_visible && !prefs.hidden_columns.contains(&c.id.to_string());
                (c.id.to_string(), c.label.to_string(), visible)
            })
            .collect();
        self.command_palette.set_columns_info(Some(info));
    } else {
        self.command_palette.set_columns_info(None);
    }
```

- [ ] **Step 5: Render column toggle entries in palette**

In the palette `render()` method, handle `ColumnToggle` entries with a checkbox-style display:

```rust
    PaletteEntry::ColumnToggle { label, visible, .. } => {
        let check = if *visible { "[x]" } else { "[ ]" };
        ListItem::new(Line::from(vec![
            Span::styled(format!(" {check} "), style),
            Span::styled(label.as_str(), style),
        ]))
    }
```

- [ ] **Step 6: Handle Enter on ColumnToggle in handle_key**

In the `Enter` match arm, add:

```rust
    PaletteEntry::ColumnToggle { id, .. } => {
        CommandPaletteAction::ToggleColumn(id.clone())
    }
```

- [ ] **Step 7: Run quality gate**

Run: `cargo fmt --all && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-targets --all-features`
Expected: clean, all tests pass

- [ ] **Step 8: Commit**

```bash
git add src/ui/components/command_palette.rs src/app.rs
git commit -m "feat(columns): add column toggle via action palette"
```

---

## Chunk 5: View Rendering Integration

### Task 6: Update representative views to use column registry

**Files:**
- Modify: `src/ui/views/deployments.rs` — use resolved columns for header + row rendering
- Modify: `src/ui/views/pods.rs` — same
- Modify: `src/ui/views/nodes.rs` — same

This is the most complex task. Each view's `render_*` function needs to:
1. Accept resolved `ViewPreferences` (or the resolved column list)
2. Build header from visible columns only
3. Build rows from visible columns only
4. Use `visible_constraints()` for widths

Start with deployments as the template, then apply the pattern to pods and nodes.

- [ ] **Step 1: Update render_deployments signature**

Add `visible_columns: &[ColumnDef]` parameter:

```rust
pub fn render_deployments(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
    sort: Option<WorkloadSortState>,
    visible_columns: &[ColumnDef],
)
```

- [ ] **Step 2: Build header dynamically from visible columns**

Replace hardcoded header with:

```rust
    let header_cells: Vec<Cell> = visible_columns.iter().map(|col| {
        let label = match col.id {
            "name" => format!("  {}", workload_sort_header(col.label, sort, WorkloadSortColumn::Name)),
            "age" => workload_sort_header(col.label, sort, WorkloadSortColumn::Age),
            _ => col.label.to_string(),
        };
        Cell::from(Span::styled(label, theme.header_style()))
    }).collect();
    let header = Row::new(header_cells).height(1).style(theme.header_style());
```

- [ ] **Step 3: Build rows dynamically from visible columns**

Replace hardcoded row cells with a column-driven approach:

```rust
    let cells: Vec<Cell> = visible_columns.iter().map(|col| {
        match col.id {
            "name" => Cell::from(Line::from(vec![
                Span::styled("  ", name_style),
                Span::styled(deploy.name.as_str(), name_style),
            ])),
            "namespace" => Cell::from(Span::styled(deploy.namespace.as_str(), dim_style)),
            "ready" => Cell::from(Span::styled(deploy.ready.as_str(), ready_style)),
            "updated" => Cell::from(Span::styled(
                format_small_int(i64::from(deploy.updated_replicas)), dim_style,
            )),
            "available" => Cell::from(Span::styled(
                format_small_int(i64::from(deploy.available_replicas)), dim_style,
            )),
            "age" => Cell::from(Span::styled(age_text.as_ref(), theme.inactive_style())),
            "image" => Cell::from(Span::styled(image_text.as_ref(), muted_style)),
            _ => Cell::from(""),
        }
    }).collect();
    rows.push(Row::new(cells).style(row_style));
```

- [ ] **Step 4: Use visible_constraints for widths**

Replace hardcoded constraint array:

```rust
    let constraints = crate::columns::visible_constraints(visible_columns);
    let widths = responsive_table_widths_vec(area.width, &constraints);
```

Note: `responsive_table_widths` currently uses const generics `[Constraint; N]`. We need a `Vec<Constraint>` version. Add to `src/ui/mod.rs`:

```rust
pub(crate) fn responsive_table_widths_vec(
    area_width: u16,
    wide: &[Constraint],
) -> Vec<Constraint> {
    // Same logic as responsive_table_widths but with Vec
}
```

- [ ] **Step 5: Update caller sites**

In the main render dispatch (likely `src/ui/mod.rs` or `src/ui/views/mod.rs`), resolve columns before calling `render_deployments`:

```rust
    let visible_columns = if let Some(registry) = columns_for_view(app.view()) {
        let prefs = resolve_view_preferences(
            view_key(app.view()),
            &app.preferences,
            &app.cluster_preferences,
            app.current_context_name.as_deref(),
        );
        resolve_columns(registry, &prefs)
    } else {
        Vec::new()
    };
    render_deployments(frame, area, snapshot, app.selected_idx(), query, sort, &visible_columns);
```

- [ ] **Step 6: Apply same pattern to pods.rs**

Pods has extra complexity with `PodSortState` column indicators. Update header to check sort column against column ID.

- [ ] **Step 7: Apply same pattern to nodes.rs**

Nodes is similar to deployments with `WorkloadSortState`.

- [ ] **Step 8: Run quality gate**

Run: `cargo fmt --all && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-targets --all-features`
Expected: clean, all tests pass

- [ ] **Step 9: Commit**

```bash
git add src/ui/views/deployments.rs src/ui/views/pods.rs src/ui/views/nodes.rs src/ui/mod.rs
git commit -m "feat(columns): integrate column registry into deployments, pods, nodes views"
```

---

### Task 7: Update remaining views with column registries

**Files:**
- Modify: remaining `src/ui/views/*.rs` files that have table columns
- Modify: `src/columns.rs` — add registries for remaining views if not yet defined

This task applies the same column-driven rendering pattern to the remaining views. For views without a column registry yet, add one to `src/columns.rs` and wire it up.

Views to update: statefulsets, daemonsets, services, jobs, cronjobs, and any others that currently use hardcoded columns.

- [ ] **Step 1: Add remaining column registries to `src/columns.rs`**

Add registries for views not yet covered (endpoints, ingresses, configmaps, secrets, etc.). Follow the same pattern as existing registries.

- [ ] **Step 2: Update each view's render function**

Apply the same pattern from Task 6 to each view:
1. Add `visible_columns: &[ColumnDef]` parameter
2. Build header dynamically
3. Build rows dynamically
4. Use `visible_constraints`

- [ ] **Step 3: Update caller sites**

Ensure each view's render call passes resolved columns.

- [ ] **Step 4: Run quality gate**

Run: `cargo fmt --all && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-targets --all-features`
Expected: clean, all tests pass

- [ ] **Step 5: Commit**

```bash
git add src/ui/views/ src/columns.rs
git commit -m "feat(columns): integrate column registry into all remaining table views"
```

---

## Chunk 6: Save Triggers & Context Wiring

### Task 8: Wire save triggers and context name tracking

**Files:**
- Modify: `src/main.rs` — save config after preference changes, set context name on switch
- Modify: `src/app.rs` — add `needs_config_save` flag

- [ ] **Step 1: Add dirty flag to AppState**

```rust
    /// When true, config should be saved at next convenient point.
    pub needs_config_save: bool,
```

Default: `false`.

- [ ] **Step 2: Set dirty flag on preference changes**

In `save_sort_to_preferences()`, at the end: `self.needs_config_save = true;`

In the column toggle handler: `self.needs_config_save = true;`

In `toggle_nav_group()`: `self.needs_config_save = true;`

- [ ] **Step 3: Drain dirty flag in event loop**

In `src/main.rs` event loop, after processing events, check:

```rust
    if app.needs_config_save {
        app.needs_config_save = false;
        save_config(&app);
    }
```

- [ ] **Step 4: Update context switch to set context name**

In the context switch handler in `src/main.rs`, after switching context:

```rust
    app.current_context_name = Some(context_name.clone());
```

- [ ] **Step 5: Run quality gate**

Run: `cargo fmt --all && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-targets --all-features`
Expected: clean, all tests pass

- [ ] **Step 6: Commit**

```bash
git add src/app.rs src/main.rs
git commit -m "feat(prefs): wire save triggers and context name tracking"
```

---

## Chunk 7: Help Overlay & Plan Update

### Task 9: Update help overlay and plan.md

**Files:**
- Modify: `src/ui/components/help_overlay.rs` — add columns shortcut hint
- Modify: `plan.md` — mark M14 complete

- [ ] **Step 1: Add column toggle hint to help overlay**

In the keybindings section, add:

```
(":columns", "Toggle columns (in palette)")
```

- [ ] **Step 2: Update plan.md**

Mark M14 as completed with test count and what shipped.

- [ ] **Step 3: Run final quality gate**

Run: `cargo fmt --all && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-targets --all-features`
Expected: clean, all tests pass

- [ ] **Step 4: Commit**

```bash
git add src/ui/components/help_overlay.rs plan.md
git commit -m "docs: mark M14 View Personalization complete"
```
