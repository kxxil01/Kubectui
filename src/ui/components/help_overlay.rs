//! Keybinding help overlay displayed with `?`.

use std::cell::Cell;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Wrap,
    },
};

use crate::ui::{components::default_theme, keybindings::LOG_PRESET_KEYS_HINT, wrapped_line_count};
use crate::{app::DetailViewState, policy::DetailAction};

#[derive(Debug, Clone, Default)]
pub struct HelpOverlay {
    is_open: bool,
    scroll: usize,
    last_visible_height: Cell<usize>,
}

const DETAIL_FALLBACK_BINDINGS: &[(&str, &str)] = &[
    ("D", "View config drift / drain node"),
    ("h", "View Helm revision history / rollback"),
    ("O", "View rollout control center"),
    ("A", "Open RBAC access review for this resource"),
    ("y", "View YAML"),
    ("o", "View decoded Secret data"),
    ("B", "Toggle bookmark"),
    ("v", "View timeline"),
    ("l", "View logs"),
    ("g", "Launch debug container / node debug shell"),
    ("x", "Exec into pod"),
    ("f", "Port forward"),
    ("s", "Scale replicas"),
    ("p", "Probe panel"),
    ("R", "Restart rollout / reconcile Flux resource"),
    ("e", "Edit YAML"),
    ("d", "Delete resource"),
    ("F", "Force delete/drain (in confirm dialog)"),
    ("T", "Trigger CronJob"),
    ("S", "Pause/resume CronJob"),
    ("N", "View network policy analysis"),
    ("C", "Check pod reachability (policy intent)"),
    ("t", "Open traffic debug (service / ingress / DNS path)"),
    ("w", "View relations"),
    ("c", "Cordon node"),
    ("u", "Uncordon node"),
];

const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Global",
        &[
            ("?", "Toggle this help"),
            ("Esc then Enter", "Quit"),
            ("Esc", "Back / close overlay"),
            ("Tab / Shift+Tab", "Next / previous view"),
            ("j / k / \u{2193} / \u{2191}", "Navigate list"),
            ("Enter", "Open detail / activate"),
            (";", "Toggle list / secondary pane focus"),
            ("/", "Search / filter"),
            ("~", "Namespace picker"),
            ("c", "Context picker"),
            (
                ":",
                "Action palette (resources, recent activity, views, actions, runbooks, workspaces, banks, templates, AI, columns)",
            ),
            ("r", "Refresh data"),
            ("Ctrl+y", "Copy resource name"),
            ("Y", "Copy namespace/name"),
            ("B", "Toggle bookmark for selected resource"),
            ("W", "Save current workspace"),
            ("{ / }", "Previous / next saved workspace"),
            ("T", "Cycle theme"),
            ("I", "Cycle icon mode"),
            ("b", "Toggle workbench"),
            (", / .", "Previous / next workbench tab"),
            ("Ctrl+W", "Close workbench tab"),
            ("Ctrl+Up / Ctrl+Down", "Resize workbench"),
        ],
    ),
    (
        "Sort (Pods)",
        &[
            ("n", "Sort by name"),
            ("a / 1", "Sort by age"),
            ("2", "Sort by status"),
            ("3", "Sort by restarts"),
            ("0", "Clear sort"),
        ],
    ),
    (
        "Sort (Other Views)",
        &[
            ("n", "Sort by name"),
            ("a / 1", "Sort by age"),
            ("0", "Clear sort"),
        ],
    ),
    (
        "Runbooks",
        &[
            (":", "Open incident packs and guided runbooks"),
            ("Enter", "Run or toggle the selected step"),
            ("d", "Mark step done"),
            ("s", "Skip or unskip the selected step"),
        ],
    ),
    (
        "Flux",
        &[
            (
                "R",
                "Reconcile selected Flux resource (Flux views/detail only)",
            ),
            ("Ctrl+R", "Refresh data"),
        ],
    ),
    (
        "Workbench (focused)",
        &[
            ("z", "Maximize / restore"),
            (", / .", "Previous / next workbench tab"),
            ("Ctrl+W", "Close active workbench tab"),
            ("Ctrl+Up / Ctrl+Down", "Resize workbench"),
            ("j / k", "Scroll down / up"),
            ("g / G", "Jump to top / bottom"),
            ("PageDown / PageUp", "Scroll by page"),
            ("Esc", "Un-maximize or blur"),
        ],
    ),
    (
        "Exec (workbench)",
        &[
            ("Esc", "Enter controls mode from exec input"),
            ("z", "Maximize / restore in controls mode"),
            (", / .", "Previous / next tab in controls mode"),
            ("Ctrl+W", "Close tab in controls mode"),
            ("i / Enter", "Return to exec input from controls mode"),
            ("Ctrl+L", "Clear exec output"),
            ("Up / Down", "Previous / next command"),
            ("PageUp / PageDown", "Scroll output"),
        ],
    ),
    (
        "Helm History (workbench)",
        &[
            ("Enter", "Open values diff for selected revision"),
            ("R", "Open rollback confirmation for selected revision"),
            (
                "y / Enter / R",
                "Confirm rollback when confirmation is open",
            ),
            ("Esc", "Cancel rollback confirmation / close values diff"),
        ],
    ),
    (
        "Decoded Secret",
        &[
            ("m", "Toggle masked / visible values"),
            ("e / Enter", "Edit selected text value"),
            ("s", "Save decoded Secret changes"),
            ("Ctrl+U", "Clear current editor input"),
            ("Esc", "Cancel edit or return focus"),
        ],
    ),
    (
        "Bookmarks",
        &[
            ("Enter", "Jump to bookmarked resource"),
            ("B", "Remove bookmark from current cluster"),
        ],
    ),
    (
        "Projects",
        &[
            ("Enter", "Open representative resource for selected project"),
            ("/", "Filter by project, namespace, or related resource"),
            ("PageDown / PageUp", "Scroll summary"),
            ("; then j/k", "Focus and scroll summary"),
            ("Ctrl+f / Ctrl+b", "Scroll summary by page"),
            ("Ctrl+d / Ctrl+u", "Scroll summary faster"),
            ("W", "Save workspace jump with current project filter"),
        ],
    ),
    (
        "Governance",
        &[
            (
                "Enter",
                "Open representative resource for selected namespace",
            ),
            ("/", "Filter by namespace, project, or risk"),
            ("PageDown / PageUp", "Scroll summary"),
            ("; then j/k", "Focus and scroll summary"),
            ("Ctrl+f / Ctrl+b", "Scroll summary by page"),
            ("Ctrl+d / Ctrl+u", "Scroll summary faster"),
        ],
    ),
    (
        "RBAC Detail Panes",
        &[
            ("PageDown / PageUp", "Scroll selected row detail"),
            ("; then j/k", "Focus and scroll selected row detail"),
            ("Ctrl+f / Ctrl+b", "Scroll selected row detail by page"),
            ("Ctrl+d / Ctrl+u", "Scroll selected row detail faster"),
            ("Enter", "Open selected RBAC resource"),
        ],
    ),
    (
        "Dashboard & Metrics",
        &[
            ("", "5 gauges: Nodes, Pods, Workload, CPU, Mem"),
            ("", "Overcommit & governance panel"),
            ("", "Top-5 CPU/memory pod consumers"),
            ("", "Namespace utilization with %CPU/R %MEM/R"),
            ("PageDown / PageUp", "Scroll alerts"),
            ("; then j/k", "Focus and scroll alerts"),
            ("Ctrl+f / Ctrl+b", "Scroll alerts by page"),
            (":", "Toggle pod metric columns via palette"),
        ],
    ),
    (
        "Node Actions",
        &[
            ("c", "Cordon node"),
            ("u", "Uncordon node"),
            ("D", "Drain node (with confirmation)"),
        ],
    ),
    (
        "Relations Tree",
        &[
            ("j / k", "Move cursor down / up"),
            ("l / Right", "Expand node"),
            ("h / Left", "Collapse / jump to parent"),
            ("g / G", "Jump to top / bottom"),
            ("Enter", "Open detail for resource"),
            ("Esc", "Return focus from workbench"),
        ],
    ),
    (
        "Logs",
        &[
            ("f", "Toggle follow mode"),
            ("P", "Toggle previous logs"),
            ("t", "Toggle timestamps"),
            ("/", "Search in logs"),
            ("R", "Toggle text / regex search"),
            ("W", "Cycle time window"),
            ("T", "Jump to timestamp"),
            ("C", "Toggle request-id correlation"),
            ("J", "Toggle structured JSON summary"),
            (
                LOG_PRESET_KEYS_HINT,
                "Previous / next saved preset (logs tabs)",
            ),
            ("M / m", "Save current log preset"),
            ("Enter / Esc", "Apply / cancel log search"),
            ("Ctrl+U", "Clear log search input"),
            ("n / N", "Next / previous match"),
            ("y", "Copy log content"),
            ("S / s", "Save logs to file"),
            (", / .", "Previous / next workbench tab"),
        ],
    ),
    (
        "Workload Logs",
        &[
            ("f", "Toggle follow mode"),
            ("p", "Cycle pod filter"),
            ("c", "Cycle container filter"),
            ("/", "Text filter"),
            ("R", "Toggle text / regex filter"),
            ("W", "Cycle time window"),
            ("T", "Jump to timestamp"),
            ("L", "Cycle pod label filter"),
            ("C", "Toggle request-id correlation"),
            ("J", "Toggle structured JSON summary"),
            (
                LOG_PRESET_KEYS_HINT,
                "Previous / next saved preset (logs tabs)",
            ),
            ("M / m", "Save current log preset"),
            ("Enter / Esc", "Apply / cancel text filter"),
            ("Ctrl+U", "Clear text filter input"),
            ("y", "Copy log content"),
            ("S / s", "Save logs to file"),
            (", / .", "Previous / next workbench tab"),
        ],
    ),
];

impl HelpOverlay {
    pub fn open(&mut self) {
        self.is_open = true;
        self.scroll = 0;
        self.last_visible_height.set(0);
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.last_visible_height.set(0);
    }

    pub fn toggle(&mut self) {
        if self.is_open {
            self.close();
        } else {
            self.open();
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn scroll_page_down(&mut self) {
        self.scroll = self.scroll.saturating_add(self.page_step());
    }

    pub fn scroll_page_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(self.page_step());
    }

    fn page_step(&self) -> usize {
        let last_visible_height = self.last_visible_height.get();
        if last_visible_height == 0 {
            10
        } else {
            last_visible_height.saturating_sub(1).max(1)
        }
    }

    pub fn total_lines() -> usize {
        let mut count = 0;
        count += 1;
        count += detail_bindings(None).len();
        count += 1;
        for (_, bindings) in SECTIONS {
            count += 1; // section header
            count += bindings.len();
            count += 1; // blank line
        }
        count
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, detail: Option<&DetailViewState>) {
        let theme = default_theme();

        let popup_width = 60u16.min(area.width.saturating_sub(4));
        let popup_height = 30u16.min(area.height.saturating_sub(4));
        let popup = centered_rect(popup_width, popup_height, area);
        frame.render_widget(Clear, popup);

        let block = Block::default()
            .title(Span::styled(
                " Keybindings ",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_style())
            .style(Style::default().bg(theme.bg_surface));

        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(Span::styled(
            "  Detail View",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )));
        for (key, desc) in detail_bindings(detail) {
            lines.push(Line::from(vec![
                Span::styled(format!("    {key:<24}"), Style::default().fg(theme.fg)),
                Span::styled(desc, Style::default().fg(theme.fg_dim)),
            ]));
        }
        lines.push(Line::from(""));

        for (section_name, bindings) in SECTIONS {
            lines.push(Line::from(Span::styled(
                format!("  {section_name}"),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )));
            for (key, desc) in *bindings {
                lines.push(Line::from(vec![
                    Span::styled(format!("    {key:<24}"), Style::default().fg(theme.fg)),
                    Span::styled(*desc, Style::default().fg(theme.fg_dim)),
                ]));
            }
            lines.push(Line::from(""));
        }

        let footer_lines = vec![Line::from(vec![
            Span::styled(
                " [?/Esc] close  [j/k] scroll ",
                Style::default().fg(theme.fg_dim),
            ),
            Span::styled(
                format!(" [{}/{}] ", 0, 0),
                Style::default().fg(theme.accent),
            ),
        ])];
        let footer_height =
            crate::ui::wrapped_line_count(&footer_lines, inner.width.max(1)).max(1) as u16;
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(footer_height)])
            .split(inner);

        let visible_height = sections[0].height as usize;
        self.last_visible_height.set(visible_height);
        let (scroll, end, total) =
            help_overlay_window(&lines, sections[0].width, visible_height, self.scroll);

        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((scroll.min(u16::MAX as usize) as u16, 0)),
            sections[0],
        );
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");
        let mut scrollbar_state = ScrollbarState::new(total).position(scroll);
        frame.render_stateful_widget(scrollbar, sections[0], &mut scrollbar_state);

        let footer_lines = vec![Line::from(vec![
            Span::styled(
                " [?/Esc] close  [j/k] line  [PgUp/PgDn] page ",
                Style::default().fg(theme.fg_dim),
            ),
            Span::styled(
                format!(" [{end}/{total}] "),
                Style::default().fg(theme.accent),
            ),
        ])];
        frame.render_widget(
            Paragraph::new(footer_lines).wrap(Wrap { trim: false }),
            sections[1],
        );
    }
}

fn help_overlay_window(
    lines: &[Line<'_>],
    width: u16,
    visible_height: usize,
    scroll: usize,
) -> (usize, usize, usize) {
    let visible_height = visible_height.max(1);
    let total_lines = wrapped_line_count(lines, width);
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll = scroll.min(max_scroll);
    let end = (scroll + visible_height).min(total_lines);
    (scroll, end, total_lines)
}

fn detail_bindings(detail: Option<&DetailViewState>) -> Vec<(&'static str, &'static str)> {
    let Some(detail) = detail else {
        return DETAIL_FALLBACK_BINDINGS.to_vec();
    };

    let mut bindings = Vec::with_capacity(DETAIL_FALLBACK_BINDINGS.len());

    if detail.supports_action(DetailAction::Drain) {
        bindings.push(("D", "Drain node (with confirmation)"));
    } else if detail.supports_action(DetailAction::ViewConfigDrift) {
        bindings.push(("D", "View config drift (live vs last-applied)"));
    }
    if detail.supports_action(DetailAction::ViewHelmHistory) {
        bindings.push(("h", "View Helm revision history / rollback"));
    }
    if detail.supports_action(DetailAction::ViewRollout) {
        bindings.push(("O", "View rollout control center"));
    }
    if detail.supports_action(DetailAction::ViewAccessReview) {
        bindings.push(("A", "Open RBAC access review for this resource"));
    }
    if detail.supports_action(DetailAction::ViewYaml) {
        bindings.push(("y", "View YAML"));
    }
    if detail.supports_action(DetailAction::ViewDecodedSecret) {
        bindings.push(("o", "View decoded Secret data"));
    }
    if detail.supports_action(DetailAction::ToggleBookmark) {
        bindings.push(("B", "Toggle bookmark"));
    }
    if detail.supports_action(DetailAction::ViewEvents) {
        bindings.push(("v", "View timeline"));
    }
    if detail.supports_action(DetailAction::Logs) {
        bindings.push(("l", "View logs"));
    }
    if detail.supports_action(DetailAction::NodeDebugShell) {
        bindings.push(("g", "Launch node debug shell"));
    } else if detail.supports_action(DetailAction::DebugContainer) {
        bindings.push(("g", "Launch debug container"));
    }
    if detail.supports_action(DetailAction::Exec) {
        bindings.push(("x", "Exec into pod"));
    }
    if detail.supports_action(DetailAction::PortForward) {
        bindings.push(("f", "Port forward"));
    }
    if detail.supports_action(DetailAction::Scale) {
        bindings.push(("s", "Scale replicas"));
    }
    if detail.supports_action(DetailAction::Probes) {
        bindings.push(("p", "Probe panel"));
    }
    if detail.supports_action(DetailAction::Restart) {
        bindings.push(("R", "Restart rollout"));
    } else if detail.supports_action(DetailAction::FluxReconcile) {
        bindings.push(("R", "Reconcile Flux resource"));
    }
    if detail.supports_action(DetailAction::EditYaml) {
        bindings.push(("e", "Edit YAML"));
    }
    if detail.supports_action(DetailAction::Delete) {
        bindings.push(("d", "Delete resource"));
    }
    if detail.confirm_drain {
        bindings.push(("F", "Force drain (in confirm dialog)"));
    } else if detail.confirm_delete {
        bindings.push(("F", "Force delete (in confirm dialog)"));
    }
    if detail.supports_action(DetailAction::Trigger) {
        bindings.push(("T", "Trigger CronJob"));
    }
    if detail.supports_action(DetailAction::SuspendCronJob) {
        bindings.push(("S", "Pause CronJob"));
    } else if detail.supports_action(DetailAction::ResumeCronJob) {
        bindings.push(("S", "Resume CronJob"));
    }
    if detail.supports_action(DetailAction::ViewNetworkPolicies) {
        bindings.push(("N", "View network policy analysis"));
    }
    if detail.supports_action(DetailAction::CheckNetworkConnectivity) {
        bindings.push(("C", "Check pod reachability (policy intent)"));
    }
    if detail.supports_action(DetailAction::ViewTrafficDebug) {
        bindings.push(("t", "Open traffic debug (service / ingress / DNS path)"));
    }
    if detail.supports_action(DetailAction::ViewRelationships) {
        bindings.push(("w", "View relations"));
    }
    if detail.supports_action(DetailAction::Cordon) {
        bindings.push(("c", "Cordon node"));
    }
    if detail.supports_action(DetailAction::Uncordon) {
        bindings.push(("u", "Uncordon node"));
    }

    bindings
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{DetailViewState, ResourceRef};

    #[test]
    fn help_overlay_toggle() {
        let mut overlay = HelpOverlay::default();
        assert!(!overlay.is_open());
        overlay.toggle();
        assert!(overlay.is_open());
        overlay.toggle();
        assert!(!overlay.is_open());
    }

    #[test]
    fn help_overlay_scroll() {
        let mut overlay = HelpOverlay::default();
        overlay.open();
        assert_eq!(overlay.scroll, 0);
        overlay.scroll_down();
        assert_eq!(overlay.scroll, 1);
        overlay.scroll_up();
        assert_eq!(overlay.scroll, 0);
        overlay.scroll_up();
        assert_eq!(overlay.scroll, 0);
    }

    #[test]
    fn help_overlay_page_scroll_uses_last_visible_height() {
        let mut overlay = HelpOverlay::default();
        overlay.open();
        overlay.last_visible_height.set(12);

        overlay.scroll_page_down();
        assert_eq!(overlay.scroll, 11);

        overlay.scroll_page_up();
        assert_eq!(overlay.scroll, 0);
    }

    #[test]
    fn total_lines_is_nonzero() {
        assert!(HelpOverlay::total_lines() > 20);
    }

    #[test]
    fn help_overlay_window_clamps_scroll_to_last_page() {
        let lines = vec![Line::from("row"); 100];
        let (scroll, end, total) = help_overlay_window(&lines, 20, 10, 999);
        assert_eq!(scroll, 90);
        assert_eq!(end, 100);
        assert_eq!(total, 100);
    }

    #[test]
    fn detail_bindings_show_drift_for_non_node_detail() {
        let detail = DetailViewState {
            resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
            ..DetailViewState::default()
        };

        let bindings = detail_bindings(Some(&detail));
        assert!(bindings.contains(&("D", "View config drift (live vs last-applied)")));
        assert!(!bindings.contains(&("h", "View Helm revision history / rollback")));
        assert!(!bindings.contains(&("O", "View rollout control center")));
        assert!(bindings.contains(&("g", "Launch debug container")));
        assert!(bindings.contains(&("C", "Check pod reachability (policy intent)")));
        assert!(bindings.contains(&("t", "Open traffic debug (service / ingress / DNS path)")));
    }

    #[test]
    fn detail_bindings_show_drain_for_node_detail() {
        let detail = DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            ..DetailViewState::default()
        };

        let bindings = detail_bindings(Some(&detail));
        assert!(bindings.contains(&("D", "Drain node (with confirmation)")));
        assert!(!bindings.contains(&("D", "View config drift (live vs last-applied)")));
        assert!(bindings.contains(&("g", "Launch node debug shell")));
        assert!(!bindings.contains(&("C", "Check pod reachability")));
    }

    #[test]
    fn detail_bindings_show_helm_history_for_helm_release_detail() {
        let detail = DetailViewState {
            resource: Some(ResourceRef::HelmRelease(
                "my-app".to_string(),
                "demo".to_string(),
            )),
            ..DetailViewState::default()
        };

        let bindings = detail_bindings(Some(&detail));
        assert!(bindings.contains(&("h", "View Helm revision history / rollback")));
    }

    #[test]
    fn detail_bindings_show_rollout_for_deployment_detail() {
        let detail = DetailViewState {
            resource: Some(ResourceRef::Deployment(
                "api".to_string(),
                "default".to_string(),
            )),
            ..DetailViewState::default()
        };

        let bindings = detail_bindings(Some(&detail));
        assert!(bindings.contains(&("O", "View rollout control center")));
    }

    #[test]
    fn detail_bindings_show_traffic_debug_for_service_detail() {
        let detail = DetailViewState {
            resource: Some(ResourceRef::Service(
                "api".to_string(),
                "default".to_string(),
            )),
            ..DetailViewState::default()
        };

        let bindings = detail_bindings(Some(&detail));
        assert!(bindings.contains(&("t", "Open traffic debug (service / ingress / DNS path)")));
        assert!(!bindings.contains(&("C", "Check pod reachability (policy intent)")));
    }

    #[test]
    fn detail_bindings_hide_unsupported_shortcuts_for_current_resource() {
        let detail = DetailViewState {
            resource: Some(ResourceRef::Deployment(
                "api".to_string(),
                "default".to_string(),
            )),
            ..DetailViewState::default()
        };

        let bindings = detail_bindings(Some(&detail));
        assert!(!bindings.contains(&("o", "View decoded Secret data")));
        assert!(!bindings.contains(&("x", "Exec into pod")));
        assert!(!bindings.contains(&("p", "Probe panel")));
        assert!(bindings.contains(&("R", "Restart rollout")));
    }

    #[test]
    fn detail_bindings_use_flux_specific_r_label_for_flux_resource() {
        let detail = DetailViewState {
            resource: Some(ResourceRef::CustomResource {
                name: "apps".to_string(),
                namespace: Some("flux-system".to_string()),
                group: "kustomize.toolkit.fluxcd.io".to_string(),
                version: "v1".to_string(),
                kind: "Kustomization".to_string(),
                plural: "kustomizations".to_string(),
            }),
            ..DetailViewState::default()
        };

        let bindings = detail_bindings(Some(&detail));
        assert!(bindings.contains(&("R", "Reconcile Flux resource")));
        assert!(!bindings.contains(&("R", "Restart rollout")));
    }

    #[test]
    fn detail_bindings_force_shortcut_matches_active_confirmation_mode() {
        let node_detail = DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            confirm_drain: true,
            ..DetailViewState::default()
        };

        let node_bindings = detail_bindings(Some(&node_detail));
        assert!(node_bindings.contains(&("F", "Force drain (in confirm dialog)")));
        assert!(!node_bindings.contains(&("F", "Force delete (in confirm dialog)")));
    }

    #[test]
    fn help_lists_canonical_workbench_tab_keys() {
        let global = SECTIONS
            .iter()
            .find(|(title, _)| *title == "Global")
            .expect("global section exists")
            .1;

        assert!(
            global
                .iter()
                .any(|(key, desc)| { *key == ", / ." && *desc == "Previous / next workbench tab" }),
            "global help must list canonical terminal-safe workbench tab keys"
        );
        for (section, bindings) in SECTIONS {
            for (key, desc) in *bindings {
                let is_workbench_tab_switch = desc.contains("Previous / next workbench tab");
                if is_workbench_tab_switch {
                    assert_eq!(
                        *key, ", / .",
                        "{section} must use canonical workbench tab keys"
                    );
                    assert!(
                        !key.contains("Ctrl+Tab")
                            && !key.contains("Ctrl+Shift+Tab")
                            && !key.contains("[ / ]"),
                        "{section} must not advertise ambiguous workbench tab keys"
                    );
                }
            }
        }
    }

    #[test]
    fn focused_workbench_help_lists_resize_shortcuts() {
        let section = SECTIONS
            .iter()
            .find(|(title, _)| *title == "Workbench (focused)")
            .expect("workbench section exists")
            .1;

        assert!(
            section.iter().any(|(key, desc)| {
                *key == "Ctrl+Up / Ctrl+Down" && *desc == "Resize workbench"
            })
        );
    }

    #[test]
    fn exec_help_lists_controls_mode_escape_handoff() {
        let section = SECTIONS
            .iter()
            .find(|(title, _)| *title == "Exec (workbench)")
            .expect("exec section exists")
            .1;

        assert!(section.iter().any(|(key, desc)| {
            *key == "Esc" && *desc == "Enter controls mode from exec input"
        }));
        assert!(
            section.iter().any(|(key, desc)| {
                *key == "z" && *desc == "Maximize / restore in controls mode"
            })
        );
        assert!(section.iter().any(|(key, desc)| {
            *key == "i / Enter" && *desc == "Return to exec input from controls mode"
        }));
    }

    #[test]
    fn help_lists_unambiguous_log_preset_keys() {
        for section_name in ["Logs", "Workload Logs"] {
            let section = SECTIONS
                .iter()
                .find(|(title, _)| *title == section_name)
                .expect("logs help section exists")
                .1;

            assert!(
                section.iter().any(|(key, desc)| {
                    *key == LOG_PRESET_KEYS_HINT
                        && *desc == "Previous / next saved preset (logs tabs)"
                }),
                "{section_name} must list literal bracket preset keys"
            );
            assert!(
                !section.iter().any(|(key, _)| *key == "[ / ]"),
                "{section_name} must not render preset keys like slash"
            );
        }
    }

    #[test]
    fn global_help_lists_exact_quit_sequence() {
        let global = SECTIONS
            .iter()
            .find(|(title, _)| *title == "Global")
            .expect("global section exists")
            .1;

        assert!(
            global
                .iter()
                .any(|(key, desc)| *key == "Esc then Enter" && *desc == "Quit")
        );
        assert!(
            !global
                .iter()
                .any(|(key, desc)| key.contains('q') && desc.contains("Quit")),
            "help must not imply q exits"
        );
    }

    #[test]
    fn helm_history_help_lists_diff_and_rollback_shortcuts() {
        let section = SECTIONS
            .iter()
            .find(|(title, _)| *title == "Helm History (workbench)")
            .expect("helm history section exists")
            .1;

        assert!(
            section
                .iter()
                .any(|(key, desc)| *key == "Enter" && desc.contains("values diff"))
        );
        assert!(
            section
                .iter()
                .any(|(key, desc)| *key == "R" && desc.contains("rollback confirmation"))
        );
        assert!(
            section
                .iter()
                .any(|(key, desc)| *key == "y / Enter / R" && desc.contains("Confirm rollback"))
        );
    }

    #[test]
    fn flux_help_lists_reconcile_and_refresh_shortcuts() {
        let section = SECTIONS
            .iter()
            .find(|(title, _)| *title == "Flux")
            .expect("flux section exists")
            .1;

        assert!(section.iter().any(|(key, desc)| {
            *key == "R" && desc.contains("Reconcile selected Flux resource")
        }));
        assert!(
            section
                .iter()
                .any(|(key, desc)| *key == "Ctrl+R" && desc.contains("Refresh data"))
        );
    }

    #[test]
    fn dashboard_help_lists_alert_scroll_shortcuts() {
        let section = SECTIONS
            .iter()
            .find(|(title, _)| *title == "Dashboard & Metrics")
            .expect("dashboard section exists")
            .1;

        assert!(
            section
                .iter()
                .any(|(key, desc)| *key == "PageDown / PageUp" && desc.contains("alerts"))
        );
        assert!(
            section
                .iter()
                .any(|(key, desc)| *key == "; then j/k" && desc.contains("alerts"))
        );
        assert!(
            section
                .iter()
                .any(|(key, desc)| *key == "Ctrl+f / Ctrl+b" && desc.contains("alerts"))
        );
    }
}
