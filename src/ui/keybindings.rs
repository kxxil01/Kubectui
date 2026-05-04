//! Shared terminal key modifier predicates.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
}
