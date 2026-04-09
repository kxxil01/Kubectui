//! Custom resource instance list renderer for Extensions view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Paragraph, Row},
};

use crate::k8s::dtos::CustomResourceInfo;
use crate::ui::{
    TableFrame, format_age, render_table_frame, striped_row_style, table_viewport_rows,
    table_window,
};

pub fn render_custom_resources(
    frame: &mut Frame,
    area: Rect,
    resources: &[CustomResourceInfo],
    error: Option<&str>,
    selected_idx: usize,
    is_focused: bool,
) {
    let theme = crate::ui::components::default_theme();
    if let Some(err) = error {
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
                is_focused,
            )),
            area,
        );
        return;
    }

    if resources.is_empty() {
        frame.render_widget(
            Paragraph::new(ratatui::text::Line::from(vec![
                Span::styled("○ ", Style::default().fg(theme.fg_dim)),
                Span::styled("Select a CRD to browse instances", theme.inactive_style()),
            ]))
            .alignment(ratatui::layout::Alignment::Center)
            .block(crate::ui::components::content_block(
                "Custom Resources",
                is_focused,
            )),
            area,
        );
        return;
    }

    let total = resources.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));
    let rows = resources[window.start..window.end]
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

    let title = if is_focused {
        format!("Custom Resources ({}) ▸ Enter to view", resources.len())
    } else {
        format!("Custom Resources ({})", resources.len())
    };
    let widths = [
        Constraint::Min(22),
        Constraint::Min(18),
        Constraint::Length(8),
    ];
    render_table_frame(
        frame,
        area,
        TableFrame {
            rows,
            header,
            widths: &widths,
            title: &title,
            focused: is_focused,
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
                render_custom_resources(frame, frame.area(), &resources, None, 18, true);
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
}
