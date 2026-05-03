//! Detail tab action handlers (YAML, decoded secret, events, relationships, bookmarks).

use kubectui::{
    app::{AppState, AppView, ResourceRef},
    authorization::{
        ActionAccessReview, DetailActionAuthorization, detail_action_requires_strict_authorization,
    },
    bookmarks::BookmarkToggleResult,
    k8s::client::K8sClient,
    network_policy_analysis, network_policy_connectivity,
    policy::{DetailAction, ResourceActionContext},
    rbac_subjects::{AccessReviewSubject, resolve_subject_access_review},
    secret::decode_secret_yaml,
    state::ClusterSnapshot,
    traffic_debug,
    workbench::{
        AttemptedActionReview, ConnectivityTargetOption, WorkbenchTabKey, WorkbenchTabState,
    },
};

use crate::async_types::{DetailAsyncResult, RelationsAsyncResult, ResourceDiffAsyncResult};
use crate::next_request_id;
use crate::selection_helpers::{
    detail_action_authorization, detail_action_denied_message, resource_action_context,
    selected_resource,
};

/// Spawns an async detail-view fetch for a resource.
fn spawn_detail_fetch(
    detail_tx: &tokio::sync::mpsc::Sender<DetailAsyncResult>,
    client: &K8sClient,
    snapshot: &ClusterSnapshot,
    resource: ResourceRef,
    request_id: u64,
    context_generation: u64,
) {
    let tx = detail_tx.clone();
    let client_clone = client.clone();
    let snapshot_clone = snapshot.clone();
    tokio::spawn(async move {
        let result = crate::fetch_detail_view(&client_clone, &snapshot_clone, resource.clone())
            .await
            .map_err(|err| err.to_string());
        let _ = tx
            .send(DetailAsyncResult {
                request_id,
                context_generation,
                resource,
                result,
            })
            .await;
    });
}

fn spawn_resource_diff_fetch(
    diff_tx: &tokio::sync::mpsc::Sender<ResourceDiffAsyncResult>,
    client: &K8sClient,
    resource: ResourceRef,
    request_id: u64,
    context_generation: u64,
) {
    let tx = diff_tx.clone();
    let client_clone = client.clone();
    tokio::spawn(async move {
        let result = client_clone
            .fetch_resource_diff_source_yaml(&resource)
            .await
            .map_err(|err| err.to_string());
        let _ = tx
            .send(ResourceDiffAsyncResult {
                request_id,
                context_generation,
                resource,
                result,
            })
            .await;
    });
}

/// Handles `AppAction::OpenResourceYaml`.
///
/// Returns `true` if the caller should skip the rest of the action dispatch.
pub async fn handle_open_resource_yaml(
    app: &mut AppState,
    client: &K8sClient,
    snapshot: &ClusterSnapshot,
    detail_tx: &tokio::sync::mpsc::Sender<DetailAsyncResult>,
    detail_request_seq: &mut u64,
    context_generation: u64,
) -> bool {
    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(app, snapshot));
    let Some(resource) = resource else {
        app.set_error("No resource selected for YAML inspection.".to_string());
        return true;
    };
    if redirect_blocked_detail_action_to_access_review(
        app,
        client,
        Some(snapshot),
        &resource,
        DetailAction::ViewYaml,
    )
    .await
    .is_some()
    {
        return true;
    }
    let cached_yaml = app
        .detail_view
        .as_ref()
        .and_then(|detail| {
            (detail.resource.as_ref() == Some(&resource)).then(|| detail.yaml.clone())
        })
        .flatten();
    let cached_yaml_error = app
        .detail_view
        .as_ref()
        .and_then(|detail| {
            (detail.resource.as_ref() == Some(&resource)).then(|| detail.yaml_error.clone())
        })
        .flatten();
    let pending_request_id = (cached_yaml.is_none() && cached_yaml_error.is_none())
        .then(|| next_request_id(detail_request_seq));
    app.detail_view = None;
    app.open_resource_yaml_tab(
        resource.clone(),
        cached_yaml.clone(),
        cached_yaml_error,
        pending_request_id,
    );
    if let Some(request_id) = pending_request_id {
        spawn_detail_fetch(
            detail_tx,
            client,
            snapshot,
            resource,
            request_id,
            context_generation,
        );
    }
    false
}

/// Handles `AppAction::OpenResourceDiff`.
///
/// Returns `true` if the caller should skip the rest of the action dispatch.
pub async fn handle_open_resource_diff(
    app: &mut AppState,
    client: &K8sClient,
    snapshot: &ClusterSnapshot,
    diff_tx: &tokio::sync::mpsc::Sender<ResourceDiffAsyncResult>,
    diff_request_seq: &mut u64,
    context_generation: u64,
) -> bool {
    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(app, snapshot));
    let Some(resource) = resource else {
        app.set_error("No resource selected for configuration drift inspection.".to_string());
        return true;
    };
    if redirect_blocked_detail_action_to_access_review(
        app,
        client,
        Some(snapshot),
        &resource,
        DetailAction::ViewConfigDrift,
    )
    .await
    .is_some()
    {
        return true;
    }

    let request_id = next_request_id(diff_request_seq);

    app.detail_view = None;
    app.open_resource_diff_tab(resource.clone(), None, None, Some(request_id));
    spawn_resource_diff_fetch(diff_tx, client, resource, request_id, context_generation);
    false
}

/// Handles `AppAction::OpenDecodedSecret`.
///
/// Returns `true` if the caller should skip the rest of the action dispatch.
pub async fn handle_open_decoded_secret(
    app: &mut AppState,
    client: &K8sClient,
    snapshot: &ClusterSnapshot,
    detail_tx: &tokio::sync::mpsc::Sender<DetailAsyncResult>,
    detail_request_seq: &mut u64,
    context_generation: u64,
) -> bool {
    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(app, snapshot));
    let Some(resource) = resource else {
        app.set_error("No Secret selected for decoded inspection.".to_string());
        return true;
    };
    if !matches!(resource, ResourceRef::Secret(_, _)) {
        app.set_error("Decoded Secret view is only available for Secret resources.".to_string());
        return true;
    }
    if redirect_blocked_detail_action_to_access_review(
        app,
        client,
        Some(snapshot),
        &resource,
        DetailAction::ViewDecodedSecret,
    )
    .await
    .is_some()
    {
        return true;
    }
    let cached_yaml = app
        .detail_view
        .as_ref()
        .and_then(|detail| {
            (detail.resource.as_ref() == Some(&resource)).then(|| detail.yaml.clone())
        })
        .flatten();
    let cached_error = app
        .detail_view
        .as_ref()
        .and_then(|detail| {
            (detail.resource.as_ref() == Some(&resource)).then(|| detail.yaml_error.clone())
        })
        .flatten();
    let pending_request_id = (cached_yaml.is_none() && cached_error.is_none())
        .then(|| next_request_id(detail_request_seq));
    app.detail_view = None;
    app.open_decoded_secret_tab(
        resource.clone(),
        cached_yaml.clone(),
        cached_error,
        pending_request_id,
    );
    if let Some(tab) = app
        .workbench_mut()
        .find_tab_mut(&WorkbenchTabKey::DecodedSecret(resource.clone()))
        && let WorkbenchTabState::DecodedSecret(secret_tab) = &mut tab.state
        && let Some(yaml) = cached_yaml.as_deref()
        && !secret_tab.has_local_edit_state()
    {
        match decode_secret_yaml(yaml) {
            Ok(entries) => {
                secret_tab.source_yaml = Some(yaml.to_string());
                secret_tab.entries = entries;
                secret_tab.loading = false;
                secret_tab.error = None;
                secret_tab.clamp_selected();
            }
            Err(err) => {
                secret_tab.loading = false;
                secret_tab.error = Some(err.to_string());
            }
        }
    }
    if let Some(request_id) = pending_request_id {
        spawn_detail_fetch(
            detail_tx,
            client,
            snapshot,
            resource,
            request_id,
            context_generation,
        );
    }
    false
}

/// Handles `AppAction::OpenResourceEvents`.
///
/// Returns `true` if the caller should skip the rest of the action dispatch.
pub async fn handle_open_resource_events(
    app: &mut AppState,
    client: &K8sClient,
    snapshot: &ClusterSnapshot,
    detail_tx: &tokio::sync::mpsc::Sender<DetailAsyncResult>,
    detail_request_seq: &mut u64,
    context_generation: u64,
) -> bool {
    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(app, snapshot));
    let Some(resource) = resource else {
        app.set_error("No resource selected for event inspection.".to_string());
        return true;
    };
    if !resource.supports_events_tab() {
        app.set_error(format!(
            "Events are not available for {} '{}'.",
            resource.kind(),
            resource.name()
        ));
        return true;
    }
    if redirect_blocked_detail_action_to_access_review(
        app,
        client,
        Some(snapshot),
        &resource,
        DetailAction::ViewEvents,
    )
    .await
    .is_some()
    {
        return true;
    }
    let cached_detail = app
        .detail_view
        .as_ref()
        .filter(|detail| detail.resource.as_ref() == Some(&resource));
    let cached_events = cached_detail
        .map(|detail| detail.events.clone())
        .unwrap_or_default();
    let cached_events_error = cached_detail.and_then(|detail| detail.events_error.clone());
    let cached_events_loaded = cached_detail
        .map(|detail| !detail.loading && detail.error.is_none())
        .unwrap_or(false);
    let loading =
        !cached_events_loaded && cached_events.is_empty() && cached_events_error.is_none();
    let pending_request_id = loading.then(|| next_request_id(detail_request_seq));
    app.detail_view = None;
    app.open_resource_events_tab(
        resource.clone(),
        cached_events,
        loading,
        cached_events_error,
        pending_request_id,
    );
    if let Some(request_id) = pending_request_id {
        spawn_detail_fetch(
            detail_tx,
            client,
            snapshot,
            resource,
            request_id,
            context_generation,
        );
    }
    false
}

/// Handles `AppAction::OpenAccessReview`.
pub async fn handle_open_access_review(
    app: &mut AppState,
    client: &K8sClient,
    snapshot: &ClusterSnapshot,
) -> bool {
    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(app, snapshot));
    let Some(resource) = resource else {
        app.set_error("No resource selected for access review.".to_string());
        return true;
    };

    match open_access_review_for_resource(app, client, Some(snapshot), resource, None).await {
        Ok(()) => false,
        Err(err) => {
            app.set_error(err);
            true
        }
    }
}

async fn resolve_access_review_context(
    app: &AppState,
    client: &K8sClient,
    snapshot: Option<&ClusterSnapshot>,
    resource: &ResourceRef,
) -> Result<ResourceActionContext, String> {
    if let Some(resource_ctx) = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource_action_context())
        .filter(|resource_ctx| &resource_ctx.resource == resource)
    {
        return Ok(resource_ctx);
    }

    let Some(snapshot) = snapshot else {
        return Err(format!(
            "Access review for {} '{}' requires snapshot context.",
            resource.kind(),
            resource.name()
        ));
    };

    let mut resource_ctx = resource_action_context(snapshot, resource.clone());
    resource_ctx.action_authorizations = client
        .fetch_detail_action_authorizations(&resource_ctx.resource)
        .await;
    if let Some(log_resource) = resource_ctx.effective_logs_resource.as_ref() {
        resource_ctx.effective_logs_authorization = client
            .is_detail_action_authorized(log_resource, DetailAction::Logs)
            .await;
    }
    Ok(resource_ctx)
}

pub async fn open_access_review_for_resource(
    app: &mut AppState,
    client: &K8sClient,
    snapshot: Option<&ClusterSnapshot>,
    resource: ResourceRef,
    attempted_action: Option<DetailAction>,
) -> Result<(), String> {
    let resource_ctx = resolve_access_review_context(app, client, snapshot, &resource).await?;
    let entries = resource_ctx.access_review_entries();
    let subject_review = AccessReviewSubject::from_resource(&resource)
        .and_then(|subject| snapshot.map(|loaded| resolve_subject_access_review(loaded, subject)));
    let attempted_review = attempted_action
        .map(|action| attempted_review_from_resource_context(&resource_ctx, action));
    open_access_review_tab_with_state(app, resource, entries, subject_review, attempted_review);
    Ok(())
}

pub async fn open_access_review_for_resource_with_attempted_review(
    app: &mut AppState,
    client: &K8sClient,
    snapshot: Option<&ClusterSnapshot>,
    resource: ResourceRef,
    attempted_review: AttemptedActionReview,
) -> Result<(), String> {
    let resource_ctx = resolve_access_review_context(app, client, snapshot, &resource).await?;
    let entries = resource_ctx.access_review_entries();
    let subject_review = AccessReviewSubject::from_resource(&resource)
        .and_then(|subject| snapshot.map(|loaded| resolve_subject_access_review(loaded, subject)));
    open_access_review_tab_with_state(
        app,
        resource,
        entries,
        subject_review,
        Some(attempted_review),
    );
    Ok(())
}

pub async fn build_helm_rollback_attempted_review(
    client: &K8sClient,
    release_name: &str,
    namespace: &str,
    current_revision: i32,
    target_revision: i32,
) -> Result<AttemptedActionReview, String> {
    let checks = client
        .helm_rollback_access_checks(release_name, namespace, current_revision, target_revision)
        .await
        .map_err(|err| {
            format!(
                "Failed to derive Helm rollback access review for revision {current_revision} -> {target_revision}: {err:#}"
            )
        })?;
    let authorization =
        DetailActionAuthorization::from_allowed(client.evaluate_access_checks(&checks).await);
    Ok(AttemptedActionReview {
        action: DetailAction::RollbackHelm,
        authorization: Some(authorization),
        strict: detail_action_requires_strict_authorization(DetailAction::RollbackHelm),
        checks,
        note: Some(format!(
            "Derived from the Helm manifest transition {current_revision} -> {target_revision}, plus release history secret access."
        )),
    })
}

fn open_access_review_tab_with_state(
    app: &mut AppState,
    resource: ResourceRef,
    entries: Vec<ActionAccessReview>,
    subject_review: Option<kubectui::rbac_subjects::SubjectAccessReview>,
    attempted_review: Option<AttemptedActionReview>,
) {
    app.detail_view = None;
    app.open_access_review_tab(
        resource,
        app.current_context_name.clone(),
        app.current_namespace.clone(),
        entries,
        subject_review,
        attempted_review,
    );
}

fn attempted_review_from_resource_context(
    resource_ctx: &ResourceActionContext,
    action: DetailAction,
) -> AttemptedActionReview {
    let entry = resource_ctx
        .access_review_for_action(action)
        .unwrap_or_else(|| ActionAccessReview {
            action,
            authorization: resource_ctx.authorization_for_action(action),
            strict: detail_action_requires_strict_authorization(action),
            checks: Vec::new(),
        });
    AttemptedActionReview {
        action: entry.action,
        authorization: entry.authorization,
        strict: entry.strict,
        checks: entry.checks,
        note: None,
    }
}

pub async fn redirect_blocked_detail_action_to_access_review(
    app: &mut AppState,
    client: &K8sClient,
    snapshot: Option<&ClusterSnapshot>,
    resource: &ResourceRef,
    action: DetailAction,
) -> Option<String> {
    let status = detail_action_authorization(app, client, resource, action)
        .await
        .unwrap_or(DetailActionAuthorization::Unknown);
    if status.permits(action) {
        return None;
    }

    let denial_message = detail_action_denied_message(action, resource, status);
    match open_access_review_for_resource(app, client, snapshot, resource.clone(), Some(action))
        .await
    {
        Ok(()) => Some(denial_message),
        Err(_) => {
            app.set_error(denial_message.clone());
            Some(denial_message)
        }
    }
}

pub fn handle_apply_access_review_subject(app: &mut AppState, snapshot: &ClusterSnapshot) -> bool {
    let Some(tab) = app.workbench.active_tab_mut() else {
        return true;
    };
    let WorkbenchTabState::AccessReview(access_tab) = &mut tab.state else {
        return true;
    };

    let input = access_tab.subject_input.value.trim();
    if input.is_empty() {
        access_tab.subject_review = None;
        access_tab.subject_input_error = None;
        access_tab.subject_input.error = false;
        access_tab.scroll = access_tab.subject_input_offset();
        access_tab.stop_subject_input();
        return true;
    }

    match AccessReviewSubject::parse(input) {
        Ok(subject) => {
            access_tab.subject_input.value = subject.spec();
            access_tab.subject_input.cursor_end();
            access_tab.subject_input_error = None;
            access_tab.subject_input.error = false;
            access_tab.subject_review = Some(resolve_subject_access_review(snapshot, subject));
            access_tab.scroll = access_tab.subject_review_offset().saturating_sub(1);
            access_tab.stop_subject_input();
            true
        }
        Err(err) => {
            access_tab.subject_review = None;
            access_tab.subject_input_error = Some(err);
            access_tab.subject_input.error = true;
            access_tab.scroll = access_tab.subject_input_offset();
            true
        }
    }
}

/// Handles `AppAction::OpenRelationships`.
///
/// Returns `true` if the caller should skip the rest of the action dispatch.
pub fn handle_open_relationships(
    app: &mut AppState,
    snapshot: &ClusterSnapshot,
    client: &K8sClient,
    relations_tx: &tokio::sync::mpsc::Sender<RelationsAsyncResult>,
    relations_request_seq: &mut u64,
    context_generation: u64,
) -> bool {
    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(app, snapshot));
    let Some(resource) = resource else {
        app.set_error("No resource selected for relationship exploration.".to_string());
        return true;
    };
    app.detail_view = None;
    let request_id = next_request_id(relations_request_seq);
    let key = kubectui::workbench::WorkbenchTabKey::Relations(resource.clone());
    if let Some(tab) = app.workbench.find_tab_mut(&key)
        && let WorkbenchTabState::Relations(relations_tab) = &mut tab.state
    {
        relations_tab.loading = true;
        relations_tab.error = None;
        relations_tab.pending_request_id = Some(request_id);
        app.workbench.activate_tab(&key);
        app.focus = kubectui::app::Focus::Workbench;
    } else {
        let mut relations_tab = kubectui::workbench::RelationsTabState::new(resource.clone());
        relations_tab.pending_request_id = Some(request_id);
        app.workbench
            .open_tab(WorkbenchTabState::Relations(relations_tab));
        app.focus = kubectui::app::Focus::Workbench;
    }

    let tx = relations_tx.clone();
    let client_clone = client.clone();
    let snapshot_clone = snapshot.clone();
    let requested_resource = resource.clone();
    tokio::spawn(async move {
        let result = kubectui::k8s::relationships::resolve_relationships(
            &requested_resource,
            &snapshot_clone,
            &client_clone,
        )
        .await
        .map_err(|err| format!("{err:#}"));
        let _ = tx
            .send(RelationsAsyncResult {
                request_id,
                context_generation,
                resource: requested_resource,
                result,
            })
            .await;
    });
    false
}

/// Handles `AppAction::OpenNetworkPolicyView`.
pub async fn handle_open_network_policies(
    app: &mut AppState,
    client: &K8sClient,
    snapshot: &ClusterSnapshot,
) -> bool {
    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(app, snapshot));
    let Some(resource) = resource else {
        app.set_error("No resource selected for network policy inspection.".to_string());
        return true;
    };
    if redirect_blocked_detail_action_to_access_review(
        app,
        client,
        Some(snapshot),
        &resource,
        DetailAction::ViewNetworkPolicies,
    )
    .await
    .is_some()
    {
        return true;
    }

    app.detail_view = None;
    match network_policy_analysis::analyze_resource(&resource, snapshot) {
        Ok(analysis) => app.open_network_policy_tab(resource, Some(analysis), None),
        Err(err) => app.open_network_policy_tab(resource, None, Some(err)),
    }
    false
}

/// Handles `AppAction::OpenNetworkConnectivity`.
pub async fn handle_open_network_connectivity(
    app: &mut AppState,
    client: &K8sClient,
    snapshot: &ClusterSnapshot,
) -> bool {
    if app.focus == kubectui::app::Focus::Workbench {
        let Some(tab) = app.workbench_mut().active_tab_mut() else {
            return false;
        };
        if let WorkbenchTabState::Connectivity(connectivity_tab) = &mut tab.state {
            let Some(target) = connectivity_tab
                .selected_target_option()
                .map(|target| target.resource.clone())
            else {
                connectivity_tab.error =
                    Some("No target pod matches the current filter.".to_string());
                return true;
            };

            match network_policy_connectivity::analyze_connectivity(
                &connectivity_tab.source,
                &target,
                snapshot,
            ) {
                Ok(analysis) => connectivity_tab.apply_analysis(target, analysis),
                Err(err) => connectivity_tab.set_error(err),
            }
            return true;
        }
    }

    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(app, snapshot));
    let Some(resource) = resource else {
        app.set_error("No Pod selected for connectivity inspection.".to_string());
        return true;
    };
    if !matches!(resource, ResourceRef::Pod(_, _)) {
        app.set_error("Connectivity inspection is only available for Pod resources.".to_string());
        return true;
    }
    if redirect_blocked_detail_action_to_access_review(
        app,
        client,
        Some(snapshot),
        &resource,
        DetailAction::CheckNetworkConnectivity,
    )
    .await
    .is_some()
    {
        return true;
    }

    let mut targets = snapshot
        .pods
        .iter()
        .filter_map(|pod| {
            let target = ResourceRef::Pod(pod.name.clone(), pod.namespace.clone());
            (target != resource).then_some(ConnectivityTargetOption {
                display: format!("{}/{}", pod.namespace, pod.name),
                resource: target,
                status: pod.status.clone(),
                pod_ip: pod.pod_ip.clone(),
            })
        })
        .collect::<Vec<_>>();
    targets.sort_by(|left, right| left.display.cmp(&right.display));

    app.detail_view = None;
    app.open_connectivity_tab(resource, targets);
    false
}

/// Handles `AppAction::OpenTrafficDebug`.
pub async fn handle_open_traffic_debug(
    app: &mut AppState,
    client: &K8sClient,
    snapshot: &ClusterSnapshot,
) -> bool {
    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(app, snapshot));
    let Some(resource) = resource else {
        app.set_error("No resource selected for traffic debugging.".to_string());
        return true;
    };
    if redirect_blocked_detail_action_to_access_review(
        app,
        client,
        Some(snapshot),
        &resource,
        DetailAction::ViewTrafficDebug,
    )
    .await
    .is_some()
    {
        return true;
    }

    app.detail_view = None;
    match traffic_debug::analyze_resource(&resource, snapshot, &app.tunnel_registry) {
        Ok(analysis) => app.open_traffic_debug_tab(resource, Some(analysis), None),
        Err(err) => app.open_traffic_debug_tab(resource, None, Some(err)),
    }
    false
}

/// Handles `AppAction::ToggleBookmark`.
///
/// Returns `true` if the caller should skip the rest of the action dispatch.
pub fn handle_toggle_bookmark(app: &mut AppState, snapshot: &ClusterSnapshot) -> bool {
    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(app, snapshot));
    let Some(resource) = resource else {
        app.set_error("No resource selected to bookmark.".to_string());
        return true;
    };

    match app.toggle_bookmark(resource.clone()) {
        Ok(BookmarkToggleResult::Added) => {
            app.clear_error();
            app.set_status(format!(
                "Bookmarked {} '{}'{}.",
                resource.kind(),
                resource.name(),
                resource
                    .namespace()
                    .map(|namespace| format!(" in namespace '{namespace}'"))
                    .unwrap_or_default()
            ));
        }
        Ok(BookmarkToggleResult::Removed) => {
            app.clear_error();
            app.set_status(format!(
                "Removed bookmark for {} '{}'{}.",
                resource.kind(),
                resource.name(),
                resource
                    .namespace()
                    .map(|namespace| format!(" in namespace '{namespace}'"))
                    .unwrap_or_default()
            ));
            if app.view() == AppView::Bookmarks {
                app.selected_idx = app
                    .selected_idx()
                    .min(app.bookmark_count().saturating_sub(1));
            }
        }
        Err(err) => app.set_error(err),
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{
        handle_apply_access_review_subject, handle_open_network_policies,
        handle_open_resource_events, handle_open_resource_yaml, handle_open_traffic_debug,
    };
    use kubectui::{
        app::{AppState, AppView, DetailViewState, ResourceRef},
        authorization::DetailActionAuthorization,
        k8s::{
            client::K8sClient,
            dtos::{ClusterRoleBindingInfo, K8sEventInfo, RoleBindingSubject},
            relationships::{RelationKind, RelationNode},
        },
        policy::DetailAction,
        rbac_subjects::{AccessReviewSubject, resolve_subject_access_review},
        state::ClusterSnapshot,
        workbench::{
            AccessReviewTabState, NetworkPolicyTabState, TrafficDebugTabState, WorkbenchTabState,
        },
    };

    #[tokio::test]
    async fn open_resource_events_rejects_unsupported_event_rows() {
        let client = K8sClient::dummy();
        let (detail_tx, mut detail_rx) = tokio::sync::mpsc::channel(1);
        let mut app = AppState {
            view: AppView::Events,
            ..AppState::default()
        };
        let snapshot = ClusterSnapshot {
            events: vec![K8sEventInfo {
                name: "event-1".to_string(),
                namespace: "default".to_string(),
                ..K8sEventInfo::default()
            }],
            ..ClusterSnapshot::default()
        };
        let mut request_seq = 0;

        let handled = handle_open_resource_events(
            &mut app,
            &client,
            &snapshot,
            &detail_tx,
            &mut request_seq,
            7,
        )
        .await;

        assert!(handled);
        assert_eq!(
            app.error_message(),
            Some("Events are not available for Event 'event-1'.")
        );
        assert!(app.workbench.tabs.is_empty());
        assert_eq!(request_seq, 0);
        assert!(detail_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn open_resource_yaml_uses_cached_yaml_error_without_refetch() {
        let client = K8sClient::dummy();
        let (detail_tx, mut detail_rx) = tokio::sync::mpsc::channel(1);
        let resource = ResourceRef::Pod("api".into(), "prod".into());
        let mut detail = DetailViewState {
            resource: Some(resource.clone()),
            yaml: None,
            yaml_error: Some("live YAML unavailable".into()),
            ..DetailViewState::default()
        };
        detail
            .metadata
            .action_authorizations
            .insert(DetailAction::ViewYaml, DetailActionAuthorization::Allowed);
        let mut app = AppState {
            detail_view: Some(detail),
            ..AppState::default()
        };
        let mut request_seq = 41;

        let handled = handle_open_resource_yaml(
            &mut app,
            &client,
            &ClusterSnapshot::default(),
            &detail_tx,
            &mut request_seq,
            7,
        )
        .await;

        assert!(!handled);
        assert_eq!(request_seq, 41);
        assert!(detail_rx.try_recv().is_err());
        let Some(tab) = app.workbench.active_tab() else {
            panic!("missing yaml tab");
        };
        let WorkbenchTabState::ResourceYaml(tab) = &tab.state else {
            panic!("expected yaml tab");
        };
        assert!(!tab.loading);
        assert_eq!(tab.pending_request_id, None);
        assert!(tab.yaml.is_none());
        assert_eq!(tab.error.as_deref(), Some("live YAML unavailable"));
    }

    #[tokio::test]
    async fn open_resource_events_uses_cached_event_error_without_refetch() {
        let client = K8sClient::dummy();
        let (detail_tx, mut detail_rx) = tokio::sync::mpsc::channel(1);
        let resource = ResourceRef::Pod("api".into(), "prod".into());
        let mut detail = DetailViewState {
            resource: Some(resource.clone()),
            events_error: Some("events unavailable".into()),
            ..DetailViewState::default()
        };
        detail
            .metadata
            .action_authorizations
            .insert(DetailAction::ViewEvents, DetailActionAuthorization::Allowed);
        let mut app = AppState {
            detail_view: Some(detail),
            ..AppState::default()
        };
        let mut request_seq = 42;

        let handled = handle_open_resource_events(
            &mut app,
            &client,
            &ClusterSnapshot::default(),
            &detail_tx,
            &mut request_seq,
            7,
        )
        .await;

        assert!(!handled);
        assert_eq!(request_seq, 42);
        assert!(detail_rx.try_recv().is_err());
        let Some(tab) = app.workbench.active_tab() else {
            panic!("missing events tab");
        };
        let WorkbenchTabState::ResourceEvents(tab) = &tab.state else {
            panic!("expected events tab");
        };
        assert!(!tab.loading);
        assert_eq!(tab.pending_request_id, None);
        assert!(tab.events.is_empty());
        assert_eq!(tab.error.as_deref(), Some("events unavailable"));
    }

    #[tokio::test]
    async fn open_resource_events_uses_loaded_empty_cache_without_refetch() {
        let client = K8sClient::dummy();
        let (detail_tx, mut detail_rx) = tokio::sync::mpsc::channel(1);
        let resource = ResourceRef::Pod("api".into(), "prod".into());
        let mut detail = DetailViewState {
            resource: Some(resource.clone()),
            events: Vec::new(),
            events_error: None,
            loading: false,
            error: None,
            ..DetailViewState::default()
        };
        detail
            .metadata
            .action_authorizations
            .insert(DetailAction::ViewEvents, DetailActionAuthorization::Allowed);
        let mut app = AppState {
            detail_view: Some(detail),
            ..AppState::default()
        };
        let mut request_seq = 42;

        let handled = handle_open_resource_events(
            &mut app,
            &client,
            &ClusterSnapshot::default(),
            &detail_tx,
            &mut request_seq,
            7,
        )
        .await;

        assert!(!handled);
        assert_eq!(request_seq, 42);
        assert!(detail_rx.try_recv().is_err());
        let Some(tab) = app.workbench.active_tab() else {
            panic!("missing events tab");
        };
        let WorkbenchTabState::ResourceEvents(tab) = &tab.state else {
            panic!("expected events tab");
        };
        assert!(!tab.loading);
        assert_eq!(tab.pending_request_id, None);
        assert!(tab.events.is_empty());
        assert_eq!(tab.error, None);
    }

    #[tokio::test]
    async fn open_network_policy_error_replaces_stale_tab_payload() {
        let client = K8sClient::dummy();
        let resource = ResourceRef::Pod("api".into(), "prod".into());
        let mut app = AppState {
            detail_view: Some(DetailViewState {
                resource: Some(resource.clone()),
                ..DetailViewState::default()
            }),
            ..AppState::default()
        };
        let mut stale_tab = NetworkPolicyTabState::new(resource.clone());
        stale_tab.summary_lines = vec!["stale allow".into()];
        stale_tab.tree = vec![RelationNode {
            resource: None,
            label: "Stale Policy".into(),
            status: None,
            namespace: None,
            relation: RelationKind::SectionHeader,
            not_found: false,
            children: Vec::new(),
        }];
        app.workbench
            .open_tab(WorkbenchTabState::NetworkPolicy(stale_tab));

        let handled =
            handle_open_network_policies(&mut app, &client, &ClusterSnapshot::default()).await;

        assert!(!handled);
        assert!(app.detail_view.is_none());
        let Some(tab) = app.workbench.active_tab() else {
            panic!("missing network policy tab");
        };
        let WorkbenchTabState::NetworkPolicy(tab) = &tab.state else {
            panic!("expected network policy tab");
        };
        assert!(tab.summary_lines.is_empty());
        assert!(tab.tree.is_empty());
        assert_eq!(
            tab.error.as_deref(),
            Some("Pod 'prod/api' is no longer in the snapshot.")
        );
    }

    #[tokio::test]
    async fn open_traffic_debug_error_replaces_stale_tab_payload() {
        let client = K8sClient::dummy();
        let resource = ResourceRef::Pod("api".into(), "prod".into());
        let mut app = AppState {
            detail_view: Some(DetailViewState {
                resource: Some(resource.clone()),
                ..DetailViewState::default()
            }),
            ..AppState::default()
        };
        let mut stale_tab = TrafficDebugTabState::new(resource.clone());
        stale_tab.summary_lines = vec!["stale route".into()];
        stale_tab.tree = vec![RelationNode {
            resource: None,
            label: "Stale Traffic".into(),
            status: None,
            namespace: None,
            relation: RelationKind::SectionHeader,
            not_found: false,
            children: Vec::new(),
        }];
        app.workbench
            .open_tab(WorkbenchTabState::TrafficDebug(stale_tab));

        let handled =
            handle_open_traffic_debug(&mut app, &client, &ClusterSnapshot::default()).await;

        assert!(!handled);
        assert!(app.detail_view.is_none());
        let Some(tab) = app.workbench.active_tab() else {
            panic!("missing traffic debug tab");
        };
        let WorkbenchTabState::TrafficDebug(tab) = &tab.state else {
            panic!("expected traffic debug tab");
        };
        assert!(tab.summary_lines.is_empty());
        assert!(tab.tree.is_empty());
        assert_eq!(
            tab.error.as_deref(),
            Some("Pod 'prod/api' is no longer in the snapshot.")
        );
    }

    #[test]
    fn apply_access_review_subject_updates_subject_review() {
        let mut app = AppState::default();
        let mut snapshot = ClusterSnapshot::default();
        snapshot.cluster_role_bindings.push(ClusterRoleBindingInfo {
            name: "alice-admin".into(),
            role_ref_kind: "ClusterRole".into(),
            role_ref_name: "admin".into(),
            subjects: vec![RoleBindingSubject {
                kind: "User".into(),
                name: "alice@example.com".into(),
                namespace: None,
                api_group: Some("rbac.authorization.k8s.io".into()),
            }],
            ..ClusterRoleBindingInfo::default()
        });

        let mut tab = AccessReviewTabState::new(
            ResourceRef::Pod("api-0".into(), "payments".into()),
            Some("prod".into()),
            "payments".into(),
            Vec::new(),
            None,
            None,
        );
        tab.start_subject_input();
        tab.subject_input.value = "User/alice@example.com".into();
        app.workbench.open_tab(WorkbenchTabState::AccessReview(tab));

        assert!(handle_apply_access_review_subject(&mut app, &snapshot));

        let Some(tab) = app.workbench.active_tab() else {
            panic!("missing access review tab");
        };
        let WorkbenchTabState::AccessReview(tab) = &tab.state else {
            panic!("expected access review tab");
        };
        let review = tab
            .subject_review
            .as_ref()
            .expect("expected subject review");
        assert_eq!(review.subject.spec(), "User/alice@example.com");
        assert_eq!(review.bindings.len(), 1);
        assert_eq!(tab.scroll, tab.subject_review_offset().saturating_sub(1));
        assert!(tab.subject_input_error.is_none());
    }

    #[test]
    fn apply_access_review_subject_clears_error_before_scrolling_to_review() {
        let mut app = AppState::default();
        let mut snapshot = ClusterSnapshot::default();
        snapshot.cluster_role_bindings.push(ClusterRoleBindingInfo {
            name: "alice-admin".into(),
            role_ref_kind: "ClusterRole".into(),
            role_ref_name: "admin".into(),
            subjects: vec![RoleBindingSubject {
                kind: "User".into(),
                name: "alice@example.com".into(),
                namespace: None,
                api_group: Some("rbac.authorization.k8s.io".into()),
            }],
            ..ClusterRoleBindingInfo::default()
        });

        let mut tab = AccessReviewTabState::new(
            ResourceRef::Pod("api-0".into(), "payments".into()),
            Some("prod".into()),
            "payments".into(),
            Vec::new(),
            None,
            None,
        );
        tab.start_subject_input();
        tab.subject_input.value = "User/alice@example.com".into();
        tab.subject_input_error = Some("bad subject".into());
        tab.subject_input.error = true;
        app.workbench.open_tab(WorkbenchTabState::AccessReview(tab));

        assert!(handle_apply_access_review_subject(&mut app, &snapshot));

        let Some(tab) = app.workbench.active_tab() else {
            panic!("missing access review tab");
        };
        let WorkbenchTabState::AccessReview(tab) = &tab.state else {
            panic!("expected access review tab");
        };
        assert!(tab.subject_input_error.is_none());
        assert_eq!(tab.scroll, tab.subject_review_offset().saturating_sub(1));
    }

    #[test]
    fn apply_access_review_subject_with_empty_input_clears_review_and_resets_scroll() {
        let mut app = AppState::default();
        let snapshot = ClusterSnapshot::default();
        let mut tab = AccessReviewTabState::new(
            ResourceRef::Pod("api-0".into(), "payments".into()),
            Some("prod".into()),
            "payments".into(),
            Vec::new(),
            Some(resolve_subject_access_review(
                &snapshot,
                AccessReviewSubject::User {
                    name: "alice@example.com".into(),
                },
            )),
            None,
        );
        tab.scroll = 99;
        tab.start_subject_input();
        tab.subject_input.clear();
        app.workbench.open_tab(WorkbenchTabState::AccessReview(tab));

        assert!(handle_apply_access_review_subject(&mut app, &snapshot));

        let Some(tab) = app.workbench.active_tab() else {
            panic!("missing access review tab");
        };
        let WorkbenchTabState::AccessReview(tab) = &tab.state else {
            panic!("expected access review tab");
        };
        assert!(tab.subject_review.is_none());
        assert!(tab.subject_input_error.is_none());
        assert_eq!(tab.scroll, tab.subject_input_offset());
    }

    #[test]
    fn apply_access_review_subject_reports_invalid_input_and_clears_stale_review() {
        let mut app = AppState::default();
        let snapshot = ClusterSnapshot::default();
        let mut tab = AccessReviewTabState::new(
            ResourceRef::Pod("api-0".into(), "payments".into()),
            Some("prod".into()),
            "payments".into(),
            Vec::new(),
            Some(resolve_subject_access_review(
                &snapshot,
                AccessReviewSubject::User {
                    name: "alice@example.com".into(),
                },
            )),
            None,
        );
        tab.start_subject_input();
        tab.subject_input.value = "Robot/api".into();
        app.workbench.open_tab(WorkbenchTabState::AccessReview(tab));

        assert!(handle_apply_access_review_subject(&mut app, &snapshot));

        let Some(tab) = app.workbench.active_tab() else {
            panic!("missing access review tab");
        };
        let WorkbenchTabState::AccessReview(tab) = &tab.state else {
            panic!("expected access review tab");
        };
        assert!(tab.subject_review.is_none());
        assert!(tab.subject_input_error.is_some());
        assert_eq!(tab.scroll, tab.subject_input_offset());
    }
}
