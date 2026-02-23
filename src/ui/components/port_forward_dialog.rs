//! Port forward dialog and tunnel list UI

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use crossterm::event::{KeyCode, KeyEvent};

use crate::k8s::portforward::{PortForwardConfig, PortForwardTarget, TunnelState};
use crate::state::port_forward::TunnelRegistry;

/// Port forward dialog state
#[derive(Debug)]
pub struct PortForwardDialog {
    /// Target namespace
    pub namespace_input: String,
    /// Target pod name
    pub pod_input: String,
    /// Remote port input
    pub remote_port_input: String,
    /// Local port input (empty = auto)
    pub local_port_input: String,
    /// Current field focus
    pub focus: InputField,
    /// Error message
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputField {
    Namespace,
    Pod,
    RemotePort,
    LocalPort,
}

impl PortForwardDialog {
    pub fn new() -> Self {
        Self {
            namespace_input: "default".to_string(),
            pod_input: String::new(),
            remote_port_input: String::new(),
            local_port_input: String::new(),
            focus: InputField::Pod,
            error: None,
        }
    }

    pub fn with_target(namespace: &str, pod: &str, remote_port: u16) -> Self {
        Self {
            namespace_input: namespace.to_string(),
            pod_input: pod.to_string(),
            remote_port_input: remote_port.to_string(),
            local_port_input: String::new(),
            focus: InputField::LocalPort,
            error: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PortForwardAction {
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
            KeyCode::Enter => {
                if self.validate() {
                    PortForwardAction::Create(self.build_config())
                } else {
                    PortForwardAction::None
                }
            }
            KeyCode::Backspace => {
                self.current_input_mut().pop();
                PortForwardAction::None
            }
            KeyCode::Char(c) => {
                self.current_input_mut().push(c);
                PortForwardAction::None
            }
            _ => PortForwardAction::None,
        }
    }

    fn validate(&mut self) -> bool {
        if self.pod_input.is_empty() {
            self.error = Some("Pod name is required".to_string());
            return false;
        }
        if self.remote_port_input.parse::<u16>().is_err() {
            self.error = Some("Invalid remote port".to_string());
            return false;
        }
        if !self.local_port_input.is_empty() && self.local_port_input.parse::<u16>().is_err() {
            self.error = Some("Invalid local port".to_string());
            return false;
        }
        self.error = None;
        true
    }

    fn build_config(&self) -> (PortForwardTarget, PortForwardConfig) {
        let target = PortForwardTarget::new(
            &self.namespace_input,
            &self.pod_input,
            self.remote_port_input.parse().unwrap(),
        );

        let config = PortForwardConfig {
            local_port: self.local_port_input.parse().unwrap_or(0),
            bind_address: "127.0.0.1".to_string(),
            timeout_secs: 30,
        };

        (target, config)
    }

    fn current_input_mut(&mut self) -> &mut String {
        match self.focus {
            InputField::Namespace => &mut self.namespace_input,
            InputField::Pod => &mut self.pod_input,
            InputField::RemotePort => &mut self.remote_port_input,
            InputField::LocalPort => &mut self.local_port_input,
        }
    }

    fn next_field(&mut self) {
        self.focus = match self.focus {
            InputField::Namespace => InputField::Pod,
            InputField::Pod => InputField::RemotePort,
            InputField::RemotePort => InputField::LocalPort,
            InputField::LocalPort => InputField::Namespace,
        };
    }

    fn prev_field(&mut self) {
        self.focus = match self.focus {
            InputField::Namespace => InputField::LocalPort,
            InputField::Pod => InputField::Namespace,
            InputField::RemotePort => InputField::Pod,
            InputField::LocalPort => InputField::RemotePort,
        };
    }

    pub fn render(&self, f: &mut ratatui::Frame, area: Rect) {
        let block = Block::default()
            .title("Port Forward")
            .borders(Borders::ALL);

        let inner = block.inner(area);
        f.render_widget(Clear, area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(2),
                Constraint::Min(1),
            ])
            .split(inner);

        // Render inputs
        self.render_input(f, chunks[0], "Namespace", &self.namespace_input, self.focus == InputField::Namespace);
        self.render_input(f, chunks[1], "Pod Name*", &self.pod_input, self.focus == InputField::Pod);
        self.render_input(f, chunks[2], "Remote Port*", &self.remote_port_input, self.focus == InputField::RemotePort);
        self.render_input(f, chunks[3], "Local Port (0=auto)", &self.local_port_input, self.focus == InputField::LocalPort);

        // Error message
        if let Some(ref error) = self.error {
            let error_text = Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red));
            f.render_widget(error_text, chunks[4]);
        }

        // Help text
        let help = Paragraph::new("Tab: Next Field | Enter: Create | Esc: Cancel")
            .style(Style::default().fg(Color::Gray));
        f.render_widget(help, chunks[5]);
    }

    fn render_input(
        &self,
        f: &mut ratatui::Frame,
        area: Rect,
        label: &str,
        value: &str,
        focused: bool,
    ) {
        let style = if focused {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let input_text = if focused {
            format!("{}: {}█", label, value)
        } else {
            format!("{}: {}", label, value)
        };

        let paragraph = Paragraph::new(input_text).style(style);
        f.render_widget(paragraph, area);
    }
}

/// Tunnel list panel
#[derive(Debug)]
pub struct TunnelListPanel {
    pub registry: TunnelRegistry,
}

impl TunnelListPanel {
    pub fn new(registry: TunnelRegistry) -> Self {
        Self { registry }
    }

    pub fn update_registry(&mut self, registry: TunnelRegistry) {
        self.registry = registry;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PortForwardAction {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.registry.select_prev();
                PortForwardAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.registry.select_next();
                PortForwardAction::None
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(tunnel) = self.registry.selected() {
                    PortForwardAction::Stop(tunnel.id.clone())
                } else {
                    PortForwardAction::None
                }
            }
            KeyCode::Char('q') | KeyCode::Esc => PortForwardAction::Close,
            _ => PortForwardAction::None,
        }
    }

    pub fn render(&self, f: &mut ratatui::Frame, area: Rect) {
        let block = Block::default()
            .title(format!("Active Tunnels ({})", self.registry.active_count()))
            .borders(Borders::ALL);

        let inner = block.inner(area);
        f.render_widget(block, area);

        if self.registry.is_empty() {
            let message = Paragraph::new("No active tunnels")
                .style(Style::default().fg(Color::Gray));
            f.render_widget(message, inner);
            return;
        }

        let mut lines = Vec::new();
        for tunnel in self.registry.tunnels().values() {
            let state_color = match tunnel.state {
                TunnelState::Active => Color::Green,
                TunnelState::Starting => Color::Yellow,
                TunnelState::Error => Color::Red,
                _ => Color::Gray,
            };

            let selected = self
                .registry
                .selected()
                .map(|t| t.id == tunnel.id)
                .unwrap_or(false);

            let content = format!(
                "{}:{} → localhost:{}",
                tunnel.target.pod_name, tunnel.target.remote_port, tunnel.local_addr.port()
            );

            let style = if selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            lines.push(Line::from(vec![
                Span::styled("● ", Style::default().fg(state_color)),
                Span::styled(content, style),
            ]));
        }

        let paragraph = Paragraph::new(lines);
        f.render_widget(paragraph, inner);
    }
}

#[derive(Debug, PartialEq)]
pub enum PortForwardAction {
    None,
    Close,
    Create((PortForwardTarget, PortForwardConfig)),
    Stop(String),
}
