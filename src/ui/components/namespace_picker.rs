//! Namespace picker modal component.

use crate::ui::{
    components::render_vertical_scrollbar, contains_ci, cursor_visible_input_line, table_window,
    wrap_span_groups, wrapped_line_count,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
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

fn namespace_picker_popup(area: Rect) -> Rect {
    let preferred_width = (area.width * 2 / 5).clamp(40, 60);
    let preferred_height = (area.height * 2 / 3).clamp(12, 30);
    crate::ui::bounded_popup_rect(area, preferred_width, preferred_height, 1, 1)
}

fn use_compact_namespace_picker_layout(popup: Rect) -> bool {
    popup.width < 44 || popup.height < 12
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

    pub fn namespaces(&self) -> &[String] {
        &self.namespaces
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
        let popup = namespace_picker_popup(area);
        let compact = use_compact_namespace_picker_layout(popup);

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

        let footer_groups = if compact {
            vec![
                vec![Span::styled(" [Enter] ", theme.keybind_key_style())],
                vec![Span::styled("select  ", theme.keybind_desc_style())],
                vec![Span::styled("[Esc] ", theme.keybind_key_style())],
                vec![Span::styled("close", theme.keybind_desc_style())],
            ]
        } else {
            vec![
                vec![Span::styled(" [↑↓/jk] ", theme.keybind_key_style())],
                vec![Span::styled("navigate  ", theme.keybind_desc_style())],
                vec![Span::styled("[Enter] ", theme.keybind_key_style())],
                vec![Span::styled("select  ", theme.keybind_desc_style())],
                vec![Span::styled("[Esc] ", theme.keybind_key_style())],
                vec![Span::styled("close", theme.keybind_desc_style())],
            ]
        };
        let footer_lines = wrap_span_groups(&footer_groups, inner.width.max(1));
        let footer_height = wrapped_line_count(&footer_lines, inner.width.max(1)).max(1) as u16 + 1;

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
        let title_lines = vec![title_line];
        let title_height = wrapped_line_count(&title_lines, inner.width.max(1)).max(1) as u16
            + u16::from(!compact);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(title_height),
                Constraint::Length(3),
                Constraint::Min(if compact { 1 } else { 3 }),
                Constraint::Length(footer_height),
            ])
            .split(inner);
        frame.render_widget(
            Paragraph::new(title_lines)
                .block(title_block)
                .wrap(Wrap { trim: false }),
            chunks[0],
        );

        let search_content = if self.search_query.is_empty() {
            Line::from(vec![
                Span::styled("  ", theme.inactive_style()),
                Span::styled("Type to filter…", theme.inactive_style()),
            ])
        } else {
            cursor_visible_input_line(
                &[Span::styled("  / ".to_string(), theme.title_style())],
                &self.search_query,
                Some(self.search_query.chars().count()),
                Style::default().fg(theme.fg),
                theme.title_style(),
                &[],
                usize::from(chunks[1].width.saturating_sub(2).max(1)),
            )
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
        let selected =
            (!namespaces.is_empty()).then_some(self.selected_index.min(namespaces.len() - 1));
        let offset = selected
            .map(|selected_index| {
                namespace_picker_offset(namespaces.len(), selected_index, chunks[2])
            })
            .unwrap_or_default();
        let mut state = ListState::default()
            .with_selected(selected)
            .with_offset(offset);
        frame.render_stateful_widget(List::new(items).block(list_block), chunks[2], &mut state);
        render_vertical_scrollbar(frame, chunks[2], namespaces.len(), offset);

        let footer_block = Block::default()
            .borders(Borders::TOP)
            .border_style(theme.border_style())
            .style(Style::default().bg(theme.statusbar_bg));
        frame.render_widget(
            Paragraph::new(footer_lines)
                .wrap(Wrap { trim: false })
                .block(footer_block),
            chunks[3],
        );
    }
}

fn namespace_picker_offset(total: usize, selected: usize, area: Rect) -> usize {
    table_window(
        total,
        selected,
        usize::from(area.height.saturating_sub(2)).max(1),
    )
    .start
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

    #[test]
    fn namespace_picker_popup_stays_within_small_terminal() {
        let popup = namespace_picker_popup(Rect::new(0, 0, 40, 10));
        assert!(popup.width <= 40);
        assert!(popup.height <= 10);
    }

    #[test]
    fn compact_namespace_picker_layout_activates_on_small_terminal() {
        assert!(use_compact_namespace_picker_layout(namespace_picker_popup(
            Rect::new(0, 0, 40, 10),
        )));
        assert!(!use_compact_namespace_picker_layout(
            namespace_picker_popup(Rect::new(0, 0, 120, 40),)
        ));
    }

    #[test]
    fn namespace_picker_offset_keeps_selection_visible() {
        let area = Rect::new(0, 0, 40, 6);
        assert_eq!(namespace_picker_offset(10, 8, area), 6);
    }
}
