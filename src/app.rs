//! Application state machine and keyboard input handling.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

/// Actions emitted by input handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    None,
    RefreshData,
    Quit,
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
    /// - `q`/`Esc` (outside search mode): quit
    /// - `Tab` / `Shift+Tab`: switch view
    /// - `↑` / `↓`: move current selection
    /// - `/`: enter search mode
    /// - `Enter` (search mode): leave search mode
    /// - `Backspace` (search mode): delete character
    /// - `Ctrl+R` or `r`: refresh data
    pub fn handle_key_event(&mut self, key: KeyEvent) -> AppAction {
        if self.is_search_mode {
            return self.handle_search_input(key);
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
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
            KeyCode::Esc | KeyCode::Enter => {
                self.is_search_mode = false;
            }
            KeyCode::Backspace => {
                self.search_query.pop();
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

    #[test]
    fn tab_cycles_views() {
        let mut app = AppState::default();
        assert_eq!(app.view(), AppView::Dashboard);

        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::Nodes);

        app.handle_key_event(KeyEvent::from(KeyCode::BackTab));
        assert_eq!(app.view(), AppView::Dashboard);
    }

    #[test]
    fn search_mode_collects_input() {
        let mut app = AppState::default();

        app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
        assert!(app.is_search_mode());

        app.handle_key_event(KeyEvent::from(KeyCode::Char('a')));
        app.handle_key_event(KeyEvent::from(KeyCode::Char('b')));

        assert_eq!(app.search_query(), "ab");

        app.handle_key_event(KeyEvent::from(KeyCode::Enter));
        assert!(!app.is_search_mode());
    }
}
