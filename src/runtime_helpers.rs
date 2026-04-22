use std::io;

use anyhow::Result;
use ratatui::{Terminal, backend::CrosstermBackend};

use kubectui::{
    app::{AppState, AppView},
    k8s::client::K8sClient,
    state::{
        RefreshScope,
        watch::{WatchManager, WatchSessionKey, WatchUpdate},
    },
};

use crate::namespace_scope;

pub(crate) fn next_request_id(sequence: &mut u64) -> u64 {
    *sequence = sequence.wrapping_add(1).max(1);
    *sequence
}

pub(crate) const fn watch_scope_for_view(view: AppView) -> RefreshScope {
    match view {
        AppView::Dashboard => RefreshScope::DASHBOARD_WATCHED,
        AppView::Projects | AppView::Governance => RefreshScope::CORE_OVERVIEW,
        AppView::Issues | AppView::HealthReport => RefreshScope::DASHBOARD_WATCHED,
        AppView::Nodes => RefreshScope::NODES,
        AppView::Namespaces => RefreshScope::NAMESPACES,
        AppView::Pods => RefreshScope::PODS,
        AppView::Deployments => RefreshScope::DEPLOYMENTS,
        AppView::StatefulSets => RefreshScope::STATEFULSETS,
        AppView::DaemonSets => RefreshScope::DAEMONSETS,
        AppView::ReplicaSets => RefreshScope::REPLICASETS,
        AppView::ReplicationControllers => RefreshScope::REPLICATION_CONTROLLERS,
        AppView::Jobs => RefreshScope::JOBS,
        AppView::CronJobs => RefreshScope::CRONJOBS,
        AppView::Services => RefreshScope::SERVICES,
        _ => RefreshScope::NONE,
    }
}

pub(crate) async fn start_watch_manager(
    client: &K8sClient,
    context_generation: u64,
    app: &AppState,
    watch_tx: &tokio::sync::mpsc::Sender<WatchUpdate>,
    initial_scope: RefreshScope,
) -> WatchManager {
    let version = client.cached_cluster_version().await;
    let watcher_config = kubectui::state::watch::recommended_watch_config(version.as_ref());
    let mut watch_manager = WatchManager::new(WatchSessionKey {
        context_generation,
        cluster_context: app.current_context_name.clone(),
        namespace: namespace_scope(app.get_namespace()).map(str::to_string),
    });
    watch_manager.start_watches(client, watch_tx.clone(), watcher_config, initial_scope);
    watch_manager
}

pub(crate) async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    crate::run_app_inner(terminal).await
}
