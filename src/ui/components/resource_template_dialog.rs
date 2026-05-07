//! Resource template dialog for bounded create/apply flows.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::{Frame, Line, Span, Style},
    widgets::{
        Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
    },
};

use crate::resource_templates::{
    MAX_DNS_LABEL_LEN, MAX_DNS_SUBDOMAIN_LEN, MAX_TEMPLATE_CONFIG_VALUE_LEN,
    MAX_TEMPLATE_IMAGE_LEN, MAX_TEMPLATE_PORT_LEN, MAX_TEMPLATE_REPLICAS_LEN, ResourceTemplateKind,
    ResourceTemplateValues,
};
use crate::ui::{
    clear_input_at_cursor, cursor_visible_input_line, delete_char_left_at_cursor,
    delete_char_right_at_cursor, insert_char_at_cursor, loading_spinner_char, move_cursor_end,
    move_cursor_home, move_cursor_left, move_cursor_right, table_window, truncate_message,
    wrapped_line_count,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceTemplateField {
    Name,
    Namespace,
    Image,
    Replicas,
    ContainerPort,
    ServicePort,
    ConfigKey,
    ConfigValue,
    CreateBtn,
    CancelBtn,
}

#[derive(Debug, Clone)]
pub struct ResourceTemplateDialogState {
    pub values: ResourceTemplateValues,
    pub focus_field: ResourceTemplateField,
    pub error_message: Option<String>,
    pub pending: bool,
    name_cursor: usize,
    namespace_cursor: usize,
    image_cursor: usize,
    replicas_cursor: usize,
    container_port_cursor: usize,
    service_port_cursor: usize,
    config_key_cursor: usize,
    config_value_cursor: usize,
}

impl ResourceTemplateDialogState {
    pub fn new(kind: ResourceTemplateKind, namespace: impl Into<String>) -> Self {
        let namespace = namespace.into();
        let values = ResourceTemplateValues {
            kind,
            name: "sample-app".into(),
            namespace: if namespace == "all" || namespace.is_empty() {
                "default".into()
            } else {
                namespace
            },
            image: "nginx:1.27".into(),
            replicas: "1".into(),
            container_port: "8080".into(),
            service_port: "80".into(),
            config_key: "app.properties".into(),
            config_value: "key=value".into(),
        };
        let mut state = Self {
            values,
            focus_field: ResourceTemplateField::Name,
            error_message: None,
            pending: false,
            name_cursor: 0,
            namespace_cursor: 0,
            image_cursor: 0,
            replicas_cursor: 0,
            container_port_cursor: 0,
            service_port_cursor: 0,
            config_key_cursor: 0,
            config_value_cursor: 0,
        };
        state.sync_cursors_to_values();
        state.revalidate();
        state
    }

    pub fn next_field(&mut self) {
        let fields = self.visible_fields();
        let current = fields
            .iter()
            .position(|field| *field == self.focus_field)
            .unwrap_or(0);
        self.focus_field = fields[(current + 1) % fields.len()];
    }

    pub fn prev_field(&mut self) {
        let fields = self.visible_fields();
        let current = fields
            .iter()
            .position(|field| *field == self.focus_field)
            .unwrap_or(0);
        self.focus_field = if current == 0 {
            *fields.last().expect("visible fields")
        } else {
            fields[current - 1]
        };
    }

    pub fn add_char(&mut self, c: char) {
        let Some(max_chars) = self.focus_field.max_input_chars() else {
            return;
        };
        if let Some((field, cursor)) = self.active_buffer_and_cursor_mut()
            && field.chars().count() < max_chars
        {
            insert_char_at_cursor(field, cursor, c);
            self.revalidate();
        }
    }

    pub fn backspace(&mut self) {
        if let Some((field, cursor)) = self.active_buffer_and_cursor_mut() {
            let previous_len = field.len();
            delete_char_left_at_cursor(field, cursor);
            if field.len() != previous_len {
                self.revalidate();
            }
        }
    }

    pub fn delete_char(&mut self) {
        if let Some((field, cursor)) = self.active_buffer_and_cursor_mut() {
            let previous_len = field.len();
            delete_char_right_at_cursor(field, *cursor);
            if field.len() != previous_len {
                self.revalidate();
            }
        }
    }

    pub fn clear_active(&mut self) {
        if let Some((field, cursor)) = self.active_buffer_and_cursor_mut() {
            clear_input_at_cursor(field, cursor);
            self.revalidate();
        }
    }

    pub fn cursor_left(&mut self) {
        move_cursor_left(self.active_cursor_mut());
    }

    pub fn cursor_right(&mut self) {
        if let Some((field, cursor)) = self.active_buffer_and_cursor_mut() {
            move_cursor_right(cursor, field);
        }
    }

    pub fn cursor_home(&mut self) {
        move_cursor_home(self.active_cursor_mut());
    }

    pub fn cursor_end(&mut self) {
        if let Some((field, cursor)) = self.active_buffer_and_cursor_mut() {
            move_cursor_end(cursor, field);
        }
    }

    pub fn is_valid(&self) -> bool {
        self.error_message.is_none()
    }

    pub fn visible_fields(&self) -> Vec<ResourceTemplateField> {
        let mut fields = vec![
            ResourceTemplateField::Name,
            ResourceTemplateField::Namespace,
        ];
        match self.values.kind {
            ResourceTemplateKind::Deployment => fields.extend([
                ResourceTemplateField::Image,
                ResourceTemplateField::Replicas,
                ResourceTemplateField::ContainerPort,
            ]),
            ResourceTemplateKind::DeploymentService => fields.extend([
                ResourceTemplateField::Image,
                ResourceTemplateField::Replicas,
                ResourceTemplateField::ContainerPort,
                ResourceTemplateField::ServicePort,
            ]),
            ResourceTemplateKind::ConfigMap => fields.extend([
                ResourceTemplateField::ConfigKey,
                ResourceTemplateField::ConfigValue,
            ]),
        }
        fields.extend([
            ResourceTemplateField::CreateBtn,
            ResourceTemplateField::CancelBtn,
        ]);
        fields
    }

    fn active_buffer_and_cursor_mut(&mut self) -> Option<(&mut String, &mut usize)> {
        match self.focus_field {
            ResourceTemplateField::Name => Some((&mut self.values.name, &mut self.name_cursor)),
            ResourceTemplateField::Namespace => {
                Some((&mut self.values.namespace, &mut self.namespace_cursor))
            }
            ResourceTemplateField::Image => Some((&mut self.values.image, &mut self.image_cursor)),
            ResourceTemplateField::Replicas => {
                Some((&mut self.values.replicas, &mut self.replicas_cursor))
            }
            ResourceTemplateField::ContainerPort => Some((
                &mut self.values.container_port,
                &mut self.container_port_cursor,
            )),
            ResourceTemplateField::ServicePort => {
                Some((&mut self.values.service_port, &mut self.service_port_cursor))
            }
            ResourceTemplateField::ConfigKey => {
                Some((&mut self.values.config_key, &mut self.config_key_cursor))
            }
            ResourceTemplateField::ConfigValue => {
                Some((&mut self.values.config_value, &mut self.config_value_cursor))
            }
            ResourceTemplateField::CreateBtn | ResourceTemplateField::CancelBtn => None,
        }
    }

    fn active_cursor_mut(&mut self) -> &mut usize {
        match self.focus_field {
            ResourceTemplateField::Name => &mut self.name_cursor,
            ResourceTemplateField::Namespace => &mut self.namespace_cursor,
            ResourceTemplateField::Image => &mut self.image_cursor,
            ResourceTemplateField::Replicas => &mut self.replicas_cursor,
            ResourceTemplateField::ContainerPort => &mut self.container_port_cursor,
            ResourceTemplateField::ServicePort => &mut self.service_port_cursor,
            ResourceTemplateField::ConfigKey => &mut self.config_key_cursor,
            ResourceTemplateField::ConfigValue => &mut self.config_value_cursor,
            ResourceTemplateField::CreateBtn | ResourceTemplateField::CancelBtn => {
                &mut self.name_cursor
            }
        }
    }

    fn cursor_for(&self, field: ResourceTemplateField) -> usize {
        match field {
            ResourceTemplateField::Name => self.name_cursor,
            ResourceTemplateField::Namespace => self.namespace_cursor,
            ResourceTemplateField::Image => self.image_cursor,
            ResourceTemplateField::Replicas => self.replicas_cursor,
            ResourceTemplateField::ContainerPort => self.container_port_cursor,
            ResourceTemplateField::ServicePort => self.service_port_cursor,
            ResourceTemplateField::ConfigKey => self.config_key_cursor,
            ResourceTemplateField::ConfigValue => self.config_value_cursor,
            ResourceTemplateField::CreateBtn | ResourceTemplateField::CancelBtn => 0,
        }
    }

    fn sync_cursors_to_values(&mut self) {
        move_cursor_end(&mut self.name_cursor, &self.values.name);
        move_cursor_end(&mut self.namespace_cursor, &self.values.namespace);
        move_cursor_end(&mut self.image_cursor, &self.values.image);
        move_cursor_end(&mut self.replicas_cursor, &self.values.replicas);
        move_cursor_end(&mut self.container_port_cursor, &self.values.container_port);
        move_cursor_end(&mut self.service_port_cursor, &self.values.service_port);
        move_cursor_end(&mut self.config_key_cursor, &self.values.config_key);
        move_cursor_end(&mut self.config_value_cursor, &self.values.config_value);
    }

    fn revalidate(&mut self) {
        self.error_message = self.values.validate().err().map(|err| err.to_string());
    }
}

impl ResourceTemplateField {
    fn max_input_chars(self) -> Option<usize> {
        match self {
            Self::Name | Self::Namespace => Some(MAX_DNS_LABEL_LEN),
            Self::Image => Some(MAX_TEMPLATE_IMAGE_LEN),
            Self::Replicas => Some(MAX_TEMPLATE_REPLICAS_LEN),
            Self::ContainerPort | Self::ServicePort => Some(MAX_TEMPLATE_PORT_LEN),
            Self::ConfigKey => Some(MAX_DNS_SUBDOMAIN_LEN),
            Self::ConfigValue => Some(MAX_TEMPLATE_CONFIG_VALUE_LEN),
            Self::CreateBtn | Self::CancelBtn => None,
        }
    }
}

pub fn render_resource_template_dialog(
    frame: &mut Frame,
    area: Rect,
    state: &ResourceTemplateDialogState,
) {
    let popup = resource_template_popup(area);
    frame.render_widget(Clear, popup);
    if use_compact_resource_template_dialog(popup) {
        render_compact_resource_template_dialog(frame, popup, state);
        return;
    }

    let footer = if state.pending {
        Line::from(Span::styled(
            format!(" {} Opening editor...", loading_spinner_char()),
            Style::default().fg(ratatui::style::Color::Yellow),
        ))
    } else if let Some(error) = &state.error_message {
        Line::from(Span::styled(
            format!(" {error}"),
            Style::default().fg(ratatui::style::Color::Red),
        ))
    } else {
        Line::from(Span::styled(
            " Tab / Shift+Tab: move  Enter: create  Esc: cancel ",
            Style::default().fg(ratatui::style::Color::DarkGray),
        ))
    };
    let footer_lines = vec![footer];
    let footer_height =
        crate::ui::wrapped_line_count(&footer_lines, popup.width.max(1)).max(1) as u16 + 1;

    let title_lines = vec![Line::from(Span::styled(
        format!("Create from Template: {}", state.values.kind.label()),
        Style::default().fg(ratatui::style::Color::Cyan).bold(),
    ))];
    let hint_lines = vec![Line::from(vec![
        Span::raw("  Fill the required fields, then "),
        Span::styled("Create", Style::default().bold()),
        Span::raw(" to open the manifest in your editor."),
    ])];
    let title_height = wrapped_line_count(&title_lines, popup.width.max(1)).max(1) as u16 + 2;
    let hint_height = wrapped_line_count(&hint_lines, popup.width.max(1)).max(1) as u16;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(title_height),
            Constraint::Length(hint_height),
            Constraint::Min(8),
            Constraint::Length(3),
            Constraint::Length(footer_height),
        ])
        .split(popup);

    frame.render_widget(
        Paragraph::new(title_lines)
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: false })
            .alignment(Alignment::Center),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(hint_lines).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        chunks[1],
    );

    let fields = state.visible_fields();
    let editable_fields = fields
        .iter()
        .copied()
        .filter(|field| {
            !matches!(
                field,
                ResourceTemplateField::CreateBtn | ResourceTemplateField::CancelBtn
            )
        })
        .collect::<Vec<_>>();
    let selected_idx = editable_fields
        .iter()
        .position(|field| *field == state.focus_field)
        .unwrap_or_else(|| editable_fields.len().saturating_sub(1));
    let field_block = Block::default().borders(Borders::LEFT | Borders::RIGHT);
    let field_inner = field_block.inner(chunks[2]);
    frame.render_widget(field_block, chunks[2]);
    let window = table_window(
        editable_fields.len(),
        selected_idx,
        template_field_viewport_rows(chunks[2]),
    );
    let rows = editable_fields[window.start..window.end]
        .iter()
        .map(|field| {
            let (label, value) = match *field {
                ResourceTemplateField::Name => ("Name", state.values.name.as_str()),
                ResourceTemplateField::Namespace => ("Namespace", state.values.namespace.as_str()),
                ResourceTemplateField::Image => ("Image", state.values.image.as_str()),
                ResourceTemplateField::Replicas => ("Replicas", state.values.replicas.as_str()),
                ResourceTemplateField::ContainerPort => {
                    ("Container Port", state.values.container_port.as_str())
                }
                ResourceTemplateField::ServicePort => {
                    ("Service Port", state.values.service_port.as_str())
                }
                ResourceTemplateField::ConfigKey => {
                    ("Config Key", state.values.config_key.as_str())
                }
                ResourceTemplateField::ConfigValue => {
                    ("Config Value", state.values.config_value.as_str())
                }
                ResourceTemplateField::CreateBtn | ResourceTemplateField::CancelBtn => {
                    unreachable!()
                }
            };
            let selected = *field == state.focus_field;
            cursor_visible_input_line(
                &[
                    Span::styled(
                        if selected {
                            " ▶ ".to_string()
                        } else {
                            "   ".to_string()
                        },
                        Style::default().fg(ratatui::style::Color::Yellow),
                    ),
                    Span::styled(format!("{label:16} "), Style::default().bold()),
                ],
                value,
                selected.then_some(state.cursor_for(*field)),
                if selected {
                    Style::default()
                        .fg(ratatui::style::Color::Black)
                        .bg(ratatui::style::Color::White)
                } else {
                    Style::default()
                },
                if selected {
                    Style::default().fg(ratatui::style::Color::White)
                } else {
                    Style::default()
                },
                &[],
                usize::from(field_inner.width.max(1)),
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(rows), field_inner);
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"))
        .track_symbol(Some("│"))
        .thumb_symbol("█");
    let mut scrollbar_state = ScrollbarState::new(editable_fields.len()).position(window.start);
    frame.render_stateful_widget(scrollbar, field_inner, &mut scrollbar_state);

    let create_selected = state.focus_field == ResourceTemplateField::CreateBtn;
    let cancel_selected = state.focus_field == ResourceTemplateField::CancelBtn;
    let button_line = Line::from(vec![
        Span::raw("  "),
        Span::styled(
            " Create ",
            if create_selected {
                Style::default()
                    .fg(ratatui::style::Color::Black)
                    .bg(ratatui::style::Color::Green)
                    .bold()
            } else {
                Style::default().fg(ratatui::style::Color::Green)
            },
        ),
        Span::raw("   "),
        Span::styled(
            " Cancel ",
            if cancel_selected {
                Style::default()
                    .fg(ratatui::style::Color::Black)
                    .bg(ratatui::style::Color::Red)
                    .bold()
            } else {
                Style::default().fg(ratatui::style::Color::Red)
            },
        ),
    ]);
    frame.render_widget(
        Paragraph::new(button_line)
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
            .alignment(Alignment::Center),
        chunks[3],
    );

    frame.render_widget(
        Paragraph::new(footer_lines)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL)),
        chunks[4],
    );
}

fn resource_template_popup(area: Rect) -> Rect {
    let preferred_width = area.width.saturating_mul(68).saturating_div(100).max(52);
    let preferred_height = area.height.saturating_mul(62).saturating_div(100).max(12);
    crate::ui::bounded_popup_rect(area, preferred_width, preferred_height, 1, 1)
}

fn use_compact_resource_template_dialog(popup: Rect) -> bool {
    popup.width < 52 || popup.height < 18
}

fn render_compact_resource_template_dialog(
    frame: &mut Frame,
    popup: Rect,
    state: &ResourceTemplateDialogState,
) {
    let block = Block::default()
        .title(format!(" Template {} ", state.values.kind.label()))
        .borders(Borders::ALL);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let fields = state.visible_fields();
    let selected_idx = fields
        .iter()
        .position(|field| *field == state.focus_field)
        .unwrap_or(0);
    let focused = fields
        .get(selected_idx)
        .copied()
        .unwrap_or(ResourceTemplateField::Name);
    let (focus, detail) = match focused {
        ResourceTemplateField::Name => ("name", format!("name: {}", state.values.name)),
        ResourceTemplateField::Namespace => ("ns", format!("ns: {}", state.values.namespace)),
        ResourceTemplateField::Image => ("image", format!("image: {}", state.values.image)),
        ResourceTemplateField::Replicas => {
            ("replicas", format!("replicas: {}", state.values.replicas))
        }
        ResourceTemplateField::ContainerPort => (
            "ctr-port",
            format!("ctr-port: {}", state.values.container_port),
        ),
        ResourceTemplateField::ServicePort => (
            "svc-port",
            format!("svc-port: {}", state.values.service_port),
        ),
        ResourceTemplateField::ConfigKey => {
            ("cfg-key", format!("cfg-key: {}", state.values.config_key))
        }
        ResourceTemplateField::ConfigValue => {
            ("cfg-val", format!("cfg-val: {}", state.values.config_value))
        }
        ResourceTemplateField::CreateBtn => ("create", "button: create".to_string()),
        ResourceTemplateField::CancelBtn => ("cancel", "button: cancel".to_string()),
    };
    let status = if state.pending {
        format!("{} opening editor...", loading_spinner_char())
    } else if let Some(error) = &state.error_message {
        format!("err: {error}")
    } else {
        "enter create  esc cancel".to_string()
    };
    let compact_line = |text: String| {
        Line::from(truncate_message(&text, usize::from(inner.width.max(1))).into_owned())
    };
    let lines = vec![
        compact_line(format!(
            "field {}/{} {}",
            selected_idx + 1,
            fields.len(),
            focus
        )),
        compact_line(detail),
        compact_line(status),
        compact_line("tab move  enter activate".to_string()),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn template_field_viewport_rows(area: Rect) -> usize {
    usize::from(area.height.saturating_sub(2)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_map_dialog_hides_image_fields() {
        let state = ResourceTemplateDialogState::new(ResourceTemplateKind::ConfigMap, "default");
        let fields = state.visible_fields();
        assert!(fields.contains(&ResourceTemplateField::ConfigKey));
        assert!(!fields.contains(&ResourceTemplateField::Image));
    }

    #[test]
    fn template_field_window_keeps_selected_row_visible() {
        let mut state =
            ResourceTemplateDialogState::new(ResourceTemplateKind::DeploymentService, "default");
        state.focus_field = ResourceTemplateField::ServicePort;
        let fields = state
            .visible_fields()
            .into_iter()
            .filter(|field| {
                !matches!(
                    field,
                    ResourceTemplateField::CreateBtn | ResourceTemplateField::CancelBtn
                )
            })
            .collect::<Vec<_>>();
        let selected = fields
            .iter()
            .position(|field| *field == state.focus_field)
            .expect("selected field should exist");
        let window = table_window(
            fields.len(),
            selected,
            template_field_viewport_rows(Rect::new(0, 0, 60, 6)),
        );
        assert!(window.start <= selected);
        assert!(window.end > selected);
    }

    #[test]
    fn render_resource_template_dialog_small_terminal_smoke() {
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal should initialize");
        let state = ResourceTemplateDialogState::new(ResourceTemplateKind::Deployment, "default");
        terminal
            .draw(|frame| render_resource_template_dialog(frame, frame.area(), &state))
            .expect("compact resource template dialog should render");
    }

    #[test]
    fn compact_template_dialog_renders_button_focus() {
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal should initialize");
        let mut state =
            ResourceTemplateDialogState::new(ResourceTemplateKind::Deployment, "default");
        state.focus_field = ResourceTemplateField::CreateBtn;

        terminal
            .draw(|frame| render_resource_template_dialog(frame, frame.area(), &state))
            .expect("compact resource template dialog should render");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("field 6/7 create"));
        assert!(rendered.contains("button: create"));
        assert!(!rendered.contains("ctr-port:"));
    }

    #[test]
    fn template_dialog_edits_at_cursor_position() {
        let mut state =
            ResourceTemplateDialogState::new(ResourceTemplateKind::Deployment, "default");
        state.focus_field = ResourceTemplateField::Name;
        state.cursor_home();
        state.cursor_right();
        state.add_char('X');

        assert_eq!(state.values.name, "sXample-app");
    }

    #[test]
    fn template_dialog_edits_unicode_at_cursor_position() {
        let mut state =
            ResourceTemplateDialogState::new(ResourceTemplateKind::Deployment, "default");
        state.focus_field = ResourceTemplateField::Image;
        state.values.image = "aåb".to_string();
        state.image_cursor = 1;

        state.add_char('β');
        state.delete_char();
        state.backspace();

        assert_eq!(state.values.image, "ab");
        assert_eq!(state.cursor_for(ResourceTemplateField::Image), 1);
    }

    #[test]
    fn template_dialog_ctrl_u_clear_resets_cursor() {
        let mut state =
            ResourceTemplateDialogState::new(ResourceTemplateKind::Deployment, "default");
        state.focus_field = ResourceTemplateField::Namespace;
        state.clear_active();

        assert!(state.values.namespace.is_empty());
        assert_eq!(state.cursor_for(ResourceTemplateField::Namespace), 0);
    }

    #[test]
    fn template_dialog_caps_input_at_template_limits() {
        let mut state =
            ResourceTemplateDialogState::new(ResourceTemplateKind::Deployment, "default");
        state.focus_field = ResourceTemplateField::Name;
        state.values.name.clear();
        state.name_cursor = 0;

        for _ in 0..(MAX_DNS_LABEL_LEN + 10) {
            state.add_char('a');
        }

        assert_eq!(state.values.name.chars().count(), MAX_DNS_LABEL_LEN);
        assert_eq!(
            state.cursor_for(ResourceTemplateField::Name),
            MAX_DNS_LABEL_LEN
        );
        assert!(state.is_valid());
    }

    #[test]
    fn template_dialog_caps_config_value_at_template_limit() {
        let mut state =
            ResourceTemplateDialogState::new(ResourceTemplateKind::ConfigMap, "default");
        state.focus_field = ResourceTemplateField::ConfigValue;
        state.values.config_value.clear();
        state.config_value_cursor = 0;

        for _ in 0..(MAX_TEMPLATE_CONFIG_VALUE_LEN + 10) {
            state.add_char('x');
        }

        assert_eq!(
            state.values.config_value.chars().count(),
            MAX_TEMPLATE_CONFIG_VALUE_LEN
        );
        assert_eq!(
            state.cursor_for(ResourceTemplateField::ConfigValue),
            MAX_TEMPLATE_CONFIG_VALUE_LEN
        );
        assert!(state.is_valid());
    }
}
