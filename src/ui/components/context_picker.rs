//! Context (kubeconfig) picker modal component.

use crate::ui::{
    components::render_vertical_scrollbar, contains_ci, cursor_visible_input_line, table_window,
    wrap_span_groups, wrapped_line_count,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Style},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
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
    search_cursor: usize,
    is_open: bool,
}

fn context_picker_popup(area: Rect) -> Rect {
    let preferred_width = (area.width * 3 / 5).clamp(50, 80);
    let preferred_height = (area.height * 2 / 3).clamp(14, 32);
    crate::ui::bounded_popup_rect(area, preferred_width, preferred_height, 1, 1)
}

fn use_compact_context_picker_layout(popup: Rect) -> bool {
    popup.width < 50 || popup.height < 12
}

impl ContextPicker {
    pub fn new(contexts: Vec<String>, current_context: Option<String>) -> Self {
        Self {
            contexts,
            current_context,
            selected_index: 0,
            search_query: String::new(),
            search_cursor: 0,
            is_open: false,
        }
    }

    pub fn open(&mut self) {
        self.is_open = true;
        self.search_query.clear();
        self.search_cursor = 0;
        let filtered = self.filtered_contexts();
        self.selected_index = self
            .current_context
            .as_ref()
            .and_then(|context| filtered.iter().position(|entry| entry == context))
            .unwrap_or(0);
    }

    pub fn close(&mut self) {
        self.is_open = false;
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    pub fn set_contexts(&mut self, contexts: Vec<String>, current_context: Option<String>) {
        let selected_context = self.filtered_contexts().get(self.selected_index).cloned();
        self.contexts = contexts;
        self.current_context = current_context;
        let filtered = self.filtered_contexts();
        self.selected_index = selected_context
            .as_ref()
            .and_then(|context| filtered.iter().position(|entry| entry == context))
            .unwrap_or_else(|| self.selected_index.min(filtered.len().saturating_sub(1)));
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
                if self.search_cursor > 0
                    && let Some((byte_idx, _)) =
                        self.search_query.char_indices().nth(self.search_cursor - 1)
                {
                    self.search_query.remove(byte_idx);
                    self.search_cursor = self.search_cursor.saturating_sub(1);
                }
                self.selected_index = 0;
                ContextPickerAction::None
            }
            KeyCode::Delete => {
                if let Some((byte_idx, _)) =
                    self.search_query.char_indices().nth(self.search_cursor)
                {
                    self.search_query.remove(byte_idx);
                }
                self.selected_index = 0;
                ContextPickerAction::None
            }
            KeyCode::Left => {
                self.search_cursor = self.search_cursor.saturating_sub(1);
                ContextPickerAction::None
            }
            KeyCode::Right => {
                self.search_cursor =
                    (self.search_cursor + 1).min(self.search_query.chars().count());
                ContextPickerAction::None
            }
            KeyCode::Home => {
                self.search_cursor = 0;
                ContextPickerAction::None
            }
            KeyCode::End => {
                self.search_cursor = self.search_query.chars().count();
                ContextPickerAction::None
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_query.clear();
                self.search_cursor = 0;
                self.selected_index = 0;
                ContextPickerAction::None
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let byte_idx = self
                    .search_query
                    .char_indices()
                    .nth(self.search_cursor)
                    .map_or(self.search_query.len(), |(idx, _)| idx);
                self.search_query.insert(byte_idx, c);
                self.search_cursor += 1;
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
        let popup = context_picker_popup(area);
        let compact = use_compact_context_picker_layout(popup);

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
                vec![Span::styled("connect  ", theme.keybind_desc_style())],
                vec![Span::styled("[Esc] ", theme.keybind_key_style())],
                vec![Span::styled("close", theme.keybind_desc_style())],
            ]
        } else {
            vec![
                vec![Span::styled(" [↑↓/jk] ", theme.keybind_key_style())],
                vec![Span::styled("navigate  ", theme.keybind_desc_style())],
                vec![Span::styled("[Enter] ", theme.keybind_key_style())],
                vec![Span::styled("connect  ", theme.keybind_desc_style())],
                vec![Span::styled("[Esc] ", theme.keybind_key_style())],
                vec![Span::styled("skip", theme.keybind_desc_style())],
            ]
        };
        let footer_lines = wrap_span_groups(&footer_groups, inner.width.max(1));
        let footer_height = wrapped_line_count(&footer_lines, inner.width.max(1)).max(1) as u16 + 1;

        let title_line = Line::from(vec![
            Span::styled(
                format!(" {}", crate::icons::chrome_icon("cluster").active()),
                theme.title_style(),
            ),
            Span::styled("Switch Cluster Context", theme.title_style()),
            if !compact {
                self.current_context
                    .as_ref()
                    .map(|cur| Span::styled(format!("  ·  current: {cur}"), theme.inactive_style()))
                    .unwrap_or_else(|| Span::raw(""))
            } else {
                Span::raw("")
            },
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
                Some(self.search_cursor),
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
        let selected =
            (!contexts.is_empty()).then_some(self.selected_index.min(contexts.len() - 1));
        let offset = selected
            .map(|selected_index| context_picker_offset(contexts.len(), selected_index, chunks[2]))
            .unwrap_or_default();
        let mut state = ListState::default()
            .with_selected(selected)
            .with_offset(offset);
        frame.render_stateful_widget(List::new(items).block(list_block), chunks[2], &mut state);
        render_vertical_scrollbar(frame, chunks[2], contexts.len(), offset);

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

fn context_picker_offset(total: usize, selected: usize, area: Rect) -> usize {
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
    fn context_picker_accepts_shift_modified_search_chars() {
        let mut picker = ContextPicker::new(
            vec!["arn:prod".to_string(), "dev".to_string()],
            Some("dev".to_string()),
        );
        picker.open();

        picker.handle_key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::SHIFT));

        assert_eq!(picker.search_query, ":");
    }

    #[test]
    fn context_picker_search_supports_cursor_editing() {
        let mut picker = ContextPicker::new(vec!["default".to_string()], None);
        picker.open();

        picker.handle_key(KeyEvent::from(KeyCode::Char('a')));
        picker.handle_key(KeyEvent::from(KeyCode::Char('c')));
        picker.handle_key(KeyEvent::from(KeyCode::Left));
        picker.handle_key(KeyEvent::from(KeyCode::Char('b')));

        assert_eq!(picker.search_query, "abc");
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
    fn context_picker_open_selects_current_context() {
        let mut picker = ContextPicker::new(
            vec!["dev".to_string(), "prod".to_string(), "staging".to_string()],
            Some("staging".to_string()),
        );

        picker.open();

        assert_eq!(picker.selected_index, 2);
        assert_eq!(
            picker.filtered_contexts().get(picker.selected_index),
            Some(&"staging".to_string())
        );
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

    #[test]
    fn context_picker_popup_stays_within_small_terminal() {
        let popup = context_picker_popup(Rect::new(0, 0, 40, 10));
        assert!(popup.width <= 40);
        assert!(popup.height <= 10);
    }

    #[test]
    fn compact_context_picker_layout_activates_on_small_terminal() {
        assert!(use_compact_context_picker_layout(context_picker_popup(
            Rect::new(0, 0, 40, 10,)
        )));
        assert!(!use_compact_context_picker_layout(context_picker_popup(
            Rect::new(0, 0, 120, 40,)
        )));
    }

    #[test]
    fn context_picker_offset_keeps_selection_visible() {
        let area = Rect::new(0, 0, 40, 6);
        assert_eq!(context_picker_offset(10, 8, area), 6);
    }

    #[test]
    fn context_picker_refresh_preserves_selected_context_identity() {
        let mut picker = ContextPicker::new(
            vec!["dev".to_string(), "prod".to_string()],
            Some("dev".to_string()),
        );
        picker.open();
        picker.handle_key(KeyEvent::from(KeyCode::Down));

        picker.set_contexts(
            vec!["dev".to_string(), "prod".to_string(), "staging".to_string()],
            Some("dev".to_string()),
        );

        assert_eq!(
            picker.filtered_contexts().get(picker.selected_index),
            Some(&"prod".to_string())
        );
    }
}
