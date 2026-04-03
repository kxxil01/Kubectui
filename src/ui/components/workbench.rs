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
    authorization::{ActionAccessReview, DetailActionAuthorization, ResourceAccessCheck},
    log_investigation::{LogQueryMode, LogSeverity, highlight_ranges},
    rbac_subjects::{SubjectAccessReview, SubjectBindingResolution},
    resource_diff::{ResourceDiffBaselineKind, ResourceDiffLineKind},
    secret::DecodedSecretValue,
    state::ClusterSnapshot,
    time::format_local,
    ui::{components::default_theme, theme::Theme},
    workbench::{RunbookStepState, WorkbenchTab, WorkbenchTabState},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VisibleWindow {
    start: usize,
    end: usize,
}

#[derive(Debug, Clone, Copy)]
struct LogHighlightOptions<'a> {
    enabled: bool,
    query: &'a str,
    mode: LogQueryMode,
    compiled: Option<&'a regex::Regex>,
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

pub fn render_workbench(frame: &mut Frame, area: Rect, app: &AppState, cluster: &ClusterSnapshot) {
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
        WorkbenchTabState::AccessReview(tab) => render_access_review_tab(frame, inner, tab),
        WorkbenchTabState::ResourceYaml(tab) => {
            render_yaml_tab(frame, inner, tab.scroll, active_tab)
        }
        WorkbenchTabState::ResourceDiff(tab) => render_resource_diff_tab(frame, inner, tab),
        WorkbenchTabState::Rollout(tab) => render_rollout_tab(frame, inner, cluster, tab),
        WorkbenchTabState::HelmHistory(tab) => render_helm_history_tab(frame, inner, tab),
        WorkbenchTabState::DecodedSecret(tab) => render_decoded_secret_tab(frame, inner, tab),
        WorkbenchTabState::ResourceEvents(tab) => {
            render_events_tab(frame, inner, tab.scroll, active_tab)
        }
        WorkbenchTabState::PodLogs(tab) => {
            render_logs_tab(frame, inner, active_tab, tab.viewer.scroll_offset)
        }
        WorkbenchTabState::WorkloadLogs(tab) => render_workload_logs_tab(frame, inner, tab),
        WorkbenchTabState::Exec(tab) => render_exec_tab(frame, inner, tab),
        WorkbenchTabState::ExtensionOutput(tab) => render_extension_output_tab(frame, inner, tab),
        WorkbenchTabState::AiAnalysis(tab) => render_ai_analysis_tab(frame, inner, tab),
        WorkbenchTabState::Runbook(tab) => render_runbook_tab(frame, inner, tab),
        WorkbenchTabState::PortForward(tab) => tab.dialog.render_embedded(frame, inner),
        WorkbenchTabState::Relations(tab) => {
            crate::ui::views::relations::render_relations_tab(frame, inner, tab, &theme)
        }
        WorkbenchTabState::NetworkPolicy(tab) => render_network_policy_tab(frame, inner, tab),
        WorkbenchTabState::TrafficDebug(tab) => render_traffic_debug_tab(frame, inner, tab),
        WorkbenchTabState::Connectivity(tab) => render_connectivity_tab(frame, inner, tab),
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
                "  [y] YAML  [D] Drift  [O] Rollout  [A] Access  [t] Traffic  [o] Decoded  [v] Timeline  [l] Logs  [x] Exec  [f] Port-Forward",
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
    let entries = app.visible_action_history_entries();

    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![Span::styled(
                    " No mutation history for the active workspace",
                    theme.section_title_style(),
                )]),
                Line::from(""),
                Line::from(
                    "  Mutating actions for the current context and namespace will appear here.",
                ),
                Line::from("  Use [Enter] on a jumpable row to reopen the affected resource."),
            ])
            .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }

    let selected = tab.selected.min(entries.len().saturating_sub(1));
    let window = centered_window(entries.len(), selected, area.height.max(1) as usize);
    let lines: Vec<Line> = entries
        .iter()
        .enumerate()
        .skip(window.start)
        .take(window.end.saturating_sub(window.start))
        .map(|(idx, entry)| render_action_history_line(entry, idx == selected, &theme))
        .collect();

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    render_scrollbar(frame, area, entries.len(), window.start);
}

fn render_access_review_tab(
    frame: &mut Frame,
    area: Rect,
    tab: &crate::workbench::AccessReviewTabState,
) {
    let theme = default_theme();
    let lines = access_review_lines(tab, &theme);
    let total_lines = lines.len();
    let window = scroll_window(total_lines, tab.scroll, area.height.max(1) as usize);
    let visible = lines
        .into_iter()
        .skip(window.start)
        .take(window.end.saturating_sub(window.start))
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(visible).wrap(Wrap { trim: false }), area);
    render_scrollbar(frame, area, total_lines, window.start);
}

fn access_review_lines(
    tab: &crate::workbench::AccessReviewTabState,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(tab.line_count());
    let context = tab.context_name.as_deref().unwrap_or("unknown");
    lines.push(Line::from(vec![
        Span::styled(" Resource: ", theme.section_title_style()),
        Span::raw(tab.resource.summary_label()),
    ]));
    lines.push(Line::from(vec![
        Span::styled(" Context: ", theme.section_title_style()),
        Span::raw(context.to_string()),
        Span::raw("  "),
        Span::styled("Namespace Scope: ", theme.section_title_style()),
        Span::raw(tab.namespace_scope.clone()),
    ]));
    lines.push(Line::from(vec![Span::styled(
        " Required Kubernetes API checks for each supported detail action",
        theme.muted_style(),
    )]));
    lines.push(Line::from(""));

    if let Some(attempted_review) = tab.attempted_review.as_ref() {
        lines.push(render_attempted_action_line(
            attempted_review.action,
            attempted_review.authorization,
            theme,
        ));
        lines.push(Line::from(vec![Span::styled(
            " This action was blocked. Review the attempted-action checks first, then the broader supported action matrix below.",
            theme.muted_style(),
        )]));
        if let Some(note) = &attempted_review.note {
            lines.push(Line::from(vec![Span::styled(
                format!(" {note}"),
                theme.muted_style(),
            )]));
        }
        lines.push(render_access_review_scope_line(
            "Attempted action checks",
            theme,
        ));
        if attempted_review.checks.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "   No RBAC check required for this action.",
                theme.muted_style(),
            )]));
        } else {
            lines.extend(render_grouped_access_review_check_lines(
                &attempted_review.checks,
                theme,
            ));
        }
        lines.push(Line::from(""));
    }

    lines.extend(render_access_review_subject_input_lines(tab, theme));

    if let Some(subject_review) = &tab.subject_review {
        lines.extend(render_subject_access_review_lines(subject_review, theme));
    }

    for entry in &tab.entries {
        lines.push(render_access_review_header_line(entry, theme));
        if entry.checks.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "   No RBAC check required for this action.",
                theme.muted_style(),
            )]));
        } else {
            lines.extend(render_grouped_access_review_check_lines(
                &entry.checks,
                theme,
            ));
        }
        lines.push(Line::from(""));
    }

    lines
}

fn render_access_review_subject_input_lines(
    tab: &crate::workbench::AccessReviewTabState,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(5);
    lines.push(Line::from(vec![
        Span::styled(" Review Subject: ", theme.section_title_style()),
        tab.subject_input.styled_text(matches!(
            tab.focus,
            crate::workbench::AccessReviewFocus::SubjectInput
        )),
    ]));
    lines.push(Line::from(vec![Span::styled(
        " Press [s] or [Tab] to edit, [Enter] to apply. Use ServiceAccount/<namespace>/<name>, User/<name>, or Group/<name>.",
        theme.muted_style(),
    )]));
    if let Some(error) = &tab.subject_input_error {
        lines.push(Line::from(vec![Span::styled(
            format!(" {error}"),
            Style::default().fg(theme.error),
        )]));
    }
    lines.push(Line::from(""));
    lines
}

fn render_attempted_action_line(
    action: crate::policy::DetailAction,
    authorization: Option<DetailActionAuthorization>,
    theme: &Theme,
) -> Line<'static> {
    let (status_label, status_style) = match authorization {
        Some(DetailActionAuthorization::Allowed) => ("allowed", Style::default().fg(theme.success)),
        Some(DetailActionAuthorization::Denied) => ("denied", Style::default().fg(theme.error)),
        Some(DetailActionAuthorization::Unknown) => ("unknown", theme.badge_warning_style()),
        None => ("not gated", theme.muted_style()),
    };

    Line::from(vec![
        Span::styled(" Attempted Action: ", theme.section_title_style()),
        Span::raw(action.label().to_string()),
        Span::raw("  "),
        Span::styled(format!("[{status_label}]"), status_style),
    ])
}

fn render_subject_access_review_lines(
    review: &SubjectAccessReview,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(" Subject: ", theme.section_title_style()),
        Span::raw(review.subject.label()),
    ]));
    lines.push(Line::from(vec![Span::styled(
        " Matching bindings and referenced roles in the current snapshot",
        theme.muted_style(),
    )]));
    lines.push(Line::from(""));

    if review.bindings.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "   No matching RoleBinding or ClusterRoleBinding found.",
            theme.muted_style(),
        )]));
        lines.push(Line::from(""));
        return lines;
    }

    for binding in &review.bindings {
        lines.push(render_subject_binding_line(binding, theme));
        lines.push(render_subject_role_line(binding, theme));
        if binding.role.rules.is_empty() {
            let message = if binding.role.missing {
                format!(
                    "   Referenced {} is missing from the current snapshot.",
                    binding.role.kind
                )
            } else {
                "   Referenced role has no rules.".to_string()
            };
            lines.push(Line::from(vec![Span::styled(message, theme.muted_style())]));
        } else {
            for rule in &binding.role.rules {
                lines.push(render_subject_rule_line(rule, theme));
            }
        }
        lines.push(Line::from(""));
    }

    lines
}

fn render_access_review_header_line(entry: &ActionAccessReview, theme: &Theme) -> Line<'static> {
    let (status_label, status_style) = match entry.authorization {
        Some(DetailActionAuthorization::Allowed) => ("allowed", Style::default().fg(theme.success)),
        Some(DetailActionAuthorization::Denied) => ("denied", Style::default().fg(theme.error)),
        Some(DetailActionAuthorization::Unknown) => ("unknown", theme.badge_warning_style()),
        None => ("not gated", theme.muted_style()),
    };
    let mut spans = Vec::with_capacity(5);
    if let Some(shortcut) = entry.action.shortcut_hint() {
        spans.push(Span::styled(
            format!(" {shortcut} "),
            theme.section_title_style(),
        ));
    }
    spans.push(Span::raw(entry.action.label().to_string()));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(format!("[{status_label}]"), status_style));
    if entry.strict {
        spans.push(Span::raw("  "));
        spans.push(Span::styled("(strict)", theme.muted_style()));
    }
    Line::from(spans)
}

fn render_access_review_check_line(check: &ResourceAccessCheck, theme: &Theme) -> Line<'static> {
    let mut target = match &check.group {
        Some(group) => format!("{group}/{}", check.resource),
        None => check.resource.clone(),
    };
    if let Some(subresource) = &check.subresource {
        target.push('/');
        target.push_str(subresource);
    }

    let mut details = Vec::new();
    if let Some(namespace) = &check.namespace {
        details.push(format!("namespace={namespace}"));
    }
    if let Some(name) = &check.name {
        details.push(format!("name={name}"));
    }

    let suffix = if details.is_empty() {
        String::new()
    } else {
        format!(" ({})", details.join(", "))
    };

    Line::from(vec![
        Span::styled("   - ", theme.muted_style()),
        Span::styled(check.verb.clone(), theme.section_title_style()),
        Span::raw(" "),
        Span::raw(target),
        Span::styled(suffix, theme.muted_style()),
    ])
}

fn render_access_review_scope_line(label: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![Span::styled(
        format!("   {label}"),
        theme.muted_style(),
    )])
}

fn render_grouped_access_review_check_lines(
    checks: &[ResourceAccessCheck],
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut namespace_checks = Vec::new();
    let mut cluster_checks = Vec::new();
    for check in checks {
        if check.namespace.is_some() {
            namespace_checks.push(check);
        } else {
            cluster_checks.push(check);
        }
    }

    if !namespace_checks.is_empty() {
        lines.push(render_access_review_scope_line(
            "Namespace-scoped checks",
            theme,
        ));
        for check in namespace_checks {
            lines.push(render_access_review_check_line(check, theme));
        }
    }

    if !cluster_checks.is_empty() {
        lines.push(render_access_review_scope_line(
            "Cluster-scoped checks",
            theme,
        ));
        for check in cluster_checks {
            lines.push(render_access_review_check_line(check, theme));
        }
    }

    lines
}

fn render_subject_binding_line(binding: &SubjectBindingResolution, theme: &Theme) -> Line<'static> {
    let (binding_label, binding_scope) = match &binding.binding {
        crate::app::ResourceRef::RoleBinding(name, namespace) => {
            (format!("RoleBinding {namespace}/{name}"), "namespace")
        }
        crate::app::ResourceRef::ClusterRoleBinding(name) => {
            (format!("ClusterRoleBinding {name}"), "cluster")
        }
        _ => (binding.binding.summary_label(), "unknown"),
    };

    Line::from(vec![
        Span::styled(" Binding: ", theme.section_title_style()),
        Span::raw(binding_label),
        Span::raw("  "),
        Span::styled(format!("[{binding_scope}]"), theme.muted_style()),
    ])
}

fn render_subject_role_line(binding: &SubjectBindingResolution, theme: &Theme) -> Line<'static> {
    let role_scope = if binding.role.namespace.is_some() {
        "namespace"
    } else {
        "cluster"
    };
    let role_label = match (&binding.role.namespace, binding.role.missing) {
        (Some(namespace), false) => {
            format!("{} {namespace}/{}", binding.role.kind, binding.role.name)
        }
        (None, false) => format!("{} {}", binding.role.kind, binding.role.name),
        (Some(namespace), true) => {
            format!(
                "{} {namespace}/{} [missing]",
                binding.role.kind, binding.role.name
            )
        }
        (None, true) => format!("{} {} [missing]", binding.role.kind, binding.role.name),
    };

    Line::from(vec![
        Span::styled("   -> ", theme.muted_style()),
        Span::raw(role_label),
        Span::raw("  "),
        Span::styled(format!("[{role_scope}]"), theme.muted_style()),
    ])
}

fn render_subject_rule_line(rule: &crate::k8s::dtos::RbacRule, theme: &Theme) -> Line<'static> {
    let verbs = if rule.verbs.is_empty() {
        "*".to_string()
    } else {
        rule.verbs.join(", ")
    };
    let resources = if !rule.resources.is_empty() {
        rule.resources.join(", ")
    } else if !rule.non_resource_urls.is_empty() {
        rule.non_resource_urls.join(", ")
    } else {
        "*".to_string()
    };

    let mut suffix = Vec::new();
    if !rule.api_groups.is_empty() {
        suffix.push(format!("groups={}", rule.api_groups.join(", ")));
    }
    if !rule.resource_names.is_empty() {
        suffix.push(format!("names={}", rule.resource_names.join(", ")));
    }
    let suffix = if suffix.is_empty() {
        String::new()
    } else {
        format!(" ({})", suffix.join("; "))
    };

    Line::from(vec![
        Span::styled("     - ", theme.muted_style()),
        Span::styled(verbs, theme.section_title_style()),
        Span::raw(" "),
        Span::raw(resources),
        Span::styled(suffix, theme.muted_style()),
    ])
}

fn render_network_policy_tab(
    frame: &mut Frame,
    area: Rect,
    tab: &crate::workbench::NetworkPolicyTabState,
) {
    let theme = default_theme();
    let summary_height = tab.summary_lines.len().min(4) as u16;
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(summary_height), Constraint::Min(0)])
        .split(area);

    if summary_height > 0 {
        let lines = tab
            .summary_lines
            .iter()
            .map(|line| {
                Line::from(Span::styled(
                    line.clone(),
                    Style::default().fg(theme.fg_dim),
                ))
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            sections[0],
        );
    }

    crate::ui::views::relations::render_relation_tree(
        frame,
        sections[1],
        crate::ui::views::relations::RelationTreeView {
            tree: &tab.tree,
            expanded: &tab.expanded,
            cursor: tab.cursor,
            loading: tab.loading,
            error: tab.error.as_deref(),
            loading_message: "Loading network policy analysis...",
            empty_message: "No network policy analysis available.",
        },
        &theme,
    );
}

fn render_connectivity_tab(
    frame: &mut Frame,
    area: Rect,
    tab: &crate::workbench::ConnectivityTabState,
) {
    use crate::workbench::ConnectivityTabFocus;

    let theme = default_theme();
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
        .split(area);

    let left_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(columns[0]);

    let source_line = Paragraph::new(vec![
        Line::from(Span::styled("Source", theme.section_title_style())),
        Line::from(Span::styled(
            format!(
                "{} -> choose target pod",
                match tab.source.namespace() {
                    Some(namespace) => format!("{namespace}/{}", tab.source.name()),
                    None => tab.source.name().to_string(),
                }
            ),
            Style::default().fg(theme.fg),
        )),
    ]);
    frame.render_widget(source_line, left_rows[0]);

    let filter_block = Block::default()
        .title(Span::styled(
            " Filter ",
            if tab.focus == ConnectivityTabFocus::Filter {
                theme.title_style()
            } else {
                theme.section_title_style()
            },
        ))
        .borders(Borders::ALL)
        .border_style(if tab.focus == ConnectivityTabFocus::Filter {
            theme.border_active_style()
        } else {
            theme.border_style()
        });
    let filter_text = Paragraph::new(Line::from(vec![
        tab.filter
            .styled_text(tab.focus == ConnectivityTabFocus::Filter),
    ]))
    .block(filter_block);
    frame.render_widget(filter_text, left_rows[1]);

    let target_lines = if tab.filtered_target_indices.is_empty() {
        vec![Line::from(Span::styled(
            "No target pod matches the current filter.",
            Style::default().fg(theme.fg_dim),
        ))]
    } else {
        let window = centered_window(
            tab.filtered_target_indices.len(),
            tab.selected_target,
            left_rows[2].height.saturating_sub(2) as usize,
        );
        tab.filtered_target_indices[window.start..window.end]
            .iter()
            .enumerate()
            .map(|(offset, target_idx)| {
                let absolute = window.start + offset;
                let target = &tab.targets[*target_idx];
                let selected = absolute == tab.selected_target;
                let style = if selected {
                    theme.selection_style()
                } else {
                    Style::default().fg(theme.fg)
                };
                let ip = target.pod_ip.as_deref().unwrap_or("-");
                Line::from(vec![
                    Span::styled(if selected { "› " } else { "  " }, style),
                    Span::styled(target.display.clone(), style),
                    Span::styled(
                        format!("  {}  {}", target.status, ip),
                        Style::default().fg(theme.fg_dim),
                    ),
                ])
            })
            .collect::<Vec<_>>()
    };
    let targets_block = Block::default()
        .title(Span::styled(
            " Targets ",
            if tab.focus == ConnectivityTabFocus::Targets {
                theme.title_style()
            } else {
                theme.section_title_style()
            },
        ))
        .borders(Borders::ALL)
        .border_style(if tab.focus == ConnectivityTabFocus::Targets {
            theme.border_active_style()
        } else {
            theme.border_style()
        });
    frame.render_widget(
        Paragraph::new(target_lines)
            .block(targets_block)
            .wrap(Wrap { trim: false }),
        left_rows[2],
    );

    let hint = match tab.focus {
        ConnectivityTabFocus::Filter => "[Tab] targets  [Ctrl+U] clear  [Esc] return",
        ConnectivityTabFocus::Targets => "[Enter] run  [/] filter  [Tab] result  [Esc] return",
        ConnectivityTabFocus::Result => {
            "[Enter] open detail  [/] filter  [Tab] filter  [Esc] return"
        }
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(hint, theme.keybind_desc_style()))),
        left_rows[3],
    );

    let right_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(tab.summary_lines.len().min(4) as u16),
            Constraint::Min(0),
        ])
        .split(columns[1]);

    if !tab.summary_lines.is_empty() {
        let lines = tab
            .summary_lines
            .iter()
            .map(|line| {
                Line::from(Span::styled(
                    line.clone(),
                    Style::default().fg(theme.fg_dim),
                ))
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            right_rows[0],
        );
    }

    crate::ui::views::relations::render_relation_tree(
        frame,
        right_rows[1],
        crate::ui::views::relations::RelationTreeView {
            tree: &tab.tree,
            expanded: &tab.expanded,
            cursor: tab.tree_cursor,
            loading: false,
            error: tab.error.as_deref(),
            loading_message: "Evaluating connectivity...",
            empty_message: "Run a connectivity check to inspect policy intent.",
        },
        &theme,
    );
}

fn render_traffic_debug_tab(
    frame: &mut Frame,
    area: Rect,
    tab: &crate::workbench::TrafficDebugTabState,
) {
    let theme = default_theme();
    let summary_height = tab.summary_lines.len().min(5) as u16;
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(summary_height), Constraint::Min(0)])
        .split(area);

    if summary_height > 0 {
        let lines = tab
            .summary_lines
            .iter()
            .map(|line| {
                Line::from(Span::styled(
                    line.clone(),
                    Style::default().fg(theme.fg_dim),
                ))
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            sections[0],
        );
    }

    crate::ui::views::relations::render_relation_tree(
        frame,
        sections[1],
        crate::ui::views::relations::RelationTreeView {
            tree: &tab.tree,
            expanded: &tab.expanded,
            cursor: tab.cursor,
            loading: false,
            error: tab.error.as_deref(),
            loading_message: "Loading traffic diagnostics...",
            empty_message: "No traffic diagnostics available for this resource.",
        },
        &theme,
    );
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
    let timestamp = entry.finished_at.unwrap_or(entry.started_at);
    let timestamp = format_local(timestamp, "%H:%M:%S");
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

fn render_resource_diff_tab(
    frame: &mut Frame,
    area: Rect,
    tab_state: &crate::workbench::ResourceDiffTabState,
) {
    let theme = default_theme();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(area);

    let baseline = match tab_state
        .baseline_kind
        .unwrap_or(ResourceDiffBaselineKind::Missing)
    {
        ResourceDiffBaselineKind::LastAppliedAnnotation => {
            Span::styled(" baseline:last-applied ", theme.badge_success_style())
        }
        ResourceDiffBaselineKind::ServerSideApplyManagedFields => {
            Span::styled(" baseline:ssa-managedFields ", theme.badge_warning_style())
        }
        ResourceDiffBaselineKind::Missing => {
            Span::styled(" baseline:missing ", theme.badge_warning_style())
        }
    };
    let summary = tab_state
        .summary
        .clone()
        .unwrap_or_else(|| "Waiting for YAML...".to_string());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            baseline,
            Span::raw(" "),
            Span::styled(summary.as_str(), Style::default().fg(theme.fg)),
            Span::raw("  "),
            Span::styled("[Esc] back", theme.keybind_desc_style()),
        ])),
        sections[0],
    );

    if tab_state.loading {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Loading resource diff...",
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

    if tab_state.lines.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(summary, theme.inactive_style()))
                .wrap(Wrap { trim: false }),
            sections[1],
        );
        return;
    }

    render_diff_lines(frame, sections[1], &tab_state.lines, tab_state.scroll);
}

fn render_rollout_tab(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    tab: &crate::workbench::RolloutTabState,
) {
    let theme = default_theme();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(0)])
        .split(area);

    let mode_badge = if let Some(mutation) = tab.mutation_pending {
        let label = match mutation {
            crate::workbench::RolloutMutationState::Restart => " mutation:restart ",
            crate::workbench::RolloutMutationState::Pause => " mutation:pause ",
            crate::workbench::RolloutMutationState::Resume => " mutation:resume ",
            crate::workbench::RolloutMutationState::Undo(_) => " mutation:undo ",
        };
        Span::styled(label, theme.badge_warning_style())
    } else if tab.confirm_undo_revision.is_some() {
        Span::styled(" undo:confirm ", theme.badge_warning_style())
    } else if tab.loading {
        Span::styled(" loading ", theme.badge_warning_style())
    } else {
        Span::styled(" mode:rollout ", theme.badge_success_style())
    };
    let workload_badge_style = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let subtle_badge_style = theme.muted_style();
    let kind_badge = tab
        .kind
        .map(|kind| Span::styled(format!(" {} ", kind.label()), workload_badge_style))
        .unwrap_or_else(|| Span::styled(" Workload ", workload_badge_style));
    let hint = if tab.mutation_pending.is_some() {
        "[mutation in progress]"
    } else if tab.confirm_undo_revision.is_some() {
        "[Enter/U] confirm undo  [Esc] cancel"
    } else if tab.kind == Some(crate::k8s::rollout::RolloutWorkloadKind::Deployment) {
        "[R] restart  [P] pause/resume  [U] undo selected  [j/k] move"
    } else {
        "[R] restart  [U] undo selected  [j/k] move"
    };

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![kind_badge, Span::raw(" "), mode_badge]),
            Line::from(vec![
                Span::styled(" Live ", theme.inactive_style()),
                Span::styled(
                    rollout_live_summary(cluster, &tab.resource)
                        .unwrap_or_else(|| "snapshot unavailable".to_string()),
                    Style::default().fg(theme.fg),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Strategy ", theme.inactive_style()),
                Span::styled(
                    tab.strategy.as_deref().unwrap_or("n/a"),
                    Style::default().fg(theme.accent2),
                ),
                Span::styled("  Current ", theme.inactive_style()),
                Span::styled(
                    tab.current_revision
                        .map(|revision| revision.to_string())
                        .unwrap_or_else(|| "n/a".to_string()),
                    Style::default().fg(theme.fg),
                ),
                Span::styled("  Target ", theme.inactive_style()),
                Span::styled(
                    tab.update_target_revision
                        .map(|revision| revision.to_string())
                        .unwrap_or_else(|| "n/a".to_string()),
                    Style::default().fg(theme.fg),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Hint ", theme.inactive_style()),
                Span::styled(hint, Style::default().fg(theme.muted)),
            ]),
        ])
        .wrap(Wrap { trim: false }),
        sections[0],
    );

    if tab.mutation_pending.is_some() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Rollout mutation in progress...",
                theme.inactive_style(),
            )),
            sections[1],
        );
        return;
    }

    if let Some(target_revision) = tab.confirm_undo_revision {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![Span::styled(
                    format!(" Roll back this workload to revision {target_revision}?"),
                    theme.section_title_style(),
                )]),
                Line::from(""),
                Line::from(
                    " This patches the live Pod template from the selected workload revision.",
                ),
                Line::from(
                    " Kubectui will refresh rollout state and workload data after completion.",
                ),
            ])
            .wrap(Wrap { trim: false }),
            sections[1],
        );
        return;
    }

    if tab.loading && tab.revisions.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Loading rollout state...",
                theme.inactive_style(),
            )),
            sections[1],
        );
        return;
    }

    if let Some(error) = &tab.error {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" Rollout fetch failed: {error}"),
                theme.badge_error_style(),
            ))
            .wrap(Wrap { trim: false }),
            sections[1],
        );
        return;
    }

    let mut lines = Vec::new();
    for line in &tab.summary_lines {
        lines.push(Line::from(vec![
            Span::styled("  ", theme.inactive_style()),
            Span::styled(line.clone(), Style::default().fg(theme.fg_dim)),
        ]));
    }
    if !tab.conditions.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " Conditions",
            theme.section_title_style(),
        )));
        for condition in &tab.conditions {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:<14}", condition.type_),
                    Style::default().fg(theme.accent),
                ),
                Span::styled(
                    format!("{:<6}", condition.status),
                    if condition.status.eq_ignore_ascii_case("true") {
                        theme.badge_success_style()
                    } else {
                        theme.badge_warning_style()
                    },
                ),
                Span::styled(
                    condition
                        .reason
                        .clone()
                        .or_else(|| condition.message.clone())
                        .unwrap_or_else(|| "—".to_string()),
                    Style::default().fg(theme.fg_dim),
                ),
            ]));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " Revisions",
        theme.section_title_style(),
    )));
    let revision_start = lines.len();
    if tab.revisions.is_empty() {
        lines.push(Line::from("  No rollout history available."));
    } else {
        for (idx, revision) in tab.revisions.iter().enumerate() {
            let selected = idx == tab.selected;
            let mut row_style = Style::default().fg(theme.fg);
            if selected {
                row_style = row_style.bg(theme.selection_bg).fg(theme.selection_fg);
            }
            let badge = if revision.is_current {
                " current "
            } else if revision.is_update_target {
                " target  "
            } else {
                "         "
            };
            lines.push(Line::from(vec![
                Span::styled(if selected { "› " } else { "  " }, row_style),
                Span::styled(format!("rev {:>3} ", revision.revision), row_style),
                Span::styled(badge, subtle_badge_style),
                Span::styled(format!("{} ", revision.summary), row_style),
                Span::styled(
                    revision
                        .change_cause
                        .clone()
                        .unwrap_or_else(|| revision.name.clone()),
                    Style::default().fg(theme.muted),
                ),
            ]));
        }
    }

    let scroll = if tab.revisions.is_empty() {
        0
    } else {
        (revision_start + tab.selected).saturating_sub(3)
    };
    let window = scroll_window(lines.len(), scroll, sections[1].height.max(1) as usize);
    frame.render_widget(
        Paragraph::new(lines[window.start..window.end].to_vec()).wrap(Wrap { trim: false }),
        sections[1],
    );
    render_scrollbar(frame, sections[1], lines.len(), window.start);
}

fn rollout_live_summary(
    cluster: &ClusterSnapshot,
    resource: &crate::app::ResourceRef,
) -> Option<String> {
    match resource {
        crate::app::ResourceRef::Deployment(name, namespace) => cluster
            .deployments
            .iter()
            .find(|item| &item.name == name && &item.namespace == namespace)
            .map(|item| {
                format!(
                    "Ready {}/{} · Updated {} · Available {}",
                    item.ready_replicas,
                    item.desired_replicas,
                    item.updated_replicas,
                    item.available_replicas
                )
            }),
        crate::app::ResourceRef::StatefulSet(name, namespace) => cluster
            .statefulsets
            .iter()
            .find(|item| &item.name == name && &item.namespace == namespace)
            .map(|item| format!("Ready {}/{}", item.ready_replicas, item.desired_replicas)),
        crate::app::ResourceRef::DaemonSet(name, namespace) => cluster
            .daemonsets
            .iter()
            .find(|item| &item.name == name && &item.namespace == namespace)
            .map(|item| {
                format!(
                    "Ready {}/{} · Unavailable {}",
                    item.ready_count, item.desired_count, item.unavailable_count
                )
            }),
        _ => None,
    }
}

fn render_helm_history_tab(
    frame: &mut Frame,
    area: Rect,
    tab: &crate::workbench::HelmHistoryTabState,
) {
    let theme = default_theme();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(area);

    let mode_badge = if tab.rollback_pending {
        Span::styled(" rollback:running ", theme.badge_warning_style())
    } else if tab.confirm_rollback_revision.is_some() {
        Span::styled(" rollback:confirm ", theme.badge_warning_style())
    } else if tab.diff.is_some() {
        Span::styled(" mode:values-diff ", theme.badge_success_style())
    } else {
        Span::styled(" mode:history ", theme.badge_success_style())
    };
    let cli_badge = tab
        .cli_version
        .as_ref()
        .map(|version| Span::styled(format!(" helm:{version} "), theme.badge_warning_style()))
        .unwrap_or_else(|| Span::styled(" helm:detecting ", theme.badge_warning_style()));
    let summary = if let Some(revision) = tab.selected_revision() {
        format!(
            "rev {}  {}  chart:{}  app:{}  {}",
            revision.revision,
            revision.status,
            revision.chart,
            if revision.app_version.is_empty() {
                "n/a"
            } else {
                revision.app_version.as_str()
            },
            revision.updated
        )
    } else if tab.loading {
        "Waiting for Helm history...".to_string()
    } else if let Some(error) = &tab.error {
        format!("Error: {error}")
    } else {
        "No Helm revisions available.".to_string()
    };
    let hint = if tab.rollback_pending {
        "[rollback in progress]"
    } else if tab.confirm_rollback_revision.is_some() {
        "[Enter/y/R] confirm  [Esc] cancel"
    } else if tab.diff.is_some() {
        "[Esc] back  [R] rollback"
    } else {
        "[Enter] diff selected vs current  [R] rollback  [j/k] move"
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                mode_badge,
                Span::raw(" "),
                cli_badge,
                Span::raw(" "),
                Span::styled(summary, Style::default().fg(theme.fg)),
            ]),
            Line::from(Span::styled(hint, theme.keybind_desc_style())),
        ]),
        sections[0],
    );

    if tab.rollback_pending {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Helm rollback is running. Action history will update when the CLI call completes.",
                theme.inactive_style(),
            ))
            .wrap(Wrap { trim: false }),
            sections[1],
        );
        return;
    }

    if let Some(target_revision) = tab.confirm_rollback_revision {
        let current = tab
            .current_revision
            .map(|revision| revision.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    format!(
                        " Roll back this release from revision {current} to revision {target_revision}?"
                    ),
                    theme.section_title_style(),
                )),
                Line::from(""),
                Line::from(
                    " Helm rollback will mutate the live release in the current kube context.",
                ),
                Line::from(
                    " Kubectui will wait for Helm and refresh the release history after completion.",
                ),
            ])
            .wrap(Wrap { trim: false }),
            sections[1],
        );
        return;
    }

    if let Some(diff) = &tab.diff {
        render_helm_values_diff(frame, sections[1], diff);
        return;
    }

    if tab.loading && tab.revisions.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Loading Helm release history...",
                theme.inactive_style(),
            )),
            sections[1],
        );
        return;
    }

    if let Some(error) = &tab.error
        && tab.revisions.is_empty()
    {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" Error: {error}"),
                theme.badge_error_style(),
            )),
            sections[1],
        );
        return;
    }

    if tab.revisions.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " No Helm history returned for this release.",
                theme.inactive_style(),
            )),
            sections[1],
        );
        return;
    }

    let total = tab.revisions.len();
    let window = centered_window(total, tab.selected, sections[1].height.max(1) as usize);
    let lines = tab.revisions[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(offset, revision)| {
            let selected = window.start + offset == tab.selected;
            render_helm_revision_line(revision, selected, tab.current_revision, &theme)
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        sections[1],
    );
    render_scrollbar(frame, sections[1], total, window.start);
}

fn render_helm_values_diff(
    frame: &mut Frame,
    area: Rect,
    diff: &crate::workbench::HelmValuesDiffState,
) {
    let theme = default_theme();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let summary = diff
        .summary
        .clone()
        .unwrap_or_else(|| "Waiting for Helm values diff...".to_string());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(
                    " current:{} target:{} ",
                    diff.current_revision, diff.target_revision
                ),
                theme.badge_success_style(),
            ),
            Span::raw(" "),
            Span::styled(summary.as_str(), Style::default().fg(theme.fg)),
        ])),
        sections[0],
    );

    if diff.loading {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Loading Helm values diff...",
                theme.inactive_style(),
            )),
            sections[1],
        );
        return;
    }

    if let Some(error) = &diff.error {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" Error: {error}"),
                theme.badge_error_style(),
            )),
            sections[1],
        );
        return;
    }

    if diff.lines.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(summary, theme.inactive_style()))
                .wrap(Wrap { trim: false }),
            sections[1],
        );
        return;
    }

    render_diff_lines(frame, sections[1], &diff.lines, diff.scroll);
}

fn render_helm_revision_line<'a>(
    revision: &crate::k8s::dtos::HelmReleaseRevisionInfo,
    selected: bool,
    current_revision: Option<i32>,
    theme: &crate::ui::theme::Theme,
) -> Line<'a> {
    let row_style = if selected {
        theme.hover_style()
    } else {
        Style::default().fg(theme.fg)
    };
    let status_style = match revision.status.as_str() {
        "deployed" => Style::default().fg(theme.success),
        "failed" => Style::default().fg(theme.error),
        "superseded" | "pending-install" | "pending-upgrade" | "pending-rollback" => {
            Style::default().fg(theme.warning)
        }
        _ => Style::default().fg(theme.fg_dim),
    };
    let current_badge = if Some(revision.revision) == current_revision {
        Span::styled(" current ", theme.badge_success_style())
    } else {
        Span::styled(" target  ", theme.badge_warning_style())
    };
    let app_version = if revision.app_version.is_empty() {
        "n/a"
    } else {
        revision.app_version.as_str()
    };

    Line::from(vec![
        Span::styled(if selected { "> " } else { "  " }, row_style),
        current_badge,
        Span::raw(" "),
        Span::styled(format!("rev {:>3} ", revision.revision), row_style),
        Span::styled(format!("{:<18}", revision.status), status_style),
        Span::styled(format!(" chart:{} ", revision.chart), row_style),
        Span::styled(format!(" app:{} ", app_version), row_style),
        Span::styled(revision.updated.clone(), theme.muted_style()),
    ])
}

fn render_diff_lines(
    frame: &mut Frame,
    area: Rect,
    lines: &[crate::resource_diff::ResourceDiffLine],
    scroll: usize,
) {
    let total = lines.len();
    let window = scroll_window(total, scroll, area.height.max(1) as usize);
    let rendered = lines[window.start..window.end]
        .iter()
        .map(render_diff_line)
        .collect::<Vec<_>>();

    frame.render_widget(Paragraph::new(rendered).wrap(Wrap { trim: false }), area);
    render_scrollbar(frame, area, total, window.start);
}

fn render_diff_line<'a>(line: &crate::resource_diff::ResourceDiffLine) -> Line<'a> {
    let theme = default_theme();
    let style = match line.kind {
        ResourceDiffLineKind::Header => theme.muted_style(),
        ResourceDiffLineKind::Hunk => Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
        ResourceDiffLineKind::Context => Style::default().fg(theme.fg),
        ResourceDiffLineKind::Added => Style::default().fg(theme.success),
        ResourceDiffLineKind::Removed => Style::default().fg(theme.error),
    };
    Line::from(Span::styled(line.content.clone(), style))
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
                let ts = event.last_timestamp;
                let ts = format_local(ts, "%H:%M:%S");
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
                let ts = format_local(*started_at, "%H:%M:%S");
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

fn render_logs_tab(frame: &mut Frame, area: Rect, tab: &WorkbenchTab, _scroll: usize) {
    let theme = default_theme();
    let WorkbenchTabState::PodLogs(tab_state) = &tab.state else {
        return;
    };
    let viewer = &tab_state.viewer;

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
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
    if viewer.structured_view {
        status_spans.push(Span::styled(
            "  [json summary ON]",
            theme.keybind_desc_style(),
        ));
    }
    if let Some(request_id) = viewer.correlation_request_id.as_deref() {
        status_spans.push(Span::styled(
            format!("  [corr {}]", compact_request_id(request_id)),
            theme.badge_success_style(),
        ));
    }
    status_spans.push(Span::styled(
        format!("  [window {}]", viewer.time_window.label()),
        theme.keybind_desc_style(),
    ));
    status_spans.push(Span::raw("  "));
    let hint = if viewer.searching {
        "[Enter] apply  [Esc] cancel  [Ctrl+U] clear"
    } else if viewer.jumping_to_time {
        "[Enter] jump  [Esc] cancel  [Ctrl+U] clear"
    } else {
        "[Esc] back  [f] follow  [P] previous  [t] timestamps  [/] search  [R] regex/text  [W] window  [T] jump-to-time  [C] correlate  [J] json  [ / ] presets  [M] save preset  [n/N] next/prev  [S] save"
    };
    status_spans.push(Span::styled(hint, theme.keybind_desc_style()));
    let mut header_lines = vec![Line::from(status_spans)];
    if viewer.searching {
        header_lines.push(Line::from(vec![
            Span::styled(" Search mode: ", theme.keybind_desc_style()),
            Span::styled(
                viewer.search_mode.label(),
                query_mode_style(&theme, viewer.search_mode),
            ),
        ]));
    } else if viewer.jumping_to_time {
        let mut spans = vec![
            Span::styled(" Jump to time: ", theme.keybind_desc_style()),
            Span::styled(
                "nearest visible timestamp (RFC3339)",
                theme.keybind_desc_style(),
            ),
        ];
        if let Some(error) = &viewer.time_jump_error {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(error, theme.badge_error_style()));
        }
        header_lines.push(Line::from(spans));
    } else {
        let search_value = if viewer.search_query.is_empty() {
            "off"
        } else {
            viewer.search_query.as_str()
        };
        let mut spans = vec![
            Span::styled(" Search: ", theme.keybind_desc_style()),
            Span::styled(
                format!(
                    "{} ({}, {})",
                    search_value,
                    viewer.search_mode.label(),
                    viewer.time_window.label()
                ),
                theme.keybind_desc_style(),
            ),
        ];
        if let Some(error) = &viewer.search_error {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(error, theme.badge_error_style()));
        }
        header_lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(header_lines), sections[0]);

    // If searching or jumping to time, render input bar and reduce log area
    let log_area = if viewer.searching || viewer.jumping_to_time {
        let search_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(sections[1]);
        let (prefix, value) = if viewer.searching {
            (" /", viewer.search_input.as_str())
        } else {
            (" T", viewer.time_jump_input.as_str())
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(prefix, theme.section_title_style()),
                Span::styled(value, Style::default().fg(theme.fg)),
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

    let filtered = viewer.filtered_indices();
    if filtered.is_empty() {
        let message = if viewer.correlation_request_id.is_some() {
            " No log lines match the current correlation/time window"
        } else {
            " No log lines match the current time window"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(message, theme.inactive_style())),
            log_area,
        );
        return;
    }

    let total = filtered.len();
    let cursor = viewer.filtered_cursor(&filtered);
    let window = scroll_window(total, cursor, log_area.height.saturating_sub(1) as usize);
    let lines: Vec<Line> = filtered[window.start..window.end]
        .iter()
        .map(|index| &viewer.lines[*index])
        .map(|line| {
            render_log_message_line(
                line.display_text(viewer.structured_view),
                line.severity(),
                line.request_id(),
                LogHighlightOptions {
                    enabled: !viewer.search_query.is_empty(),
                    query: &viewer.search_query,
                    mode: viewer.search_mode,
                    compiled: viewer.compiled_search.as_ref(),
                },
                &theme,
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), log_area);
    render_scrollbar(frame, log_area, total, window.start);
}

fn highlight_search<'a>(
    line: &str,
    query: &str,
    mode: LogQueryMode,
    compiled: Option<&regex::Regex>,
    theme: &crate::ui::theme::Theme,
) -> Line<'a> {
    let ranges = highlight_ranges(line, query, mode, compiled);
    let mut spans = Vec::new();
    let mut last = 0;
    for (start, end) in ranges {
        if start > last {
            spans.push(Span::raw(line[last..start].to_string()));
        }
        spans.push(Span::styled(
            line[start..end].to_string(),
            Style::default().bg(theme.accent).fg(theme.bg),
        ));
        last = end;
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

fn render_log_message_line<'a>(
    text: &str,
    severity: Option<LogSeverity>,
    request_id: Option<&str>,
    highlight: LogHighlightOptions<'_>,
    theme: &crate::ui::theme::Theme,
) -> Line<'a> {
    let mut spans = Vec::new();
    if let Some(severity) = severity {
        spans.push(Span::styled(
            format!(" {} ", severity.badge_label()),
            severity_badge_style(theme, severity, false),
        ));
        spans.push(Span::raw(" "));
    }
    if let Some(request_id) = request_id {
        spans.push(Span::styled(
            format!(" {} ", compact_request_id(request_id)),
            theme.keybind_desc_style(),
        ));
        spans.push(Span::raw(" "));
    }

    if highlight.enabled {
        spans.extend(
            highlight_search(
                text,
                highlight.query,
                highlight.mode,
                highlight.compiled,
                theme,
            )
            .spans,
        );
    } else {
        spans.push(Span::styled(
            text.to_string(),
            Style::default().fg(theme.fg_dim),
        ));
    }

    Line::from(spans)
}

fn query_mode_style(theme: &crate::ui::theme::Theme, mode: LogQueryMode) -> Style {
    match mode {
        LogQueryMode::Substring => theme.badge_success_style(),
        LogQueryMode::Regex => theme.badge_warning_style(),
    }
}

fn severity_badge_style(
    theme: &crate::ui::theme::Theme,
    severity: LogSeverity,
    is_stderr: bool,
) -> Style {
    if is_stderr || matches!(severity, LogSeverity::Error) {
        theme.badge_error_style()
    } else if matches!(severity, LogSeverity::Warn) {
        theme.badge_warning_style()
    } else {
        theme.badge_success_style()
    }
}

fn compact_request_id(request_id: &str) -> String {
    const MAX_REQUEST_ID_CHARS: usize = 18;
    let truncated = request_id
        .chars()
        .take(MAX_REQUEST_ID_CHARS)
        .collect::<String>();
    if request_id.chars().count() > MAX_REQUEST_ID_CHARS {
        format!("req={truncated}…")
    } else {
        format!("req={truncated}")
    }
}

#[derive(Debug, Default)]
struct WorkloadLogFilterSummary {
    total: usize,
    correlated_pods: Vec<String>,
}

fn summarize_workload_log_filters(
    tab: &crate::workbench::WorkloadLogsTabState,
    now: crate::time::AppTimestamp,
) -> WorkloadLogFilterSummary {
    let mut summary = WorkloadLogFilterSummary::default();
    let track_pods = tab.correlation_request_id.is_some();

    for line in &tab.lines {
        if !tab.matches_filter_at(line, now) {
            continue;
        }
        summary.total += 1;
        if track_pods
            && !summary
                .correlated_pods
                .iter()
                .any(|pod| pod == &line.pod_name)
            && summary.correlated_pods.len() < 4
        {
            summary.correlated_pods.push(line.pod_name.clone());
        }
    }

    summary
}

fn workload_correlation_summary(summary: &WorkloadLogFilterSummary) -> Option<String> {
    if summary.total == 0 {
        return None;
    }
    match summary.correlated_pods.as_slice() {
        [] => None,
        [pod] => Some(format!(" Correlated request spans 1 pod: {pod}")),
        [first, second] => Some(format!(
            " Correlated request spans 2 pods: {first}, {second}"
        )),
        [first, second, third] => Some(format!(
            " Correlated request spans 3 pods: {first}, {second}, {third}"
        )),
        [first, second, third, ..] => Some(format!(
            " Correlated request spans 4+ pods: {first}, {second}, {third}, +more"
        )),
    }
}

fn render_workload_logs_tab(
    frame: &mut Frame,
    area: Rect,
    tab: &crate::workbench::WorkloadLogsTabState,
) {
    let theme = default_theme();
    let now = crate::time::now();
    let filter_summary = summarize_workload_log_filters(tab, now);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let mut header_spans = vec![
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
                "pod:{}  container:{}  {}:{}  window:{}",
                tab.pod_filter.as_deref().unwrap_or("all"),
                tab.container_filter.as_deref().unwrap_or("all"),
                tab.text_filter_mode.label(),
                if tab.text_filter.is_empty() {
                    "all"
                } else {
                    tab.text_filter.as_str()
                },
                tab.time_window.label()
            ),
            theme.keybind_desc_style(),
        ),
    ];
    if let Some(label) = tab.label_filter.as_deref() {
        header_spans.push(Span::raw("  "));
        header_spans.push(Span::styled(
            format!("label:{label}"),
            theme.badge_warning_style(),
        ));
    }
    if let Some(request_id) = tab.correlation_request_id.as_deref() {
        header_spans.push(Span::raw("  "));
        header_spans.push(Span::styled(
            format!("corr {}", compact_request_id(request_id)),
            theme.badge_success_style(),
        ));
    }

    let hint = if tab.editing_text_filter {
        Line::from(Span::styled(
            format!(
                " Editing {} filter: {}  [Enter] apply  [Esc] cancel  [Ctrl+U] clear",
                tab.text_filter_mode.label(),
                tab.filter_input,
            ),
            theme.keybind_desc_style(),
        ))
    } else if tab.jumping_to_time {
        Line::from(Span::styled(
            format!(
                " Jump to time: {}  [Enter] jump  [Esc] cancel  [Ctrl+U] clear",
                tab.time_jump_input,
            ),
            theme.keybind_desc_style(),
        ))
    } else {
        Line::from(Span::styled(
            "[/] text  [R] regex/text  [W] window  [T] jump-to-time  [L] label  [C] correlate  [J] json  [p] pod  [c] container  [ / ] presets  [M] save preset  [f] follow  [S] save  [Esc] back",
            theme.keybind_desc_style(),
        ))
    };

    let mut info_spans = Vec::new();
    if tab.structured_view {
        info_spans.push(Span::styled(
            " Structured JSON summary enabled",
            theme.keybind_desc_style(),
        ));
    }
    if let Some(error) = &tab.text_filter_error {
        if !info_spans.is_empty() {
            info_spans.push(Span::raw("  "));
        }
        info_spans.push(Span::styled(error, theme.badge_error_style()));
    }
    if let Some(error) = &tab.time_jump_error {
        if !info_spans.is_empty() {
            info_spans.push(Span::raw("  "));
        }
        info_spans.push(Span::styled(error, theme.badge_error_style()));
    }
    if let Some(summary) = workload_correlation_summary(&filter_summary) {
        if !info_spans.is_empty() {
            info_spans.push(Span::raw("  "));
        }
        info_spans.push(Span::styled(summary, theme.keybind_desc_style()));
    }

    let mut header_lines = vec![Line::from(header_spans), hint];
    header_lines.push(if info_spans.is_empty() {
        Line::default()
    } else {
        Line::from(info_spans)
    });
    frame.render_widget(Paragraph::new(header_lines), sections[0]);

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

    let total = filter_summary.total;
    if total == 0 {
        let message = tab.notice.as_deref().unwrap_or(if tab.loading {
            " Loading workload logs..."
        } else if tab.correlation_request_id.is_some() {
            " No workload log lines match the current correlation/filter set"
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
        .filter(|line| tab.matches_filter_at(line, now))
        .skip(window.start)
        .take(window.end.saturating_sub(window.start))
        .map(|line| {
            let badge = if line.is_stderr {
                theme.badge_warning_style()
            } else {
                theme.badge_success_style()
            };
            let mut spans = vec![Span::styled(
                format!(" {}:{} ", line.pod_name, line.container_name),
                badge,
            )];
            if let Some(severity) = line.entry.severity() {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!(" {} ", severity.badge_label()),
                    severity_badge_style(&theme, severity, line.is_stderr),
                ));
            }
            if let Some(request_id) = line.entry.request_id() {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!(" {} ", compact_request_id(request_id)),
                    theme.keybind_desc_style(),
                ));
            }
            spans.push(Span::raw(" "));
            spans.extend(
                highlight_search(
                    line.entry.display_text(tab.structured_view),
                    &tab.text_filter,
                    tab.text_filter_mode,
                    tab.compiled_text_filter.as_ref(),
                    &theme,
                )
                .spans,
            );
            Line::from(spans)
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

fn render_extension_output_tab(
    frame: &mut Frame,
    area: Rect,
    tab: &crate::workbench::ExtensionOutputTabState,
) {
    let theme = default_theme();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(area);

    let status_badge = if tab.loading {
        Span::styled(" running ", theme.badge_warning_style())
    } else if tab.success == Some(true) {
        Span::styled(" success ", theme.badge_success_style())
    } else {
        Span::styled(" failed ", theme.badge_error_style())
    };
    let mode_badge = Span::styled(format!(" {} ", tab.mode_label), theme.badge_warning_style());
    let summary = if let Some(code) = tab.exit_code {
        format!("{}  exit:{code}", tab.command_preview)
    } else {
        tab.command_preview.clone()
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                status_badge,
                Span::raw(" "),
                mode_badge,
                Span::raw(" "),
                Span::styled(summary, Style::default().fg(theme.fg)),
            ]),
            Line::from(Span::styled(
                "[j/k] scroll  [Esc] back",
                theme.keybind_desc_style(),
            )),
        ]),
        sections[0],
    );

    if tab.loading {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Running extension command...",
                theme.inactive_style(),
            )),
            sections[1],
        );
        return;
    }

    if tab.lines.is_empty() {
        let message = tab
            .error
            .as_deref()
            .map(|error| format!(" Error: {error}"))
            .unwrap_or_else(|| " Command completed without output.".to_string());
        let style = if tab.error.is_some() {
            theme.badge_error_style()
        } else {
            theme.inactive_style()
        };
        frame.render_widget(Paragraph::new(Span::styled(message, style)), sections[1]);
        return;
    }

    let total = tab.lines.len();
    let window = scroll_window(total, tab.scroll, sections[1].height.max(1) as usize);
    let lines = tab.lines[window.start..window.end]
        .iter()
        .map(|line| {
            let style = if line.starts_with("stderr:") {
                Style::default().fg(theme.warning)
            } else {
                Style::default().fg(theme.fg)
            };
            Line::from(Span::styled(line.clone(), style))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        sections[1],
    );
    render_scrollbar(frame, sections[1], total, window.start);
}

fn render_runbook_tab(frame: &mut Frame, area: Rect, tab: &crate::workbench::RunbookTabState) {
    let theme = default_theme();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let banner = tab
        .banner
        .as_deref()
        .unwrap_or("Use [Enter] to run a step, [d] mark done, [s] skip.");
    let resource = tab
        .resource
        .as_ref()
        .map(|resource| {
            format!(
                "{} {}",
                resource.kind(),
                match resource.namespace() {
                    Some(namespace) => format!("{namespace}/{}", resource.name()),
                    None => resource.name().to_string(),
                }
            )
        })
        .unwrap_or_else(|| "global".to_string());
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" runbook ", theme.badge_success_style()),
                Span::raw(" "),
                Span::styled(tab.runbook.title.clone(), theme.title_style()),
                Span::raw("  "),
                Span::styled(
                    format!("{} complete", tab.progress_label()),
                    theme.muted_style(),
                ),
                Span::raw("  "),
                Span::styled(resource, theme.inactive_style()),
            ]),
            Line::from(vec![
                Span::styled(banner, Style::default().fg(theme.fg_dim)),
                Span::raw("  "),
                Span::styled("[Esc] back", theme.keybind_desc_style()),
            ]),
        ]),
        rows[0],
    );

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(rows[1]);

    let left_block = Block::default()
        .title(Span::styled(" Steps ", theme.section_title_style()))
        .borders(Borders::ALL)
        .border_style(theme.border_style());
    let left_inner = left_block.inner(columns[0]);
    frame.render_widget(left_block, columns[0]);

    let window = centered_window(
        tab.steps.len(),
        tab.selected,
        left_inner.height.max(1) as usize,
    );
    let step_lines = if tab.steps.is_empty() {
        vec![Line::from(Span::styled(
            "No runbook steps available.",
            theme.inactive_style(),
        ))]
    } else {
        tab.steps[window.start..window.end]
            .iter()
            .enumerate()
            .map(|(offset, runtime)| {
                let absolute = window.start + offset;
                let selected = absolute == tab.selected;
                let marker = match runtime.state {
                    RunbookStepState::Pending => "[ ]",
                    RunbookStepState::Done => "[x]",
                    RunbookStepState::Skipped => "[-]",
                };
                let state_style = match runtime.state {
                    RunbookStepState::Pending => Style::default().fg(theme.fg_dim),
                    RunbookStepState::Done => theme.badge_success_style(),
                    RunbookStepState::Skipped => theme.badge_warning_style(),
                };
                let row_style = if selected {
                    theme.selection_style()
                } else {
                    Style::default().fg(theme.fg)
                };
                Line::from(vec![
                    Span::styled(if selected { "› " } else { "  " }, row_style),
                    Span::styled(marker, state_style),
                    Span::raw(" "),
                    Span::styled(runtime.step.title.clone(), row_style),
                ])
            })
            .collect::<Vec<_>>()
    };
    frame.render_widget(
        Paragraph::new(step_lines).wrap(Wrap { trim: false }),
        left_inner,
    );
    render_scrollbar(frame, left_inner, tab.steps.len(), window.start);

    let right_block = Block::default()
        .title(Span::styled(" Step Detail ", theme.section_title_style()))
        .borders(Borders::ALL)
        .border_style(theme.border_style());
    let right_inner = right_block.inner(columns[1]);
    frame.render_widget(right_block, columns[1]);

    let detail_lines = tab.selected_step().map_or_else(
        || {
            vec![Line::from(Span::styled(
                "Select a runbook step to inspect it.",
                theme.inactive_style(),
            ))]
        },
        |runtime| {
            let mut lines = Vec::new();
            lines.push(Line::from(Span::styled(
                runtime.step.title.clone(),
                theme.title_style(),
            )));
            if let Some(description) = &runtime.step.description {
                lines.push(Line::from(Span::styled(
                    description.clone(),
                    Style::default().fg(theme.fg_dim),
                )));
                lines.push(Line::default());
            }
            lines.push(Line::from(step_kind_label(&runtime.step.kind, &theme)));
            if let crate::runbooks::LoadedRunbookStepKind::Checklist { items } = &runtime.step.kind
            {
                lines.push(Line::default());
                lines.extend(items.iter().map(|item| Line::from(format!("- {item}"))));
            }
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                "[Enter] run/toggle  [d] done  [s] skip  [j/k] select",
                theme.keybind_desc_style(),
            )));
            lines
        },
    );
    frame.render_widget(
        Paragraph::new(detail_lines).wrap(Wrap { trim: false }),
        right_inner,
    );
}

fn step_kind_label<'a>(
    kind: &'a crate::runbooks::LoadedRunbookStepKind,
    theme: &crate::ui::theme::Theme,
) -> Span<'a> {
    use crate::runbooks::LoadedRunbookStepKind;

    match kind {
        LoadedRunbookStepKind::Checklist { items } => Span::styled(
            format!(
                "Checklist • {} item{}",
                items.len(),
                if items.len() == 1 { "" } else { "s" }
            ),
            theme.badge_warning_style(),
        ),
        LoadedRunbookStepKind::Workspace { name, target } => Span::styled(
            format!("{} • {}", target.label(), name),
            theme.badge_success_style(),
        ),
        LoadedRunbookStepKind::DetailAction { action } => Span::styled(
            format!("Detail Action • {}", action.label()),
            theme.badge_success_style(),
        ),
        LoadedRunbookStepKind::ExtensionAction { action_id } => Span::styled(
            format!("Extension • {}", action_id),
            theme.badge_success_style(),
        ),
        LoadedRunbookStepKind::AiWorkflow { workflow } => Span::styled(
            format!("AI Workflow • {}", workflow.default_title()),
            theme.badge_success_style(),
        ),
    }
}

fn render_ai_analysis_tab(
    frame: &mut Frame,
    area: Rect,
    tab: &crate::workbench::AiAnalysisTabState,
) {
    let theme = default_theme();
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        if tab.loading {
            Span::styled(" running ", theme.badge_warning_style())
        } else if tab.error.is_some() {
            Span::styled(" failed ", theme.badge_error_style())
        } else {
            Span::styled(" ready ", theme.badge_success_style())
        },
        Span::raw(" "),
        Span::styled(
            if tab
                .content
                .as_ref()
                .is_none_or(|content| content.model.is_empty())
            {
                "AI".to_string()
            } else {
                let content = tab
                    .content
                    .as_ref()
                    .expect("content exists when model is set");
                format!("{} • {}", content.provider_label, content.model)
            },
            Style::default().fg(theme.fg),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        "[Esc] back",
        theme.keybind_desc_style(),
    )));
    lines.push(Line::default());

    if tab.loading {
        lines.push(Line::from(Span::styled(
            "Running AI analysis...",
            theme.inactive_style(),
        )));
    } else if let Some(error) = &tab.error {
        lines.push(Line::from(Span::styled(
            format!("Error: {error}"),
            theme.badge_error_style(),
        )));
    } else if let Some(content) = &tab.content {
        {
            lines.push(Line::from(Span::styled("Summary", theme.title_style())));
            lines.push(Line::from(content.summary.clone()));
            lines.push(Line::default());
        }
        render_ai_section(&mut lines, "Likely Causes", &content.likely_causes, &theme);
        render_ai_section(&mut lines, "Next Steps", &content.next_steps, &theme);
        render_ai_section(&mut lines, "Uncertainty", &content.uncertainty, &theme);
    }

    let scroll = tab.scroll.min(lines.len().saturating_sub(1));
    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll.min(u16::MAX as usize) as u16, 0));
    frame.render_widget(paragraph, area);
    render_scrollbar(frame, area, tab.rendered_line_count(), scroll);
}

fn render_ai_section(
    lines: &mut Vec<Line<'static>>,
    title: &str,
    items: &[String],
    theme: &crate::ui::theme::Theme,
) {
    if items.is_empty() {
        return;
    }
    lines.push(Line::from(Span::styled(
        title.to_string(),
        theme.title_style(),
    )));
    lines.extend(items.iter().map(|item| Line::from(format!("• {item}"))));
    lines.push(Line::default());
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
    use super::{VisibleWindow, access_review_lines, centered_window, scroll_window};
    use crate::{
        app::ResourceRef,
        authorization::{ActionAccessReview, DetailActionAuthorization, ResourceAccessCheck},
        k8s::dtos::RbacRule,
        policy::DetailAction,
        rbac_subjects::{
            AccessReviewSubject, SubjectAccessReview, SubjectBindingResolution,
            SubjectRoleResolution,
        },
        ui::{components::default_theme, truncate_message},
        workbench::{AccessReviewTabState, AttemptedActionReview},
    };

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

    #[test]
    fn workload_correlation_summary_uses_exact_small_counts() {
        assert_eq!(
            super::workload_correlation_summary(&super::WorkloadLogFilterSummary {
                total: 3,
                correlated_pods: vec!["api-0".into(), "api-1".into()],
            }),
            Some(" Correlated request spans 2 pods: api-0, api-1".to_string())
        );
    }

    #[test]
    fn workload_correlation_summary_caps_large_pod_sets() {
        assert_eq!(
            super::workload_correlation_summary(&super::WorkloadLogFilterSummary {
                total: 8,
                correlated_pods: vec![
                    "api-0".into(),
                    "api-1".into(),
                    "worker-0".into(),
                    "worker-1".into(),
                ],
            }),
            Some(" Correlated request spans 4+ pods: api-0, api-1, worker-0, +more".to_string())
        );
    }

    #[test]
    fn access_review_lines_include_subject_reverse_lookup_section() {
        let tab = AccessReviewTabState::new(
            ResourceRef::ServiceAccount("api".into(), "payments".into()),
            Some("prod".into()),
            "payments".into(),
            Vec::new(),
            Some(SubjectAccessReview {
                subject: AccessReviewSubject::ServiceAccount {
                    name: "api".into(),
                    namespace: "payments".into(),
                },
                bindings: vec![SubjectBindingResolution {
                    binding: ResourceRef::RoleBinding("payments-view".into(), "payments".into()),
                    role: SubjectRoleResolution {
                        resource: Some(ResourceRef::Role(
                            "payments-reader".into(),
                            "payments".into(),
                        )),
                        kind: "Role".into(),
                        name: "payments-reader".into(),
                        namespace: Some("payments".into()),
                        rules: vec![RbacRule {
                            verbs: vec!["get".into(), "list".into()],
                            resources: vec!["pods".into()],
                            ..RbacRule::default()
                        }],
                        missing: false,
                    },
                }],
            }),
            None,
        );

        let rendered = access_review_lines(&tab, &default_theme())
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Subject: ServiceAccount payments/api"));
        assert!(rendered.contains("RoleBinding payments/payments-view"));
        assert!(rendered.contains("Role payments/payments-reader"));
        assert!(rendered.contains("get, list pods"));
    }

    #[test]
    fn access_review_attempted_action_summary_is_rendered_and_scrolled_into_view() {
        let tab = AccessReviewTabState::new(
            ResourceRef::Deployment("api".into(), "payments".into()),
            Some("prod".into()),
            "payments".into(),
            vec![
                ActionAccessReview {
                    action: DetailAction::ViewYaml,
                    authorization: Some(DetailActionAuthorization::Allowed),
                    strict: false,
                    checks: vec![],
                },
                ActionAccessReview {
                    action: DetailAction::Delete,
                    authorization: Some(DetailActionAuthorization::Denied),
                    strict: true,
                    checks: vec![ResourceAccessCheck::resource(
                        "delete",
                        None,
                        "deployments",
                        Some("payments"),
                        Some("api"),
                    )],
                },
            ],
            None,
            Some(AttemptedActionReview {
                action: DetailAction::Delete,
                authorization: Some(DetailActionAuthorization::Denied),
                strict: true,
                checks: vec![ResourceAccessCheck::resource(
                    "delete",
                    Some("apps"),
                    "deployments",
                    Some("payments"),
                    Some("api"),
                )],
                note: None,
            }),
        );

        let rendered = access_review_lines(&tab, &default_theme())
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Attempted Action: Delete"));
        assert!(rendered.contains("[denied]"));
        assert!(tab.scroll > 0);
    }

    #[test]
    fn access_review_lines_render_subject_input_help() {
        let tab = AccessReviewTabState::new(
            ResourceRef::Pod("api-0".into(), "payments".into()),
            Some("prod".into()),
            "payments".into(),
            Vec::new(),
            None,
            None,
        );

        let rendered = access_review_lines(&tab, &default_theme())
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Review Subject:"));
        assert!(rendered.contains("ServiceAccount/<namespace>/<name>"));
    }

    #[test]
    fn access_review_lines_group_checks_by_scope() {
        let tab = AccessReviewTabState::new(
            ResourceRef::Pod("api-0".into(), "payments".into()),
            Some("prod".into()),
            "payments".into(),
            vec![ActionAccessReview {
                action: DetailAction::Logs,
                authorization: Some(DetailActionAuthorization::Denied),
                strict: false,
                checks: vec![
                    ResourceAccessCheck::resource(
                        "get",
                        None,
                        "namespaces",
                        None,
                        Some("payments"),
                    ),
                    ResourceAccessCheck::subresource(
                        "get",
                        None,
                        "pods",
                        "log",
                        Some("payments"),
                        Some("api-0"),
                    ),
                ],
            }],
            None,
            None,
        );

        let rendered = access_review_lines(&tab, &default_theme())
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Namespace-scoped checks"));
        assert!(rendered.contains("Cluster-scoped checks"));
    }

    #[test]
    fn access_review_lines_label_role_scope() {
        let tab = AccessReviewTabState::new(
            ResourceRef::ServiceAccount("api".into(), "payments".into()),
            Some("prod".into()),
            "payments".into(),
            Vec::new(),
            Some(SubjectAccessReview {
                subject: AccessReviewSubject::ServiceAccount {
                    name: "api".into(),
                    namespace: "payments".into(),
                },
                bindings: vec![SubjectBindingResolution {
                    binding: ResourceRef::ClusterRoleBinding("api-admin".into()),
                    role: SubjectRoleResolution {
                        resource: Some(ResourceRef::ClusterRole("ops-admin".into())),
                        kind: "ClusterRole".into(),
                        name: "ops-admin".into(),
                        namespace: None,
                        rules: vec![],
                        missing: false,
                    },
                }],
            }),
            None,
        );

        let rendered = access_review_lines(&tab, &default_theme())
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("ClusterRole ops-admin  [cluster]"));
    }

    #[test]
    fn access_review_line_count_matches_rendered_lines_for_grouped_sections() {
        let tab = AccessReviewTabState::new(
            ResourceRef::ServiceAccount("api".into(), "payments".into()),
            Some("prod".into()),
            "payments".into(),
            vec![ActionAccessReview {
                action: DetailAction::Delete,
                authorization: Some(DetailActionAuthorization::Denied),
                strict: true,
                checks: vec![
                    ResourceAccessCheck::resource(
                        "get",
                        None,
                        "namespaces",
                        None,
                        Some("payments"),
                    ),
                    ResourceAccessCheck::resource(
                        "delete",
                        None,
                        "pods",
                        Some("payments"),
                        Some("api-0"),
                    ),
                ],
            }],
            Some(SubjectAccessReview {
                subject: AccessReviewSubject::ServiceAccount {
                    name: "api".into(),
                    namespace: "payments".into(),
                },
                bindings: vec![
                    SubjectBindingResolution {
                        binding: ResourceRef::RoleBinding(
                            "payments-view".into(),
                            "payments".into(),
                        ),
                        role: SubjectRoleResolution {
                            resource: Some(ResourceRef::Role(
                                "payments-reader".into(),
                                "payments".into(),
                            )),
                            kind: "Role".into(),
                            name: "payments-reader".into(),
                            namespace: Some("payments".into()),
                            rules: vec![RbacRule {
                                verbs: vec!["get".into()],
                                resources: vec!["pods".into()],
                                ..RbacRule::default()
                            }],
                            missing: false,
                        },
                    },
                    SubjectBindingResolution {
                        binding: ResourceRef::ClusterRoleBinding("api-admin".into()),
                        role: SubjectRoleResolution {
                            resource: Some(ResourceRef::ClusterRole("ops-admin".into())),
                            kind: "ClusterRole".into(),
                            name: "ops-admin".into(),
                            namespace: None,
                            rules: vec![],
                            missing: false,
                        },
                    },
                ],
            }),
            Some(AttemptedActionReview {
                action: DetailAction::Delete,
                authorization: Some(DetailActionAuthorization::Denied),
                strict: true,
                checks: vec![ResourceAccessCheck::resource(
                    "delete",
                    Some("apps"),
                    "deployments",
                    Some("payments"),
                    Some("api"),
                )],
                note: None,
            }),
        );

        assert_eq!(
            tab.line_count(),
            access_review_lines(&tab, &default_theme()).len()
        );
    }
}
