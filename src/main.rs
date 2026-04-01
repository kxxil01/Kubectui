//! KubecTUI entry point.
//!
//! This module wires terminal lifecycle management, the application state machine,
//! the Kubernetes client, and the ratatui rendering pipeline.

#![cfg_attr(test, allow(clippy::field_reassign_with_default))]

mod action;
mod ai;
mod async_types;
mod detail_fetch;
mod event_handlers;
mod flux_reconcile;
mod mutation_helpers;
mod runtime_helpers;
mod selection_helpers;
mod startup;
mod terminal;

use async_types::*;
use detail_fetch::*;
use event_handlers::*;
use flux_reconcile::*;
use mutation_helpers::*;
use runtime_helpers::{next_request_id, run_app, start_watch_manager};
use selection_helpers::*;
use std::{
    collections::HashMap,
    io,
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream, KeyCode};
use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::{Terminal, backend::CrosstermBackend};

use kubectui::ui::components::port_forward_dialog::PortForwardDialog;
use kubectui::{
    action_history::{ActionKind, ActionStatus},
    app::{
        AppAction, AppView, DetailViewState, LogsViewerState, ResourceRef, load_config, save_config,
    },
    coordinator::{UpdateCoordinator, UpdateMessage},
    events::apply_action,
    extensions::{
        AiWorkflowKind, ExtensionExecutionMode, ExtensionRegistry, ExtensionSubstitutionContext,
        LoadedExtensionActionKind, PreparedExtensionCommand, load_extensions_registry,
        prepare_command,
    },
    k8s::{
        client::K8sClient,
        exec::{ExecEvent, ExecSessionHandle, fetch_pod_containers, spawn_exec_session},
        logs::{LogsClient, PodRef},
        portforward::PortForwarderService,
        probes::extract_probes_from_pod,
        workload_logs::{
            MAX_WORKLOAD_LOG_STREAMS, WorkloadLogTarget, resolve_workload_log_targets,
        },
    },
    policy::DetailAction,
    runbooks::{LoadedRunbookStepKind, RunbookRegistry, load_runbook_registry},
    secret::{decode_secret_yaml, encode_secret_yaml},
    state::{
        DataPhase, GlobalState, RefreshScope,
        watch::{WatchUpdate, WatchedResource},
    },
    ui,
    workbench::{RunbookTabState, WorkbenchTabKey, WorkbenchTabState},
};

use terminal::{pick_context_at_startup, restore_terminal, setup_terminal};

use crate::ai::{AiAnalysisContext, default_system_prompt_for_workflow, run_ai_analysis};

type AllContainerLogsInfo = (
    String,
    String,
    Vec<String>,
    Vec<(String, String)>,
    ResourceRef,
);

#[derive(Clone)]
struct NodeDebugSessionRuntime {
    client: K8sClient,
    node_name: String,
    pod_name: String,
    namespace: String,
}

async fn spawn_node_debug_cleanup(
    session: NodeDebugSessionRuntime,
    cleanup_tx: tokio::sync::mpsc::Sender<NodeDebugCleanupAsyncResult>,
) {
    tokio::spawn(async move {
        let result = session
            .client
            .delete_node_debug_pod(&session.namespace, &session.pod_name)
            .await
            .map_err(|err| format!("{err:#}"));
        let _ = cleanup_tx
            .send(NodeDebugCleanupAsyncResult {
                node_name: session.node_name,
                pod_name: session.pod_name,
                namespace: session.namespace,
                result,
            })
            .await;
    });
}

async fn cleanup_node_debug_session_if_needed(
    session_id: u64,
    node_debug_sessions: &mut HashMap<u64, NodeDebugSessionRuntime>,
    cleanup_tx: &tokio::sync::mpsc::Sender<NodeDebugCleanupAsyncResult>,
) {
    if let Some(session) = node_debug_sessions.remove(&session_id) {
        spawn_node_debug_cleanup(session, cleanup_tx.clone()).await;
    }
}

fn fail_context_switch(
    app: &mut kubectui::app::AppState,
    global_state: &mut GlobalState,
    message: String,
    pending_runbook_restore: &mut Option<RunbookTabState>,
    snapshot_dirty: &mut bool,
    needs_redraw: &mut bool,
) {
    app.pending_workspace_restore = None;
    pending_runbook_restore.take();
    global_state.set_phase(DataPhase::Error);
    *snapshot_dirty = true;
    *needs_redraw = true;
    app.set_error(message);
}

fn reopen_pending_runbook(
    app: &mut kubectui::app::AppState,
    pending_runbook_restore: &mut Option<RunbookTabState>,
) {
    if let Some(mut tab_state) = pending_runbook_restore.take() {
        tab_state.banner = Some("Workspace applied from runbook step.".to_string());
        app.workbench
            .open_tab(WorkbenchTabState::Runbook(Box::new(tab_state)));
        app.focus = kubectui::app::Focus::Workbench;
        if let Some(WorkbenchTabState::Runbook(tab)) =
            app.workbench.active_tab_mut().map(|tab| &mut tab.state)
        {
            tab.banner = Some("Workspace applied from runbook step.".to_string());
        }
    }
}

struct WorkspaceRestoreRuntime<'a> {
    coordinator: &'a mut UpdateCoordinator,
    workload_log_sessions: &'a mut HashMap<u64, Vec<(String, String, String)>>,
    exec_sessions: &'a mut HashMap<u64, ExecSessionHandle>,
    node_debug_sessions: &'a mut HashMap<u64, NodeDebugSessionRuntime>,
    node_debug_cleanup_tx: &'a tokio::sync::mpsc::Sender<NodeDebugCleanupAsyncResult>,
    port_forwarder: &'a mut PortForwarderService,
    global_state: &'a mut GlobalState,
    client: &'a K8sClient,
    refresh_tx: &'a tokio::sync::mpsc::Sender<RefreshAsyncResult>,
    refresh_state: &'a mut RefreshRuntimeState,
    snapshot_dirty: &'a mut bool,
    events_tx: &'a tokio::sync::mpsc::Sender<EventsAsyncResult>,
    events_state: &'a mut EventsFetchRuntimeState,
    cached_snapshot: &'a kubectui::state::ClusterSnapshot,
    extension_fetch_tx: &'a tokio::sync::mpsc::Sender<ExtensionFetchResult>,
}

async fn apply_workspace_snapshot_and_refresh(
    app: &mut kubectui::app::AppState,
    snapshot: &kubectui::workspaces::WorkspaceSnapshot,
    runtime: &mut WorkspaceRestoreRuntime<'_>,
) {
    let previous_view = app.view();
    app.pending_workspace_restore = None;
    let follow_streams: Vec<(String, String, String)> = app
        .workbench()
        .tabs
        .iter()
        .filter_map(|tab| match &tab.state {
            WorkbenchTabState::PodLogs(logs_tab) => {
                let viewer = &logs_tab.viewer;
                (viewer.follow_mode
                    && !viewer.pod_name.is_empty()
                    && !viewer.pod_namespace.is_empty()
                    && !viewer.container_name.is_empty())
                .then(|| {
                    (
                        viewer.pod_name.clone(),
                        viewer.pod_namespace.clone(),
                        viewer.container_name.clone(),
                    )
                })
            }
            _ => None,
        })
        .collect();
    let workload_sessions_to_stop: Vec<u64> = app
        .workbench()
        .tabs
        .iter()
        .filter_map(|tab| match &tab.state {
            WorkbenchTabState::WorkloadLogs(tab) => Some(tab.session_id),
            _ => None,
        })
        .collect();
    let exec_sessions_to_stop: Vec<u64> = app
        .workbench()
        .tabs
        .iter()
        .filter_map(|tab| match &tab.state {
            WorkbenchTabState::Exec(tab) => Some(tab.session_id),
            _ => None,
        })
        .collect();
    for (pod_name, namespace, container_name) in follow_streams {
        let _ = runtime
            .coordinator
            .stop_log_streaming(&pod_name, &namespace, &container_name)
            .await;
    }
    for session_id in workload_sessions_to_stop {
        if let Some(streams) = runtime.workload_log_sessions.remove(&session_id) {
            for (pod_name, namespace, container_name) in streams {
                let _ = runtime
                    .coordinator
                    .stop_log_streaming(&pod_name, &namespace, &container_name)
                    .await;
            }
        }
    }
    for session_id in exec_sessions_to_stop {
        if let Some(handle) = runtime.exec_sessions.remove(&session_id) {
            let _ = handle.cancel_tx.send(());
        }
        cleanup_node_debug_session_if_needed(
            session_id,
            runtime.node_debug_sessions,
            runtime.node_debug_cleanup_tx,
        )
        .await;
    }
    runtime.port_forwarder.stop_all().await;
    app.tunnel_registry.update_tunnels(Vec::new());
    app.apply_workspace_snapshot(snapshot);
    if previous_view != app.view()
        && !matches!(
            app.view(),
            kubectui::app::AppView::PortForwarding | kubectui::app::AppView::HelmCharts
        )
    {
        request_refresh(
            runtime.refresh_tx,
            runtime.global_state,
            runtime.client,
            namespace_scope(app.get_namespace()).map(str::to_string),
            refresh_options_for_view(app.view(), app.view().is_fluxcd(), false),
            runtime.refresh_state,
            runtime.snapshot_dirty,
        );
        if app.view() == AppView::Events {
            request_events_refresh(
                runtime.events_tx,
                runtime.global_state,
                runtime.client,
                namespace_scope(app.get_namespace()).map(str::to_string),
                runtime.refresh_state.context_generation,
                runtime.events_state,
                runtime.snapshot_dirty,
            );
        }
    }
    if app.view() == kubectui::app::AppView::Extensions {
        spawn_extensions_fetch(
            runtime.client,
            app,
            runtime.cached_snapshot,
            runtime.extension_fetch_tx,
        );
    }
}

fn edit_yaml_in_external_editor(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    file_stem: &str,
    yaml_content: &str,
    skip_unchanged: bool,
) -> Result<Option<String>> {
    struct TempPathGuard(std::path::PathBuf);

    impl Drop for TempPathGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0u64, |d| d.as_nanos() as u64)
        ^ std::process::id() as u64;
    let safe_stem = file_stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    let tmp_path = std::env::temp_dir().join(format!("kubectui-{safe_stem}-{nonce:016x}.yaml"));
    std::fs::write(&tmp_path, yaml_content)
        .with_context(|| format!("failed to write temp file '{}'", tmp_path.display()))?;
    let _tmp_path_guard = TempPathGuard(tmp_path.clone());

    let _ = restore_terminal(terminal);
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_string());
    let editor_args = parse_editor_command(&editor)
        .with_context(|| format!("invalid editor command '{editor}'"))?;
    let (program, args) = editor_args
        .split_first()
        .context("editor command cannot be empty")?;
    let status = std::process::Command::new(program)
        .args(args)
        .arg(&tmp_path)
        .status();

    *terminal = setup_terminal().context("failed to restore terminal after editor exit")?;

    let status = status.with_context(|| format!("failed to launch editor '{editor}'"))?;
    if !status.success() {
        return Ok(None);
    }

    let edited_yaml = std::fs::read_to_string(&tmp_path)
        .with_context(|| format!("failed to read edited file '{}'", tmp_path.display()))?;
    if skip_unchanged && edited_yaml.trim() == yaml_content.trim() {
        return Ok(None);
    }

    Ok(Some(edited_yaml))
}

fn parse_editor_command(command: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;

    for c in command.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }

        match c {
            '\\' => escaped = true,
            '\'' | '"' if quote == Some(c) => quote = None,
            '\'' | '"' if quote.is_none() => quote = Some(c),
            c if c.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }

    if escaped {
        return Err(anyhow::anyhow!(
            "editor command ends with a dangling escape"
        ));
    }
    if quote.is_some() {
        return Err(anyhow::anyhow!("editor command has an unmatched quote"));
    }
    if !current.is_empty() {
        args.push(current);
    }
    if args.is_empty() {
        return Err(anyhow::anyhow!("editor command cannot be empty"));
    }
    Ok(args)
}

fn config_watch_matches_path(event: &notify::Event, config_path: &Path) -> bool {
    let Some(config_name) = config_path.file_name() else {
        return false;
    };
    event
        .paths
        .iter()
        .any(|path| path == config_path || path.file_name().is_some_and(|name| name == config_name))
}

fn create_config_watcher(
    config_path: &Path,
    config_label: &str,
) -> Result<(
    RecommendedWatcher,
    std::sync::mpsc::Receiver<notify::Result<notify::Event>>,
)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |result| {
        let _ = tx.send(result);
    })
    .with_context(|| format!("failed to initialize {config_label} config watcher"))?;
    let parent = config_path
        .parent()
        .context("extensions config path has no parent directory")?;
    watcher
        .watch(parent, RecursiveMode::NonRecursive)
        .with_context(|| {
            format!(
                "failed to watch {config_label} config directory '{}'",
                parent.display(),
            )
        })?;
    Ok((watcher, rx))
}

fn extension_context_for_resource(
    app: &kubectui::app::AppState,
    snapshot: &kubectui::state::ClusterSnapshot,
    resource: &ResourceRef,
) -> ExtensionSubstitutionContext {
    let labels = app
        .detail_view
        .as_ref()
        .filter(|detail| detail.resource.as_ref() == Some(resource))
        .map(|detail| detail.metadata.labels.clone())
        .unwrap_or_else(|| {
            kubectui::detail_sections::metadata_for_resource(snapshot, resource).labels
        });
    ExtensionSubstitutionContext::from_resource(
        resource,
        app.current_context_name.as_deref(),
        labels,
    )
}

fn build_extension_output_lines(output: &std::process::Output) -> Vec<String> {
    let mut lines = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    lines.extend(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .map(|line| format!("stderr: {line}")),
    );
    lines
}

fn run_extension_command(prepared: PreparedExtensionCommand) -> ExtensionCommandRunResult {
    let mut command = std::process::Command::new(&prepared.program);
    command.args(&prepared.args);
    if let Some(cwd) = &prepared.cwd {
        command.current_dir(cwd);
    }
    if !prepared.env.is_empty() {
        command.envs(&prepared.env);
    }

    match command.output() {
        Ok(output) => ExtensionCommandRunResult {
            lines: build_extension_output_lines(&output),
            success: output.status.success(),
            exit_code: output.status.code(),
            error: (!output.status.success()).then(|| {
                output
                    .status
                    .code()
                    .map(|code| format!("extension exited with status {code}"))
                    .unwrap_or_else(|| "extension terminated by signal".to_string())
            }),
        },
        Err(err) => ExtensionCommandRunResult {
            lines: Vec::new(),
            success: false,
            exit_code: None,
            error: Some(format!("failed to launch extension command: {err}")),
        },
    }
}

fn run_extension_command_in_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    prepared: &PreparedExtensionCommand,
) -> Result<std::process::ExitStatus> {
    let _ = restore_terminal(terminal);

    let mut command = std::process::Command::new(&prepared.program);
    command.args(&prepared.args);
    if let Some(cwd) = &prepared.cwd {
        command.current_dir(cwd);
    }
    if !prepared.env.is_empty() {
        command.envs(&prepared.env);
    }

    let status = command
        .status()
        .with_context(|| format!("failed to launch extension '{}'", prepared.preview));

    *terminal = setup_terminal().context("failed to restore terminal after extension exit")?;
    status
}

fn notify_extension_load_warnings(app: &mut kubectui::app::AppState, warnings: &[String]) {
    for warning in warnings {
        app.push_toast(warning.clone(), true);
    }
}

fn notify_runbook_load_warnings(app: &mut kubectui::app::AppState, warnings: &[String]) {
    for warning in warnings {
        app.push_toast(warning.clone(), true);
    }
}

fn refresh_palette_extensions(
    app: &mut kubectui::app::AppState,
    snapshot: &kubectui::state::ClusterSnapshot,
    registry: &ExtensionRegistry,
) {
    let selected = selected_resource(app, snapshot);
    let actions = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.as_ref())
        .or(selected.as_ref())
        .map(|resource| registry.palette_actions_for(resource))
        .unwrap_or_default();
    app.command_palette.set_extension_actions(actions);
}

fn refresh_palette_runbooks(
    app: &mut kubectui::app::AppState,
    snapshot: &kubectui::state::ClusterSnapshot,
    registry: &RunbookRegistry,
) {
    let selected = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.as_ref())
        .cloned()
        .or_else(|| selected_resource(app, snapshot));
    let runbooks = registry.palette_runbooks_for(selected.as_ref());
    app.command_palette.set_runbooks(runbooks, selected);
}

fn refresh_palette_resources(
    app: &mut kubectui::app::AppState,
    snapshot: &kubectui::state::ClusterSnapshot,
) {
    let entries = kubectui::global_search::collect_global_resource_search_entries(snapshot)
        .into_iter()
        .map(
            |entry| kubectui::ui::components::command_palette::PaletteResourceEntry {
                resource: entry.resource,
                title: entry.title,
                subtitle: entry.subtitle,
                aliases: entry.aliases,
                badge_label: entry.view.label().to_string(),
            },
        )
        .collect();
    app.command_palette.set_resource_entries(entries);
}

fn refresh_palette_activity(app: &mut kubectui::app::AppState) {
    app.command_palette.set_activity_entries(
        kubectui::ui::components::command_palette::collect_activity_entries(app),
    );
}

fn truncate_ai_block(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let end = value
        .char_indices()
        .nth(max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(value.len());
    format!("{}…", &value[..end])
}

const AI_METADATA_MAX_LINES: usize = 12;
const AI_METADATA_MAX_CHARS: usize = 1_400;
const AI_ISSUE_MAX_LINES: usize = 8;
const AI_ISSUE_MAX_CHARS: usize = 1_400;
const AI_EVENT_MAX_LINES: usize = 8;
const AI_EVENT_MAX_CHARS: usize = 1_400;
const AI_PROBE_MAX_LINES: usize = 12;
const AI_PROBE_MAX_CHARS: usize = 1_800;
const AI_LOG_MAX_LINES: usize = 20;
const AI_LOG_MAX_CHARS: usize = 2_800;
const AI_YAML_MAX_CHARS: usize = 2_000;

fn cap_ai_lines(lines: Vec<String>, max_items: usize, max_total_chars: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut total_chars = 0usize;
    for line in lines.into_iter().take(max_items) {
        let line_chars = line.chars().count();
        if !result.is_empty() && total_chars + line_chars > max_total_chars {
            break;
        }
        total_chars += line_chars;
        result.push(line);
    }
    result
}

fn normalize_ai_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn ai_key_is_sensitive(key: &str) -> bool {
    let normalized = normalize_ai_key(key);
    if normalized.is_empty() {
        return false;
    }
    if matches!(
        normalized.as_str(),
        "data"
            | "stringdata"
            | "token"
            | "password"
            | "passwd"
            | "authorization"
            | "apikey"
            | "accesskey"
            | "secretaccesskey"
            | "privatekey"
            | "clientsecret"
            | "certificate"
            | "cacrt"
            | "tlscrt"
            | "tlskey"
    ) {
        return true;
    }
    if normalized.ends_with("name") || normalized.ends_with("ref") || normalized.ends_with("refs") {
        return false;
    }
    normalized.contains("token")
        || normalized.contains("password")
        || normalized.contains("secret")
        || normalized.contains("apikey")
        || normalized.contains("accesskey")
        || normalized.contains("privatekey")
        || normalized.contains("certificate")
}

fn redact_ai_yaml_value(value: &mut serde_yaml::Value) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            let env_name_is_sensitive = map
                .get(serde_yaml::Value::String("name".to_string()))
                .and_then(serde_yaml::Value::as_str)
                .is_some_and(ai_key_is_sensitive);
            if env_name_is_sensitive
                && let Some(entry) = map.get_mut(serde_yaml::Value::String("value".to_string()))
            {
                *entry = serde_yaml::Value::String("<redacted>".to_string());
            }
            for (key, nested) in map.iter_mut() {
                if key.as_str().is_some_and(ai_key_is_sensitive) {
                    *nested = serde_yaml::Value::String("<redacted>".to_string());
                } else {
                    redact_ai_yaml_value(nested);
                }
            }
        }
        serde_yaml::Value::Sequence(items) => {
            for item in items {
                redact_ai_yaml_value(item);
            }
        }
        _ => {}
    }
}

fn sanitize_ai_annotation(key: &str, value: &str) -> String {
    if ai_key_is_sensitive(key) {
        "[redacted]".to_string()
    } else {
        truncate_ai_block(value, 120)
    }
}

fn sanitize_ai_yaml_excerpt(resource: &ResourceRef, yaml: &str) -> Option<String> {
    if resource.kind().eq_ignore_ascii_case("secret") {
        return Some("# redacted: Secret manifests are not sent to AI".to_string());
    }

    let mut value = serde_yaml::from_str::<serde_yaml::Value>(yaml).ok()?;
    redact_ai_yaml_value(&mut value);
    let rendered = serde_yaml::to_string(&value).ok()?;
    Some(truncate_ai_block(rendered.trim_end(), AI_YAML_MAX_CHARS))
}

fn build_ai_workflow_context(
    app: &kubectui::app::AppState,
    snapshot: &kubectui::state::ClusterSnapshot,
    resource: &ResourceRef,
    workflow: AiWorkflowKind,
    issue_lines: &[String],
) -> (Option<String>, Vec<String>) {
    let lines = match workflow {
        AiWorkflowKind::ResourceAnalysis => Vec::new(),
        AiWorkflowKind::ExplainFailure => {
            let mut lines = Vec::new();
            if issue_lines.is_empty() {
                lines.push(
                    "No precomputed issues were attached to this resource; rely on events, probes, and logs."
                        .to_string(),
                );
            } else {
                lines.push(format!(
                    "Prioritize the top {} issue signal(s) before secondary hypotheses.",
                    issue_lines.len().min(3)
                ));
            }
            lines
        }
        AiWorkflowKind::RolloutRisk => {
            let mut lines = Vec::new();
            for tab in &app.workbench().tabs {
                if let WorkbenchTabState::Rollout(rollout_tab) = &tab.state
                    && &rollout_tab.resource == resource
                {
                    lines.extend(rollout_tab.summary_lines.iter().cloned());
                    lines.extend(rollout_tab.conditions.iter().take(6).map(|condition| {
                        format!(
                            "condition {}={} ({})",
                            condition.type_,
                            condition.status,
                            truncate_ai_block(condition.message.as_deref().unwrap_or("-"), 120)
                        )
                    }));
                    break;
                }
            }
            if lines.is_empty() {
                lines.push(
                    "No rollout tab summary was open; infer risk from the selected workload state only."
                        .to_string(),
                );
            }
            lines
        }
        AiWorkflowKind::NetworkVerdict => {
            let mut lines = Vec::new();
            for tab in &app.workbench().tabs {
                match &tab.state {
                    WorkbenchTabState::Connectivity(connectivity_tab)
                        if &connectivity_tab.source == resource
                            || connectivity_tab.current_target.as_ref() == Some(resource) =>
                    {
                        lines.extend(connectivity_tab.summary_lines.iter().cloned());
                        break;
                    }
                    WorkbenchTabState::NetworkPolicy(network_tab)
                        if &network_tab.resource == resource =>
                    {
                        lines.extend(network_tab.summary_lines.iter().cloned());
                        break;
                    }
                    WorkbenchTabState::TrafficDebug(traffic_tab)
                        if &traffic_tab.resource == resource =>
                    {
                        lines.extend(traffic_tab.summary_lines.iter().cloned());
                        break;
                    }
                    _ => {}
                }
            }
            if lines.is_empty() {
                lines.push(
                    "No live connectivity or policy analysis tab was open; use current resource context conservatively."
                        .to_string(),
                );
            }
            lines
        }
        AiWorkflowKind::TriageFindings => {
            let mut lines = kubectui::state::issues::compute_issues(snapshot)
                .iter()
                .filter(|issue| &issue.resource_ref == resource)
                .take(8)
                .map(|issue| {
                    let severity = match issue.severity {
                        kubectui::k8s::dtos::AlertSeverity::Error => "error",
                        kubectui::k8s::dtos::AlertSeverity::Warning => "warning",
                        kubectui::k8s::dtos::AlertSeverity::Info => "info",
                    };
                    format!(
                        "{} [{}]: {}",
                        issue.category.label(),
                        severity,
                        truncate_ai_block(&issue.message, 140)
                    )
                })
                .collect::<Vec<_>>();
            lines.extend(
                kubectui::state::vulnerabilities::compute_vulnerability_findings(snapshot)
                    .iter()
                    .filter(|finding| finding.resource_ref.as_ref() == Some(resource))
                    .take(4)
                    .map(|finding| {
                        if finding.fixable_count > 0 {
                            format!(
                                "Vulnerabilities [{}]: {} total, {} fixable",
                                finding.resource_kind,
                                finding.counts.total(),
                                finding.fixable_count
                            )
                        } else {
                            format!(
                                "Vulnerabilities [{}]: {} total",
                                finding.resource_kind,
                                finding.counts.total()
                            )
                        }
                    }),
            );
            if lines.is_empty() {
                lines.push(
                    "No explicit findings were attached to this resource; prioritize any runtime symptoms that remain."
                        .to_string(),
                );
            }
            lines
        }
    };
    let title = match workflow {
        AiWorkflowKind::ResourceAnalysis => None,
        AiWorkflowKind::ExplainFailure => Some("Failure Focus".to_string()),
        AiWorkflowKind::RolloutRisk => Some("Rollout Context".to_string()),
        AiWorkflowKind::NetworkVerdict => Some("Network Context".to_string()),
        AiWorkflowKind::TriageFindings => Some("Triage Context".to_string()),
    };
    (title, cap_ai_lines(lines, 12, 1_600))
}

#[cold]
#[inline(never)]
fn build_ai_analysis_context(
    app: &kubectui::app::AppState,
    snapshot: &kubectui::state::ClusterSnapshot,
    resource: &ResourceRef,
    workflow: AiWorkflowKind,
) -> AiAnalysisContext {
    let detail = app
        .detail_view
        .as_ref()
        .filter(|detail| detail.resource.as_ref() == Some(resource));
    let metadata = detail
        .map(|detail| detail.metadata.clone())
        .unwrap_or_else(|| kubectui::detail_sections::metadata_for_resource(snapshot, resource));
    let metadata_lines = cap_ai_lines(
        {
            let mut lines = Vec::new();
            if let Some(status) = metadata.status.filter(|value| !value.is_empty()) {
                lines.push(format!("status: {status}"));
            }
            if let Some(node) = metadata.node.filter(|value| !value.is_empty()) {
                lines.push(format!("node: {node}"));
            }
            if let Some(ip) = metadata.ip.filter(|value| !value.is_empty()) {
                lines.push(format!("ip: {ip}"));
            }
            lines.extend(
                metadata
                    .labels
                    .into_iter()
                    .take(8)
                    .map(|(key, value)| format!("label {key}={value}")),
            );
            lines.extend(
                metadata
                    .annotations
                    .into_iter()
                    .take(6)
                    .map(|(key, value)| {
                        format!("annotation {key}={}", sanitize_ai_annotation(&key, &value))
                    }),
            );
            lines
        },
        AI_METADATA_MAX_LINES,
        AI_METADATA_MAX_CHARS,
    );
    let issue_lines = cap_ai_lines(
        kubectui::state::issues::compute_issues(snapshot)
            .iter()
            .filter(|issue| &issue.resource_ref == resource)
            .map(|issue| {
                truncate_ai_block(
                    &format!("{}: {}", issue.category.label(), issue.message),
                    180,
                )
            })
            .collect::<Vec<_>>(),
        AI_ISSUE_MAX_LINES,
        AI_ISSUE_MAX_CHARS,
    );
    let (workflow_title, workflow_lines) =
        build_ai_workflow_context(app, snapshot, resource, workflow, &issue_lines);
    let event_match = format!("{}/{}", resource.kind(), resource.name());
    let event_lines = cap_ai_lines(
        if let Some(detail) = detail {
            detail
                .events
                .iter()
                .map(|event| {
                    truncate_ai_block(
                        &format!("{} {}: {}", event.event_type, event.reason, event.message),
                        180,
                    )
                })
                .collect::<Vec<_>>()
        } else {
            snapshot
                .events
                .iter()
                .filter(|event| {
                    event.involved_object == event_match
                        && resource
                            .namespace()
                            .is_none_or(|namespace| namespace == event.namespace.as_str())
                })
                .map(|event| {
                    truncate_ai_block(
                        &format!("{} {}: {}", event.type_, event.reason, event.message),
                        180,
                    )
                })
                .collect::<Vec<_>>()
        },
        AI_EVENT_MAX_LINES,
        AI_EVENT_MAX_CHARS,
    );
    let probe_lines = cap_ai_lines(
        detail
            .and_then(|detail| detail.probe_panel.as_ref())
            .map(|panel| {
                panel
                    .container_probes
                    .iter()
                    .flat_map(|(name, probes)| {
                        let mut lines = Vec::new();
                        if let Some(config) = probes.liveness.as_ref() {
                            lines.push(truncate_ai_block(
                                &format!("{name} liveness: {}", config.format_display()),
                                180,
                            ));
                        }
                        if let Some(config) = probes.readiness.as_ref() {
                            lines.push(truncate_ai_block(
                                &format!("{name} readiness: {}", config.format_display()),
                                180,
                            ));
                        }
                        if let Some(config) = probes.startup.as_ref() {
                            lines.push(truncate_ai_block(
                                &format!("{name} startup: {}", config.format_display()),
                                180,
                            ));
                        }
                        lines
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        AI_PROBE_MAX_LINES,
        AI_PROBE_MAX_CHARS,
    );
    let mut log_lines = Vec::new();
    for tab in &app.workbench().tabs {
        match &tab.state {
            WorkbenchTabState::PodLogs(logs_tab) if &logs_tab.resource == resource => {
                let indices = logs_tab.viewer.filtered_indices();
                log_lines.extend(
                    indices
                        .into_iter()
                        .rev()
                        .take(AI_LOG_MAX_LINES)
                        .rev()
                        .filter_map(|idx| logs_tab.viewer.lines.get(idx))
                        .map(|entry| {
                            truncate_ai_block(
                                entry.display_text(logs_tab.viewer.structured_view),
                                180,
                            )
                        }),
                );
            }
            WorkbenchTabState::WorkloadLogs(logs_tab) if &logs_tab.resource == resource => {
                let filtered = logs_tab
                    .lines
                    .iter()
                    .filter(|line| logs_tab.matches_filter(line))
                    .collect::<Vec<_>>();
                log_lines.extend(filtered.into_iter().rev().take(AI_LOG_MAX_LINES).rev().map(
                    |line| {
                        truncate_ai_block(
                            &format!(
                                "{} {} {}",
                                line.pod_name,
                                line.container_name,
                                line.entry.display_text(logs_tab.structured_view)
                            ),
                            180,
                        )
                    },
                ));
            }
            _ => {}
        }
        if log_lines.len() >= AI_LOG_MAX_LINES {
            break;
        }
    }
    let log_lines = cap_ai_lines(log_lines, AI_LOG_MAX_LINES, AI_LOG_MAX_CHARS);

    AiAnalysisContext {
        resource: resource.clone(),
        cluster_context: app.current_context_name.clone(),
        metadata_lines,
        workflow_title,
        workflow_lines,
        issue_lines,
        event_lines,
        probe_lines,
        log_lines,
        yaml_excerpt: detail
            .and_then(|detail| detail.yaml.clone())
            .and_then(|yaml| sanitize_ai_yaml_excerpt(resource, &yaml)),
    }
}

/// Main asynchronous runtime entrypoint.
#[tokio::main]
async fn main() -> Result<()> {
    if startup::initialize_process()? {
        return Ok(());
    }

    let mut terminal = setup_terminal().context("failed to initialize terminal")?;
    let run_result = run_app(&mut terminal).await;
    let restore_result = restore_terminal(&mut terminal);

    if let Err(err) = ui::profiling::write_report_if_enabled() {
        eprintln!("failed to write profiling report: {err:#}");
    }

    if let Err(err) = restore_result {
        eprintln!("failed to restore terminal state: {err:#}");
    }

    run_result
}

/// Runs KubecTUI's event loop.
pub(crate) async fn run_app_inner(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<()> {
    let mut app = load_config();

    let mut client = pick_context_at_startup(terminal, &mut app).await?;
    app.current_context_name = client.cluster_context().map(str::to_string);

    let mut global_state = GlobalState::default();
    let mut startup_namespace_scope = namespace_scope(app.get_namespace()).map(str::to_string);
    // Validate persisted namespace against current cluster namespaces to avoid
    // starting in a stale namespace with empty resource lists.
    if app.get_namespace() != "all" {
        let selected_namespace = app.get_namespace().to_string();
        match fetch_namespaces_with_startup_retry(&client).await {
            Ok(namespaces) => {
                if !namespaces.iter().any(|ns| ns == &selected_namespace) {
                    app.set_namespace("all".to_string());
                    startup_namespace_scope = None;
                    app.needs_config_save = true;
                    app.set_error(format!(
                        "Namespace '{selected_namespace}' not found. Switched to 'all'."
                    ));
                }
                app.set_available_namespaces(namespaces);
            }
            // If we can't validate namespace quickly, load cluster-wide first to prevent blank startup.
            Err(err) => {
                app.set_namespace("all".to_string());
                startup_namespace_scope = None;
                app.set_error(format!(
                    "Namespace '{selected_namespace}' could not be validated quickly ({err}). Loaded 'all' namespaces."
                ));
            }
        }
    }

    let mut port_forwarder = PortForwarderService::new(std::sync::Arc::new(client.clone()));

    let (update_tx, mut update_rx) = tokio::sync::mpsc::channel::<UpdateMessage>(4096);
    let mut coordinator = UpdateCoordinator::new(client.clone(), update_tx.clone());

    // Channel for async detail view fetches — carries the requested resource to prevent stale writes.
    let (detail_tx, mut detail_rx) = tokio::sync::mpsc::channel::<DetailAsyncResult>(16);
    let mut detail_request_seq: u64 = 0;
    let (resource_diff_tx, mut resource_diff_rx) =
        tokio::sync::mpsc::channel::<ResourceDiffAsyncResult>(16);
    let mut resource_diff_request_seq: u64 = 0;
    let (rollout_inspection_tx, mut rollout_inspection_rx) =
        tokio::sync::mpsc::channel::<RolloutInspectionAsyncResult>(16);
    let mut rollout_inspection_request_seq: u64 = 0;
    let (helm_history_tx, mut helm_history_rx) =
        tokio::sync::mpsc::channel::<HelmHistoryAsyncResult>(16);
    let mut helm_history_request_seq: u64 = 0;
    let (helm_values_diff_tx, mut helm_values_diff_rx) =
        tokio::sync::mpsc::channel::<HelmValuesDiffAsyncResult>(16);
    let mut helm_values_diff_request_seq: u64 = 0;
    let (helm_rollback_tx, mut helm_rollback_rx) =
        tokio::sync::mpsc::channel::<HelmRollbackAsyncResult>(16);
    let (logs_viewer_tx, mut logs_viewer_rx) =
        tokio::sync::mpsc::channel::<LogsViewerAsyncResult>(64);
    let mut logs_viewer_request_seq: u64 = 0;
    let (delete_tx, mut delete_rx) = tokio::sync::mpsc::channel::<DeleteAsyncResult>(16);
    let mut delete_request_seq: u64 = 0;
    let mut delete_in_flight_id: Option<u64> = None;
    let (deferred_refresh_tx, mut deferred_refresh_rx) =
        tokio::sync::mpsc::channel::<DeferredRefreshTrigger>(32);
    let (scale_tx, mut scale_rx) = tokio::sync::mpsc::channel::<ScaleAsyncResult>(16);
    let (rollout_tx, mut rollout_rx) = tokio::sync::mpsc::channel::<RolloutMutationAsyncResult>(16);
    let (flux_reconcile_tx, mut flux_reconcile_rx) =
        tokio::sync::mpsc::channel::<FluxReconcileAsyncResult>(16);
    let (trigger_cronjob_tx, mut trigger_cronjob_rx) =
        tokio::sync::mpsc::channel::<TriggerCronJobAsyncResult>(16);
    let (cronjob_suspend_tx, mut cronjob_suspend_rx) =
        tokio::sync::mpsc::channel::<SetCronJobSuspendAsyncResult>(16);
    let (node_ops_tx, mut node_ops_rx) = tokio::sync::mpsc::channel::<NodeOpsAsyncResult>(16);
    let mut node_op_in_flight: bool = false;
    let (probe_tx, mut probe_rx) = tokio::sync::mpsc::channel::<ProbeAsyncResult>(16);
    let (relations_tx, mut relations_rx) = tokio::sync::mpsc::channel::<RelationsAsyncResult>(16);
    let mut relations_request_seq: u64 = 0;
    let (exec_bootstrap_tx, mut exec_bootstrap_rx) =
        tokio::sync::mpsc::channel::<ExecBootstrapResult>(16);
    let (debug_dialog_bootstrap_tx, mut debug_dialog_bootstrap_rx) =
        tokio::sync::mpsc::channel::<DebugContainerDialogBootstrapResult>(16);
    let mut debug_dialog_request_seq: u64 = 0;
    let (debug_launch_tx, mut debug_launch_rx) =
        tokio::sync::mpsc::channel::<DebugContainerLaunchAsyncResult>(16);
    let (node_debug_launch_tx, mut node_debug_launch_rx) =
        tokio::sync::mpsc::channel::<NodeDebugLaunchAsyncResult>(16);
    let (node_debug_cleanup_tx, mut node_debug_cleanup_rx) =
        tokio::sync::mpsc::channel::<NodeDebugCleanupAsyncResult>(16);
    let (exec_update_tx, mut exec_update_rx) = tokio::sync::mpsc::channel::<ExecEvent>(128);
    let mut next_exec_session_id: u64 = 1;
    let mut exec_sessions: HashMap<u64, ExecSessionHandle> = HashMap::new();
    let mut node_debug_sessions: HashMap<u64, NodeDebugSessionRuntime> = HashMap::new();
    let (workload_logs_bootstrap_tx, mut workload_logs_bootstrap_rx) =
        tokio::sync::mpsc::channel::<WorkloadLogsBootstrapResult>(16);
    let mut next_workload_logs_session_id: u64 = 1;
    let mut workload_log_sessions: HashMap<u64, Vec<(String, String, String)>> = HashMap::new();
    let (extension_fetch_tx, mut extension_fetch_rx) =
        tokio::sync::mpsc::channel::<ExtensionFetchResult>(16);
    let (extension_command_tx, mut extension_command_rx) =
        tokio::sync::mpsc::channel::<ExtensionCommandAsyncResult>(16);
    let (ai_analysis_tx, mut ai_analysis_rx) =
        tokio::sync::mpsc::channel::<AiAnalysisAsyncResult>(16);
    let mut next_extension_execution_id: u64 = 1;
    let mut next_ai_execution_id: u64 = 1;
    let (events_tx, mut events_rx) = tokio::sync::mpsc::channel::<EventsAsyncResult>(16);
    let mut events_state = EventsFetchRuntimeState::default();
    let mut pending_runbook_restore: Option<RunbookTabState> = None;

    let initial_extension_load = load_extensions_registry();
    let mut extension_registry = initial_extension_load.registry;
    notify_extension_load_warnings(&mut app, &initial_extension_load.warnings);
    let extension_config_path = initial_extension_load.path;
    let extension_watch_setup = create_config_watcher(&extension_config_path, "extensions");
    let (_extension_watcher, extension_watch_rx) = match extension_watch_setup {
        Ok((watcher, rx)) => (Some(watcher), Some(rx)),
        Err(err) => {
            app.push_toast(format!("Extensions watcher disabled: {err:#}"), true);
            (None, None)
        }
    };
    let initial_runbook_load = load_runbook_registry();
    let mut runbook_registry = initial_runbook_load.registry;
    notify_runbook_load_warnings(&mut app, &initial_runbook_load.warnings);
    let runbook_config_path = initial_runbook_load.path;
    let runbook_watch_setup = create_config_watcher(&runbook_config_path, "runbooks");
    let (_runbook_watcher, runbook_watch_rx) = match runbook_watch_setup {
        Ok((watcher, rx)) => (Some(watcher), Some(rx)),
        Err(err) => {
            app.push_toast(format!("Runbooks watcher disabled: {err:#}"), true);
            (None, None)
        }
    };

    // Channel for background data refreshes — namespace switches, manual refresh, auto-refresh
    // all go through here so the UI stays responsive during API calls.
    let (refresh_tx, mut refresh_rx) = tokio::sync::mpsc::channel::<RefreshAsyncResult>(16);
    // Background refresh scheduling state — one in-flight + one coalesced queued request.
    let mut refresh_state = RefreshRuntimeState::default();
    let mut snapshot_dirty = false;

    // Watch-backed resource caches — pushes live updates for core resources.
    let (watch_tx, mut watch_rx) = tokio::sync::mpsc::channel::<WatchUpdate>(32);
    let mut watch_manager =
        start_watch_manager(&client, refresh_state.context_generation, &app, &watch_tx).await;

    // Start with view-scoped refresh (workload-first for core views, secondary deferred).
    let startup_include_flux = app.view().is_fluxcd();
    request_refresh(
        &refresh_tx,
        &mut global_state,
        &client,
        startup_namespace_scope,
        refresh_options_for_view(app.view(), startup_include_flux, true),
        &mut refresh_state,
        &mut snapshot_dirty,
    );
    if app.view() == AppView::Events {
        request_events_refresh(
            &events_tx,
            &mut global_state,
            &client,
            namespace_scope(app.get_namespace()).map(str::to_string),
            refresh_state.context_generation,
            &mut events_state,
            &mut snapshot_dirty,
        );
    }

    // Cached snapshot — only re-clone when state is marked dirty
    let mut cached_snapshot = global_state.snapshot();
    snapshot_dirty = false;

    // Render-skip: only redraw when state actually changed
    let mut needs_redraw = true;

    let mut tick = tokio::time::interval(Duration::from_millis(50));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Auto-refresh: periodically re-fetch cluster data
    let refresh_secs = if app.refresh_interval_secs == 0 {
        86400
    } else {
        app.refresh_interval_secs
    };
    let mut auto_refresh = tokio::time::interval(Duration::from_secs(refresh_secs));
    auto_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Skip the first immediate tick — we already fetched at startup
    auto_refresh.reset();

    // Track consecutive refresh failures for backoff
    let mut consecutive_refresh_failures: u32 = 0;
    let mut backoff_until: Option<Instant> = None;
    let mut last_config_save: Option<Instant> = None;
    let mut auto_refresh_count: u64 = 0;
    let mut status_message_clear_at: Option<Instant> = None;
    let mut pending_palette_action: Option<AppAction> = None;
    let mut pending_flux_reconcile_verifications: Vec<PendingFluxReconcileVerification> =
        Vec::new();
    let mut pending_context_switch: Option<(String, tokio::task::JoinHandle<Result<K8sClient>>)> =
        None;

    let mut event_stream = EventStream::new();

    loop {
        if let Some(rx) = &extension_watch_rx {
            let mut reload_requested = false;
            loop {
                match rx.try_recv() {
                    Ok(Ok(event)) => {
                        if config_watch_matches_path(&event, &extension_config_path) {
                            reload_requested = true;
                        }
                    }
                    Ok(Err(err)) => {
                        app.push_toast(format!("Extensions watch error: {err}"), true);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            }
            if reload_requested {
                let reload = load_extensions_registry();
                extension_registry = reload.registry;
                notify_extension_load_warnings(&mut app, &reload.warnings);
                if app.command_palette.is_open() {
                    refresh_palette_extensions(&mut app, &cached_snapshot, &extension_registry);
                }
                app.push_toast(
                    format!(
                        "Reloaded extensions config ({} action{})",
                        extension_registry.actions().len(),
                        if extension_registry.actions().len() == 1 {
                            ""
                        } else {
                            "s"
                        }
                    ),
                    false,
                );
                needs_redraw = true;
            }
        }

        if let Some(rx) = &runbook_watch_rx {
            let mut reload_requested = false;
            loop {
                match rx.try_recv() {
                    Ok(Ok(event)) => {
                        if config_watch_matches_path(&event, &runbook_config_path) {
                            reload_requested = true;
                        }
                    }
                    Ok(Err(err)) => {
                        app.push_toast(format!("Runbooks watch error: {err}"), true);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            }
            if reload_requested {
                let reload = load_runbook_registry();
                runbook_registry = reload.registry;
                notify_runbook_load_warnings(&mut app, &reload.warnings);
                if app.command_palette.is_open() {
                    refresh_palette_runbooks(&mut app, &cached_snapshot, &runbook_registry);
                }
                app.push_toast(
                    format!(
                        "Reloaded runbooks config ({} runbook{})",
                        runbook_registry.runbooks().len(),
                        if runbook_registry.runbooks().len() == 1 {
                            ""
                        } else {
                            "s"
                        }
                    ),
                    false,
                );
                needs_redraw = true;
            }
        }

        // Re-clone snapshot only when something changed
        if snapshot_dirty {
            cached_snapshot = global_state.snapshot();
            snapshot_dirty = false;
            needs_redraw = true;
        }

        if needs_redraw {
            terminal
                .draw(|frame| ui::render(frame, &app, &cached_snapshot))
                .context("failed to render frame")?;
            needs_redraw = false;
        }

        if app.should_quit() {
            break;
        }

        // Check if a deferred palette action is ready to dispatch.
        let pending_action_ready = pending_palette_action.as_ref().is_some_and(|a| {
            let needs_loaded_detail = palette_action_requires_loaded_detail(a);
            !needs_loaded_detail
                || app
                    .detail_view
                    .as_ref()
                    .is_some_and(|d| !d.loading && d.error.is_none())
        });

        let mut action_to_process: Option<AppAction> = None;

        // Wait concurrently on: tick, input event, coordinator update, detail fetch, or auto-refresh.
        // `biased` ensures coordinator messages and detail results are drained before blocking on input.
        tokio::select! {
            biased;

            // Coordinator updates (log lines, probe updates) — highest priority
            msg = update_rx.recv() => {
                if let Some(msg) = msg {
                    apply_coordinator_msg(msg, &mut app);
                    needs_redraw = true;
                }
                // Drain any additional queued messages without blocking
                let mut drain_count = 0;
                while drain_count < 100 {
                    match update_rx.try_recv() {
                        Ok(msg) => {
                            apply_coordinator_msg(msg, &mut app);
                            drain_count += 1;
                        }
                        Err(_) => break,
                    }
                }
            }

            // Detail view fetch completed in background task
            result = detail_rx.recv() => {
                if let Some(result) = result {
                    let DetailAsyncResult {
                        request_id,
                        resource: requested_resource,
                        result,
                    } = result;
                    let detail_still_waiting_for_this = app
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| {
                            detail.resource.as_ref() == Some(&requested_resource)
                                && detail.pending_request_id == Some(request_id)
                        });
                    let workbench_waiting_for_this = app.workbench.tabs.iter().any(|tab| {
                        matches!(
                            &tab.state,
                            WorkbenchTabState::ResourceYaml(yaml_tab)
                                if yaml_tab.resource == requested_resource
                                    && yaml_tab.loading
                                    && yaml_tab.pending_request_id == Some(request_id)
                        ) || matches!(
                            &tab.state,
                            WorkbenchTabState::ResourceEvents(events_tab)
                                if events_tab.resource == requested_resource
                                    && events_tab.loading
                                    && events_tab.pending_request_id == Some(request_id)
                        )
                    });
                    if !detail_still_waiting_for_this && !workbench_waiting_for_this {
                        continue;
                    }
                    needs_redraw = true;
                    match result {
                        Ok(state) => {
                            apply_detail_state_to_workbench(&mut app, request_id, &state);
                            if detail_still_waiting_for_this {
                                app.detail_view = Some(state);
                            }
                        }
                        Err(err) => {
                            apply_detail_error_to_workbench(
                                &mut app,
                                request_id,
                                &requested_resource,
                                &err,
                            );
                            if detail_still_waiting_for_this {
                                app.detail_view = Some(DetailViewState {
                                    resource: Some(requested_resource),
                                    pending_request_id: None,
                                    loading: false,
                                    error: Some(err),
                                    ..DetailViewState::default()
                                });
                            }
                        }
                    }
                }
            }

            result = resource_diff_rx.recv() => {
                if let Some(result) = result {
                    let ResourceDiffAsyncResult {
                        request_id,
                        resource,
                        result,
                    } = result;
                    let workbench_waiting_for_this = app.workbench.tabs.iter().any(|tab| {
                        matches!(
                            &tab.state,
                            WorkbenchTabState::ResourceDiff(diff_tab)
                                if diff_tab.resource == resource
                                    && diff_tab.loading
                                    && diff_tab.pending_request_id == Some(request_id)
                        )
                    });
                    if !workbench_waiting_for_this {
                        continue;
                    }

                    needs_redraw = true;
                    match result {
                        Ok(live_yaml) => match kubectui::resource_diff::build_resource_diff(&live_yaml)
                        {
                            Ok(diff) => apply_resource_diff_result_to_workbench(
                                &mut app,
                                request_id,
                                &resource,
                                diff,
                            ),
                            Err(err) => apply_resource_diff_error_to_workbench(
                                &mut app,
                                request_id,
                                &resource,
                                &err.to_string(),
                            ),
                        },
                        Err(err) => apply_resource_diff_error_to_workbench(
                            &mut app,
                            request_id,
                            &resource,
                            &err,
                        ),
                    }
                }
            }

            result = helm_history_rx.recv() => {
                if let Some(result) = result {
                    let HelmHistoryAsyncResult {
                        request_id,
                        resource,
                        result,
                    } = result;
                    let workbench_waiting_for_this = app.workbench.tabs.iter().any(|tab| {
                        matches!(
                            &tab.state,
                            WorkbenchTabState::HelmHistory(history_tab)
                                if history_tab.resource == resource
                                    && history_tab.pending_history_request_id == Some(request_id)
                        )
                    });
                    if !workbench_waiting_for_this {
                        continue;
                    }

                    needs_redraw = true;
                    match result {
                        Ok(history) => apply_helm_history_result_to_workbench(
                            &mut app,
                            request_id,
                            &resource,
                            history,
                        ),
                        Err(err) => {
                            apply_helm_history_error_to_workbench(
                                &mut app,
                                request_id,
                                &resource,
                                &err,
                            );
                        }
                    }
                }
            }

            result = rollout_inspection_rx.recv() => {
                if let Some(result) = result {
                    let RolloutInspectionAsyncResult {
                        request_id,
                        resource,
                        result,
                    } = result;
                    let workbench_waiting_for_this = app.workbench.tabs.iter().any(|tab| {
                        matches!(
                            &tab.state,
                            WorkbenchTabState::Rollout(rollout_tab)
                                if rollout_tab.resource == resource
                                    && rollout_tab.pending_request_id == Some(request_id)
                        )
                    });
                    if !workbench_waiting_for_this {
                        continue;
                    }

                    needs_redraw = true;
                    match result {
                        Ok(inspection) => apply_rollout_inspection_result_to_workbench(
                            &mut app,
                            request_id,
                            &resource,
                            inspection,
                        ),
                        Err(err) => apply_rollout_inspection_error_to_workbench(
                            &mut app,
                            request_id,
                            &resource,
                            &err,
                        ),
                    }
                }
            }

            result = helm_values_diff_rx.recv() => {
                if let Some(result) = result {
                    let HelmValuesDiffAsyncResult {
                        request_id,
                        resource,
                        result,
                    } = result;
                    let workbench_waiting_for_this = app.workbench.tabs.iter().any(|tab| {
                        matches!(
                            &tab.state,
                            WorkbenchTabState::HelmHistory(history_tab)
                                if history_tab.resource == resource
                                    && history_tab
                                        .diff
                                        .as_ref()
                                        .is_some_and(|diff| diff.pending_request_id == Some(request_id))
                        )
                    });
                    if !workbench_waiting_for_this {
                        continue;
                    }

                    needs_redraw = true;
                    match result {
                        Ok(diff) => apply_helm_values_diff_result_to_workbench(
                            &mut app,
                            request_id,
                            &resource,
                            diff.diff,
                        ),
                        Err(err) => {
                            apply_helm_values_diff_error_to_workbench(
                                &mut app,
                                request_id,
                                &resource,
                                &err,
                            );
                        }
                    }
                }
            }

            // Logs viewer async responses (container discovery + tail snapshot)
            result = logs_viewer_rx.recv() => {
                if let Some(result) = result {
                    match result {
                        LogsViewerAsyncResult::Containers { request_id, pod_name, namespace, result } => {
                            let mut tail_request: Option<(u64, String, String, String)> = None;
                            for tab in &mut app.workbench.tabs {
                                if let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state {
                                    let viewer = &mut logs_tab.viewer;
                                    if viewer.pod_name == pod_name
                                        && viewer.pod_namespace == namespace
                                        && viewer.pending_container_request_id == Some(request_id)
                                    {
                                        needs_redraw = true;
                                        viewer.pending_container_request_id = None;
                                        match &result {
                                            Ok(containers) => {
                                                viewer.containers = containers.clone();
                                                viewer.container_cursor = 0;
                                                viewer.lines.clear();
                                                viewer.scroll_offset = 0;
                                                viewer.error = None;

                                                match containers.len() {
                                                    0 => {
                                                        viewer.container_name.clear();
                                                        viewer.picking_container = false;
                                                        viewer.pending_logs_request_id = None;
                                                        viewer.loading = false;
                                                        viewer.error = Some(
                                                            "No containers found for this pod.".to_string(),
                                                        );
                                                    }
                                                    1 => {
                                                        let container_name = containers[0].clone();
                                                        logs_viewer_request_seq =
                                                            logs_viewer_request_seq.wrapping_add(1);
                                                        let tail_request_id = logs_viewer_request_seq;
                                                        viewer.container_name = container_name.clone();
                                                        viewer.picking_container = false;
                                                        viewer.loading = true;
                                                        viewer.pending_logs_request_id = Some(tail_request_id);
                                                        tail_request = Some((
                                                            tail_request_id,
                                                            pod_name.clone(),
                                                            namespace.clone(),
                                                            container_name,
                                                        ));
                                                    }
                                                    _ => {
                                                        viewer.container_name.clear();
                                                        viewer.picking_container = true;
                                                        viewer.pending_logs_request_id = None;
                                                        viewer.loading = false;
                                                    }
                                                }
                                            }
                                            Err(err) => {
                                                viewer.picking_container = false;
                                                viewer.pending_logs_request_id = None;
                                                viewer.loading = false;
                                                viewer.error =
                                                    Some(format!("Failed to load containers: {err}"));
                                            }
                                        }
                                    }
                                }
                            }

                            if let Some((tail_request_id, pod_name, pod_ns, container_name)) =
                                tail_request
                            {
                                let client_clone = client.clone();
                                let tx = logs_viewer_tx.clone();
                                tokio::spawn(async move {
                                    let logs_client = LogsClient::new(client_clone.get_client());
                                    let pod_ref = PodRef::new(pod_name.clone(), pod_ns.clone());
                                    let result = logs_client
                                        .tail_logs(&pod_ref, Some(500), Some(container_name.as_str()))
                                        .await
                                        .map_err(|err| err.to_string());
                                    let _ = tx.send(LogsViewerAsyncResult::Tail {
                                        request_id: tail_request_id,
                                        pod_name,
                                        namespace: pod_ns,
                                        container_name,
                                        result,
                                    }).await;
                                });
                            }
                        }
                        LogsViewerAsyncResult::Tail {
                            request_id,
                            pod_name,
                            namespace,
                            container_name,
                            result,
                        } => {
                            for tab in &mut app.workbench.tabs {
                                if let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state {
                                    let viewer = &mut logs_tab.viewer;
                                    if viewer.pod_name == pod_name
                                        && viewer.pod_namespace == namespace
                                        && viewer.container_name == container_name
                                        && viewer.pending_logs_request_id == Some(request_id)
                                    {
                                        needs_redraw = true;
                                        viewer.pending_logs_request_id = None;
                                        viewer.loading = false;
                                        match &result {
                                            Ok(lines) => {
                                                viewer.lines = lines
                                                    .iter()
                                                    .cloned()
                                                    .map(kubectui::log_investigation::LogEntry::from_raw)
                                                    .collect();
                                                viewer.error = None;
                                            }
                                            Err(err) => {
                                                viewer.error = Some(err.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Background refresh completed (namespace switch, manual refresh, auto-refresh)
            result = refresh_rx.recv() => {
                if let Some(result) = result {
                    // Ignore stale refreshes from previous context generations or superseded requests.
                    if result.context_generation != refresh_state.context_generation
                        || refresh_state.in_flight_id != Some(result.request_id)
                    {
                        continue;
                    }

                    refresh_state.in_flight_id = None;
                    refresh_state.in_flight_task = None;
                    needs_redraw = true;
                    let active_namespace_scope = namespace_scope(app.get_namespace()).map(str::to_string);
                    let namespace_matches = result.requested_namespace == active_namespace_scope;

                    if namespace_matches {
                        match result.result {
                            Ok(new_state) => {
                                global_state = new_state;
                                consecutive_refresh_failures = 0;
                                backoff_until = None;
                                let previously_selected_namespace = app.get_namespace().to_string();
                                let namespace_still_exists = previously_selected_namespace == "all"
                                    || global_state
                                        .namespaces()
                                        .iter()
                                        .any(|ns| ns == &previously_selected_namespace);
                                if !namespace_still_exists {
                                    app.set_namespace("all".to_string());
                                    app.needs_config_save = true;
                                    app.set_error(format!(
                                        "Namespace '{previously_selected_namespace}' not found. Switched to 'all'."
                                    ));
                                    request_refresh(
                                        &refresh_tx,
                                        &mut global_state,
                                        &client,
                                        None,
                                        refresh_options_for_view(
                                            app.view(),
                                            app.view().is_fluxcd(),
                                            false,
                                        ),
                                        &mut refresh_state,
                                        &mut snapshot_dirty,
                                    );
                                    if app.view() == AppView::Events {
                                        request_events_refresh(
                                            &events_tx,
                                            &mut global_state,
                                            &client,
                                            None,
                                            refresh_state.context_generation,
                                            &mut events_state,
                                            &mut snapshot_dirty,
                                        );
                                    }
                                } else {
                                    app.clear_error();
                                }
                                app.set_available_namespaces(global_state.namespaces().to_vec());
                                if process_flux_reconcile_verifications(
                                    &mut app,
                                    &global_state.snapshot(),
                                    &mut pending_flux_reconcile_verifications,
                                    &mut |a, msg| set_transient_status(a, &mut status_message_clear_at, msg),
                                ) {
                                    needs_redraw = true;
                                }
                                snapshot_dirty = true;
                                spawn_extensions_fetch(&client, &mut app, &global_state.snapshot(), &extension_fetch_tx);
                            }
                            Err(err) => {
                                consecutive_refresh_failures += 1;
                                let delay = match consecutive_refresh_failures {
                                    1 => 5,
                                    2 => 15,
                                    3 => 30,
                                    4 => 60,
                                    _ => 120,
                                };
                                backoff_until = Some(Instant::now() + Duration::from_secs(delay));
                                app.set_error(format!("Refresh failed: {err}"));
                            }
                        }
                    } else {
                        // Namespace changed while this refresh was running. Skip applying stale data.
                    }

                    if let Some(queued) = refresh_state.queued_refresh.take()
                        && queued.context_generation == refresh_state.context_generation
                    {
                        if queued_refresh_requires_two_phase(queued.primary_scope, queued.options) {
                            request_refresh(
                                &refresh_tx,
                                &mut global_state,
                                &client,
                                queued.namespace,
                                RefreshDispatch {
                                    primary_scope: queued.primary_scope,
                                    options: queued.options,
                                },
                                &mut refresh_state,
                                &mut snapshot_dirty,
                            );
                        } else {
                            refresh_state.in_flight_id = Some(queued.request_id);
                            refresh_state.in_flight_task = Some(spawn_refresh_task(
                                refresh_tx.clone(),
                                global_state.clone(),
                                client.clone(),
                                queued.namespace,
                                queued.options,
                                queued.request_id,
                                queued.context_generation,
                            ));
                        }
                    }
                }
            }

            result = events_rx.recv() => {
                if let Some(result) = result {
                    if result.context_generation != refresh_state.context_generation
                        || events_state.in_flight_id != Some(result.request_id)
                    {
                        continue;
                    }

                    events_state.in_flight_id = None;
                    events_state.in_flight_namespace = None;
                    events_state.in_flight_task = None;
                    needs_redraw = true;

                    if result.requested_namespace
                        == namespace_scope(app.get_namespace()).map(str::to_string)
                    {
                        match result.result {
                            Ok(events) => {
                                global_state.apply_events_update(events);
                                snapshot_dirty = true;
                            }
                            Err(err) => {
                                global_state.fail_events_refresh(err.clone());
                                snapshot_dirty = true;
                                app.set_error(format!("Events refresh failed: {err}"));
                            }
                        }
                    }

                    if let Some(namespace) = events_state.queued_namespace.take() {
                        events_state.request_seq = events_state.request_seq.wrapping_add(1);
                        let request_id = events_state.request_seq;
                        events_state.in_flight_id = Some(request_id);
                        events_state.in_flight_namespace = Some(namespace.clone());
                        events_state.in_flight_task = Some(spawn_events_fetch_task(
                            events_tx.clone(),
                            client.clone(),
                            namespace,
                            request_id,
                            refresh_state.context_generation,
                        ));
                    }
                }
            }

            // Watch-backed resource updates — live cluster changes via watch streams.
            Some(update) = watch_rx.recv() => {
                if update.context_generation == refresh_state.context_generation {
                    let watched_resource = update.resource;
                    global_state.apply_watch_update(update);
                    snapshot_dirty = true;
                    if watched_resource == WatchedResource::Namespaces {
                        needs_redraw = true;
                        app.set_available_namespaces(global_state.namespaces().to_vec());
                        let previously_selected_namespace = app.get_namespace().to_string();
                        let namespace_still_exists = previously_selected_namespace == "all"
                            || global_state
                                .namespaces()
                                .iter()
                                .any(|ns| ns == &previously_selected_namespace);
                        if !namespace_still_exists {
                            app.set_namespace("all".to_string());
                            app.needs_config_save = true;
                            app.set_error(format!(
                                "Namespace '{previously_selected_namespace}' not found. Switched to 'all'."
                            ));
                            request_refresh(
                                &refresh_tx,
                                &mut global_state,
                                &client,
                                None,
                                refresh_options_for_view(
                                    app.view(),
                                    app.view().is_fluxcd(),
                                    false,
                                ),
                                &mut refresh_state,
                                &mut snapshot_dirty,
                            );
                            if app.view() == AppView::Events {
                                request_events_refresh(
                                    &events_tx,
                                    &mut global_state,
                                    &client,
                                    None,
                                    refresh_state.context_generation,
                                    &mut events_state,
                                    &mut snapshot_dirty,
                                );
                            }
                        } else {
                            app.clear_error();
                        }
                    }
                }
            }

            // Background delete completion — deletion itself runs off the UI loop.
            result = delete_rx.recv() => {
                if let Some(result) = result {
                    // Ignore stale results (e.g. context changed while request was in flight).
                    if result.context_generation != refresh_state.context_generation
                        || delete_in_flight_id != Some(result.request_id)
                    {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            "Delete verification was cancelled because the active context changed.",
                            true,
                        );
                        continue;
                    }

                    delete_in_flight_id = None;
                    needs_redraw = true;

                    match result.result {
                        Ok(()) => {
                            global_state.apply_optimistic_delete(&result.resource);
                            snapshot_dirty = true;
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Succeeded,
                                format!(
                                    "Deleted {} '{}'.",
                                    result.resource.kind(),
                                    result.resource.name()
                                ),
                                false,
                            );
                            apply_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                result.origin_view,
                                format!(
                                    "Deleted {} '{}'. Refreshing view...",
                                    result.resource.kind(),
                                    result.resource.name()
                                ),
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!("Delete failed: {err}"),
                                true,
                            );
                            status_message_clear_at = None;
                            app.set_error(format!("Delete failed: {err}"));
                        }
                    }
                }
            }

            result = scale_rx.recv() => {
                if let Some(result) = result {
                    if result.context_generation != refresh_state.context_generation {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            "Scale verification was cancelled because the active context changed.",
                            true,
                        );
                        continue;
                    }
                    needs_redraw = true;
                    match result.result {
                        Ok(()) => {
                            global_state
                                .apply_optimistic_scale(&result.resource, result.target_replicas);
                            snapshot_dirty = true;
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Succeeded,
                                format!("Scaled {}.", result.resource_label),
                                true,
                            );
                            apply_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                result.origin_view,
                                format!("Scaled {}. Refreshing view...", result.resource_label),
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!("Scale failed: {err}"),
                                true,
                            );
                            status_message_clear_at = None;
                            app.set_error(format!("Scale failed: {err}"));
                        }
                    }
                }
            }

            result = rollout_rx.recv() => {
                if let Some(result) = result {
                    if result.context_generation != refresh_state.context_generation {
                        if let Some(tab) = app
                            .workbench_mut()
                            .find_tab_mut(&WorkbenchTabKey::Rollout(result.resource.clone()))
                            && let WorkbenchTabState::Rollout(rollout_tab) = &mut tab.state
                        {
                            rollout_tab.mutation_pending = None;
                        }
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            "Rollout mutation verification was cancelled because the active context changed.",
                            true,
                        );
                        continue;
                    }
                    needs_redraw = true;
                    match result.result {
                        Ok(()) => {
                            let (success_message, refresh_message) = match result.kind {
                                RolloutMutationKind::Restart => (
                                    format!("Restart requested for {}.", result.resource_label),
                                    format!(
                                        "Restart requested for {}. Refreshing view...",
                                        result.resource_label
                                    ),
                                ),
                                RolloutMutationKind::Pause => (
                                    format!("Paused rollout for {}.", result.resource_label),
                                    format!(
                                        "Paused rollout for {}. Refreshing view...",
                                        result.resource_label
                                    ),
                                ),
                                RolloutMutationKind::Resume => (
                                    format!("Resumed rollout for {}.", result.resource_label),
                                    format!(
                                        "Resumed rollout for {}. Refreshing view...",
                                        result.resource_label
                                    ),
                                ),
                                RolloutMutationKind::Undo(revision) => (
                                    format!(
                                        "Rolled back {} to revision {}.",
                                        result.resource_label, revision
                                    ),
                                    format!(
                                        "Rolled back {} to revision {}. Refreshing view...",
                                        result.resource_label, revision
                                    ),
                                ),
                            };
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Succeeded,
                                success_message,
                                true,
                            );
                            action::rollout::refresh_rollout_tab(
                                &mut app,
                                &client,
                                &rollout_inspection_tx,
                                &mut rollout_inspection_request_seq,
                                result.resource.clone(),
                            );
                            apply_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                result.origin_view,
                                refresh_message,
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            if let Some(tab) = app
                                .workbench_mut()
                                .find_tab_mut(&WorkbenchTabKey::Rollout(result.resource.clone()))
                                && let WorkbenchTabState::Rollout(rollout_tab) = &mut tab.state
                            {
                                rollout_tab.mutation_pending = None;
                            }
                            let failure_message = match result.kind {
                                RolloutMutationKind::Restart => format!("Restart failed: {err}"),
                                RolloutMutationKind::Pause => format!("Pause failed: {err}"),
                                RolloutMutationKind::Resume => format!("Resume failed: {err}"),
                                RolloutMutationKind::Undo(_) => {
                                    format!("Rollout undo failed: {err}")
                                }
                            };
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                failure_message.clone(),
                                true,
                            );
                            status_message_clear_at = None;
                            app.set_error(failure_message);
                        }
                    }
                }
            }

            result = flux_reconcile_rx.recv() => {
                if let Some(result) = result {
                    if result.context_generation != refresh_state.context_generation {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            "Reconcile verification was cancelled because the active context changed.",
                            true,
                        );
                        continue;
                    }

                    needs_redraw = true;
                    match result.result {
                        Ok(()) => {
                            pending_flux_reconcile_verifications
                                .retain(|pending| pending.resource != result.resource);
                            pending_flux_reconcile_verifications
                                .push(PendingFluxReconcileVerification {
                                    action_history_id: result.action_history_id,
                                    resource: result.resource,
                                    resource_label: result.resource_label.clone(),
                                    baseline: result.baseline,
                                    deadline: Instant::now()
                                        + Duration::from_secs(
                                            FLUX_RECONCILE_REFRESH_DELAYS_SECS
                                                .last()
                                                .copied()
                                                .unwrap_or_default()
                                                + 3,
                                        ),
                                });
                            apply_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                result.origin_view,
                                format!(
                                    "Reconcile requested for {}. Refreshing Flux status...",
                                    result.resource_label
                                ),
                                true,
                                FLUX_RECONCILE_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!("Flux reconcile failed: {err}"),
                                true,
                            );
                            status_message_clear_at = None;
                            app.set_error(format!("Flux reconcile failed: {err}"));
                        }
                    }
                }
            }

            result = trigger_cronjob_rx.recv() => {
                if let Some(result) = result {
                    if result.context_generation != refresh_state.context_generation {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            "Trigger cancelled because the active context changed.",
                            true,
                        );
                        continue;
                    }

                    needs_redraw = true;
                    match result.result {
                        Ok(job_name) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Succeeded,
                                format!("Created Job '{job_name}' from {}.", result.resource_label),
                                true,
                            );
                            apply_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                result.origin_view,
                                format!("Created Job '{job_name}'."),
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!("Trigger failed: {err}"),
                                true,
                            );
                            status_message_clear_at = None;
                            app.set_error(format!("Trigger failed: {err}"));
                        }
                    }
                }
            }

            result = cronjob_suspend_rx.recv() => {
                if let Some(result) = result {
                    if result.context_generation != refresh_state.context_generation {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            format!(
                                "{} cancelled because the active context changed.",
                                if result.suspend { "Suspend" } else { "Resume" }
                            ),
                            true,
                        );
                        continue;
                    }

                    needs_redraw = true;
                    match result.result {
                        Ok(()) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Succeeded,
                                format!(
                                    "{} requested for {}.",
                                    if result.suspend { "Suspend" } else { "Resume" },
                                    result.resource_label
                                ),
                                true,
                            );
                            apply_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                result.origin_view,
                                format!(
                                    "{} requested for {}. Refreshing view...",
                                    if result.suspend { "Suspend" } else { "Resume" },
                                    result.resource_label
                                ),
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!(
                                    "{} failed: {err}",
                                    if result.suspend { "Suspend" } else { "Resume" }
                                ),
                                true,
                            );
                            status_message_clear_at = None;
                            app.set_error(format!(
                                "{} failed: {err}",
                                if result.suspend { "Suspend" } else { "Resume" }
                            ));
                        }
                    }
                }
            }

            result = helm_rollback_rx.recv() => {
                if let Some(result) = result {
                    if result.context_generation != refresh_state.context_generation {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            "Helm rollback verification was cancelled because the active context changed.",
                            true,
                        );
                        continue;
                    }

                    needs_redraw = true;
                    match result.result {
                        Ok(stdout) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Succeeded,
                                format!(
                                    "Rolled back {} to revision {}.",
                                    result.resource.name(),
                                    result.target_revision
                                ),
                                true,
                            );
                            if let Some(tab) = app
                                .workbench_mut()
                                .find_tab_mut(&WorkbenchTabKey::HelmHistory(result.resource.clone()))
                                && let WorkbenchTabState::HelmHistory(history_tab) = &mut tab.state
                            {
                                history_tab.rollback_pending = false;
                            }
                            action::helm::refresh_helm_history_tab(
                                &mut app,
                                &helm_history_tx,
                                &mut helm_history_request_seq,
                                result.resource.clone(),
                            );
                            apply_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                result.origin_view,
                                if stdout.is_empty() {
                                    format!(
                                        "Rolled back Helm release '{}' to revision {}. Refreshing view...",
                                        result.resource.name(),
                                        result.target_revision
                                    )
                                } else {
                                    format!(
                                        "Rolled back Helm release '{}' to revision {}. {}",
                                        result.resource.name(),
                                        result.target_revision,
                                        stdout
                                    )
                                },
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            if let Some(tab) = app
                                .workbench_mut()
                                .find_tab_mut(&WorkbenchTabKey::HelmHistory(result.resource.clone()))
                                && let WorkbenchTabState::HelmHistory(history_tab) = &mut tab.state
                            {
                                history_tab.rollback_pending = false;
                            }
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!("Helm rollback failed: {err}"),
                                true,
                            );
                            status_message_clear_at = None;
                            app.set_error(format!("Helm rollback failed: {err}"));
                        }
                    }
                }
            }

            result = node_ops_rx.recv() => {
                if let Some(result) = result {
                    if result.context_generation != refresh_state.context_generation {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            format!("{} cancelled because the active context changed.", result.op_kind.label()),
                            true,
                        );
                        continue;
                    }

                    needs_redraw = true;
                    node_op_in_flight = false;
                    match result.result {
                        Ok(()) => {
                            // Optimistic update: flip unschedulable in cache.
                            // Drain cordons the node, so it also sets unschedulable = true.
                            let new_val = !matches!(result.op_kind, NodeOpKind::Uncordon);
                            global_state.apply_optimistic_node_schedulable(&result.node_name, new_val);
                            snapshot_dirty = true;
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Succeeded,
                                format!("{} succeeded for Node '{}'.", result.op_kind.label(), result.node_name),
                                true,
                            );
                            apply_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                result.origin_view,
                                format!("{} succeeded for Node '{}'. Refreshing view...", result.op_kind.label(), result.node_name),
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!("{} failed: {err}", result.op_kind.label()),
                                true,
                            );
                            status_message_clear_at = None;
                            app.set_error(format!("{} failed: {err}", result.op_kind.label()));
                        }
                    }
                }
            }

            result = exec_bootstrap_rx.recv() => {
                if let Some(result) = result {
                    let mut start_session: Option<(u64, String, String, String)> = None;
                    if let Some(tab) = app
                        .workbench_mut()
                        .find_tab_mut(&WorkbenchTabKey::Exec(result.resource.clone()))
                        && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
                        && exec_tab.session_id == result.session_id
                    {
                        match result.result {
                            Ok(containers) => {
                                exec_tab.set_containers(containers.clone());
                                exec_tab.loading = exec_tab.picking_container;
                                exec_tab.error = None;
                                if containers.len() == 1 {
                                    start_session = Some((
                                        result.session_id,
                                        exec_tab.pod_name.clone(),
                                        exec_tab.namespace.clone(),
                                        containers[0].clone(),
                                    ));
                                }
                            }
                            Err(err) => {
                                exec_tab.loading = false;
                                exec_tab.error = Some(err);
                            }
                        }
                    }

                    if let Some((session_id, pod_name, namespace, container_name)) = start_session {
                        match spawn_exec_session(
                            client.clone(),
                            session_id,
                            pod_name,
                            namespace,
                            container_name.clone(),
                            exec_update_tx.clone(),
                        )
                        .await
                        {
                            Ok(handle) => {
                                exec_sessions.insert(session_id, handle);
                                if let Some(tab) = app
                                    .workbench_mut()
                                    .find_tab_mut(&WorkbenchTabKey::Exec(result.resource.clone()))
                                    && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
                                {
                                    exec_tab.container_name = container_name;
                                    exec_tab.loading = true;
                                }
                            }
                            Err(err) => {
                                if let Some(tab) = app
                                    .workbench_mut()
                                    .find_tab_mut(&WorkbenchTabKey::Exec(result.resource.clone()))
                                    && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
                                {
                                    exec_tab.loading = false;
                                    exec_tab.error = Some(format!("{err:#}"));
                                }
                            }
                        }
                    }
                }
            }

            result = debug_dialog_bootstrap_rx.recv() => {
                if let Some(result) = result {
                    needs_redraw = true;
                    if let Some(detail) = app.detail_view.as_mut()
                        && detail.resource.as_ref() == Some(&result.resource)
                        && let Some(dialog) = detail.debug_dialog.as_mut()
                        && dialog.pending_request_id == Some(result.request_id)
                    {
                        match result.result {
                            Ok(containers) => dialog.set_target_containers(containers),
                            Err(err) => dialog.set_target_fetch_error(err),
                        }
                    }
                }
            }

            result = debug_launch_rx.recv() => {
                if let Some(result) = result {
                    needs_redraw = true;
                    if result.context_generation != refresh_state.context_generation {
                        if let Some(detail) = app.detail_view.as_mut()
                            && detail.resource.as_ref() == Some(&result.resource)
                            && let Some(dialog) = detail.debug_dialog.as_mut()
                        {
                            dialog.set_pending_launch(false);
                            dialog.error_message = Some(
                                "Debug container launch was cancelled because the active context changed."
                                    .to_string(),
                            );
                        }
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            "Debug container launch was cancelled because the active context changed.",
                            true,
                        );
                        continue;
                    }

                    match result.result {
                        Ok(launch) => {
                            if let Some(existing_session_id) =
                                app.workbench().exec_session_id(&result.resource)
                                && let Some(handle) = exec_sessions.remove(&existing_session_id)
                            {
                                let _ = handle.cancel_tx.send(());
                                cleanup_node_debug_session_if_needed(
                                    existing_session_id,
                                    &mut node_debug_sessions,
                                    &node_debug_cleanup_tx,
                                )
                                .await;
                            }
                            match spawn_exec_session(
                                client.clone(),
                                result.session_id,
                                launch.pod_name.clone(),
                                launch.namespace.clone(),
                                launch.container_name.clone(),
                                exec_update_tx.clone(),
                            )
                            .await
                            {
                                Ok(handle) => {
                                    exec_sessions.insert(result.session_id, handle);
                                    app.complete_action_history(
                                        result.action_history_id,
                                        ActionStatus::Succeeded,
                                        format!(
                                            "Started debug container '{}' in Pod '{}'.",
                                            launch.container_name, launch.pod_name
                                        ),
                                        true,
                                    );
                                    apply_mutation_success(
                                        &mut app,
                                        &mut MutationRuntime {
                                            global_state: &mut global_state,
                                            client: &client,
                                            refresh_tx: &refresh_tx,
                                            deferred_refresh_tx: &deferred_refresh_tx,
                                            refresh_state: &mut refresh_state,
                                            snapshot_dirty: &mut snapshot_dirty,
                                            auto_refresh: &mut auto_refresh,
                                            status_message_clear_at: &mut status_message_clear_at,
                                        },
                                        result.origin_view,
                                        format!(
                                            "Started debug container '{}' in Pod '{}'. Refreshing view...",
                                            launch.container_name, launch.pod_name
                                        ),
                                        false,
                                        MUTATION_REFRESH_DELAYS_SECS,
                                    );
                                    app.detail_view = None;
                                    app.open_exec_tab_for_container(
                                        result.resource.clone(),
                                        result.session_id,
                                        launch.pod_name,
                                        launch.namespace,
                                        launch.container_name,
                                    );
                                }
                                Err(err) => {
                                    let error_message = format!(
                                        "Debug container launched, but shell attach failed: {err:#}"
                                    );
                                    app.complete_action_history(
                                        result.action_history_id,
                                        ActionStatus::Failed,
                                        error_message.clone(),
                                        true,
                                    );
                                    apply_mutation_success(
                                        &mut app,
                                        &mut MutationRuntime {
                                            global_state: &mut global_state,
                                            client: &client,
                                            refresh_tx: &refresh_tx,
                                            deferred_refresh_tx: &deferred_refresh_tx,
                                            refresh_state: &mut refresh_state,
                                            snapshot_dirty: &mut snapshot_dirty,
                                            auto_refresh: &mut auto_refresh,
                                            status_message_clear_at: &mut status_message_clear_at,
                                        },
                                        result.origin_view,
                                        format!(
                                            "Debug container '{}' launched, but shell attach failed. Refreshing view...",
                                            launch.container_name
                                        ),
                                        false,
                                        MUTATION_REFRESH_DELAYS_SECS,
                                    );
                                    if let Some(detail) = app.detail_view.as_mut()
                                        && detail.resource.as_ref() == Some(&result.resource)
                                        && let Some(dialog) = detail.debug_dialog.as_mut()
                                    {
                                        dialog.set_pending_launch(false);
                                        dialog.error_message = Some(error_message.clone());
                                    }
                                    status_message_clear_at = None;
                                    app.set_error(error_message);
                                }
                            }
                        }
                        Err(err) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!("Debug container launch failed: {err}"),
                                true,
                            );
                            if let Some(detail) = app.detail_view.as_mut()
                                && detail.resource.as_ref() == Some(&result.resource)
                                && let Some(dialog) = detail.debug_dialog.as_mut()
                            {
                                dialog.set_pending_launch(false);
                                dialog.error_message = Some(err);
                            } else {
                                status_message_clear_at = None;
                                app.set_error(format!("Debug container launch failed: {err}"));
                            }
                        }
                    }
                }
            }

            result = node_debug_launch_rx.recv() => {
                if let Some(result) = result {
                    needs_redraw = true;
                    match result.result {
                        Ok(launch) => {
                            if result.context_generation != refresh_state.context_generation {
                                if let Some(detail) = app.detail_view.as_mut()
                                    && detail.resource.as_ref() == Some(&result.resource)
                                    && let Some(dialog) = detail.node_debug_dialog.as_mut()
                                {
                                    dialog.set_pending_launch(false);
                                    dialog.error_message = Some(
                                        "Node debug shell launch was cancelled because the active context changed."
                                            .to_string(),
                                    );
                                }
                                app.complete_action_history(
                                    result.action_history_id,
                                    ActionStatus::Failed,
                                    "Node debug shell launch was cancelled because the active context changed.",
                                    true,
                                );
                                spawn_node_debug_cleanup(
                                    NodeDebugSessionRuntime {
                                        client: result.cleanup_client,
                                        node_name: launch.node_name,
                                        pod_name: launch.pod_name,
                                        namespace: launch.namespace,
                                    },
                                    node_debug_cleanup_tx.clone(),
                                )
                                .await;
                                continue;
                            }
                            if let Some(existing_session_id) =
                                app.workbench().exec_session_id(&result.resource)
                            {
                                if let Some(handle) = exec_sessions.remove(&existing_session_id) {
                                    let _ = handle.cancel_tx.send(());
                                }
                                cleanup_node_debug_session_if_needed(
                                    existing_session_id,
                                    &mut node_debug_sessions,
                                    &node_debug_cleanup_tx,
                                )
                                .await;
                            }
                            match spawn_exec_session(
                                result.cleanup_client.clone(),
                                result.session_id,
                                launch.pod_name.clone(),
                                launch.namespace.clone(),
                                launch.container_name.clone(),
                                exec_update_tx.clone(),
                            )
                            .await
                            {
                                Ok(handle) => {
                                    exec_sessions.insert(result.session_id, handle);
                                    node_debug_sessions.insert(
                                        result.session_id,
                                        NodeDebugSessionRuntime {
                                            client: result.cleanup_client,
                                            node_name: launch.node_name.clone(),
                                            pod_name: launch.pod_name.clone(),
                                            namespace: launch.namespace.clone(),
                                        },
                                    );
                                    app.complete_action_history(
                                        result.action_history_id,
                                        ActionStatus::Succeeded,
                                        format!(
                                            "Started {} node debug shell for Node '{}'.",
                                            launch.profile.label(),
                                            launch.node_name
                                        ),
                                        true,
                                    );
                                    app.open_exec_tab_for_container(
                                        result.resource.clone(),
                                        result.session_id,
                                        launch.pod_name.clone(),
                                        launch.namespace.clone(),
                                        launch.container_name.clone(),
                                    );
                                    app.append_exec_banner(
                                        &result.resource,
                                        result.session_id,
                                        &action::node_debug::node_debug_shell_banner(&launch),
                                    );
                                    app.detail_view = None;
                                }
                                Err(err) => {
                                    spawn_node_debug_cleanup(
                                        NodeDebugSessionRuntime {
                                            client: result.cleanup_client,
                                            node_name: launch.node_name.clone(),
                                            pod_name: launch.pod_name.clone(),
                                            namespace: launch.namespace.clone(),
                                        },
                                        node_debug_cleanup_tx.clone(),
                                    )
                                    .await;
                                    let error_message = format!(
                                        "Node debug pod launched, but shell attach failed: {err:#}"
                                    );
                                    app.complete_action_history(
                                        result.action_history_id,
                                        ActionStatus::Failed,
                                        format!("{error_message}. Cleanup requested."),
                                        true,
                                    );
                                    if let Some(detail) = app.detail_view.as_mut()
                                        && detail.resource.as_ref() == Some(&result.resource)
                                        && let Some(dialog) = detail.node_debug_dialog.as_mut()
                                    {
                                        dialog.set_pending_launch(false);
                                        dialog.error_message =
                                            Some(format!("{error_message}. Cleanup requested."));
                                    }
                                    app.set_error(format!("{error_message}. Cleanup requested."));
                                }
                            }
                        }
                        Err(err) => {
                            app.complete_action_history(
                                result.action_history_id,
                                ActionStatus::Failed,
                                format!("Node debug shell launch failed: {err}"),
                                true,
                            );
                            if let Some(detail) = app.detail_view.as_mut()
                                && detail.resource.as_ref() == Some(&result.resource)
                                && let Some(dialog) = detail.node_debug_dialog.as_mut()
                            {
                                dialog.set_pending_launch(false);
                                dialog.error_message = Some(err);
                            } else {
                                app.set_error(format!("Node debug shell launch failed: {err}"));
                            }
                        }
                    }
                }
            }

            result = exec_update_rx.recv() => {
                needs_redraw = true;
                if let Some(result) = result {
                    for tab in &mut app.workbench.tabs {
                        if let WorkbenchTabState::Exec(exec_tab) = &mut tab.state {
                            if exec_tab.session_id != match &result {
                                ExecEvent::Opened { session_id, .. }
                                | ExecEvent::Output { session_id, .. }
                                | ExecEvent::Exited { session_id, .. }
                                | ExecEvent::Error { session_id, .. } => *session_id,
                            } {
                                continue;
                            }

                            match result {
                                ExecEvent::Opened { shell, .. } => {
                                    exec_tab.shell_name = Some(shell);
                                    exec_tab.loading = false;
                                    exec_tab.exited = false;
                                    exec_tab.error = None;
                                }
                                ExecEvent::Output { chunk, is_stderr, .. } => {
                                    if is_stderr {
                                        exec_tab.append_output(&format!("[stderr] {chunk}"));
                                    } else {
                                        exec_tab.append_output(&chunk);
                                    }
                                }
                                ExecEvent::Exited { success, message, session_id } => {
                                    exec_tab.loading = false;
                                    exec_tab.exited = true;
                                    exec_tab.error = (!success).then_some(message.clone());
                                    exec_tab.append_output(&format!("{message}\n"));
                                    exec_sessions.remove(&session_id);
                                    cleanup_node_debug_session_if_needed(
                                        session_id,
                                        &mut node_debug_sessions,
                                        &node_debug_cleanup_tx,
                                    )
                                    .await;
                                }
                                ExecEvent::Error { error, session_id } => {
                                    exec_tab.loading = false;
                                    exec_tab.error = Some(error.clone());
                                    exec_tab.exited = true;
                                    exec_sessions.remove(&session_id);
                                    cleanup_node_debug_session_if_needed(
                                        session_id,
                                        &mut node_debug_sessions,
                                        &node_debug_cleanup_tx,
                                    )
                                    .await;
                                }
                            }
                            break;
                        }
                    }
                }
            }

            result = node_debug_cleanup_rx.recv() => {
                    if let Some(result) = result {
                        needs_redraw = true;
                        if let Err(err) = result.result {
                            app.set_error(format!(
                                "Node debug pod '{}'(namespace '{}') for node '{}' was not cleaned up automatically: {}",
                                result.pod_name, result.namespace, result.node_name, err
                            ));
                        }
                    }
                }

            result = workload_logs_bootstrap_rx.recv() => {
                needs_redraw = true;
                if let Some(result) = result {
                    let mut sources_to_start = Vec::new();
                    if let Some(tab) = app
                        .workbench_mut()
                        .find_tab_mut(&WorkbenchTabKey::WorkloadLogs(result.resource.clone()))
                        && let WorkbenchTabState::WorkloadLogs(logs_tab) = &mut tab.state
                        && logs_tab.session_id == result.session_id
                    {
                        match result.result {
                            Ok(targets) => {
                                logs_tab.update_targets(&targets);
                                let mut sources = Vec::new();
                                for target in targets {
                                    for container in target.containers {
                                        if sources.len() >= MAX_WORKLOAD_LOG_STREAMS {
                                            logs_tab.notice = Some(format!(
                                                "Stream cap reached at {MAX_WORKLOAD_LOG_STREAMS} pod/container streams."
                                            ));
                                            break;
                                        }
                                        sources.push((
                                            target.pod_name.clone(),
                                            target.namespace.clone(),
                                            container,
                                        ));
                                    }
                                }
                                if sources.is_empty() {
                                    logs_tab.loading = false;
                                    logs_tab.error = Some("No pod/container streams were resolved.".to_string());
                                } else {
                                    logs_tab.sources = sources.clone();
                                    logs_tab.loading = false;
                                    workload_log_sessions.insert(result.session_id, sources.clone());
                                    sources_to_start = sources;
                                }
                            }
                            Err(err) => {
                                logs_tab.loading = false;
                                logs_tab.error = Some(err);
                            }
                        }
                    }

                    for (pod_name, namespace, container_name) in sources_to_start {
                        let _ = coordinator
                            .start_log_streaming(pod_name, namespace, container_name, true, false, false)
                            .await;
                    }
                }
            }

            result = probe_rx.recv() => {
                if let Some(result) = result {
                    needs_redraw = true;
                    if let Some(detail) = &mut app.detail_view
                        && detail.resource.as_ref() == Some(&result.resource)
                    {
                        use kubectui::ui::components::probe_panel::ProbePanelState;
                        if let ResourceRef::Pod(pod_name, namespace) = result.resource {
                            detail.probe_panel = Some(match result.result {
                                Ok(probes) => ProbePanelState::new(pod_name, namespace, probes),
                                Err(error) => {
                                    let mut state =
                                        ProbePanelState::new(pod_name, namespace, Vec::new());
                                    state.error = Some(error);
                                    state
                                }
                            });
                        }
                    }
                }
            }

            result = extension_fetch_rx.recv() => {
                if let Some(result) = result {
                    needs_redraw = true;
                    apply_extension_fetch_result(&mut app, result);
                }
            }

            result = extension_command_rx.recv() => {
                if let Some(result) = result {
                    let resource_label = format!(
                        "{} on {} '{}'",
                        result.title,
                        result.resource.kind(),
                        result.resource.name(),
                    );
                    if let Some(execution_id) = result.execution_id
                        && let Some(tab) = app
                            .workbench_mut()
                            .find_tab_mut(&WorkbenchTabKey::ExtensionOutput(execution_id))
                        && let WorkbenchTabState::ExtensionOutput(tab_state) = &mut tab.state
                    {
                        tab_state.apply_output(
                            result.result.lines.clone(),
                            result.result.success,
                            result.result.exit_code,
                            result.result.error.clone(),
                        );
                    }

                    if result.result.success {
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Succeeded,
                            format!("{resource_label} completed."),
                            true,
                        );
                        set_transient_status(
                            &mut app,
                            &mut status_message_clear_at,
                            format!("{resource_label} completed."),
                        );
                    } else {
                        let message = result
                            .result
                            .error
                            .clone()
                            .unwrap_or_else(|| format!("{resource_label} failed."));
                        app.complete_action_history(
                            result.action_history_id,
                            ActionStatus::Failed,
                            message.clone(),
                            true,
                        );
                        app.set_error(message);
                    }
                    needs_redraw = true;
                }
            }

            result = ai_analysis_rx.recv() => {
                if let Some(result) = result {
                    let resource_label = format!(
                        "{} on {} '{}'",
                        result.title,
                        result.resource.kind(),
                        result.resource.name(),
                    );
                    if let Some(tab) = app
                        .workbench_mut()
                        .find_tab_mut(&WorkbenchTabKey::AiAnalysis(result.execution_id))
                        && let WorkbenchTabState::AiAnalysis(tab_state) = &mut tab.state
                    {
                        match result.result {
                            Ok(analysis) => {
                                tab_state.apply_result(
                                    analysis.provider_label,
                                    analysis.model,
                                    analysis.summary,
                                    analysis.likely_causes,
                                    analysis.next_steps,
                                    analysis.uncertainty,
                                );
                                app.complete_action_history(
                                    result.action_history_id,
                                    ActionStatus::Succeeded,
                                    format!("{resource_label} completed."),
                                    true,
                                );
                                set_transient_status(
                                    &mut app,
                                    &mut status_message_clear_at,
                                    format!("{resource_label} completed."),
                                );
                            }
                            Err(error) => {
                                tab_state.apply_error(error.clone());
                                app.complete_action_history(
                                    result.action_history_id,
                                    ActionStatus::Failed,
                                    error.clone(),
                                    true,
                                );
                                app.set_error(error);
                            }
                        }
                    } else {
                        match result.result {
                            Ok(_) => {
                                app.complete_action_history(
                                    result.action_history_id,
                                    ActionStatus::Succeeded,
                                    format!("{resource_label} completed."),
                                    true,
                                );
                                set_transient_status(
                                    &mut app,
                                    &mut status_message_clear_at,
                                    format!("{resource_label} completed."),
                                );
                            }
                            Err(error) => {
                                app.complete_action_history(
                                    result.action_history_id,
                                    ActionStatus::Failed,
                                    error.clone(),
                                    true,
                                );
                                app.set_error(error);
                            }
                        }
                    }
                    needs_redraw = true;
                }
            }

            result = relations_rx.recv() => {
                if let Some(result) = result {
                    let RelationsAsyncResult {
                        request_id,
                        resource: requested_resource,
                        result,
                    } = result;
                    let tab_key = WorkbenchTabKey::Relations(requested_resource.clone());
                    if let Some(tab) = app.workbench.find_tab_mut(&tab_key)
                        && let WorkbenchTabState::Relations(ref mut state) = tab.state
                        && state.pending_request_id == Some(request_id)
                    {
                        state.pending_request_id = None;
                        state.loading = false;
                        match result {
                            Ok(tree) => {
                                state.set_tree(tree);
                            }
                            Err(err) => {
                                state.error = Some(err);
                            }
                        }
                        needs_redraw = true;
                    }
                }
            }

            // Background context switch completed (TLS handshake finished)
            result = async {
                match &mut pending_context_switch {
                    Some((_, handle)) => handle.await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some((ctx, _)) = pending_context_switch.take() {
                    match result {
                        Ok(Ok(new_client)) => {
                            // Context-bound long-lived services must be rebuilt to avoid
                            // continuing background work against the previous cluster.
                            let _ = coordinator.shutdown().await;
                            for (_, handle) in exec_sessions.drain() {
                                let _ = handle.cancel_tx.send(());
                            }
                            for (_, session) in node_debug_sessions.drain() {
                                let _ = session
                                    .client
                                    .delete_node_debug_pod(&session.namespace, &session.pod_name)
                                    .await;
                            }
                            workload_log_sessions.clear();
                            port_forwarder.stop_all().await;
                            app.tunnel_registry.update_tunnels(Vec::new());

                            watch_manager.stop_all();

                            client = new_client;
                            app.current_context_name = Some(ctx.clone());
                            if let Some(snapshot) = app
                                .pending_workspace_restore
                                .take()
                                .filter(|snapshot| snapshot.context.as_deref() == Some(ctx.as_str()))
                            {
                                app.apply_workspace_snapshot(&snapshot);
                                reopen_pending_runbook(&mut app, &mut pending_runbook_restore);
                            }
                            coordinator = UpdateCoordinator::new(client.clone(), update_tx.clone());
                            port_forwarder =
                                PortForwarderService::new(std::sync::Arc::new(client.clone()));
                            // Invalidate stale async results from the previous client/context.
                            refresh_state.context_generation =
                                refresh_state.context_generation.wrapping_add(1);
                            abort_in_flight_refresh(&mut refresh_state);
                            abort_in_flight_events_fetch(&mut events_state);
                            events_state.queued_namespace = None;
                            refresh_state.queued_refresh = None;
                            delete_in_flight_id = None;
                            status_message_clear_at = None;
                            app.clear_status();
                            needs_redraw = true;

                            request_refresh(
                                &refresh_tx,
                                &mut global_state,
                                &client,
                                namespace_scope(app.get_namespace()).map(str::to_string),
                                refresh_options_for_view(app.view(), app.view().is_fluxcd(), true),
                                &mut refresh_state,
                                &mut snapshot_dirty,
                            );
                            // Restart watches for the new context.
                            watch_manager = start_watch_manager(
                                &client,
                                refresh_state.context_generation,
                                &app,
                                &watch_tx,
                            )
                            .await;
                            if app.view() == AppView::Events {
                                request_events_refresh(
                                    &events_tx,
                                    &mut global_state,
                                    &client,
                                    namespace_scope(app.get_namespace()).map(str::to_string),
                                    refresh_state.context_generation,
                                    &mut events_state,
                                    &mut snapshot_dirty,
                                );
                            }
                            if app.view() == kubectui::app::AppView::Extensions {
                                spawn_extensions_fetch(
                                    &client,
                                    &mut app,
                                    &cached_snapshot,
                                    &extension_fetch_tx,
                                );
                            }
                        }
                        Ok(Err(err)) => {
                            fail_context_switch(
                                &mut app,
                                &mut global_state,
                                format!("Failed to connect to context '{ctx}': {err:#}"),
                                &mut pending_runbook_restore,
                                &mut snapshot_dirty,
                                &mut needs_redraw,
                            );
                        }
                        Err(join_err) => {
                            fail_context_switch(
                                &mut app,
                                &mut global_state,
                                format!("Context switch task panicked: {join_err}"),
                                &mut pending_runbook_restore,
                                &mut snapshot_dirty,
                                &mut needs_redraw,
                            );
                        }
                    }
                }
            }

            trigger = deferred_refresh_rx.recv() => {
                if let Some(trigger) = trigger {
                    if trigger.context_generation != refresh_state.context_generation {
                        continue;
                    }
                    let events_namespace = trigger.namespace.clone();
                    request_refresh(
                        &refresh_tx,
                        &mut global_state,
                        &client,
                        trigger.namespace,
                        trigger.dispatch,
                        &mut refresh_state,
                        &mut snapshot_dirty,
                    );
                    if trigger.view == AppView::Events {
                        request_events_refresh(
                            &events_tx,
                            &mut global_state,
                            &client,
                            events_namespace,
                            refresh_state.context_generation,
                            &mut events_state,
                            &mut snapshot_dirty,
                        );
                    }
                }
            }

            // Periodic tick — heartbeat for follow-mode log scrolling
            _ = tick.tick() => {
                if status_message_clear_at.is_some_and(|deadline| Instant::now() >= deadline) {
                    app.clear_status();
                    status_message_clear_at = None;
                    needs_redraw = true;
                }
                // Animate spinner during loading/refreshing phases
                let phase = cached_snapshot.phase;
                if matches!(phase, DataPhase::Loading | DataPhase::Idle)
                    || refresh_state.in_flight_id.is_some()
                {
                    app.advance_spinner();
                    needs_redraw = true;
                }
                // Expire old toasts
                if app.expire_toasts() {
                    needs_redraw = true;
                }
            }

            // Auto-refresh: re-fetch cluster data periodically
            _ = auto_refresh.tick() => {
                // Skip auto-refresh if a detail view is open (avoid disrupting user)
                // or if we're in a backoff period from consecutive failures
                // or if a refresh is already in flight
                let in_backoff = backoff_until.is_some_and(|t| Instant::now() < t);
                if app.detail_view.is_none() && !in_backoff {
                    auto_refresh_count = auto_refresh_count.wrapping_add(1);
                    let include_flux = app.view().is_fluxcd()
                        || auto_refresh_count.is_multiple_of(FLUX_AUTO_REFRESH_EVERY);
                    let mut dispatch = refresh_options_for_view(app.view(), include_flux, false);
                    // Strip watched scopes — watches provide real-time updates
                    dispatch.primary_scope = dispatch.primary_scope.without(RefreshScope::WATCHED_SCOPES);
                    dispatch.options.scope = dispatch.options.scope.without(RefreshScope::WATCHED_SCOPES);
                    if !dispatch.options.scope.is_empty() || dispatch.options.include_cluster_info {
                        request_refresh(
                            &refresh_tx,
                            &mut global_state,
                            &client,
                            namespace_scope(app.get_namespace()).map(str::to_string),
                            dispatch,
                            &mut refresh_state,
                            &mut snapshot_dirty,
                        );
                    }
                    if app.view() == AppView::Events {
                        request_events_refresh(
                            &events_tx,
                            &mut global_state,
                            &client,
                            namespace_scope(app.get_namespace()).map(str::to_string),
                            refresh_state.context_generation,
                            &mut events_state,
                            &mut snapshot_dirty,
                        );
                    }
                }
            }

            // Deferred palette action — fires immediately when detail view is loaded
            _ = std::future::ready(()), if pending_action_ready => {
                action_to_process = pending_palette_action.take();
                needs_redraw = true;
            }

            // Keyboard / terminal input — lowest priority so messages are drained first
            maybe_event = event_stream.next() => {
                let key = match maybe_event {
                    Some(Ok(Event::Resize(_, _))) => {
                        needs_redraw = true;
                        continue;
                    }
                    Some(Ok(Event::Key(key))) => key,
                    _ => continue,
                };
                needs_redraw = true;

                let action = if key.code == KeyCode::Enter
                    && !app.is_search_mode()
                    && !app.is_namespace_picker_open()
                    && !app.is_context_picker_open()
                    && !app.command_palette.is_open()
                    && app.detail_view.is_none()
                    && app.focus != kubectui::app::Focus::Workbench
                {
                    if app.focus == kubectui::app::Focus::Content
                        && app.view() == AppView::Extensions
                        && !app.extension_in_instances
                    {
                        // Drill into instances pane
                        if !app.extension_instances.is_empty() {
                            app.extension_in_instances = true;
                            app.extension_instance_cursor = 0;
                        }
                        AppAction::None
                    } else if app.focus == kubectui::app::Focus::Content
                        && app.view() == AppView::Bookmarks
                    {
                        match prepare_bookmark_target(&mut app, &cached_snapshot) {
                            Ok(resource) => AppAction::OpenDetail(resource),
                            Err(err) => {
                                app.set_error(err);
                                AppAction::None
                            }
                        }
                    } else if app.focus == kubectui::app::Focus::Content {
                        selected_resource(&app, &cached_snapshot)
                            .map(AppAction::OpenDetail)
                            .unwrap_or(AppAction::None)
                    } else {
                        app.sidebar_activate()
                    }
                } else if key.code == KeyCode::Esc
                    && app.view() == AppView::Extensions
                    && app.extension_in_instances
                    && app.detail_view.is_none()
                    && !app.is_search_mode()
                {
                    // Return from instances pane to CRD picker
                    app.extension_in_instances = false;
                    AppAction::None
                } else {
                    app.handle_key_event(key)
                };

                action_to_process = Some(action);
            }
        }

        // --- Action dispatch (shared between keyboard input and deferred palette actions) ---
        if let Some(action) = action_to_process {
            match action {
                AppAction::None => {
                    // No-op — don't call sync_extensions_instances on every unrecognized key
                }
                AppAction::Quit => break,
                AppAction::RefreshData => {
                    // Manual refresh bypasses backoff
                    consecutive_refresh_failures = 0;
                    backoff_until = None;
                    request_refresh(
                        &refresh_tx,
                        &mut global_state,
                        &client,
                        namespace_scope(app.get_namespace()).map(str::to_string),
                        full_refresh_options(true, true),
                        &mut refresh_state,
                        &mut snapshot_dirty,
                    );
                    if app.view() == AppView::Events {
                        request_events_refresh(
                            &events_tx,
                            &mut global_state,
                            &client,
                            namespace_scope(app.get_namespace()).map(str::to_string),
                            refresh_state.context_generation,
                            &mut events_state,
                            &mut snapshot_dirty,
                        );
                    }
                    // Reset auto-refresh timer after manual refresh
                    auto_refresh.reset();
                }
                AppAction::FluxReconcile => {
                    let reconcile_resource =
                        match selected_flux_reconcile_resource(&app, &cached_snapshot) {
                            Ok(resource) => resource,
                            Err(err) => {
                                app.set_error(err);
                                continue;
                            }
                        };
                    if let Some(message) = detail_action_block_message(
                        &app,
                        &client,
                        &reconcile_resource,
                        DetailAction::FluxReconcile,
                    )
                    .await
                    {
                        app.set_error(message);
                        continue;
                    }

                    let resource_label = format!(
                        "{} '{}'",
                        reconcile_resource.kind(),
                        reconcile_resource.name()
                    );
                    let baseline =
                        flux_observed_state_for_resource(&cached_snapshot, &reconcile_resource);
                    let origin_view = app.view();
                    let action_history_id = app.record_action_pending(
                        ActionKind::FluxReconcile,
                        origin_view,
                        Some(reconcile_resource.clone()),
                        resource_label.clone(),
                        format!("Requesting reconcile for {resource_label}..."),
                    );
                    begin_detail_mutation(
                        &mut app,
                        &mut status_message_clear_at,
                        format!("Requesting reconcile for {resource_label}..."),
                    );
                    let tx = flux_reconcile_tx.clone();
                    let c = client.clone();
                    let context_generation = refresh_state.context_generation;
                    tokio::spawn(async move {
                        let result = match &reconcile_resource {
                            ResourceRef::CustomResource {
                                name,
                                namespace,
                                group,
                                version,
                                kind,
                                plural,
                            } => c
                                .request_flux_reconcile(
                                    group,
                                    version,
                                    kind,
                                    plural,
                                    name,
                                    namespace.as_deref(),
                                )
                                .await
                                .map_err(|err| format!("{err:#}")),
                            _ => Err("Flux reconcile is only available for custom resources."
                                .to_string()),
                        };
                        let _ = tx
                            .send(FluxReconcileAsyncResult {
                                action_history_id,
                                context_generation,
                                origin_view,
                                resource: reconcile_resource,
                                resource_label,
                                baseline,
                                result,
                            })
                            .await;
                    });
                }
                AppAction::ToggleWorkbench
                | AppAction::WorkbenchNextTab
                | AppAction::WorkbenchPreviousTab
                | AppAction::WorkbenchCloseActiveTab
                | AppAction::WorkbenchIncreaseHeight
                | AppAction::WorkbenchDecreaseHeight => {
                    let streams_to_stop = workbench_follow_streams_to_stop(&app, action.clone());
                    let workload_sessions_to_stop =
                        workbench_workload_log_sessions_to_stop(&app, action.clone());
                    let exec_sessions_to_stop =
                        workbench_exec_sessions_to_stop(&app, action.clone());
                    for (pod_name, namespace, container_name) in streams_to_stop {
                        let _ = coordinator
                            .stop_log_streaming(&pod_name, &namespace, &container_name)
                            .await;
                    }
                    for session_id in workload_sessions_to_stop {
                        if let Some(streams) = workload_log_sessions.remove(&session_id) {
                            for (pod_name, namespace, container_name) in streams {
                                let _ = coordinator
                                    .stop_log_streaming(&pod_name, &namespace, &container_name)
                                    .await;
                            }
                        }
                    }
                    for session_id in exec_sessions_to_stop {
                        if let Some(handle) = exec_sessions.remove(&session_id) {
                            let _ = handle.cancel_tx.send(());
                        }
                        cleanup_node_debug_session_if_needed(
                            session_id,
                            &mut node_debug_sessions,
                            &node_debug_cleanup_tx,
                        )
                        .await;
                    }
                    apply_action(action, &mut app);
                    app.needs_config_save = true;
                }
                AppAction::OpenNamespacePicker => {
                    app.set_available_namespaces(global_state.namespaces().to_vec());
                    app.open_namespace_picker();
                }
                AppAction::CloseNamespacePicker => {
                    app.close_namespace_picker();
                }
                AppAction::OpenCommandPalette => {
                    let resource_ctx = if let Some(resource_ctx) = app
                        .detail_view
                        .as_ref()
                        .and_then(|d| d.resource_action_context())
                    {
                        Some(resource_ctx)
                    } else if let Some(mut resource_ctx) =
                        selected_resource_context(&app, &cached_snapshot)
                    {
                        resource_ctx.action_authorizations = client
                            .fetch_detail_action_authorizations(&resource_ctx.resource)
                            .await;
                        Some(resource_ctx)
                    } else {
                        None
                    };
                    app.refresh_palette_columns();
                    app.refresh_palette_workspaces();
                    refresh_palette_activity(&mut app);
                    refresh_palette_resources(&mut app, &cached_snapshot);
                    refresh_palette_extensions(&mut app, &cached_snapshot, &extension_registry);
                    refresh_palette_runbooks(&mut app, &cached_snapshot, &runbook_registry);
                    app.command_palette.open_with_context(resource_ctx);
                }
                AppAction::CloseCommandPalette => {
                    app.command_palette.close();
                }
                AppAction::SaveWorkspace => {
                    app.command_palette.close();
                    app.save_current_workspace();
                }
                AppAction::ApplyPreviousWorkspace => {
                    app.command_palette.close();
                    match app.cycle_saved_workspace_name(false) {
                        Ok(name) => pending_palette_action = Some(AppAction::ApplyWorkspace(name)),
                        Err(err) => app.set_error(err),
                    }
                }
                AppAction::ApplyNextWorkspace => {
                    app.command_palette.close();
                    match app.cycle_saved_workspace_name(true) {
                        Ok(name) => pending_palette_action = Some(AppAction::ApplyWorkspace(name)),
                        Err(err) => app.set_error(err),
                    }
                }
                AppAction::PaletteAction { action, resource } => {
                    app.command_palette.close();

                    let mapped = map_palette_detail_action(action);
                    let needs_detail = palette_detail_action_needs_detail(action);

                    if needs_detail && app.detail_view.is_none() {
                        open_detail_for_resource(
                            &mut app,
                            &cached_snapshot,
                            &client,
                            &detail_tx,
                            resource,
                            &mut detail_request_seq,
                        );
                    }

                    pending_palette_action = Some(mapped);
                }
                AppAction::OpenRunbook { id, resource } => {
                    app.command_palette.close();
                    let Some(runbook) = runbook_registry.get(&id).cloned() else {
                        app.set_error(format!("Runbook '{id}' is no longer available."));
                        continue;
                    };
                    if !runbook.matches_resource(resource.as_ref()) {
                        let scope = resource
                            .as_ref()
                            .map(|value| value.kind().to_string())
                            .unwrap_or_else(|| "current".to_string());
                        app.set_error(format!(
                            "Runbook '{}' does not apply to {scope} resources.",
                            runbook.title
                        ));
                        continue;
                    }
                    app.open_runbook_tab(runbook, resource);
                }
                AppAction::RunbookToggleStepDone => {
                    if let Some(WorkbenchTabState::Runbook(tab)) =
                        app.workbench.active_tab_mut().map(|tab| &mut tab.state)
                    {
                        tab.toggle_done();
                        tab.banner = Some(format!("Updated progress: {}", tab.progress_label()));
                    }
                }
                AppAction::RunbookToggleStepSkipped => {
                    if let Some(WorkbenchTabState::Runbook(tab)) =
                        app.workbench.active_tab_mut().map(|tab| &mut tab.state)
                    {
                        tab.toggle_skipped();
                        tab.banner = Some(format!("Updated progress: {}", tab.progress_label()));
                    }
                }
                AppAction::RunbookExecuteSelectedStep => {
                    let runbook_context =
                        app.workbench.active_tab().and_then(|tab| match &tab.state {
                            WorkbenchTabState::Runbook(tab) => tab.selected_step().map(|step| {
                                (
                                    tab.as_ref().clone(),
                                    tab.runbook.title.clone(),
                                    tab.resource.clone(),
                                    step.step.clone(),
                                )
                            }),
                            _ => None,
                        });
                    let Some((runbook_tab, runbook_title, resource, step)) = runbook_context else {
                        continue;
                    };

                    let mut banner_message = Some(format!("Ready: {}", step.title));
                    match step.kind {
                        LoadedRunbookStepKind::Checklist { .. } => {
                            if let Some(WorkbenchTabState::Runbook(tab)) =
                                app.workbench.active_tab_mut().map(|tab| &mut tab.state)
                            {
                                tab.toggle_done();
                                banner_message = Some(format!(
                                    "{}: checklist progress is now {}",
                                    step.title,
                                    tab.progress_label()
                                ));
                            }
                        }
                        LoadedRunbookStepKind::Workspace { name, target } => {
                            pending_runbook_restore = Some(runbook_tab);
                            banner_message = Some(format!(
                                "{runbook_title}: applying {} '{}'",
                                target.label(),
                                name
                            ));
                            pending_palette_action = Some(match target {
                                kubectui::runbooks::RunbookWorkspaceTarget::SavedWorkspace => {
                                    AppAction::ApplyWorkspace(name)
                                }
                                kubectui::runbooks::RunbookWorkspaceTarget::WorkspaceBank => {
                                    AppAction::ActivateWorkspaceBank(name)
                                }
                            });
                        }
                        LoadedRunbookStepKind::DetailAction { action } => {
                            let Some(resource) = resource else {
                                app.set_error(format!(
                                    "Runbook '{}' requires a selected resource for '{}'.",
                                    runbook_title, step.title
                                ));
                                banner_message =
                                    Some("Resource-scoped step could not run.".to_string());
                                if let Some(WorkbenchTabState::Runbook(tab)) =
                                    app.workbench.active_tab_mut().map(|tab| &mut tab.state)
                                {
                                    tab.banner = banner_message;
                                }
                                continue;
                            };
                            let detail_action = action.into_detail_action();
                            if palette_detail_action_needs_detail(detail_action)
                                && app.detail_view.is_none()
                            {
                                open_detail_for_resource(
                                    &mut app,
                                    &cached_snapshot,
                                    &client,
                                    &detail_tx,
                                    resource.clone(),
                                    &mut detail_request_seq,
                                );
                            }
                            banner_message =
                                Some(format!("{runbook_title}: running {}", action.label()));
                            pending_palette_action = Some(map_palette_detail_action(detail_action));
                        }
                        LoadedRunbookStepKind::ExtensionAction { action_id } => {
                            let Some(resource) = resource else {
                                app.set_error(format!(
                                    "Runbook '{}' requires a selected resource for '{}'.",
                                    runbook_title, step.title
                                ));
                                banner_message =
                                    Some("Resource-scoped step could not run.".to_string());
                                if let Some(WorkbenchTabState::Runbook(tab)) =
                                    app.workbench.active_tab_mut().map(|tab| &mut tab.state)
                                {
                                    tab.banner = banner_message;
                                }
                                continue;
                            };
                            banner_message = Some(format!(
                                "{runbook_title}: running extension '{}'",
                                action_id
                            ));
                            pending_palette_action = Some(AppAction::ExecuteExtension {
                                id: action_id,
                                resource,
                            });
                        }
                        LoadedRunbookStepKind::AiWorkflow { workflow } => {
                            let Some(resource) = resource else {
                                app.set_error(format!(
                                    "Runbook '{}' requires a selected resource for '{}'.",
                                    runbook_title, step.title
                                ));
                                banner_message =
                                    Some("Resource-scoped step could not run.".to_string());
                                if let Some(WorkbenchTabState::Runbook(tab)) =
                                    app.workbench.active_tab_mut().map(|tab| &mut tab.state)
                                {
                                    tab.banner = banner_message;
                                }
                                continue;
                            };
                            banner_message = Some(format!(
                                "{runbook_title}: running AI workflow '{}'",
                                workflow.default_title()
                            ));
                            pending_palette_action = Some(AppAction::ExecuteExtension {
                                id: workflow.default_id().to_string(),
                                resource,
                            });
                        }
                    }

                    if let Some(WorkbenchTabState::Runbook(tab)) =
                        app.workbench.active_tab_mut().map(|tab| &mut tab.state)
                    {
                        tab.banner = banner_message;
                    }
                }
                AppAction::ExecuteExtension { id, resource } => {
                    app.command_palette.close();

                    let Some(action) = extension_registry.get(&id).cloned() else {
                        app.set_error(format!("Extension '{id}' is no longer available."));
                        continue;
                    };
                    if !action.matches_resource(&resource) {
                        app.set_error(format!(
                            "Extension '{}' does not apply to {} resources.",
                            action.title,
                            resource.kind()
                        ));
                        continue;
                    }

                    let resource_label = format!(
                        "{} on {} '{}'",
                        action.title,
                        resource.kind(),
                        resource.name()
                    );
                    let action_history_id = app.record_action_pending(
                        ActionKind::Extension,
                        app.view(),
                        Some(resource.clone()),
                        resource_label.clone(),
                        format!("Running {resource_label}..."),
                    );

                    match action.kind.clone() {
                        LoadedExtensionActionKind::Command { mode, command } => {
                            let context =
                                extension_context_for_resource(&app, &cached_snapshot, &resource);
                            let prepared = match prepare_command(&action.title, &command, &context)
                            {
                                Ok(prepared) => prepared,
                                Err(err) => {
                                    let message = format!("{resource_label} failed: {err}");
                                    app.complete_action_history(
                                        action_history_id,
                                        ActionStatus::Failed,
                                        message.clone(),
                                        true,
                                    );
                                    app.set_error(message);
                                    needs_redraw = true;
                                    continue;
                                }
                            };

                            match mode {
                                ExtensionExecutionMode::Foreground => {
                                    match run_extension_command_in_terminal(terminal, &prepared) {
                                        Ok(status) if status.success() => {
                                            app.complete_action_history(
                                                action_history_id,
                                                ActionStatus::Succeeded,
                                                format!("{resource_label} completed."),
                                                true,
                                            );
                                            set_transient_status(
                                                &mut app,
                                                &mut status_message_clear_at,
                                                format!("{resource_label} completed."),
                                            );
                                            needs_redraw = true;
                                        }
                                        Ok(status) => {
                                            let message = status
                                        .code()
                                        .map(|code| {
                                            format!("{resource_label} exited with status {code}.")
                                        })
                                        .unwrap_or_else(|| {
                                            format!("{resource_label} terminated by signal.")
                                        });
                                            app.complete_action_history(
                                                action_history_id,
                                                ActionStatus::Failed,
                                                message.clone(),
                                                true,
                                            );
                                            app.set_error(message);
                                            needs_redraw = true;
                                        }
                                        Err(err) => {
                                            let message =
                                                format!("{resource_label} failed: {err:#}");
                                            app.complete_action_history(
                                                action_history_id,
                                                ActionStatus::Failed,
                                                message.clone(),
                                                true,
                                            );
                                            app.set_error(message);
                                            needs_redraw = true;
                                        }
                                    }
                                }
                                ExtensionExecutionMode::Background
                                | ExtensionExecutionMode::Silent => {
                                    let execution_id = if mode == ExtensionExecutionMode::Background
                                    {
                                        let execution_id = next_extension_execution_id;
                                        next_extension_execution_id =
                                            next_extension_execution_id.wrapping_add(1).max(1);
                                        app.detail_view = None;
                                        app.open_extension_output_tab(
                                            execution_id,
                                            action.title.clone(),
                                            Some(resource.clone()),
                                            mode.label(),
                                            prepared.preview.clone(),
                                        );
                                        Some(execution_id)
                                    } else {
                                        set_transient_status(
                                            &mut app,
                                            &mut status_message_clear_at,
                                            format!("Running {resource_label}..."),
                                        );
                                        None
                                    };

                                    let tx = extension_command_tx.clone();
                                    let title = action.title.clone();
                                    tokio::spawn(async move {
                                        let result = match tokio::task::spawn_blocking(move || {
                                            run_extension_command(prepared)
                                        })
                                        .await
                                        {
                                            Ok(result) => result,
                                            Err(err) => ExtensionCommandRunResult {
                                                lines: Vec::new(),
                                                success: false,
                                                exit_code: None,
                                                error: Some(format!(
                                                    "extension task failed to join: {err}"
                                                )),
                                            },
                                        };
                                        let _ = tx
                                            .send(ExtensionCommandAsyncResult {
                                                action_history_id,
                                                resource,
                                                execution_id,
                                                title,
                                                result,
                                            })
                                            .await;
                                    });
                                }
                            }
                        }
                        LoadedExtensionActionKind::AiAnalysis {
                            provider,
                            workflow,
                            system_prompt,
                        } => {
                            let execution_id = next_ai_execution_id;
                            next_ai_execution_id = next_ai_execution_id.wrapping_add(1).max(1);
                            let context = build_ai_analysis_context(
                                &app,
                                &cached_snapshot,
                                &resource,
                                workflow,
                            );
                            app.detail_view = None;
                            app.open_ai_analysis_tab(
                                execution_id,
                                action.title.clone(),
                                resource.clone(),
                            );
                            let tx = ai_analysis_tx.clone();
                            let title = action.title.clone();
                            tokio::spawn(async move {
                                let provider = provider.clone();
                                let system_prompt = system_prompt.clone().unwrap_or_else(|| {
                                    default_system_prompt_for_workflow(workflow).to_string()
                                });
                                let context = context.clone();
                                let result = match tokio::task::spawn_blocking(move || {
                                    run_ai_analysis(&provider, system_prompt.as_str(), &context)
                                })
                                .await
                                {
                                    Ok(result) => result
                                        .map_err(|err| format!("{resource_label} failed: {err:#}")),
                                    Err(err) => Err(format!(
                                        "{resource_label} AI analysis task failed to join: {err}"
                                    )),
                                };
                                let _ = tx
                                    .send(AiAnalysisAsyncResult {
                                        action_history_id,
                                        resource,
                                        execution_id,
                                        title,
                                        result,
                                    })
                                    .await;
                            });
                        }
                    }
                }
                AppAction::ApplyWorkspace(name) => {
                    app.command_palette.close();
                    let Some(snapshot) = app.saved_workspace_snapshot(&name) else {
                        pending_runbook_restore.take();
                        app.set_error(format!("Saved workspace '{name}' was not found."));
                        continue;
                    };
                    if let Some(target_context) = snapshot.context.clone()
                        && app.current_context_name.as_deref() != Some(target_context.as_str())
                    {
                        app.pending_workspace_restore = Some(snapshot);
                        pending_palette_action = Some(AppAction::SelectContext(target_context));
                        continue;
                    }
                    if snapshot.namespace != app.get_namespace() {
                        let target_namespace = snapshot.namespace.clone();
                        app.pending_workspace_restore = Some(snapshot);
                        pending_palette_action = Some(AppAction::SelectNamespace(target_namespace));
                        continue;
                    }
                    let mut runtime = WorkspaceRestoreRuntime {
                        coordinator: &mut coordinator,
                        workload_log_sessions: &mut workload_log_sessions,
                        exec_sessions: &mut exec_sessions,
                        node_debug_sessions: &mut node_debug_sessions,
                        node_debug_cleanup_tx: &node_debug_cleanup_tx,
                        port_forwarder: &mut port_forwarder,
                        global_state: &mut global_state,
                        client: &client,
                        refresh_tx: &refresh_tx,
                        refresh_state: &mut refresh_state,
                        snapshot_dirty: &mut snapshot_dirty,
                        events_tx: &events_tx,
                        events_state: &mut events_state,
                        cached_snapshot: &cached_snapshot,
                        extension_fetch_tx: &extension_fetch_tx,
                    };
                    apply_workspace_snapshot_and_refresh(&mut app, &snapshot, &mut runtime).await;
                    reopen_pending_runbook(&mut app, &mut pending_runbook_restore);
                    app.set_status(format!("Applied workspace: {name}"));
                }
                AppAction::ActivateWorkspaceBank(name) => {
                    app.command_palette.close();
                    let Some(snapshot) = app.workspace_bank_snapshot(&name) else {
                        pending_runbook_restore.take();
                        app.set_error(format!("Workspace bank '{name}' was not found."));
                        continue;
                    };
                    if let Some(target_context) = snapshot.context.clone()
                        && app.current_context_name.as_deref() != Some(target_context.as_str())
                    {
                        app.pending_workspace_restore = Some(snapshot);
                        pending_palette_action = Some(AppAction::SelectContext(target_context));
                        continue;
                    }
                    if snapshot.namespace != app.get_namespace() {
                        let target_namespace = snapshot.namespace.clone();
                        app.pending_workspace_restore = Some(snapshot);
                        pending_palette_action = Some(AppAction::SelectNamespace(target_namespace));
                        continue;
                    }
                    let mut runtime = WorkspaceRestoreRuntime {
                        coordinator: &mut coordinator,
                        workload_log_sessions: &mut workload_log_sessions,
                        exec_sessions: &mut exec_sessions,
                        node_debug_sessions: &mut node_debug_sessions,
                        node_debug_cleanup_tx: &node_debug_cleanup_tx,
                        port_forwarder: &mut port_forwarder,
                        global_state: &mut global_state,
                        client: &client,
                        refresh_tx: &refresh_tx,
                        refresh_state: &mut refresh_state,
                        snapshot_dirty: &mut snapshot_dirty,
                        events_tx: &events_tx,
                        events_state: &mut events_state,
                        cached_snapshot: &cached_snapshot,
                        extension_fetch_tx: &extension_fetch_tx,
                    };
                    apply_workspace_snapshot_and_refresh(&mut app, &snapshot, &mut runtime).await;
                    reopen_pending_runbook(&mut app, &mut pending_runbook_restore);
                    app.set_status(format!("Activated workspace bank: {name}"));
                }
                AppAction::NavigateTo(view) => {
                    app.command_palette.close();
                    app.record_recent_view_jump(view);
                    app.navigate_to_view(view);
                    app.focus = kubectui::app::Focus::Content;
                    app.extension_in_instances = false;
                    if !matches!(
                        view,
                        kubectui::app::AppView::PortForwarding | kubectui::app::AppView::HelmCharts
                    ) {
                        request_refresh(
                            &refresh_tx,
                            &mut global_state,
                            &client,
                            namespace_scope(app.get_namespace()).map(str::to_string),
                            refresh_options_for_view(view, view.is_fluxcd(), false),
                            &mut refresh_state,
                            &mut snapshot_dirty,
                        );
                        if view == AppView::Events {
                            request_events_refresh(
                                &events_tx,
                                &mut global_state,
                                &client,
                                namespace_scope(app.get_namespace()).map(str::to_string),
                                refresh_state.context_generation,
                                &mut events_state,
                                &mut snapshot_dirty,
                            );
                        }
                    }
                    // Trigger extensions sync when navigating to Extensions view
                    if view == kubectui::app::AppView::Extensions {
                        spawn_extensions_fetch(
                            &client,
                            &mut app,
                            &cached_snapshot,
                            &extension_fetch_tx,
                        );
                    }
                }
                AppAction::OpenContextPicker => {
                    let contexts = K8sClient::list_contexts();
                    let current = kube::config::Kubeconfig::read()
                        .ok()
                        .and_then(|cfg| cfg.current_context);
                    app.open_context_picker(contexts, current);
                }
                AppAction::CloseContextPicker => {
                    app.close_context_picker();
                }
                AppAction::SelectContext(ctx) => {
                    app.close_context_picker();
                    if app
                        .pending_workspace_restore
                        .as_ref()
                        .and_then(|snapshot| snapshot.context.as_deref())
                        != Some(ctx.as_str())
                    {
                        app.pending_workspace_restore = None;
                    }
                    pending_flux_reconcile_verifications.clear();
                    // Show loading state immediately; TLS handshake runs in background.
                    global_state.begin_loading_transition(true);
                    app.selected_idx = 0;
                    app.search_query.clear();
                    app.is_search_mode = false;
                    app.detail_view = None;
                    app.workbench.close_resource_tabs();
                    app.sync_workbench_focus();
                    snapshot_dirty = true;
                    needs_redraw = true;

                    let ctx_clone = ctx.clone();
                    pending_context_switch = Some((
                        ctx,
                        tokio::spawn(async move {
                            tokio::time::timeout(
                                Duration::from_secs(15),
                                K8sClient::connect_with_context(&ctx_clone),
                            )
                            .await
                            .map_err(|_| anyhow::anyhow!("connection timed out after 15s"))?
                        }),
                    ));
                }
                AppAction::SelectNamespace(namespace) => {
                    let selected_namespace = namespace.clone();
                    let workspace_restore_pending = app.pending_workspace_restore.is_some();
                    app.set_namespace(namespace);
                    pending_flux_reconcile_verifications.clear();
                    app.selected_idx = 0;
                    app.close_namespace_picker();
                    app.needs_config_save = true;
                    watch_manager.stop_all();
                    // Invalidate stale async results from previous namespace selections.
                    refresh_state.context_generation =
                        refresh_state.context_generation.wrapping_add(1);
                    abort_in_flight_refresh(&mut refresh_state);
                    abort_in_flight_events_fetch(&mut events_state);
                    events_state.queued_namespace = None;
                    refresh_state.queued_refresh = None;
                    delete_in_flight_id = None;
                    for (_, handle) in exec_sessions.drain() {
                        let _ = handle.cancel_tx.send(());
                    }
                    for (_, session) in node_debug_sessions.drain() {
                        let _ = session
                            .client
                            .delete_node_debug_pod(&session.namespace, &session.pod_name)
                            .await;
                    }
                    for (_, streams) in workload_log_sessions.drain() {
                        for (pod_name, namespace, container_name) in streams {
                            let _ = coordinator
                                .stop_log_streaming(&pod_name, &namespace, &container_name)
                                .await;
                        }
                    }
                    if workspace_restore_pending {
                        port_forwarder.stop_all().await;
                        app.tunnel_registry.update_tunnels(Vec::new());
                    }
                    status_message_clear_at = None;
                    app.clear_status();
                    // Drop old namespace data immediately to prevent inconsistent mixed views.
                    global_state.begin_loading_transition(false);
                    snapshot_dirty = true;
                    app.detail_view = None;
                    app.workbench.close_resource_tabs();
                    if let Some(snapshot) =
                        app.pending_workspace_restore.take().filter(|snapshot| {
                            snapshot.namespace == selected_namespace
                                && snapshot.context.as_deref().is_none_or(|ctx| {
                                    app.current_context_name.as_deref() == Some(ctx)
                                })
                        })
                    {
                        app.apply_workspace_snapshot(&snapshot);
                        reopen_pending_runbook(&mut app, &mut pending_runbook_restore);
                    }
                    app.sync_workbench_focus();

                    // Queue newest namespace refresh; if one is in flight it gets coalesced.
                    request_refresh(
                        &refresh_tx,
                        &mut global_state,
                        &client,
                        namespace_scope(app.get_namespace()).map(str::to_string),
                        refresh_options_for_view(app.view(), app.view().is_fluxcd(), false),
                        &mut refresh_state,
                        &mut snapshot_dirty,
                    );
                    if app.view() == AppView::Events {
                        request_events_refresh(
                            &events_tx,
                            &mut global_state,
                            &client,
                            namespace_scope(app.get_namespace()).map(str::to_string),
                            refresh_state.context_generation,
                            &mut events_state,
                            &mut snapshot_dirty,
                        );
                    }
                    // Restart watches for the new namespace.
                    watch_manager = start_watch_manager(
                        &client,
                        refresh_state.context_generation,
                        &app,
                        &watch_tx,
                    )
                    .await;
                    if app.view() == kubectui::app::AppView::Extensions {
                        spawn_extensions_fetch(
                            &client,
                            &mut app,
                            &cached_snapshot,
                            &extension_fetch_tx,
                        );
                    }
                }
                AppAction::JumpToResource(resource) => {
                    app.command_palette.close();
                    match prepare_resource_target(&mut app, &cached_snapshot, &resource) {
                        Ok(()) => {
                            open_detail_for_resource(
                                &mut app,
                                &cached_snapshot,
                                &client,
                                &detail_tx,
                                resource,
                                &mut detail_request_seq,
                            );
                        }
                        Err(err) => app.set_error(err),
                    }
                }
                AppAction::ActivateWorkbenchTab(key) => {
                    app.command_palette.close();
                    if app.workbench.activate_tab(&key) {
                        app.focus = kubectui::app::Focus::Workbench;
                    } else {
                        app.set_error("Selected workbench activity is no longer available.".into());
                    }
                }
                AppAction::OpenDetail(resource) => {
                    open_detail_for_resource(
                        &mut app,
                        &cached_snapshot,
                        &client,
                        &detail_tx,
                        resource,
                        &mut detail_request_seq,
                    );
                    if app.focus == kubectui::app::Focus::Workbench {
                        app.focus = kubectui::app::Focus::Content;
                    }
                }
                AppAction::ActionHistoryOpenSelected => {
                    let Some(target) = app.selected_action_history_target().cloned() else {
                        app.set_error(
                            "Selected history entry does not have a jumpable resource.".to_string(),
                        );
                        continue;
                    };
                    app.view = target.view;
                    app.selected_idx = 0;
                    app.focus = kubectui::app::Focus::Content;
                    app.extension_in_instances = false;
                    open_detail_for_resource(
                        &mut app,
                        &cached_snapshot,
                        &client,
                        &detail_tx,
                        target.resource,
                        &mut detail_request_seq,
                    );
                }
                AppAction::CloseDetail => {
                    app.detail_view = None;
                }
                AppAction::OpenResourceYaml => {
                    if action::detail_tabs::handle_open_resource_yaml(
                        &mut app,
                        &client,
                        &cached_snapshot,
                        &detail_tx,
                        &mut detail_request_seq,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::OpenResourceDiff => {
                    if action::detail_tabs::handle_open_resource_diff(
                        &mut app,
                        &client,
                        &cached_snapshot,
                        &resource_diff_tx,
                        &mut resource_diff_request_seq,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::OpenRollout => {
                    if action::rollout::handle_open_rollout(
                        &mut app,
                        &client,
                        &cached_snapshot,
                        &rollout_inspection_tx,
                        &mut rollout_inspection_request_seq,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::OpenHelmHistory => {
                    if action::helm::handle_open_helm_history(
                        &mut app,
                        &client,
                        &cached_snapshot,
                        &helm_history_tx,
                        &mut helm_history_request_seq,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::OpenHelmValuesDiff => {
                    if action::helm::handle_open_helm_values_diff(
                        &mut app,
                        &helm_values_diff_tx,
                        &mut helm_values_diff_request_seq,
                    ) {
                        continue;
                    }
                }
                AppAction::OpenDecodedSecret => {
                    if action::detail_tabs::handle_open_decoded_secret(
                        &mut app,
                        &client,
                        &cached_snapshot,
                        &detail_tx,
                        &mut detail_request_seq,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::ToggleBookmark => {
                    if action::detail_tabs::handle_toggle_bookmark(&mut app, &cached_snapshot) {
                        continue;
                    }
                }
                AppAction::OpenRelationships => {
                    if action::detail_tabs::handle_open_relationships(
                        &mut app,
                        &cached_snapshot,
                        &client,
                        &relations_tx,
                        &mut relations_request_seq,
                    ) {
                        continue;
                    }
                }
                AppAction::OpenNetworkPolicyView => {
                    if action::detail_tabs::handle_open_network_policies(
                        &mut app,
                        &client,
                        &cached_snapshot,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::OpenNetworkConnectivity => {
                    if action::detail_tabs::handle_open_network_connectivity(
                        &mut app,
                        &client,
                        &cached_snapshot,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::OpenTrafficDebug => {
                    if action::detail_tabs::handle_open_traffic_debug(
                        &mut app,
                        &client,
                        &cached_snapshot,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::OpenResourceEvents => {
                    if action::detail_tabs::handle_open_resource_events(
                        &mut app,
                        &client,
                        &cached_snapshot,
                        &detail_tx,
                        &mut detail_request_seq,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::LogsViewerOpen => {
                    let resource = app
                        .detail_view
                        .as_ref()
                        .and_then(DetailViewState::selected_logs_resource)
                        .or_else(|| {
                            app.detail_view
                                .as_ref()
                                .and_then(|detail| detail.resource.clone())
                        })
                        .or_else(|| selected_resource(&app, &cached_snapshot));
                    let Some(resource) = resource else {
                        app.set_error("No resource selected for logs.".to_string());
                        continue;
                    };
                    if let Some(message) =
                        detail_action_block_message(&app, &client, &resource, DetailAction::Logs)
                            .await
                    {
                        app.set_error(message);
                        continue;
                    }
                    let Some((pod_name, pod_ns, pod_resource)) = (match &resource {
                        ResourceRef::Pod(pod_name, pod_ns) => {
                            Some((pod_name.clone(), pod_ns.clone(), resource.clone()))
                        }
                        _ => None,
                    }) else {
                        if !matches!(
                            resource,
                            ResourceRef::Deployment(_, _)
                                | ResourceRef::StatefulSet(_, _)
                                | ResourceRef::DaemonSet(_, _)
                                | ResourceRef::ReplicaSet(_, _)
                                | ResourceRef::ReplicationController(_, _)
                                | ResourceRef::Job(_, _)
                        ) {
                            app.set_error(
                                    "Logs are only available for Pods and supported workload resources."
                                        .to_string(),
                                );
                            continue;
                        }
                        let session_id = next_workload_logs_session_id;
                        next_workload_logs_session_id =
                            next_workload_logs_session_id.wrapping_add(1).max(1);
                        app.detail_view = None;
                        app.open_workload_logs_tab(resource.clone(), session_id);
                        let tx = workload_logs_bootstrap_tx.clone();
                        let client_clone = client.get_client();
                        tokio::spawn(async move {
                            let result = resolve_workload_log_targets(client_clone, &resource)
                                .await
                                .map_err(|err| format!("{err:#}"));
                            let _ = tx
                                .send(WorkloadLogsBootstrapResult {
                                    session_id,
                                    resource,
                                    result,
                                })
                                .await;
                        });
                        continue;
                    };
                    let mut container_request: Option<(u64, String, String)> = None;
                    app.detail_view = None;
                    app.open_pod_logs_tab(pod_resource);
                    if let Some(tab) = app.workbench_mut().find_tab_mut(&WorkbenchTabKey::PodLogs(
                        ResourceRef::Pod(pod_name.clone(), pod_ns.clone()),
                    )) && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
                    {
                        logs_viewer_request_seq = logs_viewer_request_seq.wrapping_add(1);
                        let request_id = logs_viewer_request_seq;
                        logs_tab.viewer = LogsViewerState {
                            pod_name: pod_name.clone(),
                            pod_namespace: pod_ns.clone(),
                            loading: true,
                            pending_container_request_id: Some(request_id),
                            pending_logs_request_id: None,
                            container_cursor: 0,
                            container_name: String::new(),
                            containers: Vec::new(),
                            picking_container: false,
                            ..Default::default()
                        };
                        container_request = Some((request_id, pod_name, pod_ns));
                    }

                    if let Some((request_id, pod_name, pod_ns)) = container_request {
                        let client_clone = client.clone();
                        let tx = logs_viewer_tx.clone();
                        tokio::spawn(async move {
                            let pods_api: Api<Pod> =
                                Api::namespaced(client_clone.get_client(), &pod_ns);
                            let result = pods_api
                                .get(&pod_name)
                                .await
                                .map_err(|err| err.to_string())
                                .map(|pod| {
                                    pod.spec
                                        .map(|spec| {
                                            spec.containers
                                                .into_iter()
                                                .map(|container| container.name)
                                                .collect::<Vec<_>>()
                                        })
                                        .unwrap_or_default()
                                });
                            let _ = tx
                                .send(LogsViewerAsyncResult::Containers {
                                    request_id,
                                    pod_name,
                                    namespace: pod_ns,
                                    result,
                                })
                                .await;
                        });
                    }
                }
                AppAction::OpenExec => {
                    let resource = app
                        .detail_view
                        .as_ref()
                        .and_then(|detail| detail.resource.clone())
                        .or_else(|| selected_resource(&app, &cached_snapshot));
                    let Some(ResourceRef::Pod(pod_name, pod_ns)) = resource else {
                        app.set_error("Exec is only available for Pod resources.".to_string());
                        continue;
                    };
                    let resource = ResourceRef::Pod(pod_name.clone(), pod_ns.clone());
                    if let Some(message) =
                        detail_action_block_message(&app, &client, &resource, DetailAction::Exec)
                            .await
                    {
                        app.set_error(message);
                        continue;
                    }

                    if let Some(existing_session_id) = app.workbench().exec_session_id(&resource)
                        && let Some(handle) = exec_sessions.remove(&existing_session_id)
                    {
                        let _ = handle.cancel_tx.send(());
                        cleanup_node_debug_session_if_needed(
                            existing_session_id,
                            &mut node_debug_sessions,
                            &node_debug_cleanup_tx,
                        )
                        .await;
                    }

                    let session_id = next_exec_session_id;
                    next_exec_session_id = next_exec_session_id.wrapping_add(1).max(1);
                    app.detail_view = None;
                    app.open_exec_tab(
                        resource.clone(),
                        session_id,
                        pod_name.clone(),
                        pod_ns.clone(),
                    );
                    let tx = exec_bootstrap_tx.clone();
                    let client_clone = client.clone();
                    tokio::spawn(async move {
                        let result = fetch_pod_containers(&client_clone, &pod_name, &pod_ns)
                            .await
                            .map_err(|err| format!("{err:#}"));
                        let _ = tx
                            .send(ExecBootstrapResult {
                                session_id,
                                resource,
                                result,
                            })
                            .await;
                    });
                }
                AppAction::DebugContainerDialogOpen => {
                    if action::debug::handle_debug_container_dialog_open(
                        &mut app,
                        &client,
                        &debug_dialog_bootstrap_tx,
                        &mut debug_dialog_request_seq,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::DebugContainerDialogSubmit => {
                    if action::debug::handle_debug_container_dialog_submit(
                        &mut app,
                        &client,
                        &debug_launch_tx,
                        &mut next_exec_session_id,
                        refresh_state.context_generation,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::NodeDebugDialogOpen => {
                    app.set_available_namespaces(global_state.namespaces().to_vec());
                    if action::node_debug::handle_node_debug_dialog_open(&mut app) {
                        continue;
                    }
                }
                AppAction::NodeDebugDialogSubmit => {
                    if action::node_debug::handle_node_debug_dialog_submit(
                        &mut app,
                        &client,
                        &node_debug_launch_tx,
                        &mut next_exec_session_id,
                        refresh_state.context_generation,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::ExecSelectContainer(container_name) => {
                    let mut start_session: Option<(u64, ResourceRef, String, String, String)> =
                        None;
                    if let Some(tab) = app.workbench_mut().active_tab_mut()
                        && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
                    {
                        exec_tab.picking_container = false;
                        exec_tab.container_name = container_name.clone();
                        exec_tab.loading = true;
                        exec_tab.error = None;
                        start_session = Some((
                            exec_tab.session_id,
                            exec_tab.resource.clone(),
                            exec_tab.pod_name.clone(),
                            exec_tab.namespace.clone(),
                            container_name,
                        ));
                    }
                    if let Some((session_id, resource, pod_name, namespace, container_name)) =
                        start_session
                    {
                        match spawn_exec_session(
                            client.clone(),
                            session_id,
                            pod_name,
                            namespace,
                            container_name.clone(),
                            exec_update_tx.clone(),
                        )
                        .await
                        {
                            Ok(handle) => {
                                exec_sessions.insert(session_id, handle);
                            }
                            Err(err) => {
                                if let Some(tab) = app
                                    .workbench_mut()
                                    .find_tab_mut(&WorkbenchTabKey::Exec(resource))
                                    && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
                                {
                                    exec_tab.loading = false;
                                    exec_tab.error = Some(format!("{err:#}"));
                                }
                            }
                        }
                    }
                }
                AppAction::ExecSendInput => {
                    if let Some(tab) = app.workbench_mut().active_tab_mut()
                        && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
                    {
                        if exec_tab.input.is_empty() {
                            continue;
                        }
                        let mut bytes = exec_tab.input.clone().into_bytes();
                        bytes.push(b'\n');
                        let session_id = exec_tab.session_id;
                        exec_tab.input.clear();
                        if let Some(handle) = exec_sessions.get(&session_id) {
                            if let Err(err) = handle.input_tx.send(bytes).await {
                                exec_tab.error = Some(format!("failed to send exec input: {err}"));
                            }
                        } else {
                            exec_tab.error = Some(
                                "exec session is not running for the selected tab.".to_string(),
                            );
                        }
                    }
                }
                AppAction::LogsViewerSelectContainer(container) => {
                    let mut logs_request: Option<(u64, String, String, String)> = None;
                    if let Some(tab) = app.workbench_mut().active_tab_mut()
                        && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
                    {
                        let viewer = &mut logs_tab.viewer;
                        logs_viewer_request_seq = logs_viewer_request_seq.wrapping_add(1);
                        let request_id = logs_viewer_request_seq;
                        viewer.picking_container = false;
                        viewer.container_name = container.clone();
                        viewer.loading = true;
                        viewer.lines.clear();
                        viewer.scroll_offset = 0;
                        viewer.error = None;
                        viewer.pending_logs_request_id = Some(request_id);
                        logs_request = Some((
                            request_id,
                            viewer.pod_name.clone(),
                            viewer.pod_namespace.clone(),
                            container,
                        ));
                    }

                    if let Some((request_id, pod_name, pod_ns, container_name)) = logs_request {
                        let client_clone = client.clone();
                        let tx = logs_viewer_tx.clone();
                        tokio::spawn(async move {
                            let logs_client = LogsClient::new(client_clone.get_client());
                            let pod_ref = PodRef::new(pod_name.clone(), pod_ns.clone());
                            let result = logs_client
                                .tail_logs(&pod_ref, Some(500), Some(container_name.as_str()))
                                .await
                                .map_err(|err| err.to_string());
                            let _ = tx
                                .send(LogsViewerAsyncResult::Tail {
                                    request_id,
                                    pod_name,
                                    namespace: pod_ns,
                                    container_name,
                                    result,
                                })
                                .await;
                        });
                    }
                }
                AppAction::LogsViewerSelectAllContainers => {
                    // Gather pod info from the current PodLogs tab, then replace
                    // it with a WorkloadLogs tab streaming all containers.
                    let all_info: Option<AllContainerLogsInfo> =
                        app.workbench().active_tab().and_then(|tab| {
                            if let WorkbenchTabState::PodLogs(logs_tab) = &tab.state {
                                let v = &logs_tab.viewer;
                                let labels = cached_snapshot
                                    .pods
                                    .iter()
                                    .find(|pod| {
                                        pod.name == v.pod_name && pod.namespace == v.pod_namespace
                                    })
                                    .map(|pod| pod.labels.clone())
                                    .unwrap_or_default();
                                Some((
                                    v.pod_name.clone(),
                                    v.pod_namespace.clone(),
                                    v.containers.clone(),
                                    labels,
                                    logs_tab.resource.clone(),
                                ))
                            } else {
                                None
                            }
                        });

                    if let Some((pod_name, pod_ns, containers, labels, resource)) = all_info {
                        // Close the single-container PodLogs tab
                        app.workbench.close_active_tab();

                        // Open a WorkloadLogs tab for the same pod
                        let session_id = next_workload_logs_session_id;
                        next_workload_logs_session_id =
                            next_workload_logs_session_id.wrapping_add(1).max(1);
                        app.open_workload_logs_tab(resource, session_id);

                        // Build sources: all containers from this single pod
                        let sources: Vec<(String, String, String)> = containers
                            .iter()
                            .map(|c| (pod_name.clone(), pod_ns.clone(), c.clone()))
                            .collect();

                        // Configure the tab directly
                        if let Some(tab) = app.workbench_mut().active_tab_mut()
                            && let WorkbenchTabState::WorkloadLogs(logs_tab) = &mut tab.state
                            && logs_tab.session_id == session_id
                        {
                            logs_tab.update_targets(&[WorkloadLogTarget {
                                pod_name: pod_name.clone(),
                                namespace: pod_ns.clone(),
                                containers: containers.clone(),
                                labels,
                            }]);
                            logs_tab.sources = sources.clone();
                            logs_tab.loading = false;
                            workload_log_sessions.insert(session_id, sources.clone());
                        }

                        // Start streaming each container
                        for (pod, ns, container) in sources {
                            let _ = coordinator
                                .start_log_streaming(pod, ns, container, true, false, false)
                                .await;
                        }
                    }
                }
                AppAction::LogsViewerToggleFollow => {
                    let follow_info = app.workbench().active_tab().and_then(|tab| {
                        if let WorkbenchTabState::PodLogs(logs_tab) = &tab.state {
                            let v = &logs_tab.viewer;
                            Some((
                                v.pod_name.clone(),
                                v.pod_namespace.clone(),
                                v.container_name.clone(),
                                v.follow_mode,
                                v.picking_container,
                                v.show_timestamps,
                            ))
                        } else {
                            None
                        }
                    });
                    if let Some((
                        pod_name,
                        pod_ns,
                        container_name,
                        was_following,
                        picking_container,
                        timestamps,
                    )) = follow_info
                    {
                        if !was_following
                            && (pod_name.is_empty()
                                || container_name.is_empty()
                                || picking_container)
                        {
                            if let Some(tab) = app.workbench_mut().active_tab_mut()
                                && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
                            {
                                let viewer = &mut logs_tab.viewer;
                                viewer.error = Some(
                                    "Select a container before enabling follow mode.".to_string(),
                                );
                            }
                        } else {
                            apply_action(AppAction::LogsViewerToggleFollow, &mut app);
                            if !was_following {
                                let _ = coordinator
                                    .start_log_streaming(
                                        pod_name,
                                        pod_ns,
                                        container_name,
                                        true,
                                        false,
                                        timestamps,
                                    )
                                    .await;
                            } else if !pod_name.is_empty() && !container_name.is_empty() {
                                let _ = coordinator
                                    .stop_log_streaming(&pod_name, &pod_ns, &container_name)
                                    .await;
                            }
                        }
                    }
                }
                AppAction::LogsViewerTogglePrevious => {
                    let prev_info = app.workbench().active_tab().and_then(|tab| {
                        if let WorkbenchTabState::PodLogs(logs_tab) = &tab.state {
                            let v = &logs_tab.viewer;
                            if v.picking_container || v.container_name.is_empty() {
                                return None;
                            }
                            Some((
                                v.pod_name.clone(),
                                v.pod_namespace.clone(),
                                v.container_name.clone(),
                                v.previous_logs,
                                v.follow_mode,
                            ))
                        } else {
                            None
                        }
                    });
                    if let Some((pod_name, pod_ns, container_name, was_previous, was_following)) =
                        prev_info
                    {
                        // Cancel any current log stream
                        if was_following || was_previous {
                            let _ = coordinator
                                .stop_log_streaming(&pod_name, &pod_ns, &container_name)
                                .await;
                        }

                        // Toggle previous_logs and reset viewer state
                        if let Some(tab) = app.workbench_mut().active_tab_mut()
                            && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
                        {
                            let viewer = &mut logs_tab.viewer;
                            viewer.previous_logs = !was_previous;
                            viewer.follow_mode = false;
                            viewer.lines.clear();
                            viewer.scroll_offset = 0;
                            viewer.loading = true;
                            viewer.error = None;

                            logs_viewer_request_seq = logs_viewer_request_seq.wrapping_add(1);
                            let request_id = logs_viewer_request_seq;
                            viewer.pending_logs_request_id = Some(request_id);

                            let new_previous = viewer.previous_logs;
                            let tx = logs_viewer_tx.clone();
                            let client_clone = client.clone();
                            let pn = pod_name.clone();
                            let pns = pod_ns.clone();
                            let cn = container_name.clone();
                            tokio::spawn(async move {
                                let logs_client = LogsClient::new(client_clone.get_client());
                                let pod_ref = PodRef::new(pn.clone(), pns.clone());
                                let tail = Some(500);
                                let result = if new_previous {
                                    logs_client
                                        .tail_previous_logs(&pod_ref, tail, Some(cn.as_str()))
                                        .await
                                        .map_err(|err| err.to_string())
                                } else {
                                    logs_client
                                        .tail_logs(&pod_ref, tail, Some(cn.as_str()))
                                        .await
                                        .map_err(|err| err.to_string())
                                };
                                let _ = tx
                                    .send(LogsViewerAsyncResult::Tail {
                                        request_id,
                                        pod_name: pn,
                                        namespace: pns,
                                        container_name: cn,
                                        result,
                                    })
                                    .await;
                            });
                        }
                    }
                }
                AppAction::LogsViewerToggleTimestamps => {
                    let ts_info = app.workbench().active_tab().and_then(|tab| {
                        if let WorkbenchTabState::PodLogs(logs_tab) = &tab.state {
                            let v = &logs_tab.viewer;
                            if v.picking_container || v.container_name.is_empty() {
                                return None;
                            }
                            Some((
                                v.pod_name.clone(),
                                v.pod_namespace.clone(),
                                v.container_name.clone(),
                                v.show_timestamps,
                                v.follow_mode,
                                v.previous_logs,
                            ))
                        } else {
                            None
                        }
                    });
                    if let Some((
                        pod_name,
                        pod_ns,
                        container_name,
                        was_timestamps,
                        was_following,
                        is_previous,
                    )) = ts_info
                    {
                        if was_following || was_timestamps {
                            let _ = coordinator
                                .stop_log_streaming(&pod_name, &pod_ns, &container_name)
                                .await;
                        }
                        if let Some(tab) = app.workbench_mut().active_tab_mut()
                            && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
                        {
                            let viewer = &mut logs_tab.viewer;
                            viewer.show_timestamps = !viewer.show_timestamps;
                            viewer.lines.clear();
                            viewer.scroll_offset = 0;
                            viewer.loading = true;
                            viewer.error = None;
                        }
                        let new_timestamps = !was_timestamps;
                        let follow = !is_previous;
                        let _ = coordinator
                            .start_log_streaming(
                                pod_name,
                                pod_ns,
                                container_name,
                                follow,
                                is_previous,
                                new_timestamps,
                            )
                            .await;
                    }
                }
                AppAction::PortForwardOpen => {
                    let resource = app
                        .detail_view
                        .as_ref()
                        .and_then(|detail| detail.resource.clone())
                        .or_else(|| selected_resource(&app, &cached_snapshot));
                    let dialog = match &resource {
                        Some(ResourceRef::Pod(name, ns)) => {
                            PortForwardDialog::with_target(ns, name, 0)
                        }
                        _ => {
                            app.set_error(
                                "Port forwarding is only available for Pod resources.".to_string(),
                            );
                            continue;
                        }
                    };
                    let Some(resource) = resource else {
                        continue;
                    };
                    if let Some(message) = detail_action_block_message(
                        &app,
                        &client,
                        &resource,
                        DetailAction::PortForward,
                    )
                    .await
                    {
                        app.set_error(message);
                        continue;
                    }
                    app.detail_view = None;
                    app.open_port_forward_tab(Some(resource), dialog);
                    let tunnels = port_forwarder.list_tunnels();
                    app.tunnel_registry.update_tunnels(tunnels.clone());
                    if let Some(tab) = app
                        .workbench_mut()
                        .find_tab_mut(&WorkbenchTabKey::PortForward)
                        && let WorkbenchTabState::PortForward(port_tab) = &mut tab.state
                    {
                        let mut registry = kubectui::state::port_forward::TunnelRegistry::new();
                        registry.update_tunnels(tunnels);
                        port_tab.dialog.update_registry(registry);
                    }
                }
                AppAction::ScaleDialogSubmit => {
                    if action::scale::handle_scale_dialog_submit(
                        &mut app,
                        &client,
                        &scale_tx,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::RolloutRestart => {
                    if action::rollout::handle_rollout_restart(
                        &mut app,
                        &client,
                        &rollout_tx,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::DeleteResource => {
                    if action::delete::handle_delete_resource(
                        &mut app,
                        &client,
                        &delete_tx,
                        &mut delete_request_seq,
                        &mut delete_in_flight_id,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::ForceDeleteResource => {
                    if action::delete::handle_force_delete_resource(
                        &mut app,
                        &client,
                        &delete_tx,
                        &mut delete_request_seq,
                        &mut delete_in_flight_id,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::TriggerCronJob => {
                    if action::cronjob::handle_trigger_cronjob(
                        &mut app,
                        &client,
                        &trigger_cronjob_tx,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::ConfirmCronJobSuspend(suspend) => {
                    action::cronjob::handle_confirm_cronjob_suspend(&mut app, suspend);
                }
                AppAction::SetCronJobSuspend(suspend) => {
                    if action::cronjob::handle_set_cronjob_suspend(
                        &mut app,
                        &client,
                        &cronjob_suspend_tx,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                        suspend,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::ConfirmDrainNode => {
                    action::node_ops::handle_confirm_drain_node(&mut app);
                }
                AppAction::ConfirmRolloutUndo => {
                    if action::rollout::handle_confirm_rollout_undo(&mut app) {
                        continue;
                    }
                }
                AppAction::ConfirmHelmRollback => {
                    if action::helm::handle_confirm_helm_rollback(&mut app) {
                        continue;
                    }
                }
                AppAction::CordonNode if !node_op_in_flight => {
                    if action::node_ops::handle_cordon_node(
                        &mut app,
                        &client,
                        &node_ops_tx,
                        &mut node_op_in_flight,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::CordonNode => {} // in-flight guard
                AppAction::UncordonNode if !node_op_in_flight => {
                    if action::node_ops::handle_uncordon_node(
                        &mut app,
                        &client,
                        &node_ops_tx,
                        &mut node_op_in_flight,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::UncordonNode => {} // in-flight guard
                AppAction::DrainNode | AppAction::ForceDrainNode if !node_op_in_flight => {
                    let force = matches!(action, AppAction::ForceDrainNode);
                    if action::node_ops::handle_drain_node(
                        &mut app,
                        &client,
                        &node_ops_tx,
                        &mut node_op_in_flight,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                        force,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::DrainNode | AppAction::ForceDrainNode => {
                    action::node_ops::handle_drain_in_flight_guard(&mut app);
                }
                AppAction::ExecuteHelmRollback => {
                    if action::helm::handle_execute_helm_rollback(
                        &mut app,
                        &helm_rollback_tx,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    ) {
                        continue;
                    }
                }
                AppAction::ToggleRolloutPauseResume => {
                    if action::rollout::handle_toggle_rollout_pause_resume(
                        &mut app,
                        &client,
                        &rollout_tx,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::ExecuteRolloutUndo => {
                    if action::rollout::handle_execute_rollout_undo(
                        &mut app,
                        &client,
                        &rollout_tx,
                        refresh_state.context_generation,
                        &mut status_message_clear_at,
                    )
                    .await
                    {
                        continue;
                    }
                }
                AppAction::CopyResourceName => {
                    action::copy_export::copy_resource_name(&mut app, &cached_snapshot);
                }
                AppAction::CopyResourceFullName => {
                    action::copy_export::copy_resource_full_name(&mut app, &cached_snapshot);
                }
                AppAction::CopyLogContent => {
                    action::copy_export::copy_log_content(&mut app);
                }
                AppAction::ExportLogs => {
                    action::copy_export::export_logs(&mut app);
                }
                AppAction::EditYaml => {
                    if !app
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::EditYaml))
                    {
                        app.set_error(
                            "YAML editing is unavailable for the selected resource.".to_string(),
                        );
                        continue;
                    }
                    // Gather what we need before suspending the TUI
                    let edit_info = app.detail_view.as_ref().and_then(|d| {
                        d.resource.as_ref().zip(d.yaml.as_ref()).map(|(r, y)| {
                            (
                                r.clone(),
                                r.kind().to_ascii_lowercase(),
                                r.name().to_string(),
                                r.namespace().map(str::to_owned),
                                y.clone(),
                            )
                        })
                    });

                    if let Some((resource, kind, name, namespace, yaml_content)) = edit_info {
                        if let Some(message) = detail_action_block_message(
                            &app,
                            &client,
                            &resource,
                            DetailAction::EditYaml,
                        )
                        .await
                        {
                            app.set_error(message);
                            continue;
                        }
                        match edit_yaml_in_external_editor(
                            terminal,
                            &format!("{kind}-{name}"),
                            &yaml_content,
                            true,
                        ) {
                            Err(err) => app.set_error(format!("{err:#}")),
                            Ok(None) => {}
                            Ok(Some(edited_yaml)) => {
                                let origin_view = app.view();
                                let resource_label = format!(
                                    "{} '{}'{}",
                                    kind,
                                    name,
                                    namespace
                                        .as_deref()
                                        .map(|ns| format!(" in namespace '{ns}'"))
                                        .unwrap_or_default()
                                );
                                let jump_resource = app
                                    .detail_view
                                    .as_ref()
                                    .and_then(|detail| detail.resource.clone());
                                let action_history_id = app.record_action_pending(
                                    ActionKind::ApplyYaml,
                                    origin_view,
                                    jump_resource,
                                    resource_label.clone(),
                                    format!("Applying changes to {resource_label}..."),
                                );
                                match client
                                    .apply_resource_yaml(
                                        &edited_yaml,
                                        &kind,
                                        &name,
                                        namespace.as_deref(),
                                    )
                                    .await
                                {
                                    Ok(()) => {
                                        app.complete_action_history(
                                            action_history_id,
                                            ActionStatus::Succeeded,
                                            format!("Applied changes to {resource_label}."),
                                            true,
                                        );
                                        app.detail_view = None;
                                        app.focus = kubectui::app::Focus::Content;
                                        apply_mutation_success(
                                            &mut app,
                                            &mut MutationRuntime {
                                                global_state: &mut global_state,
                                                client: &client,
                                                refresh_tx: &refresh_tx,
                                                deferred_refresh_tx: &deferred_refresh_tx,
                                                refresh_state: &mut refresh_state,
                                                snapshot_dirty: &mut snapshot_dirty,
                                                auto_refresh: &mut auto_refresh,
                                                status_message_clear_at:
                                                    &mut status_message_clear_at,
                                            },
                                            origin_view,
                                            format!(
                                                "Applied changes to {} '{}'. Refreshing view...",
                                                kind, name
                                            ),
                                            false,
                                            MUTATION_REFRESH_DELAYS_SECS,
                                        );
                                    }
                                    Err(err) => {
                                        app.complete_action_history(
                                            action_history_id,
                                            ActionStatus::Failed,
                                            format!("Apply failed: {err:#}"),
                                            true,
                                        );
                                        app.set_error(format!("Apply failed: {err:#}"));
                                    }
                                }
                            }
                        }
                    }
                }
                AppAction::SubmitResourceTemplateDialog => {
                    let Some(dialog) = app.resource_template_dialog.clone() else {
                        app.set_error("No resource template dialog is open.".to_string());
                        continue;
                    };
                    let validated = match dialog.values.validate() {
                        Ok(validated) => validated,
                        Err(err) => {
                            if let Some(state) = &mut app.resource_template_dialog {
                                state.error_message = Some(err.to_string());
                            }
                            continue;
                        }
                    };
                    let rendered_yaml = match validated.render_yaml() {
                        Ok(yaml) => yaml,
                        Err(err) => {
                            let message = format!("Failed to render template: {err:#}");
                            if let Some(state) = &mut app.resource_template_dialog {
                                state.error_message = Some(message.clone());
                            }
                            app.set_error(message);
                            continue;
                        }
                    };
                    let file_stem = format!(
                        "template-{}-{}",
                        validated
                            .kind
                            .label()
                            .to_ascii_lowercase()
                            .replace(' ', "-"),
                        validated.name
                    );
                    let edited_yaml = match edit_yaml_in_external_editor(
                        terminal,
                        &file_stem,
                        &rendered_yaml,
                        false,
                    ) {
                        Ok(Some(edited_yaml)) => edited_yaml,
                        Ok(None) => {
                            app.resource_template_dialog = None;
                            continue;
                        }
                        Err(err) => {
                            let message = format!("{err:#}");
                            if let Some(state) = &mut app.resource_template_dialog {
                                state.error_message = Some(message.clone());
                            }
                            app.set_error(message);
                            continue;
                        }
                    };

                    let origin_view = app.view();
                    let resource_label = format!(
                        "{} '{}' in namespace '{}'",
                        validated.kind.label(),
                        validated.name,
                        validated.namespace
                    );
                    let action_history_id = app.record_action_pending(
                        ActionKind::ApplyYaml,
                        origin_view,
                        None,
                        resource_label.clone(),
                        format!("Applying {resource_label}..."),
                    );
                    match client.apply_yaml_documents(&edited_yaml).await {
                        Ok(document_count) => {
                            app.complete_action_history(
                                action_history_id,
                                ActionStatus::Succeeded,
                                format!(
                                    "Applied {document_count} manifest(s) for {resource_label}."
                                ),
                                true,
                            );
                            app.resource_template_dialog = None;
                            apply_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                origin_view,
                                format!("Applied {resource_label}. Refreshing view..."),
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            let message = format!("Apply failed: {err:#}");
                            app.complete_action_history(
                                action_history_id,
                                ActionStatus::Failed,
                                message.clone(),
                                true,
                            );
                            if let Some(state) = &mut app.resource_template_dialog {
                                state.error_message = Some(message.clone());
                            }
                            app.set_error(message);
                        }
                    }
                }
                AppAction::SaveDecodedSecret => {
                    let decoded_state =
                        app.workbench()
                            .active_tab()
                            .and_then(|tab| match &tab.state {
                                WorkbenchTabState::DecodedSecret(secret_tab) => Some((
                                    secret_tab.resource.clone(),
                                    secret_tab.source_yaml.clone(),
                                    secret_tab.entries.clone(),
                                )),
                                _ => None,
                            });

                    let Some((resource, source_yaml, entries)) = decoded_state else {
                        app.set_error(
                            "Decoded Secret save is only available from the decoded Secret tab."
                                .to_string(),
                        );
                        continue;
                    };
                    let Some(source_yaml) = source_yaml else {
                        app.set_error("Secret YAML is not loaded yet.".to_string());
                        continue;
                    };
                    let encoded_yaml = match encode_secret_yaml(&source_yaml, &entries) {
                        Ok(encoded_yaml) => encoded_yaml,
                        Err(err) => {
                            app.set_error(format!("Failed to encode Secret data: {err:#}"));
                            continue;
                        }
                    };
                    let kind = resource.kind().to_ascii_lowercase();
                    let name = resource.name().to_string();
                    let namespace = resource.namespace().map(str::to_owned);
                    let origin_view = app.view();
                    let resource_label = format!(
                        "{} '{}'{}",
                        resource.kind(),
                        name,
                        namespace
                            .as_deref()
                            .map(|ns| format!(" in namespace '{ns}'"))
                            .unwrap_or_default()
                    );
                    let action_history_id = app.record_action_pending(
                        ActionKind::ApplyYaml,
                        origin_view,
                        Some(resource.clone()),
                        resource_label.clone(),
                        format!("Applying decoded Secret changes to {resource_label}..."),
                    );
                    if let Some(message) = detail_action_block_message(
                        &app,
                        &client,
                        &resource,
                        DetailAction::EditYaml,
                    )
                    .await
                    {
                        app.set_error(message.clone());
                        app.complete_action_history(
                            action_history_id,
                            ActionStatus::Failed,
                            message,
                            true,
                        );
                        continue;
                    }
                    match client
                        .apply_resource_yaml(&encoded_yaml, &kind, &name, namespace.as_deref())
                        .await
                    {
                        Ok(()) => {
                            app.complete_action_history(
                                action_history_id,
                                ActionStatus::Succeeded,
                                format!("Applied decoded Secret changes to {resource_label}."),
                                true,
                            );
                            if let Some(tab) = app
                                .workbench_mut()
                                .find_tab_mut(&WorkbenchTabKey::DecodedSecret(resource.clone()))
                                && let WorkbenchTabState::DecodedSecret(secret_tab) = &mut tab.state
                            {
                                match decode_secret_yaml(&encoded_yaml) {
                                    Ok(decoded_entries) => {
                                        secret_tab.source_yaml = Some(encoded_yaml.clone());
                                        secret_tab.entries = decoded_entries;
                                        secret_tab.editing = false;
                                        secret_tab.edit_input.clear();
                                        secret_tab.error = None;
                                        secret_tab.loading = false;
                                        secret_tab.clamp_selected();
                                    }
                                    Err(err) => {
                                        secret_tab.error = Some(err.to_string());
                                    }
                                }
                            }
                            apply_mutation_success(
                                &mut app,
                                &mut MutationRuntime {
                                    global_state: &mut global_state,
                                    client: &client,
                                    refresh_tx: &refresh_tx,
                                    deferred_refresh_tx: &deferred_refresh_tx,
                                    refresh_state: &mut refresh_state,
                                    snapshot_dirty: &mut snapshot_dirty,
                                    auto_refresh: &mut auto_refresh,
                                    status_message_clear_at: &mut status_message_clear_at,
                                },
                                origin_view,
                                format!(
                                    "Applied decoded Secret changes to {} '{}'. Refreshing view...",
                                    resource.kind(),
                                    name
                                ),
                                false,
                                MUTATION_REFRESH_DELAYS_SECS,
                            );
                        }
                        Err(err) => {
                            app.complete_action_history(
                                action_history_id,
                                ActionStatus::Failed,
                                format!("Apply failed: {err:#}"),
                                true,
                            );
                            app.set_error(format!("Apply failed: {err:#}"));
                        }
                    }
                }
                AppAction::PortForwardCreate((target, config)) => {
                    match port_forwarder.create_tunnel_async(target, config).await {
                        Ok(tunnel_id) => {
                            app.clear_error();
                            let tunnels = port_forwarder.list_tunnels();
                            app.tunnel_registry.update_tunnels(tunnels.clone());
                            if let Some(tab) = app
                                .workbench_mut()
                                .find_tab_mut(&WorkbenchTabKey::PortForward)
                                && let WorkbenchTabState::PortForward(port_tab) = &mut tab.state
                            {
                                port_tab.dialog.success =
                                    Some(format!("Tunnel created: {tunnel_id}"));
                                let mut registry =
                                    kubectui::state::port_forward::TunnelRegistry::new();
                                registry.update_tunnels(tunnels);
                                port_tab.dialog.update_registry(registry);
                            }
                        }
                        Err(err) => {
                            if let Some(tab) = app
                                .workbench_mut()
                                .find_tab_mut(&WorkbenchTabKey::PortForward)
                                && let WorkbenchTabState::PortForward(port_tab) = &mut tab.state
                            {
                                port_tab.dialog.error = Some(format!("{err}"));
                            }
                        }
                    }
                }
                AppAction::PortForwardRefresh => {
                    refresh_port_forward_workbench(
                        &mut app,
                        &port_forwarder,
                        &mut status_message_clear_at,
                    );
                }
                AppAction::PortForwardStop(tunnel_id) => {
                    match port_forwarder.stop_forward(&tunnel_id).await {
                        Ok(()) => {
                            let tunnels = port_forwarder.list_tunnels();
                            app.tunnel_registry.update_tunnels(tunnels.clone());
                            if let Some(tab) = app
                                .workbench_mut()
                                .find_tab_mut(&WorkbenchTabKey::PortForward)
                                && let WorkbenchTabState::PortForward(port_tab) = &mut tab.state
                            {
                                let mut registry =
                                    kubectui::state::port_forward::TunnelRegistry::new();
                                registry.update_tunnels(tunnels);
                                port_tab.dialog.success =
                                    Some(format!("Closed tunnel: {tunnel_id}"));
                                port_tab.dialog.update_registry(registry);
                            }
                        }
                        Err(err) => {
                            if let Some(tab) = app
                                .workbench_mut()
                                .find_tab_mut(&WorkbenchTabKey::PortForward)
                                && let WorkbenchTabState::PortForward(port_tab) = &mut tab.state
                            {
                                port_tab.dialog.error = Some(format!("{err:#}"));
                            }
                        }
                    }
                }
                AppAction::ScaleDialogOpen => {
                    if action::scale::handle_scale_dialog_open(&mut app, &cached_snapshot) {
                        continue;
                    }
                }
                AppAction::ProbePanelOpen => {
                    if !app
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::Probes))
                    {
                        app.set_error(
                            "Probe inspection is only available for Pod resources.".to_string(),
                        );
                        continue;
                    }
                    let pod_info = app.detail_view.as_ref().and_then(|d| {
                        d.resource.as_ref().and_then(|r| match r {
                            ResourceRef::Pod(name, ns) => Some((name.clone(), ns.clone())),
                            _ => None,
                        })
                    });
                    if let Some((pod_name, pod_ns)) = pod_info {
                        let tx = probe_tx.clone();
                        let k = client.get_client();
                        let resource = ResourceRef::Pod(pod_name.clone(), pod_ns.clone());
                        if let Some(message) = detail_action_block_message(
                            &app,
                            &client,
                            &resource,
                            DetailAction::Probes,
                        )
                        .await
                        {
                            app.set_error(message);
                            continue;
                        }
                        tokio::spawn(async move {
                            let pods_api: Api<Pod> = Api::namespaced(k, &pod_ns);
                            let result = match pods_api.get(&pod_name).await {
                                Ok(pod) => Ok(extract_probes_from_pod(&pod).unwrap_or_default()),
                                Err(err) => Err(format!("Failed to load probes: {err}")),
                            };
                            let _ = tx.send(ProbeAsyncResult { resource, result }).await;
                        });
                    } else {
                        apply_action(AppAction::ProbePanelOpen, &mut app);
                    }
                }
                AppAction::CycleTheme => {
                    apply_action(AppAction::CycleTheme, &mut app);
                    app.needs_config_save = true;
                }
                AppAction::CycleIconMode => {
                    apply_action(AppAction::CycleIconMode, &mut app);
                    app.needs_config_save = true;
                }
                other => {
                    apply_action(other, &mut app);
                }
            }

            // Clear stale deferred palette action if detail view was closed or errored
            if pending_palette_action.is_some()
                && (app.detail_view.is_none()
                    || app.detail_view.as_ref().is_some_and(|d| d.error.is_some()))
            {
                pending_palette_action = None;
            }

            // Persist preferences when dirty flag is set (debounced to at most once per second)
            if app.needs_config_save {
                let should_save =
                    last_config_save.is_none_or(|t| t.elapsed() >= Duration::from_secs(1));
                if should_save {
                    app.needs_config_save = false;
                    save_config(&app);
                    last_config_save = Some(Instant::now());
                }
            }
        }
    }

    // Final flush: persist any pending config changes before exit
    if app.needs_config_save {
        save_config(&app);
    }

    for (_, handle) in exec_sessions.drain() {
        let _ = handle.cancel_tx.send(());
    }
    for (_, session) in node_debug_sessions.drain() {
        let _ = session
            .client
            .delete_node_debug_pod(&session.namespace, &session.pod_name)
            .await;
    }
    let _ = coordinator.shutdown().await;
    port_forwarder.stop_all().await;

    Ok(())
}

#[cfg(test)]
mod main_tests;
