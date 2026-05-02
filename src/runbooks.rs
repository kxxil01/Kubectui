//! Guided runbooks and incident packs built on workspace, command extension, and native AI paths.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{ai_actions::AiWorkflowKind, app::ResourceRef, policy::DetailAction};

const RUNBOOKS_FILE_NAME: &str = "runbooks.yaml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunbookWorkspaceTarget {
    SavedWorkspace,
    WorkspaceBank,
}

impl RunbookWorkspaceTarget {
    pub const fn label(self) -> &'static str {
        match self {
            Self::SavedWorkspace => "Workspace",
            Self::WorkspaceBank => "Bank",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunbookDetailAction {
    ViewYaml,
    ViewConfigDrift,
    ViewRollout,
    ViewHelmHistory,
    ViewEvents,
    Logs,
    Exec,
    PortForward,
    Probes,
    ViewNetworkPolicies,
    CheckNetworkConnectivity,
    ViewTrafficDebug,
    ViewRelationships,
}

impl RunbookDetailAction {
    pub const fn into_detail_action(self) -> DetailAction {
        match self {
            Self::ViewYaml => DetailAction::ViewYaml,
            Self::ViewConfigDrift => DetailAction::ViewConfigDrift,
            Self::ViewRollout => DetailAction::ViewRollout,
            Self::ViewHelmHistory => DetailAction::ViewHelmHistory,
            Self::ViewEvents => DetailAction::ViewEvents,
            Self::Logs => DetailAction::Logs,
            Self::Exec => DetailAction::Exec,
            Self::PortForward => DetailAction::PortForward,
            Self::Probes => DetailAction::Probes,
            Self::ViewNetworkPolicies => DetailAction::ViewNetworkPolicies,
            Self::CheckNetworkConnectivity => DetailAction::CheckNetworkConnectivity,
            Self::ViewTrafficDebug => DetailAction::ViewTrafficDebug,
            Self::ViewRelationships => DetailAction::ViewRelationships,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::ViewYaml => "Open YAML",
            Self::ViewConfigDrift => "Open Drift View",
            Self::ViewRollout => "Open Rollout Center",
            Self::ViewHelmHistory => "Open Helm History",
            Self::ViewEvents => "Open Events",
            Self::Logs => "Open Logs",
            Self::Exec => "Open Exec",
            Self::PortForward => "Open Port Forward",
            Self::Probes => "Open Probes",
            Self::ViewNetworkPolicies => "Open Network Policy View",
            Self::CheckNetworkConnectivity => "Open Connectivity Check",
            Self::ViewTrafficDebug => "Open Traffic Debug",
            Self::ViewRelationships => "Open Relationships",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunbookStepConfig {
    Checklist {
        title: String,
        #[serde(default)]
        description: Option<String>,
        items: Vec<String>,
    },
    Workspace {
        title: String,
        #[serde(default)]
        description: Option<String>,
        name: String,
        #[serde(default)]
        target: Option<RunbookWorkspaceTarget>,
    },
    DetailAction {
        title: String,
        #[serde(default)]
        description: Option<String>,
        action: RunbookDetailAction,
    },
    ExtensionAction {
        title: String,
        #[serde(default)]
        description: Option<String>,
        action_id: String,
    },
    AiWorkflow {
        title: String,
        #[serde(default)]
        description: Option<String>,
        workflow: AiWorkflowKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunbookConfig {
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
    pub steps: Vec<RunbookStepConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunbooksConfig {
    #[serde(default)]
    pub runbooks: Vec<RunbookConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadedRunbookStepKind {
    Checklist {
        items: Vec<String>,
    },
    Workspace {
        name: String,
        target: RunbookWorkspaceTarget,
    },
    DetailAction {
        action: RunbookDetailAction,
    },
    ExtensionAction {
        action_id: String,
    },
    AiWorkflow {
        workflow: AiWorkflowKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedRunbookStep {
    pub title: String,
    pub description: Option<String>,
    pub kind: LoadedRunbookStepKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedRunbook {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub aliases: Vec<String>,
    pub resource_kinds: Vec<String>,
    pub shortcut: Option<String>,
    pub steps: Vec<LoadedRunbookStep>,
}

impl LoadedRunbook {
    pub fn matches_resource(&self, resource: Option<&ResourceRef>) -> bool {
        if self.resource_kinds.is_empty() {
            return true;
        }
        let Some(resource) = resource else {
            return false;
        };
        self.resource_kinds
            .iter()
            .any(|kind| kind == "*" || kind.eq_ignore_ascii_case(resource.kind()))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunbookRegistry {
    runbooks: Vec<LoadedRunbook>,
}

impl RunbookRegistry {
    pub fn runbooks(&self) -> &[LoadedRunbook] {
        &self.runbooks
    }

    pub fn get(&self, id: &str) -> Option<&LoadedRunbook> {
        self.runbooks.iter().find(|runbook| runbook.id == id)
    }

    pub fn palette_runbooks_for(&self, resource: Option<&ResourceRef>) -> Vec<LoadedRunbook> {
        self.runbooks
            .iter()
            .filter(|runbook| runbook.matches_resource(resource))
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunbookLoadResult {
    pub registry: RunbookRegistry,
    pub warnings: Vec<String>,
    pub path: PathBuf,
}

pub fn runbooks_config_path() -> Option<PathBuf> {
    dirs::config_dir()
        .or_else(dirs::home_dir)
        .map(|base| base.join("kubectui").join(RUNBOOKS_FILE_NAME))
}

pub fn load_runbooks_config_from_path(path: &Path) -> Result<RunbooksConfig, String> {
    let content = fs::read_to_string(path)
        .map_err(|err| format!("failed to read runbooks config '{}': {err}", path.display()))?;
    serde_yaml::from_str::<RunbooksConfig>(&content).map_err(|err| {
        format!(
            "failed to parse runbooks config '{}': {err}",
            path.display()
        )
    })
}

pub fn load_runbook_registry() -> RunbookLoadResult {
    let mut warnings = Vec::new();
    let mut runbooks = built_in_runbooks();
    let Some(path) = runbooks_config_path() else {
        return RunbookLoadResult {
            registry: RunbookRegistry { runbooks },
            warnings: vec!["user config directory is unavailable; skipping custom runbooks".into()],
            path: PathBuf::from(RUNBOOKS_FILE_NAME),
        };
    };
    let Some(parent) = path.parent() else {
        return RunbookLoadResult {
            registry: RunbookRegistry { runbooks },
            warnings: vec![format!(
                "runbooks config path '{}' has no parent directory",
                path.display()
            )],
            path,
        };
    };
    if let Err(err) = fs::create_dir_all(parent) {
        return RunbookLoadResult {
            registry: RunbookRegistry { runbooks },
            warnings: vec![format!(
                "failed to create runbooks config directory '{}': {err}",
                parent.display()
            )],
            path,
        };
    }
    if path.exists() {
        match load_runbooks_config_from_path(&path) {
            Ok(config) => {
                let validated = validate_runbooks(config);
                warnings.extend(validated.warnings);
                merge_runbooks(&mut runbooks, validated.registry.runbooks);
            }
            Err(err) => warnings.push(err),
        }
    }

    RunbookLoadResult {
        registry: RunbookRegistry { runbooks },
        warnings,
        path,
    }
}

fn merge_runbooks(existing: &mut Vec<LoadedRunbook>, mut custom: Vec<LoadedRunbook>) {
    for runbook in custom.drain(..) {
        if let Some(index) = existing
            .iter()
            .position(|candidate| candidate.id == runbook.id)
        {
            existing[index] = runbook;
        } else {
            existing.push(runbook);
        }
    }
    existing.sort_by(|left, right| left.title.cmp(&right.title));
}

fn validate_runbooks(config: RunbooksConfig) -> RunbookLoadResult {
    let mut warnings = Vec::new();
    let mut runbooks = Vec::new();
    let mut seen_ids = BTreeSet::new();

    for runbook in config.runbooks {
        let id = runbook.id.trim();
        let title = runbook.title.trim();
        if id.is_empty() {
            warnings.push("skipping runbook with empty id".to_string());
            continue;
        }
        if !seen_ids.insert(id.to_string()) {
            warnings.push(format!("skipping duplicate runbook id '{id}'"));
            continue;
        }
        if title.is_empty() {
            warnings.push(format!("skipping runbook '{id}' with empty title"));
            continue;
        }
        let steps = validate_runbook_steps(id, runbook.steps, &mut warnings);
        if steps.is_empty() {
            warnings.push(format!("skipping runbook '{id}' with no valid steps"));
            continue;
        }

        let mut aliases = runbook
            .aliases
            .into_iter()
            .map(|alias| alias.trim().to_ascii_lowercase())
            .filter(|alias| !alias.is_empty())
            .collect::<Vec<_>>();
        aliases.push(title.to_ascii_lowercase());
        aliases.push("runbook".into());
        aliases.push("incident".into());
        aliases.sort();
        aliases.dedup();

        let mut resource_kinds = runbook
            .resource_kinds
            .into_iter()
            .map(|kind| kind.trim().to_string())
            .filter(|kind| !kind.is_empty())
            .collect::<Vec<_>>();
        resource_kinds.sort();
        resource_kinds.dedup();

        runbooks.push(LoadedRunbook {
            id: id.to_string(),
            title: title.to_string(),
            description: runbook.description.filter(|value| !value.trim().is_empty()),
            aliases,
            resource_kinds,
            shortcut: runbook.shortcut.filter(|value| !value.trim().is_empty()),
            steps,
        });
    }

    RunbookLoadResult {
        registry: RunbookRegistry { runbooks },
        warnings,
        path: PathBuf::from(RUNBOOKS_FILE_NAME),
    }
}

fn validate_runbook_steps(
    runbook_id: &str,
    steps: Vec<RunbookStepConfig>,
    warnings: &mut Vec<String>,
) -> Vec<LoadedRunbookStep> {
    let mut loaded = Vec::new();
    for (index, step) in steps.into_iter().enumerate() {
        let loaded_step = match step {
            RunbookStepConfig::Checklist {
                title,
                description,
                items,
            } => {
                let title = title.trim().to_string();
                let items = items
                    .into_iter()
                    .map(|item| item.trim().to_string())
                    .filter(|item| !item.is_empty())
                    .collect::<Vec<_>>();
                if title.is_empty() || items.is_empty() {
                    warnings.push(format!(
                        "skipping checklist step {} in runbook '{}' because title/items are empty",
                        index + 1,
                        runbook_id
                    ));
                    continue;
                }
                LoadedRunbookStep {
                    title,
                    description: description.filter(|value| !value.trim().is_empty()),
                    kind: LoadedRunbookStepKind::Checklist { items },
                }
            }
            RunbookStepConfig::Workspace {
                title,
                description,
                name,
                target,
            } => {
                let title = title.trim().to_string();
                let name = name.trim().to_string();
                if title.is_empty() || name.is_empty() {
                    warnings.push(format!(
                        "skipping workspace step {} in runbook '{}' because title/name are empty",
                        index + 1,
                        runbook_id
                    ));
                    continue;
                }
                LoadedRunbookStep {
                    title,
                    description: description.filter(|value| !value.trim().is_empty()),
                    kind: LoadedRunbookStepKind::Workspace {
                        name,
                        target: target.unwrap_or(RunbookWorkspaceTarget::SavedWorkspace),
                    },
                }
            }
            RunbookStepConfig::DetailAction {
                title,
                description,
                action,
            } => {
                let title = title.trim().to_string();
                if title.is_empty() {
                    warnings.push(format!(
                        "skipping detail-action step {} in runbook '{}' because title is empty",
                        index + 1,
                        runbook_id
                    ));
                    continue;
                }
                LoadedRunbookStep {
                    title,
                    description: description.filter(|value| !value.trim().is_empty()),
                    kind: LoadedRunbookStepKind::DetailAction { action },
                }
            }
            RunbookStepConfig::ExtensionAction {
                title,
                description,
                action_id,
            } => {
                let title = title.trim().to_string();
                let action_id = action_id.trim().to_string();
                if title.is_empty() || action_id.is_empty() {
                    warnings.push(format!(
                        "skipping extension step {} in runbook '{}' because title/action_id are empty",
                        index + 1,
                        runbook_id
                    ));
                    continue;
                }
                LoadedRunbookStep {
                    title,
                    description: description.filter(|value| !value.trim().is_empty()),
                    kind: LoadedRunbookStepKind::ExtensionAction { action_id },
                }
            }
            RunbookStepConfig::AiWorkflow {
                title,
                description,
                workflow,
            } => {
                let title = title.trim().to_string();
                if title.is_empty() {
                    warnings.push(format!(
                        "skipping AI step {} in runbook '{}' because title is empty",
                        index + 1,
                        runbook_id
                    ));
                    continue;
                }
                LoadedRunbookStep {
                    title,
                    description: description.filter(|value| !value.trim().is_empty()),
                    kind: LoadedRunbookStepKind::AiWorkflow { workflow },
                }
            }
        };
        loaded.push(loaded_step);
    }
    loaded
}

fn built_in_runbooks() -> Vec<LoadedRunbook> {
    vec![
        built_in_pod_failure(),
        built_in_rollout_failure(),
        built_in_helm_release_incident(),
        built_in_service_traffic(),
        built_in_pod_network(),
    ]
}

fn built_in_pod_failure() -> LoadedRunbook {
    LoadedRunbook {
        id: "pod_failure".into(),
        title: "Pod Failure Triage".into(),
        description: Some(
            "Deterministic pod failure checks with optional AI failure summary.".into(),
        ),
        aliases: vec![
            "pod failure".into(),
            "pod incident".into(),
            "crashloop".into(),
            "incident".into(),
            "runbook".into(),
        ],
        resource_kinds: vec!["Pod".into()],
        shortcut: None,
        steps: vec![
            LoadedRunbookStep {
                title: "Confirm the failure signal".into(),
                description: Some(
                    "Establish whether the pod is failing now or the alert is stale.".into(),
                ),
                kind: LoadedRunbookStepKind::Checklist {
                    items: vec![
                        "Check restart count and current phase.".into(),
                        "Check if the failure is isolated or all replicas are affected.".into(),
                        "Note the namespace, image tag, and owning workload.".into(),
                    ],
                },
            },
            LoadedRunbookStep {
                title: "Open events".into(),
                description: Some(
                    "Capture kube-scheduler, kubelet, and probe failure signals.".into(),
                ),
                kind: LoadedRunbookStepKind::DetailAction {
                    action: RunbookDetailAction::ViewEvents,
                },
            },
            LoadedRunbookStep {
                title: "Open logs".into(),
                description: Some("Review recent error lines before changing the pod.".into()),
                kind: LoadedRunbookStepKind::DetailAction {
                    action: RunbookDetailAction::Logs,
                },
            },
            LoadedRunbookStep {
                title: "Open probes".into(),
                description: Some("Compare liveness/readiness/startup probe failures.".into()),
                kind: LoadedRunbookStepKind::DetailAction {
                    action: RunbookDetailAction::Probes,
                },
            },
            LoadedRunbookStep {
                title: "Explain failure with AI".into(),
                description: Some(
                    "Use the shipped failure-analysis workflow after deterministic checks.".into(),
                ),
                kind: LoadedRunbookStepKind::AiWorkflow {
                    workflow: AiWorkflowKind::ExplainFailure,
                },
            },
        ],
    }
}

fn built_in_rollout_failure() -> LoadedRunbook {
    LoadedRunbook {
        id: "rollout_failure".into(),
        title: "Rollout Failure".into(),
        description: Some("Rollout center first, then logs and rollout-risk AI review.".into()),
        aliases: vec![
            "rollout failure".into(),
            "deployment incident".into(),
            "release regression".into(),
            "runbook".into(),
        ],
        resource_kinds: vec![
            "Deployment".into(),
            "StatefulSet".into(),
            "DaemonSet".into(),
        ],
        shortcut: None,
        steps: vec![
            LoadedRunbookStep {
                title: "Open rollout center".into(),
                description: Some(
                    "Inspect revisions, pause state, and controller-reported blockers.".into(),
                ),
                kind: LoadedRunbookStepKind::DetailAction {
                    action: RunbookDetailAction::ViewRollout,
                },
            },
            LoadedRunbookStep {
                title: "Open workload logs".into(),
                description: Some(
                    "Check for common request, startup, and dependency failures across pods."
                        .into(),
                ),
                kind: LoadedRunbookStepKind::DetailAction {
                    action: RunbookDetailAction::Logs,
                },
            },
            LoadedRunbookStep {
                title: "Verify traffic impact".into(),
                description: Some(
                    "Confirm whether Services and routes still resolve healthy backends.".into(),
                ),
                kind: LoadedRunbookStepKind::DetailAction {
                    action: RunbookDetailAction::ViewTrafficDebug,
                },
            },
            LoadedRunbookStep {
                title: "Summarize rollout risk".into(),
                description: Some(
                    "Use AI only after the concrete rollout signals are visible.".into(),
                ),
                kind: LoadedRunbookStepKind::AiWorkflow {
                    workflow: AiWorkflowKind::RolloutRisk,
                },
            },
        ],
    }
}

fn built_in_helm_release_incident() -> LoadedRunbook {
    LoadedRunbook {
        id: "helm_release_incident".into(),
        title: "Helm Release Incident".into(),
        description: Some("Review Helm history and values diff before rollback decisions.".into()),
        aliases: vec![
            "helm incident".into(),
            "helm rollback".into(),
            "release rollback".into(),
            "runbook".into(),
        ],
        resource_kinds: vec!["HelmRelease".into()],
        shortcut: None,
        steps: vec![
            LoadedRunbookStep {
                title: "Open Helm history".into(),
                description: Some(
                    "Inspect revisions and compare current values with a known-good revision."
                        .into(),
                ),
                kind: LoadedRunbookStepKind::DetailAction {
                    action: RunbookDetailAction::ViewHelmHistory,
                },
            },
            LoadedRunbookStep {
                title: "Open rollout center".into(),
                description: Some(
                    "Cross-check controller health before choosing a rollback revision.".into(),
                ),
                kind: LoadedRunbookStepKind::DetailAction {
                    action: RunbookDetailAction::ViewRollout,
                },
            },
            LoadedRunbookStep {
                title: "Summarize rollout risk".into(),
                description: Some(
                    "Use AI to compare current rollout risk after deterministic review.".into(),
                ),
                kind: LoadedRunbookStepKind::AiWorkflow {
                    workflow: AiWorkflowKind::RolloutRisk,
                },
            },
        ],
    }
}

fn built_in_service_traffic() -> LoadedRunbook {
    LoadedRunbook {
        id: "service_traffic_incident".into(),
        title: "Service Traffic Incident".into(),
        description: Some(
            "Use traffic debug, relationships, and AI network verdict together.".into(),
        ),
        aliases: vec![
            "traffic incident".into(),
            "service outage".into(),
            "network incident".into(),
            "runbook".into(),
        ],
        resource_kinds: vec![
            "Service".into(),
            "Ingress".into(),
            "Gateway".into(),
            "HTTPRoute".into(),
            "GRPCRoute".into(),
        ],
        shortcut: None,
        steps: vec![
            LoadedRunbookStep {
                title: "Open traffic debug".into(),
                description: Some(
                    "Trace route-to-service-to-pod intent before editing policy.".into(),
                ),
                kind: LoadedRunbookStepKind::DetailAction {
                    action: RunbookDetailAction::ViewTrafficDebug,
                },
            },
            LoadedRunbookStep {
                title: "Open relationships".into(),
                description: Some(
                    "Inspect attached routes, services, backends, and missing references.".into(),
                ),
                kind: LoadedRunbookStepKind::DetailAction {
                    action: RunbookDetailAction::ViewRelationships,
                },
            },
            LoadedRunbookStep {
                title: "Explain network verdict".into(),
                description: Some("Use AI after the deterministic traffic view is open.".into()),
                kind: LoadedRunbookStepKind::AiWorkflow {
                    workflow: AiWorkflowKind::NetworkVerdict,
                },
            },
        ],
    }
}

fn built_in_pod_network() -> LoadedRunbook {
    LoadedRunbook {
        id: "pod_network_policy_incident".into(),
        title: "Pod Connectivity Incident".into(),
        description: Some("Validate policy intent and pod reachability for a selected pod.".into()),
        aliases: vec![
            "pod network".into(),
            "connectivity incident".into(),
            "policy incident".into(),
            "runbook".into(),
        ],
        resource_kinds: vec!["Pod".into()],
        shortcut: None,
        steps: vec![
            LoadedRunbookStep {
                title: "Open connectivity analysis".into(),
                description: Some("Check current policy intent against another pod target.".into()),
                kind: LoadedRunbookStepKind::DetailAction {
                    action: RunbookDetailAction::CheckNetworkConnectivity,
                },
            },
            LoadedRunbookStep {
                title: "Open network policy view".into(),
                description: Some(
                    "Inspect the pod's current policy coverage and selector scope.".into(),
                ),
                kind: LoadedRunbookStepKind::DetailAction {
                    action: RunbookDetailAction::ViewNetworkPolicies,
                },
            },
            LoadedRunbookStep {
                title: "Explain network verdict".into(),
                description: Some(
                    "Use AI to summarize the connectivity decision conservatively.".into(),
                ),
                kind: LoadedRunbookStepKind::AiWorkflow {
                    workflow: AiWorkflowKind::NetworkVerdict,
                },
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_registry_contains_pod_failure_runbook() {
        let registry = load_runbook_registry().registry;
        assert!(registry.get("pod_failure").is_some());
    }

    #[test]
    fn palette_filters_runbooks_by_resource_kind() {
        let registry = RunbookRegistry {
            runbooks: vec![built_in_pod_failure(), built_in_rollout_failure()],
        };
        let pod = ResourceRef::Pod("api-0".into(), "demo".into());

        let matches = registry.palette_runbooks_for(Some(&pod));
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "pod_failure");
    }

    #[test]
    fn custom_runbook_validation_skips_empty_steps() {
        let result = validate_runbooks(RunbooksConfig {
            runbooks: vec![RunbookConfig {
                id: "empty".into(),
                title: "Empty".into(),
                description: None,
                aliases: Vec::new(),
                resource_kinds: Vec::new(),
                shortcut: None,
                steps: vec![RunbookStepConfig::Checklist {
                    title: "".into(),
                    description: None,
                    items: vec!["".into()],
                }],
            }],
        });

        assert!(result.registry.runbooks.is_empty());
        assert!(!result.warnings.is_empty());
    }
}
