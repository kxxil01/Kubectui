//! Input event routing and dispatching.

use crossterm::event::KeyEvent;

use crate::app::{AppAction, AppState};

/// Routes a keyboard event to the appropriate handler based on application state.
pub fn route_keyboard_input(key: KeyEvent, app_state: &mut AppState) -> AppAction {
    app_state.handle_key_event(key)
}

/// Applies an action to the application state and returns whether state changed.
pub fn apply_action(action: AppAction, app_state: &mut AppState) -> bool {
    match action {
        AppAction::None => false,
        AppAction::Quit => {
            app_state.should_quit = true;
            true
        }
        AppAction::RefreshData => true,
        AppAction::OpenDetail(_) => true,
        AppAction::CloseDetail => {
            app_state.detail_view = None;
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_action_none() {
        let mut app = AppState::default();
        assert!(!apply_action(AppAction::None, &mut app));
    }

    #[test]
    fn test_apply_action_quit() {
        let mut app = AppState::default();
        assert!(!app.should_quit);
        apply_action(AppAction::Quit, &mut app);
        assert!(app.should_quit);
    }

    #[test]
    fn test_apply_action_close_detail() {
        let mut app = AppState::default();
        app.detail_view = Some(Default::default());
        assert!(app.detail_view.is_some());
        apply_action(AppAction::CloseDetail, &mut app);
        assert!(app.detail_view.is_none());
    }
}
