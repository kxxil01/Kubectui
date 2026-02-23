//! Probe Panel component for displaying container health probes.

use ratatui::{
    layout::Rect,
    prelude::{Color, Frame, Line, Span, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::collections::HashSet;

use crate::k8s::probes::{ContainerProbes, ProbeStatus};

/// State for the probe panel widget.
#[derive(Debug, Clone, Default)]
pub struct ProbePanelState {
    pub pod_name: String,
    pub namespace: String,
    pub container_probes: Vec<(String, ContainerProbes)>,
    pub expanded_containers: HashSet<String>,
    pub selected_index: usize,
}

/// Actions that can be performed on the probe panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeAction {
    ToggleExpand,
    SelectNext,
    SelectPrev,
    RefreshProbes,
}

impl ProbePanelState {
    /// Create a new probe panel state.
    pub fn new(
        pod_name: String,
        namespace: String,
        container_probes: Vec<(String, ContainerProbes)>,
    ) -> Self {
        Self {
            pod_name,
            namespace,
            container_probes,
            expanded_containers: HashSet::new(),
            selected_index: 0,
        }
    }

    /// Handle navigation: move to next container.
    pub fn select_next(&mut self) {
        if !self.container_probes.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.container_probes.len();
        }
    }

    /// Handle navigation: move to previous container.
    pub fn select_prev(&mut self) {
        if !self.container_probes.is_empty() {
            self.selected_index = if self.selected_index == 0 {
                self.container_probes.len() - 1
            } else {
                self.selected_index - 1
            };
        }
    }

    /// Toggle expansion for the currently selected container.
    pub fn toggle_expand(&mut self) {
        if let Some((container_name, _)) = self.container_probes.get(self.selected_index) {
            if self.expanded_containers.contains(container_name) {
                self.expanded_containers.remove(container_name);
            } else {
                self.expanded_containers.insert(container_name.clone());
            }
        }
    }

    /// Count healthy containers (those with probes).
    pub fn healthy_count(&self) -> usize {
        self.container_probes.len()
    }

    /// Get status color for a given status.
    pub fn status_color(status: ProbeStatus) -> Color {
        match status {
            ProbeStatus::Success => Color::Green,
            ProbeStatus::Failure => Color::Red,
            ProbeStatus::Pending => Color::Blue,
            ProbeStatus::Error => Color::Yellow,
        }
    }

    /// Get status symbol for a given status.
    pub fn status_symbol(status: ProbeStatus) -> &'static str {
        match status {
            ProbeStatus::Success => "✓",
            ProbeStatus::Failure => "✗",
            ProbeStatus::Pending => "⏳",
            ProbeStatus::Error => "?",
        }
    }
}

/// Render the probe panel widget.
pub fn render_probe_panel(
    frame: &mut Frame,
    area: Rect,
    state: &ProbePanelState,
) {
    let block = Block::default()
        .title("Health Probes")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::White));

    // Build the content
    let mut lines: Vec<Line> = Vec::new();

    if state.container_probes.is_empty() {
        lines.push(Line::from(Span::raw(
            "No probes configured for any containers.",
        )));
    } else {
        for (idx, (container_name, probes)) in state.container_probes.iter().enumerate() {
            let is_selected = idx == state.selected_index;
            let is_expanded = state.expanded_containers.contains(container_name);

            // Container header line with indicator
            let indicator = if is_expanded { "▼" } else { "▶" };
            let selector = if is_selected { "█" } else { " " };

            let probe_count = if probes.liveness.is_some() as usize
                + probes.readiness.is_some() as usize
                == 0
            {
                "no probes".to_string()
            } else {
                let mut count_str = String::new();
                if probes.liveness.is_some() {
                    count_str.push_str("L");
                }
                if probes.readiness.is_some() {
                    count_str.push_str("R");
                }
                count_str
            };

            let container_line_style = if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .bg(Color::DarkGray)
            } else {
                Style::default().fg(Color::White)
            };

            let container_line = Line::from(vec![
                Span::styled(selector, container_line_style),
                Span::styled(indicator, container_line_style),
                Span::styled(format!(" {} ", container_name), container_line_style),
                Span::styled(
                    format!("[{}]", probe_count),
                    Style::default()
                        .fg(Color::DarkGray)
                        .italic(),
                ),
            ]);

            lines.push(container_line);

            // If expanded, show probe details
            if is_expanded {
                if let Some(liveness) = &probes.liveness {
                    let liveness_line = format!(
                        "  ✓ Liveness: {} (delay: {}s, period: {}s, timeout: {}s)",
                        liveness.handler,
                        liveness.initial_delay_seconds,
                        liveness.period_seconds,
                        liveness.timeout_seconds
                    );
                    lines.push(Line::from(Span::styled(
                        liveness_line,
                        Style::default().fg(Color::Green).italic(),
                    )));
                }

                if let Some(readiness) = &probes.readiness {
                    let readiness_line = format!(
                        "  ✓ Readiness: {} (delay: {}s, period: {}s, timeout: {}s)",
                        readiness.handler,
                        readiness.initial_delay_seconds,
                        readiness.period_seconds,
                        readiness.timeout_seconds
                    );
                    lines.push(Line::from(Span::styled(
                        readiness_line,
                        Style::default().fg(Color::Green).italic(),
                    )));
                }
            }
        }

        // Summary line
        lines.push(Line::from(""));
        let summary = format!(
            "{}/{} containers have probes",
            state.healthy_count(),
            state.container_probes.len()
        );
        lines.push(Line::from(Span::styled(
            summary,
            Style::default().fg(Color::Yellow),
        )));

        // Legend
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("✓", Style::default().fg(Color::Green)),
            Span::raw(" success  "),
            Span::styled("✗", Style::default().fg(Color::Red)),
            Span::raw(" failed  "),
            Span::styled("⏳", Style::default().fg(Color::Blue)),
            Span::raw(" pending  "),
            Span::styled("?", Style::default().fg(Color::Yellow)),
            Span::raw(" error"),
        ]));

        // Help text
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "↑/↓ or k/j: navigate  Enter/Space: toggle  r: refresh",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let widget = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(widget, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::probes::{ProbeConfig, ProbeHandler, ProbeType};

    #[test]
    fn test_probe_panel_navigation() {
        let probes = vec![
            (
                "container1".to_string(),
                ContainerProbes::default(),
            ),
            (
                "container2".to_string(),
                ContainerProbes::default(),
            ),
            (
                "container3".to_string(),
                ContainerProbes::default(),
            ),
        ];

        let mut state = ProbePanelState::new("test-pod".to_string(), "default".to_string(), probes);

        assert_eq!(state.selected_index, 0);

        state.select_next();
        assert_eq!(state.selected_index, 1);

        state.select_next();
        assert_eq!(state.selected_index, 2);

        // Wrap around
        state.select_next();
        assert_eq!(state.selected_index, 0);

        state.select_prev();
        assert_eq!(state.selected_index, 2);

        state.select_prev();
        assert_eq!(state.selected_index, 1);
    }

    #[test]
    fn test_probe_panel_toggle_expand() {
        let mut probes = ContainerProbes::default();
        probes.liveness = Some(ProbeConfig {
            probe_type: ProbeType::Liveness,
            handler: ProbeHandler::Tcp { port: 8080 },
            initial_delay_seconds: 0,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
        });

        let container_probes = vec![("container1".to_string(), probes)];

        let mut state = ProbePanelState::new(
            "test-pod".to_string(),
            "default".to_string(),
            container_probes,
        );

        assert!(!state.expanded_containers.contains("container1"));

        state.toggle_expand();
        assert!(state.expanded_containers.contains("container1"));

        state.toggle_expand();
        assert!(!state.expanded_containers.contains("container1"));
    }

    #[test]
    fn test_probe_panel_healthy_count() {
        let probes = vec![
            ("container1".to_string(), ContainerProbes::default()),
            ("container2".to_string(), ContainerProbes::default()),
        ];

        let state = ProbePanelState::new("test-pod".to_string(), "default".to_string(), probes);

        assert_eq!(state.healthy_count(), 2);
    }

    #[test]
    fn test_status_color_mapping() {
        assert_eq!(ProbePanelState::status_color(ProbeStatus::Success), Color::Green);
        assert_eq!(ProbePanelState::status_color(ProbeStatus::Failure), Color::Red);
        assert_eq!(ProbePanelState::status_color(ProbeStatus::Pending), Color::Blue);
        assert_eq!(ProbePanelState::status_color(ProbeStatus::Error), Color::Yellow);
    }

    #[test]
    fn test_status_symbol_mapping() {
        assert_eq!(ProbePanelState::status_symbol(ProbeStatus::Success), "✓");
        assert_eq!(ProbePanelState::status_symbol(ProbeStatus::Failure), "✗");
        assert_eq!(ProbePanelState::status_symbol(ProbeStatus::Pending), "⏳");
        assert_eq!(ProbePanelState::status_symbol(ProbeStatus::Error), "?");
    }

    #[test]
    fn test_multi_container_probes() {
        let liveness_probe = ProbeConfig {
            probe_type: ProbeType::Liveness,
            handler: ProbeHandler::Http {
                path: "/health".to_string(),
                port: 8080,
                scheme: "HTTP".to_string(),
            },
            initial_delay_seconds: 5,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
        };

        let readiness_probe = ProbeConfig {
            probe_type: ProbeType::Readiness,
            handler: ProbeHandler::Tcp { port: 8080 },
            initial_delay_seconds: 2,
            period_seconds: 5,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
        };

        let mut probes1 = ContainerProbes::default();
        probes1.liveness = Some(liveness_probe.clone());
        probes1.readiness = Some(readiness_probe.clone());

        let mut probes2 = ContainerProbes::default();
        probes2.liveness = Some(liveness_probe);

        let container_probes = vec![
            ("app-container".to_string(), probes1),
            ("sidecar-container".to_string(), probes2),
        ];

        let mut state = ProbePanelState::new(
            "test-pod".to_string(),
            "default".to_string(),
            container_probes,
        );

        assert_eq!(state.healthy_count(), 2);

        state.toggle_expand();
        assert!(state.expanded_containers.contains("app-container"));

        state.select_next();
        assert_eq!(state.selected_index, 1);
        assert!(!state.expanded_containers.contains("sidecar-container"));

        state.toggle_expand();
        assert!(state.expanded_containers.contains("sidecar-container"));
    }

    #[test]
    fn test_empty_probes() {
        let state = ProbePanelState::new(
            "test-pod".to_string(),
            "default".to_string(),
            vec![],
        );

        assert_eq!(state.healthy_count(), 0);
        assert_eq!(state.selected_index, 0);
    }
}
