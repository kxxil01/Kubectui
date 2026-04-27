//! Watch-backed resource caches for Kubernetes resources.
//!
//! Replaces steady-state polling with Kubernetes watch streams for lower
//! API cost and near-instant propagation of cluster changes. Watch state
//! feeds the same [`super::ClusterSnapshot`] model used by polling.

use std::collections::HashMap;
use std::time::Duration;

use futures::TryStreamExt;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Namespace, Node, Pod, ReplicationController, Service};
use kube::api::{ApiResource, DynamicObject, GroupVersionKind};
use kube::core::PartialObjectMeta;
use kube::runtime::WatchStreamExt;
use kube::runtime::watcher::{self, Event};
use kube::{Api, Client, ResourceExt};
use tokio::sync::mpsc;
use tracing::warn;

use super::RefreshScope;
use crate::k8s::client::{
    FluxWatchTarget, K8sClient, flux_dynamic_object_to_info, is_forbidden_error,
    is_missing_api_error,
};
use crate::k8s::conversions::{
    cronjob_to_info, daemonset_to_info, deployment_to_info, job_to_info,
    namespace_metadata_to_info, node_to_info, pod_to_info, replicaset_to_info,
    replication_controller_to_info, service_to_info, statefulset_to_info,
};
use crate::k8s::dtos::{
    ClusterVersionInfo, CronJobInfo, DaemonSetInfo, DeploymentInfo, FluxResourceInfo, JobInfo,
    NamespaceInfo, NodeInfo, PodInfo, ReplicaSetInfo, ReplicationControllerInfo, ServiceInfo,
    StatefulSetInfo,
};

const STREAMING_LISTS_MIN_MINOR: u32 = 34;
const WATCH_PUBLISH_DEBOUNCE_MS: u64 = 75;

fn normalize_watch_scope(scope: RefreshScope) -> RefreshScope {
    scope
        .without(RefreshScope::FLUX)
        .union(RefreshScope::NAMESPACES)
}

fn should_restart_watches_for_scope(
    requested_scope: RefreshScope,
    desired_scope: RefreshScope,
) -> bool {
    !requested_scope
        .without(normalize_watch_scope(desired_scope))
        .is_empty()
}

fn active_scope_after_starting_missing(
    active_scope: RefreshScope,
    missing_scope: RefreshScope,
) -> RefreshScope {
    active_scope.union(missing_scope.without(RefreshScope::FLUX))
}

/// Returns the recommended kube-runtime watcher config for a cluster version.
///
/// Kubernetes documents streaming lists as beta and enabled by default from
/// v1.34 onward. We therefore only opt into `streaming_lists()` for clusters
/// that advertise `gitVersion` >= v1.34. Older or unknown versions stay on the
/// safer `ListWatch` path, but use the cheaper `Any` list semantic for relists.
pub fn recommended_watch_config(version: Option<&ClusterVersionInfo>) -> watcher::Config {
    if version.is_some_and(|info| supports_streaming_lists(info.git_version.as_str())) {
        watcher::Config::default().streaming_lists()
    } else {
        watcher::Config::default().any_semantic()
    }
}

fn supports_streaming_lists(git_version: &str) -> bool {
    parse_kubernetes_minor_version(git_version).is_some_and(|(major, minor)| {
        major > 1 || (major == 1 && minor >= STREAMING_LISTS_MIN_MINOR)
    })
}

fn parse_kubernetes_minor_version(git_version: &str) -> Option<(u32, u32)> {
    let version = git_version.strip_prefix('v').unwrap_or(git_version);
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor_raw = parts.next()?;
    let minor_digits: String = minor_raw
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    let minor = minor_digits.parse().ok()?;
    Some((major, minor))
}

/// Identifies which watched resource produced an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchedResource {
    Pods,
    Deployments,
    ReplicaSets,
    StatefulSets,
    DaemonSets,
    Services,
    Nodes,
    ReplicationControllers,
    Jobs,
    CronJobs,
    Namespaces,
    Flux,
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
    Deployments(Vec<DeploymentInfo>),
    ReplicaSets(Vec<ReplicaSetInfo>),
    StatefulSets(Vec<StatefulSetInfo>),
    DaemonSets(Vec<DaemonSetInfo>),
    Services(Vec<ServiceInfo>),
    Nodes(Vec<NodeInfo>),
    ReplicationControllers(Vec<ReplicationControllerInfo>),
    Jobs(Vec<JobInfo>),
    CronJobs(Vec<CronJobInfo>),
    Namespaces(Vec<NamespaceInfo>),
    Flux {
        target: FluxWatchTarget,
        items: Vec<FluxResourceInfo>,
    },
    /// A watcher encountered an error or terminated.
    Error {
        message: String,
    },
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
/// Keyed by Kubernetes UID for O(1) apply/delete. Callers are responsible
/// for sorting the published `Vec<T>` for stable output.
#[derive(Debug)]
pub struct ResourceStore<T> {
    items: HashMap<String, T>,
    init_buffer: HashMap<String, T>,
    pub readiness: StoreReadiness,
    pub last_error: Option<String>,
}

impl<T> Default for ResourceStore<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ResourceStore<T> {
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

    /// Resets the store to idle, discarding all data.
    pub fn clear(&mut self) {
        self.items.clear();
        self.init_buffer.clear();
        self.readiness = StoreReadiness::Idle;
        self.last_error = None;
    }
}

impl<T: PartialEq> ResourceStore<T> {
    /// Commits the init buffer into the live store atomically.
    ///
    /// Returns `true` only when the resulting live store differs from the
    /// previous snapshot, suppressing reconnect relists that do not change any
    /// DTO-visible data.
    pub fn commit_init(&mut self) -> bool {
        let changed = self.items != self.init_buffer;
        std::mem::swap(&mut self.items, &mut self.init_buffer);
        self.init_buffer.clear();
        self.readiness = StoreReadiness::Watching;
        self.last_error = None;
        changed
    }

    /// Upserts an item for an Apply event.
    ///
    /// Returns `true` only when the DTO-visible payload actually changed.
    pub fn apply_event(&mut self, uid: String, item: T) -> bool {
        match self.items.get(&uid) {
            Some(existing) if existing == &item => false,
            _ => {
                self.items.insert(uid, item);
                true
            }
        }
    }

    /// Removes an item for a Delete event.
    ///
    /// Returns `true` only when the item previously existed.
    pub fn remove(&mut self, uid: &str) -> bool {
        self.items.remove(uid).is_some()
    }
}

impl<T: Clone> ResourceStore<T> {
    /// Publishes a snapshot of all items (unsorted — caller must sort).
    pub fn publish(&self) -> Vec<T> {
        self.items.values().cloned().collect()
    }
}

// ── Watcher macro ──
//
// Generates `start_<name>_watch`, `process_<name>_event`, and `sort_<name>s`
// for each watched resource type. The `scope` parameter controls whether
// the API is namespace-scoped or cluster-scoped.

macro_rules! define_watcher {
    (@sort $items:ident, namespaced) => {
        $items.sort_unstable_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.namespace.cmp(&b.namespace))
        });
    };
    (@sort $items:ident, cluster) => {
        $items.sort_unstable_by(|a, b| a.name.cmp(&b.name));
    };
    (
        name: $name:ident,
        k8s_type: $K8sType:ty,
        dto_type: $DtoType:ty,
        converter: $converter:path,
        resource_variant: $variant:ident,
        scope: namespaced $(,)?
    ) => {
        define_watcher!(@impl $name, $K8sType, $K8sType, $DtoType, $converter, $variant, namespaced, watcher::watcher);
    };
    (
        name: $name:ident,
        k8s_type: $K8sType:ty,
        dto_type: $DtoType:ty,
        converter: $converter:path,
        resource_variant: $variant:ident,
        scope: cluster $(,)?
    ) => {
        define_watcher!(@impl $name, $K8sType, $K8sType, $DtoType, $converter, $variant, cluster, watcher::watcher);
    };
    (
        name: $name:ident,
        k8s_type: $K8sType:ty,
        dto_type: $DtoType:ty,
        converter: $converter:path,
        resource_variant: $variant:ident,
        scope: cluster,
        mode: metadata $(,)?
    ) => {
        define_watcher!(
            @impl
            $name,
            $K8sType,
            PartialObjectMeta<$K8sType>,
            $DtoType,
            $converter,
            $variant,
            cluster,
            watcher::metadata_watcher
        );
    };
    (@impl $name:ident, $ApiType:ty, $EventType:ty, $DtoType:ty, $converter:path, $variant:ident, $scope:ident, $watch_fn:path) => {
        paste::paste! {
            pub(crate) fn [<sort_ $name s>](items: &mut [$DtoType]) {
                define_watcher!(@sort items, $scope);
            }

            fn [<start_ $name _watch>](
                client: Client,
                session: WatchSessionKey,
                watch_tx: mpsc::Sender<WatchUpdate>,
                watcher_config: watcher::Config,
                mut cancel_rx: tokio::sync::watch::Receiver<()>,
            ) {
                tokio::spawn(async move {
                    let api: Api<$ApiType> = define_watcher!(@api $scope, client, session);
                    let stream = $watch_fn(api, watcher_config).default_backoff();
                    let mut store = ResourceStore::<$DtoType>::new();
                    let mut publish_pending = false;
                    let mut publish_tick =
                        tokio::time::interval(Duration::from_millis(WATCH_PUBLISH_DEBOUNCE_MS));
                    publish_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    publish_tick.tick().await;
                    tokio::pin!(stream);

                    loop {
                        tokio::select! {
                            biased;
                            _ = cancel_rx.changed() => break,
                            _ = publish_tick.tick(), if publish_pending => {
                                publish_pending = false;
                                let mut snapshot = store.publish();
                                [<sort_ $name s>](&mut snapshot);
                                if watch_tx.send(WatchUpdate {
                                    resource: WatchedResource::$variant,
                                    context_generation: session.context_generation,
                                    data: WatchPayload::$variant(snapshot),
                                }).await.is_err() {
                                    break;
                                }
                            }
                            item = stream.try_next() => {
                                match item {
                                    Ok(Some(event)) => {
                                        if [<process_ $name _event>](&mut store, event) {
                                            publish_pending = true;
                                        }
                                    }
                                    Ok(None) => {
                                        if publish_pending {
                                            let mut snapshot = store.publish();
                                            [<sort_ $name s>](&mut snapshot);
                                            if watch_tx.send(WatchUpdate {
                                                resource: WatchedResource::$variant,
                                                context_generation: session.context_generation,
                                                data: WatchPayload::$variant(snapshot),
                                            }).await.is_err() {
                                                break;
                                            }
                                        }
                                        warn!(
                                            concat!(stringify!($name), " watch stream ended unexpectedly")
                                        );
                                        let _ = watch_tx.send(WatchUpdate {
                                            resource: WatchedResource::$variant,
                                            context_generation: session.context_generation,
                                            data: WatchPayload::Error {
                                                message: "watch stream terminated".to_string(),
                                            },
                                        }).await;
                                        break;
                                    }
                                    Err(err) => {
                                        warn!(
                                            error = %err,
                                            concat!(stringify!($name), " watch stream error"),
                                        );
                                        let _ = watch_tx.send(WatchUpdate {
                                            resource: WatchedResource::$variant,
                                            context_generation: session.context_generation,
                                            data: WatchPayload::Error {
                                                message: err.to_string(),
                                            },
                                        }).await;
                                    }
                                }
                            }
                        }
                    }
                });
            }

            fn [<process_ $name _event>](
                store: &mut ResourceStore<$DtoType>,
                event: Event<$EventType>,
            ) -> bool {
                match event {
                    Event::Init => {
                        store.begin_init();
                        false
                    }
                    Event::InitApply(obj) => {
                        let uid = obj.uid().unwrap_or_default();
                        if uid.is_empty() {
                            warn!(
                                name = obj.metadata.name.as_deref().unwrap_or("<unknown>"),
                                concat!("skipping ", stringify!($name), " with empty UID during init"),
                            );
                            return false;
                        }
                        store.apply_init_page(uid, $converter(obj));
                        false
                    }
                    Event::InitDone => store.commit_init(),
                    Event::Apply(obj) => {
                        let uid = obj.uid().unwrap_or_default();
                        if uid.is_empty() {
                            warn!(
                                name = obj.metadata.name.as_deref().unwrap_or("<unknown>"),
                                concat!("skipping ", stringify!($name), " with empty UID on apply"),
                            );
                            return false;
                        }
                        store.apply_event(uid, $converter(obj))
                    }
                    Event::Delete(obj) => {
                        let uid = obj.uid().unwrap_or_default();
                        if uid.is_empty() {
                            warn!(
                                name = obj.metadata.name.as_deref().unwrap_or("<unknown>"),
                                concat!("skipping ", stringify!($name), " with empty UID on delete"),
                            );
                            return false;
                        }
                        store.remove(&uid)
                    }
                }
            }
        }
    };
    // API construction helpers — namespace-aware vs cluster-scoped.
    (@api namespaced, $client:ident, $session:ident) => {
        match &$session.namespace {
            Some(ns) => Api::namespaced($client, ns),
            None => Api::all($client),
        }
    };
    (@api cluster, $client:ident, $session:ident) => {{
        let _ = &$session; // suppress unused warning
        Api::all($client)
    }};
}

fn flux_watch_error_is_terminal(err: &watcher::Error) -> bool {
    match err {
        watcher::Error::InitialListFailed(source)
        | watcher::Error::WatchStartFailed(source)
        | watcher::Error::WatchFailed(source) => {
            is_forbidden_error(source) || is_missing_api_error(source)
        }
        watcher::Error::WatchError(status) => status.code == 403 || status.code == 404,
        watcher::Error::NoResourceVersion => false,
    }
}

#[derive(Debug, Clone)]
struct FluxWatchInfo(FluxResourceInfo);

impl PartialEq for FluxWatchInfo {
    fn eq(&self, other: &Self) -> bool {
        let left = &self.0;
        let right = &other.0;
        left.name == right.name
            && left.namespace == right.namespace
            && left.kind == right.kind
            && left.group == right.group
            && left.version == right.version
            && left.plural == right.plural
            && left.source_url == right.source_url
            && left.status == right.status
            && left.message == right.message
            && left.artifact == right.artifact
            && left.suspended == right.suspended
            && left.created_at == right.created_at
            && left.conditions == right.conditions
            && left.last_reconcile_time == right.last_reconcile_time
            && left.last_applied_revision == right.last_applied_revision
            && left.last_attempted_revision == right.last_attempted_revision
            && left.observed_generation == right.observed_generation
            && left.generation == right.generation
            && left.source_ref == right.source_ref
            && left.interval == right.interval
            && left.timeout == right.timeout
    }
}

fn process_flux_event(
    store: &mut ResourceStore<FluxWatchInfo>,
    target: FluxWatchTarget,
    event: Event<DynamicObject>,
) -> bool {
    match event {
        Event::Init => {
            store.begin_init();
            false
        }
        Event::InitApply(obj) => {
            let uid = obj.uid().unwrap_or_default();
            if uid.is_empty() {
                warn!(
                    name = obj.metadata.name.as_deref().unwrap_or("<unknown>"),
                    "skipping Flux resource with empty UID during init",
                );
                return false;
            }
            store.apply_init_page(uid, FluxWatchInfo(flux_dynamic_object_to_info(obj, target)));
            false
        }
        Event::InitDone => store.commit_init(),
        Event::Apply(obj) => {
            let uid = obj.uid().unwrap_or_default();
            if uid.is_empty() {
                warn!(
                    name = obj.metadata.name.as_deref().unwrap_or("<unknown>"),
                    "skipping Flux resource with empty UID on apply",
                );
                return false;
            }
            store.apply_event(uid, FluxWatchInfo(flux_dynamic_object_to_info(obj, target)))
        }
        Event::Delete(obj) => {
            let uid = obj.uid().unwrap_or_default();
            if uid.is_empty() {
                warn!(
                    name = obj.metadata.name.as_deref().unwrap_or("<unknown>"),
                    "skipping Flux resource with empty UID on delete",
                );
                return false;
            }
            store.remove(&uid)
        }
    }
}

fn sorted_flux_resources(store: &ResourceStore<FluxWatchInfo>) -> Vec<FluxResourceInfo> {
    let mut resources: Vec<FluxResourceInfo> =
        store.publish().into_iter().map(|item| item.0).collect();
    resources.sort_unstable_by(|left, right| {
        left.namespace
            .cmp(&right.namespace)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.name.cmp(&right.name))
    });
    resources
}

async fn send_flux_watch_payload(
    watch_tx: &mpsc::Sender<WatchUpdate>,
    context_generation: u64,
    target: FluxWatchTarget,
    store: &ResourceStore<FluxWatchInfo>,
) -> bool {
    watch_tx
        .send(WatchUpdate {
            resource: WatchedResource::Flux,
            context_generation,
            data: WatchPayload::Flux {
                target,
                items: sorted_flux_resources(store),
            },
        })
        .await
        .is_ok()
}

fn start_flux_watch(
    client: Client,
    session: WatchSessionKey,
    watch_tx: mpsc::Sender<WatchUpdate>,
    watcher_config: watcher::Config,
    mut cancel_rx: tokio::sync::watch::Receiver<()>,
    target: FluxWatchTarget,
) {
    tokio::spawn(async move {
        let gvk = GroupVersionKind::gvk(target.group, target.version, target.kind);
        let mut ar = ApiResource::from_gvk(&gvk);
        ar.plural = target.plural.to_string();
        let api: Api<DynamicObject> = if target.namespaced {
            match &session.namespace {
                Some(namespace) => Api::namespaced_with(client, namespace, &ar),
                None => Api::all_with(client, &ar),
            }
        } else {
            Api::all_with(client, &ar)
        };
        let stream = watcher::watcher(api, watcher_config).default_backoff();
        let mut store = ResourceStore::<FluxWatchInfo>::new();
        let mut publish_pending = false;
        let mut publish_tick =
            tokio::time::interval(Duration::from_millis(WATCH_PUBLISH_DEBOUNCE_MS));
        publish_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        publish_tick.tick().await;
        tokio::pin!(stream);

        loop {
            tokio::select! {
                biased;
                _ = cancel_rx.changed() => break,
                _ = publish_tick.tick(), if publish_pending => {
                    publish_pending = false;
                    if watch_tx
                        .send(WatchUpdate {
                            resource: WatchedResource::Flux,
                            context_generation: session.context_generation,
                            data: WatchPayload::Flux {
                                target,
                                items: sorted_flux_resources(&store),
                            },
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                item = stream.try_next() => {
                    match item {
                        Ok(Some(event)) => {
                            if process_flux_event(&mut store, target, event) {
                                publish_pending = true;
                            }
                        }
                        Ok(None) => {
                            if publish_pending
                                && !send_flux_watch_payload(
                                    &watch_tx,
                                    session.context_generation,
                                    target,
                                    &store,
                                )
                                .await
                            {
                                break;
                            }
                            warn!(
                                group = target.group,
                                kind = target.kind,
                                version = target.version,
                                "flux watch stream ended unexpectedly",
                            );
                            let _ = watch_tx.send(WatchUpdate {
                                resource: WatchedResource::Flux,
                                context_generation: session.context_generation,
                                data: WatchPayload::Error {
                                    message: format!(
                                        "flux watch stream terminated for {}/{}/{}",
                                        target.group, target.version, target.kind
                                    ),
                                },
                            }).await;
                            break;
                        }
                        Err(err) if flux_watch_error_is_terminal(&err) => {
                            if publish_pending
                                && !send_flux_watch_payload(
                                    &watch_tx,
                                    session.context_generation,
                                    target,
                                    &store,
                                )
                                .await
                            {
                                break;
                            }
                            warn!(
                                error = %err,
                                group = target.group,
                                kind = target.kind,
                                version = target.version,
                                "flux watch unavailable; watcher stopped",
                            );
                            break;
                        }
                        Err(err) => {
                            warn!(
                                error = %err,
                                group = target.group,
                                kind = target.kind,
                                version = target.version,
                                "flux watch stream error",
                            );
                            let _ = watch_tx.send(WatchUpdate {
                                resource: WatchedResource::Flux,
                                context_generation: session.context_generation,
                                data: WatchPayload::Error {
                                    message: err.to_string(),
                                },
                            }).await;
                        }
                    }
                }
            }
        }
    });
}

define_watcher! {
    name: pod,
    k8s_type: Pod,
    dto_type: PodInfo,
    converter: pod_to_info,
    resource_variant: Pods,
    scope: namespaced,
}

define_watcher! {
    name: deployment,
    k8s_type: Deployment,
    dto_type: DeploymentInfo,
    converter: deployment_to_info,
    resource_variant: Deployments,
    scope: namespaced,
}

define_watcher! {
    name: replicaset,
    k8s_type: ReplicaSet,
    dto_type: ReplicaSetInfo,
    converter: replicaset_to_info,
    resource_variant: ReplicaSets,
    scope: namespaced,
}

define_watcher! {
    name: statefulset,
    k8s_type: StatefulSet,
    dto_type: StatefulSetInfo,
    converter: statefulset_to_info,
    resource_variant: StatefulSets,
    scope: namespaced,
}

define_watcher! {
    name: daemonset,
    k8s_type: DaemonSet,
    dto_type: DaemonSetInfo,
    converter: daemonset_to_info,
    resource_variant: DaemonSets,
    scope: namespaced,
}

define_watcher! {
    name: service,
    k8s_type: Service,
    dto_type: ServiceInfo,
    converter: service_to_info,
    resource_variant: Services,
    scope: namespaced,
}

define_watcher! {
    name: node,
    k8s_type: Node,
    dto_type: NodeInfo,
    converter: node_to_info,
    resource_variant: Nodes,
    scope: cluster,
}

define_watcher! {
    name: replication_controller,
    k8s_type: ReplicationController,
    dto_type: ReplicationControllerInfo,
    converter: replication_controller_to_info,
    resource_variant: ReplicationControllers,
    scope: namespaced,
}

define_watcher! {
    name: job,
    k8s_type: Job,
    dto_type: JobInfo,
    converter: job_to_info,
    resource_variant: Jobs,
    scope: namespaced,
}

define_watcher! {
    name: cronjob,
    k8s_type: CronJob,
    dto_type: CronJobInfo,
    converter: cronjob_to_info,
    resource_variant: CronJobs,
    scope: namespaced,
}

define_watcher! {
    name: namespace,
    k8s_type: Namespace,
    dto_type: NamespaceInfo,
    converter: namespace_metadata_to_info,
    resource_variant: Namespaces,
    scope: cluster,
    mode: metadata,
}

/// Manages all watch tasks and their lifecycle.
///
/// Instances are single-use: after [`stop_all`](WatchManager::stop_all),
/// create a new `WatchManager` with the updated session key rather than
/// restarting watches on the same instance.
pub struct WatchManager {
    session: WatchSessionKey,
    cancel_tx: Option<tokio::sync::watch::Sender<()>>,
    active_scope: RefreshScope,
    requested_scope: RefreshScope,
    watcher_config: Option<watcher::Config>,
}

impl WatchManager {
    /// Creates a new watch manager for the given session.
    pub fn new(session: WatchSessionKey) -> Self {
        Self {
            session,
            cancel_tx: None,
            active_scope: RefreshScope::NONE,
            requested_scope: RefreshScope::NONE,
            watcher_config: None,
        }
    }

    /// Returns the current session key.
    pub fn session_key(&self) -> &WatchSessionKey {
        &self.session
    }

    /// Returns the scope that auto-refresh can safely treat as watch-covered.
    ///
    /// Flux stays out of this scope so polling remains the fallback when
    /// asynchronous Flux target discovery fails or target watches terminate.
    pub fn active_scope(&self) -> RefreshScope {
        self.active_scope
    }

    /// Starts watch tasks needed by the initial visible scope.
    pub fn start_watches(
        &mut self,
        client: &K8sClient,
        watch_tx: mpsc::Sender<WatchUpdate>,
        watcher_config: watcher::Config,
        initial_scope: RefreshScope,
    ) {
        let (cancel_tx, _cancel_rx) = tokio::sync::watch::channel(());
        self.cancel_tx = Some(cancel_tx);
        self.watcher_config = Some(watcher_config);
        self.ensure_watches(client, watch_tx, initial_scope);
    }

    /// Starts any watch tasks not yet active for `scope`.
    pub fn ensure_watches(
        &mut self,
        client: &K8sClient,
        watch_tx: mpsc::Sender<WatchUpdate>,
        scope: RefreshScope,
    ) {
        let scope = normalize_watch_scope(scope);
        if should_restart_watches_for_scope(self.requested_scope, scope) {
            self.cancel_tx.take();
            let (cancel_tx, _cancel_rx) = tokio::sync::watch::channel(());
            self.cancel_tx = Some(cancel_tx);
            self.active_scope = RefreshScope::NONE;
            self.requested_scope = RefreshScope::NONE;
        }

        let missing_scope = scope.without(self.requested_scope);
        if missing_scope.is_empty() {
            return;
        }

        let Some(cancel_tx) = &self.cancel_tx else {
            return;
        };
        let Some(watcher_config) = self.watcher_config.clone() else {
            return;
        };
        let kube_client = client.get_client();
        let cancel_rx = cancel_tx.subscribe();

        if missing_scope.contains(RefreshScope::PODS) {
            start_pod_watch(
                kube_client.clone(),
                self.session.clone(),
                watch_tx.clone(),
                watcher_config.clone(),
                cancel_rx.clone(),
            );
        }
        if missing_scope.contains(RefreshScope::DEPLOYMENTS) {
            start_deployment_watch(
                kube_client.clone(),
                self.session.clone(),
                watch_tx.clone(),
                watcher_config.clone(),
                cancel_rx.clone(),
            );
        }
        if missing_scope.contains(RefreshScope::REPLICASETS) {
            start_replicaset_watch(
                kube_client.clone(),
                self.session.clone(),
                watch_tx.clone(),
                watcher_config.clone(),
                cancel_rx.clone(),
            );
        }
        if missing_scope.contains(RefreshScope::STATEFULSETS) {
            start_statefulset_watch(
                kube_client.clone(),
                self.session.clone(),
                watch_tx.clone(),
                watcher_config.clone(),
                cancel_rx.clone(),
            );
        }
        if missing_scope.contains(RefreshScope::DAEMONSETS) {
            start_daemonset_watch(
                kube_client.clone(),
                self.session.clone(),
                watch_tx.clone(),
                watcher_config.clone(),
                cancel_rx.clone(),
            );
        }
        if missing_scope.contains(RefreshScope::SERVICES) {
            start_service_watch(
                kube_client.clone(),
                self.session.clone(),
                watch_tx.clone(),
                watcher_config.clone(),
                cancel_rx.clone(),
            );
        }
        if missing_scope.contains(RefreshScope::NODES) {
            start_node_watch(
                kube_client.clone(),
                self.session.clone(),
                watch_tx.clone(),
                watcher_config.clone(),
                cancel_rx.clone(),
            );
        }
        if missing_scope.contains(RefreshScope::REPLICATION_CONTROLLERS) {
            start_replication_controller_watch(
                kube_client.clone(),
                self.session.clone(),
                watch_tx.clone(),
                watcher_config.clone(),
                cancel_rx.clone(),
            );
        }
        if missing_scope.contains(RefreshScope::JOBS) {
            start_job_watch(
                kube_client.clone(),
                self.session.clone(),
                watch_tx.clone(),
                watcher_config.clone(),
                cancel_rx.clone(),
            );
        }
        if missing_scope.contains(RefreshScope::CRONJOBS) {
            start_cronjob_watch(
                kube_client.clone(),
                self.session.clone(),
                watch_tx.clone(),
                watcher_config.clone(),
                cancel_rx.clone(),
            );
        }
        if missing_scope.contains(RefreshScope::NAMESPACES) {
            start_namespace_watch(
                kube_client.clone(),
                self.session.clone(),
                watch_tx.clone(),
                watcher_config.clone(),
                cancel_rx.clone(),
            );
        }

        if missing_scope.contains(RefreshScope::FLUX) {
            // Discover Flux targets in background so watch-manager startup never
            // blocks the UI event loop on CRD probing.
            let flux_client = client.clone();
            let flux_session = self.session.clone();
            let flux_watch_tx = watch_tx.clone();
            let flux_watcher_config = watcher_config.clone();
            let mut flux_cancel_rx = cancel_rx.clone();
            let flux_kube_client = kube_client.clone();
            tokio::spawn(async move {
                let targets = match flux_client
                    .discover_flux_watch_targets_cancellable(&mut flux_cancel_rx)
                    .await
                {
                    Ok(Some(targets)) => targets,
                    Ok(None) => return,
                    Err(err) => {
                        warn!(error = %err, "failed discovering Flux watch targets; Flux watcher disabled");
                        return;
                    }
                };

                if flux_cancel_rx.has_changed().unwrap_or(true) {
                    return;
                }
                for target in targets {
                    start_flux_watch(
                        flux_kube_client.clone(),
                        flux_session.clone(),
                        flux_watch_tx.clone(),
                        flux_watcher_config.clone(),
                        flux_cancel_rx.clone(),
                        target,
                    );
                }
            });
        }

        self.requested_scope = self.requested_scope.union(scope);
        self.active_scope = active_scope_after_starting_missing(self.active_scope, missing_scope);
    }

    /// Stops all running watch tasks.
    pub fn stop_all(&mut self) {
        // Dropping the sender causes all receivers to see a changed() error,
        // which terminates the select! in each watcher task.
        self.cancel_tx.take();
        self.active_scope = RefreshScope::NONE;
        self.requested_scope = RefreshScope::NONE;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use kube::core::PartialObjectMeta;
    use kube::runtime::watcher::{InitialListStrategy, ListSemantic};

    fn make_pod_info(name: &str) -> PodInfo {
        PodInfo {
            name: name.to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..Default::default()
        }
    }

    fn make_pod_info_ns(name: &str, namespace: &str) -> PodInfo {
        PodInfo {
            name: name.to_string(),
            namespace: namespace.to_string(),
            status: "Running".to_string(),
            ..Default::default()
        }
    }

    fn make_pod(name: &str, uid: &str) -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                uid: Some(uid.to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn make_namespace_meta(
        name: &str,
        uid: &str,
        terminating: bool,
    ) -> PartialObjectMeta<Namespace> {
        PartialObjectMeta {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                uid: Some(uid.to_string()),
                deletion_timestamp: terminating.then(|| {
                    k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(crate::time::now())
                }),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn make_flux_object(name: &str, uid: &str, resource_version: &str) -> DynamicObject {
        DynamicObject {
            types: None,
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("flux-system".to_string()),
                uid: Some(uid.to_string()),
                resource_version: Some(resource_version.to_string()),
                generation: resource_version.parse().ok(),
                ..Default::default()
            },
            data: serde_json::json!({}),
        }
    }

    fn flux_watch_target() -> FluxWatchTarget {
        FluxWatchTarget {
            group: "kustomize.toolkit.fluxcd.io",
            version: "v1",
            kind: "Kustomization",
            plural: "kustomizations",
            namespaced: true,
        }
    }

    #[test]
    fn should_restart_watches_when_desired_scope_shrinks() {
        let active = RefreshScope::CORE_OVERVIEW.union(RefreshScope::FLUX);

        assert!(should_restart_watches_for_scope(
            active,
            RefreshScope::DASHBOARD_WATCHED
        ));
        assert!(!should_restart_watches_for_scope(
            RefreshScope::DASHBOARD_WATCHED.union(RefreshScope::NAMESPACES),
            RefreshScope::CORE_OVERVIEW
        ));
        assert!(!should_restart_watches_for_scope(
            RefreshScope::NAMESPACES,
            RefreshScope::NONE
        ));
    }

    #[test]
    fn active_scope_excludes_flux_until_payload_fallback_safe() {
        let active = active_scope_after_starting_missing(
            RefreshScope::NONE,
            RefreshScope::FLUX.union(RefreshScope::PODS),
        );

        assert!(active.contains(RefreshScope::PODS));
        assert!(!active.contains(RefreshScope::FLUX));
    }

    #[test]
    fn normalized_watch_scope_uses_polling_for_flux() {
        let scope = normalize_watch_scope(RefreshScope::FLUX.union(RefreshScope::PODS));

        assert!(scope.contains(RefreshScope::PODS));
        assert!(scope.contains(RefreshScope::NAMESPACES));
        assert!(!scope.contains(RefreshScope::FLUX));
    }

    // ── ResourceStore tests ──

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
        assert!(store.apply_event("uid-1".to_string(), make_pod_info("pod-a")));
        assert_eq!(store.items.len(), 1);

        // Upsert same UID with updated data
        let mut updated = make_pod_info("pod-a");
        updated.status = "Succeeded".to_string();
        assert!(store.apply_event("uid-1".to_string(), updated));
        assert_eq!(store.items.len(), 1);
        assert_eq!(store.items["uid-1"].status, "Succeeded");
    }

    #[test]
    fn resource_store_delete() {
        let mut store = ResourceStore::<PodInfo>::new();
        assert!(store.apply_event("uid-1".to_string(), make_pod_info("pod-a")));
        assert!(store.apply_event("uid-2".to_string(), make_pod_info("pod-b")));
        assert_eq!(store.items.len(), 2);

        assert!(store.remove("uid-1"));
        assert_eq!(store.items.len(), 1);
        assert!(!store.items.contains_key("uid-1"));
        assert!(store.items.contains_key("uid-2"));
    }

    #[test]
    fn resource_store_publish_unsorted() {
        let mut store = ResourceStore::<PodInfo>::new();
        assert!(store.apply_event("uid-3".to_string(), make_pod_info("pod-c")));
        assert!(store.apply_event("uid-1".to_string(), make_pod_info("pod-a")));
        assert!(store.apply_event("uid-2".to_string(), make_pod_info("pod-b")));

        let mut snapshot = store.publish();
        sort_pods(&mut snapshot);
        assert_eq!(snapshot[0].name, "pod-a");
        assert_eq!(snapshot[1].name, "pod-b");
        assert_eq!(snapshot[2].name, "pod-c");
    }

    #[test]
    fn sort_pods_tie_breaks_same_name_by_namespace() {
        let mut snapshot = vec![
            make_pod_info_ns("api", "team-b"),
            make_pod_info_ns("api", "team-a"),
            make_pod_info_ns("worker", "team-a"),
        ];

        sort_pods(&mut snapshot);

        assert_eq!(snapshot[0].namespace, "team-a");
        assert_eq!(snapshot[0].name, "api");
        assert_eq!(snapshot[1].namespace, "team-b");
        assert_eq!(snapshot[1].name, "api");
        assert_eq!(snapshot[2].name, "worker");
    }

    #[test]
    fn resource_store_clear_resets_state() {
        let mut store = ResourceStore::<PodInfo>::new();
        assert!(store.apply_event("uid-1".to_string(), make_pod_info("pod-a")));
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
        assert!(store.apply_event("uid-old".to_string(), make_pod_info("old-pod")));
        assert_eq!(store.items.len(), 1);

        // A new init cycle should replace the old data
        store.begin_init();
        store.apply_init_page("uid-new".to_string(), make_pod_info("new-pod"));
        store.commit_init();

        assert_eq!(store.items.len(), 1);
        assert!(store.items.contains_key("uid-new"));
        assert!(!store.items.contains_key("uid-old"));
    }

    #[test]
    fn resource_store_identical_apply_is_filtered() {
        let mut store = ResourceStore::<PodInfo>::new();
        let pod = make_pod_info("pod-a");

        assert!(store.apply_event("uid-1".to_string(), pod.clone()));
        assert!(!store.apply_event("uid-1".to_string(), pod));
        assert_eq!(store.items.len(), 1);
    }

    #[test]
    fn resource_store_missing_delete_is_filtered() {
        let mut store = ResourceStore::<PodInfo>::new();
        assert!(!store.remove("missing"));
    }

    #[test]
    fn resource_store_identical_relist_is_filtered() {
        let mut store = ResourceStore::<PodInfo>::new();
        assert!(store.apply_event("uid-1".to_string(), make_pod_info("pod-a")));

        store.begin_init();
        store.apply_init_page("uid-1".to_string(), make_pod_info("pod-a"));

        assert!(!store.commit_init());
        assert_eq!(store.readiness, StoreReadiness::Watching);
        assert_eq!(store.items.len(), 1);
    }

    #[test]
    fn recommended_watch_config_uses_streaming_lists_for_v134_plus() {
        let cfg = recommended_watch_config(Some(&ClusterVersionInfo {
            git_version: "v1.34.2-gke.100".to_string(),
            platform: "linux/arm64".to_string(),
        }));

        assert_eq!(
            cfg.initial_list_strategy,
            InitialListStrategy::StreamingList
        );
    }

    #[test]
    fn recommended_watch_config_falls_back_to_any_semantic_before_v134() {
        let cfg = recommended_watch_config(Some(&ClusterVersionInfo {
            git_version: "v1.33.7".to_string(),
            platform: "linux/amd64".to_string(),
        }));

        assert_eq!(cfg.initial_list_strategy, InitialListStrategy::ListWatch);
        assert_eq!(cfg.list_semantic, ListSemantic::Any);
    }

    #[test]
    fn recommended_watch_config_defaults_to_any_semantic_when_version_unknown() {
        let cfg = recommended_watch_config(None);

        assert_eq!(cfg.initial_list_strategy, InitialListStrategy::ListWatch);
        assert_eq!(cfg.list_semantic, ListSemantic::Any);
    }

    #[test]
    fn parse_kubernetes_minor_version_handles_distribution_suffixes() {
        assert_eq!(
            parse_kubernetes_minor_version("v1.34.2-gke.100"),
            Some((1, 34))
        );
        assert_eq!(parse_kubernetes_minor_version("v1.35+"), Some((1, 35)));
        assert_eq!(parse_kubernetes_minor_version("invalid"), None);
    }

    #[test]
    fn process_flux_event_filters_identical_apply() {
        let target = flux_watch_target();
        let mut store = ResourceStore::<FluxWatchInfo>::new();
        assert!(!process_flux_event(&mut store, target, Event::Init));
        assert!(!process_flux_event(
            &mut store,
            target,
            Event::InitApply(make_flux_object("apps", "uid-a", "10"))
        ));
        assert!(process_flux_event(&mut store, target, Event::InitDone));
        assert!(!process_flux_event(
            &mut store,
            target,
            Event::Apply(make_flux_object("apps", "uid-a", "10"))
        ));
        assert!(process_flux_event(
            &mut store,
            target,
            Event::Apply(make_flux_object("apps", "uid-a", "11"))
        ));
        assert!(process_flux_event(
            &mut store,
            target,
            Event::Delete(make_flux_object("apps", "uid-a", "11"))
        ));
        assert!(!process_flux_event(
            &mut store,
            target,
            Event::Delete(make_flux_object("apps", "uid-a", "11"))
        ));
    }

    #[test]
    fn flux_watch_info_equality_ignores_age() {
        let mut left = FluxResourceInfo {
            name: "apps".to_string(),
            namespace: Some("flux-system".to_string()),
            kind: "Kustomization".to_string(),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            version: "v1".to_string(),
            plural: "kustomizations".to_string(),
            age: Some(Duration::from_secs(10)),
            ..FluxResourceInfo::default()
        };
        let mut right = left.clone();
        right.age = Some(Duration::from_secs(20));

        assert_eq!(FluxWatchInfo(left.clone()), FluxWatchInfo(right.clone()));

        right.status = "NotReady".to_string();
        left.status = "Ready".to_string();
        assert_ne!(FluxWatchInfo(left), FluxWatchInfo(right));
    }

    // ── process_pod_event tests ──

    #[test]
    fn process_event_init_clears_and_sets_listing() {
        let mut store = ResourceStore::<PodInfo>::new();
        store.apply_event("uid-1".to_string(), make_pod_info("old"));

        let publish = process_pod_event(&mut store, Event::Init);
        assert!(!publish);
        assert_eq!(store.readiness, StoreReadiness::Listing);
    }

    #[test]
    fn process_event_init_apply_buffers_without_publish() {
        let mut store = ResourceStore::<PodInfo>::new();
        store.begin_init();

        let publish = process_pod_event(&mut store, Event::InitApply(make_pod("pod-a", "uid-a")));
        assert!(!publish);
        assert!(store.items.is_empty());
        assert_eq!(store.init_buffer.len(), 1);
    }

    #[test]
    fn process_event_init_done_commits_and_publishes() {
        let mut store = ResourceStore::<PodInfo>::new();
        store.begin_init();
        store.apply_init_page("uid-a".to_string(), make_pod_info("pod-a"));

        let publish = process_pod_event(&mut store, Event::InitDone);
        assert!(publish);
        assert_eq!(store.readiness, StoreReadiness::Watching);
        assert_eq!(store.items.len(), 1);
    }

    #[test]
    fn process_event_apply_upserts_and_publishes() {
        let mut store = ResourceStore::<PodInfo>::new();

        let publish = process_pod_event(&mut store, Event::Apply(make_pod("pod-a", "uid-a")));
        assert!(publish);
        assert_eq!(store.items.len(), 1);
    }

    #[test]
    fn process_event_identical_apply_is_filtered() {
        let mut store = ResourceStore::<PodInfo>::new();
        assert!(process_pod_event(
            &mut store,
            Event::Apply(make_pod("pod-a", "uid-a"))
        ));

        let publish = process_pod_event(&mut store, Event::Apply(make_pod("pod-a", "uid-a")));
        assert!(!publish);
        assert_eq!(store.items.len(), 1);
    }

    #[test]
    fn process_event_delete_removes_and_publishes() {
        let mut store = ResourceStore::<PodInfo>::new();
        assert!(store.apply_event("uid-a".to_string(), make_pod_info("pod-a")));

        let publish = process_pod_event(&mut store, Event::Delete(make_pod("pod-a", "uid-a")));
        assert!(publish);
        assert!(store.items.is_empty());
    }

    #[test]
    fn process_event_missing_delete_is_filtered() {
        let mut store = ResourceStore::<PodInfo>::new();
        let publish = process_pod_event(&mut store, Event::Delete(make_pod("pod-a", "uid-a")));
        assert!(!publish);
    }

    #[test]
    fn process_event_empty_uid_skipped() {
        let mut store = ResourceStore::<PodInfo>::new();
        let pod_no_uid = Pod {
            metadata: ObjectMeta {
                name: Some("no-uid-pod".to_string()),
                uid: None,
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(!process_pod_event(
            &mut store,
            Event::Apply(pod_no_uid.clone())
        ));
        assert!(store.items.is_empty());

        assert!(!process_pod_event(&mut store, Event::Delete(pod_no_uid)));
        assert!(store.items.is_empty());
    }

    #[test]
    fn process_event_full_init_cycle() {
        let mut store = ResourceStore::<PodInfo>::new();

        assert!(!process_pod_event(&mut store, Event::Init));
        assert!(!process_pod_event(
            &mut store,
            Event::InitApply(make_pod("pod-a", "uid-a"))
        ));
        assert!(!process_pod_event(
            &mut store,
            Event::InitApply(make_pod("pod-b", "uid-b"))
        ));
        let publish = process_pod_event(&mut store, Event::InitDone);
        assert!(publish);
        assert_eq!(store.items.len(), 2);
        assert_eq!(store.readiness, StoreReadiness::Watching);
    }

    #[test]
    fn process_namespace_event_uses_metadata_converter() {
        let mut store = ResourceStore::<NamespaceInfo>::new();

        assert!(!process_namespace_event(&mut store, Event::Init));
        assert!(!process_namespace_event(
            &mut store,
            Event::InitApply(make_namespace_meta("default", "uid-default", false)),
        ));
        assert!(process_namespace_event(&mut store, Event::InitDone));

        let snapshot = store.publish();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].name, "default");
        assert_eq!(snapshot[0].status, "Active");
    }

    #[test]
    fn process_namespace_event_marks_terminating_on_delete_timestamp() {
        let mut store = ResourceStore::<NamespaceInfo>::new();

        assert!(process_namespace_event(
            &mut store,
            Event::Apply(make_namespace_meta("staging", "uid-staging", true)),
        ));

        let snapshot = store.publish();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].status, "Terminating");
    }
}
