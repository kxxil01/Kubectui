//! Provider-backed AI analysis for selected resources.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use kubectui::{
    app::ResourceRef,
    extensions::{AiProviderConfig, AiProviderKind, AiWorkflowKind},
};

const OPENAI_CHAT_COMPLETIONS_URL: &str = "https://api.openai.com/v1/chat/completions";
const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const DEFAULT_SYSTEM_PROMPT: &str = "You are an expert Kubernetes SRE assistant embedded in KubecTUI. \
Analyze the provided resource context conservatively. Do not invent facts. Use only the supplied context. \
Return strict JSON with keys summary, likely_causes, next_steps, and uncertainty. \
summary must be a short paragraph. likely_causes, next_steps, and uncertainty must be arrays of short strings. \
If context is incomplete, say so in uncertainty instead of guessing.";
const FAILURE_SYSTEM_PROMPT: &str = "You are an expert Kubernetes failure investigator embedded in KubecTUI. \
Focus on the most likely failure chain for the selected resource. Prioritize concrete failure signals from issues, events, probes, and logs. \
Do not invent facts. Use only the supplied context. Return strict JSON with keys summary, likely_causes, next_steps, and uncertainty. \
Keep likely_causes ordered from most likely to least likely.";
const ROLLOUT_SYSTEM_PROMPT: &str = "You are an expert Kubernetes rollout reviewer embedded in KubecTUI. \
Assess rollout safety and likely blockers for the selected workload. Prioritize revision health, rollout summary, and current error signals. \
Do not invent facts. Use only the supplied context. Return strict JSON with keys summary, likely_causes, next_steps, and uncertainty. \
Call out rollback signals explicitly when the context justifies them.";
const NETWORK_SYSTEM_PROMPT: &str = "You are an expert Kubernetes network-policy and traffic investigator embedded in KubecTUI. \
Explain the current connectivity or policy verdict conservatively. Distinguish policy intent from runtime enforcement when the context says so. \
Do not invent facts. Use only the supplied context. Return strict JSON with keys summary, likely_causes, next_steps, and uncertainty.";
const TRIAGE_SYSTEM_PROMPT: &str = "You are an expert Kubernetes incident triage assistant embedded in KubecTUI. \
Prioritize the supplied findings by impact and fastest validation path. Focus on what the operator should check first. \
Do not invent facts. Use only the supplied context. Return strict JSON with keys summary, likely_causes, next_steps, and uncertainty.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiAnalysisContext {
    pub resource: ResourceRef,
    pub cluster_context: Option<String>,
    pub metadata_lines: Vec<String>,
    pub workflow_title: Option<String>,
    pub workflow_lines: Vec<String>,
    pub issue_lines: Vec<String>,
    pub event_lines: Vec<String>,
    pub probe_lines: Vec<String>,
    pub log_lines: Vec<String>,
    pub yaml_excerpt: Option<String>,
}

impl AiAnalysisContext {
    pub fn render_prompt(&self) -> String {
        let mut sections = Vec::new();
        sections.push(format!(
            "Resource\n- kind: {}\n- name: {}\n- namespace: {}",
            self.resource.kind(),
            self.resource.name(),
            self.resource.namespace().unwrap_or("-"),
        ));
        sections.push(format!(
            "Cluster Context\n- current_context: {}",
            self.cluster_context.as_deref().unwrap_or("-"),
        ));
        sections.push(render_list_section("Metadata", &self.metadata_lines));
        if !self.workflow_lines.is_empty() {
            sections.push(render_list_section(
                self.workflow_title.as_deref().unwrap_or("Workflow Context"),
                &self.workflow_lines,
            ));
        }
        sections.push(render_list_section("Issues", &self.issue_lines));
        sections.push(render_list_section("Events", &self.event_lines));
        sections.push(render_list_section("Probes", &self.probe_lines));
        sections.push(render_list_section("Logs", &self.log_lines));
        sections.push(match &self.yaml_excerpt {
            Some(yaml) => format!("YAML Excerpt\n```yaml\n{yaml}\n```"),
            None => "YAML Excerpt\n- unavailable".to_string(),
        });
        sections.join("\n\n")
    }
}

pub const fn default_system_prompt_for_workflow(workflow: AiWorkflowKind) -> &'static str {
    match workflow {
        AiWorkflowKind::ResourceAnalysis => DEFAULT_SYSTEM_PROMPT,
        AiWorkflowKind::ExplainFailure => FAILURE_SYSTEM_PROMPT,
        AiWorkflowKind::RolloutRisk => ROLLOUT_SYSTEM_PROMPT,
        AiWorkflowKind::NetworkVerdict => NETWORK_SYSTEM_PROMPT,
        AiWorkflowKind::TriageFindings => TRIAGE_SYSTEM_PROMPT,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiAnalysisResult {
    pub provider_label: String,
    pub model: String,
    pub summary: String,
    pub likely_causes: Vec<String>,
    pub next_steps: Vec<String>,
    pub uncertainty: Vec<String>,
    pub raw_json: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StructuredAiResponse {
    summary: String,
    #[serde(default)]
    likely_causes: Vec<String>,
    #[serde(default)]
    next_steps: Vec<String>,
    #[serde(default)]
    uncertainty: Vec<String>,
}

#[cold]
#[inline(never)]
pub fn run_ai_analysis(
    provider: &AiProviderConfig,
    system_prompt: &str,
    context: &AiAnalysisContext,
) -> Result<AiAnalysisResult> {
    let system_prompt = system_prompt.trim();
    if system_prompt.is_empty() {
        bail!("AI system prompt must not be empty");
    }
    let api_key = std::env::var(&provider.api_key_env)
        .with_context(|| format!("AI API key env var '{}' is not set", provider.api_key_env))?;
    let agent_config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(provider.timeout_secs)))
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = agent_config.into();

    let user_prompt = context.render_prompt();
    let raw_json = match provider.provider {
        AiProviderKind::OpenAi => {
            call_openai(&agent, provider, &api_key, system_prompt, &user_prompt)?
        }
        AiProviderKind::Anthropic => {
            call_anthropic(&agent, provider, &api_key, system_prompt, &user_prompt)?
        }
    };
    let structured = parse_structured_response(&raw_json)?;

    Ok(AiAnalysisResult {
        provider_label: provider.provider.label().to_string(),
        model: provider.model.clone(),
        summary: structured.summary,
        likely_causes: structured.likely_causes,
        next_steps: structured.next_steps,
        uncertainty: structured.uncertainty,
        raw_json,
    })
}

#[cold]
#[inline(never)]
fn render_list_section(title: &str, items: &[String]) -> String {
    if items.is_empty() {
        format!("{title}\n- unavailable")
    } else {
        format!(
            "{title}\n{}",
            items
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

#[cold]
#[inline(never)]
fn call_openai(
    agent: &ureq::Agent,
    provider: &AiProviderConfig,
    api_key: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String> {
    let endpoint = provider
        .endpoint
        .as_deref()
        .unwrap_or(OPENAI_CHAT_COMPLETIONS_URL);
    let body = json!({
        "model": provider.model,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_prompt }
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "kubectui_ai_analysis",
                "strict": true,
                "schema": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["summary", "likely_causes", "next_steps", "uncertainty"],
                    "properties": {
                        "summary": { "type": "string" },
                        "likely_causes": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "next_steps": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "uncertainty": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    }
                }
            }
        },
        "temperature": provider.temperature.unwrap_or(0.1_f32),
        "max_completion_tokens": provider.max_output_tokens,
    });
    let auth_header = format!("Bearer {api_key}");
    let mut response = agent
        .post(endpoint)
        .header("Authorization", &auth_header)
        .send_json(&body)
        .with_context(|| format!("failed to call OpenAI endpoint '{endpoint}'"))?;
    let status = response.status();
    let value = response
        .body_mut()
        .read_json::<Value>()
        .context("failed to decode OpenAI response")?;
    if !status.is_success() {
        bail!(extract_provider_error("OpenAI", &value));
    }
    value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("OpenAI response did not include message.content"))
}

#[cold]
#[inline(never)]
fn call_anthropic(
    agent: &ureq::Agent,
    provider: &AiProviderConfig,
    api_key: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String> {
    let endpoint = provider
        .endpoint
        .as_deref()
        .unwrap_or(ANTHROPIC_MESSAGES_URL);
    let body = json!({
        "model": provider.model,
        "system": system_prompt,
        "messages": [
            { "role": "user", "content": user_prompt }
        ],
        "max_tokens": provider.max_output_tokens,
        "temperature": provider.temperature.unwrap_or(0.1_f32),
    });
    let mut response = agent
        .post(endpoint)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .send_json(&body)
        .with_context(|| format!("failed to call Anthropic endpoint '{endpoint}'"))?;
    let status = response.status();
    let value = response
        .body_mut()
        .read_json::<Value>()
        .context("failed to decode Anthropic response")?;
    if !status.is_success() {
        bail!(extract_provider_error("Anthropic", &value));
    }

    value
        .get("content")
        .and_then(Value::as_array)
        .and_then(|blocks| {
            blocks.iter().find_map(|block| {
                (block.get("type").and_then(Value::as_str) == Some("text"))
                    .then(|| block.get("text").and_then(Value::as_str))
                    .flatten()
            })
        })
        .map(str::to_string)
        .ok_or_else(|| anyhow!("Anthropic response did not include a text content block"))
}

#[cold]
#[inline(never)]
fn parse_structured_response(raw: &str) -> Result<StructuredAiResponse> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("AI provider returned an empty response");
    }
    serde_json::from_str::<StructuredAiResponse>(trimmed)
        .or_else(|_| {
            extract_json_object(trimmed)
                .and_then(|json| serde_json::from_str(json).map_err(Into::into))
        })
        .map_err(|err| anyhow!("AI response was not valid structured JSON: {err}"))
        .and_then(validate_structured_response)
}

#[cold]
#[inline(never)]
fn extract_json_object(raw: &str) -> Result<&str> {
    let start = raw
        .find('{')
        .ok_or_else(|| anyhow!("AI response did not include a JSON object"))?;
    let end = raw
        .rfind('}')
        .ok_or_else(|| anyhow!("AI response did not include a JSON object"))?;
    if end <= start {
        bail!("AI response did not include a valid JSON object");
    }
    Ok(&raw[start..=end])
}

#[cold]
#[inline(never)]
fn validate_structured_response(response: StructuredAiResponse) -> Result<StructuredAiResponse> {
    if response.summary.trim().is_empty() {
        bail!("AI response omitted summary");
    }
    Ok(StructuredAiResponse {
        summary: response.summary.trim().to_string(),
        likely_causes: trim_lines(response.likely_causes),
        next_steps: trim_lines(response.next_steps),
        uncertainty: trim_lines(response.uncertainty),
    })
}

#[cold]
#[inline(never)]
fn trim_lines(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .take(8)
        .collect()
}

#[cold]
#[inline(never)]
fn extract_provider_error(provider: &str, value: &Value) -> String {
    value
        .get("error")
        .and_then(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
        })
        .map(|message| format!("{provider} request failed: {message}"))
        .unwrap_or_else(|| format!("{provider} request failed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_structured_response_accepts_wrapped_json() {
        let parsed = parse_structured_response(
            "```json\n{\"summary\":\"ok\",\"likely_causes\":[\"a\"],\"next_steps\":[\"b\"],\"uncertainty\":[\"c\"]}\n```",
        )
        .expect("wrapped json parses");

        assert_eq!(parsed.summary, "ok");
        assert_eq!(parsed.likely_causes, vec!["a"]);
    }

    #[test]
    fn context_prompt_renders_all_sections() {
        let prompt = AiAnalysisContext {
            resource: ResourceRef::Pod("api-0".into(), "prod".into()),
            cluster_context: Some("staging".into()),
            metadata_lines: vec!["status: CrashLoopBackOff".into()],
            workflow_title: Some("Failure Focus".into()),
            workflow_lines: vec!["Prioritize probe failures".into()],
            issue_lines: vec!["runtime: CrashLoopBackOff".into()],
            event_lines: vec!["Warning BackOff: restarting".into()],
            probe_lines: vec!["api: readiness failure".into()],
            log_lines: vec!["panic: boom".into()],
            yaml_excerpt: Some("kind: Pod".into()),
        }
        .render_prompt();

        assert!(prompt.contains("Resource"));
        assert!(prompt.contains("Failure Focus"));
        assert!(prompt.contains("Logs"));
        assert!(prompt.contains("kind: Pod"));
    }

    #[test]
    fn workflow_prompts_are_available_for_specialized_actions() {
        assert!(
            default_system_prompt_for_workflow(AiWorkflowKind::ExplainFailure).contains("failure")
        );
        assert!(
            default_system_prompt_for_workflow(AiWorkflowKind::RolloutRisk).contains("rollout")
        );
    }

    #[test]
    fn empty_system_prompt_is_rejected() {
        let provider = AiProviderConfig {
            provider: AiProviderKind::OpenAi,
            model: "gpt-test".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            endpoint: None,
            timeout_secs: 5,
            max_output_tokens: 128,
            temperature: Some(0.1),
            action: None,
        };
        let context = AiAnalysisContext {
            resource: ResourceRef::Pod("api-0".into(), "prod".into()),
            cluster_context: None,
            metadata_lines: Vec::new(),
            workflow_title: None,
            workflow_lines: Vec::new(),
            issue_lines: Vec::new(),
            event_lines: Vec::new(),
            probe_lines: Vec::new(),
            log_lines: Vec::new(),
            yaml_excerpt: None,
        };

        let err = run_ai_analysis(&provider, "   ", &context).expect_err("empty prompt");
        assert!(err.to_string().contains("must not be empty"));
    }
}
