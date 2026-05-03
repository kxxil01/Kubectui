//! Detail view state types extracted from the main app module.

use crate::{
    authorization::ActionAuthorizationMap,
    cronjob::CronJobHistoryEntry,
    k8s::{
        client::EventInfo,
        dtos::{NodeMetricsInfo, OwnerRefInfo, PodMetricsInfo},
    },
    log_investigation::PodLogPreset,
    log_investigation::{
        LogEntry, LogQueryMode, LogTimeWindow, compile_query, entry_matches_correlation,
        entry_matches_query, entry_matches_time_window, format_jump_target,
        nearest_timestamp_index, parse_jump_target,
    },
    ui::{
        clear_input_at_cursor,
        components::{
            debug_container_dialog::DebugContainerDialogState,
            node_debug_dialog::NodeDebugDialogState,
            probe_panel::ProbePanelState as ProbePanelComponentState,
            scale_dialog::ScaleDialogState,
        },
        move_cursor_end,
    },
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
    DebugContainer,
    NodeDebug,
    Scale,
    ProbePanel,
}

/// Maximum number of log lines retained in the viewer buffer.
/// Older lines are dropped when this limit is exceeded.
pub const MAX_LOG_LINES: usize = 10_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilteredLogWindow {
    pub total: usize,
    pub cursor: usize,
    pub indices: Vec<usize>,
}

/// In-detail logs viewer state.
#[derive(Debug, Clone)]
pub struct LogsViewerState {
    pub scroll_offset: usize,
    pub follow_mode: bool,
    pub lines: Vec<LogEntry>,
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
    pub search_mode: LogQueryMode,
    pub compiled_search: Option<regex::Regex>,
    pub search_error: Option<String>,
    pub time_window: LogTimeWindow,
    pub correlation_request_id: Option<String>,
    pub search_input: String,
    pub search_cursor: usize,
    pub searching: bool,
    pub time_jump_input: String,
    pub time_jump_cursor: usize,
    pub jumping_to_time: bool,
    pub time_jump_error: Option<String>,
    pub structured_view: bool,
}

impl Default for LogsViewerState {
    fn default() -> Self {
        Self {
            scroll_offset: 0,
            follow_mode: false,
            lines: Vec::new(),
            pod_name: String::new(),
            pod_namespace: String::new(),
            container_name: String::new(),
            containers: Vec::new(),
            picking_container: false,
            container_cursor: 0,
            pending_container_request_id: None,
            pending_logs_request_id: None,
            loading: false,
            error: None,
            previous_logs: false,
            show_timestamps: false,
            search_query: String::new(),
            search_mode: LogQueryMode::Substring,
            compiled_search: None,
            search_error: None,
            time_window: LogTimeWindow::All,
            correlation_request_id: None,
            search_input: String::new(),
            search_cursor: 0,
            searching: false,
            time_jump_input: String::new(),
            time_jump_cursor: 0,
            jumping_to_time: false,
            time_jump_error: None,
            structured_view: true,
        }
    }
}

impl LogsViewerState {
    pub fn restart_for_pod(&mut self, pod_name: String, pod_namespace: String, request_id: u64) {
        self.scroll_offset = 0;
        self.lines.clear();
        self.pod_name = pod_name;
        self.pod_namespace = pod_namespace;
        self.containers.clear();
        self.picking_container = false;
        self.container_cursor = 0;
        self.pending_container_request_id = Some(request_id);
        self.pending_logs_request_id = None;
        self.loading = true;
        self.error = None;
        self.correlation_request_id = None;
        self.search_input = self.search_query.clone();
        move_cursor_end(&mut self.search_cursor, &self.search_input);
        self.searching = false;
        clear_input_at_cursor(&mut self.time_jump_input, &mut self.time_jump_cursor);
        self.jumping_to_time = false;
        self.time_jump_error = None;
    }

    pub fn apply_containers(&mut self, containers: Vec<String>) {
        let selected_container = if self.picking_container {
            self.containers.get(self.container_cursor).cloned()
        } else if self.container_name.is_empty() {
            None
        } else {
            Some(self.container_name.clone())
        };
        self.containers = containers;
        self.container_cursor = selected_container
            .and_then(|name| {
                self.containers
                    .iter()
                    .position(|container| container == &name)
            })
            .unwrap_or(0);
    }

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
        self.lines.push(LogEntry::from_raw(line));
        if self.lines.len() > MAX_LOG_LINES {
            let excess = self.lines.len() - MAX_LOG_LINES;
            self.lines.drain(..excess);
            self.scroll_offset = self.scroll_offset.saturating_sub(excess);
        }
    }

    pub fn open_search(&mut self) {
        self.searching = true;
        self.jumping_to_time = false;
        self.time_jump_error = None;
        self.search_input = self.search_query.clone();
        move_cursor_end(&mut self.search_cursor, &self.search_input);
    }

    pub fn commit_search(&mut self) {
        let previous_query = self.search_query.clone();
        if self.search_input == previous_query {
            self.search_error = None;
            self.searching = false;
            return;
        }
        let preserved_line = self.current_visible_line().cloned();
        let previous_compiled = self.compiled_search.clone();
        self.search_query = self.search_input.clone();
        self.search_error = None;
        match compile_query(&self.search_query, self.search_mode) {
            Ok(compiled) => {
                self.compiled_search = compiled;
            }
            Err(err) => {
                self.search_query = previous_query;
                self.compiled_search = previous_compiled;
                self.search_error = Some(err);
            }
        }
        self.searching = false;
        if !self.jump_to_first_match() {
            self.restore_filtered_scroll(preserved_line);
        }
    }

    pub fn cancel_search(&mut self) {
        self.search_input = self.search_query.clone();
        move_cursor_end(&mut self.search_cursor, &self.search_input);
        self.searching = false;
    }

    pub fn open_time_jump(&mut self) {
        self.jumping_to_time = true;
        self.time_jump_error = None;
        self.time_jump_input = self
            .current_visible_line()
            .and_then(LogEntry::timestamp)
            .map(format_jump_target)
            .unwrap_or_default();
        move_cursor_end(&mut self.time_jump_cursor, &self.time_jump_input);
    }

    pub fn commit_time_jump(&mut self) {
        self.time_jump_error = None;
        let target = match parse_jump_target(&self.time_jump_input) {
            Ok(target) => target,
            Err(err) => {
                self.time_jump_error = Some(err);
                return;
            }
        };
        let now = crate::time::now();
        let Some(index) = nearest_timestamp_index(
            self.lines
                .iter()
                .enumerate()
                .filter(|(_, entry)| self.matches_visible_filters_at(entry, now)),
            target,
        ) else {
            self.time_jump_error = Some(
                "No visible log lines have timestamps in the current investigation view."
                    .to_string(),
            );
            return;
        };
        self.scroll_offset = index;
        self.follow_mode = false;
        self.jumping_to_time = false;
    }

    pub fn cancel_time_jump(&mut self) {
        self.jumping_to_time = false;
        self.time_jump_error = None;
        self.time_jump_input.clear();
        self.time_jump_cursor = 0;
    }

    pub fn toggle_search_mode(&mut self) {
        let preserved_line = self.current_visible_line().cloned();
        self.search_mode = self.search_mode.toggle();
        self.search_error = None;
        match compile_query(&self.search_query, self.search_mode) {
            Ok(compiled) => {
                self.compiled_search = compiled;
            }
            Err(err) => {
                self.compiled_search = None;
                self.search_error = Some(err);
            }
        }
        if !self.jump_to_first_match() {
            self.restore_filtered_scroll(preserved_line);
        }
    }

    pub fn cycle_time_window(&mut self) {
        let preserved_line = self.current_visible_line().cloned();
        self.time_window = self.time_window.next();
        if !self.jump_to_first_match() {
            self.restore_filtered_scroll(preserved_line);
        }
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
        let now = crate::time::now();

        self.lines
            .iter()
            .enumerate()
            .skip(start)
            .find_map(|(index, line)| {
                (entry_matches_time_window(line, self.time_window, now)
                    && entry_matches_correlation(line, self.correlation_request_id.as_deref())
                    && entry_matches_query(
                        line,
                        &self.search_query,
                        self.search_mode,
                        self.compiled_search.as_ref(),
                        self.structured_view,
                    ))
                .then_some(index)
            })
    }

    fn find_match_backward(&self, end_exclusive: usize) -> Option<usize> {
        (!self.search_query.is_empty()).then_some(())?;
        let now = crate::time::now();

        self.lines[..end_exclusive]
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, line)| {
                (entry_matches_time_window(line, self.time_window, now)
                    && entry_matches_correlation(line, self.correlation_request_id.as_deref())
                    && entry_matches_query(
                        line,
                        &self.search_query,
                        self.search_mode,
                        self.compiled_search.as_ref(),
                        self.structured_view,
                    ))
                .then_some(index)
            })
    }

    pub fn matches_time_window(&self, line: &LogEntry) -> bool {
        entry_matches_time_window(line, self.time_window, crate::time::now())
    }

    pub fn filtered_indices(&self) -> Vec<usize> {
        let now = crate::time::now();
        self.lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| self.matches_visible_filters_at(line, now).then_some(index))
            .collect()
    }

    pub fn recent_visible_lines(&self, limit: usize) -> Vec<&LogEntry> {
        if limit == 0 {
            return Vec::new();
        }
        let now = crate::time::now();
        let mut collected = Vec::with_capacity(limit);
        for line in self.lines.iter().rev() {
            if collected.len() >= limit {
                break;
            }
            if self.matches_visible_filters_at(line, now) {
                collected.push(line);
            }
        }
        collected.reverse();
        collected
    }

    fn matches_visible_filters_at(&self, line: &LogEntry, now: crate::time::AppTimestamp) -> bool {
        entry_matches_time_window(line, self.time_window, now)
            && entry_matches_correlation(line, self.correlation_request_id.as_deref())
    }

    pub fn filtered_cursor(&self, filtered_indices: &[usize]) -> usize {
        filtered_indices
            .iter()
            .position(|index| *index >= self.scroll_offset)
            .unwrap_or_else(|| filtered_indices.len().saturating_sub(1))
    }

    pub fn filtered_window_indices(&self, viewport_rows: usize) -> FilteredLogWindow {
        let now = crate::time::now();
        let mut total = 0usize;
        let mut cursor = None;

        for (index, line) in self.lines.iter().enumerate() {
            if !self.matches_visible_filters_at(line, now) {
                continue;
            }

            if cursor.is_none() && index >= self.scroll_offset {
                cursor = Some(total);
            }
            total += 1;
        }

        if total == 0 {
            return FilteredLogWindow::default();
        }

        let cursor = cursor.unwrap_or_else(|| total.saturating_sub(1));
        let visible = viewport_rows.max(1).min(total);
        let start = cursor.min(total.saturating_sub(visible));
        let end = start + visible;
        let mut indices = Vec::with_capacity(end - start);
        let mut ordinal = 0usize;

        for (index, line) in self.lines.iter().enumerate() {
            if !self.matches_visible_filters_at(line, now) {
                continue;
            }

            if ordinal >= start && ordinal < end {
                indices.push(index);
            }
            ordinal += 1;

            if ordinal >= end {
                break;
            }
        }

        FilteredLogWindow {
            total,
            cursor,
            indices,
        }
    }

    pub fn current_visible_line(&self) -> Option<&LogEntry> {
        let now = crate::time::now();
        let mut last = None;
        for (index, line) in self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| self.matches_visible_filters_at(line, now))
        {
            if index >= self.scroll_offset {
                return Some(line);
            }
            last = Some(line);
        }
        last
    }

    fn restore_filtered_scroll(&mut self, preserved_line: Option<LogEntry>) {
        let now = crate::time::now();
        let target_scroll = self.scroll_offset;
        let preserved_line = preserved_line.as_ref();
        let mut target_visible = None;
        let mut last_visible = None;

        for (index, line) in self.lines.iter().enumerate() {
            if !self.matches_visible_filters_at(line, now) {
                continue;
            }

            if preserved_line.is_some_and(|preserved| line == preserved) {
                self.scroll_offset = index;
                return;
            }

            if target_visible.is_none() && index >= target_scroll {
                target_visible = Some(index);
            }

            last_visible = Some(index);
        }

        self.scroll_offset = target_visible.or(last_visible).unwrap_or(0);
    }

    pub fn scroll_filtered_up(&mut self) {
        let now = crate::time::now();
        let mut previous_visible = None;
        let mut current_visible = None;

        for (index, line) in self.lines.iter().enumerate() {
            if !self.matches_visible_filters_at(line, now) {
                continue;
            }

            if index >= self.scroll_offset {
                self.scroll_offset = current_visible.unwrap_or(index);
                return;
            }

            previous_visible = current_visible;
            current_visible = Some(index);
        }

        self.scroll_offset = previous_visible.or(current_visible).unwrap_or(0);
    }

    pub fn scroll_filtered_down(&mut self) {
        let now = crate::time::now();
        let mut cursor_seen = false;
        let mut last_visible = None;

        for (index, line) in self.lines.iter().enumerate() {
            if !self.matches_visible_filters_at(line, now) {
                continue;
            }

            last_visible = Some(index);

            if cursor_seen {
                self.scroll_offset = index;
                return;
            }

            if index >= self.scroll_offset {
                cursor_seen = true;
            }
        }

        if let Some(index) = last_visible {
            self.scroll_offset = index;
        }
    }

    pub fn scroll_filtered_top(&mut self) {
        let now = crate::time::now();
        self.scroll_offset = self
            .lines
            .iter()
            .enumerate()
            .find_map(|(index, line)| self.matches_visible_filters_at(line, now).then_some(index))
            .unwrap_or(0);
    }

    pub fn scroll_filtered_bottom(&mut self) {
        let now = crate::time::now();
        self.scroll_offset = self
            .lines
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, line)| self.matches_visible_filters_at(line, now).then_some(index))
            .unwrap_or(0);
    }

    pub fn preset_snapshot(&self) -> PodLogPreset {
        PodLogPreset {
            name: String::new(),
            query: self.search_query.clone(),
            mode: self.search_mode,
            time_window: self.time_window,
            structured_view: self.structured_view,
        }
    }

    pub fn apply_preset(&mut self, preset: &PodLogPreset) {
        let preserved_line = self.current_visible_line().cloned();
        self.searching = false;
        self.jumping_to_time = false;
        self.search_query = preset.query.clone();
        self.search_input = self.search_query.clone();
        move_cursor_end(&mut self.search_cursor, &self.search_input);
        clear_input_at_cursor(&mut self.time_jump_input, &mut self.time_jump_cursor);
        self.time_jump_error = None;
        self.search_mode = preset.mode;
        self.time_window = preset.time_window;
        self.correlation_request_id = None;
        self.structured_view = preset.structured_view;
        self.search_error = None;
        match compile_query(&self.search_query, self.search_mode) {
            Ok(compiled) => {
                self.compiled_search = compiled;
            }
            Err(err) => {
                self.compiled_search = None;
                self.search_error = Some(err);
            }
        }
        let found = self.jump_to_first_match();
        if self.search_query.is_empty() {
            self.follow_mode = false;
            self.scroll_offset = self.lines.len().saturating_sub(1);
        } else if !found {
            self.follow_mode = false;
            self.restore_filtered_scroll(preserved_line);
        }
    }

    pub fn toggle_correlation_on_current_line(&mut self) -> Result<Option<String>, String> {
        if self.correlation_request_id.is_some() {
            self.correlation_request_id = None;
            self.scroll_filtered_top();
            return Ok(None);
        }
        let Some(line) = self.current_visible_line() else {
            return Err("No visible log line is available for correlation.".to_string());
        };
        let Some(request_id) = line.request_id().map(str::to_string) else {
            return Err("The current log line does not contain a request token.".to_string());
        };
        self.correlation_request_id = Some(request_id.clone());
        self.scroll_filtered_top();
        Ok(Some(request_id))
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
    pub events_error: Option<String>,
    pub sections: Vec<String>,
    pub pod_metrics: Option<PodMetricsInfo>,
    pub node_metrics: Option<NodeMetricsInfo>,
    pub metrics_unavailable_message: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub debug_dialog: Option<DebugContainerDialogState>,
    pub node_debug_dialog: Option<NodeDebugDialogState>,
    pub scale_dialog: Option<ScaleDialogState>,
    pub probe_panel: Option<ProbePanelComponentState>,
    pub cronjob_history: Vec<CronJobHistoryEntry>,
    pub cronjob_history_selected: usize,
    pub top_panel_scroll: usize,
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
    pub fn has_loading_indicator(&self) -> bool {
        self.loading
            || self
                .debug_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.loading_targets || dialog.pending_launch)
            || self
                .node_debug_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.pending_launch)
    }

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

    pub fn scroll_top_panels_down(&mut self, step: usize) {
        self.top_panel_scroll = self.top_panel_scroll.saturating_add(step);
    }

    pub fn scroll_top_panels_up(&mut self, step: usize) {
        self.top_panel_scroll = self.top_panel_scroll.saturating_sub(step);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        log_investigation::LogEntry,
        ui::components::{DebugContainerDialogState, node_debug_dialog::NodeDebugDialogState},
    };

    #[test]
    fn detail_loading_indicator_reports_nested_loading_states() {
        let mut detail = DetailViewState::default();
        assert!(!detail.has_loading_indicator());

        detail.loading = true;
        assert!(detail.has_loading_indicator());

        detail.loading = false;
        detail.debug_dialog = Some(DebugContainerDialogState::new("pod-a", "default"));
        assert!(detail.has_loading_indicator());

        detail
            .debug_dialog
            .as_mut()
            .expect("debug dialog should exist")
            .set_target_containers(vec!["main".to_string()]);
        assert!(!detail.has_loading_indicator());

        detail
            .debug_dialog
            .as_mut()
            .expect("debug dialog should exist")
            .begin_launch(42);
        assert!(detail.has_loading_indicator());

        detail.debug_dialog = None;
        let mut node_dialog =
            NodeDebugDialogState::new("node-a", "default", vec!["default".to_string()]);
        node_dialog.begin_launch(43);
        detail.node_debug_dialog = Some(node_dialog);
        assert!(detail.has_loading_indicator());
    }

    #[test]
    fn logs_viewer_apply_containers_preserves_selected_identity() {
        let mut viewer = LogsViewerState {
            containers: vec![
                "main".to_string(),
                "sidecar".to_string(),
                "metrics".to_string(),
            ],
            picking_container: true,
            container_cursor: 1,
            ..LogsViewerState::default()
        };

        viewer.apply_containers(vec![
            "sidecar".to_string(),
            "metrics".to_string(),
            "main".to_string(),
        ]);

        assert_eq!(viewer.container_cursor, 0);
        assert_eq!(viewer.containers[viewer.container_cursor], "sidecar");
    }

    #[test]
    fn logs_viewer_invalid_regex_commit_keeps_previous_query_and_scroll() {
        let mut viewer = LogsViewerState::default();
        viewer.lines = vec![
            LogEntry::from_raw("first line"),
            LogEntry::from_raw("ready line"),
            LogEntry::from_raw("third line"),
        ];
        viewer.search_query = "ready".to_string();
        viewer.search_input = "ready".to_string();
        viewer.compiled_search =
            compile_query("ready", LogQueryMode::Regex).expect("regex should compile");
        viewer.search_mode = LogQueryMode::Regex;
        viewer.scroll_offset = 1;
        viewer.searching = true;

        viewer.search_input = "[".to_string();
        viewer.commit_search();

        assert_eq!(viewer.search_query, "ready");
        assert_eq!(viewer.scroll_offset, 1);
        assert!(viewer.search_error.is_some());
        assert!(!viewer.searching);
    }

    #[test]
    fn logs_viewer_unchanged_search_commit_keeps_scroll() {
        let mut viewer = LogsViewerState::default();
        viewer.lines = vec![
            LogEntry::from_raw("ready first"),
            LogEntry::from_raw("middle line"),
            LogEntry::from_raw("ready current"),
        ];
        viewer.search_query = "ready".to_string();
        viewer.search_input = "ready".to_string();
        viewer.compiled_search = compile_query("ready", LogQueryMode::Substring)
            .expect("substring query should compile");
        viewer.scroll_offset = 2;
        viewer.searching = true;

        viewer.commit_search();

        assert_eq!(viewer.search_query, "ready");
        assert_eq!(viewer.scroll_offset, 2);
        assert!(!viewer.searching);
        assert!(viewer.search_error.is_none());
    }

    #[test]
    fn logs_viewer_no_match_preset_restores_visible_anchor() {
        let mut viewer = LogsViewerState::default();
        viewer.lines = vec![
            LogEntry::from_raw("first line"),
            LogEntry::from_raw("ready line"),
            LogEntry::from_raw("third line"),
        ];
        viewer.scroll_offset = 1;

        viewer.apply_preset(&PodLogPreset {
            name: "no-match".to_string(),
            query: "does-not-exist".to_string(),
            mode: LogQueryMode::Substring,
            time_window: LogTimeWindow::All,
            structured_view: true,
        });

        assert_eq!(viewer.scroll_offset, 1);
        assert_eq!(viewer.search_query, "does-not-exist");
    }

    #[test]
    fn logs_viewer_restore_scroll_preserves_visible_line_identity() {
        let mut viewer = LogsViewerState::default();
        viewer.lines = vec![
            LogEntry::from_raw("request_id=req-7 visible first"),
            LogEntry::from_raw("middle line"),
            LogEntry::from_raw("request_id=req-7 visible selected"),
            LogEntry::from_raw("request_id=req-7 visible last"),
        ];
        viewer.correlation_request_id = Some("req-7".to_string());
        viewer.scroll_offset = 0;
        let selected = viewer.lines[2].clone();

        viewer.restore_filtered_scroll(Some(selected));

        assert_eq!(viewer.scroll_offset, 2);
        assert_eq!(
            viewer.current_visible_line().map(LogEntry::raw),
            Some("request_id=req-7 visible selected")
        );
    }

    #[test]
    fn logs_viewer_restore_scroll_clamps_to_last_visible_line() {
        let mut viewer = LogsViewerState::default();
        viewer.lines = vec![
            LogEntry::from_raw("first line"),
            LogEntry::from_raw("request_id=req-7 visible first"),
            LogEntry::from_raw("middle line"),
            LogEntry::from_raw("request_id=req-7 visible last"),
        ];
        viewer.correlation_request_id = Some("req-7".to_string());
        viewer.scroll_offset = usize::MAX;

        viewer.restore_filtered_scroll(None);

        assert_eq!(viewer.scroll_offset, 3);
        assert_eq!(
            viewer.current_visible_line().map(LogEntry::raw),
            Some("request_id=req-7 visible last")
        );
    }

    #[test]
    fn logs_viewer_current_visible_line_clamps_without_filtered_vector() {
        let mut viewer = LogsViewerState::default();
        viewer.lines = vec![
            LogEntry::from_raw("first line"),
            LogEntry::from_raw("second line"),
            LogEntry::from_raw("third line"),
        ];
        viewer.scroll_offset = usize::MAX;

        assert_eq!(
            viewer.current_visible_line().map(LogEntry::raw),
            Some("third line")
        );
    }

    #[test]
    fn logs_viewer_scroll_edges_use_visible_filtered_rows() {
        let mut viewer = LogsViewerState::default();
        viewer.lines = vec![
            LogEntry::from_raw("first line"),
            LogEntry::from_raw("request_id=req-7 visible first"),
            LogEntry::from_raw("middle line"),
            LogEntry::from_raw("request_id=req-7 visible last"),
            LogEntry::from_raw("last line"),
        ];
        viewer.correlation_request_id = Some("req-7".to_string());

        viewer.scroll_filtered_top();
        assert_eq!(viewer.scroll_offset, 1);

        viewer.scroll_filtered_bottom();
        assert_eq!(viewer.scroll_offset, 3);
    }

    #[test]
    fn logs_viewer_step_scroll_uses_visible_filtered_rows() {
        let mut viewer = LogsViewerState::default();
        viewer.lines = vec![
            LogEntry::from_raw("first line"),
            LogEntry::from_raw("request_id=req-7 visible first"),
            LogEntry::from_raw("middle line"),
            LogEntry::from_raw("request_id=req-7 visible second"),
            LogEntry::from_raw("request_id=req-7 visible third"),
            LogEntry::from_raw("last line"),
        ];
        viewer.correlation_request_id = Some("req-7".to_string());
        viewer.scroll_offset = 1;

        viewer.scroll_filtered_down();
        assert_eq!(viewer.scroll_offset, 3);

        viewer.scroll_filtered_down();
        assert_eq!(viewer.scroll_offset, 4);

        viewer.scroll_filtered_down();
        assert_eq!(viewer.scroll_offset, 4);

        viewer.scroll_filtered_up();
        assert_eq!(viewer.scroll_offset, 3);

        viewer.scroll_filtered_up();
        assert_eq!(viewer.scroll_offset, 1);

        viewer.scroll_offset = 0;
        viewer.scroll_filtered_down();
        assert_eq!(viewer.scroll_offset, 3);

        viewer.scroll_offset = 0;
        viewer.scroll_filtered_up();
        assert_eq!(viewer.scroll_offset, 1);

        viewer.scroll_offset = usize::MAX;
        viewer.scroll_filtered_down();
        assert_eq!(viewer.scroll_offset, 4);

        viewer.scroll_offset = usize::MAX;
        viewer.scroll_filtered_up();
        assert_eq!(viewer.scroll_offset, 3);
    }

    #[test]
    fn logs_viewer_filtered_window_collects_only_visible_filtered_rows() {
        let mut viewer = LogsViewerState::default();
        viewer.lines = vec![
            LogEntry::from_raw("first line"),
            LogEntry::from_raw("request_id=req-7 visible first"),
            LogEntry::from_raw("middle line"),
            LogEntry::from_raw("request_id=req-7 visible second"),
            LogEntry::from_raw("request_id=req-7 visible third"),
            LogEntry::from_raw("last line"),
        ];
        viewer.correlation_request_id = Some("req-7".to_string());
        viewer.scroll_offset = 3;

        let window = viewer.filtered_window_indices(2);

        assert_eq!(window.total, 3);
        assert_eq!(window.cursor, 1);
        assert_eq!(window.indices, vec![3, 4]);
    }

    #[test]
    fn logs_viewer_filtered_window_clamps_past_last_visible_row() {
        let mut viewer = LogsViewerState::default();
        viewer.lines = vec![
            LogEntry::from_raw("first line"),
            LogEntry::from_raw("request_id=req-7 visible first"),
            LogEntry::from_raw("middle line"),
            LogEntry::from_raw("request_id=req-7 visible second"),
            LogEntry::from_raw("request_id=req-7 visible third"),
            LogEntry::from_raw("last line"),
        ];
        viewer.correlation_request_id = Some("req-7".to_string());
        viewer.scroll_offset = usize::MAX;

        let window = viewer.filtered_window_indices(2);

        assert_eq!(window.total, 3);
        assert_eq!(window.cursor, 2);
        assert_eq!(window.indices, vec![3, 4]);
    }
}
