//! Integration tests for app navigation and keyboard routing.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kubectui::app::{AppAction, AppState, AppView, DetailViewState};

/// Verifies tab transitions cover all views and wrap around.
#[test]
#[ignore = "Optional integration run"]
fn all_tab_transitions_work() {
    let mut app = AppState::default();
    let expected = [
        AppView::Nodes,
        AppView::Pods,
        AppView::Services,
        AppView::Deployments,
        AppView::Dashboard,
    ];

    for view in expected {
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), view);
    }
}

/// Verifies selection index boundaries are saturating at zero.
#[test]
#[ignore = "Optional integration run"]
fn selection_index_wraps_at_boundaries() {
    let mut app = AppState::default();

    app.handle_key_event(KeyEvent::from(KeyCode::Up));
    assert_eq!(app.selected_idx(), 0);

    for _ in 0..3 {
        app.handle_key_event(KeyEvent::from(KeyCode::Down));
    }
    assert_eq!(app.selected_idx(), 3);
}

/// Verifies entering and exiting search mode routes keys correctly.
#[test]
#[ignore = "Optional integration run"]
fn search_mode_entry_and_exit() {
    let mut app = AppState::default();

    app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
    assert!(app.is_search_mode());

    app.handle_key_event(KeyEvent::from(KeyCode::Char('a')));
    app.handle_key_event(KeyEvent::from(KeyCode::Enter));

    assert!(!app.is_search_mode());
    assert_eq!(app.search_query(), "a");
}

/// Verifies detail modal can be closed from any active view.
#[test]
#[ignore = "Optional integration run"]
fn detail_modal_open_close_from_views() {
    let mut app = AppState::default();

    for _ in 0..5 {
        app.detail_view = Some(DetailViewState::default());
        let action = app.handle_key_event(KeyEvent::from(KeyCode::Esc));
        assert_eq!(action, AppAction::CloseDetail);
        app.detail_view = None;

        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
    }
}

/// Verifies keyboard routing prioritizes search mode over global bindings.
#[test]
#[ignore = "Optional integration run"]
fn keyboard_input_routing_prefers_search_mode() {
    let mut app = AppState::default();
    app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));

    let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('r')));

    assert_eq!(action, AppAction::None);
    assert_eq!(app.search_query(), "r");

    app.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(app.search_query(), "");
}
