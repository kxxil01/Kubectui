//! Context (kubeconfig) picker modal component.

use crate::ui::contains_ci;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Style},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph},
};

/// Actions emitted by context picker keyboard handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextPickerAction {
    None,
    Select(String),
    Close,
}

/// Modal picker for switching the active kubeconfig context at startup or runtime.
#[derive(Debug, Clone, Default)]
pub struct ContextPicker {
    contexts: Vec<String>,
    current_context: Option<String>,
    selected_index: usize,
    search_query: String,
    is_open: bool,
}

impl ContextPicker {
    pub fn new(contexts: Vec<String>, current_context: Option<String>) -> Self {
        Self {
            contexts,
            current_context,
            selected_index: 0,
            search_query: String::new(),
            is_open: false,
        }
    }

    pub fn open(&mut self) {
        self.is_open = true;
        self.selected_index = 0;
        self.search_query.clear();
    }

    pub fn close(&mut self) {
        self.is_open = false;
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn set_contexts(&mut self, contexts: Vec<String>, current_context: Option<String>) {
        self.contexts = contexts;
        self.current_context = current_context;
        self.selected_index = 0;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ContextPickerAction {
        if !self.is_open {
            return ContextPickerAction::None;
        }

        match key.code {
            KeyCode::Esc => ContextPickerAction::Close,
            KeyCode::Enter => self
                .filtered_contexts()
                .get(self.selected_index)
                .cloned()
                .map(ContextPickerAction::Select)
                .unwrap_or(ContextPickerAction::None),
            KeyCode::Down => {
                let len = self.filtered_contexts().len();
                if len > 0 {
                    self.selected_index = (self.selected_index + 1) % len;
                }
                ContextPickerAction::None
            }
            KeyCode::Up => {
                let len = self.filtered_contexts().len();
                if len > 0 {
                    self.selected_index = if self.selected_index == 0 {
                        len - 1
                    } else {
                        self.selected_index - 1
                    };
                }
                ContextPickerAction::None
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.selected_index = 0;
                ContextPickerAction::None
            }
            KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE => {
                self.search_query.push(c);
                self.selected_index = 0;
                ContextPickerAction::None
            }
            _ => ContextPickerAction::None,
        }
    }

    pub fn filtered_contexts(&self) -> Vec<String> {
        if self.search_query.is_empty() {
            return self.contexts.clone();
        }
        self.contexts
            .iter()
            .filter(|ctx| contains_ci(ctx, &self.search_query))
            .cloned()
            .collect()
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if !self.is_open {
            return;
        }

        use crate::ui::components::default_theme;

        let theme = default_theme();

        let popup_width = (area.width * 3 / 5).clamp(50, 80);
        let popup_height = (area.height * 2 / 3).clamp(14, 32);
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
            Span::styled(" ⎈ ", theme.title_style()),
            Span::styled("Switch Cluster Context", theme.title_style()),
            if let Some(ref cur) = self.current_context {
                Span::styled(format!("  ·  current: {cur}"), theme.inactive_style())
            } else {
                Span::raw("")
            },
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

        let contexts = self.filtered_contexts();
        let items: Vec<ListItem> = if contexts.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "  No contexts match",
                theme.inactive_style(),
            )))]
        } else {
            contexts
                .iter()
                .enumerate()
                .map(|(idx, ctx)| {
                    let is_current = self.current_context.as_deref() == Some(ctx.as_str());
                    if idx == self.selected_index {
                        ListItem::new(Line::from(vec![
                            Span::styled(" ▶ ", theme.title_style()),
                            Span::styled(
                                ctx.clone(),
                                Style::default()
                                    .fg(theme.selection_fg)
                                    .bg(theme.selection_bg)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            if is_current {
                                Span::styled("  ★ current", theme.badge_success_style())
                            } else {
                                Span::raw("")
                            },
                        ]))
                    } else {
                        ListItem::new(Line::from(vec![
                            Span::styled("   ", theme.inactive_style()),
                            Span::styled(ctx.clone(), Style::default().fg(theme.fg_dim)),
                            if is_current {
                                Span::styled("  ★", theme.badge_success_style())
                            } else {
                                Span::raw("")
                            },
                        ]))
                    }
                })
                .collect()
        };

        let ctx_count = contexts.len();
        let list_block = Block::default()
            .title(Span::styled(
                format!(" Contexts ({ctx_count}) "),
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
            Span::styled("connect  ", theme.keybind_desc_style()),
            Span::styled("[Esc] ", theme.keybind_key_style()),
            Span::styled("skip", theme.keybind_desc_style()),
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
    fn context_picker_navigation_wraps() {
        let mut picker = ContextPicker::new(
            vec![
                "ctx-a".to_string(),
                "ctx-b".to_string(),
                "ctx-c".to_string(),
            ],
            Some("ctx-a".to_string()),
        );
        picker.open();

        picker.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(picker.selected_index, 1);

        picker.handle_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(picker.selected_index, 0);

        picker.handle_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(picker.selected_index, 2);
    }

    #[test]
    fn context_picker_arrow_navigation_wraps() {
        let mut picker = ContextPicker::new(
            vec![
                "ctx-a".to_string(),
                "ctx-b".to_string(),
                "ctx-c".to_string(),
            ],
            Some("ctx-a".to_string()),
        );
        picker.open();

        picker.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(picker.selected_index, 1);

        picker.handle_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(picker.selected_index, 0);
    }

    #[test]
    fn context_picker_j_k_appends_to_search() {
        let mut picker = ContextPicker::new(
            vec!["ctx-a".to_string(), "ctx-b".to_string()],
            Some("ctx-a".to_string()),
        );
        picker.open();

        picker.handle_key(KeyEvent::from(KeyCode::Char('k')));
        assert_eq!(picker.search_query, "k");
        assert_eq!(picker.selected_index, 0);
    }

    #[test]
    fn context_picker_select_emits_context_name() {
        let mut picker = ContextPicker::new(
            vec!["prod".to_string(), "staging".to_string()],
            Some("prod".to_string()),
        );
        picker.open();
        picker.handle_key(KeyEvent::from(KeyCode::Down));

        let action = picker.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(action, ContextPickerAction::Select("staging".to_string()));
    }

    #[test]
    fn context_picker_filter_by_search() {
        let mut picker = ContextPicker::new(
            vec![
                "prod-us".to_string(),
                "staging-eu".to_string(),
                "prod-eu".to_string(),
            ],
            None,
        );
        picker.open();
        for c in "prod".chars() {
            picker.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        let filtered = picker.filtered_contexts();
        assert_eq!(filtered, vec!["prod-us", "prod-eu"]);
    }

    #[test]
    fn context_picker_esc_emits_close() {
        let mut picker = ContextPicker::new(vec!["ctx".to_string()], None);
        picker.open();
        assert_eq!(
            picker.handle_key(KeyEvent::from(KeyCode::Esc)),
            ContextPickerAction::Close
        );
    }
}
