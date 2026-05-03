//! Node debug shell dialog handlers.

use kubectui::{
    action_history::ActionKind,
    app::{AppState, ResourceRef},
    authorization::{DetailActionAuthorization, node_debug_shell_access_checks},
    k8s::{
        client::K8sClient,
        node_debug::{NodeDebugLaunchResult, NodeDebugProfile},
    },
    policy::DetailAction,
    ui::components::NodeDebugDialogState,
    workbench::AttemptedActionReview,
};

use crate::action::detail_tabs::open_access_review_for_resource_with_attempted_review;
use crate::async_types::NodeDebugLaunchAsyncResult;

pub fn handle_node_debug_dialog_open(app: &mut AppState) -> bool {
    let Some(ResourceRef::Node(node_name)) = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
    else {
        app.set_error("Open Node detail before launching a node debug shell.".to_string());
        return true;
    };
    let available_namespaces = app.namespace_picker().namespaces().to_vec();
    let default_namespace = if app.get_namespace() == "all" {
        "default".to_string()
    } else {
        app.get_namespace().to_string()
    };
    if let Some(detail) = app.detail_view.as_mut() {
        detail.node_debug_dialog = Some(NodeDebugDialogState::new(
            node_name,
            default_namespace,
            available_namespaces,
        ));
    }
    false
}

pub async fn handle_node_debug_dialog_submit(
    app: &mut AppState,
    client: &K8sClient,
    snapshot: &kubectui::state::ClusterSnapshot,
    launch_tx: &tokio::sync::mpsc::Sender<NodeDebugLaunchAsyncResult>,
    next_exec_session_id: &mut u64,
    context_generation: u64,
) -> bool {
    let Some(resource) = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
    else {
        app.set_error("No Node detail is open for node debug shell launch.".to_string());
        return true;
    };
    let ResourceRef::Node(node_name) = &resource else {
        app.set_error("Node debug shell is unavailable for the selected resource.".to_string());
        return true;
    };

    if !app
        .detail_view
        .as_ref()
        .is_some_and(|detail| detail.supports_action(DetailAction::NodeDebugShell))
    {
        app.set_error("Node debug shell is unavailable for the selected resource.".to_string());
        return true;
    }

    let request = match app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.node_debug_dialog.as_ref())
    {
        Some(dialog) => match dialog.build_launch_request() {
            Ok(request) => request,
            Err(error) => {
                if let Some(dialog) = app
                    .detail_view
                    .as_mut()
                    .and_then(|detail| detail.node_debug_dialog.as_mut())
                {
                    dialog.error_message = Some(error);
                }
                return true;
            }
        },
        None => {
            app.set_error("Node debug dialog is not open.".to_string());
            return true;
        }
    };

    let attempted_review =
        build_node_debug_attempted_review(client, &request.namespace, request.profile).await;
    match attempted_review.authorization {
        Some(DetailActionAuthorization::Allowed) => {}
        Some(DetailActionAuthorization::Denied) | Some(DetailActionAuthorization::Unknown) => {
            match open_access_review_for_resource_with_attempted_review(
                app,
                client,
                Some(snapshot),
                resource.clone(),
                attempted_review,
            )
            .await
            {
                Ok(()) => {}
                Err(err) => app.set_error(err),
            }
            return true;
        }
        None => {
            app.set_error(
                "Node debug access review did not return an authorization result.".to_string(),
            );
            return true;
        }
    }

    let resource_label = format!("Node '{node_name}'");
    let action_history_id = app.record_action_pending(
        ActionKind::NodeDebug,
        app.view(),
        Some(resource.clone()),
        resource_label.clone(),
        format!(
            "Launching {} node debug shell for {resource_label} in namespace '{}'...",
            request.profile.label(),
            request.namespace
        ),
    );
    let session_id = *next_exec_session_id;
    *next_exec_session_id = next_exec_session_id.wrapping_add(1).max(1);
    if let Some(dialog) = app
        .detail_view
        .as_mut()
        .and_then(|detail| detail.node_debug_dialog.as_mut())
    {
        dialog.begin_launch(action_history_id);
    }

    let tx = launch_tx.clone();
    let client_clone = client.clone();
    tokio::spawn(async move {
        let result = client_clone
            .launch_node_debug_pod(&request)
            .await
            .map_err(|err| format!("{err:#}"));
        let _ = tx
            .send(NodeDebugLaunchAsyncResult {
                action_history_id,
                cleanup_client: client_clone.clone(),
                context_generation,
                resource,
                session_id,
                result,
            })
            .await;
    });
    false
}

async fn build_node_debug_attempted_review(
    client: &K8sClient,
    namespace: &str,
    profile: NodeDebugProfile,
) -> AttemptedActionReview {
    let checks = node_debug_shell_access_checks(namespace);
    let authorization =
        DetailActionAuthorization::from_allowed(client.evaluate_access_checks(&checks).await);
    AttemptedActionReview {
        action: DetailAction::NodeDebugShell,
        authorization: Some(authorization),
        strict: kubectui::authorization::detail_action_requires_strict_authorization(
            DetailAction::NodeDebugShell,
        ),
        checks,
        note: Some(format!(
            "{} profile requires creating a temporary debug Pod in namespace '{}' and opening pods/exec.",
            profile.label(),
            namespace
        )),
    }
}

pub fn node_debug_shell_banner(result: &NodeDebugLaunchResult) -> Vec<String> {
    let mut lines = vec![
        format!(
            "# KubecTUI node debug shell on {} via pod {} in namespace {}",
            result.node_name, result.pod_name, result.namespace
        ),
        "# Host PID, IPC, and network namespaces are shared.".to_string(),
        "# The node root filesystem is mounted at /host.".to_string(),
    ];
    if result.profile.is_privileged() {
        lines.push("# Sysadmin profile is privileged; use it carefully.".to_string());
        lines.push("# Common first step: chroot /host".to_string());
    } else {
        lines.push(
            "# General profile is not privileged; some host-level operations like chroot may fail."
                .to_string(),
        );
    }
    lines.push("# Suggested checks: nsenter -t 1 -m -u -i -n -p sh".to_string());
    lines.push("# Suggested checks: journalctl -xe --no-pager".to_string());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use kubectui::authorization::ResourceAccessCheck;

    #[test]
    fn banner_mentions_host_mount_and_profile() {
        let lines = node_debug_shell_banner(&NodeDebugLaunchResult {
            node_name: "node-0".to_string(),
            namespace: "ops".to_string(),
            pod_name: "kubectui-node-debug-abc".to_string(),
            image: "busybox:1.37".to_string(),
            profile: NodeDebugProfile::General,
            container_name: "debugger".to_string(),
        });
        assert!(lines.iter().any(|line| line.contains("/host")));
        assert!(lines.iter().any(|line| line.contains("General profile")));
    }

    #[test]
    fn node_debug_attempted_review_uses_canonical_checks() {
        let checks = node_debug_shell_access_checks("ops");
        assert_eq!(
            checks,
            vec![
                ResourceAccessCheck::resource("create", None, "pods", Some("ops"), None),
                ResourceAccessCheck::resource("get", None, "pods", Some("ops"), None),
                ResourceAccessCheck::resource("delete", None, "pods", Some("ops"), None),
                ResourceAccessCheck::subresource("create", None, "pods", "exec", Some("ops"), None),
            ]
        );
    }
}
