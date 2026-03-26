//! Node debug shell dialog handlers.

use kubectui::{
    action_history::ActionKind,
    app::{AppState, ResourceRef},
    authorization::ResourceAccessCheck,
    k8s::{
        client::K8sClient,
        node_debug::{NodeDebugLaunchResult, NodeDebugProfile},
    },
    policy::DetailAction,
    ui::components::NodeDebugDialogState,
};

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

    let Some(dialog) = app
        .detail_view
        .as_mut()
        .and_then(|detail| detail.node_debug_dialog.as_mut())
    else {
        app.set_error("Node debug dialog is not open.".to_string());
        return true;
    };

    let request = match dialog.build_launch_request() {
        Ok(request) => request,
        Err(error) => {
            dialog.error_message = Some(error);
            return true;
        }
    };

    match client
        .evaluate_access_checks(&node_debug_access_checks(
            &request.namespace,
            request.profile,
        ))
        .await
    {
        Some(true) => {}
        Some(false) => {
            dialog.error_message = Some(detail_action_denied_message(&request.namespace));
            return true;
        }
        None => {
            dialog.error_message = Some(detail_action_unknown_message(&request.namespace));
            return true;
        }
    }

    let session_id = *next_exec_session_id;
    *next_exec_session_id = next_exec_session_id.wrapping_add(1).max(1);
    dialog.set_pending_launch(true);

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

fn node_debug_access_checks(
    namespace: &str,
    _profile: NodeDebugProfile,
) -> Vec<ResourceAccessCheck> {
    vec![
        ResourceAccessCheck::resource("create", None, "pods", Some(namespace), None),
        ResourceAccessCheck::resource("get", None, "pods", Some(namespace), None),
        ResourceAccessCheck::resource("delete", None, "pods", Some(namespace), None),
        ResourceAccessCheck::subresource("create", None, "pods", "exec", Some(namespace), None),
    ]
}

fn detail_action_denied_message(namespace: &str) -> String {
    format!(
        "Node debug shell requires Pod create/get/delete and pods/exec access in namespace '{}'.",
        namespace
    )
}

fn detail_action_unknown_message(namespace: &str) -> String {
    format!(
        "Unable to verify Pod create/get/delete and pods/exec access for namespace '{}'. Node debug shell is blocked until RBAC can be confirmed.",
        namespace
    )
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
}
