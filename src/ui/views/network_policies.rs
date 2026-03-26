//! NetworkPolicies list view.

use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, Mutex},
};

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::Span,
    widgets::{Cell, Row},
};

use crate::{
    app::{AppView, ResourceRef},
    bookmarks::BookmarkEntry,
    state::ClusterSnapshot,
    ui::{
        ResourceTableConfig, bookmarked_name_cell,
        components::default_theme,
        filter_cache::{cached_filter_indices, data_fingerprint},
        format_small_int, render_resource_table, striped_row_style,
        views::filtering::filtered_network_policy_indices,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct NetworkPolicyDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct NetworkPolicyDerivedCell {
    pod_selector: String,
    ingress: String,
    egress: String,
}

type NetworkPolicyDerivedCacheValue = Arc<Vec<NetworkPolicyDerivedCell>>;
static NETWORK_POLICY_DERIVED_CACHE: LazyLock<
    Mutex<Option<(NetworkPolicyDerivedCacheKey, NetworkPolicyDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

pub fn render_network_policies(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    search: &str,
    focused: bool,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = cached_filter_indices(
        AppView::NetworkPolicies,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.network_policies, cluster.snapshot_version),
        |q| filtered_network_policy_indices(&cluster.network_policies, q),
    );

    let derived = cached_network_policy_derived(cluster, query, indices.as_ref());
    let widths = [
        Constraint::Percentage(26),
        Constraint::Percentage(20),
        Constraint::Percentage(34),
        Constraint::Percentage(10),
        Constraint::Percentage(10),
    ];
    render_resource_table(
        frame,
        area,
        &theme,
        ResourceTableConfig {
            snapshot: cluster,
            view: AppView::NetworkPolicies,
            label: "NetworkPolicies",
            loading_message: "Loading network policies...",
            empty_message: "No network policies found",
            empty_query_message: "No network policies match the search query",
            query,
            focused,
            filtered_total: indices.len(),
            all_total: cluster.network_policies.len(),
            selected_idx,
            widths: &widths,
            sort_suffix: "",
        },
        |theme| {
            Row::new([
                Cell::from(Span::styled("  NAME", theme.header_style())),
                Cell::from(Span::styled("NAMESPACE", theme.header_style())),
                Cell::from(Span::styled("POD SELECTOR", theme.header_style())),
                Cell::from(Span::styled("INGRESS", theme.header_style())),
                Cell::from(Span::styled("EGRESS", theme.header_style())),
            ])
            .style(theme.header_style())
            .height(1)
        },
        |window, theme| {
            indices[window.start..window.end]
                .iter()
                .enumerate()
                .map(|(local_idx, &policy_idx)| {
                    let idx = window.start + local_idx;
                    let policy = &cluster.network_policies[policy_idx];
                    let (pod_selector, ingress, egress) = if let Some(cell) = derived.get(idx) {
                        (
                            Cow::Borrowed(cell.pod_selector.as_str()),
                            Cow::Borrowed(cell.ingress.as_str()),
                            Cow::Borrowed(cell.egress.as_str()),
                        )
                    } else {
                        (
                            Cow::Owned(policy.pod_selector.clone()),
                            Cow::Owned(format_small_int(policy.ingress_rules as i64).into_owned()),
                            Cow::Owned(format_small_int(policy.egress_rules as i64).into_owned()),
                        )
                    };
                    Row::new(vec![
                        bookmarked_name_cell(
                            || {
                                ResourceRef::NetworkPolicy(
                                    policy.name.clone(),
                                    policy.namespace.clone(),
                                )
                            },
                            bookmarks,
                            policy.name.as_str(),
                            Style::default().fg(theme.fg),
                            theme,
                        ),
                        Cell::from(Span::styled(
                            policy.namespace.clone(),
                            Style::default().fg(theme.fg_dim),
                        )),
                        Cell::from(Span::styled(
                            pod_selector,
                            Style::default().fg(theme.accent2),
                        )),
                        Cell::from(Span::styled(ingress, Style::default().fg(theme.info))),
                        Cell::from(Span::styled(egress, Style::default().fg(theme.info))),
                    ])
                    .style(striped_row_style(idx, theme))
                })
                .collect()
        },
    );
}

fn cached_network_policy_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> NetworkPolicyDerivedCacheValue {
    let key = NetworkPolicyDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(&cluster.network_policies, cluster.snapshot_version),
    };

    if let Ok(cache) = NETWORK_POLICY_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&policy_idx| {
                let policy = &cluster.network_policies[policy_idx];
                NetworkPolicyDerivedCell {
                    pod_selector: policy.pod_selector.clone(),
                    ingress: format_small_int(policy.ingress_rules as i64).into_owned(),
                    egress: format_small_int(policy.egress_rules as i64).into_owned(),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = NETWORK_POLICY_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}
