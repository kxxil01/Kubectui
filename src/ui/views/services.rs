//! Services list rendering.

use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

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
    app::{AppView, WorkloadSortColumn, WorkloadSortState, filtered_workload_indices},
    state::ClusterSnapshot,
    ui::{
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices_with_variant, data_fingerprint},
        loading_or_empty_message, responsive_table_widths, table_viewport_rows, table_window,
        workload_sort_header, workload_sort_suffix,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServiceDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct ServiceDerivedCell {
    cluster_ip: String,
    ports: String,
    age: String,
}

type ServiceDerivedCacheValue = Arc<Vec<ServiceDerivedCell>>;
static SERVICE_DERIVED_CACHE: LazyLock<
    Mutex<Option<(ServiceDerivedCacheKey, ServiceDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

/// Renders the Services table with stateful selection and scrollbar.
pub fn render_services(
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
        AppView::Services,
        query,
        snapshot.snapshot_version,
        data_fingerprint(&snapshot.services, snapshot.snapshot_version),
        cache_variant,
        |q| {
            filtered_workload_indices(
                &snapshot.services,
                q,
                sort,
                |svc, needle| {
                    contains_ci(&svc.name, needle)
                        || contains_ci(&svc.namespace, needle)
                        || contains_ci(&svc.type_, needle)
                },
                |svc| svc.name.as_str(),
                |svc| svc.namespace.as_str(),
                |svc| svc.age,
            )
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            snapshot,
            AppView::Services,
            query,
            "  Loading services...",
            "  No services found",
            "  No services match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("Services")),
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
        Cell::from(Span::styled("Type", theme.header_style())),
        Cell::from(Span::styled("ClusterIP", theme.header_style())),
        Cell::from(Span::styled("Ports", theme.header_style())),
        Cell::from(Span::styled(age_header, theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let derived = cached_service_derived(snapshot, query, indices.as_ref());
    let rows: Vec<Row> = indices[window.start..window.end]
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
                    Cow::Owned(svc.cluster_ip.clone().unwrap_or_else(|| "None".to_string())),
                    Cow::Owned(format_ports(&svc.ports)),
                    Cow::Owned(format_age(svc.age)),
                )
            };
            let type_style = service_type_style(&svc.type_, &theme);
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", svc.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    svc.namespace.clone(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(svc.type_.clone(), type_style)),
                Cell::from(Span::styled(cluster_ip, Style::default().fg(theme.fg_dim))),
                Cell::from(Span::styled(ports, Style::default().fg(theme.accent2))),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let sort_suffix = workload_sort_suffix(sort);
    let title = format!(" 🔌 Services ({total}){sort_suffix} ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = snapshot.services.len();
        active_block(&format!(
            " 🔌 Services ({total} of {all}) [/{query}]{sort_suffix}"
        ))
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Length(24),
                Constraint::Length(16),
                Constraint::Length(14),
                Constraint::Length(16),
                Constraint::Min(18),
                Constraint::Length(9),
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

fn cached_service_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> ServiceDerivedCacheValue {
    let key = ServiceDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.services, snapshot.snapshot_version),
    };

    if let Ok(cache) = SERVICE_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
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
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = SERVICE_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
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

    if joined.chars().count() <= MAX_LEN {
        return joined;
    }

    let head = ports.first().cloned().unwrap_or_else(|| joined.clone());
    format!("{head}, ...")
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

    /// Verifies long port lists are truncated using head-plus-ellipsis format.
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
        assert!(out.ends_with(", ..."));
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
}
