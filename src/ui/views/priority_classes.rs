//! PriorityClasses list view.

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
        format_small_int, render_resource_table, striped_row_style, truncate_message,
        views::filtering::filtered_priority_class_indices,
    },
};

const NARROW_PRIORITY_CLASS_WIDTH: u16 = 96;

fn priority_class_widths(area: Rect) -> [Constraint; 4] {
    if area.width < NARROW_PRIORITY_CLASS_WIDTH {
        [
            Constraint::Min(20),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Min(18),
        ]
    } else {
        [
            Constraint::Percentage(30),
            Constraint::Percentage(10),
            Constraint::Percentage(15),
            Constraint::Percentage(45),
        ]
    }
}

// ── PriorityClass derived cell cache ────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct PriorityClassDerivedCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct PriorityClassDerivedCell {
    value: String,
    description_truncated: String,
}

type PriorityClassDerivedCacheValue = Arc<Vec<PriorityClassDerivedCell>>;
static PRIORITY_CLASS_DERIVED_CACHE: LazyLock<
    Mutex<Option<(PriorityClassDerivedCacheKey, PriorityClassDerivedCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

fn cached_priority_class_derived(
    snapshot: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
) -> PriorityClassDerivedCacheValue {
    let key = PriorityClassDerivedCacheKey {
        query: query.to_string(),
        snapshot_version: snapshot.snapshot_version,
        data_fingerprint: data_fingerprint(&snapshot.priority_classes, snapshot.snapshot_version),
    };

    if let Ok(cache) = PRIORITY_CLASS_DERIVED_CACHE.lock()
        && let Some((cached_key, cached_value)) = cache.as_ref()
        && *cached_key == key
    {
        return cached_value.clone();
    }

    let built = Arc::new(
        indices
            .iter()
            .map(|&pc_idx| {
                let pc = &snapshot.priority_classes[pc_idx];
                PriorityClassDerivedCell {
                    value: format_small_int(i64::from(pc.value)).into_owned(),
                    description_truncated: truncate_message(&pc.description, 60).into_owned(),
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = PRIORITY_CLASS_DERIVED_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

pub fn render_priority_classes(
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
        AppView::PriorityClasses,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.priority_classes, cluster.snapshot_version),
        |q| filtered_priority_class_indices(&cluster.priority_classes, q),
    );

    let derived = cached_priority_class_derived(cluster, query, &indices);
    let widths = priority_class_widths(area);
    render_resource_table(
        frame,
        area,
        &theme,
        ResourceTableConfig {
            snapshot: cluster,
            view: AppView::PriorityClasses,
            label: "PriorityClasses",
            loading_message: "Loading priority classes...",
            empty_message: "No priority classes found",
            empty_query_message: "No priority classes match the search query",
            query,
            focused,
            filtered_total: indices.len(),
            all_total: cluster.priority_classes.len(),
            selected_idx,
            widths: &widths,
            sort_suffix: "",
        },
        |theme| {
            Row::new([
                Cell::from(Span::styled("  NAME", theme.header_style())),
                Cell::from(Span::styled("VALUE", theme.header_style())),
                Cell::from(Span::styled("GLOBAL DEFAULT", theme.header_style())),
                Cell::from(Span::styled("DESCRIPTION", theme.header_style())),
            ])
            .style(theme.header_style())
            .height(1)
        },
        |window, theme| {
            indices[window.start..window.end]
                .iter()
                .enumerate()
                .map(|(local_idx, &priority_class_idx)| {
                    let idx = window.start + local_idx;
                    let priority_class = &cluster.priority_classes[priority_class_idx];
                    let default_label = if priority_class.global_default {
                        "✓"
                    } else {
                        ""
                    };
                    let (value, desc): (Cow<'_, str>, Cow<'_, str>) =
                        if let Some(cell) = derived.get(idx) {
                            (
                                Cow::Borrowed(cell.value.as_str()),
                                Cow::Borrowed(cell.description_truncated.as_str()),
                            )
                        } else {
                            (
                                format_small_int(i64::from(priority_class.value)),
                                Cow::Owned(
                                    truncate_message(&priority_class.description, 60).into_owned(),
                                ),
                            )
                        };
                    Row::new(vec![
                        bookmarked_name_cell(
                            || ResourceRef::PriorityClass(priority_class.name.clone()),
                            bookmarks,
                            priority_class.name.as_str(),
                            Style::default().fg(theme.fg),
                            theme,
                        ),
                        Cell::from(Span::styled(value, Style::default().fg(theme.info))),
                        Cell::from(Span::styled(
                            default_label,
                            if priority_class.global_default {
                                Style::default().fg(theme.success)
                            } else {
                                Style::default().fg(theme.muted)
                            },
                        )),
                        Cell::from(Span::styled(desc, Style::default().fg(theme.fg_dim))),
                    ])
                    .style(striped_row_style(idx, theme))
                })
                .collect()
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{k8s::dtos::PriorityClassInfo, state::ClusterSnapshot};

    #[test]
    fn priority_class_widths_switch_to_compact_profile() {
        let widths = priority_class_widths(Rect::new(0, 0, 84, 20));
        assert_eq!(widths[0], Constraint::Min(20));
        assert_eq!(widths[1], Constraint::Length(8));
        assert_eq!(widths[3], Constraint::Min(18));
    }

    #[test]
    fn priority_class_widths_keep_wide_profile() {
        let widths = priority_class_widths(Rect::new(0, 0, 120, 20));
        assert_eq!(widths[0], Constraint::Percentage(30));
        assert_eq!(widths[3], Constraint::Percentage(45));
    }

    #[test]
    fn priority_class_derived_description_uses_canonical_truncation() {
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 23,
            ..ClusterSnapshot::default()
        };
        snapshot.priority_classes = vec![PriorityClassInfo {
            description: "x".repeat(80),
            value: 10,
            ..PriorityClassInfo::default()
        }];

        let derived = cached_priority_class_derived(&snapshot, "", &[0]);

        assert_eq!(derived[0].description_truncated.len(), 60);
        assert!(derived[0].description_truncated.ends_with("..."));
    }
}
