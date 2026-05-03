//! Scale dialog action handlers.

use std::time::Instant;

use kubectui::{
    action_history::ActionKind,
    app::{AppState, ResourceRef},
    k8s::client::K8sClient,
    policy::DetailAction,
    state::ClusterSnapshot,
    ui::components::scale_dialog::{ScaleDialogState, ScaleTargetKind},
};

use crate::action::detail_tabs::redirect_blocked_detail_action_to_access_review;
use crate::async_types::ScaleAsyncResult;
use crate::mutation_helpers::begin_detail_mutation;

/// Opens the scale dialog, reading the current replica count from the snapshot.
///
/// Returns `true` if the caller should skip the rest of the action dispatch
/// (equivalent to `continue` in the original event loop).
pub fn handle_scale_dialog_open(app: &mut AppState, cached_snapshot: &ClusterSnapshot) -> bool {
    if !app
        .detail_view
        .as_ref()
        .is_some_and(|detail| detail.supports_action(DetailAction::Scale))
    {
        app.set_error("Scale is unavailable for the selected resource.".to_string());
        return true;
    }
    let scale_info = app.detail_view.as_ref().and_then(|d| {
        d.resource.as_ref().and_then(|r| match r {
            ResourceRef::Deployment(name, ns) => {
                let replicas = cached_snapshot
                    .deployments
                    .iter()
                    .find(|d| &d.name == name && &d.namespace == ns)
                    .map(|d| d.desired_replicas)
                    .unwrap_or(1);
                Some((name.clone(), ns.clone(), replicas))
            }
            ResourceRef::StatefulSet(name, ns) => {
                let replicas = cached_snapshot
                    .statefulsets
                    .iter()
                    .find(|ss| &ss.name == name && &ss.namespace == ns)
                    .map(|ss| ss.desired_replicas)
                    .unwrap_or(1);
                Some((name.clone(), ns.clone(), replicas))
            }
            _ => None,
        })
    });
    if let Some((name, namespace, replicas)) = scale_info
        && let Some(detail) = &mut app.detail_view
    {
        detail.scale_dialog = Some(ScaleDialogState::new(
            match detail.resource.as_ref() {
                Some(ResourceRef::Deployment(_, _)) => ScaleTargetKind::Deployment,
                Some(ResourceRef::StatefulSet(_, _)) => ScaleTargetKind::StatefulSet,
                _ => ScaleTargetKind::Deployment,
            },
            name,
            namespace,
            replicas,
        ));
    }
    false
}

/// Submits the scale dialog, spawning an async task to perform the scale operation.
///
/// Returns `true` if the caller should skip the rest of the action dispatch.
pub async fn handle_scale_dialog_submit(
    app: &mut AppState,
    client: &K8sClient,
    snapshot: &ClusterSnapshot,
    scale_tx: &tokio::sync::mpsc::Sender<ScaleAsyncResult>,
    context_generation: u64,
    status_message_clear_at: &mut Option<Instant>,
) -> bool {
    if !app
        .detail_view
        .as_ref()
        .is_some_and(|detail| detail.supports_action(DetailAction::Scale))
    {
        app.set_error("Scale is unavailable for the selected resource.".to_string());
        return true;
    }
    let scale_info = app.detail_view.as_ref().and_then(|d| {
        let replicas = d.scale_dialog.as_ref()?.desired_replicas_as_int()?;
        match d.resource.as_ref()? {
            ResourceRef::Deployment(name, namespace) => Some((
                ResourceRef::Deployment(name.clone(), namespace.clone()),
                ScaleTargetKind::Deployment,
                name.clone(),
                namespace.clone(),
                "Deployment",
                replicas,
            )),
            ResourceRef::StatefulSet(name, namespace) => Some((
                ResourceRef::StatefulSet(name.clone(), namespace.clone()),
                ScaleTargetKind::StatefulSet,
                name.clone(),
                namespace.clone(),
                "StatefulSet",
                replicas,
            )),
            _ => None,
        }
    });
    if let Some((resource, target_kind, name, namespace, kind_label, replicas)) = scale_info {
        if redirect_blocked_detail_action_to_access_review(
            app,
            client,
            Some(snapshot),
            &resource,
            DetailAction::Scale,
        )
        .await
        .is_some()
        {
            return true;
        }
        let resource_label = format!("{kind_label} '{name}' in namespace '{namespace}'");
        let origin_view = app.view();
        let action_history_id = app.record_action_pending(
            ActionKind::Scale,
            origin_view,
            Some(resource.clone()),
            resource_label.clone(),
            format!("Scaling {resource_label} to {replicas}..."),
        );
        begin_detail_mutation(
            app,
            status_message_clear_at,
            format!("Scaling {resource_label} to {replicas}..."),
        );
        let tx = scale_tx.clone();
        let c = client.clone();
        tokio::spawn(async move {
            let result = match target_kind {
                ScaleTargetKind::Deployment => {
                    c.scale_deployment(&name, &namespace, replicas).await
                }
                ScaleTargetKind::StatefulSet => {
                    c.scale_statefulset(&name, &namespace, replicas).await
                }
            }
            .map_err(|e| format!("{e:#}"));
            let _ = tx
                .send(ScaleAsyncResult {
                    action_history_id,
                    context_generation,
                    origin_view,
                    resource,
                    target_replicas: replicas,
                    resource_label,
                    result,
                })
                .await;
        });
    }
    false
}
