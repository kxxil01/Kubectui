//! Application state machine and keyboard input handling.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::k8s::client::EventInfo;

/// Top-level views displayed by KubecTUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppView {
    Dashboard,
    Nodes,
    Pods,
    Services,
    Deployments,
}

impl AppView {
    const ORDER: [AppView; 5] = [
        AppView::Dashboard,
        AppView::Nodes,
        AppView::Pods,
        AppView::Services,
        AppView::Deployments,
    ];

    /// Returns a static display label for this view.
    pub const fn label(self) -> &'static str {
        match self {
            AppView::Dashboard => "Dashboard",
            AppView::Nodes => "Nodes",
            AppView::Pods => "Pods",
            AppView::Services => "Services",
            AppView::Deployments => "Deployments",
        }
    }

    fn index(self) -> usize {
        Self::ORDER
            .iter()
            .position(|view| *view == self)
            .expect("AppView::ORDER must contain all enum variants")
    }

    fn from_index(index: usize) -> Self {
        Self::ORDER[index % Self::ORDER.len()]
    }

    fn next(self) -> Self {
        Self::from_index(self.index() + 1)
    }

    fn previous(self) -> Self {
        let current = self.index();
        let next_idx = if current == 0 {
            Self::ORDER.len() - 1
        } else {
            current - 1
        };
        Self::from_index(next_idx)
    }

    /// Enumerates all available top-level tabs in stable order.
    pub const fn tabs() -> &'static [AppView; 5] {
        &Self::ORDER
    }
}

/// Logical pointer to a resource selected in the current view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceRef {
    Node(String),
    Pod(String, String),
    Service(String, String),
    Deployment(String, String),
}

impl ResourceRef {
    /// Returns resource kind label used by UI and fetch routing.
    pub fn kind(&self) -> &'static str {
        match self {
            ResourceRef::Node(_) => "Node",
            ResourceRef::Pod(_, _) => "Pod",
            ResourceRef::Service(_, _) => "Service",
            ResourceRef::Deployment(_, _) => "Deployment",
        }
    }

    /// Returns resource name.
    pub fn name(&self) -> &str {
        match self {
            ResourceRef::Node(name)
            | ResourceRef::Pod(name, _)
            | ResourceRef::Service(name, _)
            | ResourceRef::Deployment(name, _) => name,
        }
    }

    /// Returns namespace when this is a namespaced resource.
    pub fn namespace(&self) -> Option<&str> {
        match self {
            ResourceRef::Node(_) => None,
            ResourceRef::Pod(_, ns)
            | ResourceRef::Service(_, ns)
            | ResourceRef::Deployment(_, ns) => Some(ns),
        }
    }
}

/// Human-readable metadata displayed in the detail modal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetailMetadata {
    pub name: String,
    pub namespace: Option<String>,
    pub status: Option<String>,
    pub node: Option<String>,
    pub ip: Option<String>,
    pub created: Option<String>,
    pub labels: Vec<(String, String)>,
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

/// In-detail logs viewer state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogsViewerState {
    pub scroll_offset: usize,
    pub follow_mode: bool,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaleDialogInputState {
    pub replica_input: String,
    pub target_replicas: i32,
}

impl Default for ScaleDialogInputState {
    fn default() -> Self {
        Self {
            replica_input: String::new(),
            target_replicas: 0,
        }
    }
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
    pub metadata: DetailMetadata,
    pub yaml: Option<String>,
    pub events: Vec<EventInfo>,
    pub sections: Vec<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub logs_viewer: Option<LogsViewerState>,
    pub port_forward_dialog: Option<PortForwardDialogState>,
    pub scale_dialog: Option<ScaleDialogInputState>,
    pub probe_panel: Option<ProbePanelState>,
}

/// Actions emitted by input handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    None,
    RefreshData,
    Quit,
    OpenDetail(ResourceRef),
    CloseDetail,
    EscapePressed,
    LogsViewerOpen,
    LogsViewerClose,
    LogsViewerScrollUp,
    LogsViewerScrollDown,
    LogsViewerToggleFollow,
    PortForwardOpen,
    PortForwardClose,
    PortForwardNextField,
    PortForwardPrevField,
    PortForwardUpdateLocalPort(String),
    PortForwardUpdateRemotePort(String),
    PortForwardBackspace,
    ScaleDialogOpen,
    ScaleDialogClose,
    ScaleDialogUpdateInput(char),
    ScaleDialogBackspace,
    ProbePanelOpen,
    ProbePanelClose,
    ProbeToggleExpand(usize),
    ProbeSelectNext,
    ProbeSelectPrev,
}

/// Runtime state for UI interaction and navigation.
#[derive(Debug, Clone)]
pub struct AppState {
    pub view: AppView,
    pub selected_idx: usize,
    pub search_query: String,
    pub is_search_mode: bool,
    pub should_quit: bool,
    pub error_message: Option<String>,
    pub detail_view: Option<DetailViewState>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            view: AppView::Dashboard,
            selected_idx: 0,
            search_query: String::new(),
            is_search_mode: false,
            should_quit: false,
            error_message: None,
            detail_view: None,
        }
    }
}

impl AppState {
    /// Returns the active top-level view.
    pub fn view(&self) -> AppView {
        self.view
    }

    /// Returns the currently selected list index.
    pub fn selected_idx(&self) -> usize {
        self.selected_idx
    }

    /// Returns the active search query.
    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    /// Returns whether the app is currently in search input mode.
    pub fn is_search_mode(&self) -> bool {
        self.is_search_mode
    }

    /// Returns whether the event loop should terminate.
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Returns the latest UI-level error, if any.
    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    /// Sets an error message to be shown in the status bar.
    pub fn set_error(&mut self, message: String) {
        self.error_message = Some(message);
    }

    /// Clears any active error message.
    pub fn clear_error(&mut self) {
        self.error_message = None;
    }

    fn next_view(&mut self) {
        self.view = self.view.next();
        self.selected_idx = 0;
    }

    fn previous_view(&mut self) {
        self.view = self.view.previous();
        self.selected_idx = 0;
    }

    fn select_next(&mut self) {
        self.selected_idx = self.selected_idx.saturating_add(1);
    }

    fn select_previous(&mut self) {
        self.selected_idx = self.selected_idx.saturating_sub(1);
    }

    /// Returns which detail sub-component is currently active.
    pub fn active_component(&self) -> ActiveComponent {
        let Some(detail) = &self.detail_view else {
            return ActiveComponent::None;
        };

        if detail.logs_viewer.is_some() {
            ActiveComponent::LogsViewer
        } else if detail.port_forward_dialog.is_some() {
            ActiveComponent::PortForward
        } else if detail.scale_dialog.is_some() {
            ActiveComponent::Scale
        } else if detail.probe_panel.is_some() {
            ActiveComponent::ProbePanel
        } else {
            ActiveComponent::None
        }
    }

    pub fn open_logs_viewer(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.logs_viewer = Some(LogsViewerState::default());
        }
    }

    pub fn close_logs_viewer(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.logs_viewer = None;
        }
    }

    pub fn open_port_forward(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.port_forward_dialog = Some(PortForwardDialogState::default());
        }
    }

    pub fn close_port_forward(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.port_forward_dialog = None;
        }
    }

    pub fn open_scale_dialog(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.scale_dialog = Some(ScaleDialogInputState::default());
        }
    }

    pub fn close_scale_dialog(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.scale_dialog = None;
        }
    }

    pub fn open_probe_panel(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.probe_panel = Some(ProbePanelState::default());
        }
    }

    pub fn close_probe_panel(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.probe_panel = None;
        }
    }

    /// Handles a keyboard event and updates app state.
    pub fn handle_key_event(&mut self, key: KeyEvent) -> AppAction {
        if self.is_search_mode {
            return self.handle_search_input(key);
        }

        // Component-level routing priority:
        // LogsViewer > PortForward > Scale > ProbePanel > DetailView > MainView
        match self.active_component() {
            ActiveComponent::LogsViewer => {
                return match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Char('k') | KeyCode::Up => AppAction::LogsViewerScrollUp,
                    KeyCode::Char('j') | KeyCode::Down => AppAction::LogsViewerScrollDown,
                    KeyCode::Char('f') => AppAction::LogsViewerToggleFollow,
                    _ => AppAction::None,
                };
            }
            ActiveComponent::PortForward => {
                return match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Tab => AppAction::PortForwardNextField,
                    KeyCode::BackTab => AppAction::PortForwardPrevField,
                    KeyCode::Backspace => AppAction::PortForwardBackspace,
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        let field = self
                            .detail_view
                            .as_ref()
                            .and_then(|d| d.port_forward_dialog.as_ref())
                            .map(|pf| pf.active_field)
                            .unwrap_or(PortForwardField::LocalPort);

                        match field {
                            PortForwardField::LocalPort => AppAction::PortForwardUpdateLocalPort(c.to_string()),
                            PortForwardField::RemotePort => {
                                AppAction::PortForwardUpdateRemotePort(c.to_string())
                            }
                            PortForwardField::TunnelList => AppAction::None,
                        }
                    }
                    _ => AppAction::None,
                };
            }
            ActiveComponent::Scale => {
                return match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Backspace => AppAction::ScaleDialogBackspace,
                    KeyCode::Char(c) if c.is_ascii_digit() => AppAction::ScaleDialogUpdateInput(c),
                    _ => AppAction::None,
                };
            }
            ActiveComponent::ProbePanel => {
                return match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Char(' ') => {
                        let idx = self
                            .detail_view
                            .as_ref()
                            .and_then(|d| d.probe_panel.as_ref())
                            .map(|p| p.selected_idx)
                            .unwrap_or(0);
                        AppAction::ProbeToggleExpand(idx)
                    }
                    KeyCode::Char('j') | KeyCode::Down => AppAction::ProbeSelectNext,
                    KeyCode::Char('k') | KeyCode::Up => AppAction::ProbeSelectPrev,
                    _ => AppAction::None,
                };
            }
            ActiveComponent::None => {}
        }

        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                AppAction::Quit
            }
            KeyCode::Esc if self.detail_view.is_some() => AppAction::CloseDetail,
            KeyCode::Esc => {
                self.should_quit = true;
                AppAction::Quit
            }
            KeyCode::Char('l') | KeyCode::Char('L') if self.detail_view.is_some() => {
                AppAction::LogsViewerOpen
            }
            KeyCode::Char('f') if self.detail_view.is_some() => AppAction::PortForwardOpen,
            KeyCode::Char('s') if self.detail_view.is_some() => AppAction::ScaleDialogOpen,
            KeyCode::Tab => {
                self.next_view();
                AppAction::None
            }
            KeyCode::BackTab => {
                self.previous_view();
                AppAction::None
            }
            KeyCode::Down => {
                self.select_next();
                AppAction::None
            }
            KeyCode::Up => {
                self.select_previous();
                AppAction::None
            }
            KeyCode::Char('/') => {
                self.is_search_mode = true;
                AppAction::None
            }
            KeyCode::Char('r') => AppAction::RefreshData,
            KeyCode::Char('R') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                AppAction::RefreshData
            }
            _ => AppAction::None,
        }
    }

    fn handle_search_input(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.search_query.clear();
                self.is_search_mode = false;
            }
            KeyCode::Enter => {
                self.is_search_mode = false;
            }
            KeyCode::Backspace => {
                self.search_query.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_query.clear();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_query.push(c);
            }
            _ => {}
        }
        AppAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies full forward tab cycle across all views and wraps to Dashboard.
    #[test]
    fn tab_cycles_all_views_forward() {
        let mut app = AppState::default();

        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::Nodes);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::Pods);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::Services);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::Deployments);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::Dashboard);
    }

    /// Verifies reverse tab cycle wraps from Dashboard to Deployments.
    #[test]
    fn shift_tab_cycles_reverse() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::BackTab));
        assert_eq!(app.view(), AppView::Deployments);
    }

    /// Verifies entering search mode and adding/removing characters.
    #[test]
    fn search_query_add_backspace_and_clear() {
        let mut app = AppState::default();

        app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
        app.handle_key_event(KeyEvent::from(KeyCode::Char('a')));
        app.handle_key_event(KeyEvent::from(KeyCode::Char('b')));
        app.handle_key_event(KeyEvent::from(KeyCode::Backspace));

        assert_eq!(app.search_query(), "a");

        app.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(app.search_query(), "");
    }

    /// Verifies pressing Esc in search mode exits mode and clears query.
    #[test]
    fn search_mode_esc_clears_and_exits() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
        app.handle_key_event(KeyEvent::from(KeyCode::Char('x')));

        app.handle_key_event(KeyEvent::from(KeyCode::Esc));

        assert_eq!(app.search_query(), "");
        assert!(!app.is_search_mode());
    }

    /// Verifies refresh actions are emitted for `r` and Ctrl+R.
    #[test]
    fn refresh_action_transitions() {
        let mut app = AppState::default();
        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('r'))),
            AppAction::RefreshData
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::CONTROL)),
            AppAction::RefreshData
        );
    }

    /// Verifies quit action and should_quit state transition.
    #[test]
    fn quit_action_sets_should_quit() {
        let mut app = AppState::default();

        let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('q')));

        assert_eq!(action, AppAction::Quit);
        assert!(app.should_quit());
    }

    /// Verifies Esc closes detail view before requesting app quit.
    #[test]
    fn esc_closes_detail_before_quit() {
        let mut app = AppState {
            detail_view: Some(DetailViewState::default()),
            ..AppState::default()
        };

        let action = app.handle_key_event(KeyEvent::from(KeyCode::Esc));

        assert_eq!(action, AppAction::CloseDetail);
        assert!(!app.should_quit());
    }

    /// Verifies selection index saturates at zero when moving up.
    #[test]
    fn selected_index_never_underflows() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Up));
        assert_eq!(app.selected_idx(), 0);
    }

    /// Verifies selection can grow with repeated down events.
    #[test]
    fn selected_index_grows_with_down_events() {
        let mut app = AppState::default();
        for _ in 0..5 {
            app.handle_key_event(KeyEvent::from(KeyCode::Down));
        }
        assert_eq!(app.selected_idx(), 5);
    }

    /// Verifies selection resets to zero when switching tabs.
    #[test]
    fn view_switch_resets_selection_index() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Down));
        app.handle_key_event(KeyEvent::from(KeyCode::Down));
        assert_eq!(app.selected_idx(), 2);

        app.handle_key_event(KeyEvent::from(KeyCode::Tab));

        assert_eq!(app.selected_idx(), 0);
    }

    /// Verifies rapid tab switching remains stable.
    #[test]
    fn rapid_tab_switching_is_stable() {
        let mut app = AppState::default();

        for _ in 0..100 {
            app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        }

        assert_eq!(app.view(), AppView::Dashboard);
    }

    /// Verifies search input ignores Ctrl-modified characters except supported shortcuts.
    #[test]
    fn search_input_ignores_ctrl_characters() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));

        app.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));

        assert_eq!(app.search_query(), "");
    }

    /// Verifies error message can be set and cleared.
    #[test]
    fn error_message_set_and_clear() {
        let mut app = AppState::default();
        app.set_error("boom".to_string());
        assert_eq!(app.error_message(), Some("boom"));

        app.clear_error();
        assert_eq!(app.error_message(), None);
    }

    /// Verifies resource reference helper methods return expected kind/name/namespace.
    #[test]
    fn resource_ref_helpers_work_for_each_variant() {
        let node = ResourceRef::Node("n1".to_string());
        let pod = ResourceRef::Pod("p1".to_string(), "ns1".to_string());

        assert_eq!(node.kind(), "Node");
        assert_eq!(node.name(), "n1");
        assert_eq!(node.namespace(), None);

        assert_eq!(pod.kind(), "Pod");
        assert_eq!(pod.name(), "p1");
        assert_eq!(pod.namespace(), Some("ns1"));
    }
}
