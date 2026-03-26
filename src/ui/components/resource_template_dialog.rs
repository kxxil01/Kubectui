//! Resource template dialog for bounded create/apply flows.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::{Frame, Line, Span, Style},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::resource_templates::{ResourceTemplateKind, ResourceTemplateValues};

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
    let popup = crate::ui::centered_rect(68, 62, area);
    frame.render_widget(Clear, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(popup);

    let title = format!("Create from Template: {}", state.values.kind.label());
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            title,
            Style::default().fg(ratatui::style::Color::Cyan).bold(),
        )))
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Center),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  Fill the required fields, then "),
            Span::styled("Create", Style::default().bold()),
            Span::raw(" to open the manifest in your editor."),
        ]))
        .block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        chunks[1],
    );

    let fields = state.visible_fields();
    let rows = fields
        .iter()
        .filter(|field| {
            !matches!(
                field,
                ResourceTemplateField::CreateBtn | ResourceTemplateField::CancelBtn
            )
        })
        .map(|field| {
            let (label, value) = match field {
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
            Line::from(vec![
                Span::styled(
                    if selected { " ▶ " } else { "   " },
                    Style::default().fg(ratatui::style::Color::Yellow),
                ),
                Span::styled(format!("{label:16}"), Style::default().bold()),
                Span::raw(" "),
                Span::styled(
                    value,
                    if selected {
                        Style::default()
                            .fg(ratatui::style::Color::Black)
                            .bg(ratatui::style::Color::White)
                    } else {
                        Style::default()
                    },
                ),
                if selected {
                    Span::styled("█", Style::default().fg(ratatui::style::Color::White))
                } else {
                    Span::raw("")
                },
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(rows).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        chunks[2],
    );

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
    frame.render_widget(
        Paragraph::new(footer).block(Block::default().borders(Borders::ALL)),
        chunks[4],
    );
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
}
