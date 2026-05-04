//! Port forward dialog and tunnel list UI with enhanced form validation

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Span, Style},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::k8s::portforward::{PortForwardConfig, PortForwardTarget, TunnelState};
use crate::state::port_forward::TunnelRegistry;
use crate::ui::components::{input_field::InputFieldWidget, render_vertical_scrollbar};
use crate::ui::{
    bounded_popup_rect, table_window, truncate_message, wrap_span_groups, wrapped_line_count,
};

fn plain_shortcut(key: KeyEvent) -> bool {
    key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
}

fn ctrl_shortcut(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && key
            .modifiers
            .difference(KeyModifiers::CONTROL | KeyModifiers::SHIFT)
            .is_empty()
}

/// Port forward dialog modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortForwardMode {
    /// Creating a new tunnel (form mode).
    Create,
    /// Viewing and managing active tunnels.
    List,
}

/// Form field focus states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Namespace,
    PodName,
    RemotePort,
    LocalPort,
}

/// Port forward dialog state with integrated tunnel management.
#[derive(Debug, Clone)]
pub struct PortForwardDialog {
    /// Current mode.
    pub mode: PortForwardMode,

    /// Form fields with input widget state.
    pub namespace_field: InputFieldWidget,
    pub pod_name_field: InputFieldWidget,
    pub remote_port_field: InputFieldWidget,
    pub local_port_field: InputFieldWidget,

    /// Current focused field (in create mode).
    pub focus: FormField,

    /// Tunnel registry.
    pub registry: TunnelRegistry,
    /// Selected tunnel index.
    pub selected_tunnel: usize,

    /// Error message.
    pub error: Option<String>,
    /// Success message.
    pub success: Option<String>,
}

/// Actions emitted by port forward dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortForwardAction {
    None,
    Close,
    Create((PortForwardTarget, PortForwardConfig)),
    Stop(String),
    Refresh,
}

impl PortForwardDialog {
    /// Create new dialog with defaults.
    pub fn new() -> Self {
        Self {
            mode: PortForwardMode::Create,
            namespace_field: InputFieldWidget::with_value("default", 63),
            pod_name_field: InputFieldWidget::new(253),
            remote_port_field: InputFieldWidget::new(5),
            local_port_field: InputFieldWidget::with_value("0", 5),
            focus: FormField::PodName,
            registry: TunnelRegistry::new(),
            selected_tunnel: 0,
            error: None,
            success: None,
        }
    }

    /// Create dialog pre-populated with pod info.
    pub fn with_target(namespace: &str, pod: &str, remote_port: u16) -> Self {
        Self {
            mode: PortForwardMode::Create,
            namespace_field: InputFieldWidget::with_value(namespace, 63),
            pod_name_field: InputFieldWidget::with_value(pod, 253),
            remote_port_field: InputFieldWidget::with_value(&remote_port.to_string(), 5),
            local_port_field: InputFieldWidget::with_value("0", 5),
            focus: FormField::LocalPort,
            registry: TunnelRegistry::new(),
            selected_tunnel: 0,
            error: None,
            success: None,
        }
    }

    /// Update tunnel registry.
    pub fn update_registry(&mut self, registry: TunnelRegistry) {
        let selected_id = self
            .registry
            .ordered_tunnels()
            .get(self.selected_tunnel)
            .map(|tunnel| tunnel.id.clone());
        self.registry = registry;
        if let Some(selected_id) = selected_id
            && let Some(index) = self
                .registry
                .ordered_tunnels()
                .iter()
                .position(|tunnel| tunnel.id == selected_id)
        {
            self.selected_tunnel = index;
        } else if !self.registry.is_empty() && self.selected_tunnel >= self.registry.len() {
            self.selected_tunnel = self.registry.len() - 1;
        }
    }

    pub fn set_error_message(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
        self.success = None;
    }

    pub fn set_success_message(&mut self, message: impl Into<String>) {
        self.success = Some(message.into());
        self.error = None;
    }

    /// Handle keyboard input.
    pub fn handle_key(&mut self, key: KeyEvent) -> PortForwardAction {
        match self.mode {
            PortForwardMode::Create => self.handle_create_mode(key),
            PortForwardMode::List => self.handle_list_mode(key),
        }
    }

    fn handle_create_mode(&mut self, key: KeyEvent) -> PortForwardAction {
        match key.code {
            KeyCode::Esc if plain_shortcut(key) => PortForwardAction::Close,
            KeyCode::Tab if plain_shortcut(key) => {
                self.next_field();
                PortForwardAction::None
            }
            KeyCode::BackTab if plain_shortcut(key) => {
                self.prev_field();
                PortForwardAction::None
            }
            KeyCode::F(2) => {
                self.switch_mode(PortForwardMode::List);
                PortForwardAction::None
            }
            KeyCode::Enter if plain_shortcut(key) => match self.validate() {
                Ok((target, config)) => {
                    self.clear_form();
                    self.set_success_message("Creating tunnel...");
                    PortForwardAction::Create((target, config))
                }
                Err(msg) => {
                    self.set_error_message(msg);
                    PortForwardAction::None
                }
            },
            KeyCode::Backspace => {
                self.current_field_mut().backspace_char();
                self.error = None;
                PortForwardAction::None
            }
            KeyCode::Delete => {
                self.current_field_mut().delete_char();
                self.error = None;
                PortForwardAction::None
            }
            KeyCode::Home => {
                self.current_field_mut().cursor_home();
                PortForwardAction::None
            }
            KeyCode::End => {
                self.current_field_mut().cursor_end();
                PortForwardAction::None
            }
            KeyCode::Left => {
                self.current_field_mut().cursor_left();
                PortForwardAction::None
            }
            KeyCode::Right => {
                self.current_field_mut().cursor_right();
                PortForwardAction::None
            }
            KeyCode::Char('u') if ctrl_shortcut(key) => {
                self.current_field_mut().clear();
                self.error = None;
                PortForwardAction::None
            }
            KeyCode::Char(c) if plain_shortcut(key) => {
                // Port fields: only allow digits
                match self.focus {
                    FormField::RemotePort | FormField::LocalPort => {
                        if c.is_ascii_digit() {
                            self.current_field_mut().add_char(c);
                        }
                    }
                    _ => {
                        self.current_field_mut().add_char(c);
                    }
                }
                self.error = None;
                PortForwardAction::None
            }
            _ => PortForwardAction::None,
        }
    }

    fn handle_list_mode(&mut self, key: KeyEvent) -> PortForwardAction {
        match key.code {
            KeyCode::Esc if plain_shortcut(key) => PortForwardAction::Close,
            KeyCode::Char('q') if plain_shortcut(key) => PortForwardAction::Close,
            KeyCode::F(1) => {
                self.switch_mode(PortForwardMode::Create);
                PortForwardAction::None
            }
            KeyCode::Up | KeyCode::Char('k') if plain_shortcut(key) => {
                self.selected_tunnel = self.selected_tunnel.saturating_sub(1);
                PortForwardAction::None
            }
            KeyCode::Down | KeyCode::Char('j') if plain_shortcut(key) => {
                if !self.registry.is_empty() {
                    self.selected_tunnel = (self.selected_tunnel + 1) % self.registry.len();
                }
                PortForwardAction::None
            }
            KeyCode::Char('d') | KeyCode::Delete if plain_shortcut(key) => {
                if let Some(tunnel) = self.get_selected_tunnel() {
                    PortForwardAction::Stop(tunnel.id.clone())
                } else {
                    PortForwardAction::None
                }
            }
            KeyCode::Char('r') if plain_shortcut(key) => PortForwardAction::Refresh,
            KeyCode::F(5) => PortForwardAction::Refresh,
            _ => PortForwardAction::None,
        }
    }

    fn current_field_mut(&mut self) -> &mut InputFieldWidget {
        match self.focus {
            FormField::Namespace => &mut self.namespace_field,
            FormField::PodName => &mut self.pod_name_field,
            FormField::RemotePort => &mut self.remote_port_field,
            FormField::LocalPort => &mut self.local_port_field,
        }
    }

    fn next_field(&mut self) {
        self.focus = match self.focus {
            FormField::Namespace => FormField::PodName,
            FormField::PodName => FormField::RemotePort,
            FormField::RemotePort => FormField::LocalPort,
            FormField::LocalPort => FormField::Namespace,
        };
    }

    fn prev_field(&mut self) {
        self.focus = match self.focus {
            FormField::Namespace => FormField::LocalPort,
            FormField::PodName => FormField::Namespace,
            FormField::RemotePort => FormField::PodName,
            FormField::LocalPort => FormField::RemotePort,
        };
    }

    fn switch_mode(&mut self, mode: PortForwardMode) {
        self.mode = mode;
        self.error = None;
        self.success = None;
    }

    /// Validate form and build configuration.
    fn validate(&mut self) -> Result<(PortForwardTarget, PortForwardConfig), String> {
        // Validate namespace
        self.namespace_field
            .validate_required()
            .map_err(|_| "Namespace is required".to_string())?;

        // Validate pod name
        self.pod_name_field
            .validate_required()
            .map_err(|_| "Pod name is required".to_string())?;

        // Validate remote port
        let remote_port = self
            .remote_port_field
            .validate_port()
            .map_err(|e| format!("Remote port: {}", e))?;

        // Validate local port (0 = auto)
        let local_port = self
            .local_port_field
            .validate_port_optional()
            .map_err(|e| format!("Local port: {}", e))?;

        let target = PortForwardTarget::new(
            &self.namespace_field.value,
            &self.pod_name_field.value,
            remote_port,
        );

        let config = PortForwardConfig {
            local_port,
            bind_address: "127.0.0.1".to_string(),
            timeout_secs: 30,
        };

        Ok((target, config))
    }

    fn clear_form(&mut self) {
        self.pod_name_field.clear();
        self.remote_port_field.clear();
        self.local_port_field.value = "0".to_string();
        self.local_port_field.cursor_pos = 1;
        self.focus = FormField::PodName;
    }

    fn get_selected_tunnel(&self) -> Option<crate::k8s::portforward::PortForwardTunnelInfo> {
        self.registry
            .ordered_tunnels()
            .into_iter()
            .nth(self.selected_tunnel)
            .cloned()
    }

    /// Render the dialog.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        match self.mode {
            PortForwardMode::Create => self.render_create_mode(frame, area),
            PortForwardMode::List => self.render_list_mode(frame, area),
        }
    }

    /// Render inside an existing surface without centering a popup.
    pub fn render_embedded(&self, frame: &mut Frame, area: Rect) {
        match self.mode {
            PortForwardMode::Create => self.render_create_mode_in(frame, area, false),
            PortForwardMode::List => self.render_list_mode_in(frame, area, false),
        }
    }

    fn render_create_mode(&self, frame: &mut Frame, area: Rect) {
        let popup = port_forward_dialog_popup(area);
        self.render_create_mode_in(frame, popup, true);
    }

    fn render_create_mode_in(&self, frame: &mut Frame, popup: Rect, clear: bool) {
        if clear {
            frame.render_widget(Clear, popup);
        }
        if use_compact_port_forward_create_dialog(popup) {
            self.render_compact_create_mode(frame, popup);
            return;
        }

        let block = Block::default()
            .title(" Port Forward: Create ")
            .borders(Borders::ALL);
        frame.render_widget(block.clone(), popup);

        let inner = block.inner(popup);
        let footer_lines = port_forward_footer_lines(
            inner.width,
            &[
                ("[Tab] ", "Next"),
                ("[Enter] ", "Create"),
                ("[F2] ", "List"),
                ("[Esc] ", "Close"),
            ],
        );
        let footer_height = wrapped_line_count(&footer_lines, inner.width.max(1)).max(1) as u16;
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(11),
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(footer_height),
            ])
            .split(inner);

        // Form section
        let form_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(2),
                Constraint::Length(2),
                Constraint::Length(2),
                Constraint::Length(3),
            ])
            .split(chunks[0]);

        self.render_form_field(
            frame,
            form_chunks[0],
            "Namespace",
            &self.namespace_field,
            self.focus == FormField::Namespace,
        );
        self.render_form_field(
            frame,
            form_chunks[1],
            "Pod Name*",
            &self.pod_name_field,
            self.focus == FormField::PodName,
        );
        self.render_form_field(
            frame,
            form_chunks[2],
            "Remote Port*",
            &self.remote_port_field,
            self.focus == FormField::RemotePort,
        );
        self.render_form_field(
            frame,
            form_chunks[3],
            "Local Port (0=auto)",
            &self.local_port_field,
            self.focus == FormField::LocalPort,
        );

        // Messages section
        if let Some(ref error) = self.error {
            let error_text =
                Paragraph::new(format!("✗ {}", error)).style(Style::default().fg(Color::Red));
            frame.render_widget(error_text, chunks[1]);
        }

        // Tunnels summary
        let summary_text = format!("Active Tunnels: {}", self.registry.active_count());
        let summary = Paragraph::new(summary_text)
            .block(Block::default().borders(Borders::ALL).title(" Summary "))
            .style(Style::default().fg(Color::Gray));
        frame.render_widget(summary, chunks[2]);

        // Footer
        let footer = Paragraph::new(footer_lines).style(Style::default().fg(Color::Gray));
        frame.render_widget(footer, chunks[3]);
    }

    fn render_compact_create_mode(&self, frame: &mut Frame, popup: Rect) {
        let block = Block::default()
            .title(" Port Forward: Create ")
            .borders(Borders::ALL);
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let focus = match self.focus {
            FormField::Namespace => "namespace",
            FormField::PodName => "pod",
            FormField::RemotePort => "remote",
            FormField::LocalPort => "local",
        };
        let status = if let Some(error) = &self.error {
            format!("err: {error}")
        } else {
            format!("active tunnels: {}", self.registry.active_count())
        };
        let clamp = |text: String| {
            Line::from(truncate_message(&text, usize::from(inner.width.max(1))).into_owned())
        };
        let lines = vec![
            clamp(format!("ns {}", self.namespace_field.value)),
            clamp(format!("pod {}", self.pod_name_field.value)),
            clamp(format!(
                "remote {}  local {}",
                self.remote_port_field.value, self.local_port_field.value
            )),
            clamp(format!("focus {focus}  {status}")),
            clamp("[Enter] create  [F2] list  [Esc] close".to_string()),
        ];
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn render_form_field(
        &self,
        frame: &mut Frame,
        area: Rect,
        label: &str,
        field: &InputFieldWidget,
        focused: bool,
    ) {
        let mut display_field = field.clone();
        display_field.focused = focused;
        let text = display_field.styled_line(
            &[Span::raw(format!("{label}: "))],
            focused,
            usize::from(area.width.max(1)),
        );

        let paragraph = Paragraph::new(text);
        frame.render_widget(paragraph, area);
    }

    fn render_list_mode(&self, frame: &mut Frame, area: Rect) {
        let popup = port_forward_dialog_popup(area);
        self.render_list_mode_in(frame, popup, true);
    }

    fn render_list_mode_in(&self, frame: &mut Frame, popup: Rect, clear: bool) {
        if clear {
            frame.render_widget(Clear, popup);
        }
        if use_compact_port_forward_list_dialog(popup) {
            self.render_compact_list_mode(frame, popup);
            return;
        }

        let block = Block::default()
            .title(format!(
                " Active Tunnels ({}) ",
                self.registry.active_count()
            ))
            .borders(Borders::ALL);
        frame.render_widget(block.clone(), popup);

        let inner = block.inner(popup);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(10),
                Constraint::Length(
                    wrapped_line_count(
                        &port_forward_footer_lines(
                            inner.width,
                            &[
                                ("[↑↓] ", "Select"),
                                ("[d] ", "Delete"),
                                ("[r] ", "Refresh"),
                                ("[F1] ", "Create"),
                                ("[Esc] ", "Close"),
                            ],
                        ),
                        inner.width.max(1),
                    )
                    .max(1) as u16,
                ),
            ])
            .split(inner);

        if self.registry.is_empty() {
            let message =
                Paragraph::new("No active tunnels").style(Style::default().fg(Color::Gray));
            frame.render_widget(message, chunks[0]);
        } else {
            let mut lines = vec![];
            let tunnels = self.registry.ordered_tunnels();
            let selected = selected_tunnel_index(tunnels.len(), self.selected_tunnel);
            let window = table_window(
                tunnels.len(),
                selected,
                tunnel_list_viewport_rows(chunks[0]),
            );
            for (offset, tunnel) in tunnels[window.start..window.end].iter().enumerate() {
                let idx = window.start + offset;
                let is_selected = idx == selected;

                let state_color = match tunnel.state {
                    TunnelState::Active => Color::Green,
                    TunnelState::Starting => Color::Yellow,
                    TunnelState::Error => Color::Red,
                    _ => Color::Gray,
                };

                let state_indicator = match tunnel.state {
                    TunnelState::Active => "●",
                    TunnelState::Starting => "◐",
                    TunnelState::Error => "✗",
                    _ => "○",
                };

                let content = format!(
                    "{} → localhost:{}",
                    tunnel.target.pod_name,
                    tunnel.local_addr.port()
                );

                let style = if is_selected {
                    Style::default().bg(Color::DarkGray).fg(Color::White)
                } else {
                    Style::default()
                };

                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{} ", state_indicator),
                        Style::default().fg(state_color),
                    ),
                    Span::styled(content, style),
                ]));

                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("{}/{}", tunnel.target.namespace, tunnel.target.pod_name),
                        Style::default().fg(Color::Gray),
                    ),
                ]));
            }

            let tunnels_list = Paragraph::new(lines);
            frame.render_widget(tunnels_list, chunks[0]);
            let (scroll_total, scroll_position) =
                tunnel_scroll_metrics(tunnels.len(), window.start);
            render_vertical_scrollbar(frame, chunks[0], scroll_total, scroll_position);
        }

        // Footer
        let footer = Paragraph::new(port_forward_footer_lines(
            inner.width,
            &[
                ("[↑↓] ", "Select"),
                ("[d] ", "Delete"),
                ("[r] ", "Refresh"),
                ("[F1] ", "Create"),
                ("[Esc] ", "Close"),
            ],
        ))
        .style(Style::default().fg(Color::Gray));
        frame.render_widget(footer, chunks[1]);
    }

    fn render_compact_list_mode(&self, frame: &mut Frame, popup: Rect) {
        let block = Block::default()
            .title(format!(
                " Active Tunnels ({}) ",
                self.registry.active_count()
            ))
            .borders(Borders::ALL);
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        let clamp = |text: String| {
            Line::from(truncate_message(&text, usize::from(inner.width.max(1))).into_owned())
        };

        let lines = if self.registry.is_empty() {
            vec![
                clamp("no active tunnels".to_string()),
                clamp("[F1] create  [Esc] close".to_string()),
            ]
        } else {
            let tunnels = self.registry.ordered_tunnels();
            let selected = self.selected_tunnel.min(tunnels.len().saturating_sub(1));
            let tunnel = tunnels[selected];
            let state = match tunnel.state {
                TunnelState::Active => "active",
                TunnelState::Starting => "starting",
                TunnelState::Error => "error",
                _ => "idle",
            };
            vec![
                clamp(format!("sel {}/{}", selected + 1, tunnels.len())),
                clamp(format!("pod {}", tunnel.target.pod_name)),
                clamp(format!("ns {}  {state}", tunnel.target.namespace)),
                clamp(format!(
                    "local {} -> {}",
                    tunnel.local_addr.port(),
                    tunnel.target.remote_port
                )),
                clamp("[↑↓] move  [d] stop  [Esc] close".to_string()),
            ]
        };
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

impl Default for PortForwardDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn tunnel_list_viewport_rows(area: Rect) -> usize {
    usize::from(area.height.saturating_div(2)).max(1)
}

fn tunnel_scroll_metrics(total_tunnels: usize, offset: usize) -> (usize, usize) {
    if total_tunnels == 0 {
        return (1, 0);
    }

    let clamped_offset = offset.min(total_tunnels.saturating_sub(1));
    (
        total_tunnels.saturating_mul(2),
        clamped_offset.saturating_mul(2),
    )
}

fn port_forward_footer_lines(width: u16, groups: &[(&str, &str)]) -> Vec<Line<'static>> {
    let footer_groups: Vec<Vec<Span<'static>>> = groups
        .iter()
        .map(|(key, label)| {
            vec![
                Span::styled((*key).to_string(), Style::default().fg(Color::Cyan)),
                Span::styled((*label).to_string(), Style::default().fg(Color::Gray)),
            ]
        })
        .collect();
    wrap_span_groups(&footer_groups, width.max(1))
}

fn selected_tunnel_index(total: usize, selected: usize) -> usize {
    if total == 0 {
        0
    } else {
        selected.min(total.saturating_sub(1))
    }
}

fn port_forward_dialog_popup(area: Rect) -> Rect {
    let preferred_width = area.width.saturating_mul(60).saturating_div(100).max(50);
    let preferred_height = area.height.saturating_mul(70).saturating_div(100).max(12);
    bounded_popup_rect(area, preferred_width, preferred_height, 1, 1)
}

fn use_compact_port_forward_create_dialog(popup: Rect) -> bool {
    popup.width < 50 || popup.height < 18
}

fn use_compact_port_forward_list_dialog(popup: Rect) -> bool {
    popup.width < 50 || popup.height < 14
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

    fn make_tunnel(
        id: &str,
        pod: &str,
        state: TunnelState,
        port: u16,
    ) -> crate::k8s::portforward::PortForwardTunnelInfo {
        crate::k8s::portforward::PortForwardTunnelInfo {
            id: id.to_string(),
            target: PortForwardTarget::new("default", pod, 8080),
            local_addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)),
            state,
        }
    }

    fn draw_dialog(dialog: &PortForwardDialog) {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");
        terminal
            .draw(|frame| dialog.render(frame, frame.area()))
            .expect("dialog render should not panic");
    }

    #[test]
    fn test_dialog_new() {
        let dialog = PortForwardDialog::new();
        assert_eq!(dialog.mode, PortForwardMode::Create);
        assert_eq!(dialog.namespace_field.value, "default");
        assert_eq!(dialog.pod_name_field.value, "");
        assert!(dialog.error.is_none());
    }

    #[test]
    fn test_with_target_prefills_and_focuses_local_port() {
        let dialog = PortForwardDialog::with_target("demo", "nginx-1", 8080);
        assert_eq!(dialog.namespace_field.value, "demo");
        assert_eq!(dialog.pod_name_field.value, "nginx-1");
        assert_eq!(dialog.remote_port_field.value, "8080");
        assert_eq!(dialog.local_port_field.value, "0");
        assert_eq!(dialog.focus, FormField::LocalPort);
    }

    #[test]
    fn test_field_navigation() {
        let mut dialog = PortForwardDialog::new();
        assert_eq!(dialog.focus, FormField::PodName);
        dialog.next_field();
        assert_eq!(dialog.focus, FormField::RemotePort);
        dialog.prev_field();
        assert_eq!(dialog.focus, FormField::PodName);
    }

    #[test]
    fn test_validation_success() {
        let mut dialog = PortForwardDialog::new();
        dialog.pod_name_field.value = "test-pod".to_string();
        dialog.remote_port_field.value = "8080".to_string();
        let result = dialog.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_validation_errors_for_missing_required_fields() {
        let mut dialog = PortForwardDialog::new();
        dialog.pod_name_field.value.clear();
        dialog.remote_port_field.value = "8080".to_string();

        let err = dialog.validate().expect_err("missing pod name must fail");
        assert!(err.contains("Pod name is required"));
    }

    #[test]
    fn test_mode_switching() {
        let mut dialog = PortForwardDialog::new();
        assert_eq!(dialog.mode, PortForwardMode::Create);
        dialog.handle_key(KeyEvent::from(KeyCode::F(2)));
        assert_eq!(dialog.mode, PortForwardMode::List);
        assert!(dialog.error.is_none());
        assert!(dialog.success.is_none());
    }

    #[test]
    fn mode_switching_clears_stale_status_messages() {
        let mut dialog = PortForwardDialog::new();
        dialog.error = Some("create error".to_string());
        dialog.success = Some("create ok".to_string());

        dialog.handle_key(KeyEvent::from(KeyCode::F(2)));

        assert_eq!(dialog.mode, PortForwardMode::List);
        assert!(dialog.error.is_none());
        assert!(dialog.success.is_none());

        dialog.error = Some("list error".to_string());
        dialog.success = Some("list ok".to_string());
        dialog.handle_key(KeyEvent::from(KeyCode::F(1)));

        assert_eq!(dialog.mode, PortForwardMode::Create);
        assert!(dialog.error.is_none());
        assert!(dialog.success.is_none());
    }

    #[test]
    fn test_create_mode_enter_emits_create_and_clears_form() {
        let mut dialog = PortForwardDialog::new();
        dialog.pod_name_field.value = "logs-test".to_string();
        dialog.remote_port_field.value = "80".to_string();

        let action = dialog.handle_key(KeyEvent::from(KeyCode::Enter));

        assert!(matches!(action, PortForwardAction::Create(_)));
        assert_eq!(dialog.pod_name_field.value, "");
        assert_eq!(dialog.remote_port_field.value, "");
        assert_eq!(dialog.local_port_field.value, "0");
        assert!(dialog.success.is_some());
    }

    #[test]
    fn modified_enter_does_not_submit_create_form() {
        let mut dialog = PortForwardDialog::new();
        dialog.pod_name_field.value = "logs-test".to_string();
        dialog.remote_port_field.value = "80".to_string();

        for modifiers in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
            assert_eq!(
                dialog.handle_key(KeyEvent::new(KeyCode::Enter, modifiers)),
                PortForwardAction::None
            );
            assert_eq!(dialog.pod_name_field.value, "logs-test");
            assert_eq!(dialog.remote_port_field.value, "80");
            assert!(dialog.success.is_none());
        }
    }

    #[test]
    fn create_submit_clears_stale_error_message() {
        let mut dialog = PortForwardDialog::new();
        dialog.error = Some("previous failure".to_string());
        dialog.pod_name_field.value = "logs-test".to_string();
        dialog.remote_port_field.value = "80".to_string();

        let action = dialog.handle_key(KeyEvent::from(KeyCode::Enter));

        assert!(matches!(action, PortForwardAction::Create(_)));
        assert!(dialog.error.is_none());
        assert_eq!(dialog.success.as_deref(), Some("Creating tunnel..."));
    }

    #[test]
    fn test_digit_filtering_in_port_fields() {
        let mut dialog = PortForwardDialog::new();
        dialog.focus = FormField::RemotePort;
        dialog.handle_key(KeyEvent::from(KeyCode::Char('a')));
        dialog.handle_key(KeyEvent::from(KeyCode::Char('9')));

        assert_eq!(dialog.remote_port_field.value, "9");
    }

    #[test]
    fn modified_chars_do_not_insert_unrelated_chars_into_create_fields() {
        let mut dialog = PortForwardDialog::new();
        dialog.focus = FormField::PodName;
        dialog.pod_name_field.value = "api".to_string();
        dialog.pod_name_field.cursor_pos = dialog.pod_name_field.value.chars().count();

        dialog.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        dialog.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT));
        dialog.handle_key(KeyEvent::new(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ));

        assert_eq!(dialog.pod_name_field.value, "api");
    }

    #[test]
    fn ctrl_u_clears_active_create_field() {
        let mut dialog = PortForwardDialog::new();
        dialog.focus = FormField::PodName;
        dialog.pod_name_field.value = "api".to_string();
        dialog.pod_name_field.cursor_pos = dialog.pod_name_field.value.chars().count();

        dialog.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));

        assert!(dialog.pod_name_field.value.is_empty());
        assert_eq!(dialog.pod_name_field.cursor_pos, 0);
    }

    #[test]
    fn delete_removes_character_at_cursor_in_create_field() {
        let mut dialog = PortForwardDialog::new();
        dialog.focus = FormField::PodName;
        dialog.pod_name_field.value = "abcd".to_string();
        dialog.pod_name_field.cursor_pos = 1;

        dialog.handle_key(KeyEvent::from(KeyCode::Delete));

        assert_eq!(dialog.pod_name_field.value, "acd");
        assert_eq!(dialog.pod_name_field.cursor_pos, 1);
    }

    #[test]
    fn test_list_navigation_and_stop_action() {
        let mut dialog = PortForwardDialog::new();
        dialog.mode = PortForwardMode::List;

        let mut registry = TunnelRegistry::new();
        registry.add_tunnel(make_tunnel("t1", "pod-1", TunnelState::Active, 4001));
        registry.add_tunnel(make_tunnel("t2", "pod-2", TunnelState::Error, 4002));
        dialog.update_registry(registry);

        assert_eq!(dialog.selected_tunnel, 0);

        // Get the first selected tunnel before navigation
        let first_tunnel = dialog
            .get_selected_tunnel()
            .expect("first tunnel should exist");

        // Navigate down
        dialog.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(dialog.selected_tunnel, 1);

        // Get the second selected tunnel
        let second_tunnel = dialog
            .get_selected_tunnel()
            .expect("second tunnel should exist");

        // Verify they're different tunnels
        assert_ne!(first_tunnel.id, second_tunnel.id);

        // Delete action should target the currently selected tunnel (second one)
        let action = dialog.handle_key(KeyEvent::from(KeyCode::Char('d')));
        assert_eq!(action, PortForwardAction::Stop(second_tunnel.id));
    }

    #[test]
    fn modified_plain_list_shortcuts_do_not_navigate_stop_or_refresh() {
        let mut dialog = PortForwardDialog::new();
        dialog.mode = PortForwardMode::List;

        let mut registry = TunnelRegistry::new();
        registry.add_tunnel(make_tunnel("t1", "pod-1", TunnelState::Active, 4001));
        registry.add_tunnel(make_tunnel("t2", "pod-2", TunnelState::Error, 4002));
        dialog.update_registry(registry);

        for (code, modifiers) in [
            (KeyCode::Char('j'), KeyModifiers::CONTROL),
            (KeyCode::Char('k'), KeyModifiers::CONTROL),
            (KeyCode::Char('d'), KeyModifiers::CONTROL),
            (KeyCode::Char('r'), KeyModifiers::CONTROL),
            (KeyCode::Char('q'), KeyModifiers::CONTROL),
            (KeyCode::Char('j'), KeyModifiers::ALT),
            (KeyCode::Char('d'), KeyModifiers::ALT),
            (KeyCode::Char('r'), KeyModifiers::ALT),
            (KeyCode::Char('q'), KeyModifiers::ALT),
        ] {
            assert_eq!(
                dialog.handle_key(KeyEvent::new(code, modifiers)),
                PortForwardAction::None,
                "{code:?} {modifiers:?}"
            );
            assert_eq!(dialog.selected_tunnel, 0);
            assert_eq!(dialog.mode, PortForwardMode::List);
        }

        assert_eq!(
            dialog.handle_key(KeyEvent::from(KeyCode::F(5))),
            PortForwardAction::Refresh
        );
        assert!(matches!(
            dialog.handle_key(KeyEvent::from(KeyCode::Delete)),
            PortForwardAction::Stop(_)
        ));
    }

    #[test]
    fn modified_escape_does_not_close_port_forward_dialog() {
        for modifiers in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::META,
            KeyModifiers::SUPER,
            KeyModifiers::CONTROL | KeyModifiers::META,
            KeyModifiers::CONTROL | KeyModifiers::SUPER,
        ] {
            let mut create_dialog = PortForwardDialog::new();
            assert_eq!(
                create_dialog.handle_key(KeyEvent::new(KeyCode::Esc, modifiers)),
                PortForwardAction::None,
                "{modifiers:?}"
            );

            let mut list_dialog = PortForwardDialog::new();
            list_dialog.mode = PortForwardMode::List;
            assert_eq!(
                list_dialog.handle_key(KeyEvent::new(KeyCode::Esc, modifiers)),
                PortForwardAction::None,
                "{modifiers:?}"
            );
        }
    }

    #[test]
    fn test_update_registry_clamps_selection() {
        let mut dialog = PortForwardDialog::new();
        dialog.selected_tunnel = 5;

        let mut registry = TunnelRegistry::new();
        registry.add_tunnel(make_tunnel("single", "pod", TunnelState::Active, 5001));
        dialog.update_registry(registry);

        assert_eq!(dialog.selected_tunnel, 0);
    }

    #[test]
    fn update_registry_preserves_selected_tunnel_identity() {
        let mut dialog = PortForwardDialog::new();

        let mut initial = TunnelRegistry::new();
        initial.add_tunnel(make_tunnel("b", "pod-b", TunnelState::Active, 5002));
        initial.add_tunnel(make_tunnel("c", "pod-c", TunnelState::Active, 5003));
        dialog.update_registry(initial);
        dialog.selected_tunnel = 1;

        let mut refreshed = TunnelRegistry::new();
        refreshed.add_tunnel(make_tunnel("a", "pod-a", TunnelState::Active, 5001));
        refreshed.add_tunnel(make_tunnel("b", "pod-b", TunnelState::Active, 5002));
        refreshed.add_tunnel(make_tunnel("c", "pod-c", TunnelState::Active, 5003));
        dialog.update_registry(refreshed);

        let selected = dialog
            .get_selected_tunnel()
            .expect("selected tunnel should still exist");
        assert_eq!(selected.id, "c");
    }

    #[test]
    fn status_helpers_keep_single_visible_status() {
        let mut dialog = PortForwardDialog::new();

        dialog.set_error_message("boom");
        assert_eq!(dialog.error.as_deref(), Some("boom"));
        assert!(dialog.success.is_none());

        dialog.set_success_message("ok");
        assert_eq!(dialog.success.as_deref(), Some("ok"));
        assert!(dialog.error.is_none());
    }

    #[test]
    fn render_create_mode_smoke() {
        let mut dialog = PortForwardDialog::new();
        dialog.error = Some("bad input".to_string());
        draw_dialog(&dialog);
    }

    #[test]
    fn render_list_mode_with_tunnels_smoke() {
        let mut dialog = PortForwardDialog::new();
        dialog.mode = PortForwardMode::List;

        let mut registry = TunnelRegistry::new();
        registry.add_tunnel(make_tunnel("t-active", "pod-a", TunnelState::Active, 6001));
        registry.add_tunnel(make_tunnel(
            "t-starting",
            "pod-b",
            TunnelState::Starting,
            6002,
        ));
        registry.add_tunnel(make_tunnel("t-error", "pod-c", TunnelState::Error, 6003));
        dialog.update_registry(registry);

        draw_dialog(&dialog);
    }

    #[test]
    fn render_create_mode_small_terminal_smoke() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");
        let dialog = PortForwardDialog::new();
        terminal
            .draw(|frame| dialog.render(frame, frame.area()))
            .expect("compact create dialog should render");
    }

    #[test]
    fn render_list_mode_small_terminal_smoke() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");
        let mut dialog = PortForwardDialog::new();
        dialog.mode = PortForwardMode::List;

        let mut registry = TunnelRegistry::new();
        registry.add_tunnel(make_tunnel("t-active", "pod-a", TunnelState::Active, 6001));
        dialog.update_registry(registry);

        terminal
            .draw(|frame| dialog.render(frame, frame.area()))
            .expect("compact list dialog should render");
    }

    #[test]
    fn tunnel_list_viewport_rows_counts_two_lines_per_tunnel() {
        let area = Rect::new(0, 0, 60, 9);
        assert_eq!(tunnel_list_viewport_rows(area), 4);
    }

    #[test]
    fn tunnel_list_window_keeps_selected_tunnel_visible() {
        let area = Rect::new(0, 0, 60, 6);
        let window = table_window(10, 8, tunnel_list_viewport_rows(area));
        assert_eq!(window.start, 7);
        assert_eq!(window.end, 10);
        assert_eq!(window.selected, 1);
    }

    #[test]
    fn selected_tunnel_index_clamps_stale_selection() {
        assert_eq!(selected_tunnel_index(0, 9), 0);
        assert_eq!(selected_tunnel_index(2, 9), 1);
        assert_eq!(selected_tunnel_index(2, 1), 1);
    }

    #[test]
    fn tunnel_scroll_metrics_use_visual_row_offsets() {
        assert_eq!(tunnel_scroll_metrics(0, 0), (1, 0));
        assert_eq!(tunnel_scroll_metrics(3, 0), (6, 0));
        assert_eq!(tunnel_scroll_metrics(3, 2), (6, 4));
        assert_eq!(tunnel_scroll_metrics(3, 9), (6, 4));
    }

    #[test]
    fn footer_lines_wrap_when_width_is_tight() {
        let lines = port_forward_footer_lines(
            18,
            &[
                ("[Tab] ", "Next"),
                ("[Enter] ", "Create"),
                ("[F2] ", "List"),
                ("[Esc] ", "Close"),
            ],
        );
        assert!(lines.len() > 1);
    }
}
