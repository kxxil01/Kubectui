//! API-native workload rollout inspection and mutation helpers.

use anyhow::{Context, Result, anyhow, bail};
use k8s_openapi::{
    api::apps::v1::{ControllerRevision, DaemonSet, Deployment, ReplicaSet, StatefulSet},
    apimachinery::pkg::apis::meta::v1::ObjectMeta,
};
use kube::{
    Api,
    api::{Patch, PatchParams},
};
use serde_json::{Value, json};

use crate::{app::ResourceRef, k8s::client::K8sClient};

const CHANGE_CAUSE_ANNOTATION: &str = "kubernetes.io/change-cause";
const DEPLOYMENT_REVISION_ANNOTATION: &str = "deployment.kubernetes.io/revision";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RolloutWorkloadKind {
    Deployment,
    StatefulSet,
    DaemonSet,
}

impl RolloutWorkloadKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Deployment => "Deployment",
            Self::StatefulSet => "StatefulSet",
            Self::DaemonSet => "DaemonSet",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolloutConditionInfo {
    pub type_: String,
    pub status: String,
    pub reason: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolloutRevisionInfo {
    pub revision: i64,
    pub name: String,
    pub created: Option<String>,
    pub summary: String,
    pub change_cause: Option<String>,
    pub is_current: bool,
    pub is_update_target: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolloutInspection {
    pub kind: RolloutWorkloadKind,
    pub strategy: String,
    pub paused: bool,
    pub current_revision: Option<i64>,
    pub update_target_revision: Option<i64>,
    pub summary_lines: Vec<String>,
    pub conditions: Vec<RolloutConditionInfo>,
    pub revisions: Vec<RolloutRevisionInfo>,
}

impl K8sClient {
    pub async fn fetch_rollout_inspection(
        &self,
        resource: &ResourceRef,
    ) -> Result<RolloutInspection> {
        match resource {
            ResourceRef::Deployment(name, namespace) => {
                fetch_deployment_rollout(self, name, namespace).await
            }
            ResourceRef::StatefulSet(name, namespace) => {
                fetch_statefulset_rollout(self, name, namespace).await
            }
            ResourceRef::DaemonSet(name, namespace) => {
                fetch_daemonset_rollout(self, name, namespace).await
            }
            _ => bail!(
                "Rollout control is only available for Deployments, StatefulSets, and DaemonSets."
            ),
        }
    }

    pub async fn set_deployment_rollout_paused(
        &self,
        name: &str,
        namespace: &str,
        paused: bool,
    ) -> Result<()> {
        let api: Api<Deployment> = Api::namespaced(self.get_client(), namespace);
        api.patch(
            name,
            &PatchParams::default(),
            &Patch::Merge(json!({ "spec": { "paused": paused } })),
        )
        .await
        .with_context(|| {
            if paused {
                format!("failed to pause deployment '{name}' in '{namespace}'")
            } else {
                format!("failed to resume deployment '{name}' in '{namespace}'")
            }
        })?;
        Ok(())
    }

    pub async fn rollback_workload_to_revision(
        &self,
        resource: &ResourceRef,
        revision: i64,
    ) -> Result<()> {
        match resource {
            ResourceRef::Deployment(name, namespace) => {
                let api: Api<Deployment> = Api::namespaced(self.get_client(), namespace);
                let deployment = api.get(name).await.with_context(|| {
                    format!("failed to fetch deployment '{name}' in '{namespace}'")
                })?;
                let uid = deployment.metadata.uid.clone();
                let replicasets =
                    list_related_replicasets(self, namespace, name, uid.as_deref()).await?;
                let template = replicasets
                    .into_iter()
                    .find(|rs| {
                        deployment_revision(rs.metadata.annotations.as_ref()) == Some(revision)
                    })
                    .and_then(|rs| rs.spec.and_then(|spec| spec.template))
                    .map(serde_json::to_value)
                    .transpose()
                    .context("failed to encode deployment revision template")?
                    .ok_or_else(|| anyhow!("deployment revision {revision} has no Pod template"))?;
                patch_workload_template::<Deployment>(self, namespace, name, template)
                    .await
                    .with_context(|| {
                        format!("failed to roll back deployment '{name}' to revision {revision}")
                    })?;
            }
            ResourceRef::StatefulSet(name, namespace) => {
                let api: Api<StatefulSet> = Api::namespaced(self.get_client(), namespace);
                let statefulset = api.get(name).await.with_context(|| {
                    format!("failed to fetch statefulset '{name}' in '{namespace}'")
                })?;
                let revisions = list_related_controller_revisions(
                    self,
                    namespace,
                    name,
                    statefulset.metadata.uid.as_deref(),
                )
                .await?;
                let template = revisions
                    .into_iter()
                    .find(|entry| entry.revision == revision)
                    .and_then(|entry| entry.data)
                    .and_then(extract_controller_revision_template)
                    .ok_or_else(|| {
                        anyhow!("statefulset revision {revision} has no Pod template")
                    })?;
                patch_workload_template::<StatefulSet>(self, namespace, name, template)
                    .await
                    .with_context(|| {
                        format!("failed to roll back statefulset '{name}' to revision {revision}")
                    })?;
            }
            ResourceRef::DaemonSet(name, namespace) => {
                let api: Api<DaemonSet> = Api::namespaced(self.get_client(), namespace);
                let daemonset = api.get(name).await.with_context(|| {
                    format!("failed to fetch daemonset '{name}' in '{namespace}'")
                })?;
                let revisions = list_related_controller_revisions(
                    self,
                    namespace,
                    name,
                    daemonset.metadata.uid.as_deref(),
                )
                .await?;
                let template = revisions
                    .into_iter()
                    .find(|entry| entry.revision == revision)
                    .and_then(|entry| entry.data)
                    .and_then(extract_controller_revision_template)
                    .ok_or_else(|| anyhow!("daemonset revision {revision} has no Pod template"))?;
                patch_workload_template::<DaemonSet>(self, namespace, name, template)
                    .await
                    .with_context(|| {
                        format!("failed to roll back daemonset '{name}' to revision {revision}")
                    })?;
            }
            _ => bail!(
                "Rollout undo is only available for Deployments, StatefulSets, and DaemonSets."
            ),
        }

        Ok(())
    }
}

async fn fetch_deployment_rollout(
    client: &K8sClient,
    name: &str,
    namespace: &str,
) -> Result<RolloutInspection> {
    let api: Api<Deployment> = Api::namespaced(client.get_client(), namespace);
    let deployment = api
        .get(name)
        .await
        .with_context(|| format!("failed to fetch deployment '{name}' in '{namespace}'"))?;
    let status = deployment.status.as_ref();
    let spec = deployment.spec.as_ref();
    let desired = spec.and_then(|spec| spec.replicas).unwrap_or(1);
    let updated = status
        .and_then(|status| status.updated_replicas)
        .unwrap_or_default();
    let ready = status
        .and_then(|status| status.ready_replicas)
        .unwrap_or_default();
    let available = status
        .and_then(|status| status.available_replicas)
        .unwrap_or_default();
    let paused = spec.and_then(|spec| spec.paused).unwrap_or(false);
    let current_revision = deployment_revision(deployment.metadata.annotations.as_ref());
    let revisions =
        list_related_replicasets(client, namespace, name, deployment.metadata.uid.as_deref())
            .await?;
    let mut revision_entries: Vec<_> = revisions
        .into_iter()
        .filter_map(|rs| {
            let revision = deployment_revision(rs.metadata.annotations.as_ref())?;
            let created = metadata_created(&rs.metadata);
            let name = rs
                .metadata
                .name
                .clone()
                .unwrap_or_else(|| format!("revision-{revision}"));
            let desired_replicas = rs
                .spec
                .as_ref()
                .and_then(|spec| spec.replicas)
                .unwrap_or_default();
            let ready_replicas = rs
                .status
                .as_ref()
                .and_then(|status| status.ready_replicas)
                .unwrap_or_default();
            Some(RolloutRevisionInfo {
                revision,
                name,
                created,
                summary: format!("{ready_replicas}/{desired_replicas} ready"),
                change_cause: annotation_value(
                    rs.metadata.annotations.as_ref(),
                    CHANGE_CAUSE_ANNOTATION,
                ),
                is_current: Some(revision) == current_revision,
                is_update_target: Some(revision) == current_revision,
            })
        })
        .collect();
    revision_entries.sort_by(|a, b| {
        b.revision
            .cmp(&a.revision)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(RolloutInspection {
        kind: RolloutWorkloadKind::Deployment,
        strategy: spec
            .and_then(|spec| spec.strategy.as_ref())
            .and_then(|strategy| strategy.type_.clone())
            .unwrap_or_else(|| "RollingUpdate".to_string()),
        paused,
        current_revision,
        update_target_revision: current_revision,
        summary_lines: vec![
            format!(
                "Desired {desired} · Updated {updated} · Ready {ready} · Available {available}"
            ),
            format!(
                "Observed generation {}{}",
                status
                    .and_then(|status| status.observed_generation)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_string()),
                if paused { " · Paused" } else { "" }
            ),
        ],
        conditions: deployment_conditions(&deployment),
        revisions: revision_entries,
    })
}

async fn fetch_statefulset_rollout(
    client: &K8sClient,
    name: &str,
    namespace: &str,
) -> Result<RolloutInspection> {
    let api: Api<StatefulSet> = Api::namespaced(client.get_client(), namespace);
    let statefulset = api
        .get(name)
        .await
        .with_context(|| format!("failed to fetch statefulset '{name}' in '{namespace}'"))?;
    let status = statefulset.status.as_ref();
    let spec = statefulset.spec.as_ref();
    let revisions = list_related_controller_revisions(
        client,
        namespace,
        name,
        statefulset.metadata.uid.as_deref(),
    )
    .await?;
    let current_revision_name = status.and_then(|status| status.current_revision.clone());
    let update_revision_name = status.and_then(|status| status.update_revision.clone());
    let mut revision_entries: Vec<_> = revisions
        .into_iter()
        .map(|entry| {
            let created = metadata_created(&entry.metadata);
            let name = entry
                .metadata
                .name
                .clone()
                .unwrap_or_else(|| format!("revision-{}", entry.revision));
            RolloutRevisionInfo {
                revision: entry.revision,
                name,
                created,
                summary: String::from("ControllerRevision"),
                change_cause: annotation_value(
                    entry.metadata.annotations.as_ref(),
                    CHANGE_CAUSE_ANNOTATION,
                ),
                is_current: false,
                is_update_target: false,
            }
        })
        .collect();
    let current_revision = mark_controller_revision_flags(
        &mut revision_entries,
        current_revision_name.as_deref(),
        update_revision_name.as_deref(),
    );
    let update_target_revision = revision_entries
        .iter()
        .find(|entry| entry.is_update_target)
        .map(|entry| entry.revision)
        .or(current_revision);
    revision_entries.sort_by(|a, b| {
        b.revision
            .cmp(&a.revision)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(RolloutInspection {
        kind: RolloutWorkloadKind::StatefulSet,
        strategy: spec
            .and_then(|spec| spec.update_strategy.as_ref())
            .and_then(|strategy| strategy.type_.clone())
            .unwrap_or_else(|| "RollingUpdate".to_string()),
        paused: false,
        current_revision,
        update_target_revision,
        summary_lines: vec![
            format!(
                "Replicas {} · Updated {} · Ready {} · Available {}",
                status.map_or(0, |status| status.replicas),
                status
                    .and_then(|status| status.updated_replicas)
                    .unwrap_or_default(),
                status
                    .and_then(|status| status.ready_replicas)
                    .unwrap_or_default(),
                status
                    .and_then(|status| status.available_replicas)
                    .unwrap_or_default(),
            ),
            format!(
                "Current rev {} · Update rev {}",
                current_revision_name.unwrap_or_else(|| "n/a".to_string()),
                update_revision_name.unwrap_or_else(|| "n/a".to_string())
            ),
        ],
        conditions: statefulset_conditions(&statefulset),
        revisions: revision_entries,
    })
}

async fn fetch_daemonset_rollout(
    client: &K8sClient,
    name: &str,
    namespace: &str,
) -> Result<RolloutInspection> {
    let api: Api<DaemonSet> = Api::namespaced(client.get_client(), namespace);
    let daemonset = api
        .get(name)
        .await
        .with_context(|| format!("failed to fetch daemonset '{name}' in '{namespace}'"))?;
    let status = daemonset.status.as_ref();
    let spec = daemonset.spec.as_ref();
    let revisions = list_related_controller_revisions(
        client,
        namespace,
        name,
        daemonset.metadata.uid.as_deref(),
    )
    .await?;
    let mut revision_entries: Vec<_> = revisions
        .into_iter()
        .map(|entry| {
            let created = metadata_created(&entry.metadata);
            let name = entry
                .metadata
                .name
                .clone()
                .unwrap_or_else(|| format!("revision-{}", entry.revision));
            RolloutRevisionInfo {
                revision: entry.revision,
                name,
                created,
                summary: String::from("ControllerRevision"),
                change_cause: annotation_value(
                    entry.metadata.annotations.as_ref(),
                    CHANGE_CAUSE_ANNOTATION,
                ),
                is_current: false,
                is_update_target: false,
            }
        })
        .collect();
    revision_entries.sort_by(|a, b| {
        b.revision
            .cmp(&a.revision)
            .then_with(|| a.name.cmp(&b.name))
    });
    let current_revision = revision_entries.first().map(|entry| entry.revision);
    if let Some(current) = current_revision
        && let Some(entry) = revision_entries
            .iter_mut()
            .find(|entry| entry.revision == current)
    {
        entry.is_current = true;
        entry.is_update_target = true;
    }
    Ok(RolloutInspection {
        kind: RolloutWorkloadKind::DaemonSet,
        strategy: spec
            .and_then(|spec| spec.update_strategy.as_ref())
            .and_then(|strategy| strategy.type_.clone())
            .unwrap_or_else(|| "RollingUpdate".to_string()),
        paused: false,
        current_revision,
        update_target_revision: current_revision,
        summary_lines: vec![
            format!(
                "Desired {} · Updated {} · Ready {} · Available {}",
                status.map_or(0, |status| status.desired_number_scheduled),
                status
                    .and_then(|status| status.updated_number_scheduled)
                    .unwrap_or_default(),
                status.map_or(0, |status| status.number_ready),
                status
                    .and_then(|status| status.number_available)
                    .unwrap_or_default(),
            ),
            format!(
                "Misscheduled {} · Unavailable {}",
                status.map_or(0, |status| status.number_misscheduled),
                status
                    .and_then(|status| status.number_unavailable)
                    .unwrap_or_default(),
            ),
        ],
        conditions: daemonset_conditions(&daemonset),
        revisions: revision_entries,
    })
}

async fn list_related_replicasets(
    client: &K8sClient,
    namespace: &str,
    owner_name: &str,
    owner_uid: Option<&str>,
) -> Result<Vec<ReplicaSet>> {
    let api: Api<ReplicaSet> = Api::namespaced(client.get_client(), namespace);
    let items = api
        .list(&Default::default())
        .await
        .with_context(|| format!("failed to list ReplicaSets in namespace '{namespace}'"))?;
    Ok(items
        .items
        .into_iter()
        .filter(|item| owner_ref_matches(&item.metadata, "Deployment", owner_name, owner_uid))
        .collect())
}

async fn list_related_controller_revisions(
    client: &K8sClient,
    namespace: &str,
    owner_name: &str,
    owner_uid: Option<&str>,
) -> Result<Vec<ControllerRevision>> {
    let api: Api<ControllerRevision> = Api::namespaced(client.get_client(), namespace);
    let items = api.list(&Default::default()).await.with_context(|| {
        format!("failed to list ControllerRevisions in namespace '{namespace}'")
    })?;
    Ok(items
        .items
        .into_iter()
        .filter(|item| {
            owner_ref_matches(&item.metadata, "StatefulSet", owner_name, owner_uid)
                || owner_ref_matches(&item.metadata, "DaemonSet", owner_name, owner_uid)
        })
        .collect())
}

async fn patch_workload_template<K>(
    client: &K8sClient,
    namespace: &str,
    name: &str,
    template: Value,
) -> Result<()>
where
    K: Clone
        + std::fmt::Debug
        + serde::de::DeserializeOwned
        + kube::Resource<Scope = k8s_openapi::NamespaceResourceScope>,
    <K as kube::Resource>::DynamicType: Default,
{
    let api: Api<K> = Api::namespaced(client.get_client(), namespace);
    api.patch(
        name,
        &PatchParams::default(),
        &Patch::Merge(json!({ "spec": { "template": template } })),
    )
    .await
    .with_context(|| format!("failed to patch workload '{name}' in namespace '{namespace}'"))?;
    Ok(())
}

fn extract_controller_revision_template(
    data: k8s_openapi::apimachinery::pkg::runtime::RawExtension,
) -> Option<Value> {
    let value = data.0;
    value
        .pointer("/spec/template")
        .cloned()
        .or_else(|| value.pointer("/template").cloned())
        .filter(Value::is_object)
}

fn deployment_revision(
    annotations: Option<&std::collections::BTreeMap<String, String>>,
) -> Option<i64> {
    annotation_value(annotations, DEPLOYMENT_REVISION_ANNOTATION)?
        .parse()
        .ok()
}

fn annotation_value(
    annotations: Option<&std::collections::BTreeMap<String, String>>,
    key: &str,
) -> Option<String> {
    annotations.and_then(|annotations| annotations.get(key).cloned())
}

fn metadata_created(metadata: &ObjectMeta) -> Option<String> {
    metadata
        .creation_timestamp
        .as_ref()
        .map(|ts| ts.0.to_string())
}

fn owner_ref_matches(metadata: &ObjectMeta, kind: &str, name: &str, uid: Option<&str>) -> bool {
    metadata.owner_references.as_ref().is_some_and(|owners| {
        owners.iter().any(|owner| {
            owner.kind == kind && owner.name == name && uid.is_none_or(|uid| owner.uid == uid)
        })
    })
}

fn mark_controller_revision_flags(
    revisions: &mut [RolloutRevisionInfo],
    current_name: Option<&str>,
    update_name: Option<&str>,
) -> Option<i64> {
    let mut current_revision = None;
    for entry in revisions {
        entry.is_current = current_name.is_some_and(|name| entry.name == name);
        entry.is_update_target = update_name.is_some_and(|name| entry.name == name);
        if entry.is_current {
            current_revision = Some(entry.revision);
        }
    }
    current_revision
}

fn deployment_conditions(deployment: &Deployment) -> Vec<RolloutConditionInfo> {
    deployment
        .status
        .as_ref()
        .and_then(|status| status.conditions.as_ref())
        .map(|conditions| {
            conditions
                .iter()
                .map(|condition| RolloutConditionInfo {
                    type_: condition.type_.clone(),
                    status: condition.status.clone(),
                    reason: condition.reason.clone(),
                    message: condition.message.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn statefulset_conditions(statefulset: &StatefulSet) -> Vec<RolloutConditionInfo> {
    statefulset
        .status
        .as_ref()
        .and_then(|status| status.conditions.as_ref())
        .map(|conditions| {
            conditions
                .iter()
                .map(|condition| RolloutConditionInfo {
                    type_: condition.type_.clone(),
                    status: condition.status.clone(),
                    reason: condition.reason.clone(),
                    message: condition.message.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn daemonset_conditions(daemonset: &DaemonSet) -> Vec<RolloutConditionInfo> {
    daemonset
        .status
        .as_ref()
        .and_then(|status| status.conditions.as_ref())
        .map(|conditions| {
            conditions
                .iter()
                .map(|condition| RolloutConditionInfo {
                    type_: condition.type_.clone(),
                    status: condition.status.clone(),
                    reason: condition.reason.clone(),
                    message: condition.message.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn extract_controller_revision_template_prefers_spec_template() {
        let data = k8s_openapi::apimachinery::pkg::runtime::RawExtension(json!({
            "spec": {
                "template": {
                    "metadata": { "labels": { "app": "api" } },
                    "spec": { "containers": [{ "name": "api", "image": "nginx:1.0" }] }
                }
            }
        }));
        let template = extract_controller_revision_template(data).expect("template");
        assert_eq!(template["metadata"]["labels"]["app"], "api");
    }

    #[test]
    fn mark_controller_revision_flags_marks_current_and_update_target() {
        let mut revisions = vec![
            RolloutRevisionInfo {
                revision: 2,
                name: "demo-2".to_string(),
                created: None,
                summary: String::new(),
                change_cause: None,
                is_current: false,
                is_update_target: false,
            },
            RolloutRevisionInfo {
                revision: 1,
                name: "demo-1".to_string(),
                created: None,
                summary: String::new(),
                change_cause: None,
                is_current: false,
                is_update_target: false,
            },
        ];
        let current =
            mark_controller_revision_flags(&mut revisions, Some("demo-1"), Some("demo-2"));
        assert_eq!(current, Some(1));
        assert!(revisions[0].is_update_target);
        assert!(revisions[1].is_current);
    }

    #[test]
    fn deployment_revision_reads_annotation() {
        let mut annotations = BTreeMap::new();
        annotations.insert(DEPLOYMENT_REVISION_ANNOTATION.to_string(), "7".to_string());
        assert_eq!(deployment_revision(Some(&annotations)), Some(7));
    }
}
