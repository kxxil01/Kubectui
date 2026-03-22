//! Detail tab action handlers (YAML, decoded secret, events, relationships, bookmarks).

use kubectui::{
    app::{AppState, AppView, ResourceRef},
    bookmarks::BookmarkToggleResult,
    k8s::client::K8sClient,
    policy::DetailAction,
    secret::decode_secret_yaml,
    state::ClusterSnapshot,
    workbench::{WorkbenchTabKey, WorkbenchTabState},
};

use crate::async_types::{DetailAsyncResult, RelationsAsyncResult, ResourceDiffAsyncResult};
use crate::next_request_id;
use crate::selection_helpers::{
    detail_action_allowed, detail_action_denied_message, selected_resource,
};

/// Spawns an async detail-view fetch for a resource.
fn spawn_detail_fetch(
    detail_tx: &tokio::sync::mpsc::Sender<DetailAsyncResult>,
    client: &K8sClient,
    snapshot: &ClusterSnapshot,
    resource: ResourceRef,
    request_id: u64,
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
    if !detail_action_allowed(app, client, &resource, DetailAction::ViewYaml).await {
        app.set_error(detail_action_denied_message(
            DetailAction::ViewYaml,
            &resource,
        ));
        return true;
    }
    let cached_yaml = app
        .detail_view
        .as_ref()
        .and_then(|detail| {
            (detail.resource.as_ref() == Some(&resource)).then(|| detail.yaml.clone())
        })
        .flatten();
    let pending_request_id = cached_yaml
        .is_none()
        .then(|| next_request_id(detail_request_seq));
    app.detail_view = None;
    app.open_resource_yaml_tab(
        resource.clone(),
        cached_yaml.clone(),
        None,
        pending_request_id,
    );
    if let Some(request_id) = pending_request_id {
        spawn_detail_fetch(detail_tx, client, snapshot, resource, request_id);
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
    if !detail_action_allowed(app, client, &resource, DetailAction::ViewConfigDrift).await {
        app.set_error(detail_action_denied_message(
            DetailAction::ViewConfigDrift,
            &resource,
        ));
        return true;
    }

    let request_id = next_request_id(diff_request_seq);

    app.detail_view = None;
    app.open_resource_diff_tab(resource.clone(), None, None, Some(request_id));
    spawn_resource_diff_fetch(diff_tx, client, resource, request_id);
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
    if !detail_action_allowed(app, client, &resource, DetailAction::ViewDecodedSecret).await {
        app.set_error(detail_action_denied_message(
            DetailAction::ViewDecodedSecret,
            &resource,
        ));
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
        spawn_detail_fetch(detail_tx, client, snapshot, resource, request_id);
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
    if !detail_action_allowed(app, client, &resource, DetailAction::ViewEvents).await {
        app.set_error(detail_action_denied_message(
            DetailAction::ViewEvents,
            &resource,
        ));
        return true;
    }
    let cached_events = app
        .detail_view
        .as_ref()
        .and_then(|detail| {
            (detail.resource.as_ref() == Some(&resource)).then(|| detail.events.clone())
        })
        .unwrap_or_default();
    let loading = cached_events.is_empty();
    let pending_request_id = loading.then(|| next_request_id(detail_request_seq));
    app.detail_view = None;
    app.open_resource_events_tab(
        resource.clone(),
        cached_events,
        loading,
        None,
        pending_request_id,
    );
    if let Some(request_id) = pending_request_id {
        spawn_detail_fetch(detail_tx, client, snapshot, resource, request_id);
    }
    false
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
    let mut relations_tab = kubectui::workbench::RelationsTabState::new(resource.clone());
    relations_tab.pending_request_id = Some(request_id);
    app.workbench
        .open_tab(WorkbenchTabState::Relations(relations_tab));
    app.focus = kubectui::app::Focus::Workbench;

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
                resource: requested_resource,
                result,
            })
            .await;
    });
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
