//! Shared terminal key modifier predicates.

use crossterm::event::{KeyEvent, KeyModifiers};

pub fn plain_shortcut(key: KeyEvent) -> bool {
    key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
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
