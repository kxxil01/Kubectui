//! NetworkPolicies list view.

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
    app::AppView,
    state::ClusterSnapshot,
    ui::{
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices, data_fingerprint},
        format_small_int, loading_or_empty_message, table_viewport_rows, table_window,
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
    selected_idx: usize,
    search: &str,
) {
    let theme = default_theme();
    let query = search.trim();
    let indices = cached_filter_indices(
        AppView::NetworkPolicies,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.network_policies),
        |q| {
            if q.is_empty() {
                return (0..cluster.network_policies.len()).collect();
            }
            cluster
                .network_policies
                .iter()
                .enumerate()
                .filter_map(|(idx, policy)| {
                    (contains_ci(&policy.name, q)
                        || contains_ci(&policy.namespace, q)
                        || contains_ci(&policy.pod_selector, q))
                    .then_some(idx)
                })
                .collect()
        },
    );

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            query,
            "  Loading network policies...",
            "  No network policies found",
            "  No network policies match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("NetworkPolicies")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));

    let header = Row::new([
        Cell::from(Span::styled("  NAME", theme.header_style())),
        Cell::from(Span::styled("NAMESPACE", theme.header_style())),
        Cell::from(Span::styled("POD SELECTOR", theme.header_style())),
        Cell::from(Span::styled("INGRESS", theme.header_style())),
        Cell::from(Span::styled("EGRESS", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let derived = cached_network_policy_derived(cluster, query, indices.as_ref());
    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &policy_idx)| {
            let idx = window.start + local_idx;
            let policy = &cluster.network_policies[policy_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
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
                Cell::from(Span::styled(
                    format!("  {}", policy.name),
                    Style::default().fg(theme.fg),
                )),
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
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let title = if query.is_empty() {
        format!(" NetworkPolicies ({total}) ")
    } else {
        let all = cluster.network_policies.len();
        format!(" NetworkPolicies ({total} of {all}) [/{query}]")
    };

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(26),
            Constraint::Percentage(20),
            Constraint::Percentage(34),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
        ],
    )
    .header(header)
    .block(active_block(&title))
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

fn cached_network_policy_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> NetworkPolicyDerivedCacheValue {
    let key = NetworkPolicyDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(&cluster.network_policies),
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
