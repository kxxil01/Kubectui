//! Namespace picker modal component.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Style},
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
            KeyCode::Char('j') | KeyCode::Down => {
                let len = self.filtered_namespaces().len();
                if len > 0 {
                    self.selected_index = (self.selected_index + 1) % len;
                }
                NamespacePickerAction::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
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
            KeyCode::Char(c) => {
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

        let q = self.search_query.to_ascii_lowercase();
        self.namespaces
            .iter()
            .filter(|ns| ns.to_ascii_lowercase().contains(&q))
            .cloned()
            .collect()
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if !self.is_open {
            return;
        }

        let popup = Rect {
            x: area.width / 6,
            y: area.height / 6,
            width: area.width * 2 / 3,
            height: area.height * 2 / 3,
        };

        frame.render_widget(Clear, popup);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(2),
            ])
            .split(popup);

        let title = Paragraph::new("Select namespace")
            .style(Style::default().fg(Color::Cyan))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Namespace Picker"),
            );
        frame.render_widget(title, chunks[0]);

        let search = Paragraph::new(self.search_query.clone()).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Search (type to filter)"),
        );
        frame.render_widget(search, chunks[1]);

        let namespaces = self.filtered_namespaces();
        let items: Vec<ListItem> = if namespaces.is_empty() {
            vec![ListItem::new(Line::from("No namespaces found"))]
        } else {
            namespaces
                .iter()
                .enumerate()
                .map(|(idx, ns)| {
                    if idx == self.selected_index {
                        ListItem::new(Line::from(Span::styled(
                            format!("> {ns}"),
                            Style::default().fg(Color::Yellow),
                        )))
                    } else {
                        ListItem::new(Line::from(format!("  {ns}")))
                    }
                })
                .collect()
        };

        let list =
            List::new(items).block(Block::default().borders(Borders::ALL).title("Namespaces"));
        frame.render_widget(list, chunks[2]);

        let footer = Paragraph::new("[j/k or ↑/↓] navigate • [Enter] select • [Esc] close")
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(footer, chunks[3]);
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
