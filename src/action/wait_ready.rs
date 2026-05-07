//! Wait-until-ready handlers backed by kube-runtime conditions.

use std::time::{Duration, Instant};

use kubectui::{
    action_history::ActionKind,
    app::{AppState, ResourceRef},
    policy::DetailAction,
};

use crate::{
    action::detail_tabs::redirect_blocked_detail_action_to_access_review,
    async_types::WaitReadyAsyncResult, mutation_helpers::set_transient_status,
    selection_helpers::selected_resource,
};

const WAIT_READY_TIMEOUT: Duration = Duration::from_secs(300);

pub async fn handle_wait_until_ready(
    app: &mut AppState,
    client: &kubectui::k8s::client::K8sClient,
    snapshot: &kubectui::state::ClusterSnapshot,
    tx: &tokio::sync::mpsc::Sender<WaitReadyAsyncResult>,
    context_generation: u64,
    status_message_clear_at: &mut Option<Instant>,
) -> bool {
    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(app, snapshot));
    let Some(resource) = resource else {
        app.set_error("No resource selected for wait until ready.".to_string());
        return true;
    };
    if !supports_wait_until_ready(&resource) {
        app.set_error(
            "Wait until ready is only available for Deployments, Services, and Ingresses."
                .to_string(),
        );
        return true;
    }
    if redirect_blocked_detail_action_to_access_review(
        app,
        client,
        Some(snapshot),
        &resource,
        DetailAction::WaitReady,
    )
    .await
    .is_some()
    {
        return true;
    }

    let origin_view = app.view();
    let resource_label = resource.summary_label();
    let action_history_id = app.record_action_pending(
        ActionKind::WaitReady,
        origin_view,
        Some(resource.clone()),
        resource_label.clone(),
        format!("Waiting for {resource_label} to become ready..."),
    );
    set_transient_status(
        app,
        status_message_clear_at,
        format!("Waiting for {resource_label} to become ready..."),
    );

    let tx = tx.clone();
    let client = client.clone();
    tokio::spawn(async move {
        let result = client
            .wait_until_ready(&resource, WAIT_READY_TIMEOUT)
            .await
            .map_err(|err| format!("{err:#}"));
        let _ = tx
            .send(WaitReadyAsyncResult {
                action_history_id,
                context_generation,
                origin_view,
                resource_label,
                result,
            })
            .await;
    });
    false
}

pub fn supports_wait_until_ready(resource: &ResourceRef) -> bool {
    matches!(
        resource,
        ResourceRef::Deployment(_, _) | ResourceRef::Service(_, _) | ResourceRef::Ingress(_, _)
    )
}

#[cfg(test)]
mod tests {
    use super::supports_wait_until_ready;
    use kubectui::app::ResourceRef;

    #[test]
    fn wait_ready_supports_kube_runtime_condition_resources() {
        assert!(supports_wait_until_ready(&ResourceRef::Deployment(
            "api".into(),
            "prod".into()
        )));
        assert!(supports_wait_until_ready(&ResourceRef::Service(
            "api".into(),
            "prod".into()
        )));
        assert!(supports_wait_until_ready(&ResourceRef::Ingress(
            "api".into(),
            "prod".into()
        )));
        assert!(!supports_wait_until_ready(&ResourceRef::Pod(
            "api".into(),
            "prod".into()
        )));
    }
}
