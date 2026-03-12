//! Deployments list rendering.

use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

use ratatui::{
    layout::{Margin, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{
        Cell, HighlightSpacing, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState,
    },
};

use crate::{
    app::{AppView, ResourceRef, WorkloadSortColumn, WorkloadSortState},
    bookmarks::BookmarkEntry,
    columns::ColumnDef,
    state::ClusterSnapshot,
    ui::{
        bookmarked_name_cell,
        components::{active_block, default_block, default_theme},
        filter_cache::{cached_filter_indices_with_variant, data_fingerprint},
        format_age, format_image, format_small_int, loading_or_empty_message,
        responsive_table_widths_vec, sort_header_cell, table_viewport_rows, table_window,
        views::filtering::filtered_deployment_indices,
        workload_sort_suffix,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeploymentDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
    variant: u64,
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
#[allow(clippy::too_many_arguments)]
pub fn render_deployments(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    query: &str,
    sort: Option<WorkloadSortState>,
    visible_columns: &[ColumnDef],
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
        |q| filtered_deployment_indices(&snapshot.deployments, q, sort),
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

    let header_cells: Vec<Cell> = visible_columns
        .iter()
        .map(|col| match col.id {
            "name" => sort_header_cell(col.label, sort, WorkloadSortColumn::Name, &theme, true),
            "age" => sort_header_cell(col.label, sort, WorkloadSortColumn::Age, &theme, false),
            _ => Cell::from(Span::styled(col.label.to_string(), theme.header_style())),
        })
        .collect();
    let header = Row::new(header_cells).height(1).style(theme.header_style());

    let name_style = Style::default().fg(theme.fg);
    let dim_style = Style::default().fg(theme.fg_dim);
    let muted_style = Style::default().fg(theme.muted);
    let derived = cached_deployment_derived(snapshot, query, indices.as_ref(), cache_variant);

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
                Cow::Owned(format_image(deploy.image.as_deref(), 34)),
                deployment_health_from_ready(&deploy.ready),
            )
        };
        let ready_style = health_style(health, &theme);

        let row_style = if idx.is_multiple_of(2) {
            Style::default().bg(theme.bg)
        } else {
            theme.row_alt_style()
        };

        let cells: Vec<Cell> = visible_columns
            .iter()
            .map(|col| match col.id {
                "name" => bookmarked_name_cell(
                    &ResourceRef::Deployment(deploy.name.clone(), deploy.namespace.clone()),
                    bookmarks,
                    deploy.name.as_str(),
                    name_style,
                    &theme,
                ),
                "namespace" => Cell::from(Span::styled(deploy.namespace.as_str(), dim_style)),
                "ready" => Cell::from(Span::styled(deploy.ready.as_str(), ready_style)),
                "updated" => Cell::from(Span::styled(
                    format_small_int(i64::from(deploy.updated_replicas)),
                    dim_style,
                )),
                "available" => Cell::from(Span::styled(
                    format_small_int(i64::from(deploy.available_replicas)),
                    dim_style,
                )),
                "age" => Cell::from(Span::styled(age_text.to_string(), theme.inactive_style())),
                "image" => Cell::from(Span::styled(image_text.to_string(), muted_style)),
                _ => Cell::from(""),
            })
            .collect();
        rows.push(Row::new(cells).style(row_style));
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

    let constraints = crate::columns::visible_constraints(visible_columns);
    let table = Table::new(rows, responsive_table_widths_vec(area.width, &constraints))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentHealth {
    Healthy,
    Degraded,
    Failed,
}

pub fn deployment_health_from_ready(ready: &str) -> DeploymentHealth {
    let Some((ready_count, desired_count)) = ready.split_once('/') else {
        return DeploymentHealth::Degraded;
    };
    let ready = ready_count.trim().parse::<u32>().unwrap_or(0);
    let desired = desired_count.trim().parse::<u32>().unwrap_or(0);
    if desired == 0 {
        DeploymentHealth::Healthy
    } else if ready == 0 {
        DeploymentHealth::Failed
    } else if ready >= desired {
        DeploymentHealth::Healthy
    } else {
        DeploymentHealth::Degraded
    }
}

fn cached_deployment_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    variant: u64,
) -> DeploymentDerivedCacheValue {
    let key = DeploymentDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.deployments, snapshot.snapshot_version),
        variant,
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
                    image: format_image(deploy.image.as_deref(), 34),
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
        let out = format_image(Some(long), 34);
        assert!(out.ends_with("..."));
        assert!(out.len() <= 37);
    }

    /// Verifies missing image renders a dash placeholder.
    #[test]
    fn format_image_empty_placeholder() {
        assert_eq!(format_image(None, 34), "-");
    }
}
