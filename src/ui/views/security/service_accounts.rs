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
    state::ClusterSnapshot,
    ui::{
        components::{active_block, default_block, default_theme},
        contains_ci,
        filter_cache::{cached_filter_indices_with_variant, data_fingerprint},
        format_small_int, loading_or_empty_message, responsive_table_widths, table_viewport_rows,
        table_window, workload_sort_header, workload_sort_suffix,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServiceAccountDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct ServiceAccountDerivedCell {
    age: String,
    automount_label: &'static str,
}

type ServiceAccountDerivedCacheValue = Arc<Vec<ServiceAccountDerivedCell>>;
static SERVICE_ACCOUNT_DERIVED_CACHE: LazyLock<
    Mutex<
        Option<(
            ServiceAccountDerivedCacheKey,
            ServiceAccountDerivedCacheValue,
        )>,
    >,
> = LazyLock::new(|| Mutex::new(None));

pub fn render_service_accounts(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
    sort: Option<WorkloadSortState>,
) {
    let query = query.trim();
    let cache_variant = sort.map_or(0, WorkloadSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::ServiceAccounts,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.service_accounts, cluster.snapshot_version),
        cache_variant,
        |q| {
            filtered_workload_indices(
                &cluster.service_accounts,
                q,
                sort,
                |sa, needle| {
                    q.is_empty()
                        || contains_ci(&sa.name, needle)
                        || contains_ci(&sa.namespace, needle)
                },
                |sa| sa.name.as_str(),
                |sa| sa.namespace.as_str(),
                |sa| sa.age,
            )
        },
    );

    let theme = default_theme();

    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            AppView::ServiceAccounts,
            query,
            "  Loading serviceaccounts...",
            "  No serviceaccounts found",
            "  No serviceaccounts match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("ServiceAccounts")),
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
        Cell::from(Span::styled("Secrets", theme.header_style())),
        Cell::from(Span::styled("PullSecrets", theme.header_style())),
        Cell::from(Span::styled("Automount", theme.header_style())),
        Cell::from(Span::styled(age_header, theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let derived = cached_service_account_derived(cluster, query, indices.as_ref());
    let name_style = Style::default().fg(theme.fg);
    let dim_style = Style::default().fg(theme.fg_dim);

    let rows: Vec<Row> = indices[window.start..window.end]
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
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            let automount_style = match sa.automount_service_account_token {
                Some(true) => theme.badge_success_style(),
                Some(false) => theme.badge_warning_style(),
                None => theme.inactive_style(),
            };
            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled("  ", name_style),
                    Span::styled(sa.name.as_str(), name_style),
                ])),
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
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));
    let sort_suffix = workload_sort_suffix(sort);
    let title = format!(" 🔑 ServiceAccounts ({total}){sort_suffix} ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.service_accounts.len();
        active_block(&format!(
            " 🔑 ServiceAccounts ({total} of {all}) [/{query}]{sort_suffix}"
        ))
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Length(26),
                Constraint::Length(18),
                Constraint::Length(9),
                Constraint::Length(13),
                Constraint::Length(11),
                Constraint::Fill(1),
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

fn cached_service_account_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> ServiceAccountDerivedCacheValue {
    let key = ServiceAccountDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(&cluster.service_accounts, cluster.snapshot_version),
    };

    if let Ok(cache) = SERVICE_ACCOUNT_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&sa_idx| {
                let sa = &cluster.service_accounts[sa_idx];
                ServiceAccountDerivedCell {
                    age: format_age(sa.age),
                    automount_label: automount_label(sa.automount_service_account_token),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = SERVICE_ACCOUNT_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

fn automount_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "—",
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
