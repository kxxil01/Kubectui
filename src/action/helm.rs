//! Helm release history, values diff, and rollback handlers.

use std::time::Instant;

use kubectui::{
    action_history::ActionKind,
    app::{AppState, ResourceRef},
    authorization::DetailActionAuthorization,
    k8s::helm,
    policy::DetailAction,
    workbench::{WorkbenchTabKey, WorkbenchTabState},
};

use crate::{
    action::detail_tabs::{
        build_helm_rollback_attempted_review, redirect_blocked_detail_action_to_access_review,
    },
    async_types::{HelmHistoryAsyncResult, HelmRollbackAsyncResult, HelmValuesDiffAsyncResult},
    mutation_helpers::set_transient_status,
    next_request_id,
    selection_helpers::selected_resource,
};

pub fn spawn_helm_history_fetch(
    history_tx: &tokio::sync::mpsc::Sender<HelmHistoryAsyncResult>,
    resource: ResourceRef,
    kube_context: Option<String>,
    request_id: u64,
    context_generation: u64,
) {
    let tx = history_tx.clone();
    tokio::spawn(async move {
        let result = match &resource {
            ResourceRef::HelmRelease(name, namespace) => {
                helm::fetch_release_history(name, namespace, kube_context)
                    .await
                    .map_err(|err| err.to_string())
            }
            _ => Err("Helm history is only available for Helm release resources.".to_string()),
        };
        let _ = tx
            .send(HelmHistoryAsyncResult {
                request_id,
                context_generation,
                resource,
                result,
            })
            .await;
    });
}

fn spawn_helm_values_diff_fetch(
    diff_tx: &tokio::sync::mpsc::Sender<HelmValuesDiffAsyncResult>,
    resource: ResourceRef,
    kube_context: Option<String>,
    current_revision: i32,
    target_revision: i32,
    request_id: u64,
    context_generation: u64,
) {
    let tx = diff_tx.clone();
    tokio::spawn(async move {
        let result = match &resource {
            ResourceRef::HelmRelease(name, namespace) => helm::fetch_release_values_diff(
                name,
                namespace,
                kube_context,
                current_revision,
                target_revision,
            )
            .await
            .map_err(|err| err.to_string()),
            _ => Err("Helm values diff is only available for Helm release resources.".to_string()),
        };
        let _ = tx
            .send(HelmValuesDiffAsyncResult {
                request_id,
                context_generation,
                resource,
                result,
            })
            .await;
    });
}

pub async fn handle_open_helm_history(
    app: &mut AppState,
    client: &kubectui::k8s::client::K8sClient,
    snapshot: &kubectui::state::ClusterSnapshot,
    history_tx: &tokio::sync::mpsc::Sender<HelmHistoryAsyncResult>,
    request_seq: &mut u64,
    context_generation: u64,
) -> bool {
    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(app, snapshot));
    let Some(resource) = resource else {
        app.set_error("No Helm release selected for history inspection.".to_string());
        return true;
    };
    let Some(ResourceRef::HelmRelease(_, _)) = Some(resource.clone()) else {
        app.set_error("Helm history is only available for Helm release resources.".to_string());
        return true;
    };
    if redirect_blocked_detail_action_to_access_review(
        app,
        client,
        Some(snapshot),
        &resource,
        DetailAction::ViewHelmHistory,
    )
    .await
    .is_some()
    {
        return true;
    }

    let request_id = next_request_id(request_seq);
    app.detail_view = None;
    app.open_helm_history_tab(resource.clone(), None, None, Some(request_id));
    spawn_helm_history_fetch(
        history_tx,
        resource,
        app.current_context_name.clone(),
        request_id,
        context_generation,
    );
    false
}

pub fn refresh_helm_history_tab(
    app: &mut AppState,
    history_tx: &tokio::sync::mpsc::Sender<HelmHistoryAsyncResult>,
    request_seq: &mut u64,
    resource: ResourceRef,
    context_generation: u64,
) {
    let request_id = next_request_id(request_seq);
    if let Some(tab) = app
        .workbench_mut()
        .find_tab_mut(&WorkbenchTabKey::HelmHistory(resource.clone()))
        && let WorkbenchTabState::HelmHistory(history_tab) = &mut tab.state
    {
        history_tab.refresh(request_id);
    }
    spawn_helm_history_fetch(
        history_tx,
        resource,
        app.current_context_name.clone(),
        request_id,
        context_generation,
    );
}

pub async fn handle_open_helm_values_diff(
    app: &mut AppState,
    client: &kubectui::k8s::client::K8sClient,
    snapshot: &kubectui::state::ClusterSnapshot,
    diff_tx: &tokio::sync::mpsc::Sender<HelmValuesDiffAsyncResult>,
    request_seq: &mut u64,
    context_generation: u64,
) -> bool {
    let kube_context = app.current_context_name.clone();
    let Some(tab) = app.workbench.active_tab() else {
        app.set_error("No active workbench tab for Helm values diff.".to_string());
        return true;
    };
    let WorkbenchTabState::HelmHistory(history_tab) = &tab.state else {
        app.set_error("Helm values diff is only available from the Helm history tab.".to_string());
        return true;
    };
    let Some(current_revision) = history_tab.current_revision else {
        app.set_error("Helm history has no current revision to compare against.".to_string());
        return true;
    };
    let Some(target_revision) = history_tab.selected_target_revision() else {
        app.set_error(
            "Select an older Helm revision to compare against the current release.".to_string(),
        );
        return true;
    };
    let resource = history_tab.resource.clone();
    if redirect_blocked_detail_action_to_access_review(
        app,
        client,
        Some(snapshot),
        &resource,
        DetailAction::ViewHelmValuesDiff,
    )
    .await
    .is_some()
    {
        return true;
    }

    let request_id = next_request_id(request_seq);
    let Some(tab) = app.workbench.active_tab_mut() else {
        app.set_error("No active workbench tab for Helm values diff.".to_string());
        return true;
    };
    let WorkbenchTabState::HelmHistory(history_tab) = &mut tab.state else {
        app.set_error("Helm values diff is only available from the Helm history tab.".to_string());
        return true;
    };
    history_tab.begin_diff(current_revision, target_revision, request_id);
    spawn_helm_values_diff_fetch(
        diff_tx,
        resource,
        kube_context,
        current_revision,
        target_revision,
        request_id,
        context_generation,
    );
    false
}

pub fn handle_confirm_helm_rollback(app: &mut AppState) -> bool {
    let Some(tab) = app.workbench.active_tab_mut() else {
        app.set_error("No active workbench tab for Helm rollback.".to_string());
        return true;
    };
    let WorkbenchTabState::HelmHistory(history_tab) = &mut tab.state else {
        app.set_error("Helm rollback is only available from the Helm history tab.".to_string());
        return true;
    };
    let Some(revision) = history_tab.selected_target_revision() else {
        app.set_error("Select an older Helm revision before requesting rollback.".to_string());
        return true;
    };
    history_tab.begin_rollback_confirm(revision);
    false
}

pub async fn handle_execute_helm_rollback(
    app: &mut AppState,
    client: &kubectui::k8s::client::K8sClient,
    snapshot: &kubectui::state::ClusterSnapshot,
    rollback_tx: &tokio::sync::mpsc::Sender<HelmRollbackAsyncResult>,
    context_generation: u64,
    status_message_clear_at: &mut Option<Instant>,
) -> bool {
    let kube_context = app.current_context_name.clone();
    let (name, namespace, current_revision, target_revision, resource) = {
        let Some(tab) = app.workbench.active_tab_mut() else {
            app.set_error("No active workbench tab for Helm rollback.".to_string());
            return true;
        };
        let WorkbenchTabState::HelmHistory(history_tab) = &mut tab.state else {
            app.set_error("Helm rollback is only available from the Helm history tab.".to_string());
            return true;
        };
        let Some(target_revision) = history_tab.confirm_rollback_revision else {
            app.set_error("No Helm rollback target revision is selected.".to_string());
            return true;
        };
        let Some(current_revision) = history_tab.current_revision else {
            app.set_error("Current Helm revision is unavailable for rollback review.".to_string());
            return true;
        };
        let Some(ResourceRef::HelmRelease(name, namespace)) = Some(history_tab.resource.clone())
        else {
            app.set_error("Helm rollback target is no longer valid.".to_string());
            return true;
        };
        let resource = ResourceRef::HelmRelease(name.clone(), namespace.clone());
        (name, namespace, current_revision, target_revision, resource)
    };
    let attempted_review = match build_helm_rollback_attempted_review(
        client,
        &name,
        &namespace,
        current_revision,
        target_revision,
    )
    .await
    {
        Ok(review) => review,
        Err(err) => {
            app.set_error(err);
            return true;
        }
    };
    if !attempted_review
        .authorization
        .unwrap_or(DetailActionAuthorization::Unknown)
        .permits(DetailAction::RollbackHelm)
    {
        clear_helm_rollback_confirm(app);
        match crate::action::detail_tabs::open_access_review_for_resource_with_attempted_review(
            app,
            client,
            Some(snapshot),
            resource,
            attempted_review,
        )
        .await
        {
            Ok(()) => {}
            Err(err) => app.set_error(err),
        }
        return true;
    }
    let resource_label = format!("Helm release '{name}' in namespace '{namespace}'");
    let origin_view = app.view();
    let action_history_id = app.record_action_pending(
        ActionKind::Rollback,
        origin_view,
        Some(resource.clone()),
        resource_label.clone(),
        format!("Rolling back {resource_label} to revision {target_revision}..."),
    );
    if let Some(tab) = app.workbench.active_tab_mut()
        && let WorkbenchTabState::HelmHistory(history_tab) = &mut tab.state
    {
        history_tab.begin_rollback(action_history_id);
    };
    let resource_label_message =
        format!("Rolling back {resource_label} to revision {target_revision}...");
    let tx = rollback_tx.clone();
    set_transient_status(app, status_message_clear_at, resource_label_message);

    tokio::spawn(async move {
        let result = helm::rollback_release(&name, &namespace, kube_context, target_revision)
            .await
            .map_err(|err| err.to_string());
        let _ = tx
            .send(HelmRollbackAsyncResult {
                action_history_id,
                context_generation,
                origin_view,
                resource,
                target_revision,
                result,
            })
            .await;
    });
    false
}

fn clear_helm_rollback_confirm(app: &mut AppState) {
    let Some(tab) = app.workbench.active_tab_mut() else {
        return;
    };
    let WorkbenchTabState::HelmHistory(history_tab) = &mut tab.state else {
        return;
    };
    history_tab.cancel_rollback_confirm();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_helm_rollback_confirm_cancels_pending_confirmation() {
        let mut app = AppState::default();
        let mut tab = kubectui::workbench::HelmHistoryTabState::new(ResourceRef::HelmRelease(
            "web".into(),
            "default".into(),
        ));
        tab.begin_rollback_confirm(3);
        app.workbench.open_tab(WorkbenchTabState::HelmHistory(tab));

        clear_helm_rollback_confirm(&mut app);

        let Some(tab) = app.workbench.active_tab() else {
            panic!("missing helm history tab");
        };
        let WorkbenchTabState::HelmHistory(tab) = &tab.state else {
            panic!("expected helm history tab");
        };
        assert!(tab.confirm_rollback_revision.is_none());
    }
}
