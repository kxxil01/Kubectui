//! Detail modal renderer with syntax-highlighted YAML and scrollable panels.

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
};

use crate::{
    app::{DetailViewState, ResourceRef},
    ui::{
        theme::Theme,
        components::{
            default_theme,
            probe_panel::render_probe_panel,
            scale_dialog::render_scale_dialog,
        },
    },
};

// ─── YAML syntax highlighting ────────────────────────────────────────────────

/// Colorize a YAML value span based on its content.
fn colorize_value<'a>(value: &'a str, theme: &Theme) -> Span<'a> {
    let trimmed = value.trim();
    if trimmed == "true" || trimmed == "false" || trimmed == "null" || trimmed == "~" {
        Span::styled(
            value.to_string(),
            Style::default()
                .fg(theme.accent2)
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
        )
    } else if trimmed.parse::<f64>().is_ok() {
        Span::styled(value.to_string(), Style::default().fg(theme.warning))
    } else {
        Span::styled(value.to_string(), Style::default().fg(theme.success))
    }
}

/// Find the position of the key-value colon separator (not inside quotes).
fn find_key_colon(s: &str) -> Option<usize> {
    let mut in_single = false;
    let mut in_double = false;
    for (i, c) in s.char_indices() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ':' if !in_single && !in_double => {
                // Must be followed by space, newline, or end of string
                let rest = &s[i + 1..];
                if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\n') {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse a single YAML line into styled spans.
fn highlight_yaml_line<'a>(line: &'a str, theme: &Theme) -> Line<'a> {
    // Count leading spaces for indentation
    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    let content = &line[indent_len..];

    // Document separator
    if content.starts_with("---") || content.starts_with("...") {
        return Line::from(vec![
            Span::raw(indent.to_string()),
            Span::styled(
                content.to_string(),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
    }

    // Comment
    if content.starts_with('#') {
        return Line::from(vec![
            Span::raw(indent.to_string()),
            Span::styled(content.to_string(), Style::default().fg(theme.muted)),
        ]);
    }

    // List item: "- key: value" or "- value"
    if let Some(rest) = content.strip_prefix("- ") {
        let dash = Span::styled("- ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD));
        if let Some(colon_pos) = find_key_colon(rest) {
            let key = &rest[..colon_pos];
            let after_colon = &rest[colon_pos + 1..];
            let value = after_colon.trim_start();
            let mut spans = vec![
                Span::raw(indent.to_string()),
                dash,
                Span::styled(
                    key.to_string(),
                    Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(": ", Style::default().fg(theme.muted)),
            ];
            if !value.is_empty() {
                spans.push(colorize_value(value, theme));
            }
            return Line::from(spans);
        }
        return Line::from(vec![
            Span::raw(indent.to_string()),
            dash,
            colorize_value(rest, theme),
        ]);
    }

    // Key: value pair
    if let Some(colon_pos) = find_key_colon(content) {
        let key = &content[..colon_pos];
        let after_colon = &content[colon_pos + 1..];
        let value = after_colon.trim_start();
        let mut spans = vec![
            Span::raw(indent.to_string()),
            Span::styled(
                key.to_string(),
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(": ", Style::default().fg(theme.muted)),
        ];
        if !value.is_empty() {
            spans.push(colorize_value(value, theme));
        }
        return Line::from(spans);
    }

    // Plain line (continuation, multiline value, etc.)
    Line::from(vec![
        Span::raw(indent.to_string()),
        Span::styled(content.to_string(), Style::default().fg(theme.fg_dim)),
    ])
}

/// Convert a YAML string into a vec of syntax-highlighted `Line`s.
fn highlight_yaml<'a>(yaml: &'a str, theme: &Theme) -> Vec<Line<'a>> {
    yaml.lines().map(|l| highlight_yaml_line(l, theme)).collect()
}

// ─── Sub-panel renderers ──────────────────────────────────────────────────────

fn render_metadata_panel(frame: &mut Frame, area: Rect, detail_state: &DetailViewState) {
    let theme = default_theme();

    let labels_str = if detail_state.metadata.labels.is_empty() {
        "—".to_string()
    } else {
        detail_state
            .metadata
            .labels
            .iter()
            .take(3)
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("  ")
    };

    let status_str = detail_state.metadata.status.as_deref().unwrap_or("Unknown");
    let status_style = theme.get_status_style(status_str);

    let lines = vec![
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
                detail_state.metadata.namespace.as_deref().unwrap_or("cluster-scope").to_string(),
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
                detail_state.metadata.created.as_deref().unwrap_or("n/a").to_string(),
                Style::default().fg(theme.fg_dim),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Labels    ", theme.inactive_style()),
            Span::styled(labels_str, Style::default().fg(theme.muted)),
        ]),
    ];

    let block = Block::default()
        .title(Span::styled(" Metadata ", theme.section_title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
}

fn render_details_panel(frame: &mut Frame, area: Rect, detail_state: &DetailViewState) {
    let theme = default_theme();
    let mut lines: Vec<Line<'_>> = Vec::new();

    // Node / IP
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
        if section.chars().all(|c| c.is_uppercase() || c == '_' || c == ' ') {
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

    if !detail_state.events.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(" EVENTS", theme.section_title_style())));
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
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
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
            vec![Line::from(Span::styled(" No container metrics", theme.inactive_style()))]
        } else {
            pm.containers
                .iter()
                .map(|c| {
                    Line::from(vec![
                        Span::styled(format!(" {} ", c.name), theme.hover_style()),
                        Span::styled(format!("cpu={}", c.cpu), Style::default().fg(theme.accent)),
                        Span::styled("  ", theme.inactive_style()),
                        Span::styled(format!("mem={}", c.memory), Style::default().fg(theme.accent2)),
                    ])
                })
                .collect()
        }
    } else {
        vec![Line::from(Span::styled(" Metrics unavailable", theme.inactive_style()))]
    };

    let block = Block::default()
        .title(Span::styled(" Metrics ", theme.section_title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
}

fn render_yaml_panel(frame: &mut Frame, area: Rect, detail_state: &DetailViewState) {
    let theme = default_theme();

    let block = Block::default()
        .title(Span::styled(" YAML ", theme.section_title_style()))
        .title_bottom(Line::from(vec![
            Span::styled(" [j/k] ", theme.keybind_key_style()),
            Span::styled("scroll  ", theme.keybind_desc_style()),
            Span::styled("[g/G] ", theme.keybind_key_style()),
            Span::styled("top/bottom  ", theme.keybind_desc_style()),
            Span::styled("[PgUp/PgDn] ", theme.keybind_key_style()),
            Span::styled("page ", theme.keybind_desc_style()),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_active_style())
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if detail_state.loading {
        frame.render_widget(
            Paragraph::new(Span::styled(" ⟳ Loading YAML…", theme.badge_warning_style())),
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

    let yaml_str = match &detail_state.yaml {
        Some(y) => y.as_str(),
        None => {
            frame.render_widget(
                Paragraph::new(Span::styled(" YAML not available", theme.inactive_style())),
                inner,
            );
            return;
        }
    };

    let all_lines = highlight_yaml(yaml_str, &theme);
    let total = all_lines.len();
    let visible_height = inner.height as usize;

    // Clamp scroll
    let scroll = detail_state
        .yaml_scroll
        .min(total.saturating_sub(visible_height));

    let line_num_width = total.to_string().len().max(2);
    // Reserve: line_num_width + 2 (space + │ + space)
    let gutter_width = (line_num_width + 2) as u16;
    let scrollbar_width = 1u16;

    // Split inner into [gutter | content | scrollbar]
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(gutter_width),
            Constraint::Min(4),
            Constraint::Length(scrollbar_width),
        ])
        .split(inner);

    // Render line numbers
    let num_lines: Vec<Line> = (scroll..scroll + visible_height.min(total.saturating_sub(scroll)))
        .map(|i| {
            Line::from(Span::styled(
                format!("{:>width$}│", i + 1, width = line_num_width),
                Style::default().fg(theme.muted),
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(num_lines), cols[0]);

    // Render highlighted YAML lines
    let visible_lines: Vec<Line> = all_lines
        .into_iter()
        .skip(scroll)
        .take(visible_height)
        .collect();
    frame.render_widget(Paragraph::new(visible_lines), cols[1]);

    // Render scrollbar
    if total > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");
        let mut scrollbar_state = ScrollbarState::new(total).position(scroll);
        frame.render_stateful_widget(
            scrollbar,
            cols[2].inner(Margin { vertical: 0, horizontal: 0 }),
            &mut scrollbar_state,
        );
    }
}

// ─── Main detail modal ────────────────────────────────────────────────────────

/// Renders resource detail as a centered modal overlay.
pub fn render_detail(frame: &mut Frame, area: Rect, detail_state: &DetailViewState) {
    if let Some(viewer) = &detail_state.logs_viewer {
        render_logs_overlay(frame, area, viewer);
        return;
    }

    if let Some(dialog) = &detail_state.port_forward_dialog {
        dialog.render(frame, area);
        return;
    }

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

    // Layout: header | info-row | yaml (fills rest) | footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),  // header
            Constraint::Length(9),  // 3-column info row
            Constraint::Min(6),     // YAML panel
            Constraint::Length(2),  // footer
        ])
        .split(inner);

    // ── Header ──
    let (kind_label, name_label) = if let Some(resource) = &detail_state.resource {
        (resource.kind().to_ascii_uppercase(), resource.name().to_string())
    } else {
        ("RESOURCE".to_string(), "unknown".to_string())
    };

    let header_line = Line::from(vec![
        Span::styled(" ◆ ", theme.title_style()),
        Span::styled(kind_label, theme.title_style()),
        Span::styled("  /  ", theme.muted_style()),
        Span::styled(name_label, Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)),
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

    // ── 3-column info row ──
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

    // ── YAML panel ──
    render_yaml_panel(frame, chunks[2], detail_state);

    // ── Footer ──
    let mut footer_spans = Vec::new();

    let is_pod = matches!(detail_state.resource.as_ref(), Some(ResourceRef::Pod(_, _)));
    let is_scalable = matches!(
        detail_state.resource.as_ref(),
        Some(ResourceRef::Deployment(_, _) | ResourceRef::StatefulSet(_, _))
    );
    let is_restartable = matches!(
        detail_state.resource.as_ref(),
        Some(ResourceRef::Deployment(_, _) | ResourceRef::StatefulSet(_, _) | ResourceRef::DaemonSet(_, _))
    );
    let has_yaml = detail_state.yaml.is_some();

    if is_pod {
        footer_spans.push(Span::styled(" [l] ", theme.keybind_key_style()));
        footer_spans.push(Span::styled("Logs  ", theme.keybind_desc_style()));
        footer_spans.push(Span::styled("[f] ", theme.keybind_key_style()));
        footer_spans.push(Span::styled("Port-Fwd  ", theme.keybind_desc_style()));
        footer_spans.push(Span::styled("[p] ", theme.keybind_key_style()));
        footer_spans.push(Span::styled("Probes  ", theme.keybind_desc_style()));
    }
    if is_scalable {
        footer_spans.push(Span::styled("[s] ", theme.keybind_key_style()));
        footer_spans.push(Span::styled("Scale  ", theme.keybind_desc_style()));
    }
    if is_restartable {
        footer_spans.push(Span::styled("[R] ", theme.keybind_key_style()));
        footer_spans.push(Span::styled("Restart  ", theme.keybind_desc_style()));
    }
    if has_yaml {
        footer_spans.push(Span::styled("[e] ", theme.keybind_key_style()));
        footer_spans.push(Span::styled("Edit  ", theme.keybind_desc_style()));
    }
    footer_spans.push(Span::styled("[j/k] ", theme.keybind_key_style()));
    footer_spans.push(Span::styled("Scroll  ", theme.keybind_desc_style()));
    footer_spans.push(Span::styled("[Esc] ", theme.keybind_key_style()));
    footer_spans.push(Span::styled("Close", theme.keybind_desc_style()));

    if footer_spans.is_empty() {
        footer_spans.push(Span::styled(" [Esc] ", theme.keybind_key_style()));
        footer_spans.push(Span::styled("Close", theme.keybind_desc_style()));
    } else {
        // Prepend a space for padding
        footer_spans.insert(0, Span::raw(" "));
    }

    let footer_line = Line::from(footer_spans);
    let footer_block = Block::default()
        .borders(Borders::TOP)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.statusbar_bg));
    frame.render_widget(Paragraph::new(footer_line).block(footer_block), chunks[3]);
}

// ─── Logs overlay ─────────────────────────────────────────────────────────────

fn render_logs_overlay(frame: &mut Frame, area: Rect, viewer: &crate::app::LogsViewerState) {
    let theme = default_theme();
    let popup = centered_rect(94, 92, area);
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
        .constraints([Constraint::Length(2), Constraint::Min(4), Constraint::Length(2)])
        .split(inner);

    let title_line = Line::from(vec![
        Span::styled(" 📋 ", theme.title_style()),
        Span::styled(viewer.pod_name.clone(), theme.title_style()),
        Span::styled(" · ", theme.inactive_style()),
        Span::styled(viewer.pod_namespace.clone(), Style::default().fg(theme.accent2)),
        if !viewer.container_name.is_empty() {
            Span::styled(
                format!("  [{}]", viewer.container_name),
                Style::default().fg(theme.accent),
            )
        } else if viewer.picking_container {
            Span::styled(
                format!("  {} containers", viewer.containers.len()),
                Style::default().fg(theme.warning),
            )
        } else {
            Span::raw("")
        },
        Span::styled(format!("  {} lines", viewer.lines.len()), theme.inactive_style()),
        if viewer.loading {
            Span::styled("  ⟳ Loading…", Style::default().fg(theme.warning))
        } else {
            Span::raw("")
        },
    ]);
    let title_block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.header_bg));
    frame.render_widget(Paragraph::new(title_line).block(title_block), chunks[0]);

    let log_block = Block::default()
        .borders(Borders::NONE)
        .style(Style::default().bg(theme.bg));
    let log_inner = log_block.inner(chunks[1]);
    frame.render_widget(log_block, chunks[1]);

    if viewer.loading && viewer.lines.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("  ⟳ Fetching logs…", Style::default().fg(theme.warning))),
            log_inner,
        );
    } else if let Some(ref err) = viewer.error {
        frame.render_widget(
            Paragraph::new(Span::styled(format!("  ✗ {err}"), theme.badge_error_style())),
            log_inner,
        );
    } else if viewer.picking_container {
        // Container picker — shown when pod has multiple containers
        let lines: Vec<Line> = viewer.containers.iter().enumerate().map(|(i, name)| {
            if i == viewer.container_cursor {
                Line::from(vec![
                    Span::styled(" ▶ ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
                    Span::styled(name.clone(), Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)),
                ])
            } else {
                Line::from(vec![
                    Span::styled("   ", theme.inactive_style()),
                    Span::styled(name.clone(), Style::default().fg(theme.fg_dim)),
                ])
            }
        }).collect();

        let picker_block = Block::default()
            .title(Span::styled(" Select Container ", theme.section_title_style()))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_active_style())
            .style(Style::default().bg(theme.bg));
        frame.render_widget(
            Paragraph::new(lines).block(picker_block),
            log_inner,
        );
    } else if viewer.lines.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("  No logs available", theme.inactive_style())),
            log_inner,
        );
    } else {
        let visible = log_inner.height as usize;
        let total = viewer.lines.len();
        let start = viewer.scroll_offset.min(total.saturating_sub(1));
        let line_num_width = total.to_string().len().max(3);

        let lines: Vec<Line> = viewer.lines[start..]
            .iter()
            .take(visible)
            .enumerate()
            .map(|(i, content)| {
                let num = format!("{:>width$} ", start + i + 1, width = line_num_width);
                let upper = content.to_uppercase();
                let content_style = if upper.contains("ERROR") || upper.contains(" ERR ") {
                    Style::default().fg(theme.error)
                } else if upper.contains("WARN") {
                    Style::default().fg(theme.warning)
                } else if upper.contains("INFO") {
                    Style::default().fg(theme.fg)
                } else {
                    Style::default().fg(theme.fg_dim)
                };
                Line::from(vec![
                    Span::styled(num, theme.inactive_style()),
                    Span::styled("│ ", theme.inactive_style()),
                    Span::styled(content.clone(), content_style),
                ])
            })
            .collect();

        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            log_inner,
        );

        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");
        let mut scrollbar_state = ScrollbarState::new(total).position(start);
        frame.render_stateful_widget(
            scrollbar,
            chunks[1].inner(Margin { vertical: 0, horizontal: 0 }),
            &mut scrollbar_state,
        );
    }

    let footer_line = if viewer.picking_container {
        Line::from(vec![
            Span::styled(" [j/k] ", theme.keybind_key_style()),
            Span::styled("select  ", theme.keybind_desc_style()),
            Span::styled("[Enter] ", theme.keybind_key_style()),
            Span::styled("confirm  ", theme.keybind_desc_style()),
            Span::styled("[Esc] ", theme.keybind_key_style()),
            Span::styled("close", theme.keybind_desc_style()),
        ])
    } else {
        Line::from(vec![
            Span::styled(" [j/k] ", theme.keybind_key_style()),
            Span::styled("scroll  ", theme.keybind_desc_style()),
            Span::styled("[g/G] ", theme.keybind_key_style()),
            Span::styled("top/bottom  ", theme.keybind_desc_style()),
            Span::styled("[Esc] ", theme.keybind_key_style()),
            Span::styled("close", theme.keybind_desc_style()),
        ])
    };
    let footer_block = Block::default()
        .borders(Borders::TOP)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.statusbar_bg));
    frame.render_widget(Paragraph::new(footer_line).block(footer_block), chunks[2]);
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

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
