//! Scale dialog component for workload replica scaling.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::{Color, Frame, Line, Span, Style},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

/// Field focus for keyboard navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleField {
    InputField,
    ApplyBtn,
    CancelBtn,
}

/// Actions emitted by the scale dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScaleAction {
    UpdateInput(String),
    AddChar(char),
    DeleteChar,
    Increment,
    Decrement,
    Submit,
    Cancel,
    None,
}

/// Scalable workload kinds supported by the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleTargetKind {
    Deployment,
    StatefulSet,
}

impl ScaleTargetKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Deployment => "Deployment",
            Self::StatefulSet => "StatefulSet",
        }
    }
}

/// State machine for the scale dialog.
#[derive(Debug, Clone)]
pub struct ScaleDialogState {
    pub target_kind: ScaleTargetKind,
    pub workload_name: String,
    pub namespace: String,
    pub current_replicas: i32,
    pub desired_replicas: String,
    pub input_buffer: String,
    pub focus_field: ScaleField,
    pub error_message: Option<String>,
    pub warning_message: Option<String>,
    pub pending: bool,
}

impl ScaleDialogState {
    /// Creates a new scale dialog state for a scalable workload.
    pub fn new(
        target_kind: ScaleTargetKind,
        workload_name: impl Into<String>,
        namespace: impl Into<String>,
        current_replicas: i32,
    ) -> Self {
        Self {
            target_kind,
            workload_name: workload_name.into(),
            namespace: namespace.into(),
            current_replicas,
            desired_replicas: current_replicas.to_string(),
            input_buffer: String::new(),
            focus_field: ScaleField::InputField,
            error_message: None,
            warning_message: None,
            pending: false,
        }
    }

    /// Handles a scale action and updates state accordingly.
    pub fn handle_action(&mut self, action: ScaleAction) {
        match action {
            ScaleAction::AddChar(c) if c.is_ascii_digit() => {
                self.input_buffer.push(c);
                self.validate_and_update();
            }
            ScaleAction::DeleteChar => {
                self.input_buffer.pop();
                self.validate_and_update();
            }
            ScaleAction::Increment => {
                let current = self.parse_input();
                let next = (current + 1).min(100);
                self.input_buffer = next.to_string();
                self.validate_and_update();
            }
            ScaleAction::Decrement => {
                let current = self.parse_input();
                let next = if current > 0 { current - 1 } else { 0 };
                self.input_buffer = next.to_string();
                self.validate_and_update();
            }
            ScaleAction::Submit => {
                if self.is_valid() {
                    self.desired_replicas = self.input_buffer.clone();
                }
            }
            _ => {}
        }
    }

    /// Parses the input buffer as an integer, defaulting to current replicas.
    fn parse_input(&self) -> i32 {
        if self.input_buffer.is_empty() {
            self.current_replicas
        } else {
            self.input_buffer.parse().unwrap_or(self.current_replicas)
        }
    }

    /// Validates and updates desired_replicas with error/warning messages.
    fn validate_and_update(&mut self) {
        self.error_message = None;
        self.warning_message = None;

        if self.input_buffer.is_empty() {
            self.desired_replicas = String::new();
            return;
        }

        // Check for leading zeros (reject "01", "001", etc., but allow "0")
        if self.input_buffer.len() > 1 && self.input_buffer.starts_with('0') {
            self.error_message = Some("No leading zeros allowed".to_string());
            return;
        }

        match self.input_buffer.parse::<i32>() {
            Ok(n) => {
                if !(0..=100).contains(&n) {
                    self.error_message = Some("Invalid range (must be 0-100)".to_string());
                } else {
                    self.desired_replicas = n.to_string();
                    // Check for large jump warning
                    let diff = (n - self.current_replicas).abs();
                    if diff > 10 {
                        self.warning_message =
                            Some(format!("Large scale change ({} replicas)", diff));
                    }
                }
            }
            Err(_) => {
                self.error_message = Some("Invalid number".to_string());
            }
        }
    }

    /// Returns true if the current input is valid for submission.
    pub fn is_valid(&self) -> bool {
        if self.input_buffer.is_empty() {
            return false;
        }

        if self.input_buffer.len() > 1 && self.input_buffer.starts_with('0') {
            return false;
        }

        match self.input_buffer.parse::<i32>() {
            Ok(n) => (0..=100).contains(&n),
            Err(_) => false,
        }
    }

    /// Returns the desired replica count as an integer (or None if invalid).
    pub fn desired_replicas_as_int(&self) -> Option<i32> {
        if self.is_valid() {
            self.input_buffer.parse().ok()
        } else {
            None
        }
    }

    /// Moves focus to the next field (Tab navigation).
    pub fn next_field(&mut self) {
        self.focus_field = match self.focus_field {
            ScaleField::InputField => ScaleField::ApplyBtn,
            ScaleField::ApplyBtn => ScaleField::CancelBtn,
            ScaleField::CancelBtn => ScaleField::InputField,
        };
    }

    /// Moves focus to the previous field (Shift+Tab navigation).
    pub fn prev_field(&mut self) {
        self.focus_field = match self.focus_field {
            ScaleField::InputField => ScaleField::CancelBtn,
            ScaleField::ApplyBtn => ScaleField::InputField,
            ScaleField::CancelBtn => ScaleField::ApplyBtn,
        };
    }

    /// Sets the pending flag (during API call).
    pub fn set_pending(&mut self, pending: bool) {
        self.pending = pending;
    }
}

/// Renders the scale dialog popup.
pub fn render_scale_dialog(frame: &mut Frame, area: Rect, state: &ScaleDialogState) {
    let popup = centered_rect(65, 45, area);
    frame.render_widget(Clear, popup);
    if use_compact_scale_dialog(popup) {
        render_compact_scale_dialog(frame, popup, state);
        return;
    }

    // Layout: Title | Metadata | Input | Validation | Buttons | Footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(popup);

    // Title
    let title = format!(
        "Scale {}: {}",
        state.target_kind.label(),
        state.workload_name
    );
    let title_widget = Paragraph::new(Line::from(vec![Span::styled(
        title,
        Style::default().fg(Color::Cyan).bold(),
    )]))
    .block(Block::default().borders(Borders::ALL))
    .alignment(Alignment::Center);
    frame.render_widget(title_widget, chunks[0]);

    // Metadata section
    let meta_lines = vec![Line::from(format!(
        "Namespace: {} | Current replicas: {}",
        state.namespace, state.current_replicas
    ))];
    let meta_widget =
        Paragraph::new(meta_lines).block(Block::default().borders(Borders::ALL).title("Info"));
    frame.render_widget(meta_widget, chunks[1]);

    // Input section with +/- buttons
    let input_display = if state.input_buffer.is_empty() {
        format!(" {} ", state.current_replicas)
    } else {
        format!(" {} ", state.input_buffer)
    };

    let input_style = if state.error_message.is_some() {
        Style::default().fg(Color::Red)
    } else if state.warning_message.is_some() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Cyan)
    };

    let cursor = if state.focus_field == ScaleField::InputField {
        "│"
    } else {
        " "
    };
    let input_box = format!("┌─{}─{}─┐", input_display, cursor);
    let input_footer = "└─────────────────────┘";

    let input_lines = vec![
        Line::from("New replica count (0-100):"),
        Line::from(Span::styled(input_box, input_style)),
        Line::from(Span::styled(input_footer, input_style)),
    ];

    let input_widget =
        Paragraph::new(input_lines).block(Block::default().borders(Borders::ALL).title("Input"));
    frame.render_widget(input_widget, chunks[2]);

    // Validation/Warning display
    let validation_line = if let Some(err) = &state.error_message {
        Line::from(Span::styled(
            format!("✗ {}", err),
            Style::default().fg(Color::Red),
        ))
    } else if let Some(warn) = &state.warning_message {
        Line::from(Span::styled(
            format!("⚠ {}", warn),
            Style::default().fg(Color::Yellow),
        ))
    } else if state.pending {
        Line::from(Span::styled(
            "⏳ Scaling...",
            Style::default().fg(Color::Magenta),
        ))
    } else {
        Line::from(Span::styled(
            "Use +/- keys to adjust, type digits, Enter to apply",
            Style::default().fg(Color::Gray),
        ))
    };

    let validation_widget =
        Paragraph::new(validation_line).block(Block::default().borders(Borders::ALL));
    frame.render_widget(validation_widget, chunks[3]);

    // Buttons with focus indication
    let apply_style = if state.focus_field == ScaleField::ApplyBtn {
        Style::default().fg(Color::Black).bg(Color::Green)
    } else {
        Style::default().fg(Color::Green)
    };

    let cancel_style = if state.focus_field == ScaleField::CancelBtn {
        Style::default().fg(Color::Black).bg(Color::Red)
    } else {
        Style::default().fg(Color::Red)
    };

    let button_line = Line::from(vec![
        Span::raw("  [ "),
        Span::styled("Apply", apply_style),
        Span::raw(" ]     [ "),
        Span::styled("Cancel", cancel_style),
        Span::raw(" ]  "),
    ]);

    let buttons_widget = Paragraph::new(button_line)
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Center);
    frame.render_widget(buttons_widget, chunks[4]);

    // Footer with key hints
    let footer_text = if state.pending {
        "[Esc] Cancel operation"
    } else {
        "[Enter] Confirm  [Esc] Cancel  [+/-] Adjust  [Tab] Navigate"
    };

    let footer_widget = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center);
    frame.render_widget(footer_widget, chunks[5]);
}

fn use_compact_scale_dialog(popup: Rect) -> bool {
    popup.width < 44 || popup.height < 18
}

fn render_compact_scale_dialog(frame: &mut Frame, popup: Rect, state: &ScaleDialogState) {
    let block = Block::default()
        .title(format!(
            " Scale {} {} ",
            state.target_kind.label(),
            state.workload_name
        ))
        .borders(Borders::ALL);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let desired = if state.input_buffer.is_empty() {
        state.current_replicas.to_string()
    } else {
        state.input_buffer.clone()
    };
    let focus = match state.focus_field {
        ScaleField::InputField => "input",
        ScaleField::ApplyBtn => "apply",
        ScaleField::CancelBtn => "cancel",
    };
    let status = if let Some(err) = &state.error_message {
        format!("err: {err}")
    } else if let Some(warn) = &state.warning_message {
        format!("warn: {warn}")
    } else if state.pending {
        "scaling...".to_string()
    } else {
        "enter apply  esc cancel".to_string()
    };
    let lines = vec![
        Line::from(format!(
            "ns {} current {}",
            state.namespace, state.current_replicas
        )),
        Line::from(format!("desired {} focus {}", desired, focus)),
        Line::from(status),
        Line::from("[+/-] adjust  [Tab] move"),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

use crate::ui::centered_rect;

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn draw(state: &ScaleDialogState) {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");
        terminal
            .draw(|frame| render_scale_dialog(frame, frame.area(), state))
            .expect("scale dialog should render");
    }

    #[test]
    fn test_scale_dialog_state_creation() {
        let state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);
        assert_eq!(state.target_kind, ScaleTargetKind::Deployment);
        assert_eq!(state.workload_name, "nginx");
        assert_eq!(state.namespace, "default");
        assert_eq!(state.current_replicas, 3);
        assert_eq!(state.desired_replicas, "3");
    }

    #[test]
    fn test_increment_logic() {
        let mut state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);
        state.handle_action(ScaleAction::Increment);
        assert_eq!(state.input_buffer, "4");
    }

    #[test]
    fn test_decrement_logic() {
        let mut state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);
        state.handle_action(ScaleAction::Decrement);
        assert_eq!(state.input_buffer, "2");
    }

    #[test]
    fn test_digit_input() {
        let mut state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);
        state.handle_action(ScaleAction::AddChar('5'));
        assert_eq!(state.input_buffer, "5");
    }

    #[test]
    fn test_backspace() {
        let mut state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);
        state.handle_action(ScaleAction::AddChar('5'));
        state.handle_action(ScaleAction::AddChar('0'));
        state.handle_action(ScaleAction::DeleteChar);
        assert_eq!(state.input_buffer, "5");
    }

    #[test]
    fn test_validation_range() {
        let mut state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);
        state.input_buffer = "101".to_string();
        state.validate_and_update();
        assert!(state.error_message.is_some());
    }

    #[test]
    fn test_zero_replica_valid() {
        let mut state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);
        state.input_buffer = "0".to_string();
        state.validate_and_update();
        assert!(state.error_message.is_none());
        assert!(state.is_valid());
    }

    #[test]
    fn test_leading_zero_invalid() {
        let mut state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);
        state.input_buffer = "05".to_string();
        state.validate_and_update();
        assert!(state.error_message.is_some());
    }

    #[test]
    fn test_large_jump_warning() {
        let mut state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 5);
        state.handle_action(ScaleAction::AddChar('8'));
        state.handle_action(ScaleAction::AddChar('0'));
        assert!(state.warning_message.is_some());
    }

    #[test]
    fn test_field_focus_cycling() {
        let mut state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);
        assert_eq!(state.focus_field, ScaleField::InputField);
        state.next_field();
        assert_eq!(state.focus_field, ScaleField::ApplyBtn);
        state.next_field();
        assert_eq!(state.focus_field, ScaleField::CancelBtn);
        state.next_field();
        assert_eq!(state.focus_field, ScaleField::InputField);
    }

    #[test]
    fn test_prev_field_cycling() {
        let mut state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);
        state.prev_field();
        assert_eq!(state.focus_field, ScaleField::CancelBtn);
        state.prev_field();
        assert_eq!(state.focus_field, ScaleField::ApplyBtn);
    }

    #[test]
    fn test_submit_updates_desired_replicas_when_valid() {
        let mut state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);
        state.handle_action(ScaleAction::AddChar('9'));
        state.handle_action(ScaleAction::Submit);
        assert_eq!(state.desired_replicas, "9");
    }

    #[test]
    fn test_submit_ignored_for_invalid_input() {
        let mut state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);
        state.input_buffer = "500".to_string();
        state.validate_and_update();
        state.handle_action(ScaleAction::Submit);
        assert_eq!(state.desired_replicas, "3");
    }

    #[test]
    fn test_desired_replicas_as_int_only_when_valid() {
        let mut state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);
        assert_eq!(state.desired_replicas_as_int(), None);

        state.handle_action(ScaleAction::AddChar('7'));
        assert_eq!(state.desired_replicas_as_int(), Some(7));
    }

    #[test]
    fn test_increment_and_decrement_boundaries() {
        let mut state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 100);
        state.handle_action(ScaleAction::Increment);
        assert_eq!(state.input_buffer, "100");

        state.input_buffer = "0".to_string();
        state.handle_action(ScaleAction::Decrement);
        assert_eq!(state.input_buffer, "0");
    }

    #[test]
    fn render_scale_dialog_smoke_default_state() {
        let state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);
        draw(&state);
    }

    #[test]
    fn render_scale_dialog_smoke_error_warning_pending_states() {
        let mut state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);

        state.error_message = Some("Invalid range".to_string());
        draw(&state);

        state.error_message = None;
        state.warning_message = Some("Large scale change".to_string());
        draw(&state);

        state.warning_message = None;
        state.set_pending(true);
        state.focus_field = ScaleField::CancelBtn;
        draw(&state);
    }

    #[test]
    fn render_scale_dialog_small_terminal_smoke() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");
        let state = ScaleDialogState::new(ScaleTargetKind::Deployment, "nginx", "default", 3);
        terminal
            .draw(|frame| render_scale_dialog(frame, frame.area(), &state))
            .expect("compact scale dialog should render");
    }
}
