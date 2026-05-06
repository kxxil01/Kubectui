//! Config-defined extension actions and substitution helpers.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::app::ResourceRef;

const EXTENSIONS_FILE_NAME: &str = "extensions.yaml";
const LABEL_SEPARATOR: &str = ",";
pub const DEFAULT_EXTENSION_TIMEOUT_SECS: u64 = 120;
const MIN_EXTENSION_TIMEOUT_SECS: u64 = 1;
const MAX_EXTENSION_TIMEOUT_SECS: u64 = 3600;

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
    #[serde(default)]
    pub timeout_secs: Option<u64>,
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionsConfig {
    #[serde(default)]
    pub actions: Vec<ExtensionActionConfig>,
}

#[derive(Debug, Clone, PartialEq)]
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

    pub fn badge_label(&self) -> String {
        self.mode.label().to_string()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
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

#[derive(Debug, Clone, Default, PartialEq)]
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
    pub timeout: Duration,
    pub preview: String,
}

fn extensions_config_path_from_base(base_dir: Option<PathBuf>) -> Option<PathBuf> {
    base_dir.map(|base| base.join("kubectui").join(EXTENSIONS_FILE_NAME))
}

pub fn extensions_config_path() -> Option<PathBuf> {
    extensions_config_path_from_base(dirs::config_dir().or_else(dirs::home_dir))
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
    let Some(path) = extensions_config_path() else {
        return ExtensionLoadResult {
            registry: ExtensionRegistry::default(),
            warnings: vec!["user config directory is unavailable; skipping extensions load".into()],
            path: PathBuf::from(EXTENSIONS_FILE_NAME),
        };
    };
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
    if let Err(err) = fs::create_dir_all(parent) {
        return ExtensionLoadResult {
            registry: ExtensionRegistry::default(),
            warnings: vec![format!(
                "failed to create extensions config directory '{}': {err}",
                parent.display()
            )],
            path,
        };
    }
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
    title: &str,
    command: &ExtensionCommandConfig,
    context: &ExtensionSubstitutionContext,
) -> Result<PreparedExtensionCommand, String> {
    let program = substitute_template(&command.program, context);
    let args = command
        .args
        .iter()
        .map(|arg| substitute_template(arg, context))
        .collect::<Vec<_>>();
    let cwd = command
        .cwd
        .as_ref()
        .map(|cwd| PathBuf::from(substitute_template(cwd, context)));
    let env = command
        .env
        .iter()
        .map(|(key, value)| (key.clone(), substitute_template(value, context)))
        .collect::<BTreeMap<_, _>>();

    if program.trim().is_empty() {
        return Err(format!("extension '{title}' resolved to an empty program"));
    }
    validate_command_text(title, "program", &program)?;
    for arg in &args {
        validate_command_text(title, "argument", arg)?;
    }
    if let Some(cwd) = cwd.as_ref().and_then(|cwd| cwd.to_str()) {
        validate_command_text(title, "cwd", cwd)?;
    }
    for (key, value) in &env {
        validate_env_key(title, key)?;
        validate_command_text(title, "environment value", value)?;
    }
    let timeout_secs = validated_timeout_secs(title, command.timeout_secs)?;

    Ok(PreparedExtensionCommand {
        preview: shell_preview(&program, &args),
        program,
        args,
        cwd,
        env,
        timeout: Duration::from_secs(timeout_secs),
    })
}

fn validated_timeout_secs(title: &str, value: Option<u64>) -> Result<u64, String> {
    let timeout_secs = value.unwrap_or(DEFAULT_EXTENSION_TIMEOUT_SECS);
    if !(MIN_EXTENSION_TIMEOUT_SECS..=MAX_EXTENSION_TIMEOUT_SECS).contains(&timeout_secs) {
        return Err(format!(
            "extension '{title}' timeout_secs must be between {MIN_EXTENSION_TIMEOUT_SECS} and {MAX_EXTENSION_TIMEOUT_SECS}"
        ));
    }
    Ok(timeout_secs)
}

fn validate_command_text(title: &str, field: &str, value: &str) -> Result<(), String> {
    if value.contains('\0') {
        return Err(format!("extension '{title}' {field} contains a NUL byte"));
    }
    Ok(())
}

fn validate_env_key(title: &str, key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err(format!("extension '{title}' has an empty environment key"));
    }
    if key.contains('\0') || key.contains('=') {
        return Err(format!(
            "extension '{title}' environment key '{key}' must not contain '=' or NUL"
        ));
    }
    Ok(())
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
        let normalized_title = title.to_ascii_lowercase();
        if !aliases.iter().any(|alias| alias == &normalized_title) {
            aliases.push(normalized_title);
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
            description: normalized_optional_text(action.description),
            aliases,
            resource_kinds,
            shortcut: normalized_optional_text(action.shortcut),
            mode: action.mode,
            command: ExtensionCommandConfig {
                program: program.to_string(),
                args: action.command.args,
                cwd: normalized_optional_text(action.command.cwd),
                env: action.command.env,
                timeout_secs: action.command.timeout_secs,
            },
        });
    }

    ExtensionLoadResult {
        registry: ExtensionRegistry { actions },
        warnings,
        path,
    }
}

fn normalized_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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
                            timeout_secs: None,
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
                            timeout_secs: None,
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
                timeout_secs: None,
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
                timeout_secs: Some(30),
            },
        };
        let prepared = prepare_command(
            &action.title,
            &action.command,
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
        assert_eq!(prepared.timeout, Duration::from_secs(30));
        assert!(prepared.preview.contains("kubectl"));
    }

    #[test]
    fn prepare_command_rejects_invalid_env_before_launch() {
        let err = prepare_command(
            "Bad Env",
            &ExtensionCommandConfig {
                program: "kubectl".into(),
                args: vec!["get".into(), "$KIND/$NAME".into()],
                cwd: None,
                env: BTreeMap::from([("BAD=KEY".into(), "$NAME".into())]),
                timeout_secs: None,
            },
            &ExtensionSubstitutionContext::from_resource(
                &ResourceRef::Pod("api-0".into(), "prod".into()),
                Some("staging"),
                Vec::new(),
            ),
        )
        .expect_err("invalid env key should fail before command launch");

        assert!(err.contains("environment key"));
    }

    #[test]
    fn prepare_command_rejects_nul_text_before_launch() {
        let err = prepare_command(
            "Bad Arg",
            &ExtensionCommandConfig {
                program: "kubectl".into(),
                args: vec!["get\0pods".into()],
                cwd: None,
                env: BTreeMap::new(),
                timeout_secs: None,
            },
            &ExtensionSubstitutionContext::from_resource(
                &ResourceRef::Pod("api-0".into(), "prod".into()),
                Some("staging"),
                Vec::new(),
            ),
        )
        .expect_err("nul argument should fail before command launch");

        assert!(err.contains("NUL"));
    }

    #[test]
    fn validate_registry_keeps_extensions_command_only() {
        let result = validate_extensions(
            ExtensionsConfig {
                actions: vec![ExtensionActionConfig {
                    id: "kubectl_describe".into(),
                    title: "Kubectl Describe".into(),
                    description: None,
                    aliases: Vec::new(),
                    resource_kinds: vec!["Pod".into()],
                    shortcut: None,
                    mode: ExtensionExecutionMode::Background,
                    command: ExtensionCommandConfig {
                        program: "kubectl".into(),
                        args: vec!["describe".into(), "pod".into(), "$NAME".into()],
                        cwd: None,
                        env: BTreeMap::new(),
                        timeout_secs: None,
                    },
                }],
            },
            PathBuf::from("/tmp/extensions.yaml"),
        );

        assert!(result.warnings.is_empty());
        assert_eq!(result.registry.actions().len(), 1);
        assert_eq!(result.registry.actions()[0].id, "kubectl_describe");
        assert_eq!(result.registry.actions()[0].badge_label(), "BG");
    }

    #[test]
    fn validate_registry_trims_optional_text() {
        let result = validate_extensions(
            ExtensionsConfig {
                actions: vec![ExtensionActionConfig {
                    id: " describe ".into(),
                    title: " Describe ".into(),
                    description: Some("  Describe selected resource  ".into()),
                    aliases: vec![" Inspect ".into()],
                    resource_kinds: vec![" Pod ".into()],
                    shortcut: Some("  Shift+D  ".into()),
                    mode: ExtensionExecutionMode::Background,
                    command: ExtensionCommandConfig {
                        program: " kubectl ".into(),
                        args: vec!["describe".into()],
                        cwd: Some("  /tmp/kubectui  ".into()),
                        env: BTreeMap::new(),
                        timeout_secs: None,
                    },
                }],
            },
            PathBuf::from("/tmp/extensions.yaml"),
        );

        let action = &result.registry.actions()[0];
        assert_eq!(action.id, "describe");
        assert_eq!(action.title, "Describe");
        assert_eq!(
            action.description.as_deref(),
            Some("Describe selected resource")
        );
        assert_eq!(action.shortcut.as_deref(), Some("Shift+D"));
        assert_eq!(action.command.program, "kubectl");
        assert_eq!(action.command.cwd.as_deref(), Some("/tmp/kubectui"));
    }

    #[test]
    fn prepare_command_defaults_timeout() {
        let prepared = prepare_command(
            "Describe",
            &ExtensionCommandConfig {
                program: "kubectl".into(),
                args: vec!["get".into(), "pods".into()],
                cwd: None,
                env: BTreeMap::new(),
                timeout_secs: None,
            },
            &ExtensionSubstitutionContext::from_resource(
                &ResourceRef::Pod("api-0".into(), "prod".into()),
                Some("staging"),
                Vec::new(),
            ),
        )
        .expect("prepared command");

        assert_eq!(
            prepared.timeout,
            Duration::from_secs(DEFAULT_EXTENSION_TIMEOUT_SECS)
        );
    }

    #[test]
    fn prepare_command_rejects_out_of_range_timeout() {
        let err = prepare_command(
            "Slow",
            &ExtensionCommandConfig {
                program: "kubectl".into(),
                args: vec!["get".into(), "pods".into()],
                cwd: None,
                env: BTreeMap::new(),
                timeout_secs: Some(0),
            },
            &ExtensionSubstitutionContext::from_resource(
                &ResourceRef::Pod("api-0".into(), "prod".into()),
                Some("staging"),
                Vec::new(),
            ),
        )
        .expect_err("zero timeout should fail before command launch");

        assert!(err.contains("timeout_secs"));
    }

    #[test]
    fn extensions_config_rejects_legacy_ai_top_level() {
        let err = serde_yaml::from_str::<ExtensionsConfig>(
            r#"
ai:
  provider: claude_cli
"#,
        )
        .expect_err("legacy ai config should not parse as an extension config");

        assert!(err.to_string().contains("unknown field `ai`"));
    }

    #[test]
    fn extensions_config_path_requires_real_base_dir() {
        assert_eq!(
            extensions_config_path_from_base(Some(PathBuf::from("/Users/tester/.config"))),
            Some(PathBuf::from(
                "/Users/tester/.config/kubectui/extensions.yaml"
            ))
        );
        assert_eq!(extensions_config_path_from_base(None), None);
    }
}
