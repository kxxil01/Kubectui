//! Namespace picker modal component.

use crate::ui::contains_ci;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

/// Actions emitted by namespace picker keyboard handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamespacePickerAction {
    None,
    Select(String),
    Close,
}

/// Modal picker for switching active namespace.
#[derive(Debug, Clone, Default)]
pub struct NamespacePicker {
    namespaces: Vec<String>,
    selected_index: usize,
    search_query: String,
    is_open: bool,
}

impl NamespacePicker {
    pub fn new(namespaces: Vec<String>) -> Self {
        Self {
            namespaces,
            selected_index: 0,
            search_query: String::new(),
            is_open: false,
        }
    }

    pub fn open(&mut self) {
        self.is_open = true;
        self.search_query.clear();
        self.selected_index = 0;
    }

    pub fn close(&mut self) {
        self.is_open = false;
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn set_namespaces(&mut self, namespaces: Vec<String>) {
        self.namespaces = namespaces;
        self.selected_index = 0;
    }

    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> NamespacePickerAction {
        if !self.is_open {
            return NamespacePickerAction::None;
        }

        match key.code {
            KeyCode::Esc => NamespacePickerAction::Close,
            KeyCode::Enter => self
                .filtered_namespaces()
                .get(self.selected_index)
                .cloned()
                .map(NamespacePickerAction::Select)
                .unwrap_or(NamespacePickerAction::None),
            KeyCode::Down => {
                let len = self.filtered_namespaces().len();
                if len > 0 {
                    self.selected_index = (self.selected_index + 1) % len;
                }
                NamespacePickerAction::None
            }
            KeyCode::Up => {
                let len = self.filtered_namespaces().len();
                if len > 0 {
                    self.selected_index = if self.selected_index == 0 {
                        len - 1
                    } else {
                        self.selected_index - 1
                    };
                }
                NamespacePickerAction::None
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.selected_index = 0;
                NamespacePickerAction::None
            }
            KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE => {
                self.search_query.push(c);
                self.selected_index = 0;
                NamespacePickerAction::None
            }
            _ => NamespacePickerAction::None,
        }
    }

    pub fn filtered_namespaces(&self) -> Vec<String> {
        if self.search_query.is_empty() {
            return self.namespaces.clone();
        }

        self.namespaces
            .iter()
            .filter(|ns| contains_ci(ns, &self.search_query))
            .cloned()
            .collect()
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if !self.is_open {
            return;
        }

        use crate::ui::components::default_theme;
        use ratatui::widgets::BorderType;

        let theme = default_theme();

        let popup_width = (area.width * 2 / 5).clamp(40, 60);
        let popup_height = (area.height * 2 / 3).clamp(12, 30);
        let popup = Rect {
            x: (area.width.saturating_sub(popup_width)) / 2,
            y: (area.height.saturating_sub(popup_height)) / 2,
            width: popup_width,
            height: popup_height,
        };

        frame.render_widget(Clear, popup);

        let outer_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_active_style())
            .style(Style::default().bg(theme.bg));
        frame.render_widget(outer_block, popup);

        let inner = Rect {
            x: popup.x + 1,
            y: popup.y + 1,
            width: popup.width.saturating_sub(2),
            height: popup.height.saturating_sub(2),
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(2),
            ])
            .split(inner);

        let title_line = Line::from(vec![
            Span::styled(
                format!(" {}", crate::icons::chrome_icon("cluster").active()),
                theme.title_style(),
            ),
            Span::styled("Switch Namespace", theme.title_style()),
        ]);
        let title_block = Block::default()
            .borders(Borders::BOTTOM)
            .border_style(theme.border_style())
            .style(Style::default().bg(theme.header_bg));
        frame.render_widget(Paragraph::new(title_line).block(title_block), chunks[0]);

        let search_content = if self.search_query.is_empty() {
            Line::from(vec![
                Span::styled("  ", theme.inactive_style()),
                Span::styled("Type to filter…", theme.inactive_style()),
            ])
        } else {
            Line::from(vec![
                Span::styled("  / ", theme.title_style()),
                Span::styled(self.search_query.clone(), Style::default().fg(theme.fg)),
                Span::styled("█", theme.title_style()),
            ])
        };

        let search_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(if self.search_query.is_empty() {
                theme.border_style()
            } else {
                theme.border_active_style()
            })
            .style(Style::default().bg(theme.bg_surface));
        frame.render_widget(
            Paragraph::new(search_content).block(search_block),
            chunks[1],
        );

        let namespaces = self.filtered_namespaces();
        let items: Vec<ListItem> = if namespaces.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "  No namespaces match",
                theme.inactive_style(),
            )))]
        } else {
            namespaces
                .iter()
                .enumerate()
                .map(|(idx, ns)| {
                    if idx == self.selected_index {
                        ListItem::new(Line::from(vec![
                            Span::styled(" ▶ ", theme.title_style()),
                            Span::styled(
                                ns.clone(),
                                Style::default()
                                    .fg(theme.selection_fg)
                                    .bg(theme.selection_bg)
                                    .add_modifier(ratatui::style::Modifier::BOLD),
                            ),
                        ]))
                    } else {
                        ListItem::new(Line::from(vec![
                            Span::styled("   ", theme.inactive_style()),
                            Span::styled(ns.clone(), Style::default().fg(theme.fg_dim)),
                        ]))
                    }
                })
                .collect()
        };

        let ns_count = namespaces.len();
        let list_block = Block::default()
            .title(Span::styled(
                format!(" Namespaces ({ns_count}) "),
                theme.muted_style(),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_style())
            .style(Style::default().bg(theme.bg));
        frame.render_widget(List::new(items).block(list_block), chunks[2]);

        let footer_line = Line::from(vec![
            Span::styled(" [↑↓/jk] ", theme.keybind_key_style()),
            Span::styled("navigate  ", theme.keybind_desc_style()),
            Span::styled("[Enter] ", theme.keybind_key_style()),
            Span::styled("select  ", theme.keybind_desc_style()),
            Span::styled("[Esc] ", theme.keybind_key_style()),
            Span::styled("close", theme.keybind_desc_style()),
        ]);
        let footer_block = Block::default()
            .borders(Borders::TOP)
            .border_style(theme.border_style())
            .style(Style::default().bg(theme.statusbar_bg));
        frame.render_widget(Paragraph::new(footer_line).block(footer_block), chunks[3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_namespace_picker_navigation() {
        let mut picker = NamespacePicker::new(vec![
            "all".to_string(),
            "default".to_string(),
            "kube-system".to_string(),
        ]);
        picker.open();

        picker.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(picker.selected_index(), 1);

        picker.handle_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(picker.selected_index(), 0);
    }

    #[test]
    fn test_namespace_picker_arrow_navigation() {
        let mut picker = NamespacePicker::new(vec![
            "all".to_string(),
            "default".to_string(),
            "kube-system".to_string(),
        ]);
        picker.open();

        picker.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(picker.selected_index(), 1);

        picker.handle_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(picker.selected_index(), 0);
    }

    #[test]
    fn test_namespace_picker_j_k_appends_to_search() {
        let mut picker = NamespacePicker::new(vec![
            "all".to_string(),
            "default".to_string(),
            "kube-system".to_string(),
        ]);
        picker.open();

        picker.handle_key(KeyEvent::from(KeyCode::Char('k')));
        // 'k' should filter to "kube-system", not navigate
        let filtered = picker.filtered_namespaces();
        assert!(filtered.iter().any(|ns| ns.contains("kube")));
    }

    #[test]
    fn test_namespace_picker_select() {
        let mut picker = NamespacePicker::new(vec!["all".to_string(), "default".to_string()]);
        picker.open();
        picker.handle_key(KeyEvent::from(KeyCode::Down));

        let action = picker.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(action, NamespacePickerAction::Select("default".to_string()));

        picker.close();
        assert!(!picker.is_open());
    }
}
