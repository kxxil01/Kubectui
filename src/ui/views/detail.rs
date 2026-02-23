//! Detail modal renderer.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use crate::{app::DetailViewState, ui::components::default_theme};

/// Renders resource detail as a centered modal overlay.
pub fn render_detail(frame: &mut Frame, area: Rect, detail_state: &DetailViewState) {
    let theme = default_theme();
    let popup = centered_rect(88, 88, area);
    frame.render_widget(Clear, popup);

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_active_style())
        .style(Style::default().bg(theme.bg));
    frame.render_widget(outer_block, popup);

    let inner = Rect {
        x: popup.x + 1,
        y: popup.y + 1,
        width: popup.width.saturating_sub(2),
        height: popup.height.saturating_sub(2),
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(9),
            Constraint::Length(8),
            Constraint::Length(5),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(inner);

    let (kind_label, name_label) = if let Some(resource) = &detail_state.resource {
        (
            resource.kind().to_ascii_uppercase(),
            resource.name().to_string(),
        )
    } else {
        ("RESOURCE".to_string(), "unknown".to_string())
    };

    let header_line = Line::from(vec![
        Span::styled(" ◆ ", theme.title_style()),
        Span::styled(kind_label, theme.title_style()),
        Span::styled("  /  ", theme.muted_style()),
        Span::styled(
            name_label,
            Style::default()
                .fg(theme.fg)
                .add_modifier(Modifier::BOLD),
        ),
        if detail_state.loading {
            Span::styled("  ⟳ Loading…", theme.badge_warning_style())
        } else {
            Span::raw("")
        },
    ]);

    let header_block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.header_bg));
    frame.render_widget(Paragraph::new(header_line).block(header_block), chunks[0]);

    let labels_str = if detail_state.metadata.labels.is_empty() {
        "—".to_string()
    } else {
        detail_state
            .metadata
            .labels
            .iter()
            .take(4)
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("  ")
    };

    let status_str = detail_state
        .metadata
        .status
        .as_deref()
        .unwrap_or("Unknown");
    let status_style = theme.get_status_style(status_str);

    let metadata_lines = vec![
        Line::from(vec![
            Span::styled("  Name       ", theme.inactive_style()),
            Span::styled(
                detail_state.metadata.name.clone(),
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Namespace  ", theme.inactive_style()),
            Span::styled(
                detail_state
                    .metadata
                    .namespace
                    .as_deref()
                    .unwrap_or("cluster-scope")
                    .to_string(),
                Style::default().fg(theme.accent2),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Status     ", theme.inactive_style()),
            Span::styled(status_str.to_string(), status_style),
        ]),
        Line::from(vec![
            Span::styled("  Node       ", theme.inactive_style()),
            Span::styled(
                detail_state
                    .metadata
                    .node
                    .as_deref()
                    .unwrap_or("n/a")
                    .to_string(),
                Style::default().fg(theme.fg_dim),
            ),
        ]),
        Line::from(vec![
            Span::styled("  IP         ", theme.inactive_style()),
            Span::styled(
                detail_state
                    .metadata
                    .ip
                    .as_deref()
                    .unwrap_or("n/a")
                    .to_string(),
                Style::default().fg(theme.info),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Created    ", theme.inactive_style()),
            Span::styled(
                detail_state
                    .metadata
                    .created
                    .as_deref()
                    .unwrap_or("n/a")
                    .to_string(),
                Style::default().fg(theme.fg_dim),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Labels     ", theme.inactive_style()),
            Span::styled(labels_str, Style::default().fg(theme.muted)),
        ]),
    ];

    let metadata_block = Block::default()
        .title(Span::styled(" Metadata ", theme.section_title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));
    frame.render_widget(
        Paragraph::new(metadata_lines)
            .block(metadata_block)
            .wrap(Wrap { trim: false }),
        chunks[1],
    );

    let mut resource_lines: Vec<Line<'_>> = Vec::new();
    for section in &detail_state.sections {
        if section.chars().all(|c| c.is_uppercase() || c == '_' || c == ' ') {
            resource_lines.push(Line::from(Span::styled(
                format!("  {section}"),
                theme.section_title_style(),
            )));
        } else {
            resource_lines.push(Line::from(vec![
                Span::styled("  ", theme.inactive_style()),
                Span::styled(section.clone(), Style::default().fg(theme.fg_dim)),
            ]));
        }
    }

    if !detail_state.events.is_empty() {
        resource_lines.push(Line::from(""));
        resource_lines.push(Line::from(Span::styled(
            "  EVENTS",
            theme.section_title_style(),
        )));
        for event in detail_state.events.iter().take(4) {
            let (icon, ev_style) = if event.event_type.eq_ignore_ascii_case("warning") {
                ("  ⚠ ", theme.badge_warning_style())
            } else {
                ("  ✓ ", theme.badge_success_style())
            };
            resource_lines.push(Line::from(vec![
                Span::styled(icon, ev_style),
                Span::styled(
                    format!("{} (×{})  ", event.reason, event.count),
                    Style::default()
                        .fg(theme.fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(event.message.clone(), Style::default().fg(theme.fg_dim)),
            ]));
        }
    }

    if resource_lines.is_empty() {
        resource_lines.push(Line::from(Span::styled(
            "  No resource-specific details available",
            theme.inactive_style(),
        )));
    }

    let resource_block = Block::default()
        .title(Span::styled(" Details ", theme.section_title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));
    frame.render_widget(
        Paragraph::new(resource_lines)
            .block(resource_block)
            .wrap(Wrap { trim: false }),
        chunks[2],
    );

    let metrics_lines = if let Some(message) = &detail_state.metrics_unavailable_message {
        vec![Line::from(Span::styled(
            format!("  ℹ {message}"),
            theme.inactive_style(),
        ))]
    } else if let Some(node_metrics) = &detail_state.node_metrics {
        vec![
            Line::from(vec![
                Span::styled("  CPU     ", theme.inactive_style()),
                Span::styled(node_metrics.cpu.clone(), Style::default().fg(theme.accent)),
            ]),
            Line::from(vec![
                Span::styled("  Memory  ", theme.inactive_style()),
                Span::styled(
                    node_metrics.memory.clone(),
                    Style::default().fg(theme.accent2),
                ),
            ]),
        ]
    } else if let Some(pod_metrics) = &detail_state.pod_metrics {
        if pod_metrics.containers.is_empty() {
            vec![Line::from(Span::styled(
                "  No container metrics",
                theme.inactive_style(),
            ))]
        } else {
            pod_metrics
                .containers
                .iter()
                .map(|c| {
                    Line::from(vec![
                        Span::styled(format!("  {} ", c.name), theme.hover_style()),
                        Span::styled(
                            format!("cpu={}", c.cpu),
                            Style::default().fg(theme.accent),
                        ),
                        Span::styled("  ", theme.inactive_style()),
                        Span::styled(
                            format!("mem={}", c.memory),
                            Style::default().fg(theme.accent2),
                        ),
                    ])
                })
                .collect()
        }
    } else {
        vec![Line::from(Span::styled(
            "  Metrics unavailable",
            theme.inactive_style(),
        ))]
    };

    let metrics_block = Block::default()
        .title(Span::styled(" Metrics ", theme.section_title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));
    frame.render_widget(
        Paragraph::new(metrics_lines)
            .block(metrics_block)
            .wrap(Wrap { trim: false }),
        chunks[3],
    );

    let yaml_body = if detail_state.loading {
        "  ⟳ Loading YAML…".to_string()
    } else if let Some(err) = &detail_state.error {
        format!("  ✗ Error: {err}")
    } else {
        detail_state
            .yaml
            .clone()
            .unwrap_or_else(|| "  YAML not available".to_string())
    };

    let yaml_style = if detail_state.error.is_some() {
        theme.badge_error_style()
    } else {
        Style::default().fg(theme.fg_dim)
    };

    let yaml_block = Block::default()
        .title(Span::styled(" YAML ", theme.section_title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));
    frame.render_widget(
        Paragraph::new(Span::styled(yaml_body, yaml_style))
            .block(yaml_block)
            .wrap(Wrap { trim: false }),
        chunks[4],
    );

    let footer_line = Line::from(vec![
        Span::styled(" [l] ", theme.keybind_key_style()),
        Span::styled("Logs  ", theme.keybind_desc_style()),
        Span::styled("[p] ", theme.keybind_key_style()),
        Span::styled("Port-Fwd  ", theme.keybind_desc_style()),
        Span::styled("[s] ", theme.keybind_key_style()),
        Span::styled("Scale  ", theme.keybind_desc_style()),
        Span::styled("[Esc] ", theme.keybind_key_style()),
        Span::styled("Close", theme.keybind_desc_style()),
    ]);

    let footer_block = Block::default()
        .borders(Borders::TOP)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.statusbar_bg));
    frame.render_widget(Paragraph::new(footer_line).block(footer_block), chunks[5]);
}

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
