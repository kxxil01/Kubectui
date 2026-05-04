//! Context (kubeconfig) picker modal component.

use crate::ui::{
    clear_input_at_cursor, components::render_vertical_scrollbar, contains_ci,
    cursor_visible_input_line, delete_char_left_at_cursor, delete_char_right_at_cursor,
    insert_char_at_cursor, move_cursor_end, move_cursor_home, move_cursor_left, move_cursor_right,
    table_window, wrap_span_groups, wrapped_line_count,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Style},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

fn plain_shortcut(key: KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

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
    selection_anchor: Option<String>,
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
            selection_anchor: None,
            search_query: String::new(),
            search_cursor: 0,
            is_open: false,
        }
    }

    pub fn open(&mut self) {
        self.is_open = true;
        clear_input_at_cursor(&mut self.search_query, &mut self.search_cursor);
        let filtered = self.filtered_context_indices();
        self.selected_index = self
            .current_context
            .as_ref()
            .and_then(|context| {
                filtered
                    .iter()
                    .position(|index| self.contexts[*index] == *context)
            })
            .unwrap_or(0);
        self.selection_anchor = self
            .selected_context_from_indices(&filtered)
            .map(ToOwned::to_owned);
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
        let selected_context = self
            .selected_context_from_indices(&self.filtered_context_indices())
            .map(ToOwned::to_owned);
        let had_selected_context = selected_context.is_some() || self.selection_anchor.is_some();
        self.contexts = contexts;
        self.current_context = current_context;
        self.restore_selected_context(selected_context.or_else(|| self.selection_anchor.clone()));
        if had_selected_context {
            let filtered = self.filtered_context_indices();
            if !filtered.is_empty() {
                self.selection_anchor = self
                    .selected_context_from_indices(&filtered)
                    .map(ToOwned::to_owned);
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ContextPickerAction {
        if !self.is_open {
            return ContextPickerAction::None;
        }

        let filtered = self.filtered_context_indices();

        match key.code {
            KeyCode::Esc => ContextPickerAction::Close,
            KeyCode::Enter if plain_shortcut(key) => self
                .selected_context_from_indices(&filtered)
                .map(ToOwned::to_owned)
                .map(ContextPickerAction::Select)
                .unwrap_or(ContextPickerAction::None),
            KeyCode::Down if plain_shortcut(key) => {
                let len = filtered.len();
                if len > 0 {
                    self.selected_index = (self.selected_index + 1) % len;
                    self.selection_anchor = self
                        .selected_context_from_indices(&filtered)
                        .map(ToOwned::to_owned);
                }
                ContextPickerAction::None
            }
            KeyCode::Up if plain_shortcut(key) => {
                let len = filtered.len();
                if len > 0 {
                    self.selected_index = if self.selected_index == 0 {
                        len - 1
                    } else {
                        self.selected_index - 1
                    };
                    self.selection_anchor = self
                        .selected_context_from_indices(&filtered)
                        .map(ToOwned::to_owned);
                }
                ContextPickerAction::None
            }
            KeyCode::Backspace => {
                if self.search_cursor > 0 {
                    let selected_context = self
                        .selected_context_from_indices(&filtered)
                        .map(ToOwned::to_owned)
                        .or_else(|| self.selection_anchor.clone());
                    delete_char_left_at_cursor(&mut self.search_query, &mut self.search_cursor);
                    self.restore_selected_context(selected_context);
                }
                ContextPickerAction::None
            }
            KeyCode::Delete => {
                let previous_len = self.search_query.len();
                delete_char_right_at_cursor(&mut self.search_query, self.search_cursor);
                if self.search_query.len() != previous_len {
                    let selected_context = self
                        .selected_context_from_indices(&filtered)
                        .map(ToOwned::to_owned)
                        .or_else(|| self.selection_anchor.clone());
                    self.restore_selected_context(selected_context);
                }
                ContextPickerAction::None
            }
            KeyCode::Left => {
                move_cursor_left(&mut self.search_cursor);
                ContextPickerAction::None
            }
            KeyCode::Right => {
                move_cursor_right(&mut self.search_cursor, &self.search_query);
                ContextPickerAction::None
            }
            KeyCode::Home => {
                move_cursor_home(&mut self.search_cursor);
                ContextPickerAction::None
            }
            KeyCode::End => {
                move_cursor_end(&mut self.search_cursor, &self.search_query);
                ContextPickerAction::None
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if !self.search_query.is_empty() {
                    let selected_context = self
                        .selected_context_from_indices(&filtered)
                        .map(ToOwned::to_owned)
                        .or_else(|| self.selection_anchor.clone());
                    clear_input_at_cursor(&mut self.search_query, &mut self.search_cursor);
                    self.restore_selected_context(selected_context);
                }
                ContextPickerAction::None
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let selected_context = self
                    .selected_context_from_indices(&filtered)
                    .map(ToOwned::to_owned)
                    .or_else(|| self.selection_anchor.clone());
                insert_char_at_cursor(&mut self.search_query, &mut self.search_cursor, c);
                self.restore_selected_context(selected_context);
                ContextPickerAction::None
            }
            _ => ContextPickerAction::None,
        }
    }

    fn restore_selected_context(&mut self, selected_context: Option<String>) {
        let filtered = self.filtered_context_indices();
        let matched_index = selected_context.as_ref().and_then(|selected| {
            filtered
                .iter()
                .position(|index| self.contexts[*index] == *selected)
        });
        self.selected_index = matched_index.unwrap_or(0);
        self.selection_anchor = matched_index
            .and_then(|index| filtered.get(index))
            .map(|index| self.contexts[*index].clone())
            .or(selected_context)
            .or_else(|| {
                self.selected_context_from_indices(&filtered)
                    .map(ToOwned::to_owned)
            });
    }

    fn selected_context_from_indices<'a>(&'a self, indices: &[usize]) -> Option<&'a str> {
        indices
            .get(self.selected_index)
            .and_then(|index| self.contexts.get(*index))
            .map(String::as_str)
    }

    fn filtered_context_indices(&self) -> Vec<usize> {
        if self.search_query.is_empty() {
            return (0..self.contexts.len()).collect();
        }

        self.contexts
            .iter()
            .enumerate()
            .filter_map(|(index, ctx)| contains_ci(ctx, &self.search_query).then_some(index))
            .collect()
    }

    pub fn filtered_contexts(&self) -> Vec<String> {
        self.filtered_context_indices()
            .into_iter()
            .map(|index| self.contexts[index].clone())
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
                vec![Span::styled(" [↑↓] ", theme.keybind_key_style())],
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

        let contexts = self.filtered_context_indices();
        let items: Vec<ListItem> = if contexts.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "  No contexts match",
                theme.inactive_style(),
            )))]
        } else {
            contexts
                .iter()
                .enumerate()
                .map(|(idx, context_index)| {
                    let ctx = &self.contexts[*context_index];
                    let is_current = self.current_context.as_deref() == Some(ctx.as_str());
                    if idx == self.selected_index {
                        ListItem::new(Line::from(vec![
                            Span::styled(" ▶ ", theme.title_style()),
                            Span::styled(
                                ctx.as_str(),
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
                            Span::styled(ctx.as_str(), Style::default().fg(theme.fg_dim)),
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
    use ratatui::{Terminal, backend::TestBackend};

    fn rendered_text(picker: &ContextPicker, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");
        terminal
            .draw(|frame| picker.render(frame, frame.area()))
            .expect("context picker should render");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

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
    fn context_picker_modified_enter_and_arrows_do_not_select_or_navigate() {
        let mut picker = ContextPicker::new(
            vec![
                "ctx-a".to_string(),
                "ctx-b".to_string(),
                "ctx-c".to_string(),
            ],
            Some("ctx-a".to_string()),
        );
        picker.open();

        for (code, modifiers) in [
            (KeyCode::Enter, KeyModifiers::CONTROL),
            (KeyCode::Down, KeyModifiers::CONTROL),
            (KeyCode::Up, KeyModifiers::CONTROL),
            (KeyCode::Enter, KeyModifiers::ALT),
            (KeyCode::Down, KeyModifiers::ALT),
            (KeyCode::Up, KeyModifiers::ALT),
        ] {
            assert_eq!(
                picker.handle_key(KeyEvent::new(code, modifiers)),
                ContextPickerAction::None,
                "{code:?} {modifiers:?}"
            );
            assert_eq!(picker.selected_index, 0);
        }
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
    fn context_picker_footer_does_not_advertise_jk_navigation_because_jk_filter_query() {
        let mut picker = ContextPicker::new(
            vec!["ctx-a".to_string(), "ctx-b".to_string()],
            Some("ctx-a".to_string()),
        );
        picker.open();

        let rendered = rendered_text(&picker, 120, 40);

        assert!(rendered.contains("↑↓"));
        assert!(!rendered.contains("↑↓/jk"));
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
    fn context_picker_search_unicode_cursor_editing() {
        let mut picker = ContextPicker::new(vec!["default".to_string()], None);
        picker.open();

        picker.handle_key(KeyEvent::from(KeyCode::Char('a')));
        picker.handle_key(KeyEvent::from(KeyCode::Char('å')));
        picker.handle_key(KeyEvent::from(KeyCode::Char('b')));
        picker.handle_key(KeyEvent::from(KeyCode::Left));
        picker.handle_key(KeyEvent::from(KeyCode::Left));
        picker.handle_key(KeyEvent::from(KeyCode::Char('β')));
        picker.handle_key(KeyEvent::from(KeyCode::Delete));
        picker.handle_key(KeyEvent::from(KeyCode::Backspace));

        assert_eq!(picker.search_query, "ab");
        assert_eq!(picker.search_cursor, 1);
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
    fn context_picker_search_preserves_selected_context_when_still_visible() {
        let mut picker = ContextPicker::new(
            vec![
                "dev-east".to_string(),
                "prod-east".to_string(),
                "prod-west".to_string(),
            ],
            Some("prod-east".to_string()),
        );
        picker.open();
        assert_eq!(
            picker.filtered_contexts().get(picker.selected_index),
            Some(&"prod-east".to_string())
        );

        picker.handle_key(KeyEvent::from(KeyCode::Char('p')));

        assert_eq!(picker.search_query, "p");
        assert_eq!(
            picker.filtered_contexts().get(picker.selected_index),
            Some(&"prod-east".to_string())
        );
    }

    #[test]
    fn context_picker_roundtrip_preserves_selection_across_zero_matches() {
        let mut picker = ContextPicker::new(
            vec![
                "dev-east".to_string(),
                "prod-east".to_string(),
                "prod-west".to_string(),
            ],
            Some("prod-east".to_string()),
        );
        picker.open();
        for ch in ['z', 'z', 'z'] {
            picker.handle_key(KeyEvent::from(KeyCode::Char(ch)));
        }
        assert!(picker.filtered_contexts().is_empty());

        picker.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));

        assert_eq!(
            picker.filtered_contexts().get(picker.selected_index),
            Some(&"prod-east".to_string())
        );
    }

    #[test]
    fn context_picker_noop_search_edit_keys_keep_selection_and_query() {
        let mut picker = ContextPicker::new(
            vec![
                "dev-east".to_string(),
                "prod-east".to_string(),
                "prod-west".to_string(),
            ],
            Some("dev-east".to_string()),
        );
        picker.open();
        picker.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(
            picker.filtered_contexts().get(picker.selected_index),
            Some(&"prod-east".to_string())
        );

        picker.handle_key(KeyEvent::from(KeyCode::Backspace));
        picker.handle_key(KeyEvent::from(KeyCode::Delete));
        picker.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));

        assert!(picker.search_query.is_empty());
        assert_eq!(
            picker.filtered_contexts().get(picker.selected_index),
            Some(&"prod-east".to_string())
        );

        picker.handle_key(KeyEvent::from(KeyCode::Char('p')));
        picker.handle_key(KeyEvent::from(KeyCode::End));
        picker.handle_key(KeyEvent::from(KeyCode::Delete));

        assert_eq!(picker.search_query, "p");
        assert_eq!(
            picker.filtered_contexts().get(picker.selected_index),
            Some(&"prod-east".to_string())
        );
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

    #[test]
    fn context_picker_refresh_drops_stale_anchor_when_selected_context_disappears() {
        let mut picker = ContextPicker::new(
            vec!["dev".to_string(), "prod".to_string()],
            Some("dev".to_string()),
        );
        picker.open();
        picker.handle_key(KeyEvent::from(KeyCode::Down));

        picker.set_contexts(vec!["dev".to_string()], Some("dev".to_string()));
        picker.set_contexts(
            vec!["dev".to_string(), "prod".to_string()],
            Some("dev".to_string()),
        );

        assert_eq!(
            picker.filtered_contexts().get(picker.selected_index),
            Some(&"dev".to_string())
        );
    }
}
