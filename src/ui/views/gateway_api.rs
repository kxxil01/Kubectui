//! Gateway API list renderers.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Row},
};

use crate::{
    app::{AppView, ResourceRef},
    bookmarks::BookmarkEntry,
    k8s::{
        dtos::{GatewayClassInfo, GatewayInfo, GrpcRouteInfo, HttpRouteInfo, ReferenceGrantInfo},
        gateway_api::{
            GATEWAY_CLASS_SPEC, GATEWAY_SPEC, GRPC_ROUTE_SPEC, HTTP_ROUTE_SPEC,
            REFERENCE_GRANT_SPEC,
        },
    },
    state::ClusterSnapshot,
    ui::{
        ResourceTableConfig, bookmarked_name_cell,
        components::default_theme,
        render_resource_table, striped_row_style,
        views::filtering::{
            filtered_gateway_class_indices, filtered_gateway_indices, filtered_grpc_route_indices,
            filtered_http_route_indices, filtered_reference_grant_indices,
        },
    },
};

const NARROW_GATEWAY_ROUTE_WIDTH: u16 = 96;
const NARROW_GATEWAY_CLASS_WIDTH: u16 = 96;
const NARROW_GATEWAY_WIDTH: u16 = 104;
const NARROW_REFERENCE_GRANT_WIDTH: u16 = 96;

fn gateway_route_widths(area: Rect) -> [Constraint; 6] {
    if area.width < NARROW_GATEWAY_ROUTE_WIDTH {
        [
            Constraint::Min(18),
            Constraint::Length(14),
            Constraint::Min(22),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
        ]
    } else {
        [
            Constraint::Percentage(20),
            Constraint::Percentage(16),
            Constraint::Percentage(28),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
        ]
    }
}

fn gateway_class_widths(area: Rect) -> [Constraint; 4] {
    if area.width < NARROW_GATEWAY_CLASS_WIDTH {
        [
            Constraint::Min(18),
            Constraint::Min(24),
            Constraint::Length(8),
            Constraint::Length(8),
        ]
    } else {
        [
            Constraint::Percentage(26),
            Constraint::Percentage(48),
            Constraint::Percentage(12),
            Constraint::Percentage(14),
        ]
    }
}

fn gateway_widths(area: Rect) -> [Constraint; 6] {
    if area.width < NARROW_GATEWAY_WIDTH {
        [
            Constraint::Min(18),
            Constraint::Length(14),
            Constraint::Min(14),
            Constraint::Min(16),
            Constraint::Length(8),
            Constraint::Length(8),
        ]
    } else {
        [
            Constraint::Percentage(20),
            Constraint::Percentage(16),
            Constraint::Percentage(18),
            Constraint::Percentage(22),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
        ]
    }
}

fn reference_grant_widths(area: Rect) -> [Constraint; 5] {
    if area.width < NARROW_REFERENCE_GRANT_WIDTH {
        [
            Constraint::Min(20),
            Constraint::Length(14),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(7),
        ]
    } else {
        [
            Constraint::Percentage(22),
            Constraint::Percentage(18),
            Constraint::Percentage(18),
            Constraint::Percentage(26),
            Constraint::Percentage(16),
        ]
    }
}

pub fn render_gateway_classes(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    search: &str,
    focused: bool,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = filtered_gateway_class_indices(&snapshot.gateway_classes, query);
    let widths = gateway_class_widths(area);
    render_resource_table(
        frame,
        area,
        &theme,
        ResourceTableConfig {
            snapshot,
            view: AppView::GatewayClasses,
            label: "GatewayClasses",
            loading_message: "Loading gateway classes...",
            empty_message: "No Gateway API classes found",
            empty_query_message: "No gateway classes match the search query",
            query,
            focused,
            filtered_total: indices.len(),
            all_total: snapshot.gateway_classes.len(),
            selected_idx,
            widths: &widths,
            sort_suffix: "",
        },
        |theme| {
            Row::new([
                Cell::from(Span::styled("  NAME", theme.header_style())),
                Cell::from(Span::styled("CONTROLLER", theme.header_style())),
                Cell::from(Span::styled("ACCEPTED", theme.header_style())),
                Cell::from(Span::styled("API", theme.header_style())),
            ])
            .style(theme.header_style())
            .height(1)
        },
        |window, theme| {
            indices[window.start..window.end]
                .iter()
                .enumerate()
                .map(|(local_idx, &resource_idx)| {
                    let idx = window.start + local_idx;
                    let class = &snapshot.gateway_classes[resource_idx];
                    Row::new(vec![
                        bookmarked_name_cell(
                            || gateway_class_ref(class),
                            bookmarks,
                            class.name.as_str(),
                            Style::default().fg(theme.fg),
                            theme,
                        ),
                        Cell::from(Span::styled(
                            class.controller_name.as_str(),
                            Style::default().fg(theme.accent2),
                        )),
                        Cell::from(Span::styled(
                            bool_label(class.accepted, "yes", "no", "unknown"),
                            accepted_style(class.accepted, theme),
                        )),
                        Cell::from(Span::styled(
                            class.version.as_str(),
                            Style::default().fg(theme.fg_dim),
                        )),
                    ])
                    .style(striped_row_style(idx, theme))
                })
                .collect()
        },
    );
}

pub fn render_gateways(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    search: &str,
    focused: bool,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = filtered_gateway_indices(&snapshot.gateways, query);
    let widths = gateway_widths(area);
    render_resource_table(
        frame,
        area,
        &theme,
        ResourceTableConfig {
            snapshot,
            view: AppView::Gateways,
            label: "Gateways",
            loading_message: "Loading gateways...",
            empty_message: "No gateways found",
            empty_query_message: "No gateways match the search query",
            query,
            focused,
            filtered_total: indices.len(),
            all_total: snapshot.gateways.len(),
            selected_idx,
            widths: &widths,
            sort_suffix: "",
        },
        |theme| {
            Row::new([
                Cell::from(Span::styled("  NAME", theme.header_style())),
                Cell::from(Span::styled("NAMESPACE", theme.header_style())),
                Cell::from(Span::styled("CLASS", theme.header_style())),
                Cell::from(Span::styled("ADDRESSES", theme.header_style())),
                Cell::from(Span::styled("LISTENERS", theme.header_style())),
                Cell::from(Span::styled("API", theme.header_style())),
            ])
            .style(theme.header_style())
            .height(1)
        },
        |window, theme| {
            indices[window.start..window.end]
                .iter()
                .enumerate()
                .map(|(local_idx, &resource_idx)| {
                    let idx = window.start + local_idx;
                    let gateway = &snapshot.gateways[resource_idx];
                    Row::new(vec![
                        bookmarked_name_cell(
                            || gateway_ref(gateway),
                            bookmarks,
                            gateway.name.as_str(),
                            Style::default().fg(theme.fg),
                            theme,
                        ),
                        Cell::from(Span::styled(
                            gateway.namespace.as_str(),
                            Style::default().fg(theme.fg_dim),
                        )),
                        Cell::from(Span::styled(
                            gateway.gateway_class_name.as_str(),
                            Style::default().fg(theme.info),
                        )),
                        Cell::from(Span::styled(
                            joined_or_dash(&gateway.addresses),
                            Style::default().fg(theme.accent2),
                        )),
                        Cell::from(Span::styled(
                            gateway.listeners.len().to_string(),
                            Style::default().fg(theme.success),
                        )),
                        Cell::from(Span::styled(
                            gateway.version.as_str(),
                            Style::default().fg(theme.fg_dim),
                        )),
                    ])
                    .style(striped_row_style(idx, theme))
                })
                .collect()
        },
    );
}

pub fn render_http_routes(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    search: &str,
    focused: bool,
) {
    render_route_table(
        frame,
        area,
        snapshot,
        bookmarks,
        selected_idx,
        search,
        focused,
        AppView::HttpRoutes,
        "HTTPRoutes",
        &snapshot.http_routes,
        |query| filtered_http_route_indices(&snapshot.http_routes, query),
        http_route_ref,
        |route| &route.name,
        |route| &route.namespace,
        |route| &route.version,
        |route| route.hostnames.as_slice(),
        |route| route.parent_refs.len(),
        |route| route.backend_refs.len(),
    );
}

pub fn render_grpc_routes(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    search: &str,
    focused: bool,
) {
    render_route_table(
        frame,
        area,
        snapshot,
        bookmarks,
        selected_idx,
        search,
        focused,
        AppView::GrpcRoutes,
        "GRPCRoutes",
        &snapshot.grpc_routes,
        |query| filtered_grpc_route_indices(&snapshot.grpc_routes, query),
        grpc_route_ref,
        |route| &route.name,
        |route| &route.namespace,
        |route| &route.version,
        |route| route.hostnames.as_slice(),
        |route| route.parent_refs.len(),
        |route| route.backend_refs.len(),
    );
}

#[allow(clippy::too_many_arguments)]
fn render_route_table<T, Filter, Resource, Name, Namespace, Version, Hosts, Parents, Backends>(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    search: &str,
    focused: bool,
    view: AppView,
    label: &'static str,
    items: &[T],
    filter: Filter,
    resource: Resource,
    name: Name,
    namespace: Namespace,
    version: Version,
    hosts: Hosts,
    parents: Parents,
    backends: Backends,
) where
    Filter: FnOnce(&str) -> Vec<usize>,
    Resource: Fn(&T) -> ResourceRef,
    Name: Fn(&T) -> &str,
    Namespace: Fn(&T) -> &str,
    Version: Fn(&T) -> &str,
    Hosts: Fn(&T) -> &[String],
    Parents: Fn(&T) -> usize,
    Backends: Fn(&T) -> usize,
{
    let theme = default_theme();
    let query = search.trim();
    let indices = filter(query);
    let widths = gateway_route_widths(area);
    render_resource_table(
        frame,
        area,
        &theme,
        ResourceTableConfig {
            snapshot,
            view,
            label,
            loading_message: "Loading routes...",
            empty_message: "No routes found",
            empty_query_message: "No routes match the search query",
            query,
            focused,
            filtered_total: indices.len(),
            all_total: items.len(),
            selected_idx,
            widths: &widths,
            sort_suffix: "",
        },
        |theme| {
            Row::new([
                Cell::from(Span::styled("  NAME", theme.header_style())),
                Cell::from(Span::styled("NAMESPACE", theme.header_style())),
                Cell::from(Span::styled("HOSTNAMES", theme.header_style())),
                Cell::from(Span::styled("PARENTS", theme.header_style())),
                Cell::from(Span::styled("BACKENDS", theme.header_style())),
                Cell::from(Span::styled("API", theme.header_style())),
            ])
            .style(theme.header_style())
            .height(1)
        },
        |window, theme| {
            indices[window.start..window.end]
                .iter()
                .enumerate()
                .map(|(local_idx, &resource_idx)| {
                    let idx = window.start + local_idx;
                    let route = &items[resource_idx];
                    Row::new(vec![
                        bookmarked_name_cell(
                            || resource(route),
                            bookmarks,
                            name(route),
                            Style::default().fg(theme.fg),
                            theme,
                        ),
                        Cell::from(Span::styled(
                            namespace(route),
                            Style::default().fg(theme.fg_dim),
                        )),
                        Cell::from(Span::styled(
                            joined_or_dash(hosts(route)),
                            Style::default().fg(theme.accent2),
                        )),
                        Cell::from(Span::styled(
                            parents(route).to_string(),
                            Style::default().fg(theme.info),
                        )),
                        Cell::from(Span::styled(
                            backends(route).to_string(),
                            Style::default().fg(theme.success),
                        )),
                        Cell::from(Span::styled(
                            version(route),
                            Style::default().fg(theme.fg_dim),
                        )),
                    ])
                    .style(striped_row_style(idx, theme))
                })
                .collect()
        },
    );
}

pub fn render_reference_grants(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    search: &str,
    focused: bool,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = filtered_reference_grant_indices(&snapshot.reference_grants, query);
    let widths = reference_grant_widths(area);
    render_resource_table(
        frame,
        area,
        &theme,
        ResourceTableConfig {
            snapshot,
            view: AppView::ReferenceGrants,
            label: "ReferenceGrants",
            loading_message: "Loading reference grants...",
            empty_message: "No reference grants found",
            empty_query_message: "No reference grants match the search query",
            query,
            focused,
            filtered_total: indices.len(),
            all_total: snapshot.reference_grants.len(),
            selected_idx,
            widths: &widths,
            sort_suffix: "",
        },
        |theme| {
            Row::new([
                Cell::from(Span::styled("  NAME", theme.header_style())),
                Cell::from(Span::styled("NAMESPACE", theme.header_style())),
                Cell::from(Span::styled("FROM", theme.header_style())),
                Cell::from(Span::styled("TO", theme.header_style())),
                Cell::from(Span::styled("API", theme.header_style())),
            ])
            .style(theme.header_style())
            .height(1)
        },
        |window, theme| {
            indices[window.start..window.end]
                .iter()
                .enumerate()
                .map(|(local_idx, &resource_idx)| {
                    let grant = &snapshot.reference_grants[resource_idx];
                    let idx = window.start + local_idx;
                    Row::new(vec![
                        bookmarked_name_cell(
                            || reference_grant_ref(grant),
                            bookmarks,
                            grant.name.as_str(),
                            Style::default().fg(theme.fg),
                            theme,
                        ),
                        Cell::from(Span::styled(
                            grant.namespace.as_str(),
                            Style::default().fg(theme.fg_dim),
                        )),
                        Cell::from(Span::styled(
                            grant.from.len().to_string(),
                            Style::default().fg(theme.info),
                        )),
                        Cell::from(Span::styled(
                            grant.to.len().to_string(),
                            Style::default().fg(theme.success),
                        )),
                        Cell::from(Span::styled(
                            grant.version.as_str(),
                            Style::default().fg(theme.fg_dim),
                        )),
                    ])
                    .style(striped_row_style(idx, theme))
                })
                .collect()
        },
    );
}

fn gateway_class_ref(class: &GatewayClassInfo) -> ResourceRef {
    ResourceRef::CustomResource {
        name: class.name.clone(),
        namespace: None,
        group: "gateway.networking.k8s.io".to_string(),
        version: class.version.clone(),
        kind: GATEWAY_CLASS_SPEC.kind.to_string(),
        plural: GATEWAY_CLASS_SPEC.plural.to_string(),
    }
}

fn gateway_ref(gateway: &GatewayInfo) -> ResourceRef {
    ResourceRef::CustomResource {
        name: gateway.name.clone(),
        namespace: Some(gateway.namespace.clone()),
        group: "gateway.networking.k8s.io".to_string(),
        version: gateway.version.clone(),
        kind: GATEWAY_SPEC.kind.to_string(),
        plural: GATEWAY_SPEC.plural.to_string(),
    }
}

fn http_route_ref(route: &HttpRouteInfo) -> ResourceRef {
    ResourceRef::CustomResource {
        name: route.name.clone(),
        namespace: Some(route.namespace.clone()),
        group: "gateway.networking.k8s.io".to_string(),
        version: route.version.clone(),
        kind: HTTP_ROUTE_SPEC.kind.to_string(),
        plural: HTTP_ROUTE_SPEC.plural.to_string(),
    }
}

fn grpc_route_ref(route: &GrpcRouteInfo) -> ResourceRef {
    ResourceRef::CustomResource {
        name: route.name.clone(),
        namespace: Some(route.namespace.clone()),
        group: "gateway.networking.k8s.io".to_string(),
        version: route.version.clone(),
        kind: GRPC_ROUTE_SPEC.kind.to_string(),
        plural: GRPC_ROUTE_SPEC.plural.to_string(),
    }
}

fn reference_grant_ref(grant: &ReferenceGrantInfo) -> ResourceRef {
    ResourceRef::CustomResource {
        name: grant.name.clone(),
        namespace: Some(grant.namespace.clone()),
        group: "gateway.networking.k8s.io".to_string(),
        version: grant.version.clone(),
        kind: REFERENCE_GRANT_SPEC.kind.to_string(),
        plural: REFERENCE_GRANT_SPEC.plural.to_string(),
    }
}

fn joined_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(", ")
    }
}

fn bool_label(
    value: Option<bool>,
    yes: &'static str,
    no: &'static str,
    unknown: &'static str,
) -> &'static str {
    match value {
        Some(true) => yes,
        Some(false) => no,
        None => unknown,
    }
}

fn accepted_style(value: Option<bool>, theme: &crate::ui::theme::Theme) -> Style {
    match value {
        Some(true) => Style::default().fg(theme.success),
        Some(false) => Style::default().fg(theme.error),
        None => Style::default().fg(theme.warning),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_route_widths_switch_to_compact_profile() {
        let widths = gateway_route_widths(Rect::new(0, 0, 84, 20));
        assert_eq!(widths[1], Constraint::Length(14));
        assert_eq!(widths[2], Constraint::Min(22));
    }

    #[test]
    fn gateway_route_widths_keep_wide_profile_on_large_area() {
        let widths = gateway_route_widths(Rect::new(0, 0, 120, 20));
        assert_eq!(widths[0], Constraint::Percentage(20));
        assert_eq!(widths[2], Constraint::Percentage(28));
    }

    #[test]
    fn gateway_class_widths_switch_to_compact_profile() {
        let widths = gateway_class_widths(Rect::new(0, 0, 84, 20));
        assert_eq!(widths[0], Constraint::Min(18));
        assert_eq!(widths[1], Constraint::Min(24));
        assert_eq!(widths[2], Constraint::Length(8));
    }

    #[test]
    fn gateway_widths_switch_to_compact_profile() {
        let widths = gateway_widths(Rect::new(0, 0, 92, 20));
        assert_eq!(widths[0], Constraint::Min(18));
        assert_eq!(widths[1], Constraint::Length(14));
        assert_eq!(widths[3], Constraint::Min(16));
    }

    #[test]
    fn reference_grant_widths_switch_to_compact_profile() {
        let widths = reference_grant_widths(Rect::new(0, 0, 84, 20));
        assert_eq!(widths[0], Constraint::Min(20));
        assert_eq!(widths[2], Constraint::Length(8));
        assert_eq!(widths[4], Constraint::Length(7));
    }
}
