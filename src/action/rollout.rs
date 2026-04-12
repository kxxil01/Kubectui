//! Workload rollout control center handlers.

use std::time::Instant;

use kubectui::{
    action_history::ActionKind,
    app::{AppState, ResourceRef},
    policy::DetailAction,
    workbench::{RolloutMutationState, WorkbenchTabKey, WorkbenchTabState},
};

use crate::{
    action::detail_tabs::redirect_blocked_detail_action_to_access_review,
    async_types::{RolloutInspectionAsyncResult, RolloutMutationAsyncResult, RolloutMutationKind},
    mutation_helpers::set_transient_status,
    next_request_id,
    selection_helpers::selected_resource,
};

pub fn spawn_rollout_inspection_fetch(
    tx: &tokio::sync::mpsc::Sender<RolloutInspectionAsyncResult>,
    client: &kubectui::k8s::client::K8sClient,
    resource: ResourceRef,
    request_id: u64,
) {
    let tx = tx.clone();
    let client = client.clone();
    tokio::spawn(async move {
        let result = client
            .fetch_rollout_inspection(&resource)
            .await
            .map_err(|err| err.to_string());
        let _ = tx
            .send(RolloutInspectionAsyncResult {
                request_id,
                resource,
                result,
            })
            .await;
    });
}

pub async fn handle_open_rollout(
    app: &mut AppState,
    client: &kubectui::k8s::client::K8sClient,
    snapshot: &kubectui::state::ClusterSnapshot,
    tx: &tokio::sync::mpsc::Sender<RolloutInspectionAsyncResult>,
    request_seq: &mut u64,
) -> bool {
    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(app, snapshot));
    let Some(resource) = resource else {
        app.set_error("No workload selected for rollout inspection.".to_string());
        return true;
    };
    if !matches!(
        resource,
        ResourceRef::Deployment(_, _)
            | ResourceRef::StatefulSet(_, _)
            | ResourceRef::DaemonSet(_, _)
    ) {
        app.set_error(
            "Rollout control is only available for Deployments, StatefulSets, and DaemonSets."
                .to_string(),
        );
        return true;
    }
    if redirect_blocked_detail_action_to_access_review(
        app,
        client,
        Some(snapshot),
        &resource,
        DetailAction::ViewRollout,
    )
    .await
    .is_some()
    {
        return true;
    }
    let request_id = next_request_id(request_seq);
    app.detail_view = None;
    app.open_rollout_tab(resource.clone(), None, None, Some(request_id));
    spawn_rollout_inspection_fetch(tx, client, resource, request_id);
    false
}

pub fn refresh_rollout_tab(
    app: &mut AppState,
    client: &kubectui::k8s::client::K8sClient,
    tx: &tokio::sync::mpsc::Sender<RolloutInspectionAsyncResult>,
    request_seq: &mut u64,
    resource: ResourceRef,
) {
    let Some(tab) = app
        .workbench_mut()
        .find_tab_mut(&WorkbenchTabKey::Rollout(resource.clone()))
    else {
        return;
    };
    let WorkbenchTabState::Rollout(rollout_tab) = &mut tab.state else {
        return;
    };
    let request_id = next_request_id(request_seq);
    rollout_tab.refresh(request_id);
    spawn_rollout_inspection_fetch(tx, client, resource, request_id);
}

pub async fn handle_rollout_restart(
    app: &mut AppState,
    client: &kubectui::k8s::client::K8sClient,
    snapshot: &kubectui::state::ClusterSnapshot,
    tx: &tokio::sync::mpsc::Sender<RolloutMutationAsyncResult>,
    context_generation: u64,
    status_message_clear_at: &mut Option<Instant>,
) -> bool {
    let resource = rollout_target_resource(app);
    let Some(resource) = resource else {
        app.set_error("Restart is unavailable for the selected resource.".to_string());
        return true;
    };
    if redirect_blocked_detail_action_to_access_review(
        app,
        client,
        Some(snapshot),
        &resource,
        DetailAction::Restart,
    )
    .await
    .is_some()
    {
        return true;
    }
    let (kind, name, namespace) = workload_identity(&resource);
    let resource_label = format!("{kind} '{name}' in namespace '{namespace}'");
    let origin_view = app.view();
    let action_history_id = app.record_action_pending(
        ActionKind::Restart,
        origin_view,
        Some(resource.clone()),
        resource_label.clone(),
        format!("Requesting restart for {resource_label}..."),
    );
    if let Some(tab) = rollout_tab_mut(app, &resource) {
        tab.begin_mutation(RolloutMutationState::Restart, action_history_id);
    }
    set_transient_status(
        app,
        status_message_clear_at,
        format!("Requesting restart for {resource_label}..."),
    );
    let tx = tx.clone();
    let client = client.clone();
    tokio::spawn(async move {
        let result = client
            .rollout_restart(kind, &name, &namespace)
            .await
            .map_err(|err| format!("{err:#}"));
        let _ = tx
            .send(RolloutMutationAsyncResult {
                action_history_id,
                context_generation,
                origin_view,
                resource,
                resource_label,
                kind: RolloutMutationKind::Restart,
                result,
            })
            .await;
    });
    false
}

pub async fn handle_toggle_rollout_pause_resume(
    app: &mut AppState,
    client: &kubectui::k8s::client::K8sClient,
    snapshot: &kubectui::state::ClusterSnapshot,
    tx: &tokio::sync::mpsc::Sender<RolloutMutationAsyncResult>,
    context_generation: u64,
    status_message_clear_at: &mut Option<Instant>,
) -> bool {
    let Some((resource, paused)) = app.workbench.active_tab().and_then(|tab| match &tab.state {
        WorkbenchTabState::Rollout(rollout_tab)
            if matches!(rollout_tab.resource, ResourceRef::Deployment(_, _)) =>
        {
            Some((rollout_tab.resource.clone(), rollout_tab.paused))
        }
        _ => None,
    }) else {
        app.set_error("Pause/resume is only available from a Deployment rollout tab.".to_string());
        return true;
    };
    let next_paused = !paused;
    if redirect_blocked_detail_action_to_access_review(
        app,
        client,
        Some(snapshot),
        &resource,
        if next_paused {
            DetailAction::PauseRollout
        } else {
            DetailAction::ResumeRollout
        },
    )
    .await
    .is_some()
    {
        return true;
    }
    let (_, name, namespace) = workload_identity(&resource);
    let action_kind = if next_paused {
        ActionKind::Pause
    } else {
        ActionKind::Resume
    };
    let mutation_kind = if next_paused {
        RolloutMutationKind::Pause
    } else {
        RolloutMutationKind::Resume
    };
    let resource_label = format!("deployment '{name}' in namespace '{namespace}'");
    let verb = if next_paused { "Pausing" } else { "Resuming" };
    let origin_view = app.view();
    let action_history_id = app.record_action_pending(
        action_kind,
        origin_view,
        Some(resource.clone()),
        resource_label.clone(),
        format!("{verb} rollout for {resource_label}..."),
    );
    if let Some(tab) = rollout_tab_mut(app, &resource) {
        tab.begin_mutation(
            if next_paused {
                RolloutMutationState::Pause
            } else {
                RolloutMutationState::Resume
            },
            action_history_id,
        );
    }
    set_transient_status(
        app,
        status_message_clear_at,
        format!("{verb} rollout for {resource_label}..."),
    );
    let tx = tx.clone();
    let client = client.clone();
    tokio::spawn(async move {
        let result = client
            .set_deployment_rollout_paused(&name, &namespace, next_paused)
            .await
            .map_err(|err| format!("{err:#}"));
        let _ = tx
            .send(RolloutMutationAsyncResult {
                action_history_id,
                context_generation,
                origin_view,
                resource,
                resource_label,
                kind: mutation_kind,
                result,
            })
            .await;
    });
    false
}

pub fn handle_confirm_rollout_undo(app: &mut AppState) -> bool {
    let Some(tab) = app.workbench.active_tab_mut() else {
        app.set_error("No active rollout tab for undo.".to_string());
        return true;
    };
    let WorkbenchTabState::Rollout(rollout_tab) = &mut tab.state else {
        app.set_error("Rollout undo is only available from the rollout tab.".to_string());
        return true;
    };
    let Some(revision) = rollout_tab.selected_undo_revision() else {
        app.set_error("Select an older revision before requesting rollout undo.".to_string());
        return true;
    };
    rollout_tab.begin_undo_confirm(revision);
    false
}

pub async fn handle_execute_rollout_undo(
    app: &mut AppState,
    client: &kubectui::k8s::client::K8sClient,
    snapshot: &kubectui::state::ClusterSnapshot,
    tx: &tokio::sync::mpsc::Sender<RolloutMutationAsyncResult>,
    context_generation: u64,
    status_message_clear_at: &mut Option<Instant>,
) -> bool {
    let (resource, target_revision) = {
        let Some(tab) = app.workbench.active_tab() else {
            app.set_error("No active rollout tab for undo.".to_string());
            return true;
        };
        let WorkbenchTabState::Rollout(rollout_tab) = &tab.state else {
            app.set_error("Rollout undo is only available from the rollout tab.".to_string());
            return true;
        };
        let Some(target_revision) = rollout_tab.confirm_undo_revision else {
            app.set_error("No rollout undo revision is selected.".to_string());
            return true;
        };
        (rollout_tab.resource.clone(), target_revision)
    };
    if redirect_blocked_detail_action_to_access_review(
        app,
        client,
        Some(snapshot),
        &resource,
        DetailAction::RollbackRollout,
    )
    .await
    .is_some()
    {
        clear_rollout_undo_confirm(app, &resource);
        return true;
    }
    let (kind, name, namespace) = workload_identity(&resource);
    let resource_label = format!("{kind} '{name}' in namespace '{namespace}'");
    let origin_view = app.view();
    let action_history_id = app.record_action_pending(
        ActionKind::Rollback,
        origin_view,
        Some(resource.clone()),
        resource_label.clone(),
        format!("Rolling back {resource_label} to revision {target_revision}..."),
    );
    if let Some(tab) = rollout_tab_mut(app, &resource) {
        tab.begin_mutation(
            RolloutMutationState::Undo(target_revision),
            action_history_id,
        );
    }
    set_transient_status(
        app,
        status_message_clear_at,
        format!("Rolling back {resource_label} to revision {target_revision}..."),
    );
    let tx = tx.clone();
    let client = client.clone();
    tokio::spawn(async move {
        let result = client
            .rollback_workload_to_revision(&resource, target_revision)
            .await
            .map_err(|err| format!("{err:#}"));
        let _ = tx
            .send(RolloutMutationAsyncResult {
                action_history_id,
                context_generation,
                origin_view,
                resource,
                resource_label,
                kind: RolloutMutationKind::Undo(target_revision),
                result,
            })
            .await;
    });
    false
}

fn clear_rollout_undo_confirm(app: &mut AppState, resource: &ResourceRef) {
    if let Some(tab) = rollout_tab_mut(app, resource) {
        tab.cancel_undo_confirm();
    }
}

fn rollout_target_resource(app: &AppState) -> Option<ResourceRef> {
    app.workbench
        .active_tab()
        .and_then(|tab| match &tab.state {
            WorkbenchTabState::Rollout(rollout_tab) => Some(rollout_tab.resource.clone()),
            _ => None,
        })
        .or_else(|| {
            app.detail_view
                .as_ref()
                .and_then(|detail| detail.resource.clone())
        })
        .filter(|resource| {
            matches!(
                resource,
                ResourceRef::Deployment(_, _)
                    | ResourceRef::StatefulSet(_, _)
                    | ResourceRef::DaemonSet(_, _)
            )
        })
}

fn rollout_tab_mut<'a>(
    app: &'a mut AppState,
    resource: &ResourceRef,
) -> Option<&'a mut kubectui::workbench::RolloutTabState> {
    app.workbench_mut()
        .find_tab_mut(&WorkbenchTabKey::Rollout(resource.clone()))
        .and_then(|tab| match &mut tab.state {
            WorkbenchTabState::Rollout(rollout_tab) => Some(rollout_tab),
            _ => None,
        })
}

fn workload_identity(resource: &ResourceRef) -> (&'static str, String, String) {
    match resource {
        ResourceRef::Deployment(name, namespace) => ("deployment", name.clone(), namespace.clone()),
        ResourceRef::StatefulSet(name, namespace) => {
            ("statefulset", name.clone(), namespace.clone())
        }
        ResourceRef::DaemonSet(name, namespace) => ("daemonset", name.clone(), namespace.clone()),
        _ => unreachable!("validated rollout resource"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_rollout_undo_confirm_cancels_pending_confirmation() {
        let resource = ResourceRef::Deployment("api".into(), "default".into());
        let mut app = AppState::default();
        let mut tab = kubectui::workbench::RolloutTabState::new(resource.clone());
        tab.begin_undo_confirm(4);
        app.workbench.open_tab(WorkbenchTabState::Rollout(tab));

        clear_rollout_undo_confirm(&mut app, &resource);

        let Some(tab) = app.workbench.active_tab() else {
            panic!("missing rollout tab");
        };
        let WorkbenchTabState::Rollout(tab) = &tab.state else {
            panic!("expected rollout tab");
        };
        assert!(tab.confirm_undo_revision.is_none());
    }
}
