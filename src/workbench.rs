//! Canonical workbench state for long-lived bottom-pane surfaces.

use crate::{
    app::{LogsViewerState, ResourceRef},
    k8s::client::EventInfo,
    ui::components::port_forward_dialog::PortForwardDialog,
};

pub const DEFAULT_WORKBENCH_HEIGHT: u16 = 12;
pub const MIN_WORKBENCH_HEIGHT: u16 = 8;
pub const MAX_WORKBENCH_HEIGHT: u16 = 40;
pub const MAX_WORKLOAD_LOG_LINES: usize = 5_000;
pub const MAX_EXEC_OUTPUT_LINES: usize = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkbenchTabKind {
    ActionHistory,
    ResourceYaml,
    ResourceEvents,
    PodLogs,
    WorkloadLogs,
    Exec,
    PortForward,
    Relations,
}

impl WorkbenchTabKind {
    pub const fn title(self) -> &'static str {
        match self {
            WorkbenchTabKind::ActionHistory => "History",
            WorkbenchTabKind::ResourceYaml => "YAML",
            WorkbenchTabKind::ResourceEvents => "Events",
            WorkbenchTabKind::PodLogs => "Logs",
            WorkbenchTabKind::WorkloadLogs => "Workload Logs",
            WorkbenchTabKind::Exec => "Exec",
            WorkbenchTabKind::PortForward => "Port-Forward",
            WorkbenchTabKind::Relations => "Relations",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkbenchTabKey {
    ActionHistory,
    ResourceYaml(ResourceRef),
    ResourceEvents(ResourceRef),
    PodLogs(ResourceRef),
    WorkloadLogs(ResourceRef),
    Exec(ResourceRef),
    PortForward,
    Relations(ResourceRef),
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
    pub yaml: Option<String>,
    pub scroll: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl ResourceYamlTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            yaml: None,
            scroll: 0,
            loading: true,
            error: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResourceEventsTabState {
    pub resource: ResourceRef,
    pub events: Vec<EventInfo>,
    pub scroll: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl ResourceEventsTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            events: Vec::new(),
            scroll: 0,
            loading: true,
            error: None,
        }
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
            self.scroll = self.lines.len().saturating_sub(1);
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
            && (self.text_filter.is_empty()
                || line
                    .content
                    .to_ascii_lowercase()
                    .contains(&self.text_filter.to_ascii_lowercase()))
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
        if self.containers.len() > 1 {
            self.picking_container = true;
            self.loading = false;
        } else if let Some(container) = self.containers.first() {
            self.container_name = container.clone();
            self.picking_container = false;
        }
    }

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
        if self.lines.len() > MAX_EXEC_OUTPUT_LINES {
            let excess = self.lines.len() - MAX_EXEC_OUTPUT_LINES;
            self.lines.drain(..excess);
            self.scroll = self.scroll.saturating_sub(excess);
        }
        self.scroll = self.lines.len().saturating_sub(1);
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
    pub tree: Vec<crate::k8s::relationships::RelationNode>,
    pub cursor: usize,
    pub expanded: std::collections::HashSet<usize>,
    pub loading: bool,
    pub error: Option<String>,
}

impl RelationsTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
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
    ResourceEvents(ResourceEventsTabState),
    PodLogs(PodLogsTabState),
    WorkloadLogs(WorkloadLogsTabState),
    Exec(ExecTabState),
    PortForward(PortForwardTabState),
    Relations(RelationsTabState),
}

impl WorkbenchTabState {
    pub const fn kind(&self) -> WorkbenchTabKind {
        match self {
            Self::ActionHistory(_) => WorkbenchTabKind::ActionHistory,
            Self::ResourceYaml(_) => WorkbenchTabKind::ResourceYaml,
            Self::ResourceEvents(_) => WorkbenchTabKind::ResourceEvents,
            Self::PodLogs(_) => WorkbenchTabKind::PodLogs,
            Self::WorkloadLogs(_) => WorkbenchTabKind::WorkloadLogs,
            Self::Exec(_) => WorkbenchTabKind::Exec,
            Self::PortForward(_) => WorkbenchTabKind::PortForward,
            Self::Relations(_) => WorkbenchTabKind::Relations,
        }
    }

    pub fn key(&self) -> WorkbenchTabKey {
        match self {
            Self::ActionHistory(_) => WorkbenchTabKey::ActionHistory,
            Self::ResourceYaml(tab) => WorkbenchTabKey::ResourceYaml(tab.resource.clone()),
            Self::ResourceEvents(tab) => WorkbenchTabKey::ResourceEvents(tab.resource.clone()),
            Self::PodLogs(tab) => WorkbenchTabKey::PodLogs(tab.resource.clone()),
            Self::WorkloadLogs(tab) => WorkbenchTabKey::WorkloadLogs(tab.resource.clone()),
            Self::Exec(tab) => WorkbenchTabKey::Exec(tab.resource.clone()),
            Self::PortForward(_) => WorkbenchTabKey::PortForward,
            Self::Relations(tab) => WorkbenchTabKey::Relations(tab.resource.clone()),
        }
    }

    pub fn title(&self) -> String {
        match self {
            Self::ActionHistory(_) => "History".to_string(),
            Self::ResourceYaml(tab) => format!("YAML {}", resource_title(&tab.resource)),
            Self::ResourceEvents(tab) => format!("Events {}", resource_title(&tab.resource)),
            Self::PodLogs(tab) => format!("Logs {}", resource_title(&tab.resource)),
            Self::WorkloadLogs(tab) => format!("Logs {}", resource_title(&tab.resource)),
            Self::Exec(tab) => format!("Exec {}", resource_title(&tab.resource)),
            Self::PortForward(tab) => match &tab.target {
                Some(resource) => format!("Port-Forward {}", resource_title(resource)),
                None => "Port-Forward Sessions".to_string(),
            },
            Self::Relations(tab) => format!("Relations {}", resource_title(&tab.resource)),
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
        self.open = !self.open;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.maximized = false;
    }

    pub fn toggle_maximize(&mut self) {
        self.maximized = !self.maximized;
    }

    pub fn close_active_tab(&mut self) {
        if self.tabs.is_empty() {
            self.open = false;
            self.active_tab = 0;
            return;
        }

        let idx = self.active_tab.min(self.tabs.len().saturating_sub(1));
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            self.open = false;
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

    pub fn has_tab(&self, key: &WorkbenchTabKey) -> bool {
        self.tabs.iter().any(|tab| tab.state.key() == *key)
    }

    /// Remove all resource-bound tabs (YAML, Events, Logs, Exec) that become
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
}
