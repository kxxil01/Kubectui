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
const DEFAULT_AI_TIMEOUT_SECS: u64 = 30;
const DEFAULT_AI_MAX_OUTPUT_TOKENS: u32 = 800;
const DEFAULT_AI_ACTION_ID: &str = "ask_ai";
const DEFAULT_AI_ACTION_TITLE: &str = "Ask AI";
const DEFAULT_AI_RESOURCE_KINDS: &[&str] = &[
    "Pod",
    "Node",
    "Deployment",
    "StatefulSet",
    "DaemonSet",
    "Job",
    "CronJob",
    "Service",
    "Ingress",
    "NetworkPolicy",
    "HelmRelease",
];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderKind {
    OpenAi,
    Anthropic,
}

impl AiProviderKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "AI",
            Self::Anthropic => "Claude",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiWorkflowKind {
    ResourceAnalysis,
    ExplainFailure,
    RolloutRisk,
    NetworkVerdict,
    TriageFindings,
}

impl AiWorkflowKind {
    pub const fn default_id(self) -> &'static str {
        match self {
            Self::ResourceAnalysis => DEFAULT_AI_ACTION_ID,
            Self::ExplainFailure => "ai_explain_failure",
            Self::RolloutRisk => "ai_rollout_risk",
            Self::NetworkVerdict => "ai_network_verdict",
            Self::TriageFindings => "ai_triage_findings",
        }
    }

    pub const fn default_title(self) -> &'static str {
        match self {
            Self::ResourceAnalysis => DEFAULT_AI_ACTION_TITLE,
            Self::ExplainFailure => "Explain Failure",
            Self::RolloutRisk => "Summarize Rollout Risk",
            Self::NetworkVerdict => "Explain Network Verdict",
            Self::TriageFindings => "Triage Findings",
        }
    }

    pub fn default_aliases(self) -> Vec<String> {
        match self {
            Self::ResourceAnalysis => vec!["ask ai".into(), "ai".into(), "diagnose".into()],
            Self::ExplainFailure => vec![
                "explain failure".into(),
                "why failing".into(),
                "failure diagnosis".into(),
            ],
            Self::RolloutRisk => vec![
                "rollout risk".into(),
                "release risk".into(),
                "deployment risk".into(),
            ],
            Self::NetworkVerdict => vec![
                "network verdict".into(),
                "explain connectivity".into(),
                "policy verdict".into(),
            ],
            Self::TriageFindings => vec![
                "triage findings".into(),
                "triage issues".into(),
                "prioritize issues".into(),
            ],
        }
    }

    pub fn default_resource_kinds(self) -> Vec<String> {
        match self {
            Self::ResourceAnalysis | Self::TriageFindings => DEFAULT_AI_RESOURCE_KINDS
                .iter()
                .map(|kind| (*kind).to_string())
                .collect(),
            Self::ExplainFailure => vec!["Pod".into(), "Job".into(), "CronJob".into()],
            Self::RolloutRisk => vec![
                "Deployment".into(),
                "StatefulSet".into(),
                "DaemonSet".into(),
                "HelmRelease".into(),
            ],
            Self::NetworkVerdict => vec![
                "Pod".into(),
                "Service".into(),
                "Ingress".into(),
                "NetworkPolicy".into(),
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiActionConfig {
    #[serde(default = "default_ai_action_id")]
    pub id: String,
    #[serde(default = "default_ai_action_title")]
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub resource_kinds: Vec<String>,
    #[serde(default)]
    pub shortcut: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiProviderConfig {
    pub provider: AiProviderKind,
    pub model: String,
    pub api_key_env: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default = "default_ai_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_ai_max_output_tokens")]
    pub max_output_tokens: u32,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub action: Option<AiActionConfig>,
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtensionsConfig {
    #[serde(default)]
    pub actions: Vec<ExtensionActionConfig>,
    #[serde(default)]
    pub ai: Option<AiProviderConfig>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoadedExtensionActionKind {
    Command {
        mode: ExtensionExecutionMode,
        command: ExtensionCommandConfig,
    },
    AiAnalysis {
        provider: AiProviderConfig,
        workflow: AiWorkflowKind,
        system_prompt: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoadedExtensionAction {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub aliases: Vec<String>,
    pub resource_kinds: Vec<String>,
    pub shortcut: Option<String>,
    pub kind: LoadedExtensionActionKind,
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
        match &self.kind {
            LoadedExtensionActionKind::Command { mode, .. } => mode.label().to_string(),
            LoadedExtensionActionKind::AiAnalysis { provider, .. } => {
                provider.provider.label().to_string()
            }
        }
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
    pub preview: String,
}

fn default_ai_timeout_secs() -> u64 {
    DEFAULT_AI_TIMEOUT_SECS
}

fn default_ai_max_output_tokens() -> u32 {
    DEFAULT_AI_MAX_OUTPUT_TOKENS
}

fn default_ai_action_id() -> String {
    DEFAULT_AI_ACTION_ID.to_string()
}

fn default_ai_action_title() -> String {
    DEFAULT_AI_ACTION_TITLE.to_string()
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
            kind: LoadedExtensionActionKind::Command {
                mode: action.mode,
                command: ExtensionCommandConfig {
                    program: program.to_string(),
                    args: action.command.args,
                    cwd: action.command.cwd.filter(|value| !value.trim().is_empty()),
                    env: action.command.env,
                },
            },
        });
    }

    if let Some(ai) = config.ai {
        let ai_for_workflows = ai.clone();
        match validate_ai_extension(ai, &mut seen_ids) {
            Ok(action) => {
                actions.push(action);
                for workflow in [
                    AiWorkflowKind::ExplainFailure,
                    AiWorkflowKind::RolloutRisk,
                    AiWorkflowKind::NetworkVerdict,
                    AiWorkflowKind::TriageFindings,
                ] {
                    match build_default_ai_workflow_action(
                        ai_for_workflows.clone(),
                        workflow,
                        &mut seen_ids,
                    ) {
                        Ok(action) => actions.push(action),
                        Err(warning) => warnings.push(warning),
                    }
                }
            }
            Err(warning) => warnings.push(warning),
        }
    }

    ExtensionLoadResult {
        registry: ExtensionRegistry { actions },
        warnings,
        path,
    }
}

fn build_default_ai_workflow_action(
    ai: AiProviderConfig,
    workflow: AiWorkflowKind,
    seen_ids: &mut BTreeSet<String>,
) -> Result<LoadedExtensionAction, String> {
    let id = workflow.default_id();
    if !seen_ids.insert(id.to_string()) {
        return Err(format!("skipping duplicate extension id '{id}'"));
    }

    let model = ai.model.trim();
    let api_key_env = ai.api_key_env.trim();
    if model.is_empty() {
        return Err(format!("skipping AI workflow '{id}' with empty model"));
    }
    if api_key_env.is_empty() {
        return Err(format!(
            "skipping AI workflow '{id}' with empty api_key_env"
        ));
    }

    Ok(LoadedExtensionAction {
        id: id.to_string(),
        title: workflow.default_title().to_string(),
        description: Some(format!(
            "{} with the configured AI provider",
            workflow.default_title()
        )),
        aliases: workflow.default_aliases(),
        resource_kinds: workflow.default_resource_kinds(),
        shortcut: None,
        kind: LoadedExtensionActionKind::AiAnalysis {
            provider: AiProviderConfig {
                provider: ai.provider,
                model: model.to_string(),
                api_key_env: api_key_env.to_string(),
                endpoint: ai.endpoint.filter(|value| !value.trim().is_empty()),
                timeout_secs: ai.timeout_secs.max(1),
                max_output_tokens: ai.max_output_tokens.max(64),
                temperature: ai.temperature,
                action: None,
            },
            workflow,
            system_prompt: None,
        },
    })
}

fn validate_ai_extension(
    ai: AiProviderConfig,
    seen_ids: &mut BTreeSet<String>,
) -> Result<LoadedExtensionAction, String> {
    let model = ai.model.trim();
    let api_key_env = ai.api_key_env.trim();
    if model.is_empty() {
        return Err("skipping AI action with empty model".to_string());
    }
    if api_key_env.is_empty() {
        return Err("skipping AI action with empty api_key_env".to_string());
    }
    let action = ai.action.unwrap_or(AiActionConfig {
        id: default_ai_action_id(),
        title: default_ai_action_title(),
        description: Some("Ask the configured AI provider to analyze this resource".into()),
        aliases: vec!["ask ai".into(), "ai".into(), "diagnose".into()],
        resource_kinds: DEFAULT_AI_RESOURCE_KINDS
            .iter()
            .map(|kind| (*kind).to_string())
            .collect(),
        shortcut: None,
        system_prompt: None,
    });
    let id = action.id.trim();
    let title = action.title.trim();
    if id.is_empty() {
        return Err("skipping AI action with empty id".to_string());
    }
    if !seen_ids.insert(id.to_string()) {
        return Err(format!("skipping duplicate extension id '{id}'"));
    }
    if title.is_empty() {
        return Err(format!("skipping AI action '{id}' with empty title"));
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
    if resource_kinds.is_empty() {
        resource_kinds.extend(
            DEFAULT_AI_RESOURCE_KINDS
                .iter()
                .map(|kind| (*kind).to_string()),
        );
    }
    resource_kinds.sort();
    resource_kinds.dedup();

    Ok(LoadedExtensionAction {
        id: id.to_string(),
        title: title.to_string(),
        description: action.description.filter(|value| !value.trim().is_empty()),
        aliases,
        resource_kinds,
        shortcut: action.shortcut.filter(|value| !value.trim().is_empty()),
        kind: LoadedExtensionActionKind::AiAnalysis {
            provider: AiProviderConfig {
                provider: ai.provider,
                model: model.to_string(),
                api_key_env: api_key_env.to_string(),
                endpoint: ai.endpoint.filter(|value| !value.trim().is_empty()),
                timeout_secs: ai.timeout_secs.max(1),
                max_output_tokens: ai.max_output_tokens.max(64),
                temperature: ai.temperature,
                action: None,
            },
            workflow: AiWorkflowKind::ResourceAnalysis,
            system_prompt: action
                .system_prompt
                .filter(|value| !value.trim().is_empty()),
        },
    })
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
                ai: None,
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
            kind: LoadedExtensionActionKind::Command {
                mode: ExtensionExecutionMode::Foreground,
                command: ExtensionCommandConfig {
                    program: "open".into(),
                    args: Vec::new(),
                    cwd: None,
                    env: BTreeMap::new(),
                },
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
            kind: LoadedExtensionActionKind::Command {
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
            },
        };
        let command = match &action.kind {
            LoadedExtensionActionKind::Command { command, .. } => command,
            LoadedExtensionActionKind::AiAnalysis { .. } => panic!("expected command action"),
        };
        let prepared = prepare_command(
            &action.title,
            command,
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

    #[test]
    fn validate_registry_adds_ai_action_when_configured() {
        let result = validate_extensions(
            ExtensionsConfig {
                actions: Vec::new(),
                ai: Some(AiProviderConfig {
                    provider: AiProviderKind::Anthropic,
                    model: "claude-sonnet".into(),
                    api_key_env: "ANTHROPIC_API_KEY".into(),
                    endpoint: None,
                    timeout_secs: 15,
                    max_output_tokens: 512,
                    temperature: Some(0.1),
                    action: None,
                }),
            },
            PathBuf::from("/tmp/extensions.yaml"),
        );

        assert_eq!(result.registry.actions().len(), 5);
        assert_eq!(result.registry.actions()[0].id, "ask_ai");
        assert_eq!(result.registry.actions()[0].badge_label(), "Claude");
        assert!(result.registry.get("ai_explain_failure").is_some());
        assert!(result.registry.get("ai_rollout_risk").is_some());
        assert!(result.registry.get("ai_network_verdict").is_some());
        assert!(result.registry.get("ai_triage_findings").is_some());
        assert!(
            !result.registry.actions()[0]
                .matches_resource(&ResourceRef::Secret("app-secret".into(), "prod".into(),))
        );
        assert!(
            result.registry.actions()[0]
                .matches_resource(&ResourceRef::Pod("api-0".into(), "prod".into(),))
        );
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
