//! Color theme system for KubecTUI

use ratatui::style::{Color, Style, Modifier};

/// Represents a color theme for the application
#[derive(Debug, Clone)]
pub struct Theme {
    /// Theme name (e.g., "dark", "nord", "dracula")
    pub name: String,
    /// Background color
    pub bg: Color,
    /// Foreground text color
    pub fg: Color,
    /// Border color
    pub border: Color,
    /// Accent color (blue)
    pub accent: Color,
    /// Success color (green)
    pub success: Color,
    /// Warning color (orange)
    pub warning: Color,
    /// Error color (red)
    pub error: Color,
    /// Disabled/muted color
    pub muted: Color,
    /// Selection highlight background
    pub selection_bg: Color,
}

impl Theme {
    /// Load theme by name, returns dark theme if not found
    pub fn load(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "nord" => Self::nord(),
            "dracula" => Self::dracula(),
            _ => Self::dark(),
        }
    }

    /// Dark theme (default)
    pub fn dark() -> Self {
        Self {
            name: "dark".to_string(),
            bg: Color::Rgb(13, 17, 23),
            fg: Color::Rgb(230, 237, 243),
            border: Color::Rgb(48, 54, 61),
            accent: Color::Rgb(88, 166, 255),
            success: Color::Rgb(63, 185, 80),
            warning: Color::Rgb(210, 153, 34),
            error: Color::Rgb(248, 81, 73),
            muted: Color::Rgb(110, 118, 129),
            selection_bg: Color::Rgb(48, 54, 61),
        }
    }

    /// Nord theme
    pub fn nord() -> Self {
        Self {
            name: "nord".to_string(),
            bg: Color::Rgb(46, 52, 64),
            fg: Color::Rgb(236, 239, 244),
            border: Color::Rgb(76, 86, 106),
            accent: Color::Rgb(136, 192, 208),
            success: Color::Rgb(163, 190, 140),
            warning: Color::Rgb(235, 203, 139),
            error: Color::Rgb(191, 97, 106),
            muted: Color::Rgb(129, 161, 193),
            selection_bg: Color::Rgb(76, 86, 106),
        }
    }

    /// Dracula theme
    pub fn dracula() -> Self {
        Self {
            name: "dracula".to_string(),
            bg: Color::Rgb(40, 42, 54),
            fg: Color::Rgb(248, 248, 242),
            border: Color::Rgb(68, 71, 90),
            accent: Color::Rgb(139, 233, 253),
            success: Color::Rgb(80, 250, 123),
            warning: Color::Rgb(241, 250, 140),
            error: Color::Rgb(255, 121, 198),
            muted: Color::Rgb(98, 114, 164),
            selection_bg: Color::Rgb(68, 71, 90),
        }
    }

    /// Get a style for a given color key
    pub fn get_style(&self, color_type: &str) -> Style {
        match color_type {
            "accent" => Style::default().fg(self.accent),
            "success" => Style::default().fg(self.success),
            "warning" => Style::default().fg(self.warning),
            "error" => Style::default().fg(self.error),
            "muted" => Style::default().fg(self.muted),
            "border" => Style::default().fg(self.border),
            _ => Style::default().fg(self.fg),
        }
    }

    /// Get a style for log levels
    pub fn get_log_level_style(&self, level: &str) -> Style {
        match level.to_uppercase().as_str() {
            "ERROR" | "ERR" => Style::default().fg(self.error).add_modifier(Modifier::BOLD),
            "WARN" | "WARNING" => Style::default().fg(self.warning),
            "INFO" | "INFORMATION" => Style::default().fg(self.fg),
            "DEBUG" => Style::default().fg(self.muted),
            "TRACE" => Style::default().fg(self.muted).add_modifier(Modifier::DIM),
            _ => Style::default().fg(self.fg),
        }
    }

    /// Get a style for status indicators
    pub fn get_status_style(&self, status: &str) -> Style {
        match status.to_lowercase().as_str() {
            "running" | "active" | "true" => Style::default().fg(self.success),
            "pending" | "starting" => Style::default().fg(self.warning),
            "failed" | "error" | "false" => Style::default().fg(self.error),
            _ => Style::default().fg(self.muted),
        }
    }

    /// Get title bar style
    pub fn title_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Get border style
    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    /// Get selection highlight style
    pub fn selection_style(&self) -> Style {
        Style::default()
            .bg(self.selection_bg)
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Get hovered item style
    pub fn hover_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Get disabled/inactive style
    pub fn inactive_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    /// Get muted style
    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.muted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_dark_theme() {
        let theme = Theme::load("dark");
        assert_eq!(theme.name, "dark");
    }

    #[test]
    fn test_load_nord_theme() {
        let theme = Theme::load("nord");
        assert_eq!(theme.name, "nord");
    }

    #[test]
    fn test_load_dracula_theme() {
        let theme = Theme::load("dracula");
        assert_eq!(theme.name, "dracula");
    }
}
