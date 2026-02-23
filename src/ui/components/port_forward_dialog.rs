//! Port forward dialog and tunnel list UI with enhanced form validation

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Span, Style},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::k8s::portforward::{PortForwardConfig, PortForwardTarget, TunnelState};
use crate::state::port_forward::TunnelRegistry;
use crate::ui::components::input_field::InputFieldWidget;

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
        self.registry = registry;
        if !self.registry.is_empty() && self.selected_tunnel >= self.registry.len() {
            self.selected_tunnel = self.registry.len() - 1;
        }
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
            KeyCode::Esc => PortForwardAction::Close,
            KeyCode::Tab => {
                self.next_field();
                PortForwardAction::None
            }
            KeyCode::BackTab => {
                self.prev_field();
                PortForwardAction::None
            }
            KeyCode::F(2) => {
                self.mode = PortForwardMode::List;
                PortForwardAction::None
            }
            KeyCode::Enter => match self.validate() {
                Ok((target, config)) => {
                    self.clear_form();
                    self.success = Some("Creating tunnel...".to_string());
                    PortForwardAction::Create((target, config))
                }
                Err(msg) => {
                    self.error = Some(msg);
                    PortForwardAction::None
                }
            },
            KeyCode::Backspace => {
                self.current_field_mut().delete_char();
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
            KeyCode::Char(c) => {
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
            KeyCode::Esc | KeyCode::Char('q') => PortForwardAction::Close,
            KeyCode::F(1) => {
                self.mode = PortForwardMode::Create;
                PortForwardAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected_tunnel = self.selected_tunnel.saturating_sub(1);
                PortForwardAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.registry.is_empty() {
                    self.selected_tunnel = (self.selected_tunnel + 1) % self.registry.len();
                }
                PortForwardAction::None
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(tunnel) = self.get_selected_tunnel() {
                    PortForwardAction::Stop(tunnel.id.clone())
                } else {
                    PortForwardAction::None
                }
            }
            KeyCode::Char('r') | KeyCode::F(5) => PortForwardAction::Refresh,
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
            .tunnels()
            .values()
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

    fn render_create_mode(&self, frame: &mut Frame, area: Rect) {
        let popup = centered_rect(60, 70, area);
        frame.render_widget(Clear, popup);

        let block = Block::default()
            .title(" Port Forward: Create ")
            .borders(Borders::ALL);
        frame.render_widget(block.clone(), popup);

        let inner = block.inner(popup);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(11),
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(2),
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
        let footer = Paragraph::new("[Tab] Next │ [Enter] Create │ [F2] List │ [Esc] Close")
            .style(Style::default().fg(Color::Gray));
        frame.render_widget(footer, chunks[3]);
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
        let styled = display_field.styled_text(focused);

        let text = Line::from(vec![Span::raw(format!("{}: ", label)), styled]);

        let paragraph = Paragraph::new(text);
        frame.render_widget(paragraph, area);
    }

    fn render_list_mode(&self, frame: &mut Frame, area: Rect) {
        let popup = centered_rect(60, 70, area);
        frame.render_widget(Clear, popup);

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
            .constraints([Constraint::Min(10), Constraint::Length(2)])
            .split(inner);

        if self.registry.is_empty() {
            let message =
                Paragraph::new("No active tunnels").style(Style::default().fg(Color::Gray));
            frame.render_widget(message, chunks[0]);
        } else {
            let mut lines = vec![];
            for (idx, tunnel) in self.registry.tunnels().values().enumerate() {
                let is_selected = idx == self.selected_tunnel;

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
        }

        // Footer
        let footer =
            Paragraph::new("[↑↓] Select │ [d] Delete │ [r] Refresh │ [F1] Create │ [Esc] Close")
                .style(Style::default().fg(Color::Gray));
        frame.render_widget(footer, chunks[1]);
    }
}

impl Default for PortForwardDialog {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to create centered rectangle.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
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
    fn test_digit_filtering_in_port_fields() {
        let mut dialog = PortForwardDialog::new();
        dialog.focus = FormField::RemotePort;
        dialog.handle_key(KeyEvent::from(KeyCode::Char('a')));
        dialog.handle_key(KeyEvent::from(KeyCode::Char('9')));

        assert_eq!(dialog.remote_port_field.value, "9");
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
    fn test_update_registry_clamps_selection() {
        let mut dialog = PortForwardDialog::new();
        dialog.selected_tunnel = 5;

        let mut registry = TunnelRegistry::new();
        registry.add_tunnel(make_tunnel("single", "pod", TunnelState::Active, 5001));
        dialog.update_registry(registry);

        assert_eq!(dialog.selected_tunnel, 0);
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
}
