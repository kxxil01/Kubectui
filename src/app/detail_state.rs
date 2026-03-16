//! Detail view state types extracted from the main app module.

use crate::{
    authorization::ActionAuthorizationMap,
    cronjob::CronJobHistoryEntry,
    k8s::{
        client::EventInfo,
        dtos::{NodeMetricsInfo, OwnerRefInfo, PodMetricsInfo},
    },
    ui::components::{
        probe_panel::ProbePanelState as ProbePanelComponentState, scale_dialog::ScaleDialogState,
    },
    ui::contains_ci,
};

use super::ResourceRef;

/// Human-readable metadata displayed in the detail modal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetailMetadata {
    pub name: String,
    pub namespace: Option<String>,
    pub status: Option<String>,
    pub node_unschedulable: Option<bool>,
    pub cronjob_suspended: Option<bool>,
    pub node: Option<String>,
    pub ip: Option<String>,
    pub created: Option<String>,
    pub labels: Vec<(String, String)>,
    pub annotations: Vec<(String, String)>,
    pub owner_references: Vec<OwnerRefInfo>,
    pub flux_reconcile_enabled: bool,
    pub action_authorizations: ActionAuthorizationMap,
}

/// Top-level active component when detail modal is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveComponent {
    None,
    LogsViewer,
    PortForward,
    Scale,
    ProbePanel,
}

/// Maximum number of log lines retained in the viewer buffer.
/// Older lines are dropped when this limit is exceeded.
pub const MAX_LOG_LINES: usize = 10_000;

/// In-detail logs viewer state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogsViewerState {
    pub scroll_offset: usize,
    pub follow_mode: bool,
    pub lines: Vec<String>,
    pub pod_name: String,
    pub pod_namespace: String,
    pub container_name: String,
    /// All containers in this pod — populated before logs are fetched.
    pub containers: Vec<String>,
    /// When true, show the container picker instead of logs.
    pub picking_container: bool,
    /// Cursor index in the container picker list.
    pub container_cursor: usize,
    /// Monotonic request id for in-flight container list fetch.
    pub pending_container_request_id: Option<u64>,
    /// Monotonic request id for in-flight tail logs fetch.
    pub pending_logs_request_id: Option<u64>,
    pub loading: bool,
    pub error: Option<String>,
    /// When true, fetch logs from the previous (crashed/restarted) container.
    pub previous_logs: bool,
    /// When true, request timestamps from the Kubernetes API.
    pub show_timestamps: bool,
    pub search_query: String,
    pub search_input: String,
    pub searching: bool,
}

impl LogsViewerState {
    /// Appends a log line, evicting the oldest lines if the buffer exceeds [`MAX_LOG_LINES`].
    pub fn push_line(&mut self, line: String) {
        const MAX_LINE_BYTES: usize = 10_000;
        let line = if line.len() > MAX_LINE_BYTES {
            // Find the nearest char boundary at or before the limit to avoid
            // panicking on multi-byte UTF-8 sequences.
            let end = line.floor_char_boundary(MAX_LINE_BYTES);
            let mut truncated = line;
            truncated.truncate(end);
            truncated.push_str("…[truncated]");
            truncated
        } else {
            line
        };
        self.lines.push(line);
        if self.lines.len() > MAX_LOG_LINES {
            let excess = self.lines.len() - MAX_LOG_LINES;
            self.lines.drain(..excess);
            self.scroll_offset = self.scroll_offset.saturating_sub(excess);
        }
    }

    pub fn open_search(&mut self) {
        self.searching = true;
        self.search_input = self.search_query.clone();
    }

    pub fn commit_search(&mut self) {
        self.search_query = self.search_input.clone();
        self.searching = false;
        self.jump_to_first_match();
    }

    pub fn cancel_search(&mut self) {
        self.search_input = self.search_query.clone();
        self.searching = false;
    }

    pub fn jump_to_first_match(&mut self) -> bool {
        self.find_match_forward(0).is_some_and(|index| {
            self.scroll_offset = index;
            self.follow_mode = false;
            true
        })
    }

    pub fn jump_to_next_match(&mut self) -> bool {
        let start = self.scroll_offset.saturating_add(1);
        self.find_match_forward(start).is_some_and(|index| {
            self.scroll_offset = index;
            self.follow_mode = false;
            true
        })
    }

    pub fn jump_to_prev_match(&mut self) -> bool {
        if self.scroll_offset == 0 {
            return false;
        }

        self.find_match_backward(self.scroll_offset)
            .is_some_and(|index| {
                self.scroll_offset = index;
                self.follow_mode = false;
                true
            })
    }

    fn find_match_forward(&self, start: usize) -> Option<usize> {
        (!self.search_query.is_empty()).then_some(())?;

        self.lines
            .iter()
            .enumerate()
            .skip(start)
            .find_map(|(index, line)| contains_ci(line, &self.search_query).then_some(index))
    }

    fn find_match_backward(&self, end_exclusive: usize) -> Option<usize> {
        (!self.search_query.is_empty()).then_some(())?;

        self.lines[..end_exclusive]
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, line)| contains_ci(line, &self.search_query).then_some(index))
    }
}

/// Active form field in the lightweight port-forward dialog state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortForwardField {
    LocalPort,
    RemotePort,
    TunnelList,
}

/// In-detail port-forward dialog state used by keyboard routing tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortForwardDialogState {
    pub active_field: PortForwardField,
    pub local_port: String,
    pub remote_port: String,
}

impl Default for PortForwardDialogState {
    fn default() -> Self {
        Self {
            active_field: PortForwardField::LocalPort,
            local_port: String::new(),
            remote_port: String::new(),
        }
    }
}

/// In-detail scale dialog state used by keyboard routing tests.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScaleDialogInputState {
    pub replica_input: String,
    pub target_replicas: i32,
}

/// In-detail probe panel state used by keyboard routing tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProbePanelState {
    pub probes: Vec<String>,
    pub expanded: Vec<bool>,
    pub selected_idx: usize,
}

/// Detail modal state for the currently focused resource.
#[derive(Debug, Clone, Default)]
pub struct DetailViewState {
    pub resource: Option<ResourceRef>,
    pub pending_request_id: Option<u64>,
    pub metadata: DetailMetadata,
    pub yaml: Option<String>,
    pub yaml_error: Option<String>,
    pub events: Vec<EventInfo>,
    pub sections: Vec<String>,
    pub pod_metrics: Option<PodMetricsInfo>,
    pub node_metrics: Option<NodeMetricsInfo>,
    pub metrics_unavailable_message: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub scale_dialog: Option<ScaleDialogState>,
    pub probe_panel: Option<ProbePanelComponentState>,
    pub cronjob_history: Vec<CronJobHistoryEntry>,
    pub cronjob_history_selected: usize,
    /// When true, a delete confirmation prompt is shown in the detail view.
    pub confirm_delete: bool,
    /// When true, a drain confirmation prompt is shown in the detail view.
    pub confirm_drain: bool,
    /// Target suspend value when a CronJob suspend/resume confirmation is shown.
    pub confirm_cronjob_suspend: Option<bool>,
    /// When true, metadata labels/annotations are shown in full (no truncation).
    pub metadata_expanded: bool,
}

impl DetailViewState {
    pub fn has_confirmation_dialog(&self) -> bool {
        self.confirm_delete || self.confirm_drain || self.confirm_cronjob_suspend.is_some()
    }

    pub fn selected_cronjob_history(&self) -> Option<&CronJobHistoryEntry> {
        self.cronjob_history.get(
            self.cronjob_history_selected
                .min(self.cronjob_history.len().saturating_sub(1)),
        )
    }

    pub fn select_next_cronjob_history(&mut self) {
        if !self.cronjob_history.is_empty() {
            let max = self.cronjob_history.len().saturating_sub(1);
            self.cronjob_history_selected = (self.cronjob_history_selected + 1).min(max);
        }
    }

    pub fn select_prev_cronjob_history(&mut self) {
        self.cronjob_history_selected = self.cronjob_history_selected.saturating_sub(1);
    }

    pub fn selected_detail_resource(&self) -> Option<ResourceRef> {
        match self.resource.as_ref() {
            Some(ResourceRef::CronJob(_, _)) => self
                .selected_cronjob_history()
                .map(|entry| ResourceRef::Job(entry.job_name.clone(), entry.namespace.clone())),
            _ => None,
        }
    }

    pub fn selected_logs_resource(&self) -> Option<ResourceRef> {
        match self.resource.as_ref() {
            Some(
                resource @ (ResourceRef::Pod(_, _)
                | ResourceRef::Deployment(_, _)
                | ResourceRef::StatefulSet(_, _)
                | ResourceRef::DaemonSet(_, _)
                | ResourceRef::ReplicaSet(_, _)
                | ResourceRef::ReplicationController(_, _)
                | ResourceRef::Job(_, _)),
            ) => Some(resource.clone()),
            Some(ResourceRef::CronJob(_, _)) => self
                .selected_cronjob_history()
                .filter(|entry| entry.has_log_target())
                .and_then(|_| self.selected_detail_resource()),
            _ => None,
        }
    }
}
