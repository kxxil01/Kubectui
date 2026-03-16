//! CRD picker renderer for Extensions view.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Color, Frame, Style},
    text::Span,
    widgets::{Cell, Paragraph, Row, Table},
};

use crate::k8s::dtos::CustomResourceDefinitionInfo;
use crate::ui::{contains_ci, responsive_table_widths};

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
    let filtered = filtered_crd_indices(crds, query);
    let query_trimmed = query.trim();

    if filtered.is_empty() {
        let theme = crate::ui::components::default_theme();
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

    let clamped_idx = selected_idx.min(filtered.len().saturating_sub(1));
    let rows = filtered.iter().enumerate().map(|(idx, &crd_idx)| {
        let crd = &crds[crd_idx];
        let style = if is_focused && idx == clamped_idx {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        Row::new(vec![
            Cell::from(crd.kind.clone()),
            Cell::from(crd.group.clone()),
            Cell::from(crd.scope.clone()),
            Cell::from(crd.instances.to_string()),
        ])
        .style(style)
    });

    let header =
        Row::new(["Kind", "Group", "Scope", "Instances"]).style(Style::default().fg(Color::Cyan));

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Length(22),
                Constraint::Length(24),
                Constraint::Length(12),
                Constraint::Length(10),
            ],
        ),
    )
    .header(header)
    .block(crate::ui::components::default_block("CRDs"));

    frame.render_widget(table, area);
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
