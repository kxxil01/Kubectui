//! Detail modal renderer.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, HighlightSpacing, Paragraph, Row, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Table, TableState, Wrap,
    },
};

use crate::{
    app::DetailViewState,
    ui::{
        components::{
            default_theme, probe_panel::render_probe_panel, render_debug_container_dialog,
            render_node_debug_dialog, render_vertical_scrollbar, scale_dialog::render_scale_dialog,
        },
        format_age, table_window, wrapped_line_count,
    },
};

fn render_scrollable_detail_panel<'a>(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    lines: Vec<Line<'a>>,
    scroll: usize,
) {
    let theme = default_theme();
    let block = Block::default()
        .title(Span::styled(title, theme.section_title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (total, position) = detail_panel_scroll_metrics(&lines, inner, scroll);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((position.min(u16::MAX as usize) as u16, 0)),
        inner,
    );
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"))
        .track_symbol(Some("│"))
        .thumb_symbol("█");
    let mut scrollbar_state = ScrollbarState::new(total).position(position);
    frame.render_stateful_widget(scrollbar, inner, &mut scrollbar_state);
}

fn detail_panel_scroll_metrics(lines: &[Line<'_>], area: Rect, scroll: usize) -> (usize, usize) {
    let total = wrapped_line_count(lines, area.width);
    let position = scroll.min(total.saturating_sub(area.height.max(1) as usize));
    (total, position)
}

fn truncate_line_content(line: &Line<'_>, width: usize) -> Line<'static> {
    let text = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    Line::from(crate::ui::truncate_message(&text, width.max(1)).into_owned())
}

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
        let label_iter: Box<dyn Iterator<Item = _>> = if detail_state.metadata_expanded {
            Box::new(detail_state.metadata.labels.iter())
        } else {
            Box::new(detail_state.metadata.labels.iter().take(5))
        };
        for (k, v) in label_iter {
            let display_val = if detail_state.metadata_expanded || v.len() <= 50 {
                v.clone()
            } else {
                format!("{}…", &v[..v.floor_char_boundary(50)])
            };
            lines.push(Line::from(vec![
                Span::styled("   ", theme.inactive_style()),
                Span::styled(k.clone(), Style::default().fg(theme.accent)),
                Span::styled("=", Style::default().fg(theme.muted)),
                Span::styled(display_val, Style::default().fg(theme.fg_dim)),
            ]));
        }
        if !detail_state.metadata_expanded && detail_state.metadata.labels.len() > 5 {
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
        let annot_iter: Box<dyn Iterator<Item = _>> = if detail_state.metadata_expanded {
            Box::new(detail_state.metadata.annotations.iter())
        } else {
            Box::new(detail_state.metadata.annotations.iter().take(3))
        };
        for (k, v) in annot_iter {
            let display_val = if detail_state.metadata_expanded || v.len() <= 50 {
                v.clone()
            } else {
                format!("{}…", &v[..v.floor_char_boundary(50)])
            };
            lines.push(Line::from(vec![
                Span::styled("   ", theme.inactive_style()),
                Span::styled(k.clone(), Style::default().fg(theme.accent)),
                Span::styled("=", Style::default().fg(theme.muted)),
                Span::styled(display_val, Style::default().fg(theme.fg_dim)),
            ]));
        }
        if !detail_state.metadata_expanded && detail_state.metadata.annotations.len() > 3 {
            lines.push(Line::from(Span::styled(
                format!("   … +{} more", detail_state.metadata.annotations.len() - 3),
                Style::default().fg(theme.muted),
            )));
        }
    }

    // Show expand/collapse hint if there's truncatable content
    let has_truncated = (!detail_state.metadata.labels.is_empty()
        && detail_state.metadata.labels.len() > 5)
        || (!detail_state.metadata.annotations.is_empty()
            && detail_state.metadata.annotations.len() > 3);
    if has_truncated || detail_state.metadata_expanded {
        let hint = if detail_state.metadata_expanded {
            " [m] collapse"
        } else {
            " [m] expand all"
        };
        lines.push(Line::from(Span::styled(
            hint,
            Style::default().fg(theme.muted),
        )));
    }

    render_scrollable_detail_panel(
        frame,
        area,
        " Metadata ",
        lines,
        detail_state.top_panel_scroll,
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

    render_scrollable_detail_panel(
        frame,
        area,
        " Details ",
        lines,
        detail_state.top_panel_scroll,
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

    render_scrollable_detail_panel(
        frame,
        area,
        " Metrics ",
        lines,
        detail_state.top_panel_scroll,
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

    if matches!(
        detail_state.resource.as_ref(),
        Some(crate::app::ResourceRef::CronJob(_, _))
    ) {
        render_cronjob_history_panel(frame, inner, detail_state);
        return;
    }

    let yaml_lines = detail_state
        .yaml
        .as_ref()
        .map(|yaml| yaml.lines().count())
        .unwrap_or(0);

    let yaml_error_line = detail_state.yaml_error.as_ref().map(|e| {
        Line::from(vec![
            Span::styled(" ✗ ", theme.badge_error_style()),
            Span::styled(e.clone(), Style::default().fg(theme.error)),
        ])
    });

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
        if let Some(err_line) = yaml_error_line {
            err_line
        } else {
            Line::from(vec![
                Span::styled(" YAML lines ", theme.inactive_style()),
                Span::styled(yaml_lines.to_string(), Style::default().fg(theme.fg)),
            ])
        },
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

fn render_cronjob_history_panel(frame: &mut Frame, area: Rect, detail_state: &DetailViewState) {
    let theme = default_theme();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3)])
        .split(area);

    let mut primary_hint = vec![
        Span::styled(" [j/k] ", theme.keybind_key_style()),
        Span::styled("Select run  ", theme.keybind_desc_style()),
        Span::styled("[Enter] ", theme.keybind_key_style()),
        Span::styled("Open Job", theme.keybind_desc_style()),
    ];
    if detail_state.supports_action(crate::policy::DetailAction::Logs) {
        primary_hint.extend([
            Span::styled("  [l] ", theme.keybind_key_style()),
            Span::styled("Logs for selected Job", theme.keybind_desc_style()),
        ]);
    }

    let mut secondary_hint = Vec::new();
    if detail_state.supports_action(crate::policy::DetailAction::Trigger) {
        secondary_hint.extend([
            Span::styled(" [T] ", theme.keybind_key_style()),
            Span::styled("Trigger run", theme.keybind_desc_style()),
        ]);
    }
    if detail_state.supports_action(crate::policy::DetailAction::SuspendCronJob)
        || detail_state.supports_action(crate::policy::DetailAction::ResumeCronJob)
    {
        if !secondary_hint.is_empty() {
            secondary_hint.push(Span::raw("  "));
        }
        secondary_hint.extend([
            Span::styled("[S] ", theme.keybind_key_style()),
            Span::styled("Pause/resume schedule", theme.keybind_desc_style()),
        ]);
    }

    let hints = vec![
        Line::from(primary_hint),
        Line::from(if secondary_hint.is_empty() {
            vec![Span::styled(
                " No actions available",
                theme.inactive_style(),
            )]
        } else {
            secondary_hint
        }),
    ];
    frame.render_widget(Paragraph::new(hints).wrap(Wrap { trim: false }), rows[0]);

    if detail_state.cronjob_history.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    " No recent Jobs for this CronJob yet.",
                    theme.inactive_style(),
                )),
                Line::from(Span::styled(
                    " Trigger it with [T] to seed the execution history.",
                    theme.inactive_style(),
                )),
            ])
            .wrap(Wrap { trim: false }),
            rows[1],
        );
        return;
    }

    let total = detail_state.cronjob_history.len();
    let selected = detail_state
        .cronjob_history_selected
        .min(total.saturating_sub(1));
    let viewport_rows = usize::from(rows[1].height.saturating_sub(1)).max(1);
    let window = table_window(total, selected, viewport_rows);
    let header = Row::new([
        Cell::from(Span::styled("Job", theme.header_style())),
        Cell::from(Span::styled("Status", theme.header_style())),
        Cell::from(Span::styled("Duration", theme.header_style())),
        Cell::from(Span::styled("Pods", theme.header_style())),
        Cell::from(Span::styled("Done", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let table_rows = detail_state.cronjob_history[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(offset, entry)| {
            let idx = window.start + offset;
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let status_style = theme.get_status_style(&entry.status);
            let pod_style = if entry.failed_pods > 0 {
                theme.badge_error_style()
            } else if entry.active_pods > 0 {
                theme.badge_warning_style()
            } else {
                Style::default().fg(theme.fg_dim)
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    entry.job_name.as_str(),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(entry.status.as_str(), status_style)),
                Cell::from(Span::styled(
                    entry.duration.as_deref().unwrap_or("-"),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(entry.pod_count.to_string(), pod_style)),
                Cell::from(Span::styled(
                    entry
                        .completion_pct
                        .map(|pct| format!("{pct}%"))
                        .unwrap_or_else(|| "-".to_string()),
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(Span::styled(
                    format_age(entry.age),
                    Style::default().fg(theme.fg_dim),
                )),
            ])
            .style(row_style)
        })
        .collect::<Vec<_>>();

    let widths = cronjob_history_widths(rows[1]);
    let mut table_state = TableState::default().with_selected(Some(window.selected));
    let table = Table::new(table_rows, widths)
        .header(header)
        .row_highlight_style(theme.selection_style())
        .highlight_symbol(theme.highlight_symbol())
        .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(table, rows[1], &mut table_state);
    render_vertical_scrollbar(frame, rows[1], total, window.start);
}

const NARROW_CRONJOB_HISTORY_WIDTH: u16 = 88;

fn cronjob_history_widths(area: Rect) -> [Constraint; 6] {
    if area.width < NARROW_CRONJOB_HISTORY_WIDTH {
        [
            Constraint::Min(18),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Length(7),
        ]
    } else {
        [
            Constraint::Percentage(35),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Length(8),
        ]
    }
}

fn render_compact_detail(frame: &mut Frame, inner: Rect, detail_state: &DetailViewState) {
    let theme = default_theme();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
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
    let header = Line::from(vec![
        Span::styled(kind_label, theme.title_style()),
        Span::styled(" / ", theme.muted_style()),
        Span::styled(
            name_label,
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        if detail_state.loading {
            Span::styled("  loading", theme.badge_warning_style())
        } else {
            Span::raw("")
        },
    ]);
    frame.render_widget(
        Paragraph::new(header).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(theme.border_style()),
        ),
        rows[0],
    );

    let mut lines = vec![
        Line::from(vec![
            Span::styled("ns ", theme.inactive_style()),
            Span::styled(
                detail_state
                    .metadata
                    .namespace
                    .as_deref()
                    .unwrap_or("cluster-scope"),
                Style::default().fg(theme.accent2),
            ),
            Span::styled("  status ", theme.inactive_style()),
            Span::styled(
                detail_state.metadata.status.as_deref().unwrap_or("Unknown"),
                theme
                    .get_status_style(detail_state.metadata.status.as_deref().unwrap_or("Unknown")),
            ),
        ]),
        Line::from(vec![
            Span::styled("labels ", theme.inactive_style()),
            Span::styled(
                detail_state.metadata.labels.len().to_string(),
                Style::default().fg(theme.fg),
            ),
            Span::styled("  events ", theme.inactive_style()),
            Span::styled(
                detail_state.events.len().to_string(),
                Style::default().fg(theme.fg),
            ),
        ]),
    ];

    if detail_state.loading {
        lines.push(Line::from(Span::styled(
            "Loading resource details...",
            theme.badge_warning_style(),
        )));
    } else if let Some(err) = &detail_state.error {
        lines.push(Line::from(Span::styled(
            format!("Error: {err}"),
            theme.badge_error_style(),
        )));
    } else if let Some(msg) = &detail_state.metrics_unavailable_message {
        lines.push(Line::from(Span::styled(
            format!("Metrics: {msg}"),
            theme.inactive_style(),
        )));
    } else if let Some(metrics) = &detail_state.node_metrics {
        lines.push(Line::from(format!(
            "Metrics: cpu {}  mem {}",
            metrics.cpu, metrics.memory
        )));
    } else if let Some(metrics) = &detail_state.pod_metrics {
        lines.push(Line::from(format!(
            "Metrics: {} container(s)",
            metrics.containers.len()
        )));
    }

    if let Some(section) = detail_state
        .sections
        .iter()
        .find(|line| !line.trim().is_empty())
    {
        lines.push(Line::from(vec![
            Span::styled("detail ", theme.inactive_style()),
            Span::styled(section.as_str(), Style::default().fg(theme.fg_dim)),
        ]));
    }

    lines.push(Line::from(Span::styled(
        "Expand terminal for full metadata, metrics, and inspection panels.",
        theme.inactive_style(),
    )));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rows[1]);

    let action_keys = detail_state
        .footer_actions()
        .iter()
        .map(|action| action.key_hint())
        .collect::<Vec<_>>()
        .join("/");
    let footer = if action_keys.is_empty() {
        " Esc close ".to_string()
    } else {
        format!(" {action_keys} actions  Esc close ")
    };
    frame.render_widget(
        Paragraph::new(footer).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(theme.border_style()),
        ),
        rows[2],
    );
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

    if inner.width < 70 || inner.height < 19 {
        render_compact_detail(frame, inner, detail_state);
    } else {
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
    }

    if detail_state.confirm_delete {
        render_delete_confirm(frame, popup, detail_state);
    } else if detail_state.confirm_drain {
        render_drain_confirm(frame, popup, detail_state);
    } else if detail_state.confirm_cronjob_suspend.is_some() {
        render_cronjob_suspend_confirm(frame, popup, detail_state);
    } else if let Some(dialog) = &detail_state.debug_dialog {
        render_debug_container_dialog(frame, popup, dialog);
    } else if let Some(dialog) = &detail_state.node_debug_dialog {
        render_node_debug_dialog(frame, popup, dialog);
    }
}

fn render_delete_confirm(frame: &mut Frame, parent: Rect, detail_state: &DetailViewState) {
    let theme = default_theme();
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
    let footer = Line::from(vec![
        Span::styled(" [D] / [y] / [Enter] ", theme.keybind_key_style()),
        Span::styled("Confirm  ", theme.keybind_desc_style()),
        Span::styled("[F] ", theme.keybind_key_style()),
        Span::styled("Force  ", theme.keybind_desc_style()),
        Span::styled("[Esc] ", theme.keybind_key_style()),
        Span::styled("Cancel", theme.keybind_desc_style()),
    ]);
    render_detail_confirm_dialog(
        frame,
        parent,
        48,
        " Confirm Delete ",
        theme.badge_error_style().add_modifier(Modifier::BOLD),
        body,
        footer,
    );
}

fn render_drain_confirm(frame: &mut Frame, parent: Rect, detail_state: &DetailViewState) {
    let theme = default_theme();
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
    let footer = Line::from(vec![
        Span::styled(" [D] / [y] / [Enter] ", theme.keybind_key_style()),
        Span::styled("Drain  ", theme.keybind_desc_style()),
        Span::styled("[F] ", theme.keybind_key_style()),
        Span::styled("Force drain  ", theme.keybind_desc_style()),
        Span::styled("[Esc] ", theme.keybind_key_style()),
        Span::styled("Cancel", theme.keybind_desc_style()),
    ]);
    render_detail_confirm_dialog(
        frame,
        parent,
        52,
        " Confirm Drain ",
        theme.badge_warning_style().add_modifier(Modifier::BOLD),
        body,
        footer,
    );
}

fn render_cronjob_suspend_confirm(frame: &mut Frame, parent: Rect, detail_state: &DetailViewState) {
    let theme = default_theme();
    let suspend = detail_state.confirm_cronjob_suspend.unwrap_or(false);
    let badge_style = if suspend {
        theme.badge_warning_style()
    } else {
        theme.badge_success_style()
    };
    let title = if suspend {
        " Confirm Pause "
    } else {
        " Confirm Resume "
    };

    let cronjob_name = detail_state
        .resource
        .as_ref()
        .map(|r| r.name().to_string())
        .unwrap_or_else(|| "selected cronjob".to_string());
    let action_label = if suspend {
        "Pause schedule"
    } else {
        "Resume schedule"
    };
    let detail_line = if suspend {
        " New Jobs will not be scheduled until you resume this CronJob."
    } else {
        " Scheduled runs will resume on the next matching cron window."
    };

    let body = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(format!(" {action_label} "), badge_style),
            Span::styled(format!("'{cronjob_name}'"), Style::default().fg(theme.fg)),
            Span::styled(" ?", Style::default().fg(theme.fg)),
        ]),
        Line::from(""),
        Line::from(Span::styled(detail_line, theme.inactive_style())),
    ];
    let footer = Line::from(vec![
        Span::styled(" [S] / [y] / [Enter] ", theme.keybind_key_style()),
        Span::styled(format!("{action_label}  "), theme.keybind_desc_style()),
        Span::styled("[Esc] ", theme.keybind_key_style()),
        Span::styled("Cancel", theme.keybind_desc_style()),
    ]);
    render_detail_confirm_dialog(
        frame,
        parent,
        56,
        title,
        badge_style.add_modifier(Modifier::BOLD),
        body,
        footer,
    );
}

fn detail_confirm_popup(parent: Rect, preferred_width: u16) -> Rect {
    crate::ui::bounded_popup_rect(parent, preferred_width, 10, 1, 1)
}

fn use_compact_detail_confirm(popup: Rect) -> bool {
    popup.width < 42 || popup.height < 8
}

fn render_detail_confirm_dialog(
    frame: &mut Frame,
    parent: Rect,
    preferred_width: u16,
    title: &str,
    title_style: Style,
    body: Vec<Line<'static>>,
    footer: Line<'static>,
) {
    let theme = default_theme();
    let popup = detail_confirm_popup(parent, preferred_width);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(title, title_style))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_active_style())
        .style(Style::default().bg(theme.bg));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    if use_compact_detail_confirm(popup) {
        let width = usize::from(inner.width.max(1));
        let mut lines = body
            .into_iter()
            .map(|line| truncate_line_content(&line, width))
            .collect::<Vec<_>>();
        lines.push(Line::from(""));
        lines.push(truncate_line_content(&footer, width));
        frame.render_widget(
            Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
            inner,
        );
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(inner);
    frame.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .alignment(ratatui::layout::Alignment::Center),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(footer).alignment(ratatui::layout::Alignment::Center),
        rows[1],
    );
}

use crate::ui::centered_rect;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ResourceRef;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn cronjob_history_widths_switch_to_compact_profile() {
        let widths = cronjob_history_widths(Rect::new(0, 0, 80, 20));
        assert_eq!(widths[0], Constraint::Min(18));
        assert_eq!(widths[1], Constraint::Length(10));
        assert_eq!(widths[5], Constraint::Length(7));
    }

    #[test]
    fn cronjob_history_widths_keep_wide_profile() {
        let widths = cronjob_history_widths(Rect::new(0, 0, 120, 20));
        assert_eq!(widths[0], Constraint::Percentage(35));
        assert_eq!(widths[1], Constraint::Length(12));
        assert_eq!(widths[5], Constraint::Length(8));
    }

    #[test]
    fn render_delete_confirm_small_terminal_smoke() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");
        let state = DetailViewState {
            resource: Some(ResourceRef::Pod("pod-0".to_string(), "default".to_string())),
            confirm_delete: true,
            ..DetailViewState::default()
        };
        terminal
            .draw(|frame| render_detail(frame, frame.area(), &state))
            .expect("delete confirm should render on small terminal");
    }

    #[test]
    fn render_drain_confirm_small_terminal_smoke() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");
        let state = DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            confirm_drain: true,
            ..DetailViewState::default()
        };
        terminal
            .draw(|frame| render_detail(frame, frame.area(), &state))
            .expect("drain confirm should render on small terminal");
    }

    #[test]
    fn render_cronjob_suspend_confirm_small_terminal_smoke() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");
        let state = DetailViewState {
            resource: Some(ResourceRef::CronJob(
                "job-0".to_string(),
                "default".to_string(),
            )),
            confirm_cronjob_suspend: Some(true),
            ..DetailViewState::default()
        };
        terminal
            .draw(|frame| render_detail(frame, frame.area(), &state))
            .expect("cronjob suspend confirm should render on small terminal");
    }

    #[test]
    fn detail_panel_scroll_metrics_clamp_to_last_full_page() {
        let lines = vec![Line::from("row"); 20];
        assert_eq!(
            detail_panel_scroll_metrics(&lines, Rect::new(0, 0, 18, 6), 99),
            (20, 14)
        );
        let short = vec![Line::from("row"); 3];
        assert_eq!(
            detail_panel_scroll_metrics(&short, Rect::new(0, 0, 18, 6), 99),
            (3, 0)
        );
    }

    #[test]
    fn truncate_line_content_keeps_compact_confirm_single_row() {
        let line = Line::from(vec![
            Span::raw(" [D] / [y] / [Enter] "),
            Span::raw("Confirm  "),
            Span::raw("[F] "),
            Span::raw("Force  "),
            Span::raw("[Esc] "),
            Span::raw("Cancel"),
        ]);
        let truncated = truncate_line_content(&line, 16);
        let text = truncated
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.chars().count() <= 16);
    }
}
