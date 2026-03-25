//! Async result types used as channel payloads in the event loop.

use std::time::Instant;

use kubectui::{
    app::{AppView, DetailViewState, ResourceRef},
    k8s::{
        client::K8sClient,
        dtos::K8sEventInfo,
        exec::DebugContainerLaunchResult,
        helm::{HelmHistoryResult, HelmValuesDiffResult},
        probes::ContainerProbes,
        relationships::RelationNode,
        workload_logs::WorkloadLogTarget,
    },
    state::{GlobalState, RefreshOptions, RefreshScope},
    time::AppTimestamp,
};

#[derive(Debug)]
pub enum LogsViewerAsyncResult {
    Containers {
        request_id: u64,
        pod_name: String,
        namespace: String,
        result: Result<Vec<String>, String>,
    },
    Tail {
        request_id: u64,
        pod_name: String,
        namespace: String,
        container_name: String,
        result: Result<Vec<String>, String>,
    },
}

#[derive(Debug)]
pub struct RefreshAsyncResult {
    pub request_id: u64,
    pub context_generation: u64,
    pub requested_namespace: Option<String>,
    pub result: Result<GlobalState, String>,
}

#[derive(Debug, Clone)]
pub struct QueuedRefresh {
    pub request_id: u64,
    pub namespace: Option<String>,
    pub primary_scope: RefreshScope,
    pub options: RefreshOptions,
    pub context_generation: u64,
}

#[derive(Debug, Default)]
pub struct RefreshRuntimeState {
    pub request_seq: u64,
    pub in_flight_id: Option<u64>,
    pub in_flight_task: Option<tokio::task::JoinHandle<()>>,
    pub queued_refresh: Option<QueuedRefresh>,
    pub context_generation: u64,
}

#[derive(Debug)]
pub struct DeleteAsyncResult {
    pub request_id: u64,
    pub action_history_id: u64,
    pub context_generation: u64,
    pub origin_view: AppView,
    pub resource: ResourceRef,
    pub result: Result<(), String>,
}

#[derive(Debug)]
pub struct ScaleAsyncResult {
    pub action_history_id: u64,
    pub context_generation: u64,
    pub origin_view: AppView,
    pub resource: ResourceRef,
    pub target_replicas: i32,
    pub resource_label: String,
    pub result: Result<(), String>,
}

#[derive(Debug)]
pub struct RolloutRestartAsyncResult {
    pub action_history_id: u64,
    pub context_generation: u64,
    pub origin_view: AppView,
    pub resource_label: String,
    pub result: Result<(), String>,
}

#[derive(Debug)]
pub struct FluxReconcileAsyncResult {
    pub action_history_id: u64,
    pub context_generation: u64,
    pub origin_view: AppView,
    pub resource: ResourceRef,
    pub resource_label: String,
    pub baseline: Option<FluxReconcileObservedState>,
    pub result: Result<(), String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FluxReconcileObservedState {
    pub status: String,
    pub message: Option<String>,
    pub last_reconcile_time: Option<AppTimestamp>,
    pub last_applied_revision: Option<String>,
    pub last_attempted_revision: Option<String>,
    pub observed_generation: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct PendingFluxReconcileVerification {
    pub action_history_id: u64,
    pub resource: ResourceRef,
    pub resource_label: String,
    pub baseline: Option<FluxReconcileObservedState>,
    pub deadline: Instant,
}

#[derive(Debug)]
pub struct TriggerCronJobAsyncResult {
    pub action_history_id: u64,
    pub context_generation: u64,
    pub origin_view: AppView,
    pub resource_label: String,
    pub result: Result<String, String>,
}

#[derive(Debug)]
pub struct SetCronJobSuspendAsyncResult {
    pub action_history_id: u64,
    pub context_generation: u64,
    pub origin_view: AppView,
    pub resource_label: String,
    pub suspend: bool,
    pub result: Result<(), String>,
}

#[derive(Debug)]
pub struct ProbeAsyncResult {
    pub resource: ResourceRef,
    pub result: Result<Vec<(String, ContainerProbes)>, String>,
}

#[derive(Debug)]
pub struct ExecBootstrapResult {
    pub session_id: u64,
    pub resource: ResourceRef,
    pub result: Result<Vec<String>, String>,
}

#[derive(Debug)]
pub struct DebugContainerDialogBootstrapResult {
    pub request_id: u64,
    pub resource: ResourceRef,
    pub result: Result<Vec<String>, String>,
}

#[derive(Debug)]
pub struct DebugContainerLaunchAsyncResult {
    pub action_history_id: u64,
    pub context_generation: u64,
    pub origin_view: AppView,
    pub resource: ResourceRef,
    pub session_id: u64,
    pub result: Result<DebugContainerLaunchResult, String>,
}

#[derive(Debug)]
pub struct WorkloadLogsBootstrapResult {
    pub session_id: u64,
    pub resource: ResourceRef,
    pub result: Result<Vec<WorkloadLogTarget>, String>,
}

#[derive(Debug)]
pub struct ExtensionFetchResult {
    pub crd_name: String,
    pub result: Result<Vec<kubectui::k8s::dtos::CustomResourceInfo>, String>,
}

#[derive(Debug)]
pub struct DetailAsyncResult {
    pub request_id: u64,
    pub resource: ResourceRef,
    pub result: Result<DetailViewState, String>,
}

#[derive(Debug)]
pub struct ResourceDiffAsyncResult {
    pub request_id: u64,
    pub resource: ResourceRef,
    pub result: Result<String, String>,
}

#[derive(Debug)]
pub struct HelmHistoryAsyncResult {
    pub request_id: u64,
    pub resource: ResourceRef,
    pub result: Result<HelmHistoryResult, String>,
}

#[derive(Debug)]
pub struct HelmValuesDiffAsyncResult {
    pub request_id: u64,
    pub resource: ResourceRef,
    pub result: Result<HelmValuesDiffResult, String>,
}

#[derive(Debug)]
pub struct HelmRollbackAsyncResult {
    pub action_history_id: u64,
    pub context_generation: u64,
    pub origin_view: AppView,
    pub resource: ResourceRef,
    pub target_revision: i32,
    pub result: Result<String, String>,
}

#[derive(Debug)]
pub struct EventsAsyncResult {
    pub request_id: u64,
    pub context_generation: u64,
    pub requested_namespace: Option<String>,
    pub result: Result<Vec<K8sEventInfo>, String>,
}

#[derive(Debug)]
pub struct RelationsAsyncResult {
    pub request_id: u64,
    pub resource: ResourceRef,
    pub result: Result<Vec<RelationNode>, String>,
}

#[derive(Debug, Default)]
pub struct EventsFetchRuntimeState {
    pub request_seq: u64,
    pub in_flight_id: Option<u64>,
    pub in_flight_namespace: Option<Option<String>>,
    pub in_flight_task: Option<tokio::task::JoinHandle<()>>,
    pub queued_namespace: Option<Option<String>>,
}

#[derive(Debug, Clone, Copy)]
pub enum NodeOpKind {
    Cordon,
    Uncordon,
    Drain,
}

impl NodeOpKind {
    pub fn label(self) -> &'static str {
        match self {
            NodeOpKind::Cordon => "Cordon",
            NodeOpKind::Uncordon => "Uncordon",
            NodeOpKind::Drain => "Drain",
        }
    }
}

#[derive(Debug)]
pub struct NodeOpsAsyncResult {
    pub action_history_id: u64,
    pub context_generation: u64,
    pub origin_view: AppView,
    pub node_name: String,
    pub op_kind: NodeOpKind,
    pub result: Result<(), String>,
}

#[derive(Debug, Clone)]
pub struct DeferredRefreshTrigger {
    pub context_generation: u64,
    pub view: AppView,
    pub dispatch: RefreshDispatch,
    pub namespace: Option<String>,
}

pub struct MutationRuntime<'a> {
    pub global_state: &'a mut GlobalState,
    pub client: &'a K8sClient,
    pub refresh_tx: &'a tokio::sync::mpsc::Sender<RefreshAsyncResult>,
    pub deferred_refresh_tx: &'a tokio::sync::mpsc::Sender<DeferredRefreshTrigger>,
    pub refresh_state: &'a mut RefreshRuntimeState,
    pub snapshot_dirty: &'a mut bool,
    pub auto_refresh: &'a mut tokio::time::Interval,
    pub status_message_clear_at: &'a mut Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
pub struct RefreshDispatch {
    pub primary_scope: RefreshScope,
    pub options: RefreshOptions,
}

impl RefreshDispatch {
    pub const fn new(primary_scope: RefreshScope, scope: RefreshScope) -> Self {
        Self {
            primary_scope,
            options: RefreshOptions {
                scope,
                include_cluster_info: false,
                skip_core: false,
            },
        }
    }
}
