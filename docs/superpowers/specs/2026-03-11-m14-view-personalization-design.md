# M14: View Personalization and Workspace Persistence

## Overview

Make KubecTUI remember how each user works. Persist sort preferences, column visibility/order, workbench state, and nav group collapse state. Support global defaults with per-cluster overrides.

## Data Model

### Core Types (`src/preferences.rs`)

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ViewPreferences {
    pub sort_column: Option<String>,
    #[serde(default = "default_true")]
    pub sort_ascending: bool,
    #[serde(default)]
    pub hidden_columns: Vec<String>,
    pub column_order: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserPreferences {
    #[serde(default)]
    pub views: HashMap<String, ViewPreferences>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClusterPreferences {
    #[serde(default)]
    pub views: HashMap<String, ViewPreferences>,
}
```

### Resolution

`clusters[context_name].views[view]` → `preferences.views[view]` → hardcoded defaults

Per-field merge: cluster override field takes precedence if set, otherwise fall through to global, then defaults.

## Column Definition System

```rust
#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub id: &'static str,
    pub label: &'static str,
    pub default_width: Constraint,
    pub hideable: bool,
    pub default_visible: bool,
}
```

Each view declares a `&'static [ColumnDef]` registry. At render time:
1. Get column registry for view
2. Resolve ViewPreferences (cluster → global → defaults)
3. Filter hidden columns, reorder if column_order set
4. Build Constraint array from visible columns
5. Render header + cells using visible column list

Sort column IDs reference the same column IDs. Invalid IDs silently ignored.

## Config Expansion

File: `~/.kube/kubectui-config.json` (unchanged path)

```rust
struct AppConfig {
    namespace: String,
    theme: Option<String>,
    refresh_interval_secs: u64,
    workbench_open: bool,
    workbench_height: u16,
    collapsed_nav_groups: Option<Vec<String>>,    // NEW
    preferences: Option<UserPreferences>,          // NEW
    clusters: Option<HashMap<String, ClusterPreferences>>, // NEW
}
```

All new fields `Option` + `#[serde(default)]` for backward compatibility.

## Column Toggle UX

Action palette command: `:columns` or searching "columns" / "toggle columns".
Opens a checklist overlay showing available columns for the current view.
Non-hideable columns (e.g., "name") shown but disabled.
Changes persist immediately.

## Save Triggers

- Sort change
- Column toggle
- Workbench resize/open/close (existing)
- App exit (existing)

Atomic write via .tmp + rename (existing pattern).

## Invalid Config Handling

- Unknown sort_column → no sort (default)
- Unknown column IDs in hidden_columns/column_order → silently skipped
- Missing fields → serde defaults
