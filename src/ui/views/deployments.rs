//! Deployments list rendering.

use std::{borrow::Cow, sync::LazyLock};

use ratatui::{
    layout::Rect,
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Row},
};

use crate::{
    app::{AppView, ResourceRef, WorkloadSortColumn, WorkloadSortState},
    bookmarks::BookmarkEntry,
    columns::ColumnDef,
    state::ClusterSnapshot,
    ui::{
        ResourceTableConfig, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{
            DerivedRowsCache, DerivedRowsCacheKey, DerivedRowsCacheValue, cached_derived_rows,
            cached_filter_indices_with_variant, data_fingerprint,
        },
        format_age, format_image, format_small_int, name_cell_with_bookmark, render_resource_table,
        sort_header_cell, striped_row_style,
        views::filtering::filtered_deployment_indices,
        workload_sort_suffix,
    },
};

#[derive(Debug, Clone)]
struct DeploymentDerivedCell {
    age: String,
    image: String,
    health: DeploymentHealth,
}

type DeploymentDerivedCacheValue = DerivedRowsCacheValue<DeploymentDerivedCell>;
static DEPLOYMENT_DERIVED_CACHE: LazyLock<DerivedRowsCache<DeploymentDerivedCell>> =
    LazyLock::new(Default::default);

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
    focused: bool,
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

    let name_style = Style::default().fg(theme.fg);
    let dim_style = Style::default().fg(theme.fg_dim);
    let muted_style = Style::default().fg(theme.muted);
    let derived = cached_deployment_derived(snapshot, query, indices.as_ref(), cache_variant);
    let widths = crate::columns::visible_constraints(visible_columns);
    let sort_suffix = workload_sort_suffix(sort);

    render_resource_table(
        frame,
        area,
        &theme,
        ResourceTableConfig {
            snapshot,
            view: AppView::Deployments,
            label: "Deployments",
            loading_message: "Loading deployments...",
            empty_message: "No deployments found",
            empty_query_message: "No deployments match the search query",
            query,
            focused,
            filtered_total: indices.len(),
            all_total: snapshot.deployments.len(),
            selected_idx,
            widths: &widths,
            sort_suffix: &sort_suffix,
        },
        |theme| {
            let header_cells: Vec<Cell> = visible_columns
                .iter()
                .map(|col| match col.id {
                    "name" => {
                        sort_header_cell(col.label, sort, WorkloadSortColumn::Name, theme, true)
                    }
                    "age" => {
                        sort_header_cell(col.label, sort, WorkloadSortColumn::Age, theme, false)
                    }
                    _ => Cell::from(Span::styled(col.label.to_string(), theme.header_style())),
                })
                .collect();
            Row::new(header_cells).height(1).style(theme.header_style())
        },
        |window, theme| {
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
                let ready_style = health_style(health, theme);
                let cells: Vec<Cell> = visible_columns
                    .iter()
                    .map(|col| match col.id {
                        "name" => {
                            if bookmarks.is_empty() {
                                name_cell_with_bookmark(
                                    false,
                                    deploy.name.as_str(),
                                    name_style,
                                    theme,
                                )
                            } else {
                                bookmarked_name_cell(
                                    || {
                                        ResourceRef::Deployment(
                                            deploy.name.clone(),
                                            deploy.namespace.clone(),
                                        )
                                    },
                                    bookmarks,
                                    deploy.name.as_str(),
                                    name_style,
                                    theme,
                                )
                            }
                        }
                        "namespace" => {
                            Cell::from(Span::styled(deploy.namespace.as_str(), dim_style))
                        }
                        "ready" => Cell::from(Span::styled(deploy.ready.as_str(), ready_style)),
                        "updated" => Cell::from(Span::styled(
                            format_small_int(i64::from(deploy.updated_replicas)),
                            dim_style,
                        )),
                        "available" => Cell::from(Span::styled(
                            format_small_int(i64::from(deploy.available_replicas)),
                            dim_style,
                        )),
                        "age" => {
                            Cell::from(Span::styled(age_text.to_string(), theme.inactive_style()))
                        }
                        "image" => Cell::from(Span::styled(image_text.to_string(), muted_style)),
                        _ => Cell::from(""),
                    })
                    .collect();
                rows.push(Row::new(cells).style(striped_row_style(idx, theme)));
            }
            rows
        },
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
    let key = DerivedRowsCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.deployments, snapshot.snapshot_version),
        variant,
        freshness_bucket: 0,
    };

    cached_derived_rows(&DEPLOYMENT_DERIVED_CACHE, key, || {
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
            .collect()
    })
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
