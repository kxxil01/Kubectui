//! CRD picker renderer for Extensions view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Paragraph, Row},
};

use crate::k8s::dtos::CustomResourceDefinitionInfo;
use crate::ui::{
    TableFrame, components::default_theme, contains_ci, render_table_frame, resource_table_title,
    striped_row_style, table_viewport_rows, table_window,
};

pub fn filtered_crd_indices(crds: &[CustomResourceDefinitionInfo], query: &str) -> Vec<usize> {
    let query = query.trim();

    crds.iter()
        .enumerate()
        .filter_map(|(idx, crd)| {
            (query.is_empty()
                || contains_ci(&crd.name, query)
                || contains_ci(&crd.kind, query)
                || contains_ci(&crd.group, query))
            .then_some(idx)
        })
        .collect()
}

pub fn selected_crd<'a>(
    crds: &'a [CustomResourceDefinitionInfo],
    query: &str,
    selected_idx: usize,
) -> Option<&'a CustomResourceDefinitionInfo> {
    let filtered = filtered_crd_indices(crds, query);
    filtered
        .get(selected_idx.min(filtered.len().saturating_sub(1)))
        .and_then(|&idx| crds.get(idx))
}

pub fn render_crd_picker(
    frame: &mut Frame,
    area: Rect,
    crds: &[CustomResourceDefinitionInfo],
    is_loading: bool,
    selected_idx: usize,
    query: &str,
    is_focused: bool,
) {
    let theme = default_theme();
    let filtered = filtered_crd_indices(crds, query);
    let query_trimmed = query.trim();

    if filtered.is_empty() {
        let (icon, icon_color, msg) = if is_loading {
            ("⟳ ", theme.accent, "Loading CRDs...")
        } else if query_trimmed.is_empty() {
            ("○ ", theme.fg_dim, "No CRDs found")
        } else {
            ("⊘ ", theme.warning, "No CRDs match search")
        };
        frame.render_widget(
            Paragraph::new(ratatui::text::Line::from(vec![
                Span::styled(icon, Style::default().fg(icon_color)),
                Span::styled(msg, theme.inactive_style()),
            ]))
            .alignment(ratatui::layout::Alignment::Center)
            .block(crate::ui::components::content_block("CRDs", is_focused)),
            area,
        );
        return;
    }

    let total = filtered.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));
    let rows = filtered[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(offset, &crd_idx)| {
            let idx = window.start + offset;
            let crd = &crds[crd_idx];
            Row::new(vec![
                Cell::from(crd.kind.clone()),
                Cell::from(crd.group.clone()),
                Cell::from(crd.scope.clone()),
                Cell::from(crd.instances.to_string()),
            ])
            .style(striped_row_style(idx, &theme))
        })
        .collect();

    let header = Row::new([
        Cell::from(Span::styled("Kind", theme.header_style())),
        Cell::from(Span::styled("Group", theme.header_style())),
        Cell::from(Span::styled("Scope", theme.header_style())),
        Cell::from(Span::styled("Instances", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let title = resource_table_title(" ", "CRDs", total, crds.len(), query_trimmed, "");
    let widths = [
        Constraint::Length(22),
        Constraint::Length(24),
        Constraint::Length(12),
        Constraint::Length(10),
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

    fn crd(name: &str, kind: &str, group: &str) -> CustomResourceDefinitionInfo {
        CustomResourceDefinitionInfo {
            name: name.to_string(),
            kind: kind.to_string(),
            group: group.to_string(),
            version: "v1".to_string(),
            plural: format!("{kind}s").to_ascii_lowercase(),
            scope: "Namespaced".to_string(),
            instances: 0,
        }
    }

    #[test]
    fn selected_crd_uses_filtered_indices() {
        let crds = vec![
            crd("widgets.demo.io", "Widget", "demo.io"),
            crd("gadgets.demo.io", "Gadget", "demo.io"),
        ];

        let selected = selected_crd(&crds, "gadget", 0).expect("filtered selection");
        assert_eq!(selected.name, "gadgets.demo.io");
    }

    #[test]
    fn render_crd_picker_windows_selected_row_into_view() {
        let crds = (0..24)
            .map(|idx| {
                crd(
                    &format!("kind-{idx}.demo.io"),
                    &format!("Kind{idx}"),
                    "demo.io",
                )
            })
            .collect::<Vec<_>>();
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal
            .draw(|frame| {
                render_crd_picker(frame, frame.area(), &crds, false, 18, "", true);
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

        assert!(out.contains("Kind18"));
        assert!(!out.contains("Kind0"));
    }
}
