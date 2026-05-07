//! Event handling and input routing for KubecTUI.

pub mod input;

pub use input::{apply_action, mouse_content_row_at, route_keyboard_input, route_mouse_input};
