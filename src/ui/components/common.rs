//! Common UI components and utilities for KubecTUI

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::ui::theme::Theme;

/// Draw a status badge
#[derive(Debug, Clone, Copy)]
pub enum BadgeStatus {
    Success,
    Error,
    Warning,
    Pending,
    Info,
}

pub fn draw_badge(theme: &Theme, status: BadgeStatus, label: &str) -> Span {
    let (symbol, color) = match status {
        BadgeStatus::Success => ("✓", theme.success),
        BadgeStatus::Error => ("✗", theme.error),
        BadgeStatus::Warning => ("!", theme.warning),
        BadgeStatus::Pending => ("⋯", theme.warning),
        BadgeStatus::Info => ("ℹ", theme.accent),
    };

    let text = format!(" {} {} ", symbol, label);
    Span::styled(text, Style::default().fg(color).add_modifier(Modifier::BOLD))
}

/// Draw a styled block with theme colors
pub fn draw_styled_block(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    title: &str,
    content: Vec<Line>,
) {
    let block = Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .style(theme.border_style());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if !content.is_empty() {
        let paragraph = Paragraph::new(content);
        frame.render_widget(paragraph, inner);
    }
}

/// Draw a title bar with metadata
pub fn draw_title_bar<'a>(
    theme: &'a Theme,
    title: &'a str,
    metadata: Vec<(&'a str, &'a str)>,
) -> Line<'a> {
    let mut spans = vec![
        Span::styled(title, theme.title_style()),
        Span::raw(" "),
    ];

    for (i, (key, value)) in metadata.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" | "));
        }
        spans.push(Span::styled(
            format!("{}: {}", key, value),
            theme.inactive_style(),
        ));
    }

    Line::from(spans)
}

/// Draw help/shortcut line
pub fn draw_help_line<'a>(theme: &'a Theme, shortcuts: Vec<(&'a str, &'a str)>) -> Line<'a> {
    let mut spans = vec![];

    for (i, (key, description)) in shortcuts.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }

        spans.push(Span::styled(
            key.to_string(),
            theme.get_style("accent").add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(": "));
        spans.push(Span::styled(description.to_string(), theme.inactive_style()));
    }

    Line::from(spans)
}

/// Split layout horizontally
pub fn horizontal_layout(area: Rect, widths: &[Constraint]) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(widths)
        .split(area)
        .to_vec()
}

/// Split layout vertically
pub fn vertical_layout(area: Rect, heights: &[Constraint]) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints(heights)
        .split(area)
        .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_badge_success() {
        let theme = Theme::dark();
        let badge = draw_badge(&theme, BadgeStatus::Success, "Running");
        assert!(badge.content.contains('✓'));
    }

    #[test]
    fn test_badge_error() {
        let theme = Theme::dark();
        let badge = draw_badge(&theme, BadgeStatus::Error, "Failed");
        assert!(badge.content.contains('✗'));
    }
}
