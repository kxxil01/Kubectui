//! Detail modal renderer.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    app::DetailViewState,
    ui::components::{
        default_theme, probe_panel::render_probe_panel, scale_dialog::render_scale_dialog,
    },
};

fn render_metadata_panel(frame: &mut Frame, area: Rect, detail_state: &DetailViewState) {
    let theme = default_theme();

    let status_str = detail_state.metadata.status.as_deref().unwrap_or("Unknown");
    let status_style = theme.get_status_style(status_str);

    let mut lines = vec![
        Line::from(vec![
            Span::styled(" Name      ", theme.inactive_style()),
            Span::styled(
                detail_state.metadata.name.clone(),
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Namespace ", theme.inactive_style()),
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
            Span::styled(" Status    ", theme.inactive_style()),
            Span::styled(status_str.to_string(), status_style),
        ]),
        Line::from(vec![
            Span::styled(" Created   ", theme.inactive_style()),
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
    ];

    // Labels
    if !detail_state.metadata.labels.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(" Labels    ", theme.inactive_style()),
            Span::styled(
                format!("({})", detail_state.metadata.labels.len()),
                Style::default().fg(theme.muted),
            ),
        ]));
        for (k, v) in detail_state.metadata.labels.iter().take(5) {
            lines.push(Line::from(vec![
                Span::styled("   ", theme.inactive_style()),
                Span::styled(k.clone(), Style::default().fg(theme.accent)),
                Span::styled("=", Style::default().fg(theme.muted)),
                Span::styled(v.clone(), Style::default().fg(theme.fg_dim)),
            ]));
        }
        if detail_state.metadata.labels.len() > 5 {
            lines.push(Line::from(Span::styled(
                format!("   … +{} more", detail_state.metadata.labels.len() - 5),
                Style::default().fg(theme.muted),
            )));
        }
    } else {
        lines.push(Line::from(vec![
            Span::styled(" Labels    ", theme.inactive_style()),
            Span::styled("—", Style::default().fg(theme.muted)),
        ]));
    }

    // Annotations
    if !detail_state.metadata.annotations.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(" Annot.    ", theme.inactive_style()),
            Span::styled(
                format!("({})", detail_state.metadata.annotations.len()),
                Style::default().fg(theme.muted),
            ),
        ]));
        for (k, v) in detail_state.metadata.annotations.iter().take(3) {
            let display_val = if v.len() > 50 {
                format!("{}…", &v[..v.floor_char_boundary(50)])
            } else {
                v.clone()
            };
            lines.push(Line::from(vec![
                Span::styled("   ", theme.inactive_style()),
                Span::styled(k.clone(), Style::default().fg(theme.accent)),
                Span::styled("=", Style::default().fg(theme.muted)),
                Span::styled(display_val, Style::default().fg(theme.fg_dim)),
            ]));
        }
        if detail_state.metadata.annotations.len() > 3 {
            lines.push(Line::from(Span::styled(
                format!("   … +{} more", detail_state.metadata.annotations.len() - 3),
                Style::default().fg(theme.muted),
            )));
        }
    }

    let block = Block::default()
        .title(Span::styled(" Metadata ", theme.section_title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_details_panel(frame: &mut Frame, area: Rect, detail_state: &DetailViewState) {
    let theme = default_theme();
    let mut lines: Vec<Line<'_>> = Vec::new();

    if let Some(node) = &detail_state.metadata.node {
        lines.push(Line::from(vec![
            Span::styled(" Node  ", theme.inactive_style()),
            Span::styled(node.clone(), Style::default().fg(theme.fg_dim)),
        ]));
    }
    if let Some(ip) = &detail_state.metadata.ip {
        lines.push(Line::from(vec![
            Span::styled(" IP    ", theme.inactive_style()),
            Span::styled(ip.clone(), Style::default().fg(theme.info)),
        ]));
    }

    for section in &detail_state.sections {
        if section
            .chars()
            .all(|c| c.is_uppercase() || c == '_' || c == ' ')
        {
            lines.push(Line::from(Span::styled(
                format!(" {section}"),
                theme.section_title_style(),
            )));
        } else {
            lines.push(Line::from(vec![
                Span::styled(" ", theme.inactive_style()),
                Span::styled(section.clone(), Style::default().fg(theme.fg_dim)),
            ]));
        }
    }

    if !detail_state.metadata.owner_references.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            " OWNERS",
            theme.section_title_style(),
        )));
        for oref in &detail_state.metadata.owner_references {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", oref.kind),
                    Style::default().fg(theme.accent),
                ),
                Span::styled(&*oref.name, Style::default().fg(theme.fg)),
            ]));
        }
    }

    if !detail_state.events.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            " EVENTS",
            theme.section_title_style(),
        )));
        for event in detail_state.events.iter().take(5) {
            let (icon, ev_style) = if event.event_type.eq_ignore_ascii_case("warning") {
                (" ⚠ ", theme.badge_warning_style())
            } else {
                (" ✓ ", theme.badge_success_style())
            };
            lines.push(Line::from(vec![
                Span::styled(icon, ev_style),
                Span::styled(
                    format!("{} (×{})  ", event.reason, event.count),
                    Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
                ),
                Span::styled(event.message.clone(), Style::default().fg(theme.fg_dim)),
            ]));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            " No additional details",
            theme.inactive_style(),
        )));
    }

    let block = Block::default()
        .title(Span::styled(" Details ", theme.section_title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_metrics_panel(frame: &mut Frame, area: Rect, detail_state: &DetailViewState) {
    let theme = default_theme();

    let lines = if let Some(message) = &detail_state.metrics_unavailable_message {
        vec![Line::from(Span::styled(
            format!(" ℹ {message}"),
            theme.inactive_style(),
        ))]
    } else if let Some(nm) = &detail_state.node_metrics {
        vec![
            Line::from(vec![
                Span::styled(" CPU    ", theme.inactive_style()),
                Span::styled(nm.cpu.clone(), Style::default().fg(theme.accent)),
            ]),
            Line::from(vec![
                Span::styled(" Memory ", theme.inactive_style()),
                Span::styled(nm.memory.clone(), Style::default().fg(theme.accent2)),
            ]),
        ]
    } else if let Some(pm) = &detail_state.pod_metrics {
        if pm.containers.is_empty() {
            vec![Line::from(Span::styled(
                " No container metrics",
                theme.inactive_style(),
            ))]
        } else {
            pm.containers
                .iter()
                .map(|c| {
                    Line::from(vec![
                        Span::styled(format!(" {} ", c.name), theme.hover_style()),
                        Span::styled(format!("cpu={}", c.cpu), Style::default().fg(theme.accent)),
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
            " Metrics unavailable",
            theme.inactive_style(),
        ))]
    };

    let block = Block::default()
        .title(Span::styled(" Metrics ", theme.section_title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_inspection_panel(frame: &mut Frame, area: Rect, detail_state: &DetailViewState) {
    let theme = default_theme();

    let block = Block::default()
        .title(Span::styled(" Inspection ", theme.section_title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_active_style())
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if detail_state.loading {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Loading resource details...",
                theme.badge_warning_style(),
            )),
            inner,
        );
        return;
    }

    if let Some(err) = &detail_state.error {
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" ✗ {err}"), theme.badge_error_style())),
            inner,
        );
        return;
    }

    let yaml_lines = detail_state
        .yaml
        .as_ref()
        .map(|yaml| yaml.lines().count())
        .unwrap_or(0);

    let lines = vec![
        Line::from(vec![
            Span::styled(" [y] ", theme.keybind_key_style()),
            Span::styled("Open YAML in workbench", theme.keybind_desc_style()),
        ]),
        Line::from(vec![
            Span::styled(" [v] ", theme.keybind_key_style()),
            Span::styled("Open events in workbench", theme.keybind_desc_style()),
        ]),
        if detail_state.supports_action(crate::policy::DetailAction::Logs) {
            Line::from(vec![
                Span::styled(" [l] ", theme.keybind_key_style()),
                Span::styled("Open logs in workbench", theme.keybind_desc_style()),
            ])
        } else {
            Line::from("")
        },
        if detail_state.supports_action(crate::policy::DetailAction::Exec) {
            Line::from(vec![
                Span::styled(" [x] ", theme.keybind_key_style()),
                Span::styled("Open exec shell in workbench", theme.keybind_desc_style()),
            ])
        } else {
            Line::from("")
        },
        if detail_state.supports_action(crate::policy::DetailAction::PortForward) {
            Line::from(vec![
                Span::styled(" [f] ", theme.keybind_key_style()),
                Span::styled("Open port-forward workbench", theme.keybind_desc_style()),
            ])
        } else {
            Line::from("")
        },
        Line::from(""),
        Line::from(vec![
            Span::styled(" YAML lines ", theme.inactive_style()),
            Span::styled(yaml_lines.to_string(), Style::default().fg(theme.fg)),
        ]),
        Line::from(vec![
            Span::styled(" Events     ", theme.inactive_style()),
            Span::styled(
                detail_state.events.len().to_string(),
                Style::default().fg(theme.fg),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Workbench  ", theme.inactive_style()),
            Span::styled(
                "Persistent, non-blocking inspection surface",
                Style::default().fg(theme.fg_dim),
            ),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Renders resource detail as a centered modal overlay.
pub fn render_detail(frame: &mut Frame, area: Rect, detail_state: &DetailViewState) {
    if let Some(scale) = &detail_state.scale_dialog {
        render_scale_dialog(frame, area, scale);
        return;
    }

    if let Some(panel) = &detail_state.probe_panel {
        let popup = centered_rect(80, 80, area);
        frame.render_widget(ratatui::widgets::Clear, popup);
        render_probe_panel(frame, popup, panel);
        return;
    }

    let theme = default_theme();
    let popup = centered_rect(90, 92, area);
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
            Constraint::Length(2),
            Constraint::Min(9),
            Constraint::Min(6),
            Constraint::Length(2),
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
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
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

    let info_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(35),
            Constraint::Percentage(40),
            Constraint::Percentage(25),
        ])
        .split(chunks[1]);

    render_metadata_panel(frame, info_cols[0], detail_state);
    render_details_panel(frame, info_cols[1], detail_state);
    render_metrics_panel(frame, info_cols[2], detail_state);
    render_inspection_panel(frame, chunks[2], detail_state);

    let mut footer_spans = Vec::new();
    for action in detail_state.footer_actions() {
        footer_spans.push(Span::styled(
            format!("{} ", action.key_hint()),
            theme.keybind_key_style(),
        ));
        footer_spans.push(Span::styled(
            format!("{}  ", action.label()),
            theme.keybind_desc_style(),
        ));
    }
    footer_spans.push(Span::styled("[Esc] ", theme.keybind_key_style()));
    footer_spans.push(Span::styled("Close", theme.keybind_desc_style()));
    footer_spans.insert(0, Span::raw(" "));

    let footer_line = Line::from(footer_spans);
    let footer_block = Block::default()
        .borders(Borders::TOP)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.statusbar_bg));
    frame.render_widget(Paragraph::new(footer_line).block(footer_block), chunks[3]);

    if detail_state.confirm_delete {
        render_delete_confirm(frame, popup, detail_state);
    } else if detail_state.confirm_drain {
        render_drain_confirm(frame, popup, detail_state);
    }
}

fn render_delete_confirm(frame: &mut Frame, parent: Rect, detail_state: &DetailViewState) {
    let theme = default_theme();
    let popup = centered_rect(48, 24, parent);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(
            " Confirm Delete ",
            theme.badge_error_style().add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_active_style())
        .style(Style::default().bg(theme.bg));
    frame.render_widget(block, popup);

    let inner = Rect {
        x: popup.x + 1,
        y: popup.y + 1,
        width: popup.width.saturating_sub(2),
        height: popup.height.saturating_sub(2),
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(inner);

    let resource_label = detail_state
        .resource
        .as_ref()
        .map(|r| format!("{} '{}'", r.kind(), r.name()))
        .unwrap_or_else(|| "selected resource".to_string());

    let body = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(" Delete ", theme.badge_error_style()),
            Span::styled(resource_label, Style::default().fg(theme.fg)),
            Span::styled(" ?", Style::default().fg(theme.fg)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " This action cannot be undone from KubecTUI.",
            theme.inactive_style(),
        )),
    ];
    frame.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .alignment(ratatui::layout::Alignment::Center),
        rows[0],
    );

    let footer = Line::from(vec![
        Span::styled(" [D] / [y] / [Enter] ", theme.keybind_key_style()),
        Span::styled("Confirm  ", theme.keybind_desc_style()),
        Span::styled("[F] ", theme.keybind_key_style()),
        Span::styled("Force  ", theme.keybind_desc_style()),
        Span::styled("[Esc] ", theme.keybind_key_style()),
        Span::styled("Cancel", theme.keybind_desc_style()),
    ]);
    frame.render_widget(
        Paragraph::new(footer).alignment(ratatui::layout::Alignment::Center),
        rows[1],
    );
}

fn render_drain_confirm(frame: &mut Frame, parent: Rect, detail_state: &DetailViewState) {
    let theme = default_theme();
    let popup = centered_rect(52, 24, parent);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(
            " Confirm Drain ",
            theme.badge_warning_style().add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_active_style())
        .style(Style::default().bg(theme.bg));
    frame.render_widget(block, popup);

    let inner = Rect {
        x: popup.x + 1,
        y: popup.y + 1,
        width: popup.width.saturating_sub(2),
        height: popup.height.saturating_sub(2),
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(inner);

    let node_name = detail_state
        .resource
        .as_ref()
        .map(|r| r.name().to_string())
        .unwrap_or_else(|| "selected node".to_string());

    let body = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(" Drain Node ", theme.badge_warning_style()),
            Span::styled(format!("'{node_name}'"), Style::default().fg(theme.fg)),
            Span::styled(" ?", Style::default().fg(theme.fg)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " This will evict all pods from this node.",
            theme.inactive_style(),
        )),
    ];
    frame.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .alignment(ratatui::layout::Alignment::Center),
        rows[0],
    );

    let footer = Line::from(vec![
        Span::styled(" [D] / [y] / [Enter] ", theme.keybind_key_style()),
        Span::styled("Drain  ", theme.keybind_desc_style()),
        Span::styled("[F] ", theme.keybind_key_style()),
        Span::styled("Force drain  ", theme.keybind_desc_style()),
        Span::styled("[Esc] ", theme.keybind_key_style()),
        Span::styled("Cancel", theme.keybind_desc_style()),
    ]);
    frame.render_widget(
        Paragraph::new(footer).alignment(ratatui::layout::Alignment::Center),
        rows[1],
    );
}

use crate::ui::centered_rect;
