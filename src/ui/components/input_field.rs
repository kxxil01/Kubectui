//! Reusable text input widget for form fields.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::ui::{
    cursor_visible_input_line, delete_char_left_at_cursor, delete_char_right_at_cursor,
    insert_char_at_cursor, move_cursor_end, move_cursor_home, move_cursor_left, move_cursor_right,
};

/// Reusable text input widget state.
#[derive(Debug, Clone)]
pub struct InputFieldWidget {
    /// Current input value.
    pub value: String,
    /// Maximum allowed length.
    pub max_length: usize,
    /// Whether this field is focused.
    pub focused: bool,
    /// Whether field contains error.
    pub error: bool,
    /// Cursor position (relative to visible text).
    pub cursor_pos: usize,
}

impl InputFieldWidget {
    /// Create a new input field.
    pub fn new(max_length: usize) -> Self {
        Self {
            value: String::new(),
            max_length,
            focused: false,
            error: false,
            cursor_pos: 0,
        }
    }

    /// Create with initial value.
    pub fn with_value(initial: &str, max_length: usize) -> Self {
        let value = initial.to_string();
        let cursor_pos = value.chars().count();
        Self {
            value,
            max_length,
            focused: false,
            error: false,
            cursor_pos,
        }
    }

    /// Update character at cursor position.
    pub fn add_char(&mut self, c: char) {
        if self.value.chars().count() < self.max_length {
            insert_char_at_cursor(&mut self.value, &mut self.cursor_pos, c);
            self.error = false;
        }
    }

    /// Delete character before cursor.
    pub fn backspace_char(&mut self) {
        if self.cursor_pos != 0 {
            delete_char_left_at_cursor(&mut self.value, &mut self.cursor_pos);
            self.error = false;
        }
    }

    /// Delete character at cursor.
    pub fn delete_char(&mut self) {
        let previous = self.value.len();
        delete_char_right_at_cursor(&mut self.value, self.cursor_pos);
        if self.value.len() != previous {
            self.error = false;
        }
    }

    /// Clear all input.
    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor_pos = 0;
        self.error = false;
    }

    /// Validate port number (1-65535).
    pub fn validate_port(&self) -> Result<u16, String> {
        if self.value.is_empty() {
            return Err("Port required".to_string());
        }
        let port: u16 = self
            .value
            .parse()
            .map_err(|_| "Invalid port number".to_string())?;
        if port == 0 {
            return Err("Port must be > 0".to_string());
        }
        Ok(port)
    }

    /// Validate port number (allows 0 for auto-assign).
    pub fn validate_port_optional(&self) -> Result<u16, String> {
        if self.value.is_empty() {
            return Ok(0);
        }
        self.value
            .parse()
            .map_err(|_| "Invalid port number".to_string())
    }

    /// Validate non-empty string.
    pub fn validate_required(&self) -> Result<(), String> {
        if self.value.trim().is_empty() {
            return Err("This field is required".to_string());
        }
        Ok(())
    }

    /// Get styled display text with cursor-follow inside available width.
    pub fn styled_line(
        &self,
        prefix: &[Span<'static>],
        focused: bool,
        width: usize,
    ) -> Line<'static> {
        let style = if self.error {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else if focused {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        cursor_visible_input_line(
            prefix,
            &self.value,
            focused.then_some(self.cursor_pos),
            style,
            style,
            &[],
            width,
        )
    }

    /// Move cursor left.
    pub fn cursor_left(&mut self) {
        move_cursor_left(&mut self.cursor_pos);
    }

    /// Move cursor right.
    pub fn cursor_right(&mut self) {
        move_cursor_right(&mut self.cursor_pos, &self.value);
    }

    /// Move cursor to start.
    pub fn cursor_home(&mut self) {
        move_cursor_home(&mut self.cursor_pos);
    }

    /// Move cursor to end.
    pub fn cursor_end(&mut self) {
        move_cursor_end(&mut self.cursor_pos, &self.value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_char() {
        let mut field = InputFieldWidget::new(10);
        field.add_char('a');
        field.add_char('b');
        assert_eq!(field.value, "ab");
    }

    #[test]
    fn test_backspace_char_removes_previous_character() {
        let mut field = InputFieldWidget::with_value("abcd", 10);
        field.cursor_pos = 2;
        field.backspace_char();
        assert_eq!(field.value, "acd");
        assert_eq!(field.cursor_pos, 1);
    }

    #[test]
    fn test_delete_char_removes_character_at_cursor() {
        let mut field = InputFieldWidget::with_value("abcd", 10);
        field.cursor_pos = 1;
        field.delete_char();
        assert_eq!(field.value, "acd");
        assert_eq!(field.cursor_pos, 1);
    }

    #[test]
    fn unicode_cursor_editing_uses_character_positions() {
        let mut field = InputFieldWidget::with_value("aåb", 10);
        field.cursor_pos = 1;
        field.add_char('β');
        assert_eq!(field.value, "aβåb");
        assert_eq!(field.cursor_pos, 2);

        field.delete_char();
        assert_eq!(field.value, "aβb");
        assert_eq!(field.cursor_pos, 2);

        field.backspace_char();
        assert_eq!(field.value, "ab");
        assert_eq!(field.cursor_pos, 1);
    }

    #[test]
    fn test_validate_port() {
        let mut field = InputFieldWidget::new(5);
        field.value = "8080".to_string();
        assert!(field.validate_port().is_ok());
    }

    #[test]
    fn validate_required_rejects_whitespace_only() {
        let field = InputFieldWidget::with_value("   ", 10);
        assert!(field.validate_required().is_err());
    }
}
