//! Provider-backed AI analysis for selected resources.

use std::{
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use kubectui::{
    ai_actions::{AiProviderConfig, AiProviderKind, AiWorkflowKind, ai_analysis_provider_label},
    app::ResourceRef,
};

const OPENAI_CHAT_COMPLETIONS_URL: &str = "https://api.openai.com/v1/chat/completions";
const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const PROVIDER_ERROR_MAX_LINES: usize = 12;
const PROVIDER_ERROR_MAX_CHARS: usize = 600;
const PROVIDER_ERROR_SCAN_MAX_CHARS: usize = 8_000;
const AI_OUTPUT_MAX_ITEMS: usize = 8;
const AI_OUTPUT_MAX_LINES: usize = 8;
const AI_OUTPUT_MAX_CHARS_PER_LINE: usize = 320;
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
    pub resource_state_lines: Vec<String>,
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
        let (resource_name, resource_namespace) = ai_prompt_resource_identity(&self.resource);
        let mut sections = Vec::new();
        sections.push(format!(
            "Resource\n- kind: {}\n- name: {}\n- namespace: {}",
            self.resource.kind(),
            resource_name,
            resource_namespace,
        ));
        sections.push(format!(
            "Cluster Context\n- current_context: {}",
            self.cluster_context.as_deref().unwrap_or("-"),
        ));
        sections.push(render_list_section(
            "Resource State",
            &self.resource_state_lines,
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

fn ai_prompt_resource_identity(resource: &ResourceRef) -> (&str, &str) {
    if matches!(resource, ResourceRef::Secret(_, _)) {
        ("<redacted>", "<redacted>")
    } else {
        (resource.name(), resource.namespace().unwrap_or("-"))
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
    let agent_config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(provider.timeout_secs)))
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = agent_config.into();

    let user_prompt = context.render_prompt();
    let raw_json = match provider.provider {
        AiProviderKind::OpenAi => {
            let api_key = std::env::var(&provider.api_key_env).with_context(|| {
                format!("AI API key env var '{}' is not set", provider.api_key_env)
            })?;
            call_openai(&agent, provider, &api_key, system_prompt, &user_prompt)?
        }
        AiProviderKind::Anthropic => {
            let api_key = std::env::var(&provider.api_key_env).with_context(|| {
                format!("AI API key env var '{}' is not set", provider.api_key_env)
            })?;
            call_anthropic(&agent, provider, &api_key, system_prompt, &user_prompt)?
        }
        AiProviderKind::ClaudeCli | AiProviderKind::CodexCli => {
            call_ai_cli(provider, system_prompt, &user_prompt)?
        }
    };
    let structured = parse_structured_response(&raw_json)?;
    let raw_json =
        serde_json::to_string(&structured).context("failed to encode sanitized AI response")?;

    Ok(AiAnalysisResult {
        provider_label: ai_analysis_provider_label(provider),
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
    extract_openai_message_content(&value)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("OpenAI response did not include message.content"))
}

fn extract_openai_message_content(value: &Value) -> Option<&str> {
    value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
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
fn call_ai_cli(
    provider: &AiProviderConfig,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String> {
    let command = ai_cli_command(provider);
    let args = ai_cli_args(provider, system_prompt, user_prompt);
    let mut child = Command::new(&command)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to launch AI CLI '{command}'"))?;
    let started = Instant::now();
    loop {
        if child
            .try_wait()
            .with_context(|| format!("failed to poll AI CLI '{command}'"))?
            .is_some()
        {
            let output = child
                .wait_with_output()
                .with_context(|| format!("failed to read AI CLI '{command}' output"))?;
            if output.status.success() {
                return String::from_utf8(output.stdout)
                    .context("AI CLI returned non-UTF-8 stdout");
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            let message = sanitize_provider_error_message(stderr.trim());
            if message.is_empty() {
                bail!("AI CLI '{command}' failed with status {}", output.status);
            }
            bail!("AI CLI '{command}' failed: {message}");
        }
        if started.elapsed() >= Duration::from_secs(provider.timeout_secs) {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "AI CLI '{}' timed out after {}s",
                command,
                provider.timeout_secs
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn ai_cli_command(provider: &AiProviderConfig) -> String {
    provider
        .command
        .clone()
        .unwrap_or_else(|| match provider.provider {
            AiProviderKind::ClaudeCli => "claude".to_string(),
            AiProviderKind::CodexCli => "codex".to_string(),
            AiProviderKind::OpenAi | AiProviderKind::Anthropic => {
                unreachable!("not a CLI provider")
            }
        })
}

fn ai_cli_args(provider: &AiProviderConfig, system_prompt: &str, user_prompt: &str) -> Vec<String> {
    let prompt = format!("{system_prompt}\n\n{user_prompt}");
    let default_args = match provider.provider {
        AiProviderKind::ClaudeCli => vec!["-p".to_string(), "$PROMPT".to_string()],
        AiProviderKind::CodexCli => vec!["exec".to_string(), "$PROMPT".to_string()],
        AiProviderKind::OpenAi | AiProviderKind::Anthropic => Vec::new(),
    };
    let args = if provider.args.is_empty() {
        default_args
    } else {
        provider.args.clone()
    };
    args.into_iter()
        .map(|arg| render_ai_cli_arg_template(&arg, system_prompt, &prompt))
        .collect()
}

fn render_ai_cli_arg_template(arg: &str, system_prompt: &str, prompt: &str) -> String {
    let mut rendered = String::with_capacity(arg.len());
    let mut rest = arg;
    while let Some(index) = rest.find('$') {
        rendered.push_str(&rest[..index]);
        rest = &rest[index..];
        if let Some(next) = rest.strip_prefix("$SYSTEM_PROMPT") {
            rendered.push_str(system_prompt);
            rest = next;
        } else if let Some(next) = rest.strip_prefix("$PROMPT") {
            rendered.push_str(prompt);
            rest = next;
        } else {
            rendered.push('$');
            rest = &rest['$'.len_utf8()..];
        }
    }
    rendered.push_str(rest);
    rendered
}

#[cold]
#[inline(never)]
fn parse_structured_response(raw: &str) -> Result<StructuredAiResponse> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("AI provider returned an empty response");
    }
    let mut last_error = None;
    for candidate in std::iter::once(trimmed).chain(json_object_candidates(trimmed)) {
        match parse_structured_response_value(candidate)
            .and_then(normalize_structured_response)
            .and_then(validate_structured_response)
        {
            Ok(response) => return Ok(response),
            Err(err) => last_error = Some(err),
        }
    }
    Err(anyhow!(
        "AI response was not valid structured JSON: {}",
        last_error
            .map(|err| err.to_string())
            .unwrap_or_else(|| "AI response did not include a JSON object".to_string())
    ))
}

fn parse_structured_response_value(raw: &str) -> Result<Value> {
    serde_json::from_str::<Value>(raw).map_err(Into::into)
}

fn json_object_candidates(raw: &str) -> Vec<&str> {
    raw.char_indices()
        .filter_map(|(start, ch)| {
            if ch == '{' {
                json_object_end(raw, start).map(|end| &raw[start..=end])
            } else {
                None
            }
        })
        .collect()
}

fn json_object_end(raw: &str, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in raw[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth = depth.saturating_add(1),
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn normalize_structured_response(value: Value) -> Result<StructuredAiResponse> {
    let Value::Object(mut object) = value else {
        bail!("AI response JSON root was not an object");
    };
    Ok(StructuredAiResponse {
        summary: stringify_ai_field(object.remove("summary").unwrap_or(Value::Null)),
        likely_causes: normalize_ai_lines(object.remove("likely_causes").unwrap_or(Value::Null)),
        next_steps: normalize_ai_lines(object.remove("next_steps").unwrap_or(Value::Null)),
        uncertainty: normalize_ai_lines(object.remove("uncertainty").unwrap_or(Value::Null)),
    })
}

fn normalize_ai_lines(value: Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .into_iter()
            .take(AI_OUTPUT_MAX_ITEMS)
            .map(stringify_ai_field)
            .collect(),
        Value::Null => Vec::new(),
        value => vec![stringify_ai_field(value)],
    }
}

fn stringify_ai_field(value: Value) -> String {
    match value {
        Value::String(value) => value,
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Array(items) => items
            .into_iter()
            .map(stringify_ai_field)
            .filter(|value| !value.trim().is_empty())
            .collect::<Vec<_>>()
            .join("; "),
        Value::Object(object) => object
            .into_iter()
            .map(|(key, value)| {
                let value = stringify_ai_field(value);
                if value.trim().is_empty() {
                    key
                } else {
                    format!("{key}: {value}")
                }
            })
            .collect::<Vec<_>>()
            .join("; "),
        Value::Null => String::new(),
    }
}

#[cold]
#[inline(never)]
fn validate_structured_response(response: StructuredAiResponse) -> Result<StructuredAiResponse> {
    if response.summary.trim().is_empty() {
        bail!("AI response omitted summary");
    }
    Ok(StructuredAiResponse {
        summary: sanitize_ai_model_output(response.summary.trim()),
        likely_causes: trim_ai_output_lines(response.likely_causes),
        next_steps: trim_ai_output_lines(response.next_steps),
        uncertainty: trim_ai_output_lines(response.uncertainty),
    })
}

#[cold]
#[inline(never)]
fn trim_ai_output_lines(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|line| sanitize_ai_model_output(line.trim()))
        .filter(|line| !line.is_empty())
        .take(AI_OUTPUT_MAX_ITEMS)
        .collect()
}

fn sanitize_ai_model_output(value: &str) -> String {
    let mut output = Vec::new();
    let mut truncated = false;

    for (index, line) in value.lines().enumerate() {
        if index >= AI_OUTPUT_MAX_LINES {
            truncated = true;
            break;
        }
        let sanitized = sanitize_provider_error_line(line.trim());
        if sanitized.is_empty() {
            continue;
        }
        output.push(truncate_ai_output_line(&sanitized, &mut truncated));
    }

    if truncated {
        output.push("[truncated]".to_string());
    }

    output.join("\n")
}

fn truncate_ai_output_line(line: &str, truncated: &mut bool) -> String {
    let mut char_indices = line.char_indices();
    let Some((cutoff, _)) = char_indices.nth(AI_OUTPUT_MAX_CHARS_PER_LINE) else {
        return line.to_string();
    };
    *truncated = true;
    format!("{}...", &line[..cutoff])
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
        .map(|message| {
            let message = sanitize_provider_error_message(message);
            if message.is_empty() {
                format!("{provider} request failed")
            } else {
                format!("{provider} request failed: {message}")
            }
        })
        .unwrap_or_else(|| format!("{provider} request failed"))
}

fn sanitize_provider_error_message(message: &str) -> String {
    let message = message.trim();
    if message.is_empty() {
        return String::new();
    }
    let scan_message = provider_error_scan_window(message);
    if provider_error_looks_like_prompt_echo(scan_message) {
        return "provider error output redacted because it included AI context".to_string();
    }

    let mut saw_more_lines = false;
    let mut sanitized_lines = Vec::new();
    for (idx, line) in scan_message.lines().enumerate() {
        if idx >= PROVIDER_ERROR_MAX_LINES {
            saw_more_lines = true;
            break;
        }
        let line = sanitize_provider_error_line(line);
        if !line.is_empty() {
            sanitized_lines.push(line);
        }
    }
    let mut sanitized = sanitized_lines.join("\n");
    if saw_more_lines || scan_message.len() < message.len() {
        sanitized.push_str("\n[truncated]");
    }
    truncate_provider_error(&sanitized)
}

fn provider_error_scan_window(message: &str) -> &str {
    if message.len() <= PROVIDER_ERROR_SCAN_MAX_CHARS {
        return message;
    }
    let cutoff = message.floor_char_boundary(PROVIDER_ERROR_SCAN_MAX_CHARS);
    &message[..cutoff]
}

fn provider_error_looks_like_prompt_echo(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    if lower.contains("yaml excerpt")
        || lower.contains("resource\n- kind:")
        || lower.contains("cluster context\n- current_context")
        || lower.contains("secret manifests are not sent to ai")
    {
        return true;
    }
    let has_section_header = message.lines().any(|line| {
        matches!(
            line.trim(),
            "Resource"
                | "Cluster Context"
                | "Resource State"
                | "Metadata"
                | "Workflow Context"
                | "Failure Focus"
                | "Rollout Context"
                | "Network Context"
                | "Triage Context"
                | "Issues"
                | "Events"
                | "Probes"
                | "Logs"
                | "YAML Excerpt"
        )
    });
    if has_section_header && message.lines().nth(1).is_some() {
        return true;
    }
    let markers = [
        "cluster context",
        "resource state",
        "yaml excerpt",
        "workflow context",
        "failure focus",
        "rollout context",
        "network context",
        "triage context",
        "issues",
        "events",
        "probes",
        "logs",
    ];
    markers
        .iter()
        .filter(|marker| lower.contains(**marker))
        .take(2)
        .count()
        >= 2
}

fn sanitize_provider_error_line(line: &str) -> String {
    let mut redacted = Vec::new();
    let mut state = ProviderErrorRedactionState::None;
    for token in line.split_whitespace() {
        if state != ProviderErrorRedactionState::None {
            if state == ProviderErrorRedactionState::ConsumeValue {
                let skipped_scheme =
                    token.eq_ignore_ascii_case("bearer") || token.eq_ignore_ascii_case("basic");
                state = if skipped_scheme {
                    ProviderErrorRedactionState::ConsumeValue
                } else {
                    ProviderErrorRedactionState::None
                };
                continue;
            }
            if state == ProviderErrorRedactionState::SeparatorOrValue && matches!(token, ":" | "=")
            {
                redacted.push(token.to_string());
                state = ProviderErrorRedactionState::Value;
                continue;
            }

            let redacted_scheme =
                token.eq_ignore_ascii_case("bearer") || token.eq_ignore_ascii_case("basic");
            redacted.push("[redacted]".to_string());
            state = if redacted_scheme {
                ProviderErrorRedactionState::Value
            } else {
                ProviderErrorRedactionState::None
            };
            continue;
        }

        let lower = normalize_provider_error_key_token(token);
        if provider_error_token_is_split_sensitive_key(token) {
            redacted.push(format!("{token} [redacted]"));
            state = ProviderErrorRedactionState::ConsumeValue;
            continue;
        }
        if is_sensitive_error_key(&lower) {
            redacted.push(token.to_string());
            state = ProviderErrorRedactionState::SeparatorOrValue;
            continue;
        }
        if let Some(redacted_token) = redact_secret_path_token(token) {
            redacted.push(redacted_token);
            continue;
        }

        redacted.push(sanitize_provider_error_token(token));
    }
    redacted.join(" ")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProviderErrorRedactionState {
    None,
    SeparatorOrValue,
    Value,
    ConsumeValue,
}

fn normalize_provider_error_key_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .to_ascii_lowercase()
}

fn provider_error_token_is_split_sensitive_key(token: &str) -> bool {
    let normalized = token
        .trim_matches(|ch: char| {
            !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-' && ch != ':' && ch != '='
        })
        .to_ascii_lowercase();
    normalized
        .strip_suffix(':')
        .or_else(|| normalized.strip_suffix('='))
        .is_some_and(is_sensitive_error_key)
}

fn sanitize_provider_error_token(token: &str) -> String {
    if token.contains("://") && token.contains('@') {
        return "[redacted-uri]".to_string();
    }
    if let Some(redacted_token) = redact_secret_path_token(token) {
        return redacted_token;
    }

    for separator in ['=', ':'] {
        if let Some((key, _value)) = token.split_once(separator) {
            let normalized = normalize_provider_error_key_token(key);
            if is_sensitive_error_key(&normalized) {
                return format!("{key}{separator}<redacted>");
            }
        }
    }

    token.to_string()
}

fn redact_secret_path_token(token: &str) -> Option<String> {
    let lower = token.to_ascii_lowercase();
    for marker in ["secret/", "secrets/"] {
        if let Some(index) = lower.find(marker) {
            let value_start = index + marker.len();
            let value_len = token[value_start..]
                .char_indices()
                .find_map(|(idx, ch)| {
                    (!matches!(ch, '-' | '_' | '.' | '/' | ':' | '@')
                        && !ch.is_ascii_alphanumeric())
                    .then_some(idx)
                })
                .unwrap_or_else(|| token[value_start..].len());
            let value_end = value_start + value_len;
            if value_end == value_start {
                return None;
            }
            return Some(format!(
                "{}<redacted>{}",
                &token[..value_start],
                &token[value_end..]
            ));
        }
    }
    None
}

fn is_sensitive_error_key(key: &str) -> bool {
    key == "authorization"
        || key == "password"
        || key == "passwd"
        || key == "secret"
        || key == "token"
        || key == "api_key"
        || key == "apikey"
        || key.ends_with("_password")
        || key.ends_with("_secret")
        || key.ends_with("_token")
}

fn truncate_provider_error(message: &str) -> String {
    let mut char_indices = message.char_indices();
    let Some((cutoff, _)) = char_indices.nth(PROVIDER_ERROR_MAX_CHARS) else {
        return message.to_string();
    };
    format!("{}…", &message[..cutoff])
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
    fn parse_structured_response_skips_noisy_braces_before_valid_json() {
        let parsed = parse_structured_response(
            r#"debug: ignored {not json}
```json
{
  "summary": "ok {still text}",
  "likely_causes": ["a"],
  "next_steps": ["b"],
  "uncertainty": ["c"]
}
```
trailing note {also ignored}"#,
        )
        .expect("valid structured json is extracted after noisy braces");

        assert_eq!(parsed.summary, "ok {still text}");
        assert_eq!(parsed.likely_causes, vec!["a"]);
        assert_eq!(parsed.next_steps, vec!["b"]);
        assert_eq!(parsed.uncertainty, vec!["c"]);
    }

    #[test]
    fn parse_structured_response_accepts_object_items_from_cli_providers() {
        let parsed = parse_structured_response(
            r#"{
                "summary": {"status": "failing", "resource": "pod"},
                "likely_causes": [
                    {"cause": "database unavailable", "evidence": ["connection refused", "backoff"]},
                    "image pulled successfully"
                ],
                "next_steps": [
                    {"action": "inspect postgres service", "command": "kubectl get svc"}
                ],
                "uncertainty": [{"reason": "logs truncated"}]
            }"#,
        )
        .expect("object-shaped provider output normalizes");

        assert_eq!(parsed.summary, "resource: pod; status: failing");
        assert_eq!(
            parsed.likely_causes,
            vec![
                "cause: database unavailable; evidence: connection refused; backoff",
                "image pulled successfully"
            ]
        );
        assert_eq!(
            parsed.next_steps,
            vec!["action: inspect postgres service; command: kubectl get svc"]
        );
        assert_eq!(parsed.uncertainty, vec!["reason: logs truncated"]);
    }

    #[test]
    fn parse_structured_response_accepts_scalar_arrays_from_cli_providers() {
        let parsed = parse_structured_response(
            r#"{
                "summary": "ok",
                "likely_causes": [404, true],
                "next_steps": "check events",
                "uncertainty": null
            }"#,
        )
        .expect("scalar provider output normalizes");

        assert_eq!(parsed.likely_causes, vec!["404", "true"]);
        assert_eq!(parsed.next_steps, vec!["check events"]);
        assert!(parsed.uncertainty.is_empty());
    }

    #[test]
    fn parse_structured_response_redacts_model_output_secrets() {
        let parsed = parse_structured_response(
            r#"{
                "summary": "database password : hunter2 leaked",
                "likely_causes": [
                    "client logged Authorization: Bearer live-token",
                    "dsn=postgres://user:pass@db:5432/app",
                    "kubelet mentioned secret/prod-db-password and secrets/team/api-token"
                ],
                "next_steps": ["rotate api_key=sk-live"],
                "uncertainty": ["secret: literal-value"]
            }"#,
        )
        .expect("structured response parses");

        let rendered = format!(
            "{}\n{}\n{}\n{}",
            parsed.summary,
            parsed.likely_causes.join("\n"),
            parsed.next_steps.join("\n"),
            parsed.uncertainty.join("\n")
        );
        assert!(rendered.contains("password : [redacted]"), "{rendered}");
        assert!(rendered.contains("Authorization: [redacted]"), "{rendered}");
        assert!(rendered.contains("[redacted-uri]"), "{rendered}");
        assert!(rendered.contains("secret/<redacted>"), "{rendered}");
        assert!(rendered.contains("secrets/<redacted>"), "{rendered}");
        assert!(rendered.contains("api_key=<redacted>"), "{rendered}");
        assert!(rendered.contains("secret: [redacted]"), "{rendered}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(!rendered.contains("live-token"), "{rendered}");
        assert!(!rendered.contains("user:pass"), "{rendered}");
        assert!(!rendered.contains("prod-db-password"), "{rendered}");
        assert!(!rendered.contains("api-token"), "{rendered}");
        assert!(!rendered.contains("sk-live"), "{rendered}");
        assert!(!rendered.contains("literal-value"), "{rendered}");
    }

    #[test]
    fn parse_structured_response_bounds_large_model_output() {
        let long_line = "x".repeat(AI_OUTPUT_MAX_CHARS_PER_LINE + 40);
        let summary = (0..12)
            .map(|idx| format!("summary-{idx}-{long_line}"))
            .collect::<Vec<_>>()
            .join("\\n");
        let likely_causes = (0..20)
            .map(|idx| format!(r#""cause-{idx}""#))
            .collect::<Vec<_>>()
            .join(",");
        let raw = format!(
            r#"{{
                "summary": "{summary}",
                "likely_causes": [{likely_causes}],
                "next_steps": ["check"],
                "uncertainty": []
            }}"#
        );

        let parsed = parse_structured_response(&raw).expect("large response parses");

        assert_eq!(parsed.likely_causes.len(), AI_OUTPUT_MAX_ITEMS);
        assert!(parsed.summary.contains("[truncated]"), "{}", parsed.summary);
        assert!(!parsed.summary.contains("summary-9"), "{}", parsed.summary);
        assert!(
            parsed
                .summary
                .lines()
                .filter(|line| *line != "[truncated]")
                .all(|line| line.chars().count() <= AI_OUTPUT_MAX_CHARS_PER_LINE + 3),
            "{}",
            parsed.summary
        );
    }

    #[test]
    fn openai_message_content_extracts_standard_chat_completion_text() {
        let value = json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "{\"summary\":\"ok\",\"likely_causes\":[],\"next_steps\":[],\"uncertainty\":[]}"
                    }
                }
            ]
        });

        assert_eq!(
            extract_openai_message_content(&value),
            Some("{\"summary\":\"ok\",\"likely_causes\":[],\"next_steps\":[],\"uncertainty\":[]}")
        );
    }

    #[test]
    fn openai_message_content_rejects_missing_text_content() {
        let value = json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": null
                    }
                }
            ]
        });

        assert_eq!(extract_openai_message_content(&value), None);
    }

    #[test]
    fn context_prompt_renders_all_sections() {
        let prompt = AiAnalysisContext {
            resource: ResourceRef::Pod("api-0".into(), "prod".into()),
            cluster_context: Some("staging".into()),
            resource_state_lines: vec!["status: CrashLoopBackOff".into()],
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
        assert!(prompt.contains("Resource State"));
        assert!(prompt.contains("Failure Focus"));
        assert!(prompt.contains("Logs"));
        assert!(prompt.contains("kind: Pod"));
    }

    #[test]
    fn context_prompt_redacts_secret_resource_identity() {
        let prompt = AiAnalysisContext {
            resource: ResourceRef::Secret("database-password".into(), "prod".into()),
            cluster_context: Some("prod-cluster".into()),
            resource_state_lines: Vec::new(),
            metadata_lines: Vec::new(),
            workflow_title: None,
            workflow_lines: Vec::new(),
            issue_lines: Vec::new(),
            event_lines: Vec::new(),
            probe_lines: Vec::new(),
            log_lines: Vec::new(),
            yaml_excerpt: Some("# redacted: Secret manifests are not sent to AI".into()),
        }
        .render_prompt();

        assert!(prompt.contains("- kind: Secret"), "{prompt}");
        assert!(prompt.contains("- name: <redacted>"), "{prompt}");
        assert!(prompt.contains("- namespace: <redacted>"), "{prompt}");
        assert!(!prompt.contains("database-password"), "{prompt}");
        assert!(!prompt.contains("namespace: prod\n"), "{prompt}");
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
    fn provider_error_sanitizer_redacts_prompt_echoes() {
        let message = sanitize_provider_error_message(
            "failed request\nResource State\n- status: CrashLoopBackOff\nCluster Context\n- current_context: prod\nYAML Excerpt\npassword=literal-secret",
        );

        assert_eq!(
            message,
            "provider error output redacted because it included AI context"
        );
        assert!(!message.contains("literal-secret"));
        assert!(!message.contains("CrashLoopBackOff"));
    }

    #[test]
    fn provider_error_sanitizer_redacts_single_section_prompt_echoes() {
        let message = sanitize_provider_error_message(
            "invalid output\nYAML Excerpt\n```yaml\nimagePullSecrets:\n- name: registry-credentials\n```",
        );

        assert_eq!(
            message,
            "provider error output redacted because it included AI context"
        );
        assert!(!message.contains("registry-credentials"));
    }

    #[test]
    fn provider_error_sanitizer_redacts_log_section_prompt_echoes() {
        let message = sanitize_provider_error_message(
            "invalid output\nLogs\n- pod prod/api current: session secret literal-value",
        );

        assert_eq!(
            message,
            "provider error output redacted because it included AI context"
        );
        assert!(!message.contains("literal-value"));
        assert!(!message.contains("prod/api"));
    }

    #[test]
    fn provider_error_sanitizer_redacts_specialized_context_prompt_echoes() {
        let message = sanitize_provider_error_message(
            "bad response\nRollout Context\n- deployment prod/api waiting on unavailable replicas",
        );

        assert_eq!(
            message,
            "provider error output redacted because it included AI context"
        );
        assert!(!message.contains("prod/api"));
    }

    #[test]
    fn provider_error_sanitizer_preserves_concise_errors_but_redacts_values() {
        let message = sanitize_provider_error_message(
            "request failed Authorization: Bearer live-token dsn=postgres://user:pass@db:5432/app token=secret-value",
        );

        assert!(message.contains("request failed"), "{message}");
        assert!(message.contains("Authorization: [redacted]"), "{message}");
        assert!(message.contains("[redacted-uri]"), "{message}");
        assert!(message.contains("token=<redacted>"), "{message}");
        assert!(!message.contains("live-token"), "{message}");
        assert!(!message.contains("user:pass"), "{message}");
        assert!(!message.contains("secret-value"), "{message}");
    }

    #[test]
    fn provider_error_sanitizer_redacts_split_sensitive_values() {
        let message = sanitize_provider_error_message(
            "bad request password: hunter2 api_key: sk-live token=inline-token",
        );

        assert!(message.contains("password: [redacted]"), "{message}");
        assert!(message.contains("api_key: [redacted]"), "{message}");
        assert!(message.contains("token=<redacted>"), "{message}");
        assert!(!message.contains("hunter2"), "{message}");
        assert!(!message.contains("sk-live"), "{message}");
        assert!(!message.contains("inline-token"), "{message}");
    }

    #[test]
    fn provider_error_sanitizer_redacts_spaced_sensitive_values() {
        let message = sanitize_provider_error_message(
            "bad request password : hunter2 api_key = sk-live Authorization : Bearer live-token",
        );

        assert!(message.contains("password : [redacted]"), "{message}");
        assert!(message.contains("api_key = [redacted]"), "{message}");
        assert!(message.contains("Authorization : [redacted]"), "{message}");
        assert!(!message.contains("hunter2"), "{message}");
        assert!(!message.contains("sk-live"), "{message}");
        assert!(!message.contains("live-token"), "{message}");
    }

    #[test]
    fn provider_error_sanitizer_bounds_huge_error_scans() {
        let huge = format!(
            "request failed\n{}\npassword=hunter2",
            "x".repeat(PROVIDER_ERROR_SCAN_MAX_CHARS + 2048)
        );

        let message = sanitize_provider_error_message(&huge);

        assert!(message.contains("request failed"), "{message}");
        assert!(message.ends_with('…'), "{message}");
        assert!(message.chars().count() <= PROVIDER_ERROR_MAX_CHARS + 1);
        assert!(!message.contains("hunter2"), "{message}");
    }

    #[test]
    fn extract_provider_error_sanitizes_provider_messages() {
        let value = json!({
            "error": {
                "message": "bad input api_key=sk-live password=hunter2"
            }
        });

        let message = extract_provider_error("OpenAI", &value);

        assert_eq!(
            message,
            "OpenAI request failed: bad input api_key=<redacted> password=<redacted>"
        );
    }

    #[test]
    fn ai_cli_defaults_use_prompt_argument() {
        let provider = AiProviderConfig {
            provider: AiProviderKind::ClaudeCli,
            model: String::new(),
            api_key_env: String::new(),
            endpoint: None,
            timeout_secs: 5,
            max_output_tokens: 128,
            temperature: Some(0.1),
            command: None,
            args: Vec::new(),
            action: None,
        };

        assert_eq!(ai_cli_command(&provider), "claude");
        let args = ai_cli_args(&provider, "system", "user");
        assert_eq!(args[0], "-p");
        assert!(args[1].contains("system"));
        assert!(args[1].contains("user"));
    }

    #[test]
    fn codex_cli_defaults_to_exec() {
        let provider = AiProviderConfig {
            provider: AiProviderKind::CodexCli,
            model: String::new(),
            api_key_env: String::new(),
            endpoint: None,
            timeout_secs: 5,
            max_output_tokens: 128,
            temperature: Some(0.1),
            command: None,
            args: Vec::new(),
            action: None,
        };

        assert_eq!(ai_cli_command(&provider), "codex");
        let args = ai_cli_args(&provider, "system", "user");
        assert_eq!(args[0], "exec");
        assert!(args[1].contains("system"));
        assert!(args[1].contains("user"));
    }

    #[test]
    fn ai_cli_args_do_not_recursively_expand_placeholders() {
        let provider = AiProviderConfig {
            provider: AiProviderKind::CodexCli,
            model: String::new(),
            api_key_env: String::new(),
            endpoint: None,
            timeout_secs: 5,
            max_output_tokens: 128,
            temperature: Some(0.1),
            command: None,
            args: vec![
                "--system".into(),
                "$SYSTEM_PROMPT".into(),
                "--prompt".into(),
                "$PROMPT".into(),
                "--literal".into(),
                "$UNKNOWN".into(),
            ],
            action: None,
        };

        let args = ai_cli_args(
            &provider,
            "system mentions $PROMPT literally",
            "user mentions $SYSTEM_PROMPT literally",
        );

        assert_eq!(args[1], "system mentions $PROMPT literally");
        assert!(args[3].contains("system mentions $PROMPT literally"));
        assert!(args[3].contains("user mentions $SYSTEM_PROMPT literally"));
        assert_eq!(args[5], "$UNKNOWN");
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
            command: None,
            args: Vec::new(),
            action: None,
        };
        let context = AiAnalysisContext {
            resource: ResourceRef::Pod("api-0".into(), "prod".into()),
            cluster_context: None,
            resource_state_lines: Vec::new(),
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
