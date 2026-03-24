//! Ephemeral debug container dialog handlers.

use kubectui::{
    action_history::ActionKind,
    app::{AppState, ResourceRef},
    k8s::{client::K8sClient, exec::fetch_pod_containers},
    policy::DetailAction,
    ui::components::DebugContainerDialogState,
};

use crate::async_types::{DebugContainerDialogBootstrapResult, DebugContainerLaunchAsyncResult};
use crate::selection_helpers::detail_action_block_message;

pub async fn handle_debug_container_dialog_open(
    app: &mut AppState,
    client: &K8sClient,
    bootstrap_tx: &tokio::sync::mpsc::Sender<DebugContainerDialogBootstrapResult>,
    request_seq: &mut u64,
) -> bool {
    let Some(resource) = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
    else {
        app.set_error("Open Pod detail before launching a debug container.".to_string());
        return true;
    };
    let Some(ResourceRef::Pod(pod_name, namespace)) = Some(resource.clone()) else {
        app.set_error("Debug containers are only available for Pod resources.".to_string());
        return true;
    };
    if let Some(message) =
        detail_action_block_message(app, client, &resource, DetailAction::DebugContainer).await
    {
        app.set_error(message);
        return true;
    }

    *request_seq = request_seq.wrapping_add(1).max(1);
    let request_id = *request_seq;
    let mut dialog = DebugContainerDialogState::new(pod_name.clone(), namespace.clone());
    dialog.pending_request_id = Some(request_id);
    if let Some(detail) = app.detail_view.as_mut() {
        detail.debug_dialog = Some(dialog);
    }

    let tx = bootstrap_tx.clone();
    let client_clone = client.clone();
    tokio::spawn(async move {
        let result = fetch_pod_containers(&client_clone, &pod_name, &namespace)
            .await
            .map_err(|err| format!("{err:#}"));
        let _ = tx
            .send(DebugContainerDialogBootstrapResult {
                request_id,
                resource,
                result,
            })
            .await;
    });
    false
}

pub async fn handle_debug_container_dialog_submit(
    app: &mut AppState,
    client: &K8sClient,
    launch_tx: &tokio::sync::mpsc::Sender<DebugContainerLaunchAsyncResult>,
    next_exec_session_id: &mut u64,
    context_generation: u64,
) -> bool {
    let Some(resource) = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
    else {
        app.set_error("No Pod detail is open for debug container launch.".to_string());
        return true;
    };
    let Some(ResourceRef::Pod(pod_name, namespace)) = Some(resource.clone()) else {
        app.set_error("Debug containers are only available for Pod resources.".to_string());
        return true;
    };
    if !app
        .detail_view
        .as_ref()
        .is_some_and(|detail| detail.supports_action(DetailAction::DebugContainer))
    {
        app.set_error("Debug container launch is unavailable for the selected Pod.".to_string());
        return true;
    }
    if let Some(message) =
        detail_action_block_message(app, client, &resource, DetailAction::DebugContainer).await
    {
        app.set_error(message);
        return true;
    }

    let Some(dialog) = app
        .detail_view
        .as_mut()
        .and_then(|detail| detail.debug_dialog.as_mut())
    else {
        app.set_error("Debug container dialog is not open.".to_string());
        return true;
    };
    let request = match dialog.build_launch_request() {
        Ok(request) => request,
        Err(error) => {
            dialog.error_message = Some(error);
            return true;
        }
    };

    let session_id = *next_exec_session_id;
    *next_exec_session_id = next_exec_session_id.wrapping_add(1).max(1);
    dialog.set_pending_launch(true);

    let resource_label = format!("Pod '{pod_name}' in namespace '{namespace}'");
    let origin_view = app.view();
    let action_history_id = app.record_action_pending(
        ActionKind::DebugContainer,
        origin_view,
        Some(resource.clone()),
        resource_label.clone(),
        format!("Launching debug container for {resource_label}..."),
    );

    let tx = launch_tx.clone();
    let client_clone = client.clone();
    tokio::spawn(async move {
        let result = client_clone
            .launch_debug_container(&request)
            .await
            .map_err(|err| format!("{err:#}"));
        let _ = tx
            .send(DebugContainerLaunchAsyncResult {
                action_history_id,
                context_generation,
                origin_view,
                resource,
                session_id,
                result,
            })
            .await;
    });
    false
}
