//! Resource template dialog for bounded create/apply flows.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::{Frame, Line, Span, Style},
    widgets::{
        Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
    },
};

use crate::resource_templates::{ResourceTemplateKind, ResourceTemplateValues};
use crate::ui::{cursor_visible_input_line, table_window, truncate_message, wrapped_line_count};

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
        };
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
        if let Some(field) = self.active_buffer_mut() {
            field.push(c);
            self.revalidate();
        }
    }

    pub fn backspace(&mut self) {
        if let Some(field) = self.active_buffer_mut() {
            field.pop();
            self.revalidate();
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

    fn active_buffer_mut(&mut self) -> Option<&mut String> {
        match self.focus_field {
            ResourceTemplateField::Name => Some(&mut self.values.name),
            ResourceTemplateField::Namespace => Some(&mut self.values.namespace),
            ResourceTemplateField::Image => Some(&mut self.values.image),
            ResourceTemplateField::Replicas => Some(&mut self.values.replicas),
            ResourceTemplateField::ContainerPort => Some(&mut self.values.container_port),
            ResourceTemplateField::ServicePort => Some(&mut self.values.service_port),
            ResourceTemplateField::ConfigKey => Some(&mut self.values.config_key),
            ResourceTemplateField::ConfigValue => Some(&mut self.values.config_value),
            ResourceTemplateField::CreateBtn | ResourceTemplateField::CancelBtn => None,
        }
    }

    fn revalidate(&mut self) {
        self.error_message = self.values.validate().err().map(|err| err.to_string());
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
            " Opening editor...",
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
                selected.then_some(value.chars().count()),
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

    let editable_fields = state
        .visible_fields()
        .into_iter()
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
    let window = table_window(editable_fields.len(), selected_idx, 1);
    let focused = editable_fields
        .get(window.start)
        .copied()
        .unwrap_or(ResourceTemplateField::Name);
    let (label, value) = match focused {
        ResourceTemplateField::Name => ("name", state.values.name.as_str()),
        ResourceTemplateField::Namespace => ("ns", state.values.namespace.as_str()),
        ResourceTemplateField::Image => ("image", state.values.image.as_str()),
        ResourceTemplateField::Replicas => ("replicas", state.values.replicas.as_str()),
        ResourceTemplateField::ContainerPort => ("ctr-port", state.values.container_port.as_str()),
        ResourceTemplateField::ServicePort => ("svc-port", state.values.service_port.as_str()),
        ResourceTemplateField::ConfigKey => ("cfg-key", state.values.config_key.as_str()),
        ResourceTemplateField::ConfigValue => ("cfg-val", state.values.config_value.as_str()),
        ResourceTemplateField::CreateBtn | ResourceTemplateField::CancelBtn => unreachable!(),
    };
    let focus = match state.focus_field {
        ResourceTemplateField::CreateBtn => "create",
        ResourceTemplateField::CancelBtn => "cancel",
        _ => label,
    };
    let status = if state.pending {
        "opening editor...".to_string()
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
            editable_fields.len(),
            focus
        )),
        compact_line(format!("{label}: {value}")),
        compact_line(status),
        compact_line("tab move  type edit".to_string()),
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
}
