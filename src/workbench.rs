//! Canonical workbench state for long-lived bottom-pane surfaces.

use crate::{
    app::{LogsViewerState, ResourceRef},
    k8s::client::EventInfo,
    ui::components::port_forward_dialog::PortForwardDialog,
};

pub const DEFAULT_WORKBENCH_HEIGHT: u16 = 12;
pub const MIN_WORKBENCH_HEIGHT: u16 = 8;
pub const MAX_WORKBENCH_HEIGHT: u16 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkbenchTabKind {
    ResourceYaml,
    ResourceEvents,
    PodLogs,
    PortForward,
}

impl WorkbenchTabKind {
    pub const fn title(self) -> &'static str {
        match self {
            WorkbenchTabKind::ResourceYaml => "YAML",
            WorkbenchTabKind::ResourceEvents => "Events",
            WorkbenchTabKind::PodLogs => "Logs",
            WorkbenchTabKind::PortForward => "Port-Forward",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkbenchTabKey {
    ResourceYaml(ResourceRef),
    ResourceEvents(ResourceRef),
    PodLogs(ResourceRef),
    PortForward,
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
pub enum WorkbenchTabState {
    ResourceYaml(ResourceYamlTabState),
    ResourceEvents(ResourceEventsTabState),
    PodLogs(PodLogsTabState),
    PortForward(PortForwardTabState),
}

impl WorkbenchTabState {
    pub const fn kind(&self) -> WorkbenchTabKind {
        match self {
            Self::ResourceYaml(_) => WorkbenchTabKind::ResourceYaml,
            Self::ResourceEvents(_) => WorkbenchTabKind::ResourceEvents,
            Self::PodLogs(_) => WorkbenchTabKind::PodLogs,
            Self::PortForward(_) => WorkbenchTabKind::PortForward,
        }
    }

    pub fn key(&self) -> WorkbenchTabKey {
        match self {
            Self::ResourceYaml(tab) => WorkbenchTabKey::ResourceYaml(tab.resource.clone()),
            Self::ResourceEvents(tab) => WorkbenchTabKey::ResourceEvents(tab.resource.clone()),
            Self::PodLogs(tab) => WorkbenchTabKey::PodLogs(tab.resource.clone()),
            Self::PortForward(_) => WorkbenchTabKey::PortForward,
        }
    }

    pub fn title(&self) -> String {
        match self {
            Self::ResourceYaml(tab) => format!("YAML {}", resource_title(&tab.resource)),
            Self::ResourceEvents(tab) => format!("Events {}", resource_title(&tab.resource)),
            Self::PodLogs(tab) => format!("Logs {}", resource_title(&tab.resource)),
            Self::PortForward(tab) => match &tab.target {
                Some(resource) => format!("Port-Forward {}", resource_title(resource)),
                None => "Port-Forward Sessions".to_string(),
            },
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
    pub active_tab: usize,
    pub tabs: Vec<WorkbenchTab>,
    next_tab_id: u64,
}

impl Default for WorkbenchState {
    fn default() -> Self {
        Self {
            open: false,
            height: DEFAULT_WORKBENCH_HEIGHT,
            active_tab: 0,
            tabs: Vec::new(),
            next_tab_id: 1,
        }
    }
}

impl WorkbenchState {
    pub fn open_tab(&mut self, state: WorkbenchTabState) -> usize {
        let key = state.key();
        if let Some(idx) = self.tabs.iter().position(|tab| tab.state.key() == key) {
            self.tabs[idx].state = state;
            self.active_tab = idx;
            self.open = true;
            return idx;
        }

        let id = self.next_tab_id;
        self.next_tab_id = self.next_tab_id.saturating_add(1);
        self.tabs.push(WorkbenchTab::new(id, state));
        self.active_tab = self.tabs.len().saturating_sub(1);
        self.open = true;
        self.active_tab
    }

    pub fn toggle_open(&mut self) {
        self.open = !self.open;
    }

    pub fn close(&mut self) {
        self.open = false;
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
        state.open_tab(WorkbenchTabState::ResourceYaml(ResourceYamlTabState::new(
            pod("pod-0"),
        )));
        state.open_tab(WorkbenchTabState::ResourceEvents(
            ResourceEventsTabState::new(pod("pod-0")),
        ));

        state.next_tab();
        assert_eq!(
            state.active_tab().map(|tab| tab.state.kind()),
            Some(WorkbenchTabKind::ResourceYaml)
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
}
