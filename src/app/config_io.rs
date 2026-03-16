//! Config serialization and persistence for application state.

use std::{collections::HashMap, fs, path::Path};

use serde::{Deserialize, Serialize};

use super::{AppState, views::NavGroup};
use crate::{
    preferences::{ClusterPreferences, UserPreferences},
    workbench::DEFAULT_WORKBENCH_HEIGHT,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct AppConfig {
    namespace: String,
    #[serde(default)]
    theme: Option<String>,
    /// Auto-refresh interval in seconds (0 = disabled, default = 30).
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

fn default_refresh_interval() -> u64 {
    30
}

fn default_workbench_height() -> u16 {
    DEFAULT_WORKBENCH_HEIGHT
}

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

/// Loads app state config from a given path.
pub fn load_config_from_path(path: &Path) -> AppState {
    let mut app = AppState::default();

    if let Ok(content) = fs::read_to_string(path)
        && let Ok(cfg) = serde_json::from_str::<AppConfig>(&content)
    {
        if !cfg.namespace.trim().is_empty() {
            app.set_namespace(cfg.namespace);
        }
        if let Some(theme_name) = &cfg.theme {
            let idx = match theme_name.to_lowercase().as_str() {
                "nord" => 1,
                "dracula" => 2,
                "catppuccin" | "mocha" => 3,
                "light" => 4,
                _ => 0,
            };
            crate::ui::theme::set_active_theme(idx);
        }
        app.refresh_interval_secs = cfg.refresh_interval_secs;
        app.workbench
            .set_open_and_height(cfg.workbench_open, cfg.workbench_height);
        for name in &cfg.collapsed_nav_groups {
            if let Some(g) = nav_group_from_str(name) {
                app.collapsed_groups.insert(g);
            }
        }
        app.preferences = cfg.preferences;
        app.cluster_preferences = cfg.clusters;
    }

    app.current_context_name = kube::config::Kubeconfig::read()
        .ok()
        .and_then(|cfg| cfg.current_context);

    app
}

/// Saves app namespace config to a given path.
pub fn save_config_to_path(app: &AppState, path: &Path) {
    let theme_name = crate::ui::theme::active_theme().name;
    let collapsed: Vec<String> = app
        .collapsed_groups
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

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let serialized = serde_json::to_string(&cfg).unwrap_or_else(|_| "{}".to_string());
    let tmp = path.with_extension("tmp");
    if fs::write(&tmp, &serialized).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

/// Loads app config from ~/.kube/kubectui-config.json.
pub fn load_config() -> AppState {
    let path = dirs::home_dir()
        .unwrap_or_default()
        .join(".kube")
        .join("kubectui-config.json");
    load_config_from_path(&path)
}

/// Saves app config to ~/.kube/kubectui-config.json.
pub fn save_config(app: &AppState) {
    let path = dirs::home_dir()
        .unwrap_or_default()
        .join(".kube")
        .join("kubectui-config.json");
    save_config_to_path(app, &path);
}
