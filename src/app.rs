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
}

/// Actions emitted by input handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    None,
    RefreshData,
    Quit,
    OpenDetail(ResourceRef),
    CloseDetail,
}

/// Runtime state for UI interaction and navigation.
#[derive(Debug, Clone)]
pub struct AppState {
    view: AppView,
    selected_idx: usize,
    search_query: String,
    is_search_mode: bool,
    should_quit: bool,
    error_message: Option<String>,
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

    /// Handles a keyboard event and updates app state.
    ///
    /// Keybindings:
    /// - `q`: quit
    /// - `Esc` (when detail open): close detail modal
    /// - `Esc` (outside detail/search): quit
    /// - `Tab` / `Shift+Tab`: switch view
    /// - `↑` / `↓`: move current selection
    /// - `/`: enter search mode
    /// - `Enter` (search mode): leave search mode
    /// - `Esc` (search mode): clear query + leave search mode
    /// - `Ctrl+U` (search mode): clear query
    /// - `Backspace` (search mode): delete character
    /// - `Ctrl+R` or `r`: refresh data
    pub fn handle_key_event(&mut self, key: KeyEvent) -> AppAction {
        if self.is_search_mode {
            return self.handle_search_input(key);
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
