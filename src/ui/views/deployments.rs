//! Deployments list rendering.

use ratatui::{
    layout::{Constraint, Margin, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{
        Cell, HighlightSpacing, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState,
    },
};

use crate::{
    app::AppView,
    state::{
        ClusterSnapshot,
        filters::{DeploymentHealth, deployment_health_from_ready},
    },
    ui::{
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices, data_fingerprint},
        format_small_int,
    },
};

/// Renders the Deployments table with stateful selection and scrollbar.
pub fn render_deployments(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let theme = default_theme();
    let query = query.trim();
    let indices = cached_filter_indices(
        AppView::Deployments,
        query,
        snapshot.snapshot_version,
        data_fingerprint(&snapshot.deployments),
        |q| {
            if q.is_empty() {
                return (0..snapshot.deployments.len()).collect();
            }
            snapshot
                .deployments
                .iter()
                .enumerate()
                .filter_map(|(idx, deploy)| contains_ci(&deploy.name, q).then_some(idx))
                .collect()
        },
    );

    if indices.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "  No deployments found",
                theme.inactive_style(),
            ))
            .block(default_block("Deployments")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Ready", theme.header_style())),
        Cell::from(Span::styled("Updated", theme.header_style())),
        Cell::from(Span::styled("Available", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
        Cell::from(Span::styled("Image", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());
    let name_style = Style::default().fg(theme.fg);
    let dim_style = Style::default().fg(theme.fg_dim);
    let muted_style = Style::default().fg(theme.muted);

    let mut rows: Vec<Row> = Vec::with_capacity(total);
    for (idx, &deploy_idx) in indices.iter().enumerate() {
        let deploy = &snapshot.deployments[deploy_idx];
        let health = deployment_health_from_ready(&deploy.ready);
        let ready_style = health_style(health, &theme);

        let row_style = if idx % 2 == 0 {
            Style::default().bg(theme.bg)
        } else {
            theme.row_alt_style()
        };

        rows.push(
            Row::new(vec![
                Cell::from(Span::styled(format!("  {}", deploy.name), name_style)),
                Cell::from(Span::styled(deploy.namespace.as_str(), dim_style)),
                Cell::from(Span::styled(deploy.ready.as_str(), ready_style)),
                Cell::from(Span::styled(
                    format_small_int(i64::from(deploy.updated)),
                    dim_style,
                )),
                Cell::from(Span::styled(
                    format_small_int(i64::from(deploy.available)),
                    dim_style,
                )),
                Cell::from(Span::styled(format_age(deploy.age), theme.inactive_style())),
                Cell::from(Span::styled(
                    format_image(deploy.image.as_deref()),
                    muted_style,
                )),
            ])
            .style(row_style),
        );
    }

    let mut table_state = TableState::default().with_selected(Some(selected));

    let title = format!(" 🚀 Deployments ({total}) ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = snapshot.deployments.len();
        active_block(&format!(" 🚀 Deployments ({total} of {all}) [/{query}]"))
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(24),
            Constraint::Length(16),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(11),
            Constraint::Length(9),
            Constraint::Min(20),
        ],
    )
    .header(header)
    .block(block)
    .row_highlight_style(theme.selection_style())
    .highlight_symbol(theme.highlight_symbol())
    .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(table, area, &mut table_state);

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"))
        .track_symbol(Some("│"))
        .thumb_symbol("█");

    let mut scrollbar_state = ScrollbarState::new(total).position(selected);
    frame.render_stateful_widget(
        scrollbar,
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}

fn health_style(health: DeploymentHealth, theme: &crate::ui::theme::Theme) -> Style {
    match health {
        DeploymentHealth::Healthy => theme.badge_success_style(),
        DeploymentHealth::Degraded => theme.badge_warning_style(),
        DeploymentHealth::Failed => theme.badge_error_style(),
    }
}

fn format_image(image: Option<&str>) -> String {
    let Some(image) = image else {
        return "-".to_string();
    };

    const MAX_LEN: usize = 34;
    if image.len() <= MAX_LEN {
        image.to_string()
    } else if image.is_ascii() {
        format!("{}...", &image[..MAX_LEN.saturating_sub(3)])
    } else {
        format!(
            "{}...",
            image
                .chars()
                .take(MAX_LEN.saturating_sub(3))
                .collect::<String>()
        )
    }
}

fn format_age(age: Option<std::time::Duration>) -> String {
    let Some(age) = age else {
        return "-".to_string();
    };

    let secs = age.as_secs();
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;

    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Theme;

    /// Verifies health style colors match deployment health state.
    #[test]
    fn health_style_mapping() {
        let theme = Theme::dark();
        assert_eq!(
            health_style(DeploymentHealth::Healthy, &theme).fg,
            Some(theme.success)
        );
        assert_eq!(
            health_style(DeploymentHealth::Degraded, &theme).fg,
            Some(theme.warning)
        );
        assert_eq!(
            health_style(DeploymentHealth::Failed, &theme).fg,
            Some(theme.error)
        );
    }

    /// Verifies image values are truncated when exceeding render width.
    #[test]
    fn format_image_truncates_long_strings() {
        let long = "registry.io/team/service:very-long-tag-1234567890";
        let out = format_image(Some(long));
        assert!(out.ends_with("..."));
        assert!(out.len() <= 37);
    }

    /// Verifies missing image renders a dash placeholder.
    #[test]
    fn format_image_empty_placeholder() {
        assert_eq!(format_image(None), "-");
    }
}
