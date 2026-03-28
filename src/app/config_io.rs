//! Config serialization and persistence for application state.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

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
    icon_mode: Option<String>,
    #[serde(default)]
    collapsed_nav_groups: Option<Vec<String>>,
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

fn default_config_path(base_dir: Option<PathBuf>) -> Option<PathBuf> {
    base_dir.map(|base| base.join(".kube").join("kubectui-config.json"))
}

fn nav_group_from_config(name: &str) -> Option<NavGroup> {
    match name {
        "Overview" => Some(NavGroup::Overview),
        "Workloads" => Some(NavGroup::Workloads),
        "Network" => Some(NavGroup::Network),
        "Config" => Some(NavGroup::Config),
        "Storage" => Some(NavGroup::Storage),
        "Helm" => Some(NavGroup::Helm),
        "FluxCD" => Some(NavGroup::FluxCD),
        "Access Control" => Some(NavGroup::AccessControl),
        "Custom Resources" => Some(NavGroup::CustomResources),
        _ => None,
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
        if let Some(icon_mode) = &cfg.icon_mode {
            crate::icons::set_icon_mode(crate::icons::parse_icon_mode(icon_mode));
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
        if let Some(groups) = cfg.collapsed_nav_groups {
            app.collapsed_groups = groups
                .iter()
                .filter_map(|group| nav_group_from_config(group))
                .collect();
            app.sync_sidebar_cursor_to_view();
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
    let collapsed_nav_groups = crate::app::sidebar::all_groups()
        .filter(|group| app.collapsed_groups.contains(group))
        .map(|group| group.label().to_string())
        .collect();
    let cfg = AppConfig {
        namespace: app.current_namespace.clone(),
        theme: Some(theme_name.to_string()),
        icon_mode: Some(crate::icons::icon_mode_name(crate::icons::active_icon_mode()).to_string()),
        refresh_interval_secs: app.refresh_interval_secs,
        workbench_open: app.workbench.open,
        workbench_height: app.workbench.height,
        collapsed_nav_groups: Some(collapsed_nav_groups),
        preferences: app.preferences.clone(),
        clusters: app.cluster_preferences.clone(),
    };

    if let Some(parent) = path.parent()
        && let Err(err) = fs::create_dir_all(parent)
    {
        log::warn!(
            "failed to create config directory '{}': {err}",
            parent.display()
        );
        return;
    }

    let serialized = match serde_json::to_string(&cfg) {
        Ok(serialized) => serialized,
        Err(err) => {
            log::warn!(
                "failed to serialize app config for '{}': {err}",
                path.display()
            );
            return;
        }
    };
    let tmp = path.with_extension("tmp");
    if let Err(err) = fs::write(&tmp, &serialized) {
        log::warn!("failed to write temp config '{}': {err}", tmp.display());
        return;
    }
    if let Err(err) = fs::rename(&tmp, path) {
        log::warn!(
            "failed to replace config '{}' from '{}': {err}",
            path.display(),
            tmp.display()
        );
        let _ = fs::remove_file(&tmp);
    }
}

/// Loads app config from ~/.kube/kubectui-config.json.
pub fn load_config() -> AppState {
    match default_config_path(dirs::home_dir()) {
        Some(path) => load_config_from_path(&path),
        None => {
            log::warn!("home directory is unavailable; skipping app config load");
            AppState {
                current_context_name: kube::config::Kubeconfig::read()
                    .ok()
                    .and_then(|cfg| cfg.current_context),
                ..AppState::default()
            }
        }
    }
}

/// Saves app config to ~/.kube/kubectui-config.json.
pub fn save_config(app: &AppState) {
    match default_config_path(dirs::home_dir()) {
        Some(path) => save_config_to_path(app, &path),
        None => log::warn!("home directory is unavailable; skipping app config save"),
    }
}

#[cfg(test)]
mod tests {
    use super::default_config_path;
    use std::path::PathBuf;

    #[test]
    fn default_config_path_uses_absolute_home_only() {
        assert_eq!(
            default_config_path(Some(PathBuf::from("/Users/tester"))),
            Some(PathBuf::from("/Users/tester/.kube/kubectui-config.json"))
        );
        assert_eq!(default_config_path(None), None);
    }
}
