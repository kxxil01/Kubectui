//! Custom resource instance list renderer for Extensions view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Paragraph, Row},
};

use crate::k8s::dtos::CustomResourceInfo;
use crate::ui::{
    TableFrame, format_age, loading_spinner_char, render_table_frame, striped_row_style,
    table_viewport_rows, table_window,
};

const NARROW_CUSTOM_RESOURCE_WIDTH: u16 = 88;

pub struct CustomResourcesPane<'a> {
    pub resources: &'a [CustomResourceInfo],
    pub error: Option<&'a str>,
    pub is_loading: bool,
    pub selected_crd: Option<&'a str>,
    pub selected_idx: usize,
    pub is_focused: bool,
}

fn custom_resource_widths(area: Rect) -> [Constraint; 3] {
    if area.width < NARROW_CUSTOM_RESOURCE_WIDTH {
        [
            Constraint::Min(18),
            Constraint::Min(14),
            Constraint::Length(7),
        ]
    } else {
        [
            Constraint::Min(22),
            Constraint::Min(18),
            Constraint::Length(8),
        ]
    }
}

pub fn render_custom_resources(frame: &mut Frame, area: Rect, pane: CustomResourcesPane<'_>) {
    let theme = crate::ui::components::default_theme();
    if let Some(err) = pane.error {
        frame.render_widget(
            Paragraph::new(ratatui::text::Line::from(vec![
                Span::styled("⊘ ", Style::default().fg(theme.warning)),
                Span::styled(
                    format!("Metrics/instances unavailable: {err}"),
                    theme.inactive_style(),
                ),
            ]))
            .alignment(ratatui::layout::Alignment::Center)
            .block(crate::ui::components::content_block(
                "Custom Resources",
                pane.is_focused,
            )),
            area,
        );
        return;
    }

    if pane.is_loading {
        frame.render_widget(
            Paragraph::new(ratatui::text::Line::from(vec![
                Span::styled(
                    format!("{} ", loading_spinner_char()),
                    Style::default().fg(theme.accent),
                ),
                Span::styled(
                    pane.selected_crd
                        .map(|crd| format!("Loading instances for {crd}..."))
                        .unwrap_or_else(|| "Loading instances...".to_string()),
                    theme.inactive_style(),
                ),
            ]))
            .alignment(ratatui::layout::Alignment::Center)
            .block(crate::ui::components::content_block(
                "Custom Resources",
                pane.is_focused,
            )),
            area,
        );
        return;
    }

    if pane.resources.is_empty() {
        let empty_message = pane
            .selected_crd
            .map(|crd| format!("No instances found for {crd}"))
            .unwrap_or_else(|| "Select a CRD to browse instances".to_string());
        frame.render_widget(
            Paragraph::new(ratatui::text::Line::from(vec![
                Span::styled("○ ", Style::default().fg(theme.fg_dim)),
                Span::styled(empty_message, theme.inactive_style()),
            ]))
            .alignment(ratatui::layout::Alignment::Center)
            .block(crate::ui::components::content_block(
                "Custom Resources",
                pane.is_focused,
            )),
            area,
        );
        return;
    }

    let total = pane.resources.len();
    let selected = pane.selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));
    let rows = pane.resources[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(offset, item)| {
            let idx = window.start + offset;
            Row::new(vec![
                Cell::from(item.name.clone()),
                Cell::from(
                    item.namespace
                        .clone()
                        .unwrap_or_else(|| "<cluster-scope>".to_string()),
                ),
                Cell::from(format_age(item.age)),
            ])
            .style(striped_row_style(idx, &theme))
        })
        .collect();

    let header = Row::new([
        Cell::from(Span::styled("Name", theme.header_style())),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let title = if pane.is_focused {
        format!(
            "Custom Resources ({}) ▸ Enter to view",
            pane.resources.len()
        )
    } else {
        format!("Custom Resources ({})", pane.resources.len())
    };
    let widths = custom_resource_widths(area);
    render_table_frame(
        frame,
        area,
        TableFrame {
            rows,
            header,
            widths: &widths,
            title: &title,
            focused: pane.is_focused,
            window,
            total,
            selected,
        },
        &theme,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn render_custom_resources_windows_selected_row_into_view() {
        let resources = (0..24)
            .map(|idx| CustomResourceInfo {
                name: format!("resource-{idx}"),
                namespace: Some("team-a".to_string()),
                ..CustomResourceInfo::default()
            })
            .collect::<Vec<_>>();
        let backend = TestBackend::new(64, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal
            .draw(|frame| {
                render_custom_resources(
                    frame,
                    frame.area(),
                    CustomResourcesPane {
                        resources: &resources,
                        error: None,
                        is_loading: false,
                        selected_crd: Some("widgets.demo.io"),
                        selected_idx: 18,
                        is_focused: true,
                    },
                );
            })
            .expect("render");

        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }

        assert!(out.contains("resource-18"));
        assert!(!out.contains("resource-0"));
    }

    #[test]
    fn render_custom_resources_shows_loading_for_selected_crd() {
        let backend = TestBackend::new(80, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal
            .draw(|frame| {
                render_custom_resources(
                    frame,
                    frame.area(),
                    CustomResourcesPane {
                        resources: &[],
                        error: None,
                        is_loading: true,
                        selected_crd: Some("widgets.demo.io"),
                        selected_idx: 0,
                        is_focused: true,
                    },
                );
            })
            .expect("render");

        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }

        assert!(out.contains("Loading instances for widgets.demo.io..."));
        assert!(!out.contains("Select a CRD"));
    }

    #[test]
    fn render_custom_resources_shows_empty_for_loaded_selected_crd() {
        let backend = TestBackend::new(80, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal
            .draw(|frame| {
                render_custom_resources(
                    frame,
                    frame.area(),
                    CustomResourcesPane {
                        resources: &[],
                        error: None,
                        is_loading: false,
                        selected_crd: Some("widgets.demo.io"),
                        selected_idx: 0,
                        is_focused: true,
                    },
                );
            })
            .expect("render");

        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }

        assert!(out.contains("No instances found for widgets.demo.io"));
        assert!(!out.contains("Select a CRD"));
    }

    #[test]
    fn custom_resource_widths_switch_to_compact_profile() {
        let widths = custom_resource_widths(Rect::new(0, 0, 80, 20));
        assert_eq!(widths[0], Constraint::Min(18));
        assert_eq!(widths[1], Constraint::Min(14));
        assert_eq!(widths[2], Constraint::Length(7));
    }

    #[test]
    fn custom_resource_widths_keep_wide_profile() {
        let widths = custom_resource_widths(Rect::new(0, 0, 120, 20));
        assert_eq!(widths[0], Constraint::Min(22));
        assert_eq!(widths[1], Constraint::Min(18));
        assert_eq!(widths[2], Constraint::Length(8));
    }
}
