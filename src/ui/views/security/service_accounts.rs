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
        format_age, format_small_int, render_resource_table, sort_header_cell, striped_row_style,
        views::filtering::filtered_service_account_indices,
        workload_sort_suffix,
    },
};

#[derive(Debug, Clone)]
struct ServiceAccountDerivedCell {
    age: String,
    automount_label: &'static str,
}

type ServiceAccountDerivedCacheValue = DerivedRowsCacheValue<ServiceAccountDerivedCell>;
static SERVICE_ACCOUNT_DERIVED_CACHE: LazyLock<DerivedRowsCache<ServiceAccountDerivedCell>> =
    LazyLock::new(Default::default);

#[allow(clippy::too_many_arguments)]
pub fn render_service_accounts(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    query: &str,
    sort: Option<WorkloadSortState>,
    focused: bool,
) {
    let query = query.trim();
    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::ServiceAccounts,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.service_accounts, cluster.snapshot_version),
        cache_variant,
        |q| filtered_service_account_indices(&cluster.service_accounts, q, sort),
    );

    let theme = default_theme();

    let derived = cached_service_account_derived(cluster, query, indices.as_ref(), cache_variant);
    let name_style = Style::default().fg(theme.fg);
    let dim_style = Style::default().fg(theme.fg_dim);
    let widths = [
        Constraint::Length(26),
        Constraint::Length(18),
        Constraint::Length(9),
        Constraint::Length(13),
        Constraint::Length(11),
        Constraint::Fill(1),
    ];
    let sort_suffix = workload_sort_suffix(sort);
    render_resource_table(
        frame,
        area,
        &theme,
        ResourceTableConfig {
            snapshot: cluster,
            view: AppView::ServiceAccounts,
            label: "ServiceAccounts",
            loading_message: "Loading serviceaccounts...",
            empty_message: "No serviceaccounts found",
            empty_query_message: "No serviceaccounts match the search query",
            query,
            focused,
            filtered_total: indices.len(),
            all_total: cluster.service_accounts.len(),
            selected_idx,
            widths: &widths,
            sort_suffix: &sort_suffix,
        },
        |theme| {
            Row::new([
                sort_header_cell("Name", sort, WorkloadSortColumn::Name, theme, true),
                Cell::from(Span::styled("Namespace", theme.header_style())),
                Cell::from(Span::styled("Secrets", theme.header_style())),
                Cell::from(Span::styled("PullSecrets", theme.header_style())),
                Cell::from(Span::styled("Automount", theme.header_style())),
                sort_header_cell("Age", sort, WorkloadSortColumn::Age, theme, false),
            ])
            .height(1)
            .style(theme.header_style())
        },
        |window, theme| {
            indices[window.start..window.end]
                .iter()
                .enumerate()
                .map(|(local_idx, &sa_idx)| {
                    let idx = window.start + local_idx;
                    let sa = &cluster.service_accounts[sa_idx];
                    let (age_text, automount_text) = if let Some(cell) = derived.get(idx) {
                        (
                            Cow::Borrowed(cell.age.as_str()),
                            Cow::Borrowed(cell.automount_label),
                        )
                    } else {
                        (
                            Cow::Owned(format_age(sa.age)),
                            Cow::Borrowed(automount_label(sa.automount_service_account_token)),
                        )
                    };
                    let automount_style = match sa.automount_service_account_token {
                        Some(true) => theme.badge_success_style(),
                        Some(false) => theme.badge_warning_style(),
                        None => theme.inactive_style(),
                    };
                    Row::new(vec![
                        bookmarked_name_cell(
                            &ResourceRef::ServiceAccount(sa.name.clone(), sa.namespace.clone()),
                            bookmarks,
                            sa.name.as_str(),
                            name_style,
                            theme,
                        ),
                        Cell::from(Span::styled(sa.namespace.as_str(), dim_style)),
                        Cell::from(Span::styled(
                            format_small_int(sa.secrets_count as i64),
                            dim_style,
                        )),
                        Cell::from(Span::styled(
                            format_small_int(sa.image_pull_secrets_count as i64),
                            dim_style,
                        )),
                        Cell::from(Span::styled(automount_text, automount_style)),
                        Cell::from(Span::styled(age_text, theme.inactive_style())),
                    ])
                    .style(striped_row_style(idx, theme))
                })
                .collect()
        },
    );
}

fn cached_service_account_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    variant: u64,
) -> ServiceAccountDerivedCacheValue {
    let key = DerivedRowsCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(&cluster.service_accounts, cluster.snapshot_version),
        variant,
        freshness_bucket: 0,
    };

    cached_derived_rows(&SERVICE_ACCOUNT_DERIVED_CACHE, key, || {
        indices
            .iter()
            .map(|&sa_idx| {
                let sa = &cluster.service_accounts[sa_idx];
                ServiceAccountDerivedCell {
                    age: format_age(sa.age),
                    automount_label: automount_label(sa.automount_service_account_token),
                }
            })
            .collect()
    })
}

fn automount_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "—",
    }
}
