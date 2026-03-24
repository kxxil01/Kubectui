//! Delete resource action handlers.

use std::time::{Duration, Instant};

use kubectui::{
    action_history::ActionKind,
    app::{AppState, AppView, ResourceRef},
    k8s::client::K8sClient,
    policy::DetailAction,
};

use crate::async_types::DeleteAsyncResult;
use crate::mutation_helpers::begin_detail_mutation;
use crate::selection_helpers::detail_action_block_message;

/// Spawns an async task to delete a Kubernetes resource.
#[allow(clippy::too_many_arguments)]
fn spawn_delete_task(
    delete_tx: tokio::sync::mpsc::Sender<DeleteAsyncResult>,
    client: K8sClient,
    resource: ResourceRef,
    request_id: u64,
    action_history_id: u64,
    context_generation: u64,
    origin_view: AppView,
    force: bool,
) {
    tokio::spawn(async move {
        let outcome = tokio::time::timeout(Duration::from_secs(20), async {
            match &resource {
                ResourceRef::CustomResource {
                    name,
                    namespace,
                    group,
                    version,
                    kind,
                    plural,
                } => {
                    client
                        .delete_custom_resource(
                            group,
                            version,
                            kind,
                            plural,
                            name,
                            namespace.as_deref(),
                        )
                        .await
                }
                _ => {
                    let kind = resource.kind().to_ascii_lowercase();
                    let name = resource.name().to_string();
                    let namespace = resource.namespace().map(str::to_owned);
                    if force {
                        client
                            .force_delete_resource(&kind, &name, namespace.as_deref())
                            .await
                    } else {
                        client
                            .delete_resource(&kind, &name, namespace.as_deref())
                            .await
                    }
                }
            }
        })
        .await;

        let result = match outcome {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.to_string()),
            Err(_) => Err("Delete request timed out after 20s".to_string()),
        };

        let _ = delete_tx
            .send(DeleteAsyncResult {
                request_id,
                action_history_id,
                context_generation,
                origin_view,
                resource,
                result,
            })
            .await;
    });
}

/// Handles graceful resource deletion.
///
/// Returns `true` if the caller should skip the rest of the action dispatch.
pub async fn handle_delete_resource(
    app: &mut AppState,
    client: &K8sClient,
    delete_tx: &tokio::sync::mpsc::Sender<DeleteAsyncResult>,
    delete_request_seq: &mut u64,
    delete_in_flight_id: &mut Option<u64>,
    context_generation: u64,
    status_message_clear_at: &mut Option<Instant>,
) -> bool {
    if let Some(detail) = &mut app.detail_view {
        detail.confirm_delete = false;
    }
    if !app
        .detail_view
        .as_ref()
        .is_some_and(|detail| detail.supports_action(DetailAction::Delete))
    {
        app.set_error("Delete is unavailable for the selected resource.".to_string());
        return true;
    }
    let delete_resource = app.detail_view.as_ref().and_then(|d| d.resource.clone());
    if let Some(resource) = delete_resource {
        if let Some(message) =
            detail_action_block_message(app, client, &resource, DetailAction::Delete).await
        {
            app.set_error(message);
            return true;
        }
        if delete_in_flight_id.is_some() {
            app.set_error("Delete already in progress".to_string());
            return true;
        }

        if let Some(detail) = &mut app.detail_view {
            detail.loading = true;
        }

        *delete_request_seq = delete_request_seq.wrapping_add(1);
        let request_id = *delete_request_seq;
        *delete_in_flight_id = Some(request_id);
        let resource_label = format!("{} '{}'", resource.kind(), resource.name());
        let origin_view = app.view();
        let action_history_id = app.record_action_pending(
            ActionKind::Delete,
            origin_view,
            Some(resource.clone()),
            resource_label.clone(),
            format!("Deleting {resource_label}..."),
        );
        begin_detail_mutation(
            app,
            status_message_clear_at,
            format!("Deleting {resource_label}..."),
        );
        spawn_delete_task(
            delete_tx.clone(),
            client.clone(),
            resource,
            request_id,
            action_history_id,
            context_generation,
            origin_view,
            false,
        );
    }
    false
}

/// Handles force resource deletion.
///
/// Returns `true` if the caller should skip the rest of the action dispatch.
pub async fn handle_force_delete_resource(
    app: &mut AppState,
    client: &K8sClient,
    delete_tx: &tokio::sync::mpsc::Sender<DeleteAsyncResult>,
    delete_request_seq: &mut u64,
    delete_in_flight_id: &mut Option<u64>,
    context_generation: u64,
    status_message_clear_at: &mut Option<Instant>,
) -> bool {
    if let Some(detail) = &mut app.detail_view {
        detail.confirm_delete = false;
    }
    if !app
        .detail_view
        .as_ref()
        .is_some_and(|detail| detail.supports_action(DetailAction::Delete))
    {
        app.set_error("Delete is unavailable for the selected resource.".to_string());
        return true;
    }
    let delete_resource = app.detail_view.as_ref().and_then(|d| d.resource.clone());
    if let Some(resource) = delete_resource {
        if let Some(message) =
            detail_action_block_message(app, client, &resource, DetailAction::Delete).await
        {
            app.set_error(message);
            return true;
        }
        if delete_in_flight_id.is_some() {
            app.set_error("Delete already in progress".to_string());
            return true;
        }

        if let Some(detail) = &mut app.detail_view {
            detail.loading = true;
        }

        *delete_request_seq = delete_request_seq.wrapping_add(1);
        let request_id = *delete_request_seq;
        *delete_in_flight_id = Some(request_id);
        let resource_label = format!("{} '{}'", resource.kind(), resource.name());
        let origin_view = app.view();
        let action_history_id = app.record_action_pending(
            ActionKind::Delete,
            origin_view,
            Some(resource.clone()),
            resource_label.clone(),
            format!("Force deleting {resource_label}..."),
        );
        begin_detail_mutation(
            app,
            status_message_clear_at,
            format!("Force deleting {resource_label}..."),
        );
        spawn_delete_task(
            delete_tx.clone(),
            client.clone(),
            resource,
            request_id,
            action_history_id,
            context_generation,
            origin_view,
            true,
        );
    }
    false
}
