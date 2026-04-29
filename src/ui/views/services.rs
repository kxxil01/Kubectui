//! Services list rendering.

use std::{borrow::Cow, sync::LazyLock};

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Row},
};

use crate::{
    app::{AppView, ResourceRef, WorkloadSortColumn, WorkloadSortState},
    bookmarks::BookmarkEntry,
    state::ClusterSnapshot,
    ui::{
        ResourceTableConfig, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{
            DerivedRowsCache, DerivedRowsCacheKey, DerivedRowsCacheValue, cached_derived_rows,
            cached_filter_indices_with_variant, data_fingerprint,
        },
        format_age, name_cell_with_bookmark, render_resource_table, sort_header_cell,
        striped_row_style, truncate_message,
        views::filtering::filtered_service_indices,
        workload_sort_suffix,
    },
};

const NARROW_SERVICE_WIDTH: u16 = 104;

fn service_widths(area: Rect) -> [Constraint; 6] {
    if area.width < NARROW_SERVICE_WIDTH {
        [
            Constraint::Min(18),
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Length(14),
            Constraint::Min(14),
            Constraint::Length(8),
        ]
    } else {
        [
            Constraint::Length(24),
            Constraint::Length(16),
            Constraint::Length(14),
            Constraint::Length(16),
            Constraint::Min(18),
            Constraint::Length(9),
        ]
    }
}

#[derive(Debug, Clone)]
struct ServiceDerivedCell {
    cluster_ip: String,
    ports: String,
    age: String,
}

type ServiceDerivedCacheValue = DerivedRowsCacheValue<ServiceDerivedCell>;
static SERVICE_DERIVED_CACHE: LazyLock<DerivedRowsCache<ServiceDerivedCell>> =
    LazyLock::new(Default::default);

/// Renders the Services table with stateful selection and scrollbar.
#[allow(clippy::too_many_arguments)]
pub fn render_services(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    query: &str,
    sort: Option<WorkloadSortState>,
    focused: bool,
) {
    let theme = default_theme();
    let query = query.trim();
    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::Services,
        query,
        snapshot.snapshot_version,
        data_fingerprint(&snapshot.services, snapshot.snapshot_version),
        cache_variant,
        |q| filtered_service_indices(&snapshot.services, q, sort),
    );

    let derived = cached_service_derived(snapshot, query, indices.as_ref(), cache_variant);
    let widths = service_widths(area);
    let sort_suffix = workload_sort_suffix(sort);
    render_resource_table(
        frame,
        area,
        &theme,
        ResourceTableConfig {
            snapshot,
            view: AppView::Services,
            label: "Services",
            loading_message: "Loading services...",
            empty_message: "No services found",
            empty_query_message: "No services match the search query",
            query,
            focused,
            filtered_total: indices.len(),
            all_total: snapshot.services.len(),
            selected_idx,
            widths: &widths,
            sort_suffix: &sort_suffix,
        },
        |theme| {
            Row::new([
                sort_header_cell("Name", sort, WorkloadSortColumn::Name, theme, true),
                Cell::from(Span::styled("Namespace", theme.header_style())),
                Cell::from(Span::styled("Type", theme.header_style())),
                Cell::from(Span::styled("ClusterIP", theme.header_style())),
                Cell::from(Span::styled("Ports", theme.header_style())),
                sort_header_cell("Age", sort, WorkloadSortColumn::Age, theme, false),
            ])
            .height(1)
            .style(theme.header_style())
        },
        |window, theme| {
            indices[window.start..window.end]
                .iter()
                .enumerate()
                .map(|(local_idx, &svc_idx)| {
                    let idx = window.start + local_idx;
                    let svc = &snapshot.services[svc_idx];
                    let (cluster_ip, ports, age) = if let Some(cell) = derived.get(idx) {
                        (
                            Cow::Borrowed(cell.cluster_ip.as_str()),
                            Cow::Borrowed(cell.ports.as_str()),
                            Cow::Borrowed(cell.age.as_str()),
                        )
                    } else {
                        (
                            Cow::Owned(
                                svc.cluster_ip.clone().unwrap_or_else(|| "None".to_string()),
                            ),
                            Cow::Owned(format_ports(&svc.ports)),
                            Cow::Owned(format_age(svc.age)),
                        )
                    };
                    let type_style = service_type_style(&svc.type_, theme);

                    Row::new(vec![
                        if bookmarks.is_empty() {
                            name_cell_with_bookmark(
                                false,
                                svc.name.as_str(),
                                Style::default().fg(theme.fg),
                                theme,
                            )
                        } else {
                            bookmarked_name_cell(
                                || ResourceRef::Service(svc.name.clone(), svc.namespace.clone()),
                                bookmarks,
                                svc.name.as_str(),
                                Style::default().fg(theme.fg),
                                theme,
                            )
                        },
                        Cell::from(Span::styled(
                            svc.namespace.as_str(),
                            Style::default().fg(theme.fg_dim),
                        )),
                        Cell::from(Span::styled(svc.type_.as_str(), type_style)),
                        Cell::from(Span::styled(cluster_ip, Style::default().fg(theme.fg_dim))),
                        Cell::from(Span::styled(ports, Style::default().fg(theme.accent2))),
                        Cell::from(Span::styled(age, theme.inactive_style())),
                    ])
                    .style(striped_row_style(idx, theme))
                })
                .collect()
        },
    );
}

fn cached_service_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    variant: u64,
) -> ServiceDerivedCacheValue {
    let key = DerivedRowsCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.services, snapshot.snapshot_version),
        variant,
        freshness_bucket: 0,
    };

    cached_derived_rows(&SERVICE_DERIVED_CACHE, key, || {
        indices
            .iter()
            .map(|&svc_idx| {
                let svc = &snapshot.services[svc_idx];
                ServiceDerivedCell {
                    cluster_ip: svc.cluster_ip.clone().unwrap_or_else(|| "None".to_string()),
                    ports: format_ports(&svc.ports),
                    age: format_age(svc.age),
                }
            })
            .collect()
    })
}

fn service_type_style(type_: &str, theme: &crate::ui::theme::Theme) -> Style {
    if type_.eq_ignore_ascii_case("ClusterIP") {
        Style::default().fg(theme.info)
    } else if type_.eq_ignore_ascii_case("NodePort") {
        Style::default().fg(theme.warning)
    } else if type_.eq_ignore_ascii_case("LoadBalancer") {
        Style::default().fg(theme.success)
    } else if type_.eq_ignore_ascii_case("ExternalName") {
        Style::default().fg(theme.accent2)
    } else {
        Style::default().fg(theme.muted)
    }
}

fn format_ports(ports: &[String]) -> String {
    if ports.is_empty() {
        return "-".to_string();
    }

    let joined = ports.join(", ");
    const MAX_LEN: usize = 28;

    truncate_message(&joined, MAX_LEN).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies empty port list renders with a dash placeholder.
    #[test]
    fn format_ports_empty() {
        assert_eq!(format_ports(&[]), "-");
    }

    /// Verifies short port lists render fully without truncation.
    #[test]
    fn format_ports_short_list() {
        let ports = vec!["80/TCP".to_string(), "443/TCP".to_string()];
        assert_eq!(format_ports(&ports), "80/TCP, 443/TCP");
    }

    /// Verifies long port lists are truncated to the table cell budget.
    #[test]
    fn format_ports_long_list_truncates() {
        let ports = vec![
            "80/TCP".to_string(),
            "443/TCP".to_string(),
            "8080/TCP".to_string(),
            "8443/TCP".to_string(),
            "9090/TCP".to_string(),
        ];

        let out = format_ports(&ports);
        assert!(out.starts_with("80/TCP"));
        assert!(out.ends_with("..."));
        assert_eq!(out.chars().count(), 28);
    }

    /// Verifies a single pathological port cannot overflow the table cell.
    #[test]
    fn format_ports_long_first_port_fits_budget() {
        let ports = vec![
            "123456789012345678901234567890/TCP".to_string(),
            "443/TCP".to_string(),
        ];

        let out = format_ports(&ports);
        assert!(out.ends_with("..."));
        assert_eq!(out.chars().count(), 28);
    }

    /// Verifies service type style helper maps known types.
    #[test]
    fn service_type_style_maps_known_types() {
        use crate::ui::theme::Theme;
        let theme = Theme::dark();
        assert_eq!(service_type_style("ClusterIP", &theme).fg, Some(theme.info));
        assert_eq!(
            service_type_style("NodePort", &theme).fg,
            Some(theme.warning)
        );
        assert_eq!(
            service_type_style("LoadBalancer", &theme).fg,
            Some(theme.success)
        );
        assert_eq!(
            service_type_style("ExternalName", &theme).fg,
            Some(theme.accent2)
        );
    }

    #[test]
    fn service_widths_switch_to_compact_profile() {
        let widths = service_widths(Rect::new(0, 0, 92, 20));
        assert_eq!(widths[0], Constraint::Min(18));
        assert_eq!(widths[1], Constraint::Length(14));
        assert_eq!(widths[4], Constraint::Min(14));
        assert_eq!(widths[5], Constraint::Length(8));
    }

    #[test]
    fn service_widths_keep_wide_profile() {
        let widths = service_widths(Rect::new(0, 0, 132, 20));
        assert_eq!(widths[0], Constraint::Length(24));
        assert_eq!(widths[1], Constraint::Length(16));
        assert_eq!(widths[5], Constraint::Length(9));
    }
}
