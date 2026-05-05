//! Shared terminal key modifier predicates.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub const LOG_PRESET_KEYS_HINT: &str = "[[]/[]]";

pub fn plain_shortcut(key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char(_) | KeyCode::BackTab => {
            key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
        }
        _ => key.modifiers.is_empty(),
    }
}

pub fn edit_key(key: KeyEvent) -> bool {
    key.modifiers.is_empty()
}

pub fn ctrl_shortcut(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && key
            .modifiers
            .difference(KeyModifiers::CONTROL | KeyModifiers::SHIFT)
            .is_empty()
}

pub fn ctrl_char(key: KeyEvent, expected: char) -> bool {
    ctrl_shortcut(key)
        && matches!(key.code, KeyCode::Char(actual) if actual.eq_ignore_ascii_case(&expected))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtrlScrollAction {
    LineDown,
    LineUp,
    PageDown,
    PageUp,
}

pub fn ctrl_scroll_action(key: KeyEvent) -> Option<CtrlScrollAction> {
    if !ctrl_shortcut(key) {
        return None;
    }

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some(CtrlScrollAction::LineDown),
        KeyCode::Char('k') | KeyCode::Up => Some(CtrlScrollAction::LineUp),
        KeyCode::Char('d') | KeyCode::PageDown => Some(CtrlScrollAction::PageDown),
        KeyCode::Char('u') | KeyCode::PageUp => Some(CtrlScrollAction::PageUp),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_shortcut_allows_shift_only_for_chars_and_backtab() {
        assert!(plain_shortcut(KeyEvent::new(
            KeyCode::Char('D'),
            KeyModifiers::SHIFT
        )));
        assert!(plain_shortcut(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT
        )));
        assert!(!plain_shortcut(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT
        )));
        assert!(!plain_shortcut(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::SHIFT
        )));
        assert!(!plain_shortcut(KeyEvent::new(
            KeyCode::PageDown,
            KeyModifiers::SHIFT
        )));
    }

    #[test]
    fn ctrl_scroll_action_maps_supported_keys() {
        assert_eq!(
            ctrl_scroll_action(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL)),
            Some(CtrlScrollAction::LineDown)
        );
        assert_eq!(
            ctrl_scroll_action(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL)),
            Some(CtrlScrollAction::LineDown)
        );
        assert_eq!(
            ctrl_scroll_action(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL)),
            Some(CtrlScrollAction::LineUp)
        );
        assert_eq!(
            ctrl_scroll_action(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL)),
            Some(CtrlScrollAction::LineUp)
        );
        assert_eq!(
            ctrl_scroll_action(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Some(CtrlScrollAction::PageDown)
        );
        assert_eq!(
            ctrl_scroll_action(KeyEvent::new(KeyCode::PageDown, KeyModifiers::CONTROL)),
            Some(CtrlScrollAction::PageDown)
        );
        assert_eq!(
            ctrl_scroll_action(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)),
            Some(CtrlScrollAction::PageUp)
        );
        assert_eq!(
            ctrl_scroll_action(KeyEvent::new(KeyCode::PageUp, KeyModifiers::CONTROL)),
            Some(CtrlScrollAction::PageUp)
        );
    }

    #[test]
    fn ctrl_scroll_action_rejects_unsupported_modifiers() {
        assert_eq!(
            ctrl_scroll_action(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            ctrl_scroll_action(KeyEvent::new(
                KeyCode::Char('j'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )),
            None
        );
        assert_eq!(
            ctrl_scroll_action(KeyEvent::new(
                KeyCode::Char('j'),
                KeyModifiers::CONTROL | KeyModifiers::META
            )),
            None
        );
        assert_eq!(
            ctrl_scroll_action(KeyEvent::new(
                KeyCode::Char('j'),
                KeyModifiers::CONTROL | KeyModifiers::SUPER
            )),
            None
        );
        assert_eq!(
            ctrl_scroll_action(KeyEvent::new(
                KeyCode::Char('D'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            None
        );
    }

    #[test]
    fn ctrl_char_matches_shifted_and_unshifted_ascii() {
        assert!(ctrl_char(
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            'u'
        ));
        assert!(ctrl_char(
            KeyEvent::new(
                KeyCode::Char('U'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            ),
            'u'
        ));
        assert!(!ctrl_char(
            KeyEvent::new(
                KeyCode::Char('u'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            ),
            'u'
        ));
        assert!(!ctrl_char(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            'u'
        ));
    }
}
