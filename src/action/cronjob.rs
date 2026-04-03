//! CronJob action handlers (trigger, suspend/resume).

use std::time::Instant;

use kubectui::{
    action_history::ActionKind,
    app::{AppState, ResourceRef},
    k8s::client::K8sClient,
    policy::DetailAction,
    state::ClusterSnapshot,
};

use crate::action::detail_tabs::redirect_blocked_detail_action_to_access_review;
use crate::async_types::{SetCronJobSuspendAsyncResult, TriggerCronJobAsyncResult};
use crate::mutation_helpers::begin_detail_mutation;

/// Handles triggering a CronJob to create a new Job.
///
/// Returns `true` if the caller should skip the rest of the action dispatch.
pub async fn handle_trigger_cronjob(
    app: &mut AppState,
    client: &K8sClient,
    snapshot: &ClusterSnapshot,
    trigger_cronjob_tx: &tokio::sync::mpsc::Sender<TriggerCronJobAsyncResult>,
    context_generation: u64,
    status_message_clear_at: &mut Option<Instant>,
) -> bool {
    let cronjob_info = app.detail_view.as_ref().and_then(|d| {
        if let Some(ResourceRef::CronJob(name, ns)) = &d.resource {
            Some((name.clone(), ns.clone()))
        } else {
            None
        }
    });
    if let Some((name, namespace)) = cronjob_info {
        let resource = ResourceRef::CronJob(name.clone(), namespace.clone());
        if redirect_blocked_detail_action_to_access_review(
            app,
            client,
            Some(snapshot),
            &resource,
            DetailAction::Trigger,
        )
        .await
        .is_some()
        {
            return true;
        }
        let resource_label = format!("CronJob '{name}'");
        let origin_view = app.view();
        let action_history_id = app.record_action_pending(
            ActionKind::Trigger,
            origin_view,
            app.detail_view.as_ref().and_then(|d| d.resource.clone()),
            resource_label.clone(),
            format!("Triggering {resource_label}..."),
        );
        begin_detail_mutation(
            app,
            status_message_clear_at,
            format!("Triggering {resource_label}..."),
        );
        let tx = trigger_cronjob_tx.clone();
        let c = client.clone();
        tokio::spawn(async move {
            let result = c
                .trigger_cronjob(&name, &namespace)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx
                .send(TriggerCronJobAsyncResult {
                    action_history_id,
                    context_generation,
                    origin_view,
                    resource_label,
                    result,
                })
                .await;
        });
    }
    false
}

/// Handles the confirmation dialog for suspending/resuming a CronJob.
pub fn handle_confirm_cronjob_suspend(app: &mut AppState, suspend: bool) {
    if let Some(detail) = &mut app.detail_view
        && (detail.supports_action(DetailAction::SuspendCronJob)
            || detail.supports_action(DetailAction::ResumeCronJob))
    {
        detail.confirm_cronjob_suspend = Some(suspend);
    }
}

/// Handles setting the suspend state on a CronJob.
///
/// Returns `true` if the caller should skip the rest of the action dispatch.
pub async fn handle_set_cronjob_suspend(
    app: &mut AppState,
    client: &K8sClient,
    snapshot: &ClusterSnapshot,
    cronjob_suspend_tx: &tokio::sync::mpsc::Sender<SetCronJobSuspendAsyncResult>,
    context_generation: u64,
    status_message_clear_at: &mut Option<Instant>,
    suspend: bool,
) -> bool {
    let cronjob_info = app.detail_view.as_ref().and_then(|d| {
        if let Some(ResourceRef::CronJob(name, ns)) = &d.resource {
            Some((name.clone(), ns.clone()))
        } else {
            None
        }
    });
    if let Some((name, namespace)) = cronjob_info {
        let resource = ResourceRef::CronJob(name.clone(), namespace.clone());
        let detail_action = if suspend {
            DetailAction::SuspendCronJob
        } else {
            DetailAction::ResumeCronJob
        };
        if redirect_blocked_detail_action_to_access_review(
            app,
            client,
            Some(snapshot),
            &resource,
            detail_action,
        )
        .await
        .is_some()
        {
            return true;
        }
        if let Some(detail) = &mut app.detail_view {
            detail.confirm_cronjob_suspend = None;
        }
        let resource_label = format!("CronJob '{name}'");
        let origin_view = app.view();
        let action_history_id = app.record_action_pending(
            if suspend {
                ActionKind::Suspend
            } else {
                ActionKind::Resume
            },
            origin_view,
            app.detail_view.as_ref().and_then(|d| d.resource.clone()),
            resource_label.clone(),
            format!(
                "{}ing {resource_label}...",
                if suspend { "Suspend" } else { "Resum" }
            ),
        );
        begin_detail_mutation(
            app,
            status_message_clear_at,
            format!(
                "{}ing {resource_label}...",
                if suspend { "Suspend" } else { "Resum" }
            ),
        );
        let tx = cronjob_suspend_tx.clone();
        let c = client.clone();
        tokio::spawn(async move {
            let result = c
                .set_cronjob_suspend(&name, &namespace, suspend)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx
                .send(SetCronJobSuspendAsyncResult {
                    action_history_id,
                    context_generation,
                    origin_view,
                    resource_label,
                    suspend,
                    result,
                })
                .await;
        });
    }
    false
}
