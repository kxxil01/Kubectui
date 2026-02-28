//! Flux resources view.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, LazyLock, Mutex},
};

use chrono::{DateTime, Utc};
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
        loading_or_empty_message, table_viewport_rows, table_window,
    },
};

const MAX_FORMATTED_CACHE_ENTRIES: usize = 96;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FluxFormattedCacheKey {
    snapshot_version: u64,
    minute_bucket: i64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct FluxFormattedRow {
    name: String,
    namespace: String,
    kind: String,
    status: String,
    age: String,
    message: String,
}

#[derive(Debug, Default)]
struct FluxFormattedCache {
    map: HashMap<FluxFormattedCacheKey, Arc<Vec<FluxFormattedRow>>>,
    order: VecDeque<FluxFormattedCacheKey>,
}

impl FluxFormattedCache {
    fn get(&mut self, key: &FluxFormattedCacheKey) -> Option<Arc<Vec<FluxFormattedRow>>> {
        let value = self.map.get(key).cloned();
        if value.is_some() {
            self.touch(key);
        }
        value
    }

    fn insert(&mut self, key: FluxFormattedCacheKey, value: Arc<Vec<FluxFormattedRow>>) {
        if self.map.contains_key(&key) {
            self.map.insert(key.clone(), value);
            self.touch(&key);
            return;
        }
        self.map.insert(key.clone(), value);
        self.order.push_back(key);
        self.evict_if_needed();
    }

    fn touch(&mut self, key: &FluxFormattedCacheKey) {
        if let Some(pos) = self.order.iter().position(|item| item == key) {
            self.order.remove(pos);
            self.order.push_back(key.clone());
        }
    }

    fn evict_if_needed(&mut self) {
        while self.order.len() > MAX_FORMATTED_CACHE_ENTRIES {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
    }
}

static FLUX_FORMATTED_CACHE: LazyLock<Mutex<FluxFormattedCache>> =
    LazyLock::new(|| Mutex::new(FluxFormattedCache::default()));

fn cached_formatted_rows(cluster: &ClusterSnapshot, now_unix: i64) -> Arc<Vec<FluxFormattedRow>> {
    let key = FluxFormattedCacheKey {
        snapshot_version: cluster.snapshot_version,
        minute_bucket: now_unix.div_euclid(60),
        data_fingerprint: data_fingerprint(&cluster.flux_resources),
    };

    if let Ok(mut cache) = FLUX_FORMATTED_CACHE.lock()
        && let Some(hit) = cache.get(&key)
    {
        return hit;
    }

    let built: Arc<Vec<FluxFormattedRow>> = Arc::new(
        cluster
            .flux_resources
            .iter()
            .map(|resource| {
                let message = resource.message.as_deref().unwrap_or("-");
                FluxFormattedRow {
                    name: resource.name.clone(),
                    namespace: resource
                        .namespace
                        .clone()
                        .unwrap_or_else(|| "<cluster-scope>".to_string()),
                    kind: resource.kind.clone(),
                    status: resource.status.clone(),
                    age: format_age_from_created(resource.created_at, now_unix),
                    message: truncate_message(message, 56),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = FLUX_FORMATTED_CACHE.lock() {
        cache.insert(key, built.clone());
    }
    built
}

/// Renders Flux resources aggregated from Kustomizations, HelmReleases, and Sources.
pub fn render_flux_resources(
    frame: &mut Frame,
    area: Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let query = query.trim();
    let indices = cached_filter_indices(
        AppView::Flux,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.flux_resources),
        |q| {
            cluster
                .flux_resources
                .iter()
                .enumerate()
                .filter_map(|(idx, resource)| {
                    if q.is_empty()
                        || contains_ci(&resource.name, q)
                        || contains_ci(resource.namespace.as_deref().unwrap_or_default(), q)
                        || contains_ci(&resource.kind, q)
                        || contains_ci(&resource.status, q)
                        || contains_ci(resource.message.as_deref().unwrap_or_default(), q)
                    {
                        Some(idx)
                    } else {
                        None
                    }
                })
                .collect()
        },
    );

    let theme = default_theme();
    if indices.is_empty() {
        let msg = loading_or_empty_message(
            cluster,
            query,
            "  Loading flux resources...",
            "  No flux resources found",
            "  No flux resources match the search query",
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style())).block(default_block("Flux")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));
    let formatted_rows = cached_formatted_rows(cluster, Utc::now().timestamp());

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Kind", theme.header_style())),
        Cell::from(Span::styled("Status", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
        Cell::from(Span::styled("Message", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, &resource_idx)| {
            let idx = window.start + local_idx;
            let resource = &formatted_rows[resource_idx];
            let row_style = if idx.is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", resource.name),
                    Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    resource.namespace.as_str(),
                    Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(
                    resource.kind.as_str(),
                    Style::default().fg(theme.accent2),
                )),
                Cell::from(Span::styled(
                    resource.status.as_str(),
                    status_style(&resource.status, &theme),
                )),
                Cell::from(Span::styled(resource.age.as_str(), theme.inactive_style())),
                Cell::from(Span::styled(
                    resource.message.as_str(),
                    Style::default().fg(theme.fg_dim),
                )),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));
    let title = format!(" 🌀 Flux ({total}) ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        let all = cluster.flux_resources.len();
        active_block(&format!(" 🌀 Flux ({total} of {all}) [/{query}]"))
    };

    let table = Table::new(
        rows,
        [
            Constraint::Min(22),
            Constraint::Length(18),
            Constraint::Length(16),
            Constraint::Length(11),
            Constraint::Length(9),
            Constraint::Min(28),
        ],
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

fn status_style(status: &str, theme: &crate::ui::theme::Theme) -> Style {
    if status.eq_ignore_ascii_case("ready") {
        theme.badge_success_style()
    } else if status.eq_ignore_ascii_case("notready") {
        theme.badge_error_style()
    } else if status.eq_ignore_ascii_case("suspended") {
        theme.badge_warning_style()
    } else {
        theme.inactive_style()
    }
}

fn truncate_message(message: &str, max_chars: usize) -> String {
    if message.chars().count() <= max_chars {
        return message.to_string();
    }
    let mut out = message
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    out.push_str("...");
    out
}

fn format_age_from_created(created_at: Option<DateTime<Utc>>, now_unix: i64) -> String {
    let Some(created_at) = created_at else {
        return "-".to_string();
    };

    let age_secs = now_unix.saturating_sub(created_at.timestamp());
    let days = age_secs / 86_400;
    let hours = (age_secs % 86_400) / 3_600;
    let mins = (age_secs % 3_600) / 60;

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
    use chrono::{Duration, Utc};

    #[test]
    fn status_style_maps_expected_levels() {
        let theme = Theme::dark();
        assert_eq!(status_style("Ready", &theme).fg, Some(theme.success));
        assert_eq!(status_style("NotReady", &theme).fg, Some(theme.error));
        assert_eq!(status_style("Suspended", &theme).fg, Some(theme.warning));
    }

    #[test]
    fn format_age_from_created_uses_now_bucket() {
        let now = Utc::now();
        let created = now - Duration::minutes(95);
        let rendered = format_age_from_created(Some(created), now.timestamp());
        assert_eq!(rendered, "1h 35m");
    }
}
