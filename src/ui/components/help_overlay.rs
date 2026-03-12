//! Keybinding help overlay displayed with `?`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use crate::ui::components::default_theme;

#[derive(Debug, Clone, Default)]
pub struct HelpOverlay {
    is_open: bool,
    scroll: usize,
}

const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Global",
        &[
            ("?", "Toggle this help"),
            ("q", "Quit (with confirmation)"),
            ("Esc", "Back / close overlay"),
            ("Tab / Shift+Tab", "Next / previous view"),
            ("j / k / \u{2193} / \u{2191}", "Navigate list"),
            ("Enter", "Open detail / activate"),
            ("/", "Search / filter"),
            ("~", "Namespace picker"),
            ("c", "Context picker"),
            (":", "Action palette (navigate + columns)"),
            ("r", "Refresh data"),
            ("Ctrl+y", "Copy resource name"),
            ("Y", "Copy namespace/name"),
            ("T", "Cycle theme"),
            ("b", "Toggle workbench"),
            ("[ / ]", "Previous / next workbench tab"),
            ("Ctrl+W", "Close workbench tab"),
            ("Ctrl+Up / Ctrl+Down", "Resize workbench"),
        ],
    ),
    (
        "Detail View",
        &[
            ("y", "View YAML"),
            ("v", "View timeline"),
            ("l", "View logs"),
            ("x", "Exec into pod"),
            ("f", "Port forward"),
            ("s", "Scale replicas"),
            ("p", "Probe panel"),
            ("R", "Restart rollout"),
            ("e", "Edit YAML"),
            ("d", "Delete resource"),
            ("F", "Force delete (in confirm dialog)"),
            ("T", "Trigger CronJob"),
            ("w", "View relations"),
        ],
    ),
    (
        "Sort (Pods)",
        &[
            ("n", "Sort by name"),
            ("a / 1", "Sort by age"),
            ("2", "Sort by status"),
            ("3", "Sort by restarts"),
            ("0", "Clear sort"),
        ],
    ),
    (
        "Sort (Other Views)",
        &[
            ("n", "Sort by name"),
            ("a / 1", "Sort by age"),
            ("0", "Clear sort"),
        ],
    ),
    (
        "Workbench (focused)",
        &[
            ("z", "Maximize / restore"),
            ("j / k", "Scroll down / up"),
            ("g / G", "Jump to top / bottom"),
            ("PageDown / PageUp", "Scroll by page"),
            ("Esc", "Un-maximize or blur"),
        ],
    ),
    (
        "Node Actions",
        &[
            ("c", "Cordon node"),
            ("u", "Uncordon node"),
            ("D", "Drain node (with confirmation)"),
        ],
    ),
    (
        "Relations Tree",
        &[
            ("j / k", "Move cursor down / up"),
            ("l / Right", "Expand node"),
            ("h / Left", "Collapse / jump to parent"),
            ("g / G", "Jump to top / bottom"),
            ("Enter", "Open detail for resource"),
            ("Esc", "Return focus from workbench"),
        ],
    ),
    (
        "Logs",
        &[
            ("f", "Toggle follow mode"),
            ("P", "Toggle previous logs"),
            ("t", "Toggle timestamps"),
            ("/", "Search in logs"),
            ("Enter / Esc", "Apply / cancel log search"),
            ("Ctrl+U", "Clear log search input"),
            ("n / N", "Next / previous match"),
            ("y", "Copy log content"),
            ("S", "Save logs to file"),
        ],
    ),
    (
        "Workload Logs",
        &[
            ("f", "Toggle follow mode"),
            ("p", "Cycle pod filter"),
            ("c", "Cycle container filter"),
            ("/", "Text filter"),
            ("Enter / Esc", "Apply / cancel text filter"),
            ("Ctrl+U", "Clear text filter input"),
            ("y", "Copy log content"),
            ("S", "Save logs to file"),
        ],
    ),
];

impl HelpOverlay {
    pub fn open(&mut self) {
        self.is_open = true;
        self.scroll = 0;
    }

    pub fn close(&mut self) {
        self.is_open = false;
    }

    pub fn toggle(&mut self) {
        if self.is_open {
            self.close();
        } else {
            self.open();
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn total_lines() -> usize {
        let mut count = 0;
        for (_, bindings) in SECTIONS {
            count += 1; // section header
            count += bindings.len();
            count += 1; // blank line
        }
        count
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let theme = default_theme();

        let popup_width = 60u16.min(area.width.saturating_sub(4));
        let popup_height = 30u16.min(area.height.saturating_sub(4));
        let popup = centered_rect(popup_width, popup_height, area);
        frame.render_widget(Clear, popup);

        let block = Block::default()
            .title(Span::styled(
                " Keybindings ",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_style())
            .style(Style::default().bg(theme.bg_surface));

        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);

        let mut lines: Vec<Line> = Vec::new();
        for (section_name, bindings) in SECTIONS {
            lines.push(Line::from(Span::styled(
                format!("  {section_name}"),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )));
            for (key, desc) in *bindings {
                lines.push(Line::from(vec![
                    Span::styled(format!("    {key:<24}"), Style::default().fg(theme.fg)),
                    Span::styled(*desc, Style::default().fg(theme.fg_dim)),
                ]));
            }
            lines.push(Line::from(""));
        }

        let visible_height = sections[0].height as usize;
        let max_scroll = lines.len().saturating_sub(visible_height);
        let scroll = self.scroll.min(max_scroll);
        let end = (scroll + visible_height).min(lines.len());
        let visible = if scroll < end {
            lines[scroll..end].to_vec()
        } else {
            vec![]
        };

        frame.render_widget(
            Paragraph::new(visible).wrap(Wrap { trim: false }),
            sections[0],
        );

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " [?/Esc] close  [j/k] scroll ",
                Style::default().fg(theme.fg_dim),
            ))),
            sections[1],
        );
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_overlay_toggle() {
        let mut overlay = HelpOverlay::default();
        assert!(!overlay.is_open());
        overlay.toggle();
        assert!(overlay.is_open());
        overlay.toggle();
        assert!(!overlay.is_open());
    }

    #[test]
    fn help_overlay_scroll() {
        let mut overlay = HelpOverlay::default();
        overlay.open();
        assert_eq!(overlay.scroll, 0);
        overlay.scroll_down();
        assert_eq!(overlay.scroll, 1);
        overlay.scroll_up();
        assert_eq!(overlay.scroll, 0);
        overlay.scroll_up();
        assert_eq!(overlay.scroll, 0);
    }

    #[test]
    fn total_lines_is_nonzero() {
        assert!(HelpOverlay::total_lines() > 20);
    }
}
