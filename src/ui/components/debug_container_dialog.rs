//! Ephemeral debug container dialog for Pod detail actions.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Span, Style},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use crate::k8s::exec::{DebugContainerLaunchRequest, DebugImagePreset};
use crate::ui::components::render_vertical_scrollbar;
use crate::ui::{truncate_message, wrap_span_groups, wrapped_line_count};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugContainerField {
    Preset,
    CustomImage,
    TargetMode,
    TargetContainer,
    Launch,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugContainerDialogEvent {
    None,
    Submit,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugContainerDialogState {
    pub pod_name: String,
    pub namespace: String,
    pub focus_field: DebugContainerField,
    pub selected_preset: DebugImagePreset,
    pub custom_image: String,
    pub use_target_container: bool,
    pub target_containers: Vec<String>,
    pub target_index: usize,
    pub loading_targets: bool,
    pub pending_request_id: Option<u64>,
    pub pending_launch: bool,
    pub error_message: Option<String>,
    pub body_scroll: usize,
}

impl DebugContainerDialogState {
    pub fn new(pod_name: impl Into<String>, namespace: impl Into<String>) -> Self {
        Self {
            pod_name: pod_name.into(),
            namespace: namespace.into(),
            focus_field: DebugContainerField::Preset,
            selected_preset: DebugImagePreset::default(),
            custom_image: String::new(),
            use_target_container: false,
            target_containers: Vec::new(),
            target_index: 0,
            loading_targets: true,
            pending_request_id: None,
            pending_launch: false,
            error_message: None,
            body_scroll: 0,
        }
    }

    pub fn set_target_containers(&mut self, target_containers: Vec<String>) {
        self.target_containers = target_containers;
        self.target_index = self
            .target_index
            .min(self.target_containers.len().saturating_sub(1));
        self.loading_targets = false;
        self.pending_request_id = None;
        if self.target_containers.is_empty() {
            self.use_target_container = false;
        }
        self.body_scroll = 0;
    }

    pub fn set_target_fetch_error(&mut self, error: impl Into<String>) {
        self.loading_targets = false;
        self.pending_request_id = None;
        self.use_target_container = false;
        self.error_message = Some(error.into());
        self.body_scroll = 0;
    }

    pub fn set_pending_launch(&mut self, pending: bool) {
        self.pending_launch = pending;
        if pending {
            self.error_message = None;
        }
        self.body_scroll = 0;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> DebugContainerDialogEvent {
        if self.pending_launch {
            return DebugContainerDialogEvent::None;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.body_scroll = self.body_scroll.saturating_add(1);
                    return DebugContainerDialogEvent::None;
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.body_scroll = self.body_scroll.saturating_sub(1);
                    return DebugContainerDialogEvent::None;
                }
                KeyCode::Char('d') | KeyCode::PageDown => {
                    self.body_scroll = self.body_scroll.saturating_add(10);
                    return DebugContainerDialogEvent::None;
                }
                KeyCode::Char('u') | KeyCode::PageUp => {
                    self.body_scroll = self.body_scroll.saturating_sub(10);
                    return DebugContainerDialogEvent::None;
                }
                _ => {}
            }
        }

        if self.is_editing_custom_image() {
            match key.code {
                KeyCode::Esc => return DebugContainerDialogEvent::Close,
                KeyCode::Tab | KeyCode::Down => {
                    self.error_message = None;
                    self.focus_field = self.focus_field.next();
                    return DebugContainerDialogEvent::None;
                }
                KeyCode::BackTab | KeyCode::Up => {
                    self.error_message = None;
                    self.focus_field = self.focus_field.previous();
                    return DebugContainerDialogEvent::None;
                }
                KeyCode::Enter => return self.activate_focused(),
                KeyCode::Backspace => {
                    self.custom_image.pop();
                    self.error_message = None;
                    return DebugContainerDialogEvent::None;
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.custom_image.push(c);
                    self.error_message = None;
                    return DebugContainerDialogEvent::None;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Esc => DebugContainerDialogEvent::Close,
            KeyCode::Tab | KeyCode::Char('j') | KeyCode::Down => {
                self.error_message = None;
                self.focus_field = self.focus_field.next();
                DebugContainerDialogEvent::None
            }
            KeyCode::BackTab | KeyCode::Char('k') | KeyCode::Up => {
                self.error_message = None;
                self.focus_field = self.focus_field.previous();
                DebugContainerDialogEvent::None
            }
            KeyCode::Enter => self.activate_focused(),
            KeyCode::Char(' ') => self.handle_space(),
            KeyCode::Char('h') | KeyCode::Left => {
                self.error_message = None;
                self.adjust_focused(false);
                DebugContainerDialogEvent::None
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.error_message = None;
                self.adjust_focused(true);
                DebugContainerDialogEvent::None
            }
            _ => DebugContainerDialogEvent::None,
        }
    }

    pub fn build_launch_request(&self) -> Result<DebugContainerLaunchRequest, String> {
        let image = self
            .selected_image()
            .ok_or_else(|| "Select a preset image or enter a custom debug image.".to_string())?;
        if self.loading_targets {
            return Err("Container metadata is still loading. Try again in a moment.".to_string());
        }
        if self.use_target_container && self.selected_target_container().is_none() {
            return Err(
                "Process-targeting is enabled, but no Pod container is available to target."
                    .to_string(),
            );
        }

        Ok(DebugContainerLaunchRequest {
            pod_name: self.pod_name.clone(),
            namespace: self.namespace.clone(),
            image,
            target_container_name: self
                .use_target_container
                .then(|| self.selected_target_container().cloned())
                .flatten(),
        })
    }

    fn selected_image(&self) -> Option<String> {
        match self.selected_preset {
            DebugImagePreset::Custom => {
                let trimmed = self.custom_image.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            }
            preset => preset.default_image().map(str::to_string),
        }
    }

    fn selected_target_container(&self) -> Option<&String> {
        self.target_containers.get(self.target_index)
    }

    fn is_editing_custom_image(&self) -> bool {
        self.focus_field == DebugContainerField::CustomImage
            && self.selected_preset == DebugImagePreset::Custom
    }

    fn activate_focused(&mut self) -> DebugContainerDialogEvent {
        match self.focus_field {
            DebugContainerField::Preset => {
                self.adjust_preset(true);
                DebugContainerDialogEvent::None
            }
            DebugContainerField::CustomImage => DebugContainerDialogEvent::None,
            DebugContainerField::TargetMode => {
                self.toggle_target_container();
                DebugContainerDialogEvent::None
            }
            DebugContainerField::TargetContainer => {
                self.adjust_target_container(true);
                DebugContainerDialogEvent::None
            }
            DebugContainerField::Launch => DebugContainerDialogEvent::Submit,
            DebugContainerField::Cancel => DebugContainerDialogEvent::Close,
        }
    }

    fn handle_space(&mut self) -> DebugContainerDialogEvent {
        if self.focus_field == DebugContainerField::TargetMode {
            self.toggle_target_container();
        }
        DebugContainerDialogEvent::None
    }

    fn adjust_focused(&mut self, forward: bool) {
        match self.focus_field {
            DebugContainerField::Preset => self.adjust_preset(forward),
            DebugContainerField::TargetMode => self.toggle_target_container(),
            DebugContainerField::TargetContainer => self.adjust_target_container(forward),
            DebugContainerField::Launch | DebugContainerField::Cancel => {
                self.focus_field = if self.focus_field == DebugContainerField::Launch {
                    DebugContainerField::Cancel
                } else {
                    DebugContainerField::Launch
                };
            }
            DebugContainerField::CustomImage => {}
        }
    }

    fn adjust_preset(&mut self, forward: bool) {
        let all = DebugImagePreset::ALL;
        let current = all
            .iter()
            .position(|preset| preset == &self.selected_preset)
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % all.len()
        } else {
            current.checked_sub(1).unwrap_or(all.len() - 1)
        };
        self.selected_preset = all[next];
    }

    fn adjust_target_container(&mut self, forward: bool) {
        if self.target_containers.is_empty() {
            return;
        }

        let len = self.target_containers.len();
        self.target_index = if forward {
            (self.target_index + 1) % len
        } else {
            self.target_index.checked_sub(1).unwrap_or(len - 1)
        };
    }

    fn toggle_target_container(&mut self) {
        if self.target_containers.is_empty() || self.loading_targets {
            self.use_target_container = false;
            return;
        }
        self.use_target_container = !self.use_target_container;
    }
}

impl DebugContainerField {
    const ORDER: [DebugContainerField; 6] = [
        DebugContainerField::Preset,
        DebugContainerField::CustomImage,
        DebugContainerField::TargetMode,
        DebugContainerField::TargetContainer,
        DebugContainerField::Launch,
        DebugContainerField::Cancel,
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

pub fn render_debug_container_dialog(
    frame: &mut Frame,
    area: Rect,
    state: &DebugContainerDialogState,
) {
    let popup = centered_rect(72, 60, area);
    frame.render_widget(Clear, popup);
    if use_compact_debug_container_dialog(popup) {
        render_compact_debug_container_dialog(frame, popup, state);
        return;
    }

    let block = Block::default()
        .title(" Debug Container ")
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
                } else if state.loading_targets {
                    " loading ".to_string()
                } else {
                    " ready ".to_string()
                },
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ),
            Span::raw(" "),
            Span::styled(
                format!("Pod {} / {}", state.namespace, state.pod_name),
                Style::default().fg(Color::White),
            ),
        ]],
        inner.width.max(1),
    );
    let footer_lines = wrap_span_groups(
        &[
            vec![
                Span::styled("[Enter] ".to_string(), button_style(true)),
                Span::styled("Launch".to_string(), Style::default().fg(Color::White)),
            ],
            vec![
                Span::styled("[Ctrl+j/k] ".to_string(), Style::default().fg(Color::Cyan)),
                Span::styled("body".to_string(), Style::default().fg(Color::White)),
            ],
            vec![
                Span::styled("[Esc] ".to_string(), button_style(false)),
                Span::styled("Cancel".to_string(), Style::default().fg(Color::White)),
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
            Constraint::Min(4),
            Constraint::Length(wrapped_line_count(&footer_lines, inner.width.max(1)).max(1) as u16),
        ])
        .split(popup);

    frame.render_widget(
        Paragraph::new(header_lines).alignment(Alignment::Center),
        chunks[0],
    );

    let preset = state.selected_preset;
    let preset_line = format!(
        "{} Image preset: {} ({})",
        focus_marker(state.focus_field == DebugContainerField::Preset),
        preset.label(),
        preset.description(),
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                preset_line,
                focused_style(state.focus_field == DebugContainerField::Preset),
            )),
            Line::from(Span::styled(
                "  Use h/l or Left/Right to change the preset.",
                Style::default().fg(Color::DarkGray),
            )),
        ]),
        chunks[1],
    );

    let custom_style = focused_style(state.focus_field == DebugContainerField::CustomImage);
    let custom_value = if state.selected_preset == DebugImagePreset::Custom {
        if state.custom_image.is_empty() {
            "<enter image reference>"
        } else {
            state.custom_image.as_str()
        }
    } else {
        "Only used when the Custom preset is selected."
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(
                    "{} Custom image: ",
                    focus_marker(state.focus_field == DebugContainerField::CustomImage)
                ),
                custom_style,
            ),
            Span::styled(custom_value, custom_value_style(state)),
        ])),
        chunks[2],
    );

    let target_mode = if state.use_target_container {
        "on"
    } else {
        "off"
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(
                    "{} Process target: ",
                    focus_marker(state.focus_field == DebugContainerField::TargetMode)
                ),
                focused_style(state.focus_field == DebugContainerField::TargetMode),
            ),
            Span::styled(
                target_mode,
                Style::default().fg(if state.use_target_container {
                    Color::Green
                } else {
                    Color::Yellow
                }),
            ),
            Span::styled(
                "  (targets a Pod container PID namespace when the runtime supports it)",
                Style::default().fg(Color::DarkGray),
            ),
        ])),
        chunks[3],
    );

    let target_line = if state.loading_targets {
        "Loading Pod containers...".to_string()
    } else if let Some(target) = state.selected_target_container() {
        format!("Selected target container: {target}")
    } else {
        "No target container available.".to_string()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(
                    "{} Target container: ",
                    focus_marker(state.focus_field == DebugContainerField::TargetContainer)
                ),
                focused_style(state.focus_field == DebugContainerField::TargetContainer),
            ),
            Span::styled(target_line, Style::default().fg(Color::White)),
        ])),
        chunks[4],
    );

    let resolved_image = state
        .selected_image()
        .unwrap_or_else(|| "<missing image>".to_string());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Launch image: ", Style::default().fg(Color::DarkGray)),
            Span::styled(resolved_image, Style::default().fg(Color::Cyan)),
        ])),
        chunks[5],
    );

    let mut body = vec![Line::from(Span::styled(
        " Ephemeral containers stay with the Pod until the Pod is recreated or restarted.",
        Style::default().fg(Color::Yellow),
    ))];
    if let Some(error) = &state.error_message {
        body.push(Line::from(Span::styled(
            format!(" Error: {error}"),
            Style::default().fg(Color::Red),
        )));
    } else {
        body.push(Line::from(Span::styled(
            " This launcher uses a keepalive shell loop, then opens the existing exec workbench against the new container.",
            Style::default().fg(Color::DarkGray),
        )));
    }
    let body_total = wrapped_line_count(&body, chunks[6].width);
    let body_position = state
        .body_scroll
        .min(body_total.saturating_sub(chunks[6].height.max(1) as usize));
    frame.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .scroll((body_position.min(u16::MAX as usize) as u16, 0)),
        chunks[6],
    );
    render_vertical_scrollbar(frame, chunks[6], body_total, body_position);

    frame.render_widget(
        Paragraph::new(footer_lines).alignment(Alignment::Center),
        chunks[7],
    );
}

fn use_compact_debug_container_dialog(popup: Rect) -> bool {
    popup.width < 54 || popup.height < 22
}

fn render_compact_debug_container_dialog(
    frame: &mut Frame,
    popup: Rect,
    state: &DebugContainerDialogState,
) {
    let block = Block::default()
        .title(" Debug Container ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let status = if state.pending_launch {
        "launching"
    } else if state.loading_targets {
        "loading"
    } else {
        "ready"
    };
    let image = state
        .selected_image()
        .unwrap_or_else(|| "<missing image>".to_string());
    let target = if state.use_target_container {
        state
            .selected_target_container()
            .cloned()
            .unwrap_or_else(|| "no target".to_string())
    } else {
        "off".to_string()
    };
    let note = if let Some(error) = &state.error_message {
        format!("err: {error}")
    } else {
        "enter launch  esc cancel".to_string()
    };
    let lines = compact_debug_container_lines(
        state,
        status,
        &image,
        &target,
        &note,
        inner.width,
        inner.height,
    );
    frame.render_widget(Paragraph::new(lines), inner);
}

fn compact_debug_container_lines(
    state: &DebugContainerDialogState,
    status: &str,
    image: &str,
    target: &str,
    note: &str,
    width: u16,
    height: u16,
) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    if height <= 2 {
        return vec![
            compact_line(
                format!("pod {}/{}  {}", state.namespace, state.pod_name, status),
                width,
            ),
            compact_line(format!("target {}", target), width),
        ];
    }

    if height == 3 {
        return vec![
            compact_line(
                format!("pod {}/{}  {}", state.namespace, state.pod_name, status),
                width,
            ),
            compact_line(format!("image {}", image), width),
            compact_line(note, width),
        ];
    }

    if height == 4 {
        return vec![
            compact_line(
                format!("pod {}/{}  {}", state.namespace, state.pod_name, status),
                width,
            ),
            compact_line(format!("image {}", image), width),
            compact_line(format!("target {}", target), width),
            compact_line(note, width),
        ];
    }

    vec![
        compact_line(
            format!("pod {}/{}  {}", state.namespace, state.pod_name, status),
            width,
        ),
        compact_line(format!("image {}", image), width),
        compact_line(format!("target {}", target), width),
        compact_line(note, width),
        compact_line("[Tab] move  [h/l] change  [Space] toggle", width),
    ]
}

fn compact_line(text: impl AsRef<str>, width: usize) -> Line<'static> {
    Line::from(truncate_message(text.as_ref(), width.max(1)).into_owned())
}

fn focused_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::White)
    }
}

fn custom_value_style(state: &DebugContainerDialogState) -> Style {
    if state.selected_preset != DebugImagePreset::Custom {
        Style::default().fg(Color::DarkGray)
    } else if state.custom_image.trim().is_empty() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    }
}

fn button_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default().fg(Color::Black).bg(Color::DarkGray)
    }
}

fn focus_marker(focused: bool) -> &'static str {
    if focused { ">" } else { " " }
}

fn centered_rect(width_percent: u16, height_percent: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100 - height_percent) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_uses_preset_image_and_target() {
        let mut state = DebugContainerDialogState::new("api-0", "default");
        state.loading_targets = false;
        state.target_containers = vec!["app".to_string()];
        state.use_target_container = true;

        let request = state.build_launch_request().expect("request");
        assert_eq!(request.image, "busybox:1.37");
        assert_eq!(request.target_container_name.as_deref(), Some("app"));
    }

    #[test]
    fn build_request_requires_custom_image_value() {
        let mut state = DebugContainerDialogState::new("api-0", "default");
        state.selected_preset = DebugImagePreset::Custom;
        state.loading_targets = false;

        let error = state.build_launch_request().expect_err("validation error");
        assert!(error.contains("Select a preset image"));
    }

    #[test]
    fn handle_key_cycles_presets() {
        let mut state = DebugContainerDialogState::new("api-0", "default");
        assert_eq!(state.selected_preset, DebugImagePreset::Busybox);

        let event = state.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(event, DebugContainerDialogEvent::None);
        assert_eq!(state.selected_preset, DebugImagePreset::Netshoot);
    }

    #[test]
    fn custom_image_input_accepts_navigation_letters() {
        let mut state = DebugContainerDialogState::new("api-0", "default");
        state.selected_preset = DebugImagePreset::Custom;
        state.focus_field = DebugContainerField::CustomImage;

        for ch in ['g', 'h', 'j', 'k', 'l'] {
            let event = state.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            assert_eq!(event, DebugContainerDialogEvent::None);
        }

        assert_eq!(state.custom_image, "ghjkl");
        assert_eq!(state.focus_field, DebugContainerField::CustomImage);
    }

    #[test]
    fn render_debug_container_dialog_small_terminal_smoke() {
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal should initialize");
        let state = DebugContainerDialogState::new("api-0", "default");
        terminal
            .draw(|frame| render_debug_container_dialog(frame, frame.area(), &state))
            .expect("compact debug container dialog should render");
    }

    #[test]
    fn compact_debug_container_lines_fit_two_line_body() {
        let state = DebugContainerDialogState::new("api-0", "default");
        let lines =
            compact_debug_container_lines(&state, "ready", "busybox", "off", "enter launch", 24, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].to_string().contains("target off"));
    }

    #[test]
    fn ctrl_scroll_updates_body_scroll() {
        let mut state = DebugContainerDialogState::new("api-0", "default");
        assert_eq!(state.body_scroll, 0);
        state.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        assert_eq!(state.body_scroll, 1);
        state.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(state.body_scroll, 11);
        state.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(state.body_scroll, 1);
        state.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(state.body_scroll, 0);
    }

    #[test]
    fn render_debug_container_dialog_noncompact_narrow_width_smoke() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal should initialize");
        let mut state = DebugContainerDialogState::new("pod-with-very-long-name", "default");
        state.loading_targets = false;
        state.target_containers = vec!["main".to_string()];
        terminal
            .draw(|frame| render_debug_container_dialog(frame, frame.area(), &state))
            .expect("non-compact debug container dialog should render");
    }
}
