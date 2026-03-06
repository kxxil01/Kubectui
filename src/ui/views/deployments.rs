//! Deployments list rendering.

use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

use ratatui::{
    layout::{Constraint, Margin, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{
        Cell, HighlightSpacing, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState,
    },
};

use crate::{
    app::{AppView, WorkloadSortColumn, WorkloadSortState, filtered_workload_indices},
    state::{
        ClusterSnapshot,
        filters::{DeploymentHealth, deployment_health_from_ready},
    },
    ui::{
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices_with_variant, data_fingerprint},
        format_small_int, loading_or_empty_message, responsive_table_widths, table_viewport_rows,
        table_window, workload_sort_header, workload_sort_suffix,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeploymentDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct DeploymentDerivedCell {
    age: String,
    image: String,
    health: DeploymentHealth,
}

type DeploymentDerivedCacheValue = Arc<Vec<DeploymentDerivedCell>>;
static DEPLOYMENT_DERIVED_CACHE: LazyLock<
    Mutex<Option<(DeploymentDerivedCacheKey, DeploymentDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

/// Renders the Deployments table with stateful selection and scrollbar.
pub fn render_deployments(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
    sort: Option<WorkloadSortState>,
) {
    let theme = default_theme();
    let query = query.trim();
    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::Deployments,
        query,
        snapshot.snapshot_version,
        data_fingerprint(&snapshot.deployments, snapshot.snapshot_version),
        cache_variant,
        |q| {
            filtered_workload_indices(
                &snapshot.deployments,
                q,
                sort,
                |deploy, needle| {
                    contains_ci(&deploy.name, needle) || contains_ci(&deploy.namespace, needle)
                },
                |deploy| deploy.name.as_str(),
                |deploy| deploy.namespace.as_str(),
                |deploy| deploy.age,
            )
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            snapshot,
            AppView::Deployments,
            query,
            "  Loading deployments...",
            "  No deployments found",
            "  No deployments match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("Deployments")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));
    let name_header = workload_sort_header("Name", sort, WorkloadSortColumn::Name);
    let age_header = workload_sort_header("Age", sort, WorkloadSortColumn::Age);

    let header = Row::new([
        Cell::from(Span::styled(
            format!("  {name_header}"),
            theme.header_style(),
        )),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Ready", theme.header_style())),
        Cell::from(Span::styled("Updated", theme.header_style())),
        Cell::from(Span::styled("Available", theme.header_style())),
        Cell::from(Span::styled(age_header, theme.header_style())),
        Cell::from(Span::styled("Image", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());
    let name_style = Style::default().fg(theme.fg);
    let dim_style = Style::default().fg(theme.fg_dim);
    let muted_style = Style::default().fg(theme.muted);
    let derived = cached_deployment_derived(snapshot, query, indices.as_ref());

    let mut rows: Vec<Row> = Vec::with_capacity(window.end.saturating_sub(window.start));
    for (local_idx, &deploy_idx) in indices[window.start..window.end].iter().enumerate() {
        let idx = window.start + local_idx;
        let deploy = &snapshot.deployments[deploy_idx];
        let (age_text, image_text, health) = if let Some(cell) = derived.get(idx) {
            (
                Cow::Borrowed(cell.age.as_str()),
                Cow::Borrowed(cell.image.as_str()),
                cell.health,
            )
        } else {
            (
                Cow::Owned(format_age(deploy.age)),
                Cow::Owned(format_image(deploy.image.as_deref())),
                deployment_health_from_ready(&deploy.ready),
            )
        };
        let ready_style = health_style(health, &theme);

        let row_style = if idx.is_multiple_of(2) {
            Style::default().bg(theme.bg)
        } else {
            theme.row_alt_style()
        };

        rows.push(
            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled("  ", name_style),
                    Span::styled(deploy.name.as_str(), name_style),
                ])),
                Cell::from(Span::styled(deploy.namespace.as_str(), dim_style)),
                Cell::from(Span::styled(deploy.ready.as_str(), ready_style)),
                Cell::from(Span::styled(
                    format_small_int(i64::from(deploy.updated_replicas)),
                    dim_style,
                )),
                Cell::from(Span::styled(
                    format_small_int(i64::from(deploy.available_replicas)),
                    dim_style,
                )),
                Cell::from(Span::styled(age_text, theme.inactive_style())),
                Cell::from(Span::styled(image_text, muted_style)),
            ])
            .style(row_style),
        );
    }

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let sort_suffix = workload_sort_suffix(sort);
    let title = format!(" 🚀 Deployments ({total}){sort_suffix} ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = snapshot.deployments.len();
        active_block(&format!(
            " 🚀 Deployments ({total} of {all}) [/{query}]{sort_suffix}"
        ))
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Length(24),
                Constraint::Length(16),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Length(11),
                Constraint::Length(9),
                Constraint::Min(20),
            ],
        ),
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

fn cached_deployment_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> DeploymentDerivedCacheValue {
    let key = DeploymentDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.deployments, snapshot.snapshot_version),
    };

    if let Ok(cache) = DEPLOYMENT_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&deploy_idx| {
                let deploy = &snapshot.deployments[deploy_idx];
                DeploymentDerivedCell {
                    age: format_age(deploy.age),
                    image: format_image(deploy.image.as_deref()),
                    health: deployment_health_from_ready(&deploy.ready),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = DEPLOYMENT_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

fn format_image(image: Option<&str>) -> String {
    let Some(image) = image else {
        return "-".to_string();
    };

    const MAX_LEN: usize = 34;
    if image.chars().count() <= MAX_LEN {
        image.to_string()
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
