//! Color theme system for KubecTUI

use ratatui::{
    style::{Color, Modifier, Style},
    widgets::BorderType,
};

/// Represents a color theme for the application
#[derive(Debug, Clone)]
pub struct Theme {
    /// Theme name (e.g., "dark", "nord", "dracula", "catppuccin")
    pub name: String,
    /// Primary background color
    pub bg: Color,
    /// Secondary/surface background (slightly lighter)
    pub bg_surface: Color,
    /// Foreground text color
    pub fg: Color,
    /// Dimmed foreground for secondary text
    pub fg_dim: Color,
    /// Border color (inactive panels)
    pub border: Color,
    /// Border color for active/focused panels
    pub border_active: Color,
    /// Accent color (kubernetes blue)
    pub accent: Color,
    /// Secondary accent (purple/violet)
    pub accent2: Color,
    /// Success color (green)
    pub success: Color,
    /// Warning color (orange/yellow)
    pub warning: Color,
    /// Error color (red)
    pub error: Color,
    /// Disabled/muted color
    pub muted: Color,
    /// Selection highlight background
    pub selection_bg: Color,
    /// Selection highlight foreground
    pub selection_fg: Color,
    /// Header/title bar background
    pub header_bg: Color,
    /// Tab active background
    pub tab_active_bg: Color,
    /// Tab active foreground
    pub tab_active_fg: Color,
    /// Tab inactive foreground
    pub tab_inactive_fg: Color,
    /// Status bar background
    pub statusbar_bg: Color,
    /// Info/neutral color
    pub info: Color,
}

impl Theme {
    /// Load theme by name, returns dark theme if not found
    pub fn load(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "nord" => Self::nord(),
            "dracula" => Self::dracula(),
            "catppuccin" | "mocha" => Self::catppuccin_mocha(),
            _ => Self::dark(),
        }
    }

    /// Dark theme — GitHub-inspired deep dark (default)
    pub fn dark() -> Self {
        Self {
            name: "dark".to_string(),
            bg: Color::Rgb(13, 17, 23),
            bg_surface: Color::Rgb(22, 27, 34),
            fg: Color::Rgb(230, 237, 243),
            fg_dim: Color::Rgb(139, 148, 158),
            border: Color::Rgb(48, 54, 61),
            border_active: Color::Rgb(88, 166, 255),
            accent: Color::Rgb(88, 166, 255),
            accent2: Color::Rgb(188, 140, 255),
            success: Color::Rgb(63, 185, 80),
            warning: Color::Rgb(210, 153, 34),
            error: Color::Rgb(248, 81, 73),
            muted: Color::Rgb(110, 118, 129),
            selection_bg: Color::Rgb(33, 58, 95),
            selection_fg: Color::Rgb(88, 166, 255),
            header_bg: Color::Rgb(22, 27, 34),
            tab_active_bg: Color::Rgb(33, 58, 95),
            tab_active_fg: Color::Rgb(88, 166, 255),
            tab_inactive_fg: Color::Rgb(110, 118, 129),
            statusbar_bg: Color::Rgb(22, 27, 34),
            info: Color::Rgb(88, 166, 255),
        }
    }

    /// Nord theme — Arctic, north-bluish color palette
    pub fn nord() -> Self {
        Self {
            name: "nord".to_string(),
            bg: Color::Rgb(46, 52, 64),
            bg_surface: Color::Rgb(59, 66, 82),
            fg: Color::Rgb(236, 239, 244),
            fg_dim: Color::Rgb(216, 222, 233),
            border: Color::Rgb(76, 86, 106),
            border_active: Color::Rgb(136, 192, 208),
            accent: Color::Rgb(136, 192, 208),
            accent2: Color::Rgb(180, 142, 173),
            success: Color::Rgb(163, 190, 140),
            warning: Color::Rgb(235, 203, 139),
            error: Color::Rgb(191, 97, 106),
            muted: Color::Rgb(129, 161, 193),
            selection_bg: Color::Rgb(67, 76, 94),
            selection_fg: Color::Rgb(136, 192, 208),
            header_bg: Color::Rgb(59, 66, 82),
            tab_active_bg: Color::Rgb(67, 76, 94),
            tab_active_fg: Color::Rgb(136, 192, 208),
            tab_inactive_fg: Color::Rgb(129, 161, 193),
            statusbar_bg: Color::Rgb(59, 66, 82),
            info: Color::Rgb(129, 161, 193),
        }
    }

    /// Dracula theme — dark with vibrant purple/pink accents
    pub fn dracula() -> Self {
        Self {
            name: "dracula".to_string(),
            bg: Color::Rgb(40, 42, 54),
            bg_surface: Color::Rgb(50, 52, 68),
            fg: Color::Rgb(248, 248, 242),
            fg_dim: Color::Rgb(189, 147, 249),
            border: Color::Rgb(68, 71, 90),
            border_active: Color::Rgb(139, 233, 253),
            accent: Color::Rgb(139, 233, 253),
            accent2: Color::Rgb(189, 147, 249),
            success: Color::Rgb(80, 250, 123),
            warning: Color::Rgb(241, 250, 140),
            error: Color::Rgb(255, 85, 85),
            muted: Color::Rgb(98, 114, 164),
            selection_bg: Color::Rgb(68, 71, 90),
            selection_fg: Color::Rgb(139, 233, 253),
            header_bg: Color::Rgb(50, 52, 68),
            tab_active_bg: Color::Rgb(68, 71, 90),
            tab_active_fg: Color::Rgb(139, 233, 253),
            tab_inactive_fg: Color::Rgb(98, 114, 164),
            statusbar_bg: Color::Rgb(50, 52, 68),
            info: Color::Rgb(139, 233, 253),
        }
    }

    /// Catppuccin Mocha — warm dark theme with pastel accents
    pub fn catppuccin_mocha() -> Self {
        Self {
            name: "catppuccin".to_string(),
            bg: Color::Rgb(30, 30, 46),
            bg_surface: Color::Rgb(36, 36, 54),
            fg: Color::Rgb(205, 214, 244),
            fg_dim: Color::Rgb(166, 173, 200),
            border: Color::Rgb(69, 71, 90),
            border_active: Color::Rgb(137, 180, 250),
            accent: Color::Rgb(137, 180, 250),
            accent2: Color::Rgb(203, 166, 247),
            success: Color::Rgb(166, 227, 161),
            warning: Color::Rgb(249, 226, 175),
            error: Color::Rgb(243, 139, 168),
            muted: Color::Rgb(108, 112, 134),
            selection_bg: Color::Rgb(49, 50, 68),
            selection_fg: Color::Rgb(137, 180, 250),
            header_bg: Color::Rgb(36, 36, 54),
            tab_active_bg: Color::Rgb(49, 50, 68),
            tab_active_fg: Color::Rgb(137, 180, 250),
            tab_inactive_fg: Color::Rgb(108, 112, 134),
            statusbar_bg: Color::Rgb(36, 36, 54),
            info: Color::Rgb(116, 199, 236),
        }
    }

    /// Get a style for a given semantic color key
    pub fn get_style(&self, color_type: &str) -> Style {
        match color_type {
            "accent" => Style::default().fg(self.accent),
            "accent2" => Style::default().fg(self.accent2),
            "success" => Style::default().fg(self.success),
            "warning" => Style::default().fg(self.warning),
            "error" => Style::default().fg(self.error),
            "muted" => Style::default().fg(self.muted),
            "border" => Style::default().fg(self.border),
            "border_active" => Style::default().fg(self.border_active),
            "info" => Style::default().fg(self.info),
            "fg_dim" => Style::default().fg(self.fg_dim),
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
            _ => Style::default().fg(self.fg_dim),
        }
    }

    /// Get a style for Kubernetes resource status strings
    pub fn get_status_style(&self, status: &str) -> Style {
        match status.to_lowercase().as_str() {
            "running" | "active" | "true" | "ready" | "succeeded" | "complete" | "bound" => {
                Style::default().fg(self.success)
            }
            "pending" | "starting" | "containercreating" | "init" | "terminating" => {
                Style::default().fg(self.warning)
            }
            "failed" | "error" | "false" | "crashloopbackoff" | "oomkilled" | "evicted" => {
                Style::default().fg(self.error).add_modifier(Modifier::BOLD)
            }
            "unknown" => Style::default().fg(self.muted),
            _ => Style::default().fg(self.fg_dim),
        }
    }

    /// Title / heading style
    pub fn title_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Section heading style (slightly dimmer than title)
    pub fn section_title_style(&self) -> Style {
        Style::default()
            .fg(self.fg)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    }

    /// Border style for inactive panels
    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    /// Border style for the focused/active panel
    pub fn border_active_style(&self) -> Style {
        Style::default().fg(self.border_active)
    }

    /// Table header row style
    pub fn header_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .bg(self.bg_surface)
            .add_modifier(Modifier::BOLD)
    }

    /// Selected row highlight style (REVERSED for maximum terminal compatibility)
    pub fn selection_style(&self) -> Style {
        Style::default()
            .bg(self.selection_bg)
            .fg(self.selection_fg)
            .add_modifier(Modifier::BOLD)
    }

    /// Alternate row background for zebra striping
    pub fn row_alt_style(&self) -> Style {
        Style::default().bg(self.bg_surface)
    }

    /// Hover/focus style without background change
    pub fn hover_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Disabled/inactive element style
    pub fn inactive_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    /// Muted secondary text style
    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    /// Key binding label style (accent + bold)
    pub fn keybind_key_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Key binding description style
    pub fn keybind_desc_style(&self) -> Style {
        Style::default().fg(self.fg_dim)
    }

    /// Badge/chip style for success state
    pub fn badge_success_style(&self) -> Style {
        Style::default()
            .fg(self.success)
            .add_modifier(Modifier::BOLD)
    }

    /// Badge/chip style for warning state
    pub fn badge_warning_style(&self) -> Style {
        Style::default()
            .fg(self.warning)
            .add_modifier(Modifier::BOLD)
    }

    /// Badge/chip style for error state
    pub fn badge_error_style(&self) -> Style {
        Style::default()
            .fg(self.error)
            .add_modifier(Modifier::BOLD)
    }

    /// Gauge/progress bar style (filled portion)
    pub fn gauge_style(&self, percent: u8) -> Style {
        let color = if percent >= 85 {
            self.error
        } else if percent >= 65 {
            self.warning
        } else {
            self.success
        };
        Style::default().fg(color).bg(self.bg_surface)
    }

    /// Returns the preferred border type for this theme
    pub fn border_type(&self) -> BorderType {
        BorderType::Rounded
    }

    /// Returns the highlight symbol used in tables
    pub fn highlight_symbol(&self) -> &'static str {
        " ▶ "
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

    #[test]
    fn test_load_catppuccin_theme() {
        let theme = Theme::load("catppuccin");
        assert_eq!(theme.name, "catppuccin");
    }

    #[test]
    fn test_gauge_style_thresholds() {
        let theme = Theme::dark();
        assert_eq!(theme.gauge_style(90).fg, Some(theme.error));
        assert_eq!(theme.gauge_style(70).fg, Some(theme.warning));
        assert_eq!(theme.gauge_style(50).fg, Some(theme.success));
    }
}
