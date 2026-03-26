//! Workload-to-pod resolution for aggregated workload logs.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow};
use k8s_openapi::{
    api::{
        apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet},
        batch::v1::Job,
        core::v1::{Pod, ReplicationController},
    },
    apimachinery::pkg::apis::meta::v1::{LabelSelector, LabelSelectorRequirement},
};
use kube::{Api, Client, api::ListParams};

use crate::app::ResourceRef;

pub const MAX_WORKLOAD_LOG_STREAMS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadLogTarget {
    pub pod_name: String,
    pub namespace: String,
    pub containers: Vec<String>,
    pub labels: Vec<(String, String)>,
}

pub async fn resolve_workload_log_targets(
    client: Client,
    resource: &ResourceRef,
) -> Result<Vec<WorkloadLogTarget>> {
    match resource {
        ResourceRef::Pod(name, namespace) => fetch_single_pod_target(client, name, namespace).await,
        ResourceRef::Deployment(name, namespace) => {
            let api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
            let workload = api
                .get(name)
                .await
                .with_context(|| format!("failed to fetch Deployment '{name}'"))?;
            list_pods_for_selector(
                client,
                namespace,
                selector_to_string(workload.spec.map(|spec| spec.selector))?,
            )
            .await
        }
        ResourceRef::StatefulSet(name, namespace) => {
            let api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);
            let workload = api
                .get(name)
                .await
                .with_context(|| format!("failed to fetch StatefulSet '{name}'"))?;
            list_pods_for_selector(
                client,
                namespace,
                selector_to_string(workload.spec.map(|spec| spec.selector))?,
            )
            .await
        }
        ResourceRef::DaemonSet(name, namespace) => {
            let api: Api<DaemonSet> = Api::namespaced(client.clone(), namespace);
            let workload = api
                .get(name)
                .await
                .with_context(|| format!("failed to fetch DaemonSet '{name}'"))?;
            list_pods_for_selector(
                client,
                namespace,
                selector_to_string(workload.spec.map(|spec| spec.selector))?,
            )
            .await
        }
        ResourceRef::ReplicaSet(name, namespace) => {
            let api: Api<ReplicaSet> = Api::namespaced(client.clone(), namespace);
            let workload = api
                .get(name)
                .await
                .with_context(|| format!("failed to fetch ReplicaSet '{name}'"))?;
            list_pods_for_selector(
                client,
                namespace,
                selector_to_string(workload.spec.map(|spec| spec.selector))?,
            )
            .await
        }
        ResourceRef::ReplicationController(name, namespace) => {
            let api: Api<ReplicationController> = Api::namespaced(client.clone(), namespace);
            let workload = api
                .get(name)
                .await
                .with_context(|| format!("failed to fetch ReplicationController '{name}'"))?;
            list_pods_for_match_labels(
                client,
                namespace,
                workload.spec.and_then(|spec| spec.selector),
            )
            .await
        }
        ResourceRef::Job(name, namespace) => {
            let api: Api<Job> = Api::namespaced(client.clone(), namespace);
            let workload = api
                .get(name)
                .await
                .with_context(|| format!("failed to fetch Job '{name}'"))?;
            list_pods_for_selector(
                client,
                namespace,
                selector_to_string(workload.spec.and_then(|spec| spec.selector))?,
            )
            .await
        }
        _ => Err(anyhow!(
            "Aggregated logs are only available for Pods, Deployments, StatefulSets, DaemonSets, ReplicaSets, ReplicationControllers, and Jobs."
        )),
    }
}

async fn fetch_single_pod_target(
    client: Client,
    name: &str,
    namespace: &str,
) -> Result<Vec<WorkloadLogTarget>> {
    let pods: Api<Pod> = Api::namespaced(client, namespace);
    let pod = pods
        .get(name)
        .await
        .with_context(|| format!("failed to fetch Pod '{name}'"))?;
    Ok(vec![map_pod_to_target(pod)?])
}

async fn list_pods_for_match_labels(
    client: Client,
    namespace: &str,
    labels: Option<BTreeMap<String, String>>,
) -> Result<Vec<WorkloadLogTarget>> {
    let Some(labels) = labels else {
        return Err(anyhow!("workload selector is missing"));
    };
    let selector = labels
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",");
    list_pods_for_selector(client, namespace, selector).await
}

async fn list_pods_for_selector(
    client: Client,
    namespace: &str,
    selector: String,
) -> Result<Vec<WorkloadLogTarget>> {
    if selector.is_empty() {
        return Err(anyhow!("workload selector is empty"));
    }

    let pods: Api<Pod> = Api::namespaced(client, namespace);
    let list = pods
        .list(&ListParams::default().labels(&selector))
        .await
        .with_context(|| format!("failed to list pods for selector '{selector}'"))?;

    let mut targets = list
        .items
        .into_iter()
        .map(map_pod_to_target)
        .collect::<Result<Vec<_>>>()?;
    targets.sort_unstable_by(|left, right| left.pod_name.cmp(&right.pod_name));

    if targets.is_empty() {
        return Err(anyhow!("no pods matched selector '{selector}'"));
    }

    Ok(targets)
}

fn map_pod_to_target(pod: Pod) -> Result<WorkloadLogTarget> {
    let name = pod.metadata.name.context("pod missing metadata.name")?;
    let namespace = pod
        .metadata
        .namespace
        .context("pod missing metadata.namespace")?;
    let containers = pod
        .spec
        .context("pod missing spec")?
        .containers
        .into_iter()
        .map(|container| container.name)
        .collect::<Vec<_>>();

    if containers.is_empty() {
        return Err(anyhow!("pod '{name}' has no containers"));
    }

    let mut labels = pod
        .metadata
        .labels
        .unwrap_or_default()
        .into_iter()
        .collect::<Vec<_>>();
    labels.sort_unstable_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));

    Ok(WorkloadLogTarget {
        pod_name: name,
        namespace,
        containers,
        labels,
    })
}

fn selector_to_string(selector: Option<LabelSelector>) -> Result<String> {
    let Some(selector) = selector else {
        return Err(anyhow!("workload selector is missing"));
    };

    let mut parts = Vec::new();
    if let Some(labels) = selector.match_labels {
        let mut labels = labels.into_iter().collect::<Vec<_>>();
        labels.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        parts.extend(
            labels
                .into_iter()
                .map(|(key, value)| format!("{key}={value}")),
        );
    }
    if let Some(expressions) = selector.match_expressions {
        let mut expressions = expressions;
        expressions.sort_unstable_by(|left, right| left.key.cmp(&right.key));
        for expression in expressions {
            parts.push(expression_to_string(expression)?);
        }
    }

    if parts.is_empty() {
        return Err(anyhow!("workload selector is empty"));
    }

    Ok(parts.join(","))
}

fn expression_to_string(expression: LabelSelectorRequirement) -> Result<String> {
    let values = expression.values.unwrap_or_default();
    match expression.operator.as_str() {
        "In" => Ok(format!("{} in ({})", expression.key, values.join(","))),
        "NotIn" => Ok(format!("{} notin ({})", expression.key, values.join(","))),
        "Exists" => Ok(expression.key),
        "DoesNotExist" => Ok(format!("!{}", expression.key)),
        other => Err(anyhow!("unsupported label selector operator '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_to_string_includes_labels_and_expressions() {
        let selector = LabelSelector {
            match_labels: Some(BTreeMap::from([
                ("app".to_string(), "api".to_string()),
                ("tier".to_string(), "backend".to_string()),
            ])),
            match_expressions: Some(vec![LabelSelectorRequirement {
                key: "track".to_string(),
                operator: "In".to_string(),
                values: Some(vec!["stable".to_string(), "canary".to_string()]),
            }]),
        };

        let rendered = selector_to_string(Some(selector)).expect("selector");
        assert!(rendered.contains("app=api"));
        assert!(rendered.contains("tier=backend"));
        assert!(rendered.contains("track in (stable,canary)"));
    }

    #[test]
    fn selector_to_string_rejects_empty_selector() {
        let err = selector_to_string(Some(LabelSelector::default())).expect_err("empty selector");
        assert!(err.to_string().contains("selector is empty"));
    }
}
