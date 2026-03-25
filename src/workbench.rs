//! Canonical workbench state for long-lived bottom-pane surfaces.

use crate::{
    action_history::ActionHistoryState,
    app::{LogsViewerState, ResourceRef},
    icons::tab_icon,
    k8s::client::EventInfo,
    network_policy_analysis::NetworkPolicyAnalysis,
    network_policy_connectivity::ConnectivityAnalysis,
    resource_diff::{ResourceDiffBaselineKind, ResourceDiffLine, ResourceDiffResult},
    secret::DecodedSecretEntry,
    timeline::{TimelineEntry, build_timeline},
    ui::components::{input_field::InputFieldWidget, port_forward_dialog::PortForwardDialog},
};

pub const DEFAULT_WORKBENCH_HEIGHT: u16 = 12;
pub const MIN_WORKBENCH_HEIGHT: u16 = 8;
pub const MAX_WORKBENCH_HEIGHT: u16 = 40;
pub const MAX_WORKLOAD_LOG_LINES: usize = 5_000;
pub const MAX_EXEC_OUTPUT_LINES: usize = 5_000;
pub const MAX_TIMELINE_EVENTS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkbenchTabKind {
    ActionHistory,
    ResourceYaml,
    ResourceDiff,
    DecodedSecret,
    ResourceEvents,
    PodLogs,
    WorkloadLogs,
    Exec,
    PortForward,
    Relations,
    NetworkPolicy,
    Connectivity,
}

impl WorkbenchTabKind {
    pub const fn title(self) -> &'static str {
        match self {
            WorkbenchTabKind::ActionHistory => "History",
            WorkbenchTabKind::ResourceYaml => "YAML",
            WorkbenchTabKind::ResourceDiff => "Drift",
            WorkbenchTabKind::DecodedSecret => "Decoded",
            WorkbenchTabKind::ResourceEvents => "Timeline",
            WorkbenchTabKind::PodLogs => "Logs",
            WorkbenchTabKind::WorkloadLogs => "Workload Logs",
            WorkbenchTabKind::Exec => "Exec",
            WorkbenchTabKind::PortForward => "Port-Forward",
            WorkbenchTabKind::Relations => "Relations",
            WorkbenchTabKind::NetworkPolicy => "NetPol",
            WorkbenchTabKind::Connectivity => "Reach",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkbenchTabKey {
    ActionHistory,
    ResourceYaml(ResourceRef),
    ResourceDiff(ResourceRef),
    DecodedSecret(ResourceRef),
    ResourceEvents(ResourceRef),
    PodLogs(ResourceRef),
    WorkloadLogs(ResourceRef),
    Exec(ResourceRef),
    PortForward,
    Relations(ResourceRef),
    NetworkPolicy(ResourceRef),
    Connectivity(ResourceRef),
}

#[derive(Debug, Clone, Default)]
pub struct ActionHistoryTabState {
    pub selected: usize,
}

impl ActionHistoryTabState {
    pub fn select_next(&mut self, total: usize) {
        if total == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected + 1).min(total.saturating_sub(1));
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_top(&mut self) {
        self.selected = 0;
    }

    pub fn select_bottom(&mut self, total: usize) {
        self.selected = total.saturating_sub(1);
    }
}

#[derive(Debug, Clone)]
pub struct ResourceYamlTabState {
    pub resource: ResourceRef,
    pub pending_request_id: Option<u64>,
    pub yaml: Option<String>,
    pub scroll: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl ResourceYamlTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            pending_request_id: None,
            yaml: None,
            scroll: 0,
            loading: true,
            error: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResourceDiffTabState {
    pub resource: ResourceRef,
    pub pending_request_id: Option<u64>,
    pub baseline_kind: Option<ResourceDiffBaselineKind>,
    pub summary: Option<String>,
    pub lines: Vec<ResourceDiffLine>,
    pub scroll: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl ResourceDiffTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            pending_request_id: None,
            baseline_kind: None,
            summary: None,
            lines: Vec::new(),
            scroll: 0,
            loading: true,
            error: None,
        }
    }

    pub fn apply_result(&mut self, diff: ResourceDiffResult) {
        self.baseline_kind = Some(diff.baseline_kind);
        self.summary = Some(diff.summary);
        self.lines = diff.lines;
        self.scroll = 0;
        self.loading = false;
        self.error = None;
        self.pending_request_id = None;
    }

    pub fn set_error(&mut self, error: String) {
        self.baseline_kind = None;
        self.summary = None;
        self.lines.clear();
        self.scroll = 0;
        self.loading = false;
        self.error = Some(error);
        self.pending_request_id = None;
    }
}

#[derive(Debug, Clone)]
pub struct DecodedSecretTabState {
    pub resource: ResourceRef,
    pub pending_request_id: Option<u64>,
    pub source_yaml: Option<String>,
    pub entries: Vec<DecodedSecretEntry>,
    pub selected: usize,
    pub scroll: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub masked: bool,
    pub editing: bool,
    pub edit_input: String,
}

impl DecodedSecretTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            pending_request_id: None,
            source_yaml: None,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            loading: true,
            error: None,
            masked: true,
            editing: false,
            edit_input: String::new(),
        }
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.entries.iter().any(DecodedSecretEntry::is_dirty)
    }

    pub fn selected_entry(&self) -> Option<&DecodedSecretEntry> {
        self.entries.get(self.selected)
    }

    pub fn selected_entry_mut(&mut self) -> Option<&mut DecodedSecretEntry> {
        self.entries.get_mut(self.selected)
    }

    pub fn clamp_selected(&mut self) {
        let max = self.entries.len().saturating_sub(1);
        self.selected = self.selected.min(max);
        self.scroll = self.scroll.min(max);
    }
}

#[derive(Debug, Clone)]
pub struct ResourceEventsTabState {
    pub resource: ResourceRef,
    pub pending_request_id: Option<u64>,
    pub events: Vec<EventInfo>,
    pub timeline: Vec<TimelineEntry>,
    pub scroll: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl ResourceEventsTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            pending_request_id: None,
            events: Vec::new(),
            timeline: Vec::new(),
            scroll: 0,
            loading: true,
            error: None,
        }
    }

    /// Rebuild the merged timeline from current events + action history.
    pub fn rebuild_timeline(&mut self, history: &ActionHistoryState) {
        // Cap events to avoid unbounded growth from noisy controllers.
        if self.events.len() > MAX_TIMELINE_EVENTS {
            let drain = self.events.len() - MAX_TIMELINE_EVENTS;
            self.events.drain(..drain);
        }
        self.timeline = build_timeline(&self.events, history.entries(), &self.resource);
        // Always clamp scroll to the new timeline length.
        self.scroll = self.scroll.min(self.timeline.len().saturating_sub(1));
    }
}

#[derive(Debug, Clone)]
pub struct PodLogsTabState {
    pub resource: ResourceRef,
    pub viewer: LogsViewerState,
}

impl PodLogsTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            viewer: LogsViewerState::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadLogLine {
    pub pod_name: String,
    pub container_name: String,
    pub content: String,
    pub is_stderr: bool,
}

#[derive(Debug, Clone)]
pub struct WorkloadLogsTabState {
    pub resource: ResourceRef,
    pub session_id: u64,
    pub sources: Vec<(String, String, String)>,
    pub lines: Vec<WorkloadLogLine>,
    pub scroll: usize,
    pub follow_mode: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub text_filter: String,
    pub filter_input: String,
    pub editing_text_filter: bool,
    pub pod_filter: Option<String>,
    pub container_filter: Option<String>,
    pub available_pods: Vec<String>,
    pub available_containers: Vec<String>,
}

impl WorkloadLogsTabState {
    pub fn new(resource: ResourceRef, session_id: u64) -> Self {
        Self {
            resource,
            session_id,
            sources: Vec::new(),
            lines: Vec::new(),
            scroll: 0,
            follow_mode: true,
            loading: true,
            error: None,
            notice: None,
            text_filter: String::new(),
            filter_input: String::new(),
            editing_text_filter: false,
            pod_filter: None,
            container_filter: None,
            available_pods: Vec::new(),
            available_containers: Vec::new(),
        }
    }

    pub fn push_line(&mut self, line: WorkloadLogLine) {
        if !self.available_pods.iter().any(|pod| pod == &line.pod_name) {
            self.available_pods.push(line.pod_name.clone());
            self.available_pods.sort();
        }
        if !self
            .available_containers
            .iter()
            .any(|container| container == &line.container_name)
        {
            self.available_containers.push(line.container_name.clone());
            self.available_containers.sort();
        }

        self.lines.push(line);
        if self.lines.len() > MAX_WORKLOAD_LOG_LINES {
            let excess = self.lines.len() - MAX_WORKLOAD_LOG_LINES;
            self.lines.drain(..excess);
            self.scroll = self.scroll.saturating_sub(excess);
        }
        if self.follow_mode {
            // Set scroll past the end; the renderer's scroll_window clamps it
            // to the last visible filtered line. This avoids an O(n) filter
            // scan on every push.
            self.scroll = self.lines.len();
        }
    }

    pub fn cycle_pod_filter(&mut self) {
        self.pod_filter = cycle_filter_value(&self.available_pods, self.pod_filter.as_deref());
        self.scroll = 0;
    }

    pub fn cycle_container_filter(&mut self) {
        self.container_filter =
            cycle_filter_value(&self.available_containers, self.container_filter.as_deref());
        self.scroll = 0;
    }

    pub fn matches_filter(&self, line: &WorkloadLogLine) -> bool {
        self.pod_filter
            .as_ref()
            .is_none_or(|pod| pod == &line.pod_name)
            && self
                .container_filter
                .as_ref()
                .is_none_or(|container| container == &line.container_name)
            && (self.text_filter.is_empty() || contains_ci_ascii(&line.content, &self.text_filter))
    }
}

#[derive(Debug, Clone)]
pub struct ExecTabState {
    pub resource: ResourceRef,
    pub session_id: u64,
    pub pod_name: String,
    pub namespace: String,
    pub container_name: String,
    pub containers: Vec<String>,
    pub picking_container: bool,
    pub container_cursor: usize,
    pub input: String,
    pub lines: Vec<String>,
    pub scroll: usize,
    pub loading: bool,
    pub shell_name: Option<String>,
    pub error: Option<String>,
    pub exited: bool,
    pub pending_fragment: String,
}

impl ExecTabState {
    pub fn new(
        resource: ResourceRef,
        session_id: u64,
        pod_name: String,
        namespace: String,
    ) -> Self {
        Self {
            resource,
            session_id,
            pod_name,
            namespace,
            container_name: String::new(),
            containers: Vec::new(),
            picking_container: false,
            container_cursor: 0,
            input: String::new(),
            lines: Vec::new(),
            scroll: 0,
            loading: true,
            shell_name: None,
            error: None,
            exited: false,
            pending_fragment: String::new(),
        }
    }

    pub fn set_containers(&mut self, containers: Vec<String>) {
        self.containers = containers;
        self.container_cursor = 0;
        self.exited = false;
        self.error = None;
        if self.containers.is_empty() {
            self.picking_container = false;
            self.loading = false;
            self.error = Some("No containers found in this pod.".to_string());
        } else if self.containers.len() > 1 {
            self.picking_container = true;
            self.loading = false;
        } else if let Some(container) = self.containers.first() {
            self.container_name = container.clone();
            self.picking_container = false;
        }
    }

    pub fn preset_container(&mut self, container_name: impl Into<String>) {
        let container_name = container_name.into();
        self.container_name = container_name.clone();
        self.containers = vec![container_name];
        self.container_cursor = 0;
        self.picking_container = false;
        self.loading = true;
        self.exited = false;
        self.error = None;
    }

    /// Max pending fragment size before force-flushing (1 MB).
    const MAX_PENDING_FRAGMENT: usize = 1_048_576;

    pub fn append_output(&mut self, chunk: &str) {
        for segment in chunk.split_inclusive('\n') {
            if segment.ends_with('\n') {
                self.pending_fragment
                    .push_str(segment.trim_end_matches('\n'));
                self.lines.push(std::mem::take(&mut self.pending_fragment));
            } else {
                self.pending_fragment.push_str(segment);
            }
        }
        // Force-flush oversized fragments (e.g. binary output without newlines).
        if self.pending_fragment.len() > Self::MAX_PENDING_FRAGMENT {
            self.lines.push(std::mem::take(&mut self.pending_fragment));
        }
        if self.lines.len() > MAX_EXEC_OUTPUT_LINES {
            let excess = self.lines.len() - MAX_EXEC_OUTPUT_LINES;
            self.lines.drain(..excess);
            self.scroll = self.scroll.saturating_sub(excess);
        }
        // Auto-follow: keep scroll at bottom if user was already at (or near) the end.
        let max = self.lines.len().saturating_sub(1);
        if self.scroll >= max.saturating_sub(1) {
            self.scroll = max;
        }
    }
}

#[derive(Debug, Clone)]
pub struct PortForwardTabState {
    pub target: Option<ResourceRef>,
    pub dialog: PortForwardDialog,
}

impl PortForwardTabState {
    pub fn new(target: Option<ResourceRef>, dialog: PortForwardDialog) -> Self {
        Self { target, dialog }
    }
}

#[derive(Debug, Clone)]
pub struct RelationsTabState {
    pub resource: ResourceRef,
    pub pending_request_id: Option<u64>,
    pub tree: Vec<crate::k8s::relationships::RelationNode>,
    pub cursor: usize,
    pub expanded: std::collections::HashSet<usize>,
    pub loading: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NetworkPolicyTabState {
    pub resource: ResourceRef,
    pub summary_lines: Vec<String>,
    pub tree: Vec<crate::k8s::relationships::RelationNode>,
    pub cursor: usize,
    pub expanded: std::collections::HashSet<usize>,
    pub loading: bool,
    pub error: Option<String>,
}

impl NetworkPolicyTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            summary_lines: Vec::new(),
            tree: Vec::new(),
            cursor: 0,
            expanded: std::collections::HashSet::new(),
            loading: false,
            error: None,
        }
    }

    pub fn apply_analysis(&mut self, analysis: NetworkPolicyAnalysis) {
        let mut expanded = std::collections::HashSet::new();
        let mut counter = 0usize;
        for section in &analysis.tree {
            expanded.insert(counter);
            counter += 1;
            for child in &section.children {
                expanded.insert(counter);
                counter += 1;
                crate::k8s::relationships::count_descendants(&child.children, &mut counter);
            }
        }
        self.summary_lines = analysis.summary_lines;
        self.tree = analysis.tree;
        self.expanded = expanded;
        let flat = crate::k8s::relationships::flatten_tree(&self.tree, &self.expanded);
        if self.cursor >= flat.len() {
            self.cursor = flat.len().saturating_sub(1);
        }
        self.loading = false;
        self.error = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectivityTargetOption {
    pub resource: ResourceRef,
    pub display: String,
    pub status: String,
    pub pod_ip: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectivityTabFocus {
    Filter,
    Targets,
    Result,
}

#[derive(Debug, Clone)]
pub struct ConnectivityTabState {
    pub source: ResourceRef,
    pub focus: ConnectivityTabFocus,
    pub filter: InputFieldWidget,
    pub targets: Vec<ConnectivityTargetOption>,
    pub filtered_target_indices: Vec<usize>,
    pub selected_target: usize,
    pub current_target: Option<ResourceRef>,
    pub summary_lines: Vec<String>,
    pub tree: Vec<crate::k8s::relationships::RelationNode>,
    pub tree_cursor: usize,
    pub expanded: std::collections::HashSet<usize>,
    pub error: Option<String>,
}

impl ConnectivityTabState {
    pub fn new(source: ResourceRef, targets: Vec<ConnectivityTargetOption>) -> Self {
        let mut tab = Self {
            source,
            focus: ConnectivityTabFocus::Targets,
            filter: InputFieldWidget::new(64),
            targets,
            filtered_target_indices: Vec::new(),
            selected_target: 0,
            current_target: None,
            summary_lines: vec![
                "Select a target pod, then press [Enter] to evaluate whether any traffic is allowed by policy intent."
                    .to_string(),
                "Result shows intent only; CNI enforcement/runtime packet filters may still differ."
                    .to_string(),
            ],
            tree: Vec::new(),
            tree_cursor: 0,
            expanded: std::collections::HashSet::new(),
            error: None,
        };
        tab.refresh_filter();
        tab
    }

    pub fn refresh_filter(&mut self) {
        let query = self.filter.value.to_ascii_lowercase();
        self.filtered_target_indices = self
            .targets
            .iter()
            .enumerate()
            .filter_map(|(idx, target)| {
                (query.is_empty()
                    || target.display.to_ascii_lowercase().contains(&query)
                    || target.status.to_ascii_lowercase().contains(&query)
                    || target
                        .pod_ip
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains(&query))
                .then_some(idx)
            })
            .collect();
        let max = self.filtered_target_indices.len().saturating_sub(1);
        self.selected_target = self.selected_target.min(max);
    }

    pub fn selected_target_option(&self) -> Option<&ConnectivityTargetOption> {
        self.filtered_target_indices
            .get(self.selected_target)
            .and_then(|idx| self.targets.get(*idx))
    }

    pub fn apply_analysis(&mut self, target: ResourceRef, analysis: ConnectivityAnalysis) {
        self.current_target = Some(target);
        self.summary_lines = analysis.summary_lines;
        self.tree = analysis.tree;
        self.focus = ConnectivityTabFocus::Result;
        self.tree_cursor = 0;
        self.error = None;
        self.expanded.clear();

        let mut counter = 0usize;
        for section in &self.tree {
            self.expanded.insert(counter);
            counter += 1;
            for child in &section.children {
                self.expanded.insert(counter);
                counter += 1;
                crate::k8s::relationships::count_descendants(&child.children, &mut counter);
            }
        }
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
        self.current_target = None;
        self.summary_lines = vec![
            "Connectivity query could not be evaluated from the current snapshot.".to_string(),
        ];
        self.tree.clear();
        self.tree_cursor = 0;
        self.expanded.clear();
        self.focus = ConnectivityTabFocus::Targets;
    }
}

impl RelationsTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            pending_request_id: None,
            tree: Vec::new(),
            cursor: 0,
            expanded: std::collections::HashSet::new(),
            loading: true,
            error: None,
        }
    }

    /// Populate the tree and auto-expand section headers and their immediate
    /// children so the user sees a useful overview on first open.
    pub fn set_tree(&mut self, tree: Vec<crate::k8s::relationships::RelationNode>) {
        let mut expanded = std::collections::HashSet::new();
        let mut counter = 0usize;
        for section in &tree {
            expanded.insert(counter);
            counter += 1;
            for child in &section.children {
                expanded.insert(counter);
                counter += 1;
                crate::k8s::relationships::count_descendants(&child.children, &mut counter);
            }
        }
        self.expanded = expanded;
        self.tree = tree;
        // Clamp cursor so it stays valid if the new tree is smaller.
        let flat = crate::k8s::relationships::flatten_tree(&self.tree, &self.expanded);
        if self.cursor >= flat.len() {
            self.cursor = flat.len().saturating_sub(1);
        }
    }
}

#[derive(Debug, Clone)]
pub enum WorkbenchTabState {
    ActionHistory(ActionHistoryTabState),
    ResourceYaml(ResourceYamlTabState),
    ResourceDiff(ResourceDiffTabState),
    DecodedSecret(DecodedSecretTabState),
    ResourceEvents(ResourceEventsTabState),
    PodLogs(PodLogsTabState),
    WorkloadLogs(WorkloadLogsTabState),
    Exec(ExecTabState),
    PortForward(PortForwardTabState),
    Relations(RelationsTabState),
    NetworkPolicy(NetworkPolicyTabState),
    Connectivity(ConnectivityTabState),
}

impl WorkbenchTabState {
    pub const fn kind(&self) -> WorkbenchTabKind {
        match self {
            Self::ActionHistory(_) => WorkbenchTabKind::ActionHistory,
            Self::ResourceYaml(_) => WorkbenchTabKind::ResourceYaml,
            Self::ResourceDiff(_) => WorkbenchTabKind::ResourceDiff,
            Self::DecodedSecret(_) => WorkbenchTabKind::DecodedSecret,
            Self::ResourceEvents(_) => WorkbenchTabKind::ResourceEvents,
            Self::PodLogs(_) => WorkbenchTabKind::PodLogs,
            Self::WorkloadLogs(_) => WorkbenchTabKind::WorkloadLogs,
            Self::Exec(_) => WorkbenchTabKind::Exec,
            Self::PortForward(_) => WorkbenchTabKind::PortForward,
            Self::Relations(_) => WorkbenchTabKind::Relations,
            Self::NetworkPolicy(_) => WorkbenchTabKind::NetworkPolicy,
            Self::Connectivity(_) => WorkbenchTabKind::Connectivity,
        }
    }

    pub fn key(&self) -> WorkbenchTabKey {
        match self {
            Self::ActionHistory(_) => WorkbenchTabKey::ActionHistory,
            Self::ResourceYaml(tab) => WorkbenchTabKey::ResourceYaml(tab.resource.clone()),
            Self::ResourceDiff(tab) => WorkbenchTabKey::ResourceDiff(tab.resource.clone()),
            Self::DecodedSecret(tab) => WorkbenchTabKey::DecodedSecret(tab.resource.clone()),
            Self::ResourceEvents(tab) => WorkbenchTabKey::ResourceEvents(tab.resource.clone()),
            Self::PodLogs(tab) => WorkbenchTabKey::PodLogs(tab.resource.clone()),
            Self::WorkloadLogs(tab) => WorkbenchTabKey::WorkloadLogs(tab.resource.clone()),
            Self::Exec(tab) => WorkbenchTabKey::Exec(tab.resource.clone()),
            Self::PortForward(_) => WorkbenchTabKey::PortForward,
            Self::Relations(tab) => WorkbenchTabKey::Relations(tab.resource.clone()),
            Self::NetworkPolicy(tab) => WorkbenchTabKey::NetworkPolicy(tab.resource.clone()),
            Self::Connectivity(tab) => WorkbenchTabKey::Connectivity(tab.source.clone()),
        }
    }

    pub fn title(&self) -> String {
        let kind_label = self.kind().title();
        let icon = tab_icon(kind_label).active();
        match self {
            Self::ActionHistory(_) => format!("{icon}{kind_label}"),
            Self::ResourceYaml(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
            Self::ResourceDiff(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
            Self::DecodedSecret(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
            Self::ResourceEvents(tab) => {
                format!("{icon}Events {}", resource_title(&tab.resource))
            }
            Self::PodLogs(tab) => format!("{icon}Logs {}", resource_title(&tab.resource)),
            Self::WorkloadLogs(tab) => format!("{icon}Logs {}", resource_title(&tab.resource)),
            Self::Exec(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
            Self::PortForward(tab) => match &tab.target {
                Some(resource) => {
                    format!("{icon}{kind_label} {}", resource_title(resource))
                }
                None => format!("{icon}{kind_label} Sessions"),
            },
            Self::Relations(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
            Self::NetworkPolicy(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
            Self::Connectivity(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.source))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkbenchTab {
    pub id: u64,
    pub state: WorkbenchTabState,
}

impl WorkbenchTab {
    pub fn new(id: u64, state: WorkbenchTabState) -> Self {
        Self { id, state }
    }
}

#[derive(Debug, Clone)]
pub struct WorkbenchState {
    pub open: bool,
    pub height: u16,
    pub maximized: bool,
    pub active_tab: usize,
    pub tabs: Vec<WorkbenchTab>,
    next_tab_id: u64,
}

impl Default for WorkbenchState {
    fn default() -> Self {
        Self {
            open: false,
            height: DEFAULT_WORKBENCH_HEIGHT,
            maximized: false,
            active_tab: 0,
            tabs: Vec::new(),
            next_tab_id: 1,
        }
    }
}

impl WorkbenchState {
    pub fn open_tab(&mut self, state: WorkbenchTabState) -> usize {
        self.ensure_tab(state, true)
    }

    pub fn ensure_background_tab(&mut self, state: WorkbenchTabState) -> usize {
        self.ensure_tab(state, false)
    }

    fn ensure_tab(&mut self, state: WorkbenchTabState, focus: bool) -> usize {
        let key = state.key();
        let was_open = self.open;
        let previous_active = self.active_tab;
        if let Some(idx) = self.tabs.iter().position(|tab| tab.state.key() == key) {
            self.tabs[idx].state = state;
            if focus {
                self.active_tab = idx;
                self.open = true;
            } else {
                self.active_tab = previous_active.min(self.tabs.len().saturating_sub(1));
                self.open = was_open;
            }
            return idx;
        }

        let id = self.next_tab_id;
        self.next_tab_id = self.next_tab_id.saturating_add(1);
        self.tabs.push(WorkbenchTab::new(id, state));
        let idx = self.tabs.len().saturating_sub(1);
        if focus {
            self.active_tab = idx;
            self.open = true;
        } else {
            self.active_tab = previous_active.min(self.tabs.len().saturating_sub(1));
            self.open = was_open;
        }
        idx
    }

    pub fn toggle_open(&mut self) {
        if self.open {
            self.open = false;
            self.maximized = false;
        } else if !self.tabs.is_empty() {
            self.open = true;
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.maximized = false;
    }

    pub fn toggle_maximize(&mut self) {
        if !self.tabs.is_empty() {
            self.maximized = !self.maximized;
        }
    }

    pub fn close_active_tab(&mut self) {
        if self.tabs.is_empty() {
            self.open = false;
            self.maximized = false;
            self.active_tab = 0;
            return;
        }

        let idx = self.active_tab.min(self.tabs.len().saturating_sub(1));
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            self.open = false;
            self.maximized = false;
            self.active_tab = 0;
            return;
        }

        self.active_tab = idx.min(self.tabs.len().saturating_sub(1));
    }

    pub fn next_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        self.active_tab = (self.active_tab + 1) % self.tabs.len();
    }

    pub fn previous_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        self.active_tab = if self.active_tab == 0 {
            self.tabs.len() - 1
        } else {
            self.active_tab - 1
        };
    }

    pub fn resize_larger(&mut self) {
        self.height = self
            .height
            .saturating_add(1)
            .clamp(MIN_WORKBENCH_HEIGHT, MAX_WORKBENCH_HEIGHT);
    }

    pub fn resize_smaller(&mut self) {
        self.height = self
            .height
            .saturating_sub(1)
            .clamp(MIN_WORKBENCH_HEIGHT, MAX_WORKBENCH_HEIGHT);
    }

    pub fn active_tab(&self) -> Option<&WorkbenchTab> {
        self.tabs
            .get(self.active_tab.min(self.tabs.len().saturating_sub(1)))
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut WorkbenchTab> {
        let idx = self.active_tab.min(self.tabs.len().saturating_sub(1));
        self.tabs.get_mut(idx)
    }

    pub fn find_tab_mut(&mut self, key: &WorkbenchTabKey) -> Option<&mut WorkbenchTab> {
        self.tabs.iter_mut().find(|tab| tab.state.key() == *key)
    }

    pub fn exec_session_id(&self, resource: &ResourceRef) -> Option<u64> {
        self.tabs.iter().find_map(|tab| match &tab.state {
            WorkbenchTabState::Exec(exec_tab) if &exec_tab.resource == resource => {
                Some(exec_tab.session_id)
            }
            _ => None,
        })
    }

    pub fn has_tab(&self, key: &WorkbenchTabKey) -> bool {
        self.tabs.iter().any(|tab| tab.state.key() == *key)
    }

    /// Remove all resource-bound tabs (YAML, Drift, Events, Logs, Exec) that become
    /// stale after a context or namespace switch.  ActionHistory and PortForward
    /// are retained since they are not resource-scoped.
    pub fn close_resource_tabs(&mut self) {
        self.tabs.retain(|tab| {
            matches!(
                tab.state.kind(),
                WorkbenchTabKind::ActionHistory | WorkbenchTabKind::PortForward
            )
        });
        if self.tabs.is_empty() {
            self.open = false;
            self.maximized = false;
            self.active_tab = 0;
        } else {
            self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
        }
    }

    pub fn activate_tab(&mut self, key: &WorkbenchTabKey) -> bool {
        if let Some(idx) = self.tabs.iter().position(|tab| tab.state.key() == *key) {
            self.active_tab = idx;
            self.open = true;
            return true;
        }
        false
    }

    pub fn set_open_and_height(&mut self, open: bool, height: u16) {
        self.height = height.clamp(MIN_WORKBENCH_HEIGHT, MAX_WORKBENCH_HEIGHT);
        self.open = open;
    }
}

fn resource_title(resource: &ResourceRef) -> String {
    match resource.namespace() {
        Some(namespace) => format!("{}/{}", namespace, resource.name()),
        None => resource.name().to_string(),
    }
}

fn cycle_filter_value(values: &[String], current: Option<&str>) -> Option<String> {
    if values.is_empty() {
        return None;
    }
    match current {
        None => values.first().cloned(),
        Some(current) => values
            .iter()
            .position(|value| value == current)
            .and_then(|idx| values.get(idx + 1))
            .cloned(),
        // Wrap back to "All"
    }
}

#[inline]
fn contains_ci_ascii(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(name: &str) -> ResourceRef {
        ResourceRef::Pod(name.to_string(), "ns".to_string())
    }

    #[test]
    fn default_workbench_starts_closed_and_empty() {
        let state = WorkbenchState::default();
        assert!(!state.open);
        assert!(state.tabs.is_empty());
        assert_eq!(state.active_tab, 0);
    }

    #[test]
    fn open_tab_reuses_existing_resource_tab() {
        let mut state = WorkbenchState::default();
        let first = state.open_tab(WorkbenchTabState::ResourceYaml(ResourceYamlTabState::new(
            pod("pod-0"),
        )));
        let second = state.open_tab(WorkbenchTabState::ResourceYaml(ResourceYamlTabState::new(
            pod("pod-0"),
        )));

        assert_eq!(first, second);
        assert_eq!(state.tabs.len(), 1);
        assert!(state.open);
    }

    #[test]
    fn resource_diff_tab_deduplicates_by_resource() {
        let mut state = WorkbenchState::default();
        let first = state.open_tab(WorkbenchTabState::ResourceDiff(ResourceDiffTabState::new(
            pod("pod-0"),
        )));
        let second = state.open_tab(WorkbenchTabState::ResourceDiff(ResourceDiffTabState::new(
            pod("pod-0"),
        )));

        assert_eq!(first, second);
        assert_eq!(state.tabs.len(), 1);
        assert!(state.open);
    }

    #[test]
    fn network_policy_tab_deduplicates_by_resource() {
        let mut state = WorkbenchState::default();
        let first = state.open_tab(WorkbenchTabState::NetworkPolicy(
            NetworkPolicyTabState::new(pod("pod-0")),
        ));
        let second = state.open_tab(WorkbenchTabState::NetworkPolicy(
            NetworkPolicyTabState::new(pod("pod-0")),
        ));

        assert_eq!(first, second);
        assert_eq!(state.tabs.len(), 1);
        assert!(state.open);
    }

    #[test]
    fn connectivity_tab_deduplicates_by_source_resource() {
        let mut state = WorkbenchState::default();
        let first = state.open_tab(WorkbenchTabState::Connectivity(ConnectivityTabState::new(
            pod("pod-0"),
            Vec::new(),
        )));
        let second = state.open_tab(WorkbenchTabState::Connectivity(ConnectivityTabState::new(
            pod("pod-0"),
            Vec::new(),
        )));

        assert_eq!(first, second);
        assert_eq!(state.tabs.len(), 1);
        assert!(state.open);
    }

    #[test]
    fn tab_navigation_wraps() {
        let mut state = WorkbenchState::default();
        state.open_tab(WorkbenchTabState::ActionHistory(
            ActionHistoryTabState::default(),
        ));
        state.open_tab(WorkbenchTabState::ResourceYaml(ResourceYamlTabState::new(
            pod("pod-0"),
        )));
        state.open_tab(WorkbenchTabState::ResourceEvents(
            ResourceEventsTabState::new(pod("pod-0")),
        ));

        state.next_tab();
        assert_eq!(
            state.active_tab().map(|tab| tab.state.kind()),
            Some(WorkbenchTabKind::ActionHistory)
        );

        state.previous_tab();
        assert_eq!(
            state.active_tab().map(|tab| tab.state.kind()),
            Some(WorkbenchTabKind::ResourceEvents)
        );
    }

    #[test]
    fn close_active_tab_closes_workbench_when_last_tab_removed() {
        let mut state = WorkbenchState::default();
        state.open_tab(WorkbenchTabState::ResourceYaml(ResourceYamlTabState::new(
            pod("pod-0"),
        )));

        state.close_active_tab();

        assert!(!state.open);
        assert!(state.tabs.is_empty());
    }

    #[test]
    fn resize_clamps_to_supported_range() {
        let mut state = WorkbenchState::default();
        state.height = MIN_WORKBENCH_HEIGHT;
        state.resize_smaller();
        assert_eq!(state.height, MIN_WORKBENCH_HEIGHT);

        state.height = MAX_WORKBENCH_HEIGHT;
        state.resize_larger();
        assert_eq!(state.height, MAX_WORKBENCH_HEIGHT);
    }

    #[test]
    fn background_tab_preserves_focus_and_open_state() {
        let mut state = WorkbenchState::default();
        state.open_tab(WorkbenchTabState::ResourceYaml(ResourceYamlTabState::new(
            pod("pod-0"),
        )));

        state.ensure_background_tab(WorkbenchTabState::ActionHistory(
            ActionHistoryTabState::default(),
        ));

        assert_eq!(
            state.active_tab().map(|tab| tab.state.kind()),
            Some(WorkbenchTabKind::ResourceYaml)
        );
        assert!(state.open);
        assert_eq!(state.tabs.len(), 2);
    }

    #[test]
    fn toggle_maximize() {
        let mut state = WorkbenchState::default();
        state.open_tab(WorkbenchTabState::ActionHistory(
            ActionHistoryTabState::default(),
        ));
        assert!(!state.maximized);
        state.toggle_maximize();
        assert!(state.maximized);
        state.toggle_maximize();
        assert!(!state.maximized);
    }

    #[test]
    fn close_clears_maximized() {
        let mut state = WorkbenchState::default();
        state.maximized = true;
        state.close();
        assert!(!state.maximized);
    }

    #[test]
    fn toggle_open_clears_maximized_on_close() {
        let mut state = WorkbenchState::default();
        state.open = true;
        state.maximized = true;
        state.toggle_open();
        assert!(!state.open);
        assert!(!state.maximized);
    }

    #[test]
    fn toggle_open_does_not_open_empty_workbench() {
        let mut state = WorkbenchState::default();
        assert!(!state.open);
        state.toggle_open();
        assert!(!state.open);
    }

    #[test]
    fn toggle_maximize_does_nothing_when_empty() {
        let mut state = WorkbenchState::default();
        state.toggle_maximize();
        assert!(!state.maximized);
    }

    #[test]
    fn relations_tab_deduplicates_by_resource() {
        let mut state = WorkbenchState::default();
        let first = state.open_tab(WorkbenchTabState::Relations(RelationsTabState::new(pod(
            "pod-0",
        ))));
        let second = state.open_tab(WorkbenchTabState::Relations(RelationsTabState::new(pod(
            "pod-0",
        ))));
        assert_eq!(first, second);
        assert_eq!(state.tabs.len(), 1);
    }

    #[test]
    fn close_active_tab_clears_maximized() {
        let mut state = WorkbenchState::default();
        state.open_tab(WorkbenchTabState::ActionHistory(
            ActionHistoryTabState::default(),
        ));
        state.maximized = true;
        state.close_active_tab();
        assert!(!state.maximized);
        assert!(!state.open);
    }

    #[test]
    fn close_resource_tabs_clears_maximized() {
        let mut state = WorkbenchState::default();
        state.open_tab(WorkbenchTabState::ResourceYaml(ResourceYamlTabState {
            resource: pod("pod-0"),
            yaml: None,
            scroll: 0,
            pending_request_id: None,
            loading: false,
            error: None,
        }));
        state.maximized = true;
        state.close_resource_tabs();
        assert!(!state.maximized);
        assert!(!state.open);
    }

    #[test]
    fn exec_set_containers_clears_exited_and_error() {
        let mut tab = ExecTabState::new(pod("pod-0"), 1, "pod-0".into(), "default".into());
        tab.exited = true;
        tab.error = Some("connection lost".into());
        tab.set_containers(vec!["main".into()]);
        assert!(!tab.exited);
        assert!(tab.error.is_none());
        assert_eq!(tab.container_name, "main");
    }

    #[test]
    fn exec_append_output_follows_when_at_bottom() {
        let mut tab = ExecTabState::new(pod("pod-0"), 1, "pod-0".into(), "default".into());
        tab.append_output("line1\nline2\n");
        assert_eq!(tab.scroll, 1); // at bottom (2 lines, index 1)
        tab.append_output("line3\n");
        assert_eq!(tab.scroll, 2); // followed to new bottom
    }

    #[test]
    fn exec_append_output_does_not_follow_when_scrolled_up() {
        let mut tab = ExecTabState::new(pod("pod-0"), 1, "pod-0".into(), "default".into());
        tab.append_output("line1\nline2\nline3\n");
        tab.scroll = 0; // user scrolled to top
        tab.append_output("line4\n");
        assert_eq!(tab.scroll, 0); // stays at top
    }

    #[test]
    fn exec_session_id_returns_matching_tab_session() {
        let mut state = WorkbenchState::default();
        state.open_tab(WorkbenchTabState::Exec(ExecTabState::new(
            pod("pod-0"),
            41,
            "pod-0".into(),
            "ns".into(),
        )));

        assert_eq!(state.exec_session_id(&pod("pod-0")), Some(41));
        assert_eq!(state.exec_session_id(&pod("pod-1")), None);
    }

    #[test]
    fn workload_log_follow_mode_sets_scroll_past_end() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.follow_mode = true;
        tab.push_line(WorkloadLogLine {
            pod_name: "pod-0".into(),
            container_name: "main".into(),
            content: "hello".into(),
            is_stderr: false,
        });
        // follow mode sets scroll past end for renderer to clamp
        assert!(tab.scroll >= tab.lines.len());
    }

    #[test]
    fn events_timeline_rebuild_clamps_scroll() {
        let mut tab = ResourceEventsTabState::new(pod("pod-0"));
        tab.scroll = 100;
        tab.rebuild_timeline(&crate::action_history::ActionHistoryState::default());
        // timeline is empty, scroll should be 0
        assert_eq!(tab.scroll, 0);
    }

    #[test]
    fn workload_log_filter_matches_case_insensitively() {
        let tab = WorkloadLogsTabState {
            text_filter: "error".to_string(),
            ..WorkloadLogsTabState::new(pod("pod-0"), 1)
        };

        assert!(tab.matches_filter(&WorkloadLogLine {
            pod_name: "pod-0".to_string(),
            container_name: "main".to_string(),
            content: "ERROR: probe failed".to_string(),
            is_stderr: true,
        }));
    }
}
