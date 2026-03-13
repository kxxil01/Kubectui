//! Watch-backed resource caches for core Kubernetes resources.
//!
//! Replaces steady-state polling with Kubernetes watch streams for lower
//! API cost and near-instant propagation of cluster changes. Watch state
//! feeds the same [`super::ClusterSnapshot`] model used by polling.

use std::collections::HashMap;

use futures::TryStreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::runtime::WatchStreamExt;
use kube::runtime::watcher::{self, Event};
use kube::{Api, Client, ResourceExt};
use tokio::sync::mpsc;

use crate::k8s::conversions::pod_to_info;
use crate::k8s::dtos::PodInfo;

/// Identifies which watched resource produced an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchedResource {
    Pods,
}

/// A snapshot update published by a watcher task.
#[derive(Debug)]
pub struct WatchUpdate {
    pub resource: WatchedResource,
    pub context_generation: u64,
    pub data: WatchPayload,
}

/// Typed payload for a watch update.
#[derive(Debug)]
pub enum WatchPayload {
    Pods(Vec<PodInfo>),
}

/// Session identity for stale-event rejection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchSessionKey {
    pub context_generation: u64,
    pub cluster_context: Option<String>,
    pub namespace: Option<String>,
}

/// Readiness state of a single resource store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StoreReadiness {
    #[default]
    Idle,
    Listing,
    Watching,
    Error,
}

/// In-memory store for a single watched resource type.
///
/// Keyed by Kubernetes UID for O(1) apply/delete. Publishes sorted
/// `Vec<T>` snapshots matching the polling output format.
#[derive(Debug)]
pub struct ResourceStore<T> {
    items: HashMap<String, T>,
    init_buffer: HashMap<String, T>,
    pub readiness: StoreReadiness,
    pub last_error: Option<String>,
}

impl<T: Clone> Default for ResourceStore<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone> ResourceStore<T> {
    /// Creates an empty store in [`StoreReadiness::Idle`] state.
    pub fn new() -> Self {
        Self {
            items: HashMap::new(),
            init_buffer: HashMap::new(),
            readiness: StoreReadiness::Idle,
            last_error: None,
        }
    }

    /// Begins an init cycle, clearing the init buffer.
    pub fn begin_init(&mut self) {
        self.init_buffer.clear();
        self.readiness = StoreReadiness::Listing;
    }

    /// Buffers an item during the Init cycle.
    pub fn apply_init_page(&mut self, uid: String, item: T) {
        self.init_buffer.insert(uid, item);
    }

    /// Commits the init buffer into the live store atomically.
    pub fn commit_init(&mut self) {
        std::mem::swap(&mut self.items, &mut self.init_buffer);
        self.init_buffer.clear();
        self.readiness = StoreReadiness::Watching;
        self.last_error = None;
    }

    /// Upserts an item for an Apply event.
    pub fn apply_event(&mut self, uid: String, item: T) {
        self.items.insert(uid, item);
    }

    /// Removes an item for a Delete event.
    pub fn remove(&mut self, uid: &str) {
        self.items.remove(uid);
    }

    /// Publishes a sorted snapshot of all items.
    pub fn publish(&self) -> Vec<T> {
        self.items.values().cloned().collect()
    }

    /// Resets the store to idle, discarding all data.
    pub fn clear(&mut self) {
        self.items.clear();
        self.init_buffer.clear();
        self.readiness = StoreReadiness::Idle;
        self.last_error = None;
    }
}

/// Publish helper: sorts pods by name for stable output.
fn sort_pods(pods: &mut [PodInfo]) {
    pods.sort_by(|a, b| a.name.cmp(&b.name));
}

/// Manages all watch tasks and their lifecycle.
pub struct WatchManager {
    session: WatchSessionKey,
    cancel_tx: Option<tokio::sync::watch::Sender<()>>,
}

impl WatchManager {
    /// Creates a new watch manager for the given session.
    pub fn new(session: WatchSessionKey) -> Self {
        Self {
            session,
            cancel_tx: None,
        }
    }

    /// Returns the current session key.
    pub fn session_key(&self) -> &WatchSessionKey {
        &self.session
    }

    /// Starts all watch tasks (currently only pods).
    pub fn start_watches(&mut self, client: Client, watch_tx: mpsc::Sender<WatchUpdate>) {
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(());
        self.cancel_tx = Some(cancel_tx);

        start_pod_watch(client, self.session.clone(), watch_tx, cancel_rx);
    }

    /// Stops all running watch tasks.
    pub fn stop_all(&mut self) {
        // Dropping the sender causes all receivers to see a changed() error,
        // which terminates the select! in each watcher task.
        self.cancel_tx.take();
    }
}

/// Spawns a background task that watches pods and publishes updates.
fn start_pod_watch(
    client: Client,
    session: WatchSessionKey,
    watch_tx: mpsc::Sender<WatchUpdate>,
    mut cancel_rx: tokio::sync::watch::Receiver<()>,
) {
    tokio::spawn(async move {
        let api: Api<Pod> = match &session.namespace {
            Some(ns) => Api::namespaced(client, ns),
            None => Api::all(client),
        };

        let stream = watcher::watcher(api, watcher::Config::default()).default_backoff();

        let mut store = ResourceStore::<PodInfo>::new();

        tokio::pin!(stream);

        loop {
            tokio::select! {
                biased;
                _ = cancel_rx.changed() => {
                    break;
                }
                item = stream.try_next() => {
                    match item {
                        Ok(Some(event)) => {
                            let should_publish = process_pod_event(&mut store, event);
                            if should_publish {
                                let mut snapshot = store.publish();
                                sort_pods(&mut snapshot);
                                let update = WatchUpdate {
                                    resource: WatchedResource::Pods,
                                    context_generation: session.context_generation,
                                    data: WatchPayload::Pods(snapshot),
                                };
                                if watch_tx.send(update).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Ok(None) => {
                            break;
                        }
                        Err(err) => {
                            store.readiness = StoreReadiness::Error;
                            store.last_error = Some(err.to_string());
                        }
                    }
                }
            }
        }
    });
}

/// Processes a single watcher event, returning true if the store should
/// be published to the event loop.
fn process_pod_event(store: &mut ResourceStore<PodInfo>, event: Event<Pod>) -> bool {
    match event {
        Event::Init => {
            store.begin_init();
            false
        }
        Event::InitApply(pod) => {
            let uid = pod.uid().unwrap_or_default();
            if !uid.is_empty() {
                store.apply_init_page(uid, pod_to_info(pod));
            }
            false
        }
        Event::InitDone => {
            store.commit_init();
            true
        }
        Event::Apply(pod) => {
            let uid = pod.uid().unwrap_or_default();
            if !uid.is_empty() {
                store.apply_event(uid, pod_to_info(pod));
                true
            } else {
                false
            }
        }
        Event::Delete(pod) => {
            let uid = pod.uid().unwrap_or_default();
            if !uid.is_empty() {
                store.remove(&uid);
                true
            } else {
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pod_info(name: &str) -> PodInfo {
        PodInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn resource_store_init_cycle() {
        let mut store = ResourceStore::<PodInfo>::new();
        assert_eq!(store.readiness, StoreReadiness::Idle);

        store.begin_init();
        assert_eq!(store.readiness, StoreReadiness::Listing);

        store.apply_init_page("uid-1".to_string(), make_pod_info("pod-a"));
        store.apply_init_page("uid-2".to_string(), make_pod_info("pod-b"));
        store.apply_init_page("uid-3".to_string(), make_pod_info("pod-c"));

        // Items not yet in live store
        assert!(store.items.is_empty());

        store.commit_init();
        assert_eq!(store.readiness, StoreReadiness::Watching);
        assert_eq!(store.items.len(), 3);

        let mut snapshot = store.publish();
        sort_pods(&mut snapshot);
        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot[0].name, "pod-a");
        assert_eq!(snapshot[1].name, "pod-b");
        assert_eq!(snapshot[2].name, "pod-c");
    }

    #[test]
    fn resource_store_apply_upsert() {
        let mut store = ResourceStore::<PodInfo>::new();
        store.apply_event("uid-1".to_string(), make_pod_info("pod-a"));
        assert_eq!(store.items.len(), 1);

        // Upsert same UID with updated data
        let mut updated = make_pod_info("pod-a");
        updated.status = "Succeeded".to_string();
        store.apply_event("uid-1".to_string(), updated);
        assert_eq!(store.items.len(), 1);
        assert_eq!(store.items["uid-1"].status, "Succeeded");
    }

    #[test]
    fn resource_store_delete() {
        let mut store = ResourceStore::<PodInfo>::new();
        store.apply_event("uid-1".to_string(), make_pod_info("pod-a"));
        store.apply_event("uid-2".to_string(), make_pod_info("pod-b"));
        assert_eq!(store.items.len(), 2);

        store.remove("uid-1");
        assert_eq!(store.items.len(), 1);
        assert!(!store.items.contains_key("uid-1"));
        assert!(store.items.contains_key("uid-2"));
    }

    #[test]
    fn resource_store_publish_sorted() {
        let mut store = ResourceStore::<PodInfo>::new();
        store.apply_event("uid-3".to_string(), make_pod_info("pod-c"));
        store.apply_event("uid-1".to_string(), make_pod_info("pod-a"));
        store.apply_event("uid-2".to_string(), make_pod_info("pod-b"));

        let mut snapshot = store.publish();
        sort_pods(&mut snapshot);
        assert_eq!(snapshot[0].name, "pod-a");
        assert_eq!(snapshot[1].name, "pod-b");
        assert_eq!(snapshot[2].name, "pod-c");
    }

    #[test]
    fn resource_store_clear_resets_state() {
        let mut store = ResourceStore::<PodInfo>::new();
        store.apply_event("uid-1".to_string(), make_pod_info("pod-a"));
        store.readiness = StoreReadiness::Watching;
        store.last_error = Some("test".to_string());

        store.clear();
        assert!(store.items.is_empty());
        assert_eq!(store.readiness, StoreReadiness::Idle);
        assert!(store.last_error.is_none());
    }

    #[test]
    fn init_cycle_replaces_previous_data() {
        let mut store = ResourceStore::<PodInfo>::new();
        store.apply_event("uid-old".to_string(), make_pod_info("old-pod"));
        assert_eq!(store.items.len(), 1);

        // A new init cycle should replace the old data
        store.begin_init();
        store.apply_init_page("uid-new".to_string(), make_pod_info("new-pod"));
        store.commit_init();

        assert_eq!(store.items.len(), 1);
        assert!(store.items.contains_key("uid-new"));
        assert!(!store.items.contains_key("uid-old"));
    }
}
