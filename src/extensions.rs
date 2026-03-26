//! Config-defined extension actions and substitution helpers.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::app::ResourceRef;

const EXTENSIONS_FILE_NAME: &str = "extensions.yaml";
const LABEL_SEPARATOR: &str = ",";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionExecutionMode {
    Background,
    Foreground,
    Silent,
}

impl ExtensionExecutionMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Background => "BG",
            Self::Foreground => "FG",
            Self::Silent => "Run",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionCommandConfig {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionActionConfig {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub resource_kinds: Vec<String>,
    #[serde(default)]
    pub shortcut: Option<String>,
    pub mode: ExtensionExecutionMode,
    pub command: ExtensionCommandConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionsConfig {
    #[serde(default)]
    pub actions: Vec<ExtensionActionConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedExtensionAction {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub aliases: Vec<String>,
    pub resource_kinds: Vec<String>,
    pub shortcut: Option<String>,
    pub mode: ExtensionExecutionMode,
    pub command: ExtensionCommandConfig,
}

impl LoadedExtensionAction {
    pub fn matches_resource(&self, resource: &ResourceRef) -> bool {
        self.resource_kinds.is_empty()
            || self
                .resource_kinds
                .iter()
                .any(|kind| kind == "*" || kind.eq_ignore_ascii_case(resource.kind()))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionRegistry {
    actions: Vec<LoadedExtensionAction>,
}

impl ExtensionRegistry {
    pub fn actions(&self) -> &[LoadedExtensionAction] {
        &self.actions
    }

    pub fn get(&self, id: &str) -> Option<&LoadedExtensionAction> {
        self.actions.iter().find(|action| action.id == id)
    }

    pub fn palette_actions_for(&self, resource: &ResourceRef) -> Vec<LoadedExtensionAction> {
        self.actions
            .iter()
            .filter(|action| action.matches_resource(resource))
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionLoadResult {
    pub registry: ExtensionRegistry,
    pub warnings: Vec<String>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionSubstitutionContext {
    pub name: String,
    pub namespace: Option<String>,
    pub kind: String,
    pub context: Option<String>,
    pub labels: Vec<(String, String)>,
}

impl ExtensionSubstitutionContext {
    pub fn from_resource(
        resource: &ResourceRef,
        current_context: Option<&str>,
        labels: Vec<(String, String)>,
    ) -> Self {
        let mut labels = labels;
        labels.sort();
        labels.dedup();
        Self {
            name: resource.name().to_string(),
            namespace: resource.namespace().map(str::to_string),
            kind: resource.kind().to_string(),
            context: current_context.map(str::to_string),
            labels,
        }
    }

    pub fn labels_value(&self) -> String {
        self.labels
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(LABEL_SEPARATOR)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedExtensionCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub preview: String,
}

pub fn extensions_config_path() -> PathBuf {
    dirs::config_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_default()
        .join("kubectui")
        .join(EXTENSIONS_FILE_NAME)
}

pub fn load_extensions_config_from_path(path: &Path) -> Result<ExtensionsConfig, String> {
    let content = fs::read_to_string(path).map_err(|err| {
        format!(
            "failed to read extensions config '{}': {err}",
            path.display()
        )
    })?;
    serde_yaml::from_str::<ExtensionsConfig>(&content).map_err(|err| {
        format!(
            "failed to parse extensions config '{}': {err}",
            path.display()
        )
    })
}

pub fn load_extensions_registry() -> ExtensionLoadResult {
    let path = extensions_config_path();
    let Some(parent) = path.parent() else {
        return ExtensionLoadResult {
            registry: ExtensionRegistry::default(),
            warnings: vec![format!(
                "extensions config path '{}' has no parent directory",
                path.display()
            )],
            path,
        };
    };
    let _ = fs::create_dir_all(parent);
    if !path.exists() {
        return ExtensionLoadResult {
            registry: ExtensionRegistry::default(),
            warnings: Vec::new(),
            path,
        };
    }

    match load_extensions_config_from_path(&path) {
        Ok(config) => validate_extensions(config, path),
        Err(err) => ExtensionLoadResult {
            registry: ExtensionRegistry::default(),
            warnings: vec![err],
            path,
        },
    }
}

pub fn prepare_command(
    action: &LoadedExtensionAction,
    context: &ExtensionSubstitutionContext,
) -> Result<PreparedExtensionCommand, String> {
    let program = substitute_template(&action.command.program, context);
    let args = action
        .command
        .args
        .iter()
        .map(|arg| substitute_template(arg, context))
        .collect::<Vec<_>>();
    let cwd = action
        .command
        .cwd
        .as_ref()
        .map(|cwd| PathBuf::from(substitute_template(cwd, context)));
    let env = action
        .command
        .env
        .iter()
        .map(|(key, value)| (key.clone(), substitute_template(value, context)))
        .collect::<BTreeMap<_, _>>();

    if program.trim().is_empty() {
        return Err(format!(
            "extension '{}' resolved to an empty program",
            action.title
        ));
    }

    Ok(PreparedExtensionCommand {
        preview: shell_preview(&program, &args),
        program,
        args,
        cwd,
        env,
    })
}

fn validate_extensions(config: ExtensionsConfig, path: PathBuf) -> ExtensionLoadResult {
    let mut actions = Vec::new();
    let mut warnings = Vec::new();
    let mut seen_ids = BTreeSet::new();

    for action in config.actions {
        let id = action.id.trim();
        let title = action.title.trim();
        let program = action.command.program.trim();
        if id.is_empty() {
            warnings.push("skipping extension action with empty id".to_string());
            continue;
        }
        if !seen_ids.insert(id.to_string()) {
            warnings.push(format!("skipping duplicate extension id '{id}'"));
            continue;
        }
        if title.is_empty() {
            warnings.push(format!("skipping extension '{id}' with empty title"));
            continue;
        }
        if program.is_empty() {
            warnings.push(format!("skipping extension '{id}' with empty program"));
            continue;
        }

        let mut aliases = action
            .aliases
            .into_iter()
            .map(|alias| alias.trim().to_ascii_lowercase())
            .filter(|alias| !alias.is_empty())
            .collect::<Vec<_>>();
        if !aliases
            .iter()
            .any(|alias| alias == &title.to_ascii_lowercase())
        {
            aliases.push(title.to_ascii_lowercase());
        }
        aliases.sort();
        aliases.dedup();

        let mut resource_kinds = action
            .resource_kinds
            .into_iter()
            .map(|kind| kind.trim().to_string())
            .filter(|kind| !kind.is_empty())
            .collect::<Vec<_>>();
        resource_kinds.sort();
        resource_kinds.dedup();

        actions.push(LoadedExtensionAction {
            id: id.to_string(),
            title: title.to_string(),
            description: action.description.filter(|value| !value.trim().is_empty()),
            aliases,
            resource_kinds,
            shortcut: action.shortcut.filter(|value| !value.trim().is_empty()),
            mode: action.mode,
            command: ExtensionCommandConfig {
                program: program.to_string(),
                args: action.command.args,
                cwd: action.command.cwd.filter(|value| !value.trim().is_empty()),
                env: action.command.env,
            },
        });
    }

    ExtensionLoadResult {
        registry: ExtensionRegistry { actions },
        warnings,
        path,
    }
}

fn substitute_template(value: &str, context: &ExtensionSubstitutionContext) -> String {
    let labels = context.labels_value();
    [
        (
            "$NAMESPACE",
            context.namespace.as_deref().unwrap_or_default(),
        ),
        ("$KIND", context.kind.as_str()),
        ("$CONTEXT", context.context.as_deref().unwrap_or_default()),
        ("$LABELS", labels.as_str()),
        ("$NAME", context.name.as_str()),
    ]
    .into_iter()
    .fold(value.to_string(), |acc, (needle, replacement)| {
        acc.replace(needle, replacement)
    })
}

fn shell_preview(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(shell_escape)
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_escape(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | ':' | '.' | '='))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_registry_skips_invalid_entries() {
        let result = validate_extensions(
            ExtensionsConfig {
                actions: vec![
                    ExtensionActionConfig {
                        id: "".into(),
                        title: "Bad".into(),
                        description: None,
                        aliases: Vec::new(),
                        resource_kinds: Vec::new(),
                        shortcut: None,
                        mode: ExtensionExecutionMode::Background,
                        command: ExtensionCommandConfig {
                            program: "kubectl".into(),
                            args: vec!["get".into()],
                            cwd: None,
                            env: BTreeMap::new(),
                        },
                    },
                    ExtensionActionConfig {
                        id: "grafana".into(),
                        title: "Open in Grafana".into(),
                        description: None,
                        aliases: vec!["grafana".into()],
                        resource_kinds: vec!["Deployment".into()],
                        shortcut: None,
                        mode: ExtensionExecutionMode::Foreground,
                        command: ExtensionCommandConfig {
                            program: "open".into(),
                            args: vec!["https://grafana".into()],
                            cwd: None,
                            env: BTreeMap::new(),
                        },
                    },
                ],
            },
            PathBuf::from("/tmp/extensions.yaml"),
        );

        assert_eq!(result.registry.actions().len(), 1);
        assert_eq!(result.registry.actions()[0].id, "grafana");
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn matches_resource_kind_case_insensitively() {
        let action = LoadedExtensionAction {
            id: "grafana".into(),
            title: "Grafana".into(),
            description: None,
            aliases: vec!["grafana".into()],
            resource_kinds: vec!["deployment".into(), "StatefulSet".into()],
            shortcut: None,
            mode: ExtensionExecutionMode::Foreground,
            command: ExtensionCommandConfig {
                program: "open".into(),
                args: Vec::new(),
                cwd: None,
                env: BTreeMap::new(),
            },
        };

        assert!(action.matches_resource(&ResourceRef::Deployment("api".into(), "prod".into())));
        assert!(action.matches_resource(&ResourceRef::StatefulSet("db".into(), "prod".into())));
        assert!(!action.matches_resource(&ResourceRef::Pod("api".into(), "prod".into())));
    }

    #[test]
    fn prepare_command_substitutes_known_variables() {
        let action = LoadedExtensionAction {
            id: "describe".into(),
            title: "Describe".into(),
            description: None,
            aliases: vec!["describe".into()],
            resource_kinds: vec!["Pod".into()],
            shortcut: None,
            mode: ExtensionExecutionMode::Background,
            command: ExtensionCommandConfig {
                program: "kubectl".into(),
                args: vec![
                    "describe".into(),
                    "$KIND/$NAME".into(),
                    "-n".into(),
                    "$NAMESPACE".into(),
                    "--labels=$LABELS".into(),
                ],
                cwd: Some("/tmp/$CONTEXT".into()),
                env: BTreeMap::from([("CTX".into(), "$CONTEXT".into())]),
            },
        };
        let prepared = prepare_command(
            &action,
            &ExtensionSubstitutionContext::from_resource(
                &ResourceRef::Pod("api-0".into(), "prod".into()),
                Some("staging"),
                vec![("app".into(), "api".into()), ("tier".into(), "web".into())],
            ),
        )
        .expect("prepared command");

        assert_eq!(prepared.program, "kubectl");
        assert_eq!(prepared.args[1], "Pod/api-0");
        assert_eq!(prepared.args[3], "prod");
        assert_eq!(prepared.args[4], "--labels=app=api,tier=web");
        assert_eq!(prepared.cwd, Some(PathBuf::from("/tmp/staging")));
        assert_eq!(prepared.env.get("CTX").map(String::as_str), Some("staging"));
        assert!(prepared.preview.contains("kubectl"));
    }
}
