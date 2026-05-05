//! Config serialization and persistence for application state.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{AppState, views::NavGroup};
use crate::{
    ai_actions::AiConfig,
    k8s::exec::ExecConfig,
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
    #[serde(default)]
    ai: Option<AiConfig>,
    #[serde(default)]
    exec: Option<ExecConfig>,
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

pub fn config_path() -> Option<PathBuf> {
    default_config_path(dirs::home_dir())
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
            .set_open_and_height(false, cfg.workbench_height);
        if let Some(groups) = cfg.collapsed_nav_groups {
            app.collapsed_groups = groups
                .iter()
                .filter_map(|group| nav_group_from_config(group))
                .collect();
            app.sync_sidebar_cursor_to_view();
        }
        app.preferences = cfg.preferences;
        app.cluster_preferences = cfg.clusters;
        app.ai_config = cfg.ai;
        if let Some(exec_config) = cfg.exec {
            app.exec_config = exec_config;
        }
    }

    app.current_context_name = kube::config::Kubeconfig::read()
        .ok()
        .and_then(|cfg| cfg.current_context);

    app
}

pub fn load_ai_config_from_path(path: &Path) -> Result<Option<AiConfig>, String> {
    let content = fs::read_to_string(path)
        .map_err(|err| format!("failed to read app config '{}': {err}", path.display()))?;
    let cfg = serde_json::from_str::<AppConfig>(&content)
        .map_err(|err| format!("failed to parse app config '{}': {err}", path.display()))?;
    Ok(cfg.ai)
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
        workbench_height: app.workbench.height,
        collapsed_nav_groups: Some(collapsed_nav_groups),
        preferences: app.preferences.clone(),
        clusters: app.cluster_preferences.clone(),
        ai: app.ai_config.clone(),
        exec: Some(app.exec_config.clone()),
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
    match config_path() {
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
    match config_path() {
        Some(path) => save_config_to_path(app, &path),
        None => log::warn!("home directory is unavailable; skipping app config save"),
    }
}

#[cfg(test)]
mod tests {
    use super::{default_config_path, load_ai_config_from_path, load_config_from_path};
    use crate::ai_actions::AiProviderKind;
    use std::{fs, path::PathBuf};

    #[test]
    fn default_config_path_uses_absolute_home_only() {
        assert_eq!(
            default_config_path(Some(PathBuf::from("/Users/tester"))),
            Some(PathBuf::from("/Users/tester/.kube/kubectui-config.json"))
        );
        assert_eq!(default_config_path(None), None);
    }

    #[test]
    fn load_config_reads_native_ai_config() {
        let path =
            std::env::temp_dir().join(format!("kubectui-ai-config-{}.json", std::process::id()));
        fs::write(
            &path,
            r#"{"namespace":"all","ai":{"provider":"claude_cli","command":"claude"}}"#,
        )
        .expect("write config");

        let app = load_config_from_path(&path);
        let ai = app.ai_config.expect("ai config");
        assert_eq!(ai.providers.len(), 1);
        let ai = &ai.providers[0];
        assert_eq!(ai.provider, AiProviderKind::ClaudeCli);
        assert_eq!(ai.command.as_deref(), Some("claude"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn load_config_reads_multiple_native_ai_providers() {
        let path = std::env::temp_dir().join(format!(
            "kubectui-ai-config-multi-{}.json",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"{"namespace":"all","ai":{"providers":[{"provider":"codex_cli"},{"provider":"claude_cli"}]}}"#,
        )
        .expect("write config");

        let app = load_config_from_path(&path);
        let ai = app.ai_config.expect("ai config");
        assert_eq!(ai.providers.len(), 2);
        assert_eq!(ai.providers[0].provider, AiProviderKind::CodexCli);
        assert_eq!(ai.providers[1].provider, AiProviderKind::ClaudeCli);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn load_ai_config_from_path_reads_only_native_ai_block() {
        let path = std::env::temp_dir().join(format!(
            "kubectui-ai-config-reload-{}.json",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"{"namespace":"prod","refresh_interval_secs":15,"ai":{"providers":[{"provider":"codex_cli"}]}}"#,
        )
        .expect("write config");

        let ai = load_ai_config_from_path(&path)
            .expect("config parses")
            .expect("ai config");
        assert_eq!(ai.providers.len(), 1);
        assert_eq!(ai.providers[0].provider, AiProviderKind::CodexCli);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn load_config_reads_exec_config() {
        let path =
            std::env::temp_dir().join(format!("kubectui-exec-config-{}.json", std::process::id()));
        fs::write(
            &path,
            r#"{"namespace":"all","exec":{"shells":["/usr/bin/fish","/bin/bash"],"login":true}}"#,
        )
        .expect("write config");

        let app = load_config_from_path(&path);
        assert_eq!(
            app.exec_config.shells,
            vec!["/usr/bin/fish".to_string(), "/bin/bash".to_string()]
        );
        assert!(app.exec_config.login);

        let _ = fs::remove_file(path);
    }
}
