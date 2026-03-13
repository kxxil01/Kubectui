//! Bottom workbench renderer.

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Tabs, Wrap,
    },
};

use crate::{
    action_history::{ActionHistoryEntry, ActionStatus},
    app::{AppState, Focus},
    secret::DecodedSecretValue,
    state::ClusterSnapshot,
    ui::components::default_theme,
    workbench::{WorkbenchTab, WorkbenchTabState},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VisibleWindow {
    start: usize,
    end: usize,
}

#[inline]
fn scroll_window(total: usize, scroll: usize, viewport_rows: usize) -> VisibleWindow {
    if total == 0 {
        return VisibleWindow { start: 0, end: 0 };
    }

    let visible = viewport_rows.max(1).min(total);
    let start = scroll.min(total.saturating_sub(visible));
    VisibleWindow {
        start,
        end: start + visible,
    }
}

#[inline]
fn centered_window(total: usize, selected: usize, viewport_rows: usize) -> VisibleWindow {
    if total == 0 {
        return VisibleWindow { start: 0, end: 0 };
    }

    let visible = viewport_rows.max(1).min(total);
    let start = selected
        .min(total.saturating_sub(1))
        .saturating_sub(visible.saturating_sub(1) / 2)
        .min(total.saturating_sub(visible));
    VisibleWindow {
        start,
        end: start + visible,
    }
}

pub fn render_workbench(frame: &mut Frame, area: Rect, app: &AppState, _cluster: &ClusterSnapshot) {
    let theme = default_theme();
    let workbench_focused = app.focus == Focus::Workbench;

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let titles: Vec<Line> = if app.workbench().tabs.is_empty() {
        vec![Line::from(Span::raw(" Empty "))]
    } else {
        app.workbench()
            .tabs
            .iter()
            .map(|tab| Line::from(Span::raw(format!(" {} ", tab.state.title()))))
            .collect()
    };
    let selected = app
        .workbench()
        .active_tab
        .min(titles.len().saturating_sub(1));

    let title = if workbench_focused && app.workbench().maximized {
        " Workbench ACTIVE [maximized] - Esc exits maximize, Esc again returns to resources "
    } else if workbench_focused {
        " Workbench ACTIVE - Esc returns to resources "
    } else if app.workbench().maximized {
        " Workbench [maximized] "
    } else {
        " Workbench "
    };
    let border_style = if workbench_focused {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        theme.border_style()
    };
    let title_style = if workbench_focused {
        Style::default()
            .fg(theme.bg)
            .bg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        theme.section_title_style()
    };
    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .title(Span::styled(title, title_style))
                .borders(Borders::ALL)
                .border_type(theme.border_type())
                .border_style(border_style)
                .style(Style::default().bg(theme.bg_surface)),
        )
        .select(selected)
        .style(Style::default().fg(theme.tab_inactive_fg))
        .highlight_style(
            Style::default()
                .fg(theme.tab_active_fg)
                .bg(theme.tab_active_bg)
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::styled("│", theme.muted_style()));
    frame.render_widget(tabs, sections[0]);

    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_type(theme.border_type())
        .border_style(border_style)
        .style(Style::default().bg(theme.bg));
    let inner = block.inner(sections[1]);
    frame.render_widget(block, sections[1]);

    let Some(active_tab) = app.workbench().active_tab() else {
        render_empty_state(frame, inner);
        return;
    };

    match &active_tab.state {
        WorkbenchTabState::ActionHistory(tab) => render_action_history_tab(frame, inner, app, tab),
        WorkbenchTabState::ResourceYaml(tab) => {
            render_yaml_tab(frame, inner, tab.scroll, active_tab)
        }
        WorkbenchTabState::DecodedSecret(tab) => render_decoded_secret_tab(frame, inner, tab),
        WorkbenchTabState::ResourceEvents(tab) => {
            render_events_tab(frame, inner, tab.scroll, active_tab)
        }
        WorkbenchTabState::PodLogs(tab) => {
            render_logs_tab(frame, inner, active_tab, tab.viewer.scroll_offset)
        }
        WorkbenchTabState::WorkloadLogs(tab) => render_workload_logs_tab(frame, inner, tab),
        WorkbenchTabState::Exec(tab) => render_exec_tab(frame, inner, tab),
        WorkbenchTabState::PortForward(tab) => tab.dialog.render_embedded(frame, inner),
        WorkbenchTabState::Relations(tab) => {
            crate::ui::views::relations::render_relations_tab(frame, inner, tab, &theme)
        }
    }
}

fn render_empty_state(frame: &mut Frame, area: Rect) {
    let theme = default_theme();
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![Span::styled(
                " No workbench tabs open",
                theme.section_title_style(),
            )]),
            Line::from(""),
            Line::from("  Open a resource tab with:"),
            Line::from(
                "  [y] YAML  [o] Decoded  [v] Timeline  [l] Logs  [x] Exec  [f] Port-Forward",
            ),
            Line::from(""),
            Line::from("  [H] opens action history."),
            Line::from("  [b] closes the workbench, [Ctrl+W] closes the active tab."),
        ])
        .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_action_history_tab(
    frame: &mut Frame,
    area: Rect,
    app: &AppState,
    tab: &crate::workbench::ActionHistoryTabState,
) {
    let theme = default_theme();
    let entries = app.action_history().entries();

    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![Span::styled(
                    " No mutation history yet",
                    theme.section_title_style(),
                )]),
                Line::from(""),
                Line::from("  Mutating actions will appear here with pending/success/error state."),
                Line::from("  Use [Enter] on a jumpable row to reopen the affected resource."),
            ])
            .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }

    let window = centered_window(entries.len(), tab.selected, area.height.max(1) as usize);
    let lines: Vec<Line> = entries
        .iter()
        .enumerate()
        .skip(window.start)
        .take(window.end.saturating_sub(window.start))
        .map(|(idx, entry)| render_action_history_line(entry, idx == tab.selected, &theme))
        .collect();

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    render_scrollbar(frame, area, entries.len(), window.start);
}

fn render_action_history_line(
    entry: &ActionHistoryEntry,
    selected: bool,
    theme: &crate::ui::theme::Theme,
) -> Line<'static> {
    let badge = match entry.status {
        ActionStatus::Pending => Span::styled(" PENDING ", theme.badge_warning_style()),
        ActionStatus::Succeeded => Span::styled(" OK ", theme.badge_success_style()),
        ActionStatus::Failed => Span::styled(" ERROR ", theme.badge_error_style()),
    };
    let timestamp = entry
        .finished_at
        .unwrap_or(entry.started_at)
        .with_timezone(&chrono::Local)
        .format("%H:%M:%S")
        .to_string();
    let row_style = if selected {
        theme.hover_style()
    } else {
        Style::default().fg(theme.fg)
    };
    let jump_hint = if entry.target.is_some() {
        "  [Enter] open"
    } else {
        ""
    };

    Line::from(vec![
        Span::styled(if selected { "› " } else { "  " }, row_style),
        badge,
        Span::raw(" "),
        Span::styled(
            format!("{} ", entry.kind.label()),
            row_style.add_modifier(Modifier::BOLD),
        ),
        Span::styled(entry.resource_label.clone(), row_style),
        Span::styled("  ", row_style),
        Span::styled(timestamp, theme.muted_style()),
        Span::styled(jump_hint, theme.keybind_desc_style()),
        Span::styled(
            format!("  {}", entry.message),
            Style::default().fg(theme.fg_dim),
        ),
    ])
}

/// Syntax-highlight a single YAML line into styled spans.
fn highlight_yaml_line<'a>(line: &'a str, theme: &crate::ui::theme::Theme) -> Vec<Span<'a>> {
    let trimmed = line.trim_start();

    // Comment lines
    if trimmed.starts_with('#') {
        return vec![Span::styled(
            line.to_string(),
            Style::default().fg(theme.muted),
        )];
    }

    // Separator lines (---)
    if trimmed == "---" {
        return vec![Span::styled(
            line.to_string(),
            Style::default().fg(theme.muted),
        )];
    }

    // List items: "  - value"
    if let Some(rest) = trimmed.strip_prefix("- ") {
        let indent = line.len() - trimmed.len();
        let mut spans = Vec::with_capacity(3);
        if indent > 0 {
            spans.push(Span::raw(&line[..indent]));
        }
        spans.push(Span::styled("- ", Style::default().fg(theme.muted)));
        // Check if the rest is a key: value pair
        if let Some(colon_pos) = rest.find(": ") {
            spans.push(Span::styled(
                rest[..colon_pos].to_string(),
                Style::default().fg(theme.accent),
            ));
            spans.push(Span::styled(": ", Style::default().fg(theme.muted)));
            spans.extend(highlight_yaml_value(&rest[colon_pos + 2..], theme));
        } else {
            spans.push(Span::raw(rest.to_string()));
        }
        return spans;
    }

    // Key: value lines
    if let Some(colon_pos) = trimmed.find(": ") {
        let indent = line.len() - trimmed.len();
        let mut spans = Vec::with_capacity(4);
        if indent > 0 {
            spans.push(Span::raw(&line[..indent]));
        }
        spans.push(Span::styled(
            trimmed[..colon_pos].to_string(),
            Style::default().fg(theme.accent),
        ));
        spans.push(Span::styled(": ", Style::default().fg(theme.muted)));
        spans.extend(highlight_yaml_value(&trimmed[colon_pos + 2..], theme));
        return spans;
    }

    // Key-only lines ending with ":"
    if trimmed.ends_with(':') && !trimmed.is_empty() {
        let indent = line.len() - trimmed.len();
        let mut spans = Vec::with_capacity(2);
        if indent > 0 {
            spans.push(Span::raw(&line[..indent]));
        }
        spans.push(Span::styled(
            trimmed.to_string(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
        return spans;
    }

    // Plain text fallback
    vec![Span::raw(line.to_string())]
}

/// Highlight a YAML value based on type (bool, null, number, string).
fn highlight_yaml_value<'a>(value: &'a str, theme: &crate::ui::theme::Theme) -> Vec<Span<'a>> {
    let v = value.trim();
    match v {
        "true" | "false" => vec![Span::styled(
            value.to_string(),
            Style::default().fg(theme.success),
        )],
        "null" | "~" => vec![Span::styled(
            value.to_string(),
            Style::default().fg(theme.muted),
        )],
        _ if v.starts_with('"') || v.starts_with('\'') => vec![Span::styled(
            value.to_string(),
            Style::default().fg(theme.warning),
        )],
        _ if v.parse::<f64>().is_ok() => vec![Span::styled(
            value.to_string(),
            Style::default().fg(theme.accent2),
        )],
        _ => vec![Span::raw(value.to_string())],
    }
}

fn render_yaml_tab(frame: &mut Frame, area: Rect, scroll: usize, tab: &WorkbenchTab) {
    let theme = default_theme();
    let WorkbenchTabState::ResourceYaml(tab_state) = &tab.state else {
        return;
    };

    if tab_state.loading {
        frame.render_widget(
            Paragraph::new(Span::styled(" Loading YAML...", theme.inactive_style())),
            area,
        );
        return;
    }

    if let Some(error) = &tab_state.error {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" Error: {error}"),
                theme.badge_error_style(),
            )),
            area,
        );
        return;
    }

    let Some(yaml) = &tab_state.yaml else {
        frame.render_widget(
            Paragraph::new(Span::styled(" YAML not available", theme.inactive_style())),
            area,
        );
        return;
    };

    let total = yaml.lines().count();
    let window = scroll_window(total, scroll, area.height.saturating_sub(1) as usize);
    let body = if window.start < window.end {
        yaml.lines()
            .enumerate()
            .skip(window.start)
            .take(window.end.saturating_sub(window.start))
            .map(|(idx, line)| {
                let mut spans = vec![Span::styled(
                    format!("{:>4} ", idx + 1),
                    theme.muted_style(),
                )];
                spans.extend(highlight_yaml_line(line, &theme));
                Line::from(spans)
            })
            .collect()
    } else {
        vec![Line::from("")]
    };

    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), area);
    render_scrollbar(frame, area, total, window.start);
}

fn render_decoded_secret_tab(
    frame: &mut Frame,
    area: Rect,
    tab_state: &crate::workbench::DecodedSecretTabState,
) {
    let theme = default_theme();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(area);

    let visibility = if tab_state.masked {
        "masked"
    } else {
        "visible"
    };
    let dirty = if tab_state.has_unsaved_changes() {
        "  [unsaved changes]"
    } else {
        ""
    };
    let hint = if tab_state.editing {
        "[Enter] apply field  [Esc] cancel  [Ctrl+U] clear"
    } else {
        "[e] edit  [m] mask  [s] save  [Esc] back"
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {visibility} "), theme.badge_warning_style()),
            Span::styled(dirty, theme.keybind_desc_style()),
            Span::raw("  "),
            Span::styled(hint, theme.keybind_desc_style()),
        ])),
        sections[0],
    );

    if tab_state.loading {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Loading decoded Secret data...",
                theme.inactive_style(),
            )),
            sections[1],
        );
        return;
    }

    if let Some(error) = &tab_state.error {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" Error: {error}"),
                theme.badge_error_style(),
            )),
            sections[1],
        );
        return;
    }

    if tab_state.entries.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Secret has no data entries",
                theme.inactive_style(),
            )),
            sections[1],
        );
        return;
    }

    let content_area = if tab_state.editing {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(2)])
            .split(sections[1]);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" edit> ", theme.section_title_style()),
                Span::raw(tab_state.edit_input.clone()),
            ]))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(theme.border_style()),
            ),
            split[1],
        );
        split[0]
    } else {
        sections[1]
    };

    let total = tab_state.entries.len();
    let window = centered_window(
        total,
        tab_state.selected,
        content_area.height.max(1) as usize,
    );
    let lines: Vec<Line> = tab_state.entries[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, entry)| {
            let selected = window.start + local_idx == tab_state.selected;
            let row_style = if selected {
                theme.hover_style()
            } else {
                Style::default().fg(theme.fg)
            };
            let value_span = match &entry.value {
                DecodedSecretValue::Text { current, .. } => {
                    if tab_state.masked {
                        Span::styled(
                            format!("[text {} chars] ****", current.chars().count()),
                            theme.muted_style(),
                        )
                    } else {
                        Span::styled(current.clone(), row_style)
                    }
                }
                DecodedSecretValue::Binary {
                    byte_len, preview, ..
                } => Span::styled(
                    format!("[binary {byte_len} bytes] {preview}"),
                    theme.muted_style(),
                ),
                DecodedSecretValue::InvalidBase64 {
                    error, replacement, ..
                } => Span::styled(
                    replacement
                        .as_ref()
                        .map(|value| format!("[repaired] {value}"))
                        .unwrap_or_else(|| format!("[invalid base64] {error}")),
                    theme.badge_error_style(),
                ),
            };

            Line::from(vec![
                Span::styled(if selected { "› " } else { "  " }, row_style),
                Span::styled(
                    format!("{:<20}", entry.key),
                    row_style.add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                value_span,
            ])
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        content_area,
    );
    render_scrollbar(frame, content_area, total, window.start);
}

fn render_events_tab(frame: &mut Frame, area: Rect, scroll: usize, tab: &WorkbenchTab) {
    use crate::timeline::TimelineEntry;

    let theme = default_theme();
    let WorkbenchTabState::ResourceEvents(tab_state) = &tab.state else {
        return;
    };

    if tab_state.loading {
        frame.render_widget(
            Paragraph::new(Span::styled(" Loading timeline...", theme.inactive_style())),
            area,
        );
        return;
    }

    if let Some(error) = &tab_state.error {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" Error: {error}"),
                theme.badge_error_style(),
            )),
            area,
        );
        return;
    }

    if tab_state.timeline.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " No timeline entries for this resource",
                theme.inactive_style(),
            )),
            area,
        );
        return;
    }

    let total = tab_state.timeline.len();
    let window = scroll_window(total, scroll, area.height.saturating_sub(1) as usize);

    // Only build Line objects for the visible window to avoid per-frame allocations
    // for off-screen entries.
    let lines: Vec<Line> = tab_state.timeline[window.start..window.end]
        .iter()
        .map(|entry| match entry {
            TimelineEntry::Event {
                event,
                correlated_action_idx,
            } => {
                let prefix = if correlated_action_idx.is_some() {
                    Span::styled("  ~ ", Style::default().fg(theme.accent2))
                } else {
                    Span::raw("    ")
                };
                let badge = if event.event_type.eq_ignore_ascii_case("warning") {
                    Span::styled(" WARN ", theme.badge_warning_style())
                } else {
                    Span::styled(" OK ", theme.badge_success_style())
                };
                let ts = event
                    .last_timestamp
                    .with_timezone(&chrono::Local)
                    .format("%H:%M:%S")
                    .to_string();
                Line::from(vec![
                    prefix,
                    badge,
                    Span::raw(" "),
                    Span::styled(
                        format!("{} (x{}) ", event.reason, event.count),
                        Style::default().add_modifier(Modifier::BOLD).fg(theme.fg),
                    ),
                    Span::styled(
                        crate::ui::truncate_message(&event.message, 60),
                        Style::default().fg(theme.fg_dim),
                    ),
                    Span::styled(format!("  {ts}"), theme.muted_style()),
                ])
            }
            TimelineEntry::Action {
                kind,
                status,
                message,
                started_at,
                ..
            } => {
                let status_badge = match status {
                    ActionStatus::Pending => Span::styled(" PENDING ", theme.badge_warning_style()),
                    ActionStatus::Succeeded => Span::styled(" OK ", theme.badge_success_style()),
                    ActionStatus::Failed => Span::styled(" ERROR ", theme.badge_error_style()),
                };
                let ts = started_at
                    .with_timezone(&chrono::Local)
                    .format("%H:%M:%S")
                    .to_string();
                Line::from(vec![
                    Span::styled(
                        ">>> ",
                        Style::default()
                            .fg(theme.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    status_badge,
                    Span::raw(" "),
                    Span::styled(
                        format!("{} ", kind.label()),
                        Style::default()
                            .fg(theme.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        crate::ui::truncate_message(message, 60),
                        Style::default().fg(theme.fg),
                    ),
                    Span::styled(format!("  {ts}"), theme.muted_style()),
                ])
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    render_scrollbar(frame, area, total, window.start);
}

fn render_logs_tab(frame: &mut Frame, area: Rect, tab: &WorkbenchTab, scroll: usize) {
    let theme = default_theme();
    let WorkbenchTabState::PodLogs(tab_state) = &tab.state else {
        return;
    };
    let viewer = &tab_state.viewer;

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(area);

    let status = if viewer.loading {
        "loading"
    } else if viewer.previous_logs {
        "previous"
    } else if viewer.picking_container {
        "select container"
    } else if viewer.follow_mode {
        "following"
    } else {
        "paused"
    };

    let container = if viewer.container_name.is_empty() {
        "container: pending".to_string()
    } else {
        format!("container: {}", viewer.container_name)
    };

    let mut status_spans = vec![
        Span::styled(format!(" {status} "), theme.badge_warning_style()),
        Span::raw(" "),
        Span::styled(container, theme.keybind_desc_style()),
    ];
    if viewer.show_timestamps {
        status_spans.push(Span::styled(
            "  [timestamps ON]",
            theme.keybind_desc_style(),
        ));
    }
    status_spans.push(Span::raw("  "));
    let hint = if viewer.searching {
        "[Enter] apply  [Esc] cancel  [Ctrl+U] clear"
    } else {
        "[Esc] back  [f] follow  [P] previous  [t] timestamps  [/] search  [n/N] next/prev  [S] save"
    };
    status_spans.push(Span::styled(hint, theme.keybind_desc_style()));

    frame.render_widget(Paragraph::new(Line::from(status_spans)), sections[0]);

    // If searching, render search input bar and reduce log area
    let log_area = if viewer.searching {
        let search_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(sections[1]);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" /", theme.section_title_style()),
                Span::styled(viewer.search_input.as_str(), Style::default().fg(theme.fg)),
                Span::styled("_", Style::default().fg(theme.accent)),
            ])),
            search_split[1],
        );
        search_split[0]
    } else {
        sections[1]
    };

    if let Some(error) = &viewer.error {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" Error: {error}"),
                theme.badge_error_style(),
            )),
            log_area,
        );
        return;
    }

    if viewer.picking_container {
        let has_all = viewer.containers.len() > 1;
        let mut lines: Vec<Line> = Vec::new();

        if has_all {
            let prefix = if viewer.container_cursor == 0 {
                ">"
            } else {
                " "
            };
            lines.push(Line::from(vec![
                Span::raw(format!("{prefix} ")),
                Span::styled(
                    " All Containers",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        for (idx, container) in viewer.containers.iter().enumerate() {
            let picker_idx = if has_all { idx + 1 } else { idx };
            let prefix = if picker_idx == viewer.container_cursor {
                ">"
            } else {
                " "
            };
            lines.push(Line::from(format!("{prefix} {container}")));
        }

        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), log_area);
        return;
    }

    if viewer.lines.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(" No log lines yet", theme.inactive_style())),
            log_area,
        );
        return;
    }

    let total = viewer.lines.len();
    let window = scroll_window(total, scroll, log_area.height.saturating_sub(1) as usize);
    let lines: Vec<Line> = viewer.lines[window.start..window.end]
        .iter()
        .map(|line| {
            if !viewer.search_query.is_empty() {
                highlight_search(line, &viewer.search_query, &theme)
            } else {
                Line::from(line.clone())
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), log_area);
    render_scrollbar(frame, log_area, total, window.start);
}

fn highlight_search<'a>(line: &str, query: &str, theme: &crate::ui::theme::Theme) -> Line<'a> {
    let lower_line = line.to_ascii_lowercase();
    let lower_query = query.to_ascii_lowercase();
    let mut spans = Vec::new();
    let mut last = 0;
    for (start, _) in lower_line.match_indices(&lower_query) {
        if start > last {
            spans.push(Span::raw(line[last..start].to_string()));
        }
        spans.push(Span::styled(
            line[start..start + query.len()].to_string(),
            Style::default().bg(theme.accent).fg(theme.bg),
        ));
        last = start + query.len();
    }
    if last < line.len() {
        spans.push(Span::raw(line[last..].to_string()));
    }
    if spans.is_empty() {
        Line::from(line.to_string())
    } else {
        Line::from(spans)
    }
}

fn render_workload_logs_tab(
    frame: &mut Frame,
    area: Rect,
    tab: &crate::workbench::WorkloadLogsTabState,
) {
    let theme = default_theme();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let header = Line::from(vec![
        Span::styled(
            format!(
                " {} ",
                if tab.loading {
                    "loading"
                } else if tab.follow_mode {
                    "following"
                } else {
                    "paused"
                }
            ),
            theme.badge_warning_style(),
        ),
        Span::raw(" "),
        Span::styled(
            format!(
                "pod:{}  container:{}  text:{}",
                tab.pod_filter.as_deref().unwrap_or("all"),
                tab.container_filter.as_deref().unwrap_or("all"),
                if tab.text_filter.is_empty() {
                    "all"
                } else {
                    tab.text_filter.as_str()
                }
            ),
            theme.keybind_desc_style(),
        ),
    ]);
    let hint = if tab.editing_text_filter {
        Line::from(Span::styled(
            format!(
                " Editing text filter: {}  [Enter] apply  [Esc] cancel  [Ctrl+U] clear",
                tab.filter_input
            ),
            theme.keybind_desc_style(),
        ))
    } else {
        Line::from(Span::styled(
            "[/] text  [p] pod  [c] container  [f] follow  [S] save  [Esc] back",
            theme.keybind_desc_style(),
        ))
    };
    frame.render_widget(Paragraph::new(vec![header, hint]), sections[0]);

    if let Some(error) = &tab.error {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" Error: {error}"),
                theme.badge_error_style(),
            )),
            sections[1],
        );
        return;
    }

    let total = tab
        .lines
        .iter()
        .filter(|line| tab.matches_filter(line))
        .count();
    if total == 0 {
        let message = tab.notice.as_deref().unwrap_or(if tab.loading {
            " Loading workload logs..."
        } else {
            " No workload log lines match the current filters"
        });
        frame.render_widget(
            Paragraph::new(Span::styled(message, theme.inactive_style())),
            sections[1],
        );
        return;
    }

    let window = scroll_window(
        total,
        tab.scroll,
        sections[1].height.saturating_sub(1) as usize,
    );
    let lines: Vec<Line> = tab
        .lines
        .iter()
        .filter(|line| tab.matches_filter(line))
        .skip(window.start)
        .take(window.end.saturating_sub(window.start))
        .map(|line| {
            let badge = if line.is_stderr {
                theme.badge_warning_style()
            } else {
                theme.badge_success_style()
            };
            Line::from(vec![
                Span::styled(
                    format!(" {}:{} ", line.pod_name, line.container_name),
                    badge,
                ),
                Span::styled(line.content.clone(), Style::default().fg(theme.fg_dim)),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        sections[1],
    );
    render_scrollbar(frame, sections[1], total, window.start);
}

fn render_exec_tab(frame: &mut Frame, area: Rect, tab: &crate::workbench::ExecTabState) {
    let theme = default_theme();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

    let status = if tab.loading {
        "loading"
    } else if tab.picking_container {
        "select container"
    } else if tab.exited {
        "exited"
    } else {
        "connected"
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(format!(" {status} "), theme.badge_warning_style()),
                Span::raw(" "),
                Span::styled(
                    format!(
                        "{} / {}",
                        tab.pod_name,
                        if tab.container_name.is_empty() {
                            "container: pending"
                        } else {
                            tab.container_name.as_str()
                        }
                    ),
                    theme.keybind_desc_style(),
                ),
            ]),
            Line::from(Span::styled(
                "[Enter] send  [Backspace] edit  [Esc] back",
                theme.keybind_desc_style(),
            )),
        ]),
        sections[0],
    );

    if let Some(error) = &tab.error {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" Error: {error}"),
                theme.badge_error_style(),
            )),
            sections[1],
        );
        return;
    }

    if tab.picking_container {
        let lines: Vec<Line> = tab
            .containers
            .iter()
            .enumerate()
            .map(|(idx, container)| {
                let prefix = if idx == tab.container_cursor {
                    ">"
                } else {
                    " "
                };
                Line::from(format!("{prefix} {container}"))
            })
            .collect();
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            sections[1],
        );
    } else {
        let total = tab.lines.len() + usize::from(!tab.pending_fragment.is_empty());
        if total == 0 {
            frame.render_widget(
                Paragraph::new(vec![Line::from(Span::styled(
                    " Waiting for shell output...",
                    theme.inactive_style(),
                ))])
                .wrap(Wrap { trim: false }),
                sections[1],
            );
        } else {
            let window = scroll_window(
                total,
                tab.scroll,
                sections[1].height.saturating_sub(1) as usize,
            );
            let complete_start = window.start.min(tab.lines.len());
            let complete_end = window.end.min(tab.lines.len());
            let mut lines: Vec<Line> = tab.lines[complete_start..complete_end]
                .iter()
                .map(|line| Line::from(line.clone()))
                .collect();

            if !tab.pending_fragment.is_empty() {
                let pending_idx = tab.lines.len();
                if (window.start..window.end).contains(&pending_idx) {
                    lines.push(Line::from(tab.pending_fragment.clone()));
                }
            }

            frame.render_widget(
                Paragraph::new(lines).wrap(Wrap { trim: false }),
                sections[1],
            );
            render_scrollbar(frame, sections[1], total, window.start);
        }
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" $ ", theme.section_title_style()),
            Span::styled(tab.input.clone(), Style::default().fg(theme.fg)),
        ])),
        sections[2],
    );
}

fn render_scrollbar(frame: &mut Frame, area: Rect, total: usize, position: usize) {
    if total <= area.height as usize || area.width == 0 {
        return;
    }

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"))
        .track_symbol(Some("│"))
        .thumb_symbol("█");
    let mut state = ScrollbarState::new(total).position(position);
    frame.render_stateful_widget(
        scrollbar,
        area.inner(Margin {
            vertical: 0,
            horizontal: 0,
        }),
        &mut state,
    );
}

#[cfg(test)]
mod tests {
    use super::{VisibleWindow, centered_window, scroll_window};
    use crate::ui::truncate_message;

    #[test]
    fn short_message_unchanged() {
        assert_eq!(truncate_message("hello", 60).as_ref(), "hello");
    }

    #[test]
    fn exact_length_unchanged() {
        let msg = "a".repeat(60);
        assert_eq!(truncate_message(&msg, 60).as_ref(), msg);
    }

    #[test]
    fn one_over_truncated() {
        let msg = "a".repeat(61);
        let result = truncate_message(&msg, 60);
        assert!(result.ends_with("..."));
        assert_eq!(result.chars().count(), 60);
    }

    #[test]
    fn empty_string() {
        assert_eq!(truncate_message("", 60).as_ref(), "");
    }

    #[test]
    fn multibyte_chars_counted_correctly() {
        let msg = "\u{00e9}".repeat(10);
        assert_eq!(truncate_message(&msg, 15).as_ref(), msg);
    }

    #[test]
    fn multibyte_chars_truncated_on_char_boundary() {
        let msg = "\u{00e9}".repeat(20);
        let result = truncate_message(&msg, 10);
        assert!(result.ends_with("..."));
        assert_eq!(result.chars().count(), 10);
    }

    #[test]
    fn borrowed_when_short() {
        let result = truncate_message("short", 60);
        assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn very_small_max_chars_no_ellipsis() {
        let result = truncate_message("hello world", 2);
        assert_eq!(result.as_ref(), "he");
        assert_eq!(result.chars().count(), 2);
    }

    #[test]
    fn max_chars_zero() {
        let result = truncate_message("hello", 0);
        assert_eq!(result.as_ref(), "");
    }

    #[test]
    fn max_chars_three_uses_ellipsis() {
        let result = truncate_message("hello world", 4);
        assert_eq!(result.as_ref(), "h...");
        assert_eq!(result.chars().count(), 4);
    }

    #[test]
    fn scroll_window_clamps_to_bottom() {
        assert_eq!(
            scroll_window(10, 99, 3),
            VisibleWindow { start: 7, end: 10 }
        );
    }

    #[test]
    fn centered_window_keeps_selection_visible() {
        assert_eq!(
            centered_window(10, 8, 3),
            VisibleWindow { start: 7, end: 10 }
        );
    }
}
