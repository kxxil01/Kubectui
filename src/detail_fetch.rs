//! Detail view data fetching functions extracted from `main.rs`.
//!
//! These handle fetching full detail view data including YAML, events,
//! metrics, and cronjob history enrichment.

use anyhow::Result;
use futures::future::join_all;

use kubectui::{
    app::{DetailViewState, ResourceRef},
    cronjob::{cronjob_history_entries, preferred_history_index},
    detail_sections::{metadata_for_resource, sections_for_resource},
    k8s::client::K8sClient,
    policy::DetailAction,
    state::ClusterSnapshot,
};

pub(crate) async fn fetch_detail_view(
    client: &K8sClient,
    snapshot: &ClusterSnapshot,
    resource: ResourceRef,
) -> Result<DetailViewState> {
    let mut metadata = metadata_for_resource(snapshot, &resource);
    let sections = sections_for_resource(snapshot, &resource);
    let (cronjob_history, cronjob_history_selected) = match &resource {
        ResourceRef::CronJob(name, ns) => snapshot
            .cronjobs
            .iter()
            .find(|cronjob| &cronjob.name == name && &cronjob.namespace == ns)
            .map(|cronjob| {
                let history = cronjob_history_entries(cronjob, &snapshot.jobs, &snapshot.pods);
                let selected = preferred_history_index(&history);
                (history, selected)
            })
            .unwrap_or_default(),
        _ => (Vec::new(), 0),
    };
    let cronjob_history = fetch_cronjob_history_log_access(client, cronjob_history).await;

    // Run YAML, events, and metrics fetches concurrently (no dependencies between them).
    let (
        (yaml, yaml_error),
        events,
        (pod_metrics, node_metrics, metrics_unavailable_message),
        action_authorizations,
    ) = tokio::join!(
        fetch_detail_yaml(client, &resource),
        fetch_detail_events(client, &resource),
        fetch_detail_metrics(client, &resource),
        client.fetch_detail_action_authorizations(&resource),
    );
    metadata.action_authorizations = action_authorizations;

    Ok(DetailViewState {
        resource: Some(resource),
        pending_request_id: None,
        metadata,
        yaml,
        yaml_error,
        events,
        sections,
        pod_metrics,
        node_metrics,
        metrics_unavailable_message,
        loading: false,
        error: None,
        debug_dialog: None,
        node_debug_dialog: None,
        scale_dialog: None,
        probe_panel: None,
        cronjob_history,
        cronjob_history_selected,
        top_panel_scroll: 0,
        confirm_delete: false,
        confirm_drain: false,
        confirm_cronjob_suspend: None,
        metadata_expanded: false,
    })
}

async fn fetch_cronjob_history_log_access(
    client: &K8sClient,
    mut entries: Vec<kubectui::cronjob::CronJobHistoryEntry>,
) -> Vec<kubectui::cronjob::CronJobHistoryEntry> {
    let checks = join_all(entries.iter().map(|entry| async {
        if entry.live_pod_count <= 0 {
            return None;
        }

        let resource = ResourceRef::Job(entry.job_name.clone(), entry.namespace.clone());
        client
            .is_detail_action_authorized(&resource, DetailAction::Logs)
            .await
            .map(|status| status.permits(DetailAction::Logs))
    }))
    .await;

    for (entry, allowed) in entries.iter_mut().zip(checks) {
        entry.logs_authorized = allowed;
    }

    entries
}

async fn fetch_detail_yaml(
    client: &K8sClient,
    resource: &ResourceRef,
) -> (Option<String>, Option<String>) {
    let result = match resource {
        ResourceRef::CustomResource {
            group,
            version,
            kind,
            plural,
            name,
            namespace,
        } => {
            client
                .fetch_custom_resource_yaml(
                    group,
                    version,
                    kind,
                    plural,
                    name,
                    namespace.as_deref(),
                )
                .await
        }
        ResourceRef::HelmRelease(name, ns) => client.fetch_helm_release_yaml(name, ns).await,
        _ => {
            let kind = resource.kind().to_ascii_lowercase();
            let name = resource.name();
            let namespace = resource.namespace();
            client.fetch_resource_yaml(&kind, name, namespace).await
        }
    };
    match result {
        Ok(yaml) => (Some(yaml), None),
        Err(e) => (None, Some(format!("YAML fetch failed: {e}"))),
    }
}

async fn fetch_detail_events(
    client: &K8sClient,
    resource: &ResourceRef,
) -> Vec<kubectui::k8s::events::EventInfo> {
    match resource {
        ResourceRef::Pod(name, ns) => client.fetch_pod_events(name, ns).await.unwrap_or_default(),
        ResourceRef::Deployment(name, ns)
        | ResourceRef::StatefulSet(name, ns)
        | ResourceRef::DaemonSet(name, ns)
        | ResourceRef::ReplicaSet(name, ns)
        | ResourceRef::Job(name, ns)
        | ResourceRef::CronJob(name, ns)
        | ResourceRef::Service(name, ns)
        | ResourceRef::Ingress(name, ns)
        | ResourceRef::ConfigMap(name, ns)
        | ResourceRef::Pvc(name, ns)
        | ResourceRef::HelmRelease(name, ns) => {
            let kind = resource.kind();
            client
                .fetch_resource_events(kind, name, ns)
                .await
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

async fn fetch_detail_metrics(
    client: &K8sClient,
    resource: &ResourceRef,
) -> (
    Option<kubectui::k8s::dtos::PodMetricsInfo>,
    Option<kubectui::k8s::dtos::NodeMetricsInfo>,
    Option<String>,
) {
    match resource {
        ResourceRef::Pod(name, ns) => match client.fetch_pod_metrics(name, ns).await {
            Ok(Some(metrics)) => (Some(metrics), None, None),
            Ok(None) => (
                None,
                None,
                Some(
                    "metrics unavailable (metrics-server not installed or inaccessible)"
                        .to_string(),
                ),
            ),
            Err(err) => (None, None, Some(format!("metrics unavailable: {err}"))),
        },
        ResourceRef::Node(name) => match client.fetch_node_metrics(name).await {
            Ok(Some(metrics)) => (None, Some(metrics), None),
            Ok(None) => (
                None,
                None,
                Some(
                    "metrics unavailable (metrics-server not installed or inaccessible)"
                        .to_string(),
                ),
            ),
            Err(err) => (None, None, Some(format!("metrics unavailable: {err}"))),
        },
        _ => (None, None, None),
    }
}
