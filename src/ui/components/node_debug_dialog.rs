//! Guarded node debug-shell dialog.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Span, Style},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use crate::k8s::{
    exec::DebugImagePreset,
    node_debug::{NodeDebugLaunchRequest, NodeDebugProfile, default_debug_image},
};
use crate::ui::components::render_vertical_scrollbar;
use crate::ui::{
    cursor_visible_input_line, truncate_line_content, truncate_message, wrap_span_groups,
    wrapped_line_count,
};

fn plain_shortcut(key: KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeDebugField {
    Preset,
    CustomImage,
    Namespace,
    Profile,
    Launch,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeDebugDialogEvent {
    None,
    Submit,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeDebugDialogState {
    pub node_name: String,
    pub focus_field: NodeDebugField,
    pub selected_preset: DebugImagePreset,
    pub custom_image: String,
    pub available_namespaces: Vec<String>,
    pub namespace_index: usize,
    pub profile: NodeDebugProfile,
    pub pending_launch: bool,
    pub pending_launch_action_history_id: Option<u64>,
    pub error_message: Option<String>,
    pub notes_scroll: usize,
    pub custom_image_cursor: usize,
}

impl NodeDebugDialogState {
    pub fn new(
        node_name: impl Into<String>,
        default_namespace: impl Into<String>,
        available_namespaces: Vec<String>,
    ) -> Self {
        let default_namespace = default_namespace.into();
        let available_namespaces = sanitize_namespaces(available_namespaces);
        let selected_namespace = if available_namespaces
            .iter()
            .any(|ns| ns == &default_namespace)
        {
            default_namespace
        } else {
            available_namespaces
                .first()
                .cloned()
                .unwrap_or_else(|| "default".to_string())
        };
        let namespace_index = available_namespaces
            .iter()
            .position(|ns| ns == &selected_namespace)
            .unwrap_or(0);
        Self {
            node_name: node_name.into(),
            focus_field: NodeDebugField::Preset,
            selected_preset: DebugImagePreset::Busybox,
            custom_image: String::new(),
            available_namespaces,
            namespace_index,
            profile: NodeDebugProfile::General,
            pending_launch: false,
            pending_launch_action_history_id: None,
            error_message: None,
            notes_scroll: 0,
            custom_image_cursor: 0,
        }
    }

    pub fn selected_namespace(&self) -> &str {
        self.available_namespaces
            .get(self.namespace_index)
            .map(String::as_str)
            .unwrap_or("default")
    }

    pub fn begin_launch(&mut self, action_history_id: u64) {
        self.pending_launch = true;
        self.pending_launch_action_history_id = Some(action_history_id);
        self.error_message = None;
        self.notes_scroll = 0;
    }

    pub fn clear_launch_if_matches(&mut self, action_history_id: u64) {
        if self.pending_launch_action_history_id == Some(action_history_id) {
            self.pending_launch = false;
            self.pending_launch_action_history_id = None;
        }
    }

    pub fn owns_launch_action(&self, action_history_id: u64) -> bool {
        self.pending_launch_action_history_id == Some(action_history_id)
    }

    pub fn build_launch_request(&self) -> Result<NodeDebugLaunchRequest, String> {
        let image = default_debug_image(self.selected_preset, &self.custom_image)
            .ok_or_else(|| "Select a preset image or enter a custom debug image.".to_string())?;
        Ok(NodeDebugLaunchRequest {
            node_name: self.node_name.clone(),
            namespace: self.selected_namespace().to_string(),
            image,
            profile: self.profile,
        })
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> NodeDebugDialogEvent {
        if self.pending_launch {
            return match key.code {
                KeyCode::Esc => NodeDebugDialogEvent::Close,
                _ => NodeDebugDialogEvent::None,
            };
        }

        if self.is_editing_custom_image() {
            match key.code {
                KeyCode::Esc => return NodeDebugDialogEvent::Close,
                KeyCode::Tab | KeyCode::Down if plain_shortcut(key) => {
                    self.error_message = None;
                    self.focus_field = self.focus_field.next();
                    return NodeDebugDialogEvent::None;
                }
                KeyCode::BackTab | KeyCode::Up if plain_shortcut(key) => {
                    self.error_message = None;
                    self.focus_field = self.focus_field.previous();
                    return NodeDebugDialogEvent::None;
                }
                KeyCode::Enter if plain_shortcut(key) => return self.activate_focused(),
                KeyCode::Backspace => {
                    self.delete_custom_image_left();
                    self.error_message = None;
                    return NodeDebugDialogEvent::None;
                }
                KeyCode::Delete => {
                    self.delete_custom_image_right();
                    self.error_message = None;
                    return NodeDebugDialogEvent::None;
                }
                KeyCode::Left => {
                    self.custom_image_cursor = self.custom_image_cursor.saturating_sub(1);
                    return NodeDebugDialogEvent::None;
                }
                KeyCode::Right => {
                    self.custom_image_cursor =
                        (self.custom_image_cursor + 1).min(self.custom_image.chars().count());
                    return NodeDebugDialogEvent::None;
                }
                KeyCode::Home => {
                    self.custom_image_cursor = 0;
                    return NodeDebugDialogEvent::None;
                }
                KeyCode::End => {
                    self.custom_image_cursor = self.custom_image.chars().count();
                    return NodeDebugDialogEvent::None;
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.custom_image.clear();
                    self.custom_image_cursor = 0;
                    self.error_message = None;
                    return NodeDebugDialogEvent::None;
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let byte_idx = self
                        .custom_image
                        .char_indices()
                        .nth(self.custom_image_cursor)
                        .map_or(self.custom_image.len(), |(idx, _)| idx);
                    self.custom_image.insert(byte_idx, c);
                    self.custom_image_cursor += 1;
                    self.error_message = None;
                    return NodeDebugDialogEvent::None;
                }
                _ => {}
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                return NodeDebugDialogEvent::None;
            }
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.notes_scroll = self.notes_scroll.saturating_add(1);
                    return NodeDebugDialogEvent::None;
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.notes_scroll = self.notes_scroll.saturating_sub(1);
                    return NodeDebugDialogEvent::None;
                }
                KeyCode::Char('d') | KeyCode::PageDown => {
                    self.notes_scroll = self.notes_scroll.saturating_add(10);
                    return NodeDebugDialogEvent::None;
                }
                KeyCode::Char('u') | KeyCode::PageUp => {
                    self.notes_scroll = self.notes_scroll.saturating_sub(10);
                    return NodeDebugDialogEvent::None;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Esc => NodeDebugDialogEvent::Close,
            KeyCode::Tab | KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                self.error_message = None;
                self.focus_field = self.focus_field.next();
                NodeDebugDialogEvent::None
            }
            KeyCode::BackTab | KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                self.error_message = None;
                self.focus_field = self.focus_field.previous();
                NodeDebugDialogEvent::None
            }
            KeyCode::Enter if plain_shortcut(key) => self.activate_focused(),
            KeyCode::Char(' ') if plain_shortcut(key) => self.activate_focused(),
            KeyCode::Char('h') | KeyCode::Left if plain_shortcut(key) => {
                self.error_message = None;
                self.adjust_focused(false);
                NodeDebugDialogEvent::None
            }
            KeyCode::Char('l') | KeyCode::Right if plain_shortcut(key) => {
                self.error_message = None;
                self.adjust_focused(true);
                NodeDebugDialogEvent::None
            }
            _ => NodeDebugDialogEvent::None,
        }
    }

    fn activate_focused(&mut self) -> NodeDebugDialogEvent {
        match self.focus_field {
            NodeDebugField::Preset => {
                self.adjust_preset(true);
                NodeDebugDialogEvent::None
            }
            NodeDebugField::CustomImage => NodeDebugDialogEvent::None,
            NodeDebugField::Namespace => {
                self.adjust_namespace(true);
                NodeDebugDialogEvent::None
            }
            NodeDebugField::Profile => {
                self.adjust_profile(true);
                NodeDebugDialogEvent::None
            }
            NodeDebugField::Launch => NodeDebugDialogEvent::Submit,
            NodeDebugField::Cancel => NodeDebugDialogEvent::Close,
        }
    }

    fn adjust_focused(&mut self, forward: bool) {
        match self.focus_field {
            NodeDebugField::Preset => self.adjust_preset(forward),
            NodeDebugField::Namespace => self.adjust_namespace(forward),
            NodeDebugField::Profile => self.adjust_profile(forward),
            NodeDebugField::Launch | NodeDebugField::Cancel => {
                self.focus_field = if self.focus_field == NodeDebugField::Launch {
                    NodeDebugField::Cancel
                } else {
                    NodeDebugField::Launch
                };
            }
            NodeDebugField::CustomImage => {}
        }
    }

    fn adjust_preset(&mut self, forward: bool) {
        let all = DebugImagePreset::ALL;
        let current = all
            .iter()
            .position(|preset| *preset == self.selected_preset)
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % all.len()
        } else {
            current.checked_sub(1).unwrap_or(all.len() - 1)
        };
        self.selected_preset = all[next];
    }

    fn adjust_namespace(&mut self, forward: bool) {
        if self.available_namespaces.is_empty() {
            self.namespace_index = 0;
            return;
        }
        let len = self.available_namespaces.len();
        self.namespace_index = if forward {
            (self.namespace_index + 1) % len
        } else {
            self.namespace_index.checked_sub(1).unwrap_or(len - 1)
        };
    }

    fn adjust_profile(&mut self, forward: bool) {
        let all = NodeDebugProfile::ALL;
        let current = all
            .iter()
            .position(|profile| *profile == self.profile)
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % all.len()
        } else {
            current.checked_sub(1).unwrap_or(all.len() - 1)
        };
        self.profile = all[next];
        self.notes_scroll = 0;
    }

    fn is_editing_custom_image(&self) -> bool {
        self.focus_field == NodeDebugField::CustomImage
            && self.selected_preset == DebugImagePreset::Custom
    }

    fn delete_custom_image_left(&mut self) {
        if self.custom_image_cursor == 0 {
            return;
        }
        if let Some((byte_idx, _)) = self
            .custom_image
            .char_indices()
            .nth(self.custom_image_cursor - 1)
        {
            self.custom_image.remove(byte_idx);
            self.custom_image_cursor = self.custom_image_cursor.saturating_sub(1);
        }
    }

    fn delete_custom_image_right(&mut self) {
        if let Some((byte_idx, _)) = self
            .custom_image
            .char_indices()
            .nth(self.custom_image_cursor)
        {
            self.custom_image.remove(byte_idx);
        }
    }
}

impl NodeDebugField {
    const ORDER: [NodeDebugField; 6] = [
        NodeDebugField::Preset,
        NodeDebugField::CustomImage,
        NodeDebugField::Namespace,
        NodeDebugField::Profile,
        NodeDebugField::Launch,
        NodeDebugField::Cancel,
    ];

    fn next(self) -> Self {
        let index = Self::ORDER
            .iter()
            .position(|field| *field == self)
            .unwrap_or(0);
        Self::ORDER[(index + 1) % Self::ORDER.len()]
    }

    fn previous(self) -> Self {
        let index = Self::ORDER
            .iter()
            .position(|field| *field == self)
            .unwrap_or(0);
        Self::ORDER[index.checked_sub(1).unwrap_or(Self::ORDER.len() - 1)]
    }
}

pub fn render_node_debug_dialog(frame: &mut Frame, area: Rect, state: &NodeDebugDialogState) {
    let popup = centered_rect(72, 62, area);
    frame.render_widget(Clear, popup);
    if use_compact_node_debug_dialog(popup) {
        render_compact_node_debug_dialog(frame, popup, state);
        return;
    }

    let block = Block::default()
        .title(" Node Debug Shell ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(Color::Black));
    frame.render_widget(&block, popup);
    let inner = block.inner(popup);
    let header_lines = wrap_span_groups(
        &[vec![
            Span::styled(
                if state.pending_launch {
                    " launching ".to_string()
                } else {
                    " ready ".to_string()
                },
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ),
            Span::raw(" "),
            Span::styled(
                format!("Node {}", state.node_name),
                Style::default().fg(Color::White),
            ),
        ]],
        inner.width.max(1),
    );
    let footer_lines = wrap_span_groups(
        &[
            vec![
                Span::styled(
                    "[Tab/Shift-Tab] ".to_string(),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled("move".to_string(), Style::default().fg(Color::White)),
            ],
            vec![
                Span::styled("[h/l] ".to_string(), Style::default().fg(Color::Cyan)),
                Span::styled("change".to_string(), Style::default().fg(Color::White)),
            ],
            vec![
                Span::styled("[Enter] ".to_string(), Style::default().fg(Color::Cyan)),
                Span::styled("activate".to_string(), Style::default().fg(Color::White)),
            ],
            vec![
                Span::styled("[Ctrl+j/k] ".to_string(), Style::default().fg(Color::Cyan)),
                Span::styled("notes".to_string(), Style::default().fg(Color::White)),
            ],
            vec![
                Span::styled("[Esc] ".to_string(), Style::default().fg(Color::Cyan)),
                Span::styled("cancel".to_string(), Style::default().fg(Color::White)),
            ],
        ],
        inner.width.max(1),
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(wrapped_line_count(&header_lines, inner.width.max(1)).max(1) as u16),
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(wrapped_line_count(&footer_lines, inner.width.max(1)).max(1) as u16),
        ])
        .split(popup);

    frame.render_widget(
        Paragraph::new(header_lines).alignment(Alignment::Center),
        chunks[0],
    );

    render_field(
        frame,
        chunks[1],
        "Image",
        &format!(
            "{} ({})",
            state.selected_preset.label(),
            state.selected_preset.description()
        ),
        state.focus_field == NodeDebugField::Preset,
        false,
    );
    render_custom_image_field(frame, chunks[2], state);
    render_field(
        frame,
        chunks[3],
        "Namespace",
        state.selected_namespace(),
        state.focus_field == NodeDebugField::Namespace,
        true,
    );
    render_field(
        frame,
        chunks[4],
        "Profile",
        &format!(
            "{} ({})",
            state.profile.label(),
            state.profile.description()
        ),
        state.focus_field == NodeDebugField::Profile,
        true,
    );

    let buttons = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(18), Constraint::Length(18)])
        .split(chunks[5]);
    render_button(
        frame,
        buttons[0],
        "Launch Shell",
        state.focus_field == NodeDebugField::Launch,
        !state.pending_launch,
    );
    render_button(
        frame,
        buttons[1],
        "Cancel",
        state.focus_field == NodeDebugField::Cancel,
        true,
    );

    let warning_style = if state.profile.is_privileged() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Gray)
    };
    let mut notes = vec![
        Line::from(vec![
            Span::styled("• ", warning_style),
            Span::styled(
                "Debug pod runs on the selected node with host PID, network, IPC, and /host mounted.",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("• ", warning_style),
            Span::styled(
                if state.profile.is_privileged() {
                    "Sysadmin profile is privileged. Use it only when the general shell is insufficient."
                } else {
                    "General profile is not privileged. Some host-level operations like chroot may fail."
                },
                warning_style,
            ),
        ]),
        Line::from(vec![
            Span::styled("• ", warning_style),
            Span::styled(
                "KubecTUI will remove the debug pod when the shell session is closed.",
                Style::default().fg(Color::White),
            ),
        ]),
    ];
    if let Some(error) = &state.error_message {
        notes.push(Line::from(vec![
            Span::styled("Error: ", Style::default().fg(Color::Red)),
            Span::styled(error.clone(), Style::default().fg(Color::White)),
        ]));
    }
    let notes_block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));
    let notes_inner = notes_block.inner(chunks[6]);
    frame.render_widget(notes_block, chunks[6]);
    let notes_total = wrapped_line_count(&notes, notes_inner.width);
    let notes_position = state
        .notes_scroll
        .min(notes_total.saturating_sub(notes_inner.height.max(1) as usize));
    frame.render_widget(
        Paragraph::new(notes)
            .wrap(Wrap { trim: true })
            .scroll((notes_position.min(u16::MAX as usize) as u16, 0)),
        notes_inner,
    );
    render_vertical_scrollbar(frame, notes_inner, notes_total, notes_position);

    frame.render_widget(
        Paragraph::new(footer_lines).alignment(Alignment::Center),
        chunks[7],
    );
}

fn use_compact_node_debug_dialog(popup: Rect) -> bool {
    popup.width < 54 || popup.height < 22
}

fn render_compact_node_debug_dialog(frame: &mut Frame, popup: Rect, state: &NodeDebugDialogState) {
    let block = Block::default()
        .title(" Node Debug Shell ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let status = if state.pending_launch {
        "launching"
    } else {
        "ready"
    };
    let image = if state.selected_preset == DebugImagePreset::Custom {
        if state.custom_image.is_empty() {
            "<custom image>"
        } else {
            state.custom_image.as_str()
        }
    } else {
        state.selected_preset.label()
    };
    let note = if let Some(error) = &state.error_message {
        format!("err: {error}")
    } else if state.profile.is_privileged() {
        "sysadmin profile".to_string()
    } else {
        "general profile".to_string()
    };
    let lines = compact_node_debug_lines(state, status, image, &note, inner.width, inner.height);
    frame.render_widget(Paragraph::new(lines), inner);
}

fn compact_node_debug_lines(
    state: &NodeDebugDialogState,
    status: &str,
    image: &str,
    note: &str,
    width: u16,
    height: u16,
) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    if height <= 2 {
        return vec![
            compact_line(format!("node {}  {}", state.node_name, status), width),
            compact_line(
                format!(
                    "ns {}  profile {}",
                    state.selected_namespace(),
                    state.profile.label()
                ),
                width,
            ),
        ];
    }

    if height == 3 {
        return vec![
            compact_line(format!("node {}  {}", state.node_name, status), width),
            compact_line(format!("image {}", image), width),
            compact_line(note, width),
        ];
    }

    if height == 4 {
        return vec![
            compact_line(format!("node {}  {}", state.node_name, status), width),
            compact_line(format!("image {}", image), width),
            compact_line(
                format!(
                    "ns {}  profile {}",
                    state.selected_namespace(),
                    state.profile.label()
                ),
                width,
            ),
            compact_line(note, width),
        ];
    }

    vec![
        compact_line(format!("node {}  {}", state.node_name, status), width),
        compact_line(format!("image {}", image), width),
        compact_line(
            format!(
                "ns {}  profile {}",
                state.selected_namespace(),
                state.profile.label()
            ),
            width,
        ),
        compact_line(note, width),
        compact_line("[Tab] move  [h/l] change  [Enter] launch", width),
    ]
}

fn compact_line(text: impl AsRef<str>, width: usize) -> Line<'static> {
    Line::from(truncate_message(text.as_ref(), width.max(1)).into_owned())
}

fn render_field(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
    editable: bool,
) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let value_style = if editable {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::Gray)
    };
    let block = Block::default()
        .title(format!(" {label} "))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style);
    let value_line = truncate_line_content(
        &Line::from(vec![Span::styled(value.to_string(), value_style)]),
        usize::from(block.inner(area).width.max(1)),
    );
    frame.render_widget(Paragraph::new(value_line).block(block), area);
}

fn render_custom_image_field(frame: &mut Frame, area: Rect, state: &NodeDebugDialogState) {
    let editable = state.selected_preset == DebugImagePreset::Custom;
    let focused = state.focus_field == NodeDebugField::CustomImage;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .title(" Custom Image ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style);
    let inner = block.inner(area);
    let line = if editable {
        cursor_visible_input_line(
            &[],
            &state.custom_image,
            focused.then_some(state.custom_image_cursor),
            Style::default().fg(Color::White),
            Style::default().fg(Color::White),
            &[],
            usize::from(inner.width.max(1)),
        )
    } else {
        truncate_line_content(
            &Line::from(vec![Span::styled(
                "select Custom preset to edit",
                Style::default().fg(Color::Gray),
            )]),
            usize::from(inner.width.max(1)),
        )
    };
    frame.render_widget(Paragraph::new(line).block(block), area);
}

fn render_button(frame: &mut Frame, area: Rect, label: &str, focused: bool, enabled: bool) {
    let style = if !enabled {
        Style::default().fg(Color::DarkGray)
    } else if focused {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default().fg(Color::White)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(format!(" {label} "), style)]))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(if focused {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    }),
            ),
        area,
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn sanitize_namespaces(mut namespaces: Vec<String>) -> Vec<String> {
    namespaces.retain(|ns| !ns.is_empty() && ns != "all");
    namespaces.sort();
    namespaces.dedup();
    if namespaces.is_empty() {
        namespaces.push("default".to_string());
    }
    namespaces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_requested_namespace_without_all() {
        let state = NodeDebugDialogState::new(
            "node-0",
            "ops",
            vec!["all".to_string(), "default".to_string(), "ops".to_string()],
        );
        assert_eq!(state.selected_namespace(), "ops");
    }

    #[test]
    fn build_request_uses_custom_image_and_profile() {
        let mut state = NodeDebugDialogState::new("node-0", "default", vec!["default".to_string()]);
        state.selected_preset = DebugImagePreset::Custom;
        state.custom_image = " ghcr.io/acme/node-debug:1 ".to_string();
        state.profile = NodeDebugProfile::Sysadmin;
        let request = state.build_launch_request().expect("request");
        assert_eq!(request.namespace, "default");
        assert_eq!(request.image, "ghcr.io/acme/node-debug:1");
        assert_eq!(request.profile, NodeDebugProfile::Sysadmin);
    }

    #[test]
    fn custom_image_edit_accepts_hjkl() {
        let mut state = NodeDebugDialogState::new("node-0", "default", vec!["default".to_string()]);
        state.selected_preset = DebugImagePreset::Custom;
        state.focus_field = NodeDebugField::CustomImage;
        state.handle_key(KeyEvent::from(KeyCode::Char('h')));
        state.handle_key(KeyEvent::from(KeyCode::Char('j')));
        state.handle_key(KeyEvent::from(KeyCode::Char('k')));
        state.handle_key(KeyEvent::from(KeyCode::Char('l')));
        assert_eq!(state.custom_image, "hjkl");
    }

    #[test]
    fn falls_back_to_first_real_namespace_when_default_is_missing() {
        let state = NodeDebugDialogState::new(
            "node-0",
            "default",
            vec!["kube-system".to_string(), "ops".to_string()],
        );
        assert_eq!(state.selected_namespace(), "kube-system");
        assert!(!state.available_namespaces.iter().any(|ns| ns == "default"));
    }

    #[test]
    fn render_node_debug_dialog_small_terminal_smoke() {
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal should initialize");
        let state = NodeDebugDialogState::new("node-0", "default", vec!["default".to_string()]);
        terminal
            .draw(|frame| render_node_debug_dialog(frame, frame.area(), &state))
            .expect("compact node debug dialog should render");
    }

    #[test]
    fn compact_node_debug_lines_fit_two_line_body() {
        let state = NodeDebugDialogState::new("node-0", "default", vec!["default".to_string()]);
        let lines = compact_node_debug_lines(&state, "ready", "busybox", "general profile", 24, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].to_string().contains("node node-0"));
    }

    #[test]
    fn ctrl_scroll_updates_notes_scroll() {
        let mut state = NodeDebugDialogState::new("node-0", "default", vec!["default".to_string()]);
        assert_eq!(state.notes_scroll, 0);
        state.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        assert_eq!(state.notes_scroll, 1);
        state.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(state.notes_scroll, 11);
        state.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(state.notes_scroll, 1);
        state.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(state.notes_scroll, 0);
    }

    #[test]
    fn modified_plain_shortcuts_do_not_activate_or_adjust_node_debug_dialog() {
        let mut state = NodeDebugDialogState::new("node-0", "default", vec!["default".to_string()]);
        state.focus_field = NodeDebugField::Launch;

        for (code, modifiers) in [
            (KeyCode::Enter, KeyModifiers::CONTROL),
            (KeyCode::Char(' '), KeyModifiers::CONTROL),
            (KeyCode::Char('h'), KeyModifiers::CONTROL),
            (KeyCode::Char('l'), KeyModifiers::CONTROL),
            (KeyCode::Enter, KeyModifiers::ALT),
            (KeyCode::Char(' '), KeyModifiers::ALT),
            (KeyCode::Char('h'), KeyModifiers::ALT),
            (KeyCode::Char('l'), KeyModifiers::ALT),
        ] {
            assert_eq!(
                state.handle_key(KeyEvent::new(code, modifiers)),
                NodeDebugDialogEvent::None,
                "{code:?} {modifiers:?}"
            );
            assert_eq!(state.focus_field, NodeDebugField::Launch);
        }
    }

    #[test]
    fn ctrl_scroll_shortcuts_do_not_fire_while_editing_custom_image() {
        let mut state = NodeDebugDialogState::new("node-0", "default", vec!["default".to_string()]);
        state.selected_preset = DebugImagePreset::Custom;
        state.focus_field = NodeDebugField::CustomImage;
        state.custom_image = "busybox".to_string();
        state.custom_image_cursor = state.custom_image.len();
        state.notes_scroll = 7;

        state.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        state.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));

        assert_eq!(state.notes_scroll, 7);
        assert_eq!(state.custom_image, "");
    }

    #[test]
    fn pending_launch_still_allows_escape_close() {
        let mut state = NodeDebugDialogState::new("node-0", "default", vec!["default".to_string()]);
        state.begin_launch(41);

        assert_eq!(
            state.handle_key(KeyEvent::from(KeyCode::Esc)),
            NodeDebugDialogEvent::Close
        );
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL)),
            NodeDebugDialogEvent::None
        );
    }

    #[test]
    fn clear_launch_ignores_stale_action() {
        let mut state = NodeDebugDialogState::new("node-0", "default", vec!["default".to_string()]);
        state.begin_launch(41);

        state.clear_launch_if_matches(42);
        assert!(state.pending_launch);
        assert_eq!(state.pending_launch_action_history_id, Some(41));

        state.clear_launch_if_matches(41);
        assert!(!state.pending_launch);
        assert!(state.pending_launch_action_history_id.is_none());
    }

    #[test]
    fn owns_launch_action_matches_only_current_action_id() {
        let mut state = NodeDebugDialogState::new("node-0", "default", vec!["default".to_string()]);
        state.begin_launch(77);

        assert!(state.owns_launch_action(77));
        assert!(!state.owns_launch_action(78));
    }

    #[test]
    fn custom_image_editor_inserts_at_cursor() {
        let mut state = NodeDebugDialogState::new("node-0", "default", vec!["default".to_string()]);
        state.selected_preset = DebugImagePreset::Custom;
        state.focus_field = NodeDebugField::CustomImage;
        state.custom_image = "busybox".to_string();
        state.custom_image_cursor = 4;

        state.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT));

        assert_eq!(state.custom_image, "busyXbox");
    }

    #[test]
    fn render_node_debug_dialog_noncompact_narrow_width_smoke() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal should initialize");
        let state = NodeDebugDialogState::new(
            "node-with-very-long-name",
            "default",
            vec!["default".to_string()],
        );
        terminal
            .draw(|frame| render_node_debug_dialog(frame, frame.area(), &state))
            .expect("non-compact node debug dialog should render");
    }
}
