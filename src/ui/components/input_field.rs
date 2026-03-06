//! Reusable text input widget for form fields.

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
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
            let byte_pos = self
                .value
                .char_indices()
                .nth(self.cursor_pos)
                .map_or(self.value.len(), |(i, _)| i);
            self.value.insert(byte_pos, c);
            self.cursor_pos += 1;
            self.error = false;
        }
    }

    /// Delete character before cursor.
    pub fn delete_char(&mut self) {
        if self.cursor_pos > 0 {
            let byte_pos = self
                .value
                .char_indices()
                .nth(self.cursor_pos - 1)
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.value.remove(byte_pos);
            self.cursor_pos -= 1;
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
        if self.value.is_empty() {
            return Err("This field is required".to_string());
        }
        Ok(())
    }

    /// Get styled display text with cursor.
    pub fn styled_text(&self, focused: bool) -> Span<'static> {
        let mut display = self.value.clone();

        if focused {
            let byte_pos = display
                .char_indices()
                .nth(self.cursor_pos)
                .map_or(display.len(), |(i, _)| i);
            display.insert(byte_pos, '█');
        }

        let style = if self.error {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else if focused {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        Span::styled(display, style)
    }

    /// Move cursor left.
    pub fn cursor_left(&mut self) {
        self.cursor_pos = self.cursor_pos.saturating_sub(1);
    }

    /// Move cursor right.
    pub fn cursor_right(&mut self) {
        self.cursor_pos = (self.cursor_pos + 1).min(self.value.chars().count());
    }

    /// Move cursor to start.
    pub fn cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    /// Move cursor to end.
    pub fn cursor_end(&mut self) {
        self.cursor_pos = self.value.chars().count();
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
    fn test_validate_port() {
        let mut field = InputFieldWidget::new(5);
        field.value = "8080".to_string();
        assert!(field.validate_port().is_ok());
    }
}
