//! Node operation action handlers (cordon, uncordon, drain).

use std::time::Instant;

use kubectui::{
    action_history::ActionKind,
    app::{AppState, ResourceRef},
    k8s::client::K8sClient,
    policy::DetailAction,
};

use crate::async_types::{NodeOpKind, NodeOpsAsyncResult};
use crate::mutation_helpers::begin_detail_mutation;
use crate::selection_helpers::{detail_action_allowed, detail_action_denied_message};

/// Handles the confirm-drain-node action (opens the confirmation dialog).
pub fn handle_confirm_drain_node(app: &mut AppState) {
    if let Some(detail) = &mut app.detail_view
        && detail.supports_action(DetailAction::Drain)
    {
        detail.confirm_drain = true;
    }
}

/// Handles cordoning a node.
///
/// Returns `true` if the caller should skip the rest of the action dispatch.
pub async fn handle_cordon_node(
    app: &mut AppState,
    client: &K8sClient,
    node_ops_tx: &tokio::sync::mpsc::Sender<NodeOpsAsyncResult>,
    node_op_in_flight: &mut bool,
    context_generation: u64,
    status_message_clear_at: &mut Option<Instant>,
) -> bool {
    let node_name = app.detail_view.as_ref().and_then(|d| {
        if let Some(ResourceRef::Node(name)) = &d.resource {
            Some(name.clone())
        } else {
            None
        }
    });
    if let Some(name) = node_name {
        let resource = ResourceRef::Node(name.clone());
        if !detail_action_allowed(app, client, &resource, DetailAction::Cordon).await {
            app.set_error(detail_action_denied_message(
                DetailAction::Cordon,
                &resource,
            ));
            return true;
        }
        *node_op_in_flight = true;
        let resource_label = format!("Node '{name}'");
        let origin_view = app.view();
        let action_history_id = app.record_action_pending(
            ActionKind::Cordon,
            origin_view,
            app.detail_view.as_ref().and_then(|d| d.resource.clone()),
            resource_label.clone(),
            format!("Cordoning {resource_label}..."),
        );
        begin_detail_mutation(
            app,
            status_message_clear_at,
            format!("Cordoning {resource_label}..."),
        );
        let tx = node_ops_tx.clone();
        let c = client.clone();
        tokio::spawn(async move {
            let result = c.cordon_node(&name).await.map_err(|e| format!("{e:#}"));
            let _ = tx
                .send(NodeOpsAsyncResult {
                    action_history_id,
                    context_generation,
                    origin_view,
                    node_name: name,
                    op_kind: NodeOpKind::Cordon,
                    result,
                })
                .await;
        });
    }
    false
}

/// Handles uncordoning a node.
///
/// Returns `true` if the caller should skip the rest of the action dispatch.
pub async fn handle_uncordon_node(
    app: &mut AppState,
    client: &K8sClient,
    node_ops_tx: &tokio::sync::mpsc::Sender<NodeOpsAsyncResult>,
    node_op_in_flight: &mut bool,
    context_generation: u64,
    status_message_clear_at: &mut Option<Instant>,
) -> bool {
    let node_name = app.detail_view.as_ref().and_then(|d| {
        if let Some(ResourceRef::Node(name)) = &d.resource {
            Some(name.clone())
        } else {
            None
        }
    });
    if let Some(name) = node_name {
        let resource = ResourceRef::Node(name.clone());
        if !detail_action_allowed(app, client, &resource, DetailAction::Uncordon).await {
            app.set_error(detail_action_denied_message(
                DetailAction::Uncordon,
                &resource,
            ));
            return true;
        }
        *node_op_in_flight = true;
        let resource_label = format!("Node '{name}'");
        let origin_view = app.view();
        let action_history_id = app.record_action_pending(
            ActionKind::Uncordon,
            origin_view,
            app.detail_view.as_ref().and_then(|d| d.resource.clone()),
            resource_label.clone(),
            format!("Uncordoning {resource_label}..."),
        );
        begin_detail_mutation(
            app,
            status_message_clear_at,
            format!("Uncordoning {resource_label}..."),
        );
        let tx = node_ops_tx.clone();
        let c = client.clone();
        tokio::spawn(async move {
            let result = c.uncordon_node(&name).await.map_err(|e| format!("{e:#}"));
            let _ = tx
                .send(NodeOpsAsyncResult {
                    action_history_id,
                    context_generation,
                    origin_view,
                    node_name: name,
                    op_kind: NodeOpKind::Uncordon,
                    result,
                })
                .await;
        });
    }
    false
}

/// Handles draining a node (optionally forced).
///
/// Returns `true` if the caller should skip the rest of the action dispatch.
pub async fn handle_drain_node(
    app: &mut AppState,
    client: &K8sClient,
    node_ops_tx: &tokio::sync::mpsc::Sender<NodeOpsAsyncResult>,
    node_op_in_flight: &mut bool,
    context_generation: u64,
    status_message_clear_at: &mut Option<Instant>,
    force: bool,
) -> bool {
    // Always dismiss the confirmation dialog, even if node_name is None.
    if let Some(detail) = &mut app.detail_view {
        detail.confirm_drain = false;
    }
    let node_name = app.detail_view.as_ref().and_then(|d| {
        if let Some(ResourceRef::Node(name)) = &d.resource {
            Some(name.clone())
        } else {
            None
        }
    });
    if let Some(name) = node_name {
        let resource = ResourceRef::Node(name.clone());
        if !detail_action_allowed(app, client, &resource, DetailAction::Drain).await {
            app.set_error(detail_action_denied_message(DetailAction::Drain, &resource));
            return true;
        }
        *node_op_in_flight = true;
        let force_label = if force { " (force)" } else { "" };
        let resource_label = format!("Node '{name}'");
        let origin_view = app.view();
        let action_history_id = app.record_action_pending(
            ActionKind::Drain,
            origin_view,
            app.detail_view.as_ref().and_then(|d| d.resource.clone()),
            resource_label.clone(),
            format!("Draining{force_label} {resource_label}..."),
        );
        begin_detail_mutation(
            app,
            status_message_clear_at,
            format!("Draining{force_label} {resource_label}..."),
        );
        let tx = node_ops_tx.clone();
        let c = client.clone();
        tokio::spawn(async move {
            let result = c
                .drain_node(&name, 300, 30, force)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx
                .send(NodeOpsAsyncResult {
                    action_history_id,
                    context_generation,
                    origin_view,
                    node_name: name,
                    op_kind: NodeOpKind::Drain,
                    result,
                })
                .await;
        });
    }
    false
}

/// Handles the in-flight guard for drain/force-drain when a node op is already running.
pub fn handle_drain_in_flight_guard(app: &mut AppState) {
    if let Some(detail) = &mut app.detail_view {
        detail.confirm_drain = false;
    }
}
