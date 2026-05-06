//! Native AI action configuration and palette registry.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::app::ResourceRef;

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
pub enum AiProviderKind {
    OpenAi,
    Anthropic,
    ClaudeCli,
    CodexCli,
}

impl AiProviderKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::ClaudeCli => "Claude CLI",
            Self::CodexCli => "Codex CLI",
        }
    }

    const fn default_model(self) -> &'static str {
        match self {
            Self::OpenAi => "",
            Self::Anthropic => "",
            Self::ClaudeCli => "claude-cli",
            Self::CodexCli => "codex-cli",
        }
    }

    const fn slug(self) -> &'static str {
        match self {
            Self::OpenAi => "open_ai",
            Self::Anthropic => "anthropic",
            Self::ClaudeCli => "claude_cli",
            Self::CodexCli => "codex_cli",
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
    #[serde(default)]
    pub model: String,
    #[serde(default)]
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
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub action: Option<AiActionConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AiConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<AiProviderConfig>,
}

impl AiConfig {
    pub fn single(provider: AiProviderConfig) -> Self {
        Self {
            providers: vec![provider],
        }
    }
}

impl<'de> Deserialize<'de> for AiConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum AiConfigRepr {
            Multi { providers: Vec<AiProviderConfig> },
            Single(Box<AiProviderConfig>),
        }

        match AiConfigRepr::deserialize(deserializer)? {
            AiConfigRepr::Multi { providers } => Ok(Self { providers }),
            AiConfigRepr::Single(provider) => Ok(Self::single(*provider)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoadedAiAction {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub aliases: Vec<String>,
    pub resource_kinds: Vec<String>,
    pub shortcut: Option<String>,
    pub provider: AiProviderConfig,
    pub workflow: AiWorkflowKind,
    pub system_prompt: Option<String>,
}

impl LoadedAiAction {
    pub fn matches_resource(&self, resource: &ResourceRef) -> bool {
        !matches!(resource, ResourceRef::Secret(_, _))
            && (self.resource_kinds.is_empty()
                || self
                    .resource_kinds
                    .iter()
                    .any(|kind| kind == "*" || kind.eq_ignore_ascii_case(resource.kind())))
    }

    pub fn badge_label(&self) -> String {
        provider_display_label(&self.provider)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AiActionRegistry {
    actions: Vec<LoadedAiAction>,
}

impl AiActionRegistry {
    pub fn actions(&self) -> &[LoadedAiAction] {
        &self.actions
    }

    pub fn get(&self, id: &str) -> Option<&LoadedAiAction> {
        self.actions.iter().find(|action| action.id == id)
    }

    pub fn palette_actions_for(&self, resource: &ResourceRef) -> Vec<LoadedAiAction> {
        self.actions
            .iter()
            .filter(|action| action.matches_resource(resource))
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AiActionLoadResult {
    pub registry: AiActionRegistry,
    pub warnings: Vec<String>,
}

pub fn validate_ai_actions(config: Option<AiConfig>) -> AiActionLoadResult {
    let Some(config) = config else {
        return AiActionLoadResult::default();
    };
    let mut actions = Vec::new();
    let mut warnings = Vec::new();
    let mut providers = Vec::new();
    for ai in config.providers {
        if let Some(warning) = ai_provider_warning(&ai) {
            warnings.push(format!(
                "{} provider skipped: {warning}",
                ai.provider.label()
            ));
            continue;
        }
        providers.push(ai);
    }

    let provider_count = providers.len();
    let mut seen_action_ids = BTreeSet::new();
    for (provider_idx, ai) in providers.into_iter().enumerate() {
        let namespaced = provider_count > 1;
        match build_custom_ai_action(ai.clone(), provider_idx, namespaced) {
            Ok(action) => {
                push_unique_ai_action(action, &mut actions, &mut seen_action_ids, &mut warnings)
            }
            Err(warning) => warnings.push(warning),
        }
        for workflow in [
            AiWorkflowKind::ExplainFailure,
            AiWorkflowKind::RolloutRisk,
            AiWorkflowKind::NetworkVerdict,
            AiWorkflowKind::TriageFindings,
        ] {
            push_unique_ai_action(
                build_default_ai_workflow_action(ai.clone(), workflow, provider_idx, namespaced),
                &mut actions,
                &mut seen_action_ids,
                &mut warnings,
            );
        }
    }

    AiActionLoadResult {
        registry: AiActionRegistry { actions },
        warnings,
    }
}

fn push_unique_ai_action(
    action: LoadedAiAction,
    actions: &mut Vec<LoadedAiAction>,
    seen_action_ids: &mut BTreeSet<String>,
    warnings: &mut Vec<String>,
) {
    if seen_action_ids.insert(action.id.clone()) {
        actions.push(action);
    } else {
        warnings.push(format!("AI action '{}' skipped: duplicate id", action.id));
    }
}

fn build_custom_ai_action(
    ai: AiProviderConfig,
    provider_idx: usize,
    namespaced: bool,
) -> Result<LoadedAiAction, String> {
    let action = ai.action.clone().unwrap_or(AiActionConfig {
        id: default_ai_action_id(),
        title: default_ai_action_title(),
        description: Some("Ask configured AI provider to analyze this resource".into()),
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
    if title.is_empty() {
        return Err(format!("skipping AI action '{id}' with empty title"));
    }
    let normalized_ai = normalize_ai_provider(ai);
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
    if namespaced {
        add_provider_aliases(&mut aliases, &normalized_ai);
    }
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

    Ok(LoadedAiAction {
        id: action_id(id, &normalized_ai, provider_idx, namespaced),
        title: action_title(title, &normalized_ai, namespaced),
        description: action.description.filter(|value| !value.trim().is_empty()),
        aliases,
        resource_kinds,
        shortcut: action.shortcut.filter(|value| !value.trim().is_empty()),
        provider: normalized_ai,
        workflow: AiWorkflowKind::ResourceAnalysis,
        system_prompt: action
            .system_prompt
            .filter(|value| !value.trim().is_empty()),
    })
}

fn build_default_ai_workflow_action(
    ai: AiProviderConfig,
    workflow: AiWorkflowKind,
    provider_idx: usize,
    namespaced: bool,
) -> LoadedAiAction {
    let normalized_ai = normalize_ai_provider(ai);
    let mut aliases = workflow.default_aliases();
    if namespaced {
        add_provider_aliases(&mut aliases, &normalized_ai);
    }
    LoadedAiAction {
        id: action_id(
            workflow.default_id(),
            &normalized_ai,
            provider_idx,
            namespaced,
        ),
        title: action_title(workflow.default_title(), &normalized_ai, namespaced),
        description: Some(format!(
            "{} with {}",
            workflow.default_title(),
            provider_display_label(&normalized_ai)
        )),
        aliases,
        resource_kinds: workflow.default_resource_kinds(),
        shortcut: None,
        provider: normalized_ai,
        workflow,
        system_prompt: None,
    }
}

fn action_id(
    base: &str,
    provider: &AiProviderConfig,
    provider_idx: usize,
    namespaced: bool,
) -> String {
    if !namespaced {
        return base.to_string();
    }
    if provider_idx == 0 {
        base.to_string()
    } else {
        format!("{}_{}", base, provider_suffix(provider, provider_idx))
    }
}

fn action_title(base: &str, provider: &AiProviderConfig, namespaced: bool) -> String {
    if namespaced {
        format!("{base} ({})", provider_display_label(provider))
    } else {
        base.to_string()
    }
}

fn provider_suffix(provider: &AiProviderConfig, provider_idx: usize) -> String {
    format!("{}_{provider_idx}", provider.provider.slug())
}

fn add_provider_aliases(aliases: &mut Vec<String>, provider: &AiProviderConfig) {
    let label = provider.provider.label().to_ascii_lowercase();
    let display_label = provider_display_label(provider).to_ascii_lowercase();
    let slug = provider.provider.slug().replace('_', " ");
    let default_title = DEFAULT_AI_ACTION_TITLE.to_ascii_lowercase();
    aliases.extend([
        label.clone(),
        display_label.clone(),
        slug.clone(),
        format!("{default_title} {label}"),
        format!("{default_title} {display_label}"),
        format!("{default_title} {slug}"),
    ]);
    aliases.sort();
    aliases.dedup();
}

pub fn ai_analysis_provider_label(provider: &AiProviderConfig) -> String {
    match provider.provider {
        AiProviderKind::OpenAi | AiProviderKind::Anthropic => provider.provider.label().to_string(),
        AiProviderKind::ClaudeCli | AiProviderKind::CodexCli => provider
            .command
            .as_deref()
            .map(str::trim)
            .filter(|command| !command.is_empty())
            .map(|command| format!("{} ({command})", provider.provider.label()))
            .unwrap_or_else(|| provider.provider.label().to_string()),
    }
}

fn provider_display_label(provider: &AiProviderConfig) -> String {
    match provider.provider {
        AiProviderKind::OpenAi | AiProviderKind::Anthropic => {
            let model = provider.model.trim();
            if model.is_empty() {
                provider.provider.label().to_string()
            } else {
                format!("{} {model}", provider.provider.label())
            }
        }
        AiProviderKind::ClaudeCli | AiProviderKind::CodexCli => provider
            .command
            .as_deref()
            .map(str::trim)
            .filter(|command| !command.is_empty())
            .map(|command| format!("{} ({command})", provider.provider.label()))
            .unwrap_or_else(|| provider.provider.label().to_string()),
    }
}

fn ai_provider_warning(ai: &AiProviderConfig) -> Option<String> {
    match ai.provider {
        AiProviderKind::OpenAi | AiProviderKind::Anthropic => {
            if ai.model.trim().is_empty() {
                return Some("skipping AI actions with empty model".to_string());
            }
            if ai.api_key_env.trim().is_empty() {
                return Some("skipping AI actions with empty api_key_env".to_string());
            }
        }
        AiProviderKind::ClaudeCli | AiProviderKind::CodexCli => {}
    }
    None
}

fn normalize_ai_provider(mut ai: AiProviderConfig) -> AiProviderConfig {
    ai.model = ai.model.trim().to_string();
    if ai.model.is_empty() {
        ai.model = ai.provider.default_model().to_string();
    }
    ai.api_key_env = ai.api_key_env.trim().to_string();
    ai.endpoint = trim_optional(ai.endpoint);
    ai.command = trim_optional(ai.command);
    ai.args = ai
        .args
        .into_iter()
        .map(|arg| arg.trim().to_string())
        .filter(|arg| !arg.is_empty())
        .collect();
    ai.timeout_secs = ai.timeout_secs.max(1);
    ai.max_output_tokens = ai.max_output_tokens.max(64);
    ai.action = None;
    ai
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

fn trim_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli_provider(provider: AiProviderKind) -> AiProviderConfig {
        AiProviderConfig {
            provider,
            model: String::new(),
            api_key_env: String::new(),
            endpoint: None,
            timeout_secs: 15,
            max_output_tokens: 512,
            temperature: Some(0.1),
            command: None,
            args: Vec::new(),
            action: None,
        }
    }

    fn openai_provider(model: &str) -> AiProviderConfig {
        AiProviderConfig {
            provider: AiProviderKind::OpenAi,
            model: model.into(),
            api_key_env: "OPENAI_API_KEY".into(),
            endpoint: None,
            timeout_secs: 15,
            max_output_tokens: 512,
            temperature: Some(0.1),
            command: None,
            args: Vec::new(),
            action: None,
        }
    }

    #[test]
    fn validate_native_ai_actions_registers_default_workflows() {
        let result = validate_ai_actions(Some(AiConfig::single(cli_provider(
            AiProviderKind::ClaudeCli,
        ))));

        assert!(result.warnings.is_empty());
        assert_eq!(result.registry.actions().len(), 5);
        assert_eq!(result.registry.actions()[0].id, "ask_ai");
        assert_eq!(result.registry.actions()[0].badge_label(), "Claude CLI");
        assert!(result.registry.get("ai_explain_failure").is_some());
        assert!(result.registry.get("ai_rollout_risk").is_some());
        assert!(result.registry.get("ai_network_verdict").is_some());
        assert!(result.registry.get("ai_triage_findings").is_some());
    }

    #[test]
    fn http_ai_requires_model_and_api_key_env() {
        let result = validate_ai_actions(Some(AiConfig::single(AiProviderConfig {
            provider: AiProviderKind::OpenAi,
            model: String::new(),
            api_key_env: "OPENAI_API_KEY".into(),
            endpoint: None,
            timeout_secs: 15,
            max_output_tokens: 512,
            temperature: Some(0.1),
            command: None,
            args: Vec::new(),
            action: None,
        })));

        assert!(result.registry.actions().is_empty());
        assert_eq!(
            result.warnings,
            vec!["OpenAI provider skipped: skipping AI actions with empty model"]
        );
    }

    #[test]
    fn multiple_ai_providers_register_picker_actions() {
        let result = validate_ai_actions(Some(AiConfig {
            providers: vec![
                cli_provider(AiProviderKind::CodexCli),
                cli_provider(AiProviderKind::ClaudeCli),
            ],
        }));

        assert!(result.warnings.is_empty());
        assert_eq!(result.registry.actions().len(), 10);
        assert_eq!(result.registry.actions()[0].id, "ask_ai");
        assert_eq!(result.registry.actions()[0].title, "Ask AI (Codex CLI)");
        assert!(
            result
                .registry
                .get("ai_explain_failure_claude_cli_1")
                .is_some()
        );
    }

    #[test]
    fn repeated_ai_providers_have_distinct_picker_labels() {
        let mut codex_alt = cli_provider(AiProviderKind::CodexCli);
        codex_alt.command = Some("codex-dev".into());
        let result = validate_ai_actions(Some(AiConfig {
            providers: vec![
                openai_provider("gpt-4.1"),
                openai_provider("gpt-4.1-mini"),
                cli_provider(AiProviderKind::CodexCli),
                codex_alt,
            ],
        }));

        assert!(result.warnings.is_empty());
        let titles = result
            .registry
            .actions()
            .iter()
            .take(4)
            .map(|action| action.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            titles,
            vec![
                "Ask AI (OpenAI gpt-4.1)",
                "Explain Failure (OpenAI gpt-4.1)",
                "Summarize Rollout Risk (OpenAI gpt-4.1)",
                "Explain Network Verdict (OpenAI gpt-4.1)",
            ]
        );
        assert!(
            result
                .registry
                .actions()
                .iter()
                .any(|action| action.title == "Ask AI (OpenAI gpt-4.1-mini)")
        );
        assert!(
            result
                .registry
                .actions()
                .iter()
                .any(|action| action.title == "Ask AI (Codex CLI (codex-dev))")
        );
    }

    #[test]
    fn ai_analysis_header_labels_keep_model_and_cli_command_distinct() {
        let openai = openai_provider("gpt-4.1");
        assert_eq!(ai_analysis_provider_label(&openai), "OpenAI");

        let mut codex_alt = cli_provider(AiProviderKind::CodexCli);
        codex_alt.command = Some(" codex-dev ".into());
        assert_eq!(
            ai_analysis_provider_label(&codex_alt),
            "Codex CLI (codex-dev)"
        );
    }

    #[test]
    fn skipped_ai_providers_do_not_force_picker_ids_for_single_valid_provider() {
        let result = validate_ai_actions(Some(AiConfig {
            providers: vec![
                AiProviderConfig {
                    provider: AiProviderKind::OpenAi,
                    model: String::new(),
                    api_key_env: "OPENAI_API_KEY".into(),
                    endpoint: None,
                    timeout_secs: 15,
                    max_output_tokens: 512,
                    temperature: Some(0.1),
                    command: None,
                    args: Vec::new(),
                    action: None,
                },
                cli_provider(AiProviderKind::ClaudeCli),
            ],
        }));

        assert_eq!(
            result.warnings,
            vec!["OpenAI provider skipped: skipping AI actions with empty model"]
        );
        assert_eq!(result.registry.actions().len(), 5);
        assert_eq!(result.registry.actions()[0].id, "ask_ai");
        assert_eq!(result.registry.actions()[0].title, "Ask AI");
        assert!(result.registry.get("ai_explain_failure").is_some());
        assert!(
            result
                .registry
                .get("ai_explain_failure_claude_cli_1")
                .is_none()
        );
    }

    #[test]
    fn duplicate_ai_action_ids_are_skipped_with_warning() {
        let mut provider = cli_provider(AiProviderKind::CodexCli);
        provider.action = Some(AiActionConfig {
            id: "ai_explain_failure".into(),
            title: "Custom Failure Explainer".into(),
            description: None,
            aliases: vec!["custom failure".into()],
            resource_kinds: vec!["Pod".into()],
            shortcut: None,
            system_prompt: None,
        });

        let result = validate_ai_actions(Some(AiConfig::single(provider)));

        assert_eq!(result.registry.actions().len(), 4);
        assert_eq!(
            result.warnings,
            vec!["AI action 'ai_explain_failure' skipped: duplicate id"]
        );
        let action = result
            .registry
            .get("ai_explain_failure")
            .expect("custom action keeps requested id");
        assert_eq!(action.title, "Custom Failure Explainer");
        assert_eq!(action.workflow, AiWorkflowKind::ResourceAnalysis);
    }

    #[test]
    fn wildcard_ai_actions_do_not_match_secret_resources() {
        let mut provider = cli_provider(AiProviderKind::CodexCli);
        provider.action = Some(AiActionConfig {
            id: "ask_anything".into(),
            title: "Ask Anything".into(),
            description: None,
            aliases: vec!["ask anything".into()],
            resource_kinds: vec!["*".into()],
            shortcut: None,
            system_prompt: None,
        });

        let result = validate_ai_actions(Some(AiConfig::single(provider)));
        let action = result.registry.get("ask_anything").expect("custom action");

        assert!(action.matches_resource(&ResourceRef::Pod("api-0".into(), "prod".into())));
        assert!(!action.matches_resource(&ResourceRef::Secret(
            "database-password".into(),
            "prod".into()
        )));
        assert!(
            result
                .registry
                .palette_actions_for(&ResourceRef::Secret(
                    "database-password".into(),
                    "prod".into()
                ))
                .is_empty()
        );
    }
}
