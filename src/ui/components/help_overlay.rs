//! Keybinding help overlay displayed with `?`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use crate::ui::components::default_theme;
use crate::{app::DetailViewState, policy::DetailAction};

#[derive(Debug, Clone, Default)]
pub struct HelpOverlay {
    is_open: bool,
    scroll: usize,
}

const DETAIL_BASE_BINDINGS: &[(&str, &str)] = &[
    ("y", "View YAML"),
    ("o", "View decoded Secret data"),
    ("B", "Toggle bookmark"),
    ("v", "View timeline"),
    ("l", "View logs"),
    ("x", "Exec into pod"),
    ("f", "Port forward"),
    ("s", "Scale replicas"),
    ("p", "Probe panel"),
    ("R", "Restart rollout"),
    ("e", "Edit YAML"),
    ("d", "Delete resource"),
    ("F", "Force delete (in confirm dialog)"),
    ("T", "Trigger CronJob"),
    ("S", "Pause/resume CronJob"),
    ("N", "View network policy analysis"),
    ("w", "View relations"),
];

const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Global",
        &[
            ("?", "Toggle this help"),
            ("q", "Quit (with confirmation)"),
            ("Esc", "Back / close overlay"),
            ("Tab / Shift+Tab", "Next / previous view"),
            ("j / k / \u{2193} / \u{2191}", "Navigate list"),
            ("Enter", "Open detail / activate"),
            ("/", "Search / filter"),
            ("~", "Namespace picker"),
            ("c", "Context picker"),
            (
                ":",
                "Action palette (views, actions, workspaces, banks, templates, AI, columns)",
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
            ("[ / ]", "Previous / next workbench tab"),
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
        "Workbench (focused)",
        &[
            ("z", "Maximize / restore"),
            ("j / k", "Scroll down / up"),
            ("g / G", "Jump to top / bottom"),
            ("PageDown / PageUp", "Scroll by page"),
            ("Esc", "Un-maximize or blur"),
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
            ("W", "Save workspace jump with current project filter"),
        ],
    ),
    (
        "Dashboard & Metrics",
        &[
            ("", "5 gauges: Nodes, Pods, Workload, CPU, Mem"),
            ("", "Overcommit & governance panel"),
            ("", "Top-5 CPU/memory pod consumers"),
            ("", "Namespace utilization with %CPU/R %MEM/R"),
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
            ("[ / ]", "Previous / next saved preset"),
            ("M", "Save current log preset"),
            ("Enter / Esc", "Apply / cancel log search"),
            ("Ctrl+U", "Clear log search input"),
            ("n / N", "Next / previous match"),
            ("y", "Copy log content"),
            ("S", "Save logs to file"),
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
            ("[ / ]", "Previous / next saved preset"),
            ("M", "Save current log preset"),
            ("Enter / Esc", "Apply / cancel text filter"),
            ("Ctrl+U", "Clear text filter input"),
            ("y", "Copy log content"),
            ("S", "Save logs to file"),
        ],
    ),
];

impl HelpOverlay {
    pub fn open(&mut self) {
        self.is_open = true;
        self.scroll = 0;
    }

    pub fn close(&mut self) {
        self.is_open = false;
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

    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
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

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);

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

        let visible_height = sections[0].height as usize;
        let max_scroll = lines.len().saturating_sub(visible_height);
        let scroll = self.scroll.min(max_scroll);
        let end = (scroll + visible_height).min(lines.len());
        let visible = if scroll < end {
            lines[scroll..end].to_vec()
        } else {
            vec![]
        };

        frame.render_widget(
            Paragraph::new(visible).wrap(Wrap { trim: false }),
            sections[0],
        );

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " [?/Esc] close  [j/k] scroll ",
                Style::default().fg(theme.fg_dim),
            ))),
            sections[1],
        );
    }
}

fn detail_bindings(detail: Option<&DetailViewState>) -> Vec<(&'static str, &'static str)> {
    let mut bindings = Vec::with_capacity(DETAIL_BASE_BINDINGS.len() + 4);
    if detail.is_some_and(|detail| {
        detail.supports_action(DetailAction::ViewConfigDrift)
            && !detail.supports_action(DetailAction::Drain)
    }) {
        bindings.push(("D", "View config drift (live vs last-applied)"));
    } else if detail.is_some_and(|detail| detail.supports_action(DetailAction::Drain)) {
        bindings.push(("D", "Drain node (with confirmation)"));
    }
    if detail.is_some_and(|detail| detail.supports_action(DetailAction::ViewHelmHistory)) {
        bindings.push(("h", "View Helm revision history / rollback"));
    }
    if detail.is_some_and(|detail| detail.supports_action(DetailAction::ViewRollout)) {
        bindings.push(("O", "View rollout control center"));
    }
    if detail.is_some_and(|detail| detail.supports_action(DetailAction::NodeDebugShell)) {
        bindings.push(("g", "Launch node debug shell"));
    } else if detail.is_some_and(|detail| detail.supports_action(DetailAction::DebugContainer)) {
        bindings.push(("g", "Launch debug container"));
    }
    if detail.is_some_and(|detail| detail.supports_action(DetailAction::CheckNetworkConnectivity)) {
        bindings.push(("C", "Check pod reachability (policy intent)"));
    }
    if detail.is_some_and(|detail| detail.supports_action(DetailAction::ViewTrafficDebug)) {
        bindings.push(("t", "Open traffic debug (service / ingress / DNS path)"));
    }
    bindings.extend_from_slice(DETAIL_BASE_BINDINGS);
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
    fn total_lines_is_nonzero() {
        assert!(HelpOverlay::total_lines() > 20);
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
}
