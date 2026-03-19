use std::io;

use anyhow::Result;
use ratatui::{Terminal, backend::CrosstermBackend};

use kubectui::{
    app::AppState,
    k8s::client::K8sClient,
    state::watch::{WatchManager, WatchSessionKey, WatchUpdate},
};

use crate::namespace_scope;

pub(crate) fn next_request_id(sequence: &mut u64) -> u64 {
    *sequence = sequence.wrapping_add(1).max(1);
    *sequence
}

pub(crate) async fn start_watch_manager(
    client: &K8sClient,
    context_generation: u64,
    app: &AppState,
    watch_tx: &tokio::sync::mpsc::Sender<WatchUpdate>,
) -> WatchManager {
    let version = client.fetch_cluster_version().await.ok();
    let watcher_config = kubectui::state::watch::recommended_watch_config(version.as_ref());
    let mut watch_manager = WatchManager::new(WatchSessionKey {
        context_generation,
        cluster_context: app.current_context_name.clone(),
        namespace: namespace_scope(app.get_namespace()).map(str::to_string),
    });
    watch_manager.start_watches(client.get_client(), watch_tx.clone(), watcher_config);
    watch_manager
}

pub(crate) async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    crate::run_app_inner(terminal).await
}
