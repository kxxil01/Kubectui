//! User interface composition and rendering utilities.

pub mod components;
mod filter_cache;
pub mod profiling;
mod render_cache;
pub mod theme;
pub mod views;

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    prelude::Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Cell, HighlightSpacing, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState,
    },
};
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

use crate::{
    app::{
        ActiveComponent, AppState, AppView, Focus, PodSortColumn, PodSortState, ResourceRef,
        WorkloadSortColumn, WorkloadSortState, filtered_pod_indices,
    },
    bookmarks::BookmarkEntry,
    icons::view_icon,
    policy::ViewAction,
    state::{
        ClusterSnapshot, DataPhase, RefreshScope, ViewLoadState,
        alerts::{format_mib, format_millicores, parse_mib, parse_millicores},
    },
    time::{AppTimestamp, age_seconds_since, now_unix_seconds},
    ui::{
        components::{content_block, default_theme},
        render_cache::mark_area_skipped,
        theme::Theme,
    },
};

static LOADING_SPINNER_TICK: AtomicU8 = AtomicU8::new(0);

fn set_loading_spinner_tick(tick: u8) {
    LOADING_SPINNER_TICK.store(tick % 8, Ordering::Relaxed);
}

pub(crate) fn loading_spinner_char() -> char {
    const FRAMES: [char; 8] = [
        '\u{280B}', '\u{2819}', '\u{2839}', '\u{2838}', '\u{283C}', '\u{2834}', '\u{2826}',
        '\u{2827}',
    ];
    FRAMES[usize::from(LOADING_SPINNER_TICK.load(Ordering::Relaxed) % 8)]
}
use filter_cache::{
    DerivedRowsCache, DerivedRowsCacheKey, DerivedRowsCacheValue, cached_derived_rows,
    cached_filter_indices_with_variant, data_fingerprint,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SidebarCountsCacheKey {
    snapshot_version: u64,
    loaded_scope: RefreshScope,
    view_load_states_hash: u64,
    collapsed_mask: u16,
    bookmark_count: usize,
}

type SidebarCountsCacheValue = Arc<Vec<(AppView, Option<usize>)>>;

static SIDEBAR_COUNTS_CACHE: LazyLock<
    Mutex<Option<(SidebarCountsCacheKey, SidebarCountsCacheValue, u64)>>,
> = LazyLock::new(|| Mutex::new(None));

fn cached_sidebar_counts(
    app: &AppState,
    cluster: &ClusterSnapshot,
) -> (SidebarCountsCacheValue, u64) {
    use crate::app::SidebarItem;

    let key = SidebarCountsCacheKey {
        snapshot_version: cluster.snapshot_version,
        loaded_scope: cluster.loaded_scope,
        view_load_states_hash: hash_view_load_states(&cluster.view_load_states),
        collapsed_mask: crate::app::sidebar::collapsed_mask(&app.collapsed_groups),
        bookmark_count: app.bookmark_count(),
    };

    if let Ok(cache) = SIDEBAR_COUNTS_CACHE.lock()
        && let Some((cached_key, counts, counts_hash)) = cache.as_ref()
        && *cached_key == key
    {
        return (Arc::clone(counts), *counts_hash);
    }

    let counts = Arc::new(
        crate::app::sidebar_rows(&app.collapsed_groups)
            .iter()
            .filter_map(|item| match item {
                SidebarItem::View(view) if *view == AppView::Bookmarks => {
                    Some((*view, Some(key.bookmark_count)))
                }
                SidebarItem::View(view) => Some((*view, cluster.resource_count(*view))),
                SidebarItem::Group(_) => None,
            })
            .collect::<Vec<_>>(),
    );
    let counts_hash = counts
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, (view, count)| {
            let count_bits = count.map_or(u64::MAX, |value| value as u64);
            hash.wrapping_mul(0x100000001b3)
                ^ (((*view as u64) + 1).wrapping_mul(0x9e3779b97f4a7c15)
                    ^ count_bits.rotate_left(17))
        });

    if let Ok(mut cache) = SIDEBAR_COUNTS_CACHE.lock() {
        *cache = Some((key, Arc::clone(&counts), counts_hash));
    }

    (counts, counts_hash)
}

fn hash_view_load_states(states: &[ViewLoadState]) -> u64 {
    states.iter().fold(0xcbf29ce484222325_u64, |hash, state| {
        let state_bits = match state {
            ViewLoadState::Idle => 0,
            ViewLoadState::Loading => 1,
            ViewLoadState::Refreshing => 2,
            ViewLoadState::Ready => 3,
        };
        hash.wrapping_mul(0x100000001b3) ^ state_bits
    })
}

/// Case-insensitive substring match without allocating a new lowercase string.
#[inline]
pub(crate) fn contains_ci(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Formats small integer values without heap allocation for common cases.
#[inline]
pub(crate) fn format_small_int(value: i64) -> Cow<'static, str> {
    match value {
        0 => Cow::Borrowed("0"),
        1 => Cow::Borrowed("1"),
        2 => Cow::Borrowed("2"),
        3 => Cow::Borrowed("3"),
        4 => Cow::Borrowed("4"),
        5 => Cow::Borrowed("5"),
        6 => Cow::Borrowed("6"),
        7 => Cow::Borrowed("7"),
        8 => Cow::Borrowed("8"),
        9 => Cow::Borrowed("9"),
        10 => Cow::Borrowed("10"),
        _ => Cow::Owned(value.to_string()),
    }
}

/// Returns a threshold-colored style for resource utilization percentage.
/// Green <70%, yellow 70–90%, red >=90%.
pub(crate) fn utilization_style(pct: u64, theme: &Theme) -> Style {
    if pct >= 90 {
        theme.badge_error_style()
    } else if pct >= 70 {
        theme.badge_warning_style()
    } else {
        theme.badge_success_style()
    }
}

/// Builds a compact utilization bar with percentage for use in table cells.
///
/// Example output: `▓▓▓▓▓▓░░  45%` (8-char bar + percentage).
pub(crate) fn utilization_bar(pct: u64, theme: &Theme) -> Line<'static> {
    utilization_bar_labeled("", pct, theme)
}

/// Builds a utilization bar prefixed with a text label (e.g. `250m/4 ▓░░░  6%`).
pub(crate) fn utilization_bar_labeled(label: &str, pct: u64, theme: &Theme) -> Line<'static> {
    let pct = pct.min(100);
    const BAR_WIDTH: usize = 8;
    let filled = ((pct as usize) * BAR_WIDTH + 50) / 100; // round
    let empty = BAR_WIDTH - filled;
    let bar_filled: String = "▓".repeat(filled);
    let bar_empty: String = "░".repeat(empty);
    let pct_label = format!(" {pct:>3}%");
    let style = utilization_style(pct, theme);
    let dim = Style::default().fg(theme.fg_dim);
    let mut spans = Vec::with_capacity(4);
    if !label.is_empty() {
        spans.push(Span::styled(format!("{label} "), dim));
    }
    spans.push(Span::styled(bar_filled, style));
    spans.push(Span::styled(bar_empty, dim));
    spans.push(Span::styled(pct_label, style));
    Line::from(spans)
}

pub(crate) fn name_cell_with_bookmark<'a>(
    bookmarked: bool,
    name: impl Into<Cow<'a, str>>,
    name_style: Style,
    theme: &Theme,
) -> Cell<'a> {
    let marker = if bookmarked { "★ " } else { "  " };
    let name = name.into();
    let marker_style = if bookmarked {
        theme.badge_warning_style()
    } else {
        Style::default().fg(theme.fg_dim)
    };
    Cell::from(Line::from(vec![
        Span::styled(marker, marker_style),
        Span::styled(name, name_style),
    ]))
}

pub(crate) fn bookmarked_name_cell<'a, F>(
    resource: F,
    bookmarks: &[BookmarkEntry],
    name: impl Into<Cow<'a, str>>,
    name_style: Style,
    theme: &Theme,
) -> Cell<'a>
where
    F: FnOnce() -> ResourceRef,
{
    let bookmarked = !bookmarks.is_empty() && {
        let resource = resource();
        bookmarks
            .iter()
            .any(|bookmark| bookmark.resource == resource)
    };
    name_cell_with_bookmark(bookmarked, name, name_style, theme)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TableWindow {
    pub start: usize,
    pub end: usize,
    pub selected: usize,
}

/// Computes how many table rows can be displayed inside a bordered table with a one-line header.
#[inline]
pub(crate) fn table_viewport_rows(area: Rect) -> usize {
    usize::from(area.height.saturating_sub(3)).max(1)
}

#[inline]
pub(crate) fn wrapped_line_count(lines: &[Line<'_>], width: u16) -> usize {
    let usable_width = usize::from(width.saturating_sub(1).max(1));
    lines
        .iter()
        .map(|line| {
            let content_width = line
                .spans
                .iter()
                .map(|span| span.content.chars().count())
                .sum::<usize>();
            content_width.max(1).div_ceil(usable_width)
        })
        .sum()
}

pub(crate) fn wrap_span_groups(groups: &[Vec<Span<'static>>], width: u16) -> Vec<Line<'static>> {
    if groups.is_empty() {
        return vec![Line::from(String::new())];
    }

    let usable_width = usize::from(width.max(1));
    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut current_width = 0usize;

    for group in groups {
        let group_width = group
            .iter()
            .map(|span| span.content.chars().count())
            .sum::<usize>()
            .max(1);
        if !current.is_empty() && current_width + group_width > usable_width {
            lines.push(Line::from(std::mem::take(&mut current)));
            current_width = 0;
        }
        current.extend(group.iter().cloned());
        current_width += group_width;
    }

    if current.is_empty() {
        lines.push(Line::from(String::new()));
    } else {
        lines.push(Line::from(current));
    }

    lines
}

/// Computes the visible window for a selected row, centered when possible.
#[inline]
pub(crate) fn table_window(total: usize, selected: usize, viewport_rows: usize) -> TableWindow {
    if total == 0 {
        return TableWindow {
            start: 0,
            end: 0,
            selected: 0,
        };
    }
    let selected = selected.min(total.saturating_sub(1));
    let visible = viewport_rows.max(1).min(total);
    let mut start = selected.saturating_sub(visible / 2);
    let max_start = total.saturating_sub(visible);
    if start > max_start {
        start = max_start;
    }
    let end = start + visible;
    TableWindow {
        start,
        end,
        selected: selected.saturating_sub(start),
    }
}

/// Shared parameters for [`render_table_frame`].
pub(crate) struct TableFrame<'a> {
    pub rows: Vec<Row<'a>>,
    pub header: Row<'a>,
    pub widths: &'a [Constraint],
    pub title: &'a str,
    pub focused: bool,
    pub window: TableWindow,
    pub total: usize,
    pub selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SplitPaneFocus {
    None,
    List,
    Detail,
}

/// Shared configuration for the common resource table render path.
pub(crate) struct ResourceTableConfig<'a> {
    pub snapshot: &'a ClusterSnapshot,
    pub view: AppView,
    pub label: &'a str,
    pub loading_message: &'a str,
    pub empty_message: &'a str,
    pub empty_query_message: &'a str,
    pub query: &'a str,
    pub focused: bool,
    pub filtered_total: usize,
    pub all_total: usize,
    pub selected_idx: usize,
    pub widths: &'a [Constraint],
    pub sort_suffix: &'a str,
}

/// Renders the shared table frame: selection state, title block, table widget, and scrollbar.
///
/// Views build their own `rows`, `header`, and `widths`, then delegate the identical
/// table/scrollbar/block assembly to this helper.
pub(crate) fn render_table_frame(frame: &mut Frame, area: Rect, tf: TableFrame<'_>, theme: &Theme) {
    let mut table_state = TableState::default().with_selected(Some(tf.window.selected));

    let block = content_block(tf.title, tf.focused);

    let table = Table::new(tf.rows, responsive_table_widths_vec(area.width, tf.widths))
        .header(tf.header)
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

    let mut scrollbar_state = ScrollbarState::new(tf.total).position(tf.selected);
    frame.render_stateful_widget(
        scrollbar,
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}

/// Returns the alternating row style used by standard resource tables.
#[inline]
pub(crate) fn striped_row_style(index: usize, theme: &Theme) -> Style {
    if index.is_multiple_of(2) {
        Style::default().bg(theme.bg)
    } else {
        theme.row_alt_style()
    }
}

/// Renders the common resource table path used by workload-style list views.
pub(crate) fn render_resource_table<'a, FHeader, FRows>(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    config: ResourceTableConfig<'_>,
    build_header: FHeader,
    build_rows: FRows,
) where
    FHeader: FnOnce(&Theme) -> Row<'a>,
    FRows: FnOnce(TableWindow, &Theme) -> Vec<Row<'a>>,
{
    if config.filtered_total == 0 {
        render_centered_message(
            frame,
            area,
            config.snapshot,
            config.view,
            config.query,
            config.label,
            config.loading_message,
            config.empty_message,
            config.empty_query_message,
            config.focused,
        );
        return;
    }

    let total = config.filtered_total;
    let selected = config.selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));
    let header = build_header(theme);
    let rows = build_rows(window, theme);
    debug_assert_eq!(rows.len(), window.end.saturating_sub(window.start));

    let title_suffix =
        resource_table_title_suffix(config.snapshot, config.view, config.sort_suffix);

    let title = resource_table_title(
        view_icon(config.view).active(),
        config.label,
        total,
        config.all_total,
        config.query,
        &title_suffix,
    );

    render_table_frame(
        frame,
        area,
        TableFrame {
            rows,
            header,
            widths: config.widths,
            title: &title,
            focused: config.focused,
            window,
            total,
            selected,
        },
        theme,
    );
}

/// Builds the standard title string for a resource table view.
///
/// The `icon` parameter should include its own trailing space (or be empty
/// for plain mode) — no extra space is inserted between icon and label.
pub(crate) fn resource_table_title(
    icon: &str,
    label: &str,
    total: usize,
    all: usize,
    query: &str,
    sort_suffix: &str,
) -> String {
    if query.is_empty() {
        format!(" {icon}{label} ({total}){sort_suffix} ")
    } else {
        format!(" {icon}{label} ({total} of {all}) [/{query}]{sort_suffix}")
    }
}

pub(crate) fn resource_table_title_suffix<'a>(
    snapshot: &ClusterSnapshot,
    view: AppView,
    base_suffix: &'a str,
) -> Cow<'a, str> {
    let activity_suffix = match snapshot.view_load_state(view) {
        ViewLoadState::Loading => format!(" [{} loading]", loading_spinner_char()),
        ViewLoadState::Refreshing => format!(" [{} refreshing]", loading_spinner_char()),
        ViewLoadState::Idle | ViewLoadState::Ready => String::new(),
    };
    if activity_suffix.is_empty() {
        Cow::Borrowed(base_suffix)
    } else if base_suffix.is_empty() {
        Cow::Owned(activity_suffix)
    } else {
        Cow::Owned(format!("{base_suffix}{activity_suffix}"))
    }
}

/// Delegates to [`responsive_table_widths_vec`] and converts back to a fixed-size array.
pub(crate) fn responsive_table_widths<const N: usize>(
    area_width: u16,
    wide: [Constraint; N],
) -> [Constraint; N] {
    let vec = responsive_table_widths_vec(area_width, &wide);
    debug_assert_eq!(vec.len(), N, "responsive_table_widths_vec length mismatch");
    std::array::from_fn(|idx| vec[idx])
}

/// Proportionally scales column constraints to fit the available width.
///
/// If the ideal total fits, returns the input unchanged. Otherwise converts
/// to percentage-based constraints using largest-remainder allocation.
pub(crate) fn responsive_table_widths_vec(area_width: u16, wide: &[Constraint]) -> Vec<Constraint> {
    let n = wide.len();
    if n == 0 {
        return Vec::new();
    }
    let usable_width = area_width.saturating_sub(3);
    let ideal_total: u16 = wide.iter().map(|c| constraint_ideal_width(*c)).sum();

    if usable_width >= ideal_total {
        return wide.to_vec();
    }

    let total_weight = ideal_total.max(1) as u32;
    let mut percentages = vec![0u16; n];
    let mut assigned = 0u16;
    let mut remainders: Vec<(u32, usize)> = Vec::with_capacity(n);

    for (idx, constraint) in wide.iter().copied().enumerate() {
        let ideal = u32::from(constraint_ideal_width(constraint).max(1));
        let scaled = ideal * 100;
        let percentage = (scaled / total_weight) as u16;
        percentages[idx] = percentage;
        assigned = assigned.saturating_add(percentage);
        remainders.push((scaled % total_weight, idx));
    }

    remainders
        .sort_unstable_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    let remaining = 100u16.saturating_sub(assigned);
    for idx in 0..usize::from(remaining) {
        percentages[remainders[idx % n].1] = percentages[remainders[idx % n].1].saturating_add(1);
    }

    percentages
        .into_iter()
        .map(Constraint::Percentage)
        .collect()
}

fn constraint_ideal_width(constraint: Constraint) -> u16 {
    match constraint {
        Constraint::Percentage(value) => value.max(1),
        Constraint::Ratio(numerator, denominator) => numerator
            .saturating_mul(100)
            .checked_div(denominator)
            .map(|value| value.try_into().unwrap_or(100))
            .unwrap_or(1),
        Constraint::Length(value) | Constraint::Min(value) | Constraint::Max(value) => value.max(1),
        Constraint::Fill(value) => value.max(1),
    }
}

/// Renders a centered loading/empty message inside a bordered block.
/// Used by all view renderers when there is no data to show.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_centered_message(
    frame: &mut Frame,
    area: Rect,
    snapshot: &ClusterSnapshot,
    view: AppView,
    query: &str,
    title: &str,
    loading_text: &str,
    empty_text: &str,
    no_match_text: &str,
    focused: bool,
) {
    use ratatui::layout::Alignment;
    use ratatui::text::Line;

    let theme = default_theme();
    let is_loading = matches!(
        snapshot.view_load_state(view),
        ViewLoadState::Idle | ViewLoadState::Loading | ViewLoadState::Refreshing
    );

    let line = if is_loading {
        Line::from(vec![
            Span::styled(
                format!("{} ", loading_spinner_char()),
                Style::default().fg(theme.accent),
            ),
            Span::styled(loading_text, Style::default().fg(theme.muted)),
        ])
    } else if query.trim().is_empty() {
        Line::from(vec![
            Span::styled("○ ", Style::default().fg(theme.fg_dim)),
            Span::styled(empty_text, theme.inactive_style()),
        ])
    } else {
        Line::from(vec![
            Span::styled("⊘ ", Style::default().fg(theme.warning)),
            Span::styled(no_match_text, theme.inactive_style()),
        ])
    };

    frame.render_widget(
        Paragraph::new(line)
            .alignment(Alignment::Center)
            .block(components::content_block(title, focused)),
        area,
    );
}

fn effective_workbench_height(
    total_body_height: u16,
    requested_height: u16,
    open: bool,
    maximized: bool,
) -> u16 {
    if !open || total_body_height <= 12 {
        return 0;
    }

    if maximized {
        return total_body_height;
    }

    let max_height = total_body_height.saturating_sub(8);
    requested_height.min(max_height).max(6)
}

const MIN_TERMINAL_WIDTH: u16 = 40;
const MIN_TERMINAL_HEIGHT: u16 = 10;
const DEFAULT_SIDEBAR_WIDTH: u16 = 26;
const MIN_SIDEBAR_WIDTH: u16 = 14;
const MIN_CONTENT_WIDTH: u16 = 24;

fn effective_sidebar_width(total_width: u16) -> u16 {
    let max_sidebar = total_width.saturating_sub(MIN_CONTENT_WIDTH);
    DEFAULT_SIDEBAR_WIDTH.min(max_sidebar.max(MIN_SIDEBAR_WIDTH))
}

fn current_view_activity(
    snapshot: &ClusterSnapshot,
    view: AppView,
    spinner: char,
) -> Option<String> {
    match snapshot.view_load_state(view) {
        ViewLoadState::Loading => Some(format!("{spinner} {} loading...", view.label())),
        ViewLoadState::Refreshing => Some(format!("{spinner} {} refreshing...", view.label())),
        ViewLoadState::Idle | ViewLoadState::Ready => None,
    }
}

/// Returns (header_text, is_active_sort_column).
pub(crate) fn workload_sort_header(
    label: &str,
    sort: Option<WorkloadSortState>,
    column: WorkloadSortColumn,
) -> (String, bool) {
    match sort {
        Some(WorkloadSortState {
            column: active,
            descending: true,
        }) if active == column => (format!("{label} ▼"), true),
        Some(WorkloadSortState {
            column: active,
            descending: false,
        }) if active == column => (format!("{label} ▲"), true),
        _ => (label.to_string(), false),
    }
}

/// Returns a styled `Cell` for a sortable column header.
/// When `padded` is true, the label is prefixed with two spaces (for the Name column).
pub(crate) fn sort_header_cell(
    label: &str,
    sort: Option<WorkloadSortState>,
    column: WorkloadSortColumn,
    theme: &Theme,
    padded: bool,
) -> Cell<'static> {
    let (text, is_sorted) = workload_sort_header(label, sort, column);
    let text = if padded { format!("  {text}") } else { text };
    let style = if is_sorted {
        theme.sort_indicator_style()
    } else {
        theme.header_style()
    };
    Cell::from(Span::styled(text, style))
}

pub(crate) fn workload_sort_suffix(sort: Option<WorkloadSortState>) -> String {
    sort.map(|state| format!(" • sort: {}", state.short_label()))
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
struct PodDerivedCell {
    age: String,
}

type PodDerivedCacheValue = DerivedRowsCacheValue<PodDerivedCell>;
type PodMetricsMap<'a> = HashMap<(&'a str, &'a str), (u64, u64)>;
static POD_DERIVED_CACHE: LazyLock<DerivedRowsCache<PodDerivedCell>> =
    LazyLock::new(Default::default);

fn cached_pod_derived(
    cluster: &ClusterSnapshot,
    query: &str,
    indices: &[usize],
    now_unix: i64,
    variant: u64,
) -> PodDerivedCacheValue {
    let key = DerivedRowsCacheKey {
        query: query.to_string(),
        snapshot_version: cluster.snapshot_version,
        data_fingerprint: data_fingerprint(&cluster.pods, cluster.snapshot_version),
        variant,
        freshness_bucket: now_unix / 60,
    };

    cached_derived_rows(&POD_DERIVED_CACHE, key, || {
        indices
            .iter()
            .map(|&pod_idx| {
                let pod = &cluster.pods[pod_idx];
                PodDerivedCell {
                    age: format_age_from_timestamp(pod.created_at, now_unix),
                }
            })
            .collect()
    })
}

fn resolve_visible_columns(app: &AppState) -> Cow<'static, [crate::columns::ColumnDef]> {
    if let Some(registry) = crate::columns::columns_for_view(app.view()) {
        if app.preferences.is_none() && app.cluster_preferences.is_none() {
            Cow::Borrowed(registry)
        } else {
            let view_key = crate::columns::view_key(app.view());
            let prefs = crate::preferences::resolve_view_preferences(
                view_key,
                &app.preferences,
                &app.cluster_preferences,
                app.current_context_name.as_deref(),
            );
            Cow::Owned(crate::columns::resolve_columns(registry, &prefs))
        }
    } else {
        Cow::Borrowed(&[])
    }
}

fn truncate_error(msg: &str, max_len: usize) -> &str {
    if msg.len() <= max_len {
        return msg;
    }
    let end = msg.floor_char_boundary(max_len.saturating_sub(1));
    &msg[..end]
}

fn focus_owner_label(app: &AppState, secondary_pane_active: bool) -> &'static str {
    if app.help_overlay.is_open() {
        return "help";
    }
    if app.resource_template_dialog.is_some() {
        return "template dialog";
    }
    if app.command_palette.is_open() {
        return "palette";
    }
    if app.is_context_picker_open() {
        return "context picker";
    }
    if app.is_namespace_picker_open() {
        return "namespace picker";
    }
    if app.confirm_quit {
        return "quit confirm";
    }

    match app.active_component() {
        ActiveComponent::LogsViewer => return "logs",
        ActiveComponent::PortForward => return "port-forward",
        ActiveComponent::DebugContainer => return "debug dialog",
        ActiveComponent::NodeDebug => return "node debug",
        ActiveComponent::Scale => return "scale dialog",
        ActiveComponent::ProbePanel => return "probe panel",
        ActiveComponent::None => {}
    }

    if app.detail_view.is_some() {
        return "detail";
    }

    match app.focus {
        Focus::Sidebar => "sidebar",
        Focus::Content if secondary_pane_active => "secondary pane",
        Focus::Content => "resource list",
        Focus::Workbench => "workbench",
    }
}

fn active_overlay_mask(app: &AppState) -> u16 {
    let mut mask = 0_u16;
    if app.detail_view.is_some() {
        mask |= 1 << 0;
    }
    if app.is_namespace_picker_open() {
        mask |= 1 << 1;
    }
    if app.is_context_picker_open() {
        mask |= 1 << 2;
    }
    if app.command_palette.is_open() {
        mask |= 1 << 3;
    }
    if app.resource_template_dialog.is_some() {
        mask |= 1 << 4;
    }
    if app.confirm_quit {
        mask |= 1 << 5;
    }
    if app.help_overlay.is_open() {
        mask |= 1 << 6;
    }
    mask
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ViewRenderKey {
    view: AppView,
    area: Rect,
    overlay_mask: u16,
    snapshot_version: u64,
    selected_idx: usize,
    content_detail_scroll: usize,
    split_pane_focus: SplitPaneFocus,
    query_hash: u64,
    focused: bool,
    theme_index: u8,
    icon_mode: u8,
    context_hash: u64,
    namespace_hash: u64,
    sort_variant: u64,
    bookmark_hash: u64,
    visible_columns_hash: u64,
    app_view_state_hash: u64,
    phase: DataPhase,
    view_load_state: ViewLoadState,
    loading_spinner_tick: u8,
    transient_hash: u64,
    freshness_bucket: i64,
}

thread_local! {
    static VIEW_RENDERED_STATE: RefCell<Option<(ViewRenderKey, usize)>> =
        const { RefCell::new(None) };
}

#[inline]
fn hash_str(value: &str) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[inline]
fn constraint_signature(constraint: ratatui::layout::Constraint) -> u64 {
    match constraint {
        Constraint::Min(value) => (1_u64 << 32) | u64::from(value),
        Constraint::Max(value) => (2_u64 << 32) | u64::from(value),
        Constraint::Length(value) => (3_u64 << 32) | u64::from(value),
        Constraint::Percentage(value) => (4_u64 << 32) | u64::from(value),
        Constraint::Ratio(num, den) => (5_u64 << 32) ^ (u64::from(num) << 16) ^ u64::from(den),
        Constraint::Fill(value) => (6_u64 << 32) | u64::from(value),
    }
}

fn visible_columns_signature(columns: &[crate::columns::ColumnDef]) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    for column in columns {
        column.id.hash(&mut hasher);
        constraint_signature(column.default_width).hash(&mut hasher);
    }
    hasher.finish()
}

fn bookmark_render_hash(bookmarks: &[BookmarkEntry]) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    for bookmark in bookmarks {
        bookmark.resource.kind().hash(&mut hasher);
        bookmark.resource.name().hash(&mut hasher);
        bookmark.resource.namespace().hash(&mut hasher);
        bookmark.bookmarked_at_unix.hash(&mut hasher);
    }
    hasher.finish()
}

fn active_view_transient_hash(view: AppView, cluster: &ClusterSnapshot) -> u64 {
    match view {
        AppView::Events => cluster.events_last_error.as_deref().map_or(0, hash_str),
        _ => 0,
    }
}

fn active_view_app_state_hash(view: AppView, app: &AppState) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    match view {
        AppView::Extensions => {
            app.extension_in_instances.hash(&mut hasher);
            app.extension_instances_loading.hash(&mut hasher);
            app.extension_selected_crd.hash(&mut hasher);
            app.extension_error.hash(&mut hasher);
            app.extension_instance_cursor.hash(&mut hasher);
            app.extension_instances.len().hash(&mut hasher);
            for resource in &app.extension_instances {
                resource.name.hash(&mut hasher);
                resource.namespace.hash(&mut hasher);
                resource.age.map(|age| age.as_secs()).hash(&mut hasher);
            }
        }
        AppView::PortForwarding => {
            app.tunnel_registry.selected_index().hash(&mut hasher);
            let tunnels = app.tunnel_registry.ordered_tunnels();
            tunnels.len().hash(&mut hasher);
            for tunnel in tunnels {
                tunnel.id.hash(&mut hasher);
                tunnel.target.namespace.hash(&mut hasher);
                tunnel.target.pod_name.hash(&mut hasher);
                tunnel.target.remote_port.hash(&mut hasher);
                tunnel.local_addr.hash(&mut hasher);
                tunnel_state_hash_value(tunnel.state).hash(&mut hasher);
            }
        }
        _ => {}
    }
    hasher.finish()
}

fn tunnel_state_hash_value(state: crate::k8s::portforward::TunnelState) -> u8 {
    match state {
        crate::k8s::portforward::TunnelState::Starting => 0,
        crate::k8s::portforward::TunnelState::Active => 1,
        crate::k8s::portforward::TunnelState::Error => 2,
        crate::k8s::portforward::TunnelState::Closing => 3,
        crate::k8s::portforward::TunnelState::Closed => 4,
    }
}

#[derive(Clone, Copy)]
enum PodColumnKind {
    Name,
    Namespace,
    Ip,
    Status,
    Node,
    Restarts,
    Age,
    CpuUsage,
    MemUsage,
    CpuReq,
    MemReq,
    CpuLim,
    MemLim,
    CpuPctReq,
    MemPctReq,
    CpuPctLim,
    MemPctLim,
    Unknown,
}

impl PodColumnKind {
    fn from_id(id: &str) -> Self {
        match id {
            "name" => Self::Name,
            "namespace" => Self::Namespace,
            "ip" => Self::Ip,
            "status" => Self::Status,
            "node" => Self::Node,
            "restarts" => Self::Restarts,
            "age" => Self::Age,
            "cpu_usage" => Self::CpuUsage,
            "mem_usage" => Self::MemUsage,
            "cpu_req" => Self::CpuReq,
            "mem_req" => Self::MemReq,
            "cpu_lim" => Self::CpuLim,
            "mem_lim" => Self::MemLim,
            "cpu_pct_req" => Self::CpuPctReq,
            "mem_pct_req" => Self::MemPctReq,
            "cpu_pct_lim" => Self::CpuPctLim,
            "mem_pct_lim" => Self::MemPctLim,
            _ => Self::Unknown,
        }
    }

    const fn needs_metrics(self) -> bool {
        matches!(
            self,
            Self::CpuUsage
                | Self::MemUsage
                | Self::CpuPctReq
                | Self::MemPctReq
                | Self::CpuPctLim
                | Self::MemPctLim
        )
    }
}

/// Renders a full frame for the current app and cluster state.
pub fn render(frame: &mut Frame, app: &AppState, cluster: &ClusterSnapshot) {
    let _frame_scope = profiling::frame_scope(app.view());
    let _render_scope = profiling::span_scope("render");
    let area = frame.area();
    set_loading_spinner_tick(app.spinner_tick);

    // Guard against terminals too small to render the layout
    if area.height < MIN_TERMINAL_HEIGHT || area.width < MIN_TERMINAL_WIDTH {
        let msg = format!(
            "Terminal too small ({}x{}). Need at least {}x{}.",
            area.width, area.height, MIN_TERMINAL_WIDTH, MIN_TERMINAL_HEIGHT
        );
        frame.render_widget(
            Paragraph::new(Span::styled(msg, ratatui::style::Style::default())),
            area,
        );
        return;
    }

    let root = {
        let _layout_scope = profiling::span_scope("layout");
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(2),
            ])
            .split(frame.area())
    };

    let overlay_mask = active_overlay_mask(app);

    {
        let _header_scope = profiling::span_scope("header");
        components::render_header(
            frame,
            root[0],
            concat!("KubecTUI v", env!("CARGO_PKG_VERSION")),
            cluster.cluster_summary(),
            cluster.connection_health,
            overlay_mask,
        );
    }

    let body_root = {
        let _body_layout_scope = profiling::span_scope("body_layout");
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(8),
                Constraint::Length(effective_workbench_height(
                    root[1].height,
                    app.workbench().height,
                    app.workbench().open,
                    app.workbench().maximized,
                )),
            ])
            .split(root[1])
    };

    let body = {
        let _body_layout_scope = profiling::span_scope("body_layout");
        let sidebar_width = effective_sidebar_width(body_root[0].width);
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(sidebar_width),
                Constraint::Min(MIN_CONTENT_WIDTH),
            ])
            .split(body_root[0])
    };

    let (sidebar_counts, sidebar_counts_hash) = cached_sidebar_counts(app, cluster);

    {
        let _sidebar_scope = profiling::span_scope("sidebar");
        let sidebar_data = components::SidebarRenderData {
            collapsed: &app.collapsed_groups,
            focus: app.focus,
            counts_hash: sidebar_counts_hash,
            counts: sidebar_counts.as_ref(),
        };
        components::render_sidebar(
            frame,
            body[0],
            app.view(),
            app.sidebar_cursor,
            &sidebar_data,
            overlay_mask,
        );
    }

    if app.workbench().open && body_root[1].height > 0 {
        let _workbench_scope = profiling::span_scope("workbench");
        components::render_workbench(frame, body_root[1], app, cluster);
    }

    let content_full = body[1];

    // When in search mode, carve out a 1-row search bar at the top of the content area
    let (search_area, content) = if app.is_search_mode() {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(content_full);
        (Some(split[0]), split[1])
    } else if !app.search_query().is_empty() {
        // Show active filter hint even after exiting search mode
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(content_full);
        (Some(split[0]), split[1])
    } else {
        (None, content_full)
    };

    if let Some(search_rect) = search_area {
        let theme = default_theme();
        let content = if app.search_query().is_empty() && app.is_search_mode() {
            Line::from(vec![
                Span::styled(
                    " / ",
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("type to filter", Style::default().fg(theme.muted)),
            ])
        } else {
            cursor_visible_input_line(
                &[Span::styled(
                    " / ".to_string(),
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )],
                app.search_query(),
                app.is_search_mode().then_some(app.search_cursor()),
                Style::default().fg(theme.fg),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
                &[Span::styled(
                    "  [Esc] clear".to_string(),
                    Style::default().fg(theme.muted),
                )],
                usize::from(search_rect.width.max(1)),
            )
        };
        frame.render_widget(
            Paragraph::new(content).style(Style::default().bg(theme.bg_surface)),
            search_rect,
        );
    }

    let visible_columns = resolve_visible_columns(app);
    let secondary_pane_active = app.content_secondary_pane_active();
    let split_pane_focus = if secondary_pane_active {
        SplitPaneFocus::Detail
    } else if app.focus == Focus::Content {
        SplitPaneFocus::List
    } else {
        SplitPaneFocus::None
    };
    let content_focused = matches!(split_pane_focus, SplitPaneFocus::List);
    let view_load_state = cluster.view_load_state(app.view());
    let loading_spinner_tick = if matches!(
        view_load_state,
        ViewLoadState::Idle | ViewLoadState::Loading | ViewLoadState::Refreshing
    ) {
        app.spinner_tick
    } else {
        0
    };
    let view_cache_key = ViewRenderKey {
        view: app.view(),
        area: content,
        overlay_mask,
        snapshot_version: cluster.snapshot_version,
        selected_idx: app.selected_idx(),
        content_detail_scroll: app.content_detail_scroll,
        split_pane_focus,
        query_hash: hash_str(app.search_query()),
        focused: content_focused,
        theme_index: crate::ui::theme::active_theme_index(),
        icon_mode: crate::icons::active_icon_mode() as u8,
        context_hash: app.current_context_name.as_deref().map_or(0, hash_str),
        namespace_hash: hash_str(app.get_namespace()),
        sort_variant: app
            .workload_sort()
            .map_or(0, |sort| sort.cache_variant())
            .wrapping_add(app.pod_sort().map_or(0, |sort| sort.cache_variant()) << 8),
        bookmark_hash: bookmark_render_hash(app.bookmarks()),
        visible_columns_hash: visible_columns_signature(visible_columns.as_ref()),
        app_view_state_hash: active_view_app_state_hash(app.view(), app),
        phase: cluster.phase,
        view_load_state,
        loading_spinner_tick,
        transient_hash: active_view_transient_hash(app.view(), cluster),
        // Keep age-sensitive cells advancing without disabling stable-frame skipping.
        freshness_bucket: now_unix_seconds() / 60,
    };
    let frame_count = frame.count();
    let view_skipped = {
        let buffer = frame.buffer_mut();
        VIEW_RENDERED_STATE.with(|cell| {
            let mut cache = cell.borrow_mut();
            if let Some((cached_key, prev_frame)) = cache.as_mut()
                && *cached_key == view_cache_key
                && frame_count == *prev_frame + 1
            {
                mark_area_skipped(buffer, content);
                *prev_frame = frame_count;
                return true;
            }
            false
        })
    };

    if !view_skipped {
        let _view_scope = profiling::span_scope(app.view().profiling_key());
        match app.view() {
            AppView::Dashboard => views::dashboard::render_dashboard(
                frame,
                content,
                cluster,
                app.content_detail_scroll,
                split_pane_focus,
            ),
            AppView::Nodes => views::nodes::render_nodes(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                visible_columns.as_ref(),
                content_focused,
            ),
            AppView::Pods => render_pods_widget(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.pod_sort(),
                visible_columns.as_ref(),
                content_focused,
            ),
            AppView::ReplicaSets => views::replicasets::render_replicasets(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                content_focused,
            ),
            AppView::ReplicationControllers => {
                views::replication_controllers::render_replication_controllers(
                    frame,
                    content,
                    cluster,
                    app.bookmarks(),
                    app.selected_idx(),
                    app.search_query(),
                    app.workload_sort(),
                    content_focused,
                )
            }
            AppView::HelmCharts => views::helm::render_helm_repos(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::HelmReleases => views::helm::render_helm_releases(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::FluxCDAlertProviders
            | AppView::FluxCDAlerts
            | AppView::FluxCDAll
            | AppView::FluxCDArtifacts
            | AppView::FluxCDHelmReleases
            | AppView::FluxCDHelmRepositories
            | AppView::FluxCDImages
            | AppView::FluxCDKustomizations
            | AppView::FluxCDReceivers
            | AppView::FluxCDSources => views::flux::render_flux_resources(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.view(),
                app.workload_sort(),
                content_focused,
            ),
            AppView::Endpoints => views::endpoints::render_endpoints(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::Ingresses => views::ingresses::render_ingresses(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::IngressClasses => views::ingresses::render_ingress_classes(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::GatewayClasses => views::gateway_api::render_gateway_classes(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::Gateways => views::gateway_api::render_gateways(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::HttpRoutes => views::gateway_api::render_http_routes(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::GrpcRoutes => views::gateway_api::render_grpc_routes(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::ReferenceGrants => views::gateway_api::render_reference_grants(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::NetworkPolicies => views::network_policies::render_network_policies(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::PortForwarding => views::port_forwarding::render_port_forwarding(
                frame,
                content,
                &app.tunnel_registry,
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::ConfigMaps => views::config::render_config_maps(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::Secrets => views::config::render_secrets(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::HPAs => views::hpas::render_hpas(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::PriorityClasses => views::priority_classes::render_priority_classes(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::PersistentVolumeClaims => views::storage::render_pvcs(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                content_focused,
            ),
            AppView::PersistentVolumes => views::storage::render_pvs(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                content_focused,
            ),
            AppView::StorageClasses => views::storage::render_storage_classes(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                content_focused,
            ),
            AppView::Namespaces => views::namespaces::render_namespaces(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::Events => views::events::render_events(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::Issues => views::issue_center::render_issues(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::Projects => views::projects::render_projects(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.content_detail_scroll,
                split_pane_focus,
            ),
            AppView::Governance => views::governance::center::render_governance(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                app.content_detail_scroll,
                split_pane_focus,
            ),
            AppView::HealthReport => views::issue_center::render_health_report(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::Vulnerabilities => views::vulnerabilities::render_vulnerabilities(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::Bookmarks => views::bookmarks::render_bookmarks(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                content_focused,
            ),
            AppView::Services => views::services::render_services(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                content_focused,
            ),
            AppView::Deployments => views::deployments::render_deployments(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                visible_columns.as_ref(),
                content_focused,
            ),
            AppView::StatefulSets => views::statefulsets::render_statefulsets(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                content_focused,
            ),
            AppView::DaemonSets => views::daemonsets::render_daemonsets(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                content_focused,
            ),
            AppView::Jobs => views::jobs::render_jobs(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                content_focused,
            ),
            AppView::CronJobs => views::cronjobs::render_cronjobs(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                content_focused,
            ),
            AppView::ServiceAccounts => views::security::service_accounts::render_service_accounts(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                content_focused,
            ),
            AppView::Roles => views::security::roles::render_roles(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                app.content_detail_scroll,
                split_pane_focus,
            ),
            AppView::RoleBindings => views::security::role_bindings::render_role_bindings(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                app.content_detail_scroll,
                split_pane_focus,
            ),
            AppView::ClusterRoles => views::security::cluster_roles::render_cluster_roles(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                app.content_detail_scroll,
                split_pane_focus,
            ),
            AppView::ClusterRoleBindings => {
                views::security::cluster_role_bindings::render_cluster_role_bindings(
                    frame,
                    content,
                    cluster,
                    app.bookmarks(),
                    app.selected_idx(),
                    app.search_query(),
                    app.workload_sort(),
                    app.content_detail_scroll,
                    split_pane_focus,
                )
            }
            AppView::ResourceQuotas => views::governance::quotas::render_resource_quotas(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                content_focused,
            ),
            AppView::LimitRanges => views::governance::limits::render_limit_ranges(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                content_focused,
            ),
            AppView::PodDisruptionBudgets => views::governance::pdbs::render_pdbs(
                frame,
                content,
                cluster,
                app.bookmarks(),
                app.selected_idx(),
                app.search_query(),
                app.workload_sort(),
                content_focused,
            ),
            AppView::Extensions => {
                views::extensions::render_extensions(frame, content, cluster, app, content_focused)
            }
        }

        VIEW_RENDERED_STATE.with(|cell| {
            *cell.borrow_mut() = Some((view_cache_key, frame_count));
        });
    }

    // Toast notifications take priority over regular status messages
    let active_toast = app.toasts.last();
    let focus_owner = focus_owner_label(app, secondary_pane_active);
    let status_prefix = format!(
        "[{}] view: {} • focus: {focus_owner}",
        app.get_namespace(),
        app.view().label()
    );
    let status = if let Some(toast) = active_toast.filter(|t| t.is_error) {
        format!(
            "{status_prefix} • ✗ {}",
            truncate_error(&toast.message, 120)
        )
    } else if let Some(err) = app.error_message() {
        format!("{status_prefix} • ERROR: {}", truncate_error(err, 120))
    } else if let Some(toast) = active_toast {
        format!("{status_prefix} • ● {}", toast.message)
    } else if let Some(message) = app.status_message() {
        format!("{status_prefix} • {message}")
    } else {
        let theme_name = theme::active_theme().name;
        let current_activity = current_view_activity(cluster, app.view(), app.spinner_char())
            .map(|activity| format!(" {activity} •"))
            .unwrap_or_default();
        let staleness = cluster.last_updated.map_or(String::new(), |ts| {
            let elapsed = age_seconds_since(ts, now_unix_seconds());
            if elapsed > 45 {
                format!(" {elapsed}s ago •")
            } else {
                String::new()
            }
        });
        let sort_hint = if app.view() == AppView::Pods {
            let active = app.pod_sort().map_or("default", PodSortState::short_label);
            format!(" • [n/a] sort ({active}) • [1/2/3] pod-sort • [0] clear-sort")
        } else {
            let caps = app.view().shared_sort_capabilities();
            if caps.is_empty() {
                String::new()
            } else {
                let key_hint = if caps == [WorkloadSortColumn::Name] {
                    "[n]"
                } else {
                    "[n/a]"
                };
                let active = app
                    .workload_sort()
                    .map_or("default", WorkloadSortState::short_label);
                format!(" • {key_hint} sort ({active}) • [0] clear-sort")
            }
        };
        let flux_reconcile_hint = if app.detail_view.is_none()
            && app
                .view()
                .supports_view_action(ViewAction::SelectedFluxReconcile)
        {
            " • [R] reconcile"
        } else {
            ""
        };
        let workbench_hint = if app.workbench().open {
            " • [H] history • [b] workbench • [[]/]] tabs • [Ctrl+Up/Down] wb-size • [Ctrl+w] close-tab"
        } else {
            " • [H] history • [b] workbench"
        };
        let focus_hint = match app.focus {
            Focus::Workbench if app.workbench().maximized => " • [Esc] exit maximize",
            Focus::Workbench => " • [Esc] return",
            Focus::Content if secondary_pane_active => " • [;] resource list",
            Focus::Content
                if app.detail_view.is_none() && app.view().supports_secondary_pane_scroll() =>
            {
                " • [;] secondary pane"
            }
            _ => "",
        };
        let navigation_hint = if secondary_pane_active {
            "[j/k] scroll"
        } else {
            "[j/k] navigate"
        };
        format!(
            "{status_prefix}{current_activity}{focus_hint}{staleness} {navigation_hint} • [/] search • [~] ns • [c] ctx • [T] theme:{theme_name}{sort_hint}{flux_reconcile_hint}{workbench_hint} • [r] refresh • [Esc then Enter] quit"
        )
    };

    {
        let _status_scope = profiling::span_scope("status");
        let is_error = active_toast.is_some_and(|t| t.is_error) || app.error_message().is_some();
        components::render_status_bar_with_overlay_mask(
            frame,
            root[2],
            &status,
            is_error,
            overlay_mask,
        );
    }

    if let Some(detail_state) = app.detail_view.as_ref() {
        let _detail_scope = profiling::span_scope("overlay.detail");
        views::detail::render_detail(frame, frame.area(), detail_state);
    }

    if app.is_namespace_picker_open() {
        let _namespace_scope = profiling::span_scope("overlay.namespace_picker");
        app.namespace_picker().render(frame, frame.area());
    }

    if app.is_context_picker_open() {
        let _context_scope = profiling::span_scope("overlay.context_picker");
        app.context_picker.render(frame, frame.area());
    }

    if app.command_palette.is_open() {
        let _command_scope = profiling::span_scope("overlay.command_palette");
        app.command_palette.render(frame, frame.area());
    }

    if let Some(dialog) = app.resource_template_dialog.as_ref() {
        let _template_scope = profiling::span_scope("overlay.resource_template");
        crate::ui::components::render_resource_template_dialog(frame, frame.area(), dialog);
    }

    if app.confirm_quit {
        let _quit_scope = profiling::span_scope("overlay.quit_confirm");
        render_quit_confirm(frame, frame.area());
    }

    if app.help_overlay.is_open() {
        let _help_scope = profiling::span_scope("overlay.help");
        app.help_overlay
            .render(frame, frame.area(), app.detail_view.as_ref());
    }
}

fn render_quit_confirm(frame: &mut Frame, area: ratatui::layout::Rect) {
    use ratatui::{
        style::Modifier,
        text::Line,
        widgets::{Block, BorderType, Borders, Clear},
    };

    let theme = default_theme();

    let w = 36u16;
    let h = 5u16;
    let popup = ratatui::layout::Rect {
        x: (area.width.saturating_sub(w)) / 2,
        y: (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.badge_error_style())
        .style(ratatui::style::Style::default().bg(theme.bg));
    frame.render_widget(block, popup);

    let inner = ratatui::layout::Rect {
        x: popup.x + 1,
        y: popup.y + 1,
        width: popup.width.saturating_sub(2),
        height: popup.height.saturating_sub(2),
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(2)])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  Quit KubecTUI? ",
            theme.title_style(),
        )])),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "  [Enter] ",
                ratatui::style::Style::default()
                    .fg(theme.error)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("quit  ", theme.inactive_style()),
            Span::styled("[any] ", theme.keybind_key_style()),
            Span::styled("cancel", theme.keybind_desc_style()),
        ])),
        chunks[1],
    );
}

#[allow(clippy::too_many_arguments)]
fn render_pods_widget(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    cluster: &ClusterSnapshot,
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    query: &str,
    pod_sort: Option<PodSortState>,
    visible_columns: &[crate::columns::ColumnDef],
    focused: bool,
) {
    let theme = default_theme();
    let cache_variant = pod_sort.map_or(0, PodSortState::cache_variant);
    let indices = cached_filter_indices_with_variant(
        AppView::Pods,
        query,
        cluster.snapshot_version,
        data_fingerprint(&cluster.pods, cluster.snapshot_version),
        cache_variant,
        |q| filtered_pod_indices(&cluster.pods, q, pod_sort),
    );

    if indices.is_empty() {
        render_centered_message(
            frame,
            area,
            cluster,
            AppView::Pods,
            query,
            "Pods",
            "Loading pods...",
            "No pods available (try pressing ~ to switch namespace, or select 'all')",
            "No pods match the search query",
            focused,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));
    let pod_columns = visible_columns
        .iter()
        .map(|col| PodColumnKind::from_id(col.id))
        .collect::<Vec<_>>();

    let pod_sort_header = |kind: PodColumnKind, label: &str| -> String {
        match kind {
            PodColumnKind::Name => format!(
                "  {}",
                match pod_sort {
                    Some(PodSortState {
                        column: PodSortColumn::Name,
                        descending: true,
                    }) => format!("{label}\u{25bc}"),
                    Some(PodSortState {
                        column: PodSortColumn::Name,
                        descending: false,
                    }) => format!("{label}\u{25b2}"),
                    _ => label.to_string(),
                }
            ),
            PodColumnKind::Age => match pod_sort {
                Some(PodSortState {
                    column: PodSortColumn::Age,
                    descending: true,
                }) => format!("{label}\u{25bc}"),
                Some(PodSortState {
                    column: PodSortColumn::Age,
                    descending: false,
                }) => format!("{label}\u{25b2}"),
                _ => label.to_string(),
            },
            PodColumnKind::Status => match pod_sort {
                Some(PodSortState {
                    column: PodSortColumn::Status,
                    descending: true,
                }) => format!("{label}\u{25bc}"),
                Some(PodSortState {
                    column: PodSortColumn::Status,
                    descending: false,
                }) => format!("{label}\u{25b2}"),
                _ => label.to_string(),
            },
            PodColumnKind::Restarts => match pod_sort {
                Some(PodSortState {
                    column: PodSortColumn::Restarts,
                    descending: true,
                }) => format!("{label}\u{25bc}"),
                Some(PodSortState {
                    column: PodSortColumn::Restarts,
                    descending: false,
                }) => format!("{label}\u{25b2}"),
                _ => label.to_string(),
            },
            _ => label.to_string(),
        }
    };

    let header_cells: Vec<Cell> = visible_columns
        .iter()
        .zip(pod_columns.iter().copied())
        .map(|(col, kind)| {
            Cell::from(Span::styled(
                pod_sort_header(kind, col.label),
                theme.header_style(),
            ))
        })
        .collect();
    let header = Row::new(header_cells).height(1).style(theme.header_style());

    let name_style = Style::default().fg(theme.fg);
    let dim_style = Style::default().fg(theme.fg_dim);
    let even_row_style = Style::default().bg(theme.bg);
    let odd_row_style = theme.row_alt_style();
    let age_style = theme.inactive_style();
    let zero_restart_style = theme.inactive_style();
    let warning_restart_style = theme.badge_warning_style();
    let error_restart_style = theme.badge_error_style();
    let now_unix = now_unix_seconds();
    let derived = cached_pod_derived(cluster, query, indices.as_ref(), now_unix, cache_variant);
    let pod_metrics_map: Option<PodMetricsMap<'_>> = pod_columns
        .iter()
        .copied()
        .any(PodColumnKind::needs_metrics)
        .then(|| {
            cluster
                .pod_metrics
                .iter()
                .map(|pm| {
                    let (cpu, mem) = pm.containers.iter().fold((0u64, 0u64), |(ac, am), c| {
                        (ac + parse_millicores(&c.cpu), am + parse_mib(&c.memory))
                    });
                    ((pm.name.as_str(), pm.namespace.as_str()), (cpu, mem))
                })
                .collect()
        });

    let mut rows = Vec::with_capacity(window.end.saturating_sub(window.start));
    for (local_idx, &pod_idx) in indices[window.start..window.end].iter().enumerate() {
        let idx = window.start + local_idx;
        let pod = &cluster.pods[pod_idx];
        let status = pod.status.as_str();
        let status_style = theme.get_status_style(status);
        let restart_style = if pod.restarts > 5 {
            error_restart_style
        } else if pod.restarts > 0 {
            warning_restart_style
        } else {
            zero_restart_style
        };
        let row_style = if idx.is_multiple_of(2) {
            even_row_style
        } else {
            odd_row_style
        };
        let age = derived
            .get(idx)
            .map(|cell| cell.age.as_str())
            .unwrap_or("-");
        let cells: Vec<Cell> = pod_columns
            .iter()
            .copied()
            .map(|kind| match kind {
                PodColumnKind::Name => {
                    if bookmarks.is_empty() {
                        Cell::from(Span::styled(pod.name.as_str(), name_style))
                    } else {
                        bookmarked_name_cell(
                            || ResourceRef::Pod(pod.name.clone(), pod.namespace.clone()),
                            bookmarks,
                            pod.name.as_str(),
                            name_style,
                            &theme,
                        )
                    }
                }
                PodColumnKind::Namespace => {
                    Cell::from(Span::styled(pod.namespace.as_str(), dim_style))
                }
                PodColumnKind::Ip => Cell::from(Span::styled(
                    pod.pod_ip.as_deref().unwrap_or("-"),
                    dim_style,
                )),
                PodColumnKind::Status => Cell::from(Span::styled(status, status_style)),
                PodColumnKind::Node => Cell::from(Span::styled(
                    pod.node.as_deref().unwrap_or("n/a"),
                    dim_style,
                )),
                PodColumnKind::Restarts => Cell::from(Span::styled(
                    format_small_int(i64::from(pod.restarts)),
                    restart_style,
                )),
                PodColumnKind::Age => Cell::from(Span::styled(age, age_style)),
                PodColumnKind::CpuUsage | PodColumnKind::MemUsage => match pod_metrics_map
                    .as_ref()
                    .and_then(|map| map.get(&(pod.name.as_str(), pod.namespace.as_str())))
                {
                    Some(&(cpu_m, mem_mib)) => {
                        let is_cpu = matches!(kind, PodColumnKind::CpuUsage);
                        let usage = if is_cpu { cpu_m } else { mem_mib };
                        let formatted = if is_cpu {
                            format_millicores(usage)
                        } else {
                            format_mib(usage)
                        };
                        let request_raw = if is_cpu {
                            pod.cpu_request.as_deref()
                        } else {
                            pod.memory_request.as_deref()
                        };
                        let style = match request_raw {
                            Some(req) => {
                                let req_val = if is_cpu {
                                    parse_millicores(req)
                                } else {
                                    parse_mib(req)
                                };
                                if req_val > 0 {
                                    let pct =
                                        usage.saturating_mul(100).checked_div(req_val).unwrap_or(0);
                                    utilization_style(pct, &theme)
                                } else {
                                    dim_style
                                }
                            }
                            None => dim_style,
                        };
                        Cell::from(Span::styled(formatted, style))
                    }
                    None => Cell::from(Span::styled("-", dim_style)),
                },
                PodColumnKind::CpuReq => Cell::from(Span::styled(
                    pod.cpu_request.as_deref().unwrap_or("-"),
                    dim_style,
                )),
                PodColumnKind::MemReq => Cell::from(Span::styled(
                    pod.memory_request.as_deref().unwrap_or("-"),
                    dim_style,
                )),
                PodColumnKind::CpuLim => Cell::from(Span::styled(
                    pod.cpu_limit.as_deref().unwrap_or("-"),
                    dim_style,
                )),
                PodColumnKind::MemLim => Cell::from(Span::styled(
                    pod.memory_limit.as_deref().unwrap_or("-"),
                    dim_style,
                )),
                PodColumnKind::CpuPctReq
                | PodColumnKind::MemPctReq
                | PodColumnKind::CpuPctLim
                | PodColumnKind::MemPctLim => match pod_metrics_map
                    .as_ref()
                    .and_then(|map| map.get(&(pod.name.as_str(), pod.namespace.as_str())))
                {
                    Some(&(cpu_m, mem_mib)) => {
                        let is_cpu =
                            matches!(kind, PodColumnKind::CpuPctReq | PodColumnKind::CpuPctLim);
                        let is_req =
                            matches!(kind, PodColumnKind::CpuPctReq | PodColumnKind::MemPctReq);
                        let usage = if is_cpu { cpu_m } else { mem_mib };
                        let denom_str = if is_cpu {
                            if is_req {
                                pod.cpu_request.as_deref()
                            } else {
                                pod.cpu_limit.as_deref()
                            }
                        } else if is_req {
                            pod.memory_request.as_deref()
                        } else {
                            pod.memory_limit.as_deref()
                        };
                        match denom_str {
                            Some(d) => {
                                let denom = if is_cpu {
                                    parse_millicores(d)
                                } else {
                                    parse_mib(d)
                                };
                                if denom > 0 {
                                    let pct =
                                        usage.saturating_mul(100).checked_div(denom).unwrap_or(0);
                                    Cell::from(utilization_bar(pct, &theme))
                                } else {
                                    Cell::from(Span::styled("-", dim_style))
                                }
                            }
                            None => Cell::from(Span::styled("-", dim_style)),
                        }
                    }
                    None => Cell::from(Span::styled("-", dim_style)),
                },
                PodColumnKind::Unknown => Cell::from(""),
            })
            .collect();
        rows.push(Row::new(cells).style(row_style));
    }

    let mut table_state = TableState::default().with_selected(Some(window.selected));

    let sort_suffix = pod_sort
        .map(|state| format!(" • sort: {}", state.short_label()))
        .unwrap_or_default();
    let title_suffix = resource_table_title_suffix(cluster, AppView::Pods, &sort_suffix);
    let icon = view_icon(AppView::Pods).active();
    let title = format!(" {icon}Pods ({total}){title_suffix} ");
    let block = if query.is_empty() {
        content_block(&title, focused)
    } else {
        let all = cluster.pods.len();
        content_block(
            &format!(" {icon}Pods ({total} of {all}) [/{query}]{title_suffix}"),
            focused,
        )
    };

    let constraints =
        crate::columns::visible_constraints_for_area(AppView::Pods, visible_columns, area.width);
    let table = Table::new(rows, responsive_table_widths_vec(area.width, &constraints))
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

/// Formats a `Duration` as a human-readable age string (e.g. "3d 2h", "5h 12m", "7m").
pub fn format_age(age: Option<std::time::Duration>) -> String {
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

pub(crate) fn readiness_style(
    ready: i32,
    desired: i32,
    theme: &crate::ui::theme::Theme,
) -> ratatui::prelude::Style {
    if desired == 0 && ready == 0 {
        theme.inactive_style()
    } else if desired > 0 && ready >= desired {
        theme.badge_success_style()
    } else if ready > 0 {
        theme.badge_warning_style()
    } else {
        theme.badge_error_style()
    }
}

pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

pub(crate) fn centered_rect_by_size(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.max(1).min(area.width.max(1));
    let height = height.max(1).min(area.height.max(1));
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

pub(crate) fn bounded_popup_rect(
    area: Rect,
    preferred_width: u16,
    preferred_height: u16,
    horizontal_margin: u16,
    vertical_margin: u16,
) -> Rect {
    let max_width = area
        .width
        .saturating_sub(horizontal_margin.saturating_mul(2))
        .max(1);
    let max_height = area
        .height
        .saturating_sub(vertical_margin.saturating_mul(2))
        .max(1);
    centered_rect_by_size(
        preferred_width.min(max_width),
        preferred_height.min(max_height),
        area,
    )
}

pub(crate) fn vertical_primary_detail_chunks(
    area: Rect,
    full_primary_percent: u16,
    compact_detail_height: u16,
    min_full_height: u16,
) -> (Rect, Rect) {
    let compact = area.height < min_full_height;
    let detail_height = compact_detail_height.min(area.height.saturating_sub(1).max(1));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if compact {
            [Constraint::Min(1), Constraint::Length(detail_height)]
        } else {
            [
                Constraint::Percentage(full_primary_percent),
                Constraint::Percentage(100_u16.saturating_sub(full_primary_percent)),
            ]
        })
        .split(area);
    (chunks[0], chunks[1])
}

/// Formats a timestamp as a human-readable age relative to `now_unix`.
#[inline]
pub(crate) fn format_age_from_timestamp(created_at: Option<AppTimestamp>, now_unix: i64) -> String {
    let Some(created_at) = created_at else {
        return "-".to_string();
    };
    let age_secs = age_seconds_since(created_at, now_unix);
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

/// Truncates a string to `max_chars` characters, appending "..." when it fits.
/// Returns `Cow::Borrowed` when no truncation is needed to avoid allocation.
pub(crate) fn truncate_message(msg: &str, max_chars: usize) -> Cow<'_, str> {
    if msg.len() <= max_chars {
        return Cow::Borrowed(msg);
    }

    let keep_chars = max_chars.saturating_sub(3);
    let mut keep_end = None;
    let mut max_end = None;

    for (char_index, (byte_index, _)) in msg.char_indices().enumerate() {
        if max_chars >= 4 && char_index == keep_chars {
            keep_end = Some(byte_index);
        }
        if char_index == max_chars {
            max_end = Some(byte_index);
            break;
        }
    }

    let Some(max_end) = max_end else {
        return Cow::Borrowed(msg);
    };

    if max_chars < 4 {
        return Cow::Owned(msg[..max_end].to_string());
    }

    let keep_end = keep_end.unwrap_or(max_end);
    let mut truncated = String::with_capacity(keep_end + 3);
    truncated.push_str(&msg[..keep_end]);
    truncated.push_str("...");
    Cow::Owned(truncated)
}

pub(crate) fn truncate_line_content(line: &Line<'_>, width: usize) -> Line<'static> {
    let text = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    Line::from(truncate_message(&text, width.max(1)).into_owned())
}

pub(crate) fn cursor_visible_input_line(
    prefix: &[Span<'static>],
    value: &str,
    cursor_chars: Option<usize>,
    value_style: Style,
    cursor_style: Style,
    suffix: &[Span<'static>],
    width: usize,
) -> Line<'static> {
    let width = width.max(1);
    let prefix_width = prefix
        .iter()
        .map(|span| span.content.chars().count())
        .sum::<usize>();
    let suffix_width = suffix
        .iter()
        .map(|span| span.content.chars().count())
        .sum::<usize>();
    let cursor_width = usize::from(cursor_chars.is_some());
    let available = width.saturating_sub(prefix_width + suffix_width + cursor_width);
    let char_count = value.chars().count();

    let (start, end) = if available == 0 {
        (0, 0)
    } else if char_count <= available {
        (0, char_count)
    } else if let Some(cursor) = cursor_chars {
        let cursor = cursor.min(char_count);
        let start = cursor
            .saturating_sub(available / 2)
            .min(char_count.saturating_sub(available));
        (start, (start + available).min(char_count))
    } else {
        (0, available.min(char_count))
    };

    let show_ellipses = cursor_chars.is_none() || available > 6;
    let visible_len = end.saturating_sub(start);
    let leading_ellipses = start > 0 && available > 3 && show_ellipses;
    let trailing_ellipses = end < char_count && available > 3 && show_ellipses;

    let mut spans = Vec::with_capacity(prefix.len() + suffix.len() + 3);
    spans.extend(prefix.iter().cloned());

    if let Some(cursor) = cursor_chars {
        let cursor = cursor.min(char_count);
        let cursor_offset = cursor.saturating_sub(start).min(visible_len);
        let (before, after) = visible_input_segments(
            value,
            start,
            visible_len,
            cursor_offset,
            leading_ellipses,
            trailing_ellipses,
        );
        if !before.is_empty() {
            spans.push(Span::styled(before, value_style));
        }
        spans.push(Span::styled("█", cursor_style));
        if !after.is_empty() {
            spans.push(Span::styled(after, value_style));
        }
    } else if visible_len > 0 {
        spans.push(Span::styled(
            visible_input_text(
                value,
                start,
                visible_len,
                leading_ellipses,
                trailing_ellipses,
            ),
            value_style,
        ));
    }
    spans.extend(suffix.iter().cloned());
    Line::from(spans)
}

fn visible_input_text(
    value: &str,
    start: usize,
    visible_len: usize,
    leading_ellipses: bool,
    trailing_ellipses: bool,
) -> String {
    let mut visible = String::with_capacity(visible_len);
    for (offset, ch) in value.chars().skip(start).take(visible_len).enumerate() {
        if (leading_ellipses && offset < 3)
            || (trailing_ellipses && offset >= visible_len.saturating_sub(3))
        {
            visible.push('.');
        } else {
            visible.push(ch);
        }
    }
    visible
}

fn visible_input_segments(
    value: &str,
    start: usize,
    visible_len: usize,
    cursor_offset: usize,
    leading_ellipses: bool,
    trailing_ellipses: bool,
) -> (String, String) {
    let mut before = String::with_capacity(cursor_offset);
    let mut after = String::with_capacity(visible_len.saturating_sub(cursor_offset));
    for (offset, ch) in value.chars().skip(start).take(visible_len).enumerate() {
        let visible_ch = if (leading_ellipses && offset < 3)
            || (trailing_ellipses && offset >= visible_len.saturating_sub(3))
        {
            '.'
        } else {
            ch
        };
        if offset < cursor_offset {
            before.push(visible_ch);
        } else {
            after.push(visible_ch);
        }
    }
    (before, after)
}

pub(crate) fn insert_char_at_cursor(value: &mut String, cursor: &mut usize, ch: char) {
    let byte_idx = value
        .char_indices()
        .nth(*cursor)
        .map_or(value.len(), |(idx, _)| idx);
    value.insert(byte_idx, ch);
    *cursor += 1;
}

pub(crate) fn delete_char_left_at_cursor(value: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    if let Some((byte_idx, _)) = value.char_indices().nth(*cursor - 1) {
        value.remove(byte_idx);
        *cursor = cursor.saturating_sub(1);
    }
}

pub(crate) fn delete_char_right_at_cursor(value: &mut String, cursor: usize) {
    if let Some((byte_idx, _)) = value.char_indices().nth(cursor) {
        value.remove(byte_idx);
    }
}

pub(crate) fn move_cursor_left(cursor: &mut usize) {
    *cursor = cursor.saturating_sub(1);
}

pub(crate) fn move_cursor_right(cursor: &mut usize, value: &str) {
    *cursor = (*cursor + 1).min(value.chars().count());
}

pub(crate) fn move_cursor_home(cursor: &mut usize) {
    *cursor = 0;
}

pub(crate) fn move_cursor_end(cursor: &mut usize, value: &str) {
    *cursor = value.chars().count();
}

pub(crate) fn clear_input_at_cursor(value: &mut String, cursor: &mut usize) {
    value.clear();
    *cursor = 0;
}

pub(crate) fn format_image(image: Option<&str>, max_len: usize) -> String {
    let Some(image) = image else {
        return "-".to_string();
    };
    truncate_message(image, max_len).into_owned()
}
#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{LazyLock, Mutex},
    };

    use jiff::ToSpan;

    use super::resource_table_title;
    use ratatui::{Terminal, backend::TestBackend, style::Color, text::Span};

    use crate::{
        app::{AppState, AppView, DetailMetadata, DetailViewState, ResourceRef},
        bookmarks::BookmarkEntry,
        icons::IconMode,
        k8s::{
            dtos::{
                ClusterRoleBindingInfo, ClusterRoleInfo, ConfigMapInfo, CronJobInfo,
                CustomResourceDefinitionInfo, CustomResourceInfo, DaemonSetInfo, DeploymentInfo,
                FluxResourceInfo, GatewayClassInfo, GatewayInfo, GrpcRouteInfo,
                HelmReleaseRevisionInfo, HttpRouteInfo, IngressClassInfo, IngressInfo, JobInfo,
                LimitRangeInfo, NetworkPolicyInfo, NodeInfo, PodDisruptionBudgetInfo, PodInfo,
                PvInfo, PvcInfo, ReferenceGrantInfo, ResourceQuotaInfo, RoleBindingInfo, RoleInfo,
                ServiceAccountInfo, ServiceInfo, StatefulSetInfo, StorageClassInfo,
            },
            helm::HelmHistoryResult,
            rollout::{RolloutInspection, RolloutRevisionInfo, RolloutWorkloadKind},
        },
        state::{ClusterSnapshot, DataPhase, ViewLoadState},
        time::{now, now_unix_seconds},
    };

    use super::*;

    static RENDER_INVALIDATION_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn draw(app: &AppState, snapshot: &ClusterSnapshot) {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| render(frame, app, snapshot))
            .expect("render should not panic");
    }

    fn draw_in_terminal(
        terminal: &mut Terminal<TestBackend>,
        app: &AppState,
        snapshot: &ClusterSnapshot,
    ) {
        terminal
            .draw(|frame| render(frame, app, snapshot))
            .expect("render should not panic");
    }

    fn draw_with_size(app: &AppState, snapshot: &ClusterSnapshot, width: u16, height: u16) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| render(frame, app, snapshot))
            .expect("render should not panic");
    }

    #[test]
    fn cursor_visible_input_line_keeps_input_tail_visible() {
        let line = cursor_visible_input_line(
            &[Span::raw(" / ")],
            "very-long-search-query-tail",
            Some("very-long-search-query-tail".chars().count()),
            Style::default(),
            Style::default(),
            &[Span::raw("")],
            12,
        );
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(text, " / ...-tail█");
    }

    #[test]
    fn cursor_visible_input_line_handles_tight_width_without_overflow() {
        let line = cursor_visible_input_line(
            &[Span::raw(" : ")],
            "abcdef",
            Some("abcdef".chars().count()),
            Style::default(),
            Style::default(),
            &[Span::raw("")],
            5,
        );
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(text.chars().count(), 5);
        assert_eq!(text, " : f█");
    }

    #[test]
    fn cursor_visible_input_line_keeps_middle_cursor_visible() {
        let line = cursor_visible_input_line(
            &[Span::raw(" / ")],
            "abcdefghi",
            Some(3),
            Style::default(),
            Style::default(),
            &[],
            9,
        );
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(text, " / bc█def");
    }

    #[test]
    fn cursor_visible_input_line_truncates_without_cursor() {
        let line = cursor_visible_input_line(
            &[],
            "abcdefghij",
            None,
            Style::default(),
            Style::default(),
            &[],
            7,
        );
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(text, "abcd...");
    }

    #[test]
    fn cursor_visible_input_line_handles_unicode_without_overflow() {
        let line = cursor_visible_input_line(
            &[Span::raw(" / ")],
            "åβcdefghijk",
            Some(6),
            Style::default(),
            Style::default(),
            &[],
            10,
        );
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(text.chars().count(), 10);
        assert_eq!(text, " / def█ghi");
    }

    fn terminal_to_string(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn render_to_string(app: &AppState, snapshot: &ClusterSnapshot) -> String {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        draw_in_terminal(&mut terminal, app, snapshot);
        terminal_to_string(&terminal)
    }

    fn render_to_string_with_size(
        app: &AppState,
        snapshot: &ClusterSnapshot,
        width: u16,
        height: u16,
    ) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        draw_in_terminal(&mut terminal, app, snapshot);
        terminal_to_string(&terminal)
    }

    fn cell_colors(terminal: &Terminal<TestBackend>, x: u16, y: u16) -> (Color, Color) {
        let cell = &terminal.backend().buffer()[(x, y)];
        (cell.fg, cell.bg)
    }

    fn cells_with_colors(
        terminal: &Terminal<TestBackend>,
        fg: Color,
        bg: Color,
    ) -> Vec<(u16, u16)> {
        let buffer = terminal.backend().buffer();
        let mut cells = Vec::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let cell = &buffer[(x, y)];
                if cell.fg == fg && cell.bg == bg {
                    cells.push((x, y));
                }
            }
        }
        cells
    }

    fn selected_lines(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let theme = default_theme();
        let buffer = terminal.backend().buffer();
        let mut lines = Vec::new();
        for y in 0..buffer.area.height {
            let selected = (0..buffer.area.width).any(|x| {
                let cell = &buffer[(x, y)];
                cell.fg == theme.selection_fg && cell.bg == theme.selection_bg
            });
            if selected {
                let mut line = String::new();
                for x in 0..buffer.area.width {
                    line.push_str(buffer[(x, y)].symbol());
                }
                lines.push(line);
            }
        }
        lines
    }

    fn pods_snapshot_for_render_tests() -> ClusterSnapshot {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.view_load_states[AppView::Pods.index()] = ViewLoadState::Ready;
        snapshot.pods.push(PodInfo {
            name: "alpha-pod".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });
        snapshot.pods.push(PodInfo {
            name: "beta-pod".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });
        snapshot
    }

    fn projects_snapshot_for_detail_scroll_tests() -> ClusterSnapshot {
        let labels = BTreeMap::from([(
            "app.kubernetes.io/part-of".to_string(),
            "checkout".to_string(),
        )]);
        let mut snapshot = ClusterSnapshot::default();
        snapshot.view_load_states[AppView::Projects.index()] = ViewLoadState::Ready;
        snapshot.pods.push(PodInfo {
            name: "checkout-pod".to_string(),
            namespace: "payments".to_string(),
            status: "Running".to_string(),
            labels: labels
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            ..PodInfo::default()
        });
        for idx in 0..4 {
            snapshot.deployments.push(DeploymentInfo {
                name: format!("checkout-worker-{idx}"),
                namespace: "payments".to_string(),
                pod_template_labels: labels.clone(),
                ..DeploymentInfo::default()
            });
            snapshot.services.push(ServiceInfo {
                name: format!("checkout-svc-{idx}"),
                namespace: "payments".to_string(),
                labels: labels.clone(),
                ..ServiceInfo::default()
            });
            snapshot.ingresses.push(IngressInfo {
                name: format!("checkout-ingress-{idx}"),
                namespace: "payments".to_string(),
                labels: labels.clone(),
                ..IngressInfo::default()
            });
        }
        snapshot
    }

    struct ThemeResetGuard(u8);

    impl Drop for ThemeResetGuard {
        fn drop(&mut self) {
            crate::ui::theme::set_active_theme(self.0);
        }
    }

    struct IconResetGuard(IconMode);

    impl Drop for IconResetGuard {
        fn drop(&mut self) {
            crate::icons::set_icon_mode(self.0);
        }
    }

    fn app_with_view(view: AppView) -> AppState {
        let mut app = AppState::default();
        while app.view() != view {
            app.handle_key_event(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Tab,
            ));
        }
        app
    }

    #[test]
    fn bookmarked_name_cell_marks_saved_resources() {
        let theme = default_theme();
        let resource = ResourceRef::Namespace("prod".to_string());
        let bookmarks = vec![BookmarkEntry {
            resource: resource.clone(),
            bookmarked_at_unix: 0,
        }];

        let bookmarked = format!(
            "{:?}",
            bookmarked_name_cell(
                || resource.clone(),
                &bookmarks,
                "prod",
                Style::default().fg(theme.fg),
                &theme,
            )
        );
        let plain = format!(
            "{:?}",
            bookmarked_name_cell(
                || resource.clone(),
                &[],
                "prod",
                Style::default().fg(theme.fg),
                &theme,
            )
        );

        assert!(bookmarked.contains("★ "));
        assert!(!plain.contains("★ "));
    }

    #[test]
    fn table_window_keeps_selected_visible_near_top() {
        let window = table_window(100, 0, 10);
        assert_eq!(window.start, 0);
        assert_eq!(window.end, 10);
        assert_eq!(window.selected, 0);
    }

    #[test]
    fn table_window_centers_selection_in_middle() {
        let window = table_window(100, 50, 11);
        assert_eq!(window.start, 45);
        assert_eq!(window.end, 56);
        assert_eq!(window.selected, 5);
    }

    #[test]
    fn table_window_clamps_selection_near_bottom() {
        let window = table_window(100, 99, 10);
        assert_eq!(window.start, 90);
        assert_eq!(window.end, 100);
        assert_eq!(window.selected, 9);
    }

    #[test]
    fn table_window_handles_empty_lists() {
        let window = table_window(0, 0, 10);
        assert_eq!(window.start, 0);
        assert_eq!(window.end, 0);
        assert_eq!(window.selected, 0);
    }

    #[test]
    fn table_viewport_rows_has_minimum_one_row() {
        let area = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 2,
        };
        assert_eq!(table_viewport_rows(area), 1);
    }

    #[test]
    fn responsive_table_widths_preserves_wide_layouts_when_space_allows() {
        let wide = [
            Constraint::Length(24),
            Constraint::Length(16),
            Constraint::Length(9),
            Constraint::Min(20),
        ];

        assert_eq!(responsive_table_widths(96, wide), wide);
    }

    #[test]
    fn responsive_table_widths_falls_back_to_percentages_that_fill_width() {
        let widths = responsive_table_widths(
            40,
            [
                Constraint::Length(24),
                Constraint::Length(16),
                Constraint::Length(9),
                Constraint::Min(20),
            ],
        );

        assert!(
            widths
                .iter()
                .all(|constraint| matches!(constraint, Constraint::Percentage(_)))
        );
        let total: u16 = widths
            .iter()
            .map(|constraint| match constraint {
                Constraint::Percentage(value) => *value,
                _ => 0,
            })
            .sum();
        assert_eq!(total, 100);
    }

    #[test]
    fn vertical_primary_detail_chunks_compact_on_short_height() {
        let (primary, detail) = vertical_primary_detail_chunks(Rect::new(0, 0, 90, 18), 60, 8, 24);
        assert_eq!(primary.height, 10);
        assert_eq!(detail.height, 8);
    }

    #[test]
    fn vertical_primary_detail_chunks_use_percentage_split_when_tall() {
        let (primary, detail) = vertical_primary_detail_chunks(Rect::new(0, 0, 90, 30), 60, 8, 24);
        assert_eq!(primary.height, 18);
        assert_eq!(detail.height, 12);
    }

    /// Verifies dashboard renders without panic for empty snapshot.
    #[test]
    fn render_dashboard_empty_snapshot_smoke() {
        let app = app_with_view(AppView::Dashboard);
        draw(&app, &ClusterSnapshot::default());
    }

    /// Verifies dashboard renders without panic for populated snapshot.
    #[test]
    fn render_dashboard_full_snapshot_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.nodes.push(NodeInfo {
            name: "n1".to_string(),
            ready: true,
            ..NodeInfo::default()
        });
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });
        snapshot.services.push(ServiceInfo {
            name: "svc".to_string(),
            namespace: "default".to_string(),
            type_: "ClusterIP".to_string(),
            ..ServiceInfo::default()
        });
        snapshot.deployments.push(DeploymentInfo {
            name: "dep".to_string(),
            namespace: "default".to_string(),
            ready: "1/1".to_string(),
            ..DeploymentInfo::default()
        });

        let app = app_with_view(AppView::Dashboard);
        draw(&app, &snapshot);
    }

    #[test]
    fn render_dashboard_narrow_width_uses_single_column_fallback() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.nodes.push(NodeInfo {
            name: "n1".to_string(),
            ready: true,
            ..NodeInfo::default()
        });
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });
        let app = app_with_view(AppView::Dashboard);
        let rendered = render_to_string_with_size(&app, &snapshot, 64, 24);

        assert!(rendered.contains("Cluster"));
        assert!(rendered.contains("Health Gauges"));
        assert!(rendered.contains("Alerts"));
    }

    #[test]
    fn sidebar_uses_unknown_count_placeholder_for_unloaded_scopes() {
        // Use Pods view so Workloads group is expanded and Pods is visible.
        let loaded_snapshot = ClusterSnapshot {
            loaded_scope: crate::state::RefreshScope::PODS,
            ..ClusterSnapshot::default()
        };
        let app = app_with_view(AppView::Pods);

        let loaded_text = render_to_string(&app, &loaded_snapshot);
        assert!(loaded_text.contains("Pods (0)"));

        let text = render_to_string(&app, &ClusterSnapshot::default());
        assert!(text.contains("Pods (…)"));
        assert!(!text.contains("Pods (0)"));
    }

    #[test]
    fn dashboard_metrics_state_is_explicit() {
        let mut loading_snapshot = ClusterSnapshot {
            phase: DataPhase::Ready,
            loaded_scope: crate::state::RefreshScope::CORE_OVERVIEW,
            ..ClusterSnapshot::default()
        };
        loading_snapshot.nodes.push(NodeInfo {
            name: "node-a".to_string(),
            ready: true,
            ..NodeInfo::default()
        });
        loading_snapshot.pods.push(PodInfo {
            name: "pod-a".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });

        let mut app = app_with_view(AppView::Dashboard);
        let loading_text = render_to_string(&app, &loading_snapshot);
        assert!(loading_text.contains("loading..."));

        app.advance_spinner();
        let next_loading_text = render_to_string(&app, &loading_snapshot);
        assert!(next_loading_text.contains("loading..."));
        assert_ne!(loading_text, next_loading_text);

        let unavailable_snapshot = ClusterSnapshot {
            loaded_scope: crate::state::RefreshScope::CORE_OVERVIEW
                .union(crate::state::RefreshScope::METRICS),
            ..loading_snapshot
        };
        let unavailable_text =
            render_to_string(&app_with_view(AppView::Dashboard), &unavailable_snapshot);
        assert!(unavailable_text.contains("Metrics   unavailable"));
    }

    #[test]
    fn issues_view_marks_partial_coverage_until_backfill_finishes() {
        let snapshot = ClusterSnapshot {
            loaded_scope: crate::state::RefreshScope::CORE_OVERVIEW,
            pods: vec![PodInfo {
                name: "stuck-pod".to_string(),
                namespace: "default".to_string(),
                status: "Pending".to_string(),
                ..PodInfo::default()
            }],
            ..ClusterSnapshot::default()
        };
        let text = render_to_string(&app_with_view(AppView::Issues), &snapshot);
        assert!(text.contains("partial coverage"));
    }

    #[test]
    fn health_report_view_filters_out_runtime_issues() {
        let snapshot = ClusterSnapshot {
            snapshot_version: 42,
            nodes: vec![NodeInfo {
                name: "node-a".to_string(),
                ready: false,
                ..NodeInfo::default()
            }],
            config_maps: vec![ConfigMapInfo {
                name: "unused-config".to_string(),
                namespace: "default".to_string(),
                ..ConfigMapInfo::default()
            }],
            ..ClusterSnapshot::default()
        };

        let text = render_to_string(&app_with_view(AppView::HealthReport), &snapshot);
        assert!(text.contains("unused-config"));
        assert!(!text.contains("Node Not Ready"));
    }

    #[test]
    fn render_vulnerabilities_view_smoke() {
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 11,
            loaded_scope: crate::state::RefreshScope::SECURITY,
            ..ClusterSnapshot::default()
        };
        snapshot
            .vulnerability_reports
            .push(crate::k8s::dtos::VulnerabilityReportInfo {
                name: "api-web".to_string(),
                namespace: "default".to_string(),
                resource_kind: "Deployment".to_string(),
                resource_name: "api".to_string(),
                resource_namespace: "default".to_string(),
                container_name: Some("web".to_string()),
                artifact_repository: Some("ghcr.io/demo/api".to_string()),
                artifact_tag: Some("1.2.3".to_string()),
                counts: crate::k8s::dtos::VulnerabilitySummaryCounts {
                    critical: 1,
                    high: 2,
                    medium: 0,
                    low: 0,
                    unknown: 0,
                },
                fixable_count: 2,
                ..crate::k8s::dtos::VulnerabilityReportInfo::default()
            });

        let text = render_to_string(&app_with_view(AppView::Vulnerabilities), &snapshot);
        assert!(text.contains("Vulnerabilities"));
        assert!(text.contains("api"));
        assert!(text.contains("default"));
        assert!(text.contains(" 1 "));
    }

    /// Verifies nodes view renders without panic for multiple list sizes.
    #[test]
    fn render_nodes_various_sizes_smoke() {
        let app = app_with_view(AppView::Nodes);

        for size in [0, 1, 100, 1000] {
            let mut snapshot = ClusterSnapshot::default();
            for i in 0..size {
                snapshot.nodes.push(NodeInfo {
                    name: format!("node-{i}"),
                    ready: i % 2 == 0,
                    role: if i % 3 == 0 { "master" } else { "worker" }.to_string(),
                    ..NodeInfo::default()
                });
            }
            draw(&app, &snapshot);
        }
    }

    #[test]
    fn render_pods_narrow_width_smoke() {
        let app = app_with_view(AppView::Pods);
        let mut snapshot = ClusterSnapshot::default();
        snapshot.view_load_states[AppView::Pods.index()] = ViewLoadState::Ready;
        snapshot.pods.push(PodInfo {
            name: "redpanda-0".to_string(),
            namespace: "staging".to_string(),
            status: "Running".to_string(),
            node: Some("gke-luxor-staging-redp".to_string()),
            restarts: 0,
            ..PodInfo::default()
        });
        snapshot.pods.push(PodInfo {
            name: "redpanda-console-7dc45cb5d8-4482g".to_string(),
            namespace: "staging".to_string(),
            status: "Running".to_string(),
            node: Some("gke-luxor-staging-redp".to_string()),
            restarts: 2,
            ..PodInfo::default()
        });

        draw_with_size(&app, &snapshot, 96, 20);
    }

    #[test]
    fn render_invalidates_when_theme_changes_on_same_terminal() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        crate::ui::theme::set_active_theme(0);

        let app = app_with_view(AppView::Dashboard);
        let snapshot = ClusterSnapshot::default();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let before = cell_colors(&terminal, 0, 0);

        crate::ui::theme::set_active_theme(4);
        draw_in_terminal(&mut terminal, &app, &snapshot);
        let after = cell_colors(&terminal, 0, 0);

        assert_ne!(before, after);
    }

    #[test]
    fn render_repaints_sidebar_on_same_terminal_when_state_is_unchanged() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::ui::theme::set_active_theme(0);
        crate::icons::set_icon_mode(IconMode::Plain);

        let app = app_with_view(AppView::Pods);
        let snapshot = pods_snapshot_for_render_tests();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let before = terminal_to_string(&terminal);
        assert!(before.contains("Dashboard"));
        assert!(before.contains("Pods (2)"));

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let after = terminal_to_string(&terminal);

        assert!(after.contains("Dashboard"));
        assert!(after.contains("Pods (2)"));
        assert_eq!(before, after);
    }

    #[test]
    fn render_repaints_on_fresh_terminal_when_cached_state_matches() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::ui::theme::set_active_theme(0);
        crate::icons::set_icon_mode(IconMode::Plain);

        let app = app_with_view(AppView::Pods);
        let snapshot = pods_snapshot_for_render_tests();

        let mut first_terminal =
            Terminal::new(TestBackend::new(120, 40)).expect("first terminal should initialize");
        draw_in_terminal(&mut first_terminal, &app, &snapshot);
        let first = terminal_to_string(&first_terminal);
        assert!(first.contains("Dashboard"));
        assert!(first.contains("Pods (2)"));

        let mut second_terminal =
            Terminal::new(TestBackend::new(120, 40)).expect("second terminal should initialize");
        draw_in_terminal(&mut second_terminal, &app, &snapshot);
        let second = terminal_to_string(&second_terminal);

        assert!(second.contains("Dashboard"));
        assert!(second.contains("Pods (2)"));
        assert_eq!(first, second);
    }

    #[test]
    fn render_invalidates_when_icon_mode_changes_on_same_terminal() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::icons::set_icon_mode(IconMode::Nerd);

        let app = app_with_view(AppView::Pods);
        let snapshot = pods_snapshot_for_render_tests();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let before = terminal_to_string(&terminal);

        crate::icons::set_icon_mode(IconMode::Plain);
        draw_in_terminal(&mut terminal, &app, &snapshot);
        let after = terminal_to_string(&terminal);

        assert_ne!(before, after);
        assert!(after.contains("KubecTUI"));
    }

    #[test]
    fn render_invalidates_when_search_query_changes_on_same_terminal() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::ui::theme::set_active_theme(0);
        crate::icons::set_icon_mode(IconMode::Plain);

        let mut app = app_with_view(AppView::Pods);
        let snapshot = pods_snapshot_for_render_tests();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let before = terminal_to_string(&terminal);
        assert!(before.contains("Pods (2)"));
        assert!(!before.contains("No pods match the search query"));

        app.search_query = "nomatch".to_string();
        draw_in_terminal(&mut terminal, &app, &snapshot);
        let after = terminal_to_string(&terminal);

        assert!(after.contains("No pods match the search query"));
        assert_ne!(before, after);
    }

    #[test]
    fn render_invalidates_when_view_load_state_changes_on_same_terminal() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::ui::theme::set_active_theme(0);
        crate::icons::set_icon_mode(IconMode::Plain);

        let app = app_with_view(AppView::Pods);
        let mut snapshot = ClusterSnapshot::default();
        snapshot.view_load_states[AppView::Pods.index()] = ViewLoadState::Ready;
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let before = terminal_to_string(&terminal);
        assert!(before.contains("No pods available"));

        snapshot.view_load_states[AppView::Pods.index()] = ViewLoadState::Loading;
        draw_in_terminal(&mut terminal, &app, &snapshot);
        let after = terminal_to_string(&terminal);

        assert!(after.contains("Loading pods..."));
        assert_ne!(before, after);
    }

    #[test]
    fn centered_loading_message_animates_on_same_terminal() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::ui::theme::set_active_theme(0);
        crate::icons::set_icon_mode(IconMode::Plain);

        let mut app = app_with_view(AppView::Pods);
        let mut snapshot = ClusterSnapshot::default();
        snapshot.view_load_states[AppView::Pods.index()] = ViewLoadState::Loading;
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let before = terminal_to_string(&terminal);

        app.advance_spinner();
        draw_in_terminal(&mut terminal, &app, &snapshot);
        let after = terminal_to_string(&terminal);

        assert!(before.contains("Loading pods..."));
        assert!(after.contains("Loading pods..."));
        assert_ne!(before, after);
    }

    #[test]
    fn populated_resource_table_refresh_title_animates_on_same_terminal() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::ui::theme::set_active_theme(0);
        crate::icons::set_icon_mode(IconMode::Plain);

        let mut app = app_with_view(AppView::Pods);
        let mut snapshot = pods_snapshot_for_render_tests();
        snapshot.view_load_states[AppView::Pods.index()] = ViewLoadState::Refreshing;
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let before = terminal_to_string(&terminal);

        app.advance_spinner();
        draw_in_terminal(&mut terminal, &app, &snapshot);
        let after = terminal_to_string(&terminal);

        assert!(before.contains("Pods (2) ["));
        assert!(before.contains("refreshing]"));
        assert!(after.contains("Pods (2) ["));
        assert!(after.contains("refreshing]"));
        assert_ne!(before, after);
    }

    #[test]
    fn render_invalidates_when_selection_changes_on_same_terminal() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::ui::theme::set_active_theme(0);
        crate::icons::set_icon_mode(IconMode::Plain);

        let mut app = app_with_view(AppView::Pods);
        let snapshot = pods_snapshot_for_render_tests();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let theme = default_theme();
        let before = cells_with_colors(&terminal, theme.selection_fg, theme.selection_bg);
        assert!(
            !before.is_empty(),
            "selected row should use the selection style"
        );

        app.selected_idx = 1;
        draw_in_terminal(&mut terminal, &app, &snapshot);
        let after = cells_with_colors(&terminal, theme.selection_fg, theme.selection_bg);

        assert!(!after.is_empty(), "selected row should remain highlighted");
        assert_ne!(before, after);
    }

    #[test]
    fn render_invalidates_when_content_detail_scroll_changes_on_same_terminal() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::ui::theme::set_active_theme(0);
        crate::icons::set_icon_mode(IconMode::Plain);

        let mut app = app_with_view(AppView::Projects);
        app.focus = crate::app::Focus::Content;
        let snapshot = projects_snapshot_for_detail_scroll_tests();
        let backend = TestBackend::new(120, 18);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let before = terminal_to_string(&terminal);
        assert!(before.contains("Project Summary"));

        app.content_detail_scroll = 3;
        draw_in_terminal(&mut terminal, &app, &snapshot);
        let after = terminal_to_string(&terminal);

        assert!(after.contains("Project Summary"));
        assert_ne!(before, after);
    }

    #[test]
    fn render_invalidates_when_secondary_pane_focus_blurs_on_same_terminal() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::ui::theme::set_active_theme(0);
        crate::icons::set_icon_mode(IconMode::Plain);

        let mut app = app_with_view(AppView::Projects);
        app.focus = crate::app::Focus::Content;
        app.content_pane_focus = crate::app::ContentPaneFocus::Secondary;
        let snapshot = projects_snapshot_for_detail_scroll_tests();
        let mut terminal =
            Terminal::new(TestBackend::new(120, 18)).expect("test terminal should initialize");

        let content = Rect::new(
            effective_sidebar_width(120),
            3,
            120 - effective_sidebar_width(120),
            13,
        );
        let (_, summary_area) = vertical_primary_detail_chunks(content, 60, 8, 24);

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let active_summary_border = terminal.backend().buffer()[(summary_area.x, summary_area.y)]
            .style()
            .fg;
        assert_eq!(
            active_summary_border,
            default_theme().border_active_style().fg
        );

        app.focus = crate::app::Focus::Sidebar;
        draw_in_terminal(&mut terminal, &app, &snapshot);
        let blurred_summary_border = terminal.backend().buffer()[(summary_area.x, summary_area.y)]
            .style()
            .fg;

        assert_ne!(active_summary_border, blurred_summary_border);
        assert_eq!(blurred_summary_border, default_theme().border_style().fg);
    }

    #[test]
    fn effective_workbench_height_preserves_main_content_budget() {
        assert_eq!(effective_workbench_height(10, 12, true, false), 0);
        assert_eq!(effective_workbench_height(20, 12, true, false), 12);
        assert_eq!(effective_workbench_height(20, 16, true, false), 12);
        assert_eq!(effective_workbench_height(20, 12, false, false), 0);
        // Maximized takes full height
        assert_eq!(effective_workbench_height(40, 12, true, true), 40);
        assert_eq!(effective_workbench_height(20, 12, true, true), 20);
        // Maximized but closed still returns 0
        assert_eq!(effective_workbench_height(20, 12, false, true), 0);
    }

    #[test]
    fn effective_sidebar_width_preserves_content_budget_on_small_terminals() {
        assert_eq!(effective_sidebar_width(40), 16);
        assert_eq!(effective_sidebar_width(44), 20);
        assert_eq!(effective_sidebar_width(50), 26);
    }

    #[test]
    fn render_with_open_workbench_smoke() {
        let mut app = app_with_view(AppView::Dashboard);
        app.workbench.toggle_open();
        let snapshot = ClusterSnapshot::default();
        draw_with_size(&app, &snapshot, 120, 40);
    }

    #[test]
    fn status_bar_labels_sidebar_focus_even_when_error_is_visible() {
        let mut app = app_with_view(AppView::Pods);
        app.focus = Focus::Sidebar;
        app.set_error("boom".to_string());

        let rendered = render_to_string(&app, &pods_snapshot_for_render_tests());

        assert!(
            rendered.contains("[all] view: Pods • focus: sidebar • ERROR: boom"),
            "{rendered}"
        );
    }

    #[test]
    fn status_bar_labels_secondary_pane_focus_and_scroll_mode() {
        let mut app = app_with_view(AppView::Projects);
        app.focus = Focus::Content;
        app.content_pane_focus = crate::app::ContentPaneFocus::Secondary;

        let rendered = render_to_string(&app, &projects_snapshot_for_detail_scroll_tests());

        assert!(rendered.contains("[all] view: Projects • focus: secondary pane"));
        assert!(rendered.contains("[j/k] scroll"));
    }

    #[test]
    fn status_bar_labels_workbench_focus_even_when_status_message_is_visible() {
        let mut app = app_with_view(AppView::Dashboard);
        app.focus = Focus::Workbench;
        app.set_status("busy".to_string());

        let rendered = render_to_string(&app, &ClusterSnapshot::default());

        assert!(rendered.contains("[all] view: Dashboard • focus: workbench • busy"));
    }

    #[test]
    fn status_bar_repaints_on_fresh_terminal_when_cached_state_matches() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let mut app = app_with_view(AppView::Pods);
        app.focus = Focus::Content;
        let snapshot = pods_snapshot_for_render_tests();

        let first = render_to_string(&app, &snapshot);
        let second = render_to_string(&app, &snapshot);

        assert!(
            first.contains("[all] view: Pods • focus: resource list"),
            "{first}"
        );
        assert!(
            second.contains("[all] view: Pods • focus: resource list"),
            "{second}"
        );
    }

    #[test]
    fn render_helm_history_workbench_smoke() {
        let mut app = app_with_view(AppView::HelmReleases);
        let resource = ResourceRef::HelmRelease("web".to_string(), "default".to_string());
        app.open_helm_history_tab(
            resource,
            Some(HelmHistoryResult {
                cli_version: "v4.1.3".to_string(),
                revisions: vec![
                    HelmReleaseRevisionInfo {
                        revision: 7,
                        updated: "2026-03-25 10:15:00 +0700".to_string(),
                        status: "deployed".to_string(),
                        chart: "web-1.4.0".to_string(),
                        app_version: "2.1.0".to_string(),
                        description: "Upgrade complete".to_string(),
                    },
                    HelmReleaseRevisionInfo {
                        revision: 6,
                        updated: "2026-03-24 10:12:00 +0700".to_string(),
                        status: "superseded".to_string(),
                        chart: "web-1.3.0".to_string(),
                        app_version: "2.0.0".to_string(),
                        description: "Rollback complete".to_string(),
                    },
                ],
            }),
            None,
            None,
        );

        let snapshot = ClusterSnapshot::default();
        let rendered = render_to_string(&app, &snapshot);
        assert!(rendered.contains("Helm"));
        assert!(rendered.contains("rev   7"));
    }

    #[test]
    fn render_ai_analysis_workbench_smoke() {
        let mut app = app_with_view(AppView::Pods);
        let resource = ResourceRef::Pod("api-0".to_string(), "default".to_string());
        app.open_ai_analysis_tab(
            11,
            "Ask AI",
            resource,
            "AI",
            "gpt-test",
            vec!["Resource state 2 • Events 1 • Logs 1 • YAML redacted".to_string()],
        );
        if let Some(tab) = app.workbench_mut().active_tab_mut()
            && let crate::workbench::WorkbenchTabState::AiAnalysis(tab) = &mut tab.state
        {
            tab.apply_result(
                "AI",
                "gpt-test",
                "CrashLoopBackOff is likely caused by invalid configuration.".to_string(),
                vec!["Bad env var value".to_string()],
                vec!["Inspect Secret projection".to_string()],
                vec!["No recent logs were available".to_string()],
            );
        }

        let rendered = render_to_string(&app, &ClusterSnapshot::default());
        assert!(rendered.contains("CrashLoopBackOff"));
        assert!(rendered.contains("Likely Causes"));
    }

    #[test]
    fn render_loading_ai_analysis_shows_provider_identity() {
        let mut app = app_with_view(AppView::Pods);
        let resource = ResourceRef::Pod("api-0".to_string(), "default".to_string());
        app.open_ai_analysis_tab(
            11,
            "Explain Failure",
            resource,
            "Codex CLI",
            "codex-cli",
            vec![
                "Resource state 3 • Events unavailable • Logs 20 • YAML redacted • log gaps noted"
                    .to_string(),
            ],
        );

        let rendered = render_to_string(&app, &ClusterSnapshot::default());
        assert!(rendered.contains("running"));
        assert!(rendered.contains("Codex CLI"));
        assert!(rendered.contains("codex-cli"));
        assert!(rendered.contains("context"));
        assert!(rendered.contains("Events unavailable"));
        assert!(rendered.contains("Logs 20"));
        assert!(rendered.contains("YAML redacted"));
        assert!(rendered.contains("log gaps noted"));
    }

    #[test]
    fn loading_workbench_tabs_animate_on_same_terminal() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::ui::theme::set_active_theme(0);
        crate::icons::set_icon_mode(IconMode::Plain);

        let mut app = app_with_view(AppView::Pods);
        let resource = ResourceRef::Pod("api-0".to_string(), "default".to_string());
        app.open_ai_analysis_tab(
            11,
            "Explain Failure",
            resource,
            "Codex CLI",
            "codex-cli",
            Vec::new(),
        );
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        draw_in_terminal(&mut terminal, &app, &ClusterSnapshot::default());
        let before = terminal_to_string(&terminal);

        app.advance_spinner();
        draw_in_terminal(&mut terminal, &app, &ClusterSnapshot::default());
        let after = terminal_to_string(&terminal);

        assert!(before.contains("Running AI analysis..."));
        assert!(after.contains("Running AI analysis..."));
        assert_ne!(before, after);
    }

    #[test]
    fn background_loading_workbench_tab_title_animates_on_same_terminal() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::ui::theme::set_active_theme(0);
        crate::icons::set_icon_mode(IconMode::Plain);

        let mut app = app_with_view(AppView::Pods);
        let resource = ResourceRef::Pod("api-0".to_string(), "default".to_string());
        app.open_ai_analysis_tab(
            11,
            "Explain Failure",
            resource,
            "Codex CLI",
            "codex-cli",
            Vec::new(),
        );
        app.open_action_history_tab(true);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        draw_in_terminal(&mut terminal, &app, &ClusterSnapshot::default());
        let before = terminal_to_string(&terminal);

        app.advance_spinner();
        draw_in_terminal(&mut terminal, &app, &ClusterSnapshot::default());
        let after = terminal_to_string(&terminal);

        assert!(before.contains("AI Explain Failure"));
        assert!(after.contains("AI Explain Failure"));
        assert!(!before.contains("Running AI analysis..."));
        assert!(!after.contains("Running AI analysis..."));
        assert_ne!(before, after);
    }

    #[test]
    fn render_runbook_workbench_smoke() {
        let mut app = app_with_view(AppView::Pods);
        let resource = ResourceRef::Pod("api-0".to_string(), "default".to_string());
        app.open_runbook_tab(
            crate::runbooks::LoadedRunbook {
                id: "pod_failure".into(),
                title: "Pod Failure Triage".into(),
                description: Some("Deterministic pod checks.".into()),
                aliases: vec!["incident".into()],
                resource_kinds: vec!["Pod".into()],
                shortcut: None,
                steps: vec![
                    crate::runbooks::LoadedRunbookStep {
                        title: "Checklist".into(),
                        description: Some("Inspect the latest signals first.".into()),
                        kind: crate::runbooks::LoadedRunbookStepKind::Checklist {
                            items: vec!["Check events".into(), "Check probes".into()],
                        },
                    },
                    crate::runbooks::LoadedRunbookStep {
                        title: "Open logs".into(),
                        description: None,
                        kind: crate::runbooks::LoadedRunbookStepKind::DetailAction {
                            action: crate::runbooks::RunbookDetailAction::Logs,
                        },
                    },
                ],
            },
            Some(resource),
        );

        let rendered = render_to_string(&app, &ClusterSnapshot::default());
        assert!(rendered.contains("Runbook"));
        assert!(rendered.contains("Pod Failure Triage"));
        assert!(rendered.contains("Checklist"));
    }

    #[test]
    fn render_runbook_workbench_narrow_width_smoke() {
        let mut app = app_with_view(AppView::Pods);
        let resource = ResourceRef::Pod("api-0".to_string(), "default".to_string());
        app.open_runbook_tab(
            crate::runbooks::LoadedRunbook {
                id: "pod_failure".into(),
                title: "Pod Failure Triage".into(),
                description: Some("Deterministic pod checks.".into()),
                aliases: vec!["incident".into()],
                resource_kinds: vec!["Pod".into()],
                shortcut: None,
                steps: vec![crate::runbooks::LoadedRunbookStep {
                    title: "Checklist".into(),
                    description: Some("Inspect the latest signals first.".into()),
                    kind: crate::runbooks::LoadedRunbookStepKind::Checklist {
                        items: vec!["Check events".into(), "Check probes".into()],
                    },
                }],
            },
            Some(resource),
        );

        let rendered = render_to_string_with_size(&app, &ClusterSnapshot::default(), 72, 24);
        assert!(rendered.contains("Pod Failure Triage"));
        assert!(rendered.contains("Step Detail"));
    }

    #[test]
    fn render_connectivity_workbench_narrow_width_smoke() {
        let mut app = app_with_view(AppView::Pods);
        let source = ResourceRef::Pod("api-0".to_string(), "default".to_string());
        app.open_connectivity_tab(
            source,
            vec![crate::workbench::ConnectivityTargetOption {
                resource: ResourceRef::Pod("worker-0".to_string(), "default".to_string()),
                display: "default/worker-0".to_string(),
                status: "Running".to_string(),
                pod_ip: Some("10.0.0.8".to_string()),
            }],
        );

        let rendered = render_to_string_with_size(&app, &ClusterSnapshot::default(), 72, 24);
        assert!(rendered.contains("Targets"));
        assert!(rendered.contains("policy intent"));
    }

    #[test]
    fn render_rollout_workbench_smoke() {
        let mut app = app_with_view(AppView::Deployments);
        let resource = ResourceRef::Deployment("api".to_string(), "default".to_string());
        app.open_rollout_tab(
            resource,
            Some(RolloutInspection {
                kind: RolloutWorkloadKind::Deployment,
                strategy: "RollingUpdate".to_string(),
                paused: false,
                current_revision: Some(7),
                update_target_revision: Some(7),
                summary_lines: vec![
                    "Desired 3 · Updated 3 · Ready 3 · Available 3".to_string(),
                    "Observed generation 12".to_string(),
                ],
                conditions: Vec::new(),
                revisions: vec![
                    RolloutRevisionInfo {
                        revision: 7,
                        name: "api-7".to_string(),
                        created: Some("2026-03-25T10:15:00Z".to_string()),
                        summary: "3/3 ready".to_string(),
                        change_cause: Some("deploy api:7".to_string()),
                        is_current: true,
                        is_update_target: true,
                    },
                    RolloutRevisionInfo {
                        revision: 6,
                        name: "api-6".to_string(),
                        created: Some("2026-03-24T09:12:00Z".to_string()),
                        summary: "3/3 ready".to_string(),
                        change_cause: Some("deploy api:6".to_string()),
                        is_current: false,
                        is_update_target: false,
                    },
                ],
            }),
            None,
            None,
        );

        let snapshot = ClusterSnapshot::default();
        let rendered = render_to_string(&app, &snapshot);
        assert!(rendered.contains("Rollout"));
    }

    /// Verifies services view renders without panic for mixed service types.
    #[test]
    fn render_services_mixed_types_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        for t in ["ClusterIP", "NodePort", "LoadBalancer", "ExternalName"] {
            snapshot.services.push(ServiceInfo {
                name: format!("svc-{t}"),
                namespace: "default".to_string(),
                type_: t.to_string(),
                ports: vec!["80/TCP".to_string(), "443/TCP".to_string()],
                ..ServiceInfo::default()
            });
        }

        let app = app_with_view(AppView::Services);
        draw(&app, &snapshot);
    }

    /// Verifies deployments view renders without panic for mixed health values.
    #[test]
    fn render_deployments_mixed_health_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        for ready in ["3/3", "1/3", "0/3"] {
            snapshot.deployments.push(DeploymentInfo {
                name: format!("dep-{ready}"),
                namespace: "default".to_string(),
                ready: ready.to_string(),
                ..DeploymentInfo::default()
            });
        }

        let app = app_with_view(AppView::Deployments);
        draw(&app, &snapshot);
    }

    /// Verifies StatefulSets view renders without panic for mixed readiness states.
    #[test]
    fn render_statefulsets_mixed_readiness_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.statefulsets.push(StatefulSetInfo {
            name: "db-ready".to_string(),
            namespace: "default".to_string(),
            desired_replicas: 3,
            ready_replicas: 3,
            service_name: "db-headless".to_string(),
            image: Some("postgres:16".to_string()),
            ..StatefulSetInfo::default()
        });
        snapshot.statefulsets.push(StatefulSetInfo {
            name: "db-partial".to_string(),
            namespace: "default".to_string(),
            desired_replicas: 3,
            ready_replicas: 1,
            service_name: "db-headless".to_string(),
            image: Some("postgres:16".to_string()),
            ..StatefulSetInfo::default()
        });

        let app = app_with_view(AppView::StatefulSets);
        draw(&app, &snapshot);
    }

    /// Verifies DaemonSets view renders without panic for mixed desired/ready/unavailable counts.
    #[test]
    fn render_daemonsets_mixed_counts_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.daemonsets.push(DaemonSetInfo {
            name: "agent-ok".to_string(),
            namespace: "kube-system".to_string(),
            desired_count: 10,
            ready_count: 10,
            unavailable_count: 0,
            image: Some("agent:1".to_string()),
            ..DaemonSetInfo::default()
        });
        snapshot.daemonsets.push(DaemonSetInfo {
            name: "agent-warn".to_string(),
            namespace: "kube-system".to_string(),
            desired_count: 10,
            ready_count: 8,
            unavailable_count: 2,
            image: Some("agent:2".to_string()),
            ..DaemonSetInfo::default()
        });

        let app = app_with_view(AppView::DaemonSets);
        draw(&app, &snapshot);
    }

    /// Verifies Jobs view renders without panic for mixed status values.
    #[test]
    fn render_jobs_mixed_status_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.jobs.push(JobInfo {
            name: "batch-success".to_string(),
            namespace: "default".to_string(),
            status: "Succeeded".to_string(),
            completions: "1/1".to_string(),
            ..JobInfo::default()
        });
        snapshot.jobs.push(JobInfo {
            name: "batch-running".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            completions: "0/1".to_string(),
            ..JobInfo::default()
        });

        let app = app_with_view(AppView::Jobs);
        draw(&app, &snapshot);
    }

    /// Verifies CronJobs view renders without panic for suspended and active rows.
    #[test]
    fn render_cronjobs_suspend_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.cronjobs.push(CronJobInfo {
            name: "nightly".to_string(),
            namespace: "default".to_string(),
            schedule: "0 0 * * *".to_string(),
            suspend: false,
            ..CronJobInfo::default()
        });
        snapshot.cronjobs.push(CronJobInfo {
            name: "paused".to_string(),
            namespace: "default".to_string(),
            schedule: "*/15 * * * *".to_string(),
            suspend: true,
            ..CronJobInfo::default()
        });

        let app = app_with_view(AppView::CronJobs);
        draw(&app, &snapshot);
    }

    /// Verifies ResourceQuotas governance view renders usage bands without panic.
    #[test]
    fn render_resource_quotas_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.resource_quotas.push(ResourceQuotaInfo {
            name: "rq-default".to_string(),
            namespace: "default".to_string(),
            percent_used: [("pods".to_string(), 85.0)].into_iter().collect(),
            ..ResourceQuotaInfo::default()
        });

        let app = app_with_view(AppView::ResourceQuotas);
        draw(&app, &snapshot);
    }

    /// Verifies LimitRanges governance view renders limits summary without panic.
    #[test]
    fn render_limit_ranges_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.limit_ranges.push(LimitRangeInfo {
            name: "limits-default".to_string(),
            namespace: "default".to_string(),
            ..LimitRangeInfo::default()
        });

        let app = app_with_view(AppView::LimitRanges);
        draw(&app, &snapshot);
    }

    /// Verifies PDB governance view renders disruption stats without panic.
    #[test]
    fn render_pdbs_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot
            .pod_disruption_budgets
            .push(PodDisruptionBudgetInfo {
                name: "web-pdb".to_string(),
                namespace: "default".to_string(),
                min_available: Some("1".to_string()),
                disruptions_allowed: 1,
                ..PodDisruptionBudgetInfo::default()
            });

        let app = app_with_view(AppView::PodDisruptionBudgets);
        draw(&app, &snapshot);
    }

    /// Verifies ServiceAccounts view renders without panic.
    #[test]
    fn render_service_accounts_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.service_accounts.push(ServiceAccountInfo {
            name: "default".to_string(),
            namespace: "default".to_string(),
            secrets_count: 1,
            image_pull_secrets_count: 0,
            automount_service_account_token: Some(true),
            ..ServiceAccountInfo::default()
        });

        let app = app_with_view(AppView::ServiceAccounts);
        draw(&app, &snapshot);
    }

    /// Verifies network-related views render without panic.
    #[test]
    fn render_network_views_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.ingresses.push(IngressInfo {
            name: "web".to_string(),
            namespace: "default".to_string(),
            class: Some("nginx".to_string()),
            hosts: vec!["app.example.test".to_string()],
            address: Some("10.0.0.10".to_string()),
            ports: vec!["80".to_string(), "443".to_string()],
            ..IngressInfo::default()
        });
        snapshot.ingress_classes.push(IngressClassInfo {
            name: "nginx".to_string(),
            controller: "k8s.io/ingress-nginx".to_string(),
            is_default: true,
            ..IngressClassInfo::default()
        });
        snapshot.gateway_classes.push(GatewayClassInfo {
            name: "istio".to_string(),
            version: "v1".to_string(),
            controller_name: "istio.io/gateway-controller".to_string(),
            accepted: Some(true),
            ..GatewayClassInfo::default()
        });
        snapshot.gateways.push(GatewayInfo {
            name: "edge".to_string(),
            namespace: "default".to_string(),
            version: "v1".to_string(),
            gateway_class_name: "istio".to_string(),
            addresses: vec!["10.0.0.11".to_string()],
            listeners: vec![crate::k8s::dtos::GatewayListenerInfo {
                name: "http".to_string(),
                protocol: "HTTP".to_string(),
                port: 80,
                hostname: Some("app.example.test".to_string()),
                allowed_routes_from: Some("All".to_string()),
                allowed_routes_selector: None,
                attached_routes: 1,
                ready: Some(true),
            }],
            ..GatewayInfo::default()
        });
        snapshot.http_routes.push(HttpRouteInfo {
            name: "frontend".to_string(),
            namespace: "default".to_string(),
            version: "v1".to_string(),
            hostnames: vec!["app.example.test".to_string()],
            parent_refs: vec![crate::k8s::dtos::GatewayParentRefInfo {
                group: "gateway.networking.k8s.io".to_string(),
                kind: "Gateway".to_string(),
                name: "edge".to_string(),
                namespace: Some("default".to_string()),
                section_name: Some("http".to_string()),
            }],
            backend_refs: vec![crate::k8s::dtos::GatewayBackendRefInfo {
                group: "".to_string(),
                kind: "Service".to_string(),
                name: "frontend".to_string(),
                namespace: Some("default".to_string()),
                port: Some(8080),
            }],
            rule_count: 1,
            ..HttpRouteInfo::default()
        });
        snapshot.grpc_routes.push(GrpcRouteInfo {
            name: "grpc-api".to_string(),
            namespace: "default".to_string(),
            version: "v1".to_string(),
            parent_refs: vec![crate::k8s::dtos::GatewayParentRefInfo {
                group: "gateway.networking.k8s.io".to_string(),
                kind: "Gateway".to_string(),
                name: "edge".to_string(),
                namespace: Some("default".to_string()),
                section_name: Some("grpc".to_string()),
            }],
            backend_refs: vec![crate::k8s::dtos::GatewayBackendRefInfo {
                group: "".to_string(),
                kind: "Service".to_string(),
                name: "grpc-api".to_string(),
                namespace: Some("default".to_string()),
                port: Some(9090),
            }],
            rule_count: 1,
            ..GrpcRouteInfo::default()
        });
        snapshot.reference_grants.push(ReferenceGrantInfo {
            name: "allow-cross-namespace".to_string(),
            namespace: "default".to_string(),
            version: "v1beta1".to_string(),
            from: vec![crate::k8s::dtos::ReferenceGrantFromInfo {
                group: "gateway.networking.k8s.io".to_string(),
                kind: "HTTPRoute".to_string(),
                namespace: "edge".to_string(),
            }],
            to: vec![crate::k8s::dtos::ReferenceGrantToInfo {
                group: "".to_string(),
                kind: "Service".to_string(),
                name: Some("frontend".to_string()),
            }],
            ..ReferenceGrantInfo::default()
        });
        snapshot.network_policies.push(NetworkPolicyInfo {
            name: "deny-all".to_string(),
            namespace: "default".to_string(),
            pod_selector: "app=web".to_string(),
            ingress_rules: 0,
            egress_rules: 0,
            ..NetworkPolicyInfo::default()
        });

        draw(&app_with_view(AppView::Ingresses), &snapshot);
        draw(&app_with_view(AppView::IngressClasses), &snapshot);
        draw(&app_with_view(AppView::GatewayClasses), &snapshot);
        draw(&app_with_view(AppView::Gateways), &snapshot);
        draw(&app_with_view(AppView::HttpRoutes), &snapshot);
        draw(&app_with_view(AppView::GrpcRoutes), &snapshot);
        draw(&app_with_view(AppView::ReferenceGrants), &snapshot);
        draw(&app_with_view(AppView::NetworkPolicies), &snapshot);
    }

    /// Verifies storage-related views render without panic.
    #[test]
    fn render_storage_views_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.pvcs.push(PvcInfo {
            name: "data-web-0".to_string(),
            namespace: "default".to_string(),
            status: "Bound".to_string(),
            volume: Some("pv-web-0".to_string()),
            capacity: Some("10Gi".to_string()),
            access_modes: vec!["ReadWriteOnce".to_string()],
            storage_class: Some("fast-ssd".to_string()),
            ..PvcInfo::default()
        });
        snapshot.pvs.push(PvInfo {
            name: "pv-web-0".to_string(),
            capacity: Some("10Gi".to_string()),
            access_modes: vec!["ReadWriteOnce".to_string()],
            reclaim_policy: "Delete".to_string(),
            status: "Bound".to_string(),
            claim: Some("default/data-web-0".to_string()),
            storage_class: Some("fast-ssd".to_string()),
            ..PvInfo::default()
        });
        snapshot.storage_classes.push(StorageClassInfo {
            name: "fast-ssd".to_string(),
            provisioner: "kubernetes.io/no-provisioner".to_string(),
            reclaim_policy: Some("Delete".to_string()),
            volume_binding_mode: Some("WaitForFirstConsumer".to_string()),
            allow_volume_expansion: true,
            is_default: true,
            ..StorageClassInfo::default()
        });

        draw(&app_with_view(AppView::PersistentVolumeClaims), &snapshot);
        draw(&app_with_view(AppView::PersistentVolumes), &snapshot);
        draw(&app_with_view(AppView::StorageClasses), &snapshot);
    }

    /// Verifies Roles view renders rule details without panic.
    #[test]
    fn render_roles_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.roles.push(RoleInfo {
            name: "reader".to_string(),
            namespace: "default".to_string(),
            ..RoleInfo::default()
        });

        let app = app_with_view(AppView::Roles);
        draw(&app, &snapshot);
    }

    /// Verifies RoleBindings view renders subject details without panic.
    #[test]
    fn render_role_bindings_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.role_bindings.push(RoleBindingInfo {
            name: "reader-binding".to_string(),
            namespace: "default".to_string(),
            role_ref_kind: "Role".to_string(),
            role_ref_name: "reader".to_string(),
            ..RoleBindingInfo::default()
        });

        let app = app_with_view(AppView::RoleBindings);
        draw(&app, &snapshot);
    }

    /// Verifies ClusterRoles and ClusterRoleBindings views render without panic.
    #[test]
    fn render_cluster_rbac_views_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.cluster_roles.push(ClusterRoleInfo {
            name: "cluster-admin".to_string(),
            ..ClusterRoleInfo::default()
        });
        snapshot.cluster_role_bindings.push(ClusterRoleBindingInfo {
            name: "cluster-admin-binding".to_string(),
            role_ref_kind: "ClusterRole".to_string(),
            role_ref_name: "cluster-admin".to_string(),
            ..ClusterRoleBindingInfo::default()
        });

        let app_roles = app_with_view(AppView::ClusterRoles);
        draw(&app_roles, &snapshot);

        let app_bindings = app_with_view(AppView::ClusterRoleBindings);
        draw(&app_bindings, &snapshot);
    }

    #[test]
    fn render_extensions_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot
            .custom_resource_definitions
            .push(CustomResourceDefinitionInfo {
                name: "widgets.demo.io".to_string(),
                group: "demo.io".to_string(),
                version: "v1".to_string(),
                kind: "Widget".to_string(),
                plural: "widgets".to_string(),
                scope: "Namespaced".to_string(),
                instances: 1,
            });

        let mut app = app_with_view(AppView::Extensions);
        app.extension_instances = vec![CustomResourceInfo {
            name: "sample".to_string(),
            namespace: Some("default".to_string()),
            ..CustomResourceInfo::default()
        }];

        draw(&app, &snapshot);
    }

    #[test]
    fn render_extensions_narrow_width_stacks_panes() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot
            .custom_resource_definitions
            .push(CustomResourceDefinitionInfo {
                name: "widgets.demo.io".to_string(),
                group: "demo.io".to_string(),
                version: "v1".to_string(),
                kind: "Widget".to_string(),
                plural: "widgets".to_string(),
                scope: "Namespaced".to_string(),
                instances: 1,
            });

        let mut app = app_with_view(AppView::Extensions);
        app.extension_instances = vec![CustomResourceInfo {
            name: "sample".to_string(),
            namespace: Some("default".to_string()),
            ..CustomResourceInfo::default()
        }];

        let rendered = render_to_string_with_size(&app, &snapshot, 56, 24);
        assert!(rendered.contains("CRDs"));
        assert!(rendered.contains("Custom Resources"));
        assert!(rendered.contains("sample"));
    }

    #[test]
    fn extensions_render_cache_invalidates_when_instances_finish_loading() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::ui::theme::set_active_theme(0);
        crate::icons::set_icon_mode(IconMode::Plain);

        let mut snapshot = ClusterSnapshot::default();
        snapshot
            .custom_resource_definitions
            .push(CustomResourceDefinitionInfo {
                name: "widgets.demo.io".to_string(),
                group: "demo.io".to_string(),
                version: "v1".to_string(),
                kind: "Widget".to_string(),
                plural: "widgets".to_string(),
                scope: "Namespaced".to_string(),
                instances: 1,
            });

        let mut app = app_with_view(AppView::Extensions);
        app.extension_selected_crd = Some("widgets.demo.io".to_string());
        app.extension_instances_loading = true;
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let before = terminal_to_string(&terminal);
        assert!(before.contains("Loading instances for widgets.demo.io..."));

        app.extension_instances_loading = false;
        app.extension_instances = vec![CustomResourceInfo {
            name: "sample".to_string(),
            namespace: Some("default".to_string()),
            ..CustomResourceInfo::default()
        }];

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let after = terminal_to_string(&terminal);
        assert!(after.contains("sample"));
        assert_ne!(before, after);
    }

    #[test]
    fn port_forward_render_cache_invalidates_when_tunnel_registry_changes() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::ui::theme::set_active_theme(0);
        crate::icons::set_icon_mode(IconMode::Plain);

        let mut app = app_with_view(AppView::PortForwarding);
        let snapshot = ClusterSnapshot::default();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let before = terminal_to_string(&terminal);
        assert!(before.contains("No active port forwards"));

        app.tunnel_registry
            .add_tunnel(crate::k8s::portforward::PortForwardTunnelInfo {
                id: "default/api-0/8080".to_string(),
                target: crate::k8s::portforward::PortForwardTarget::new("default", "api-0", 8080),
                local_addr: "127.0.0.1:18080".parse().expect("socket addr"),
                state: crate::k8s::portforward::TunnelState::Active,
            });

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let after = terminal_to_string(&terminal);
        assert!(after.contains("api-0"));
        assert_ne!(before, after);
    }

    /// Verifies detail modal overlay renders on top of list view without panic.
    #[test]
    fn render_detail_overlay_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });

        let mut app = app_with_view(AppView::Pods);
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Pod("p1".to_string(), "default".to_string())),
            metadata: DetailMetadata {
                name: "p1".to_string(),
                namespace: Some("default".to_string()),
                ..DetailMetadata::default()
            },
            yaml: Some("kind: Pod\nmetadata:\n  name: p1\n".to_string()),
            ..DetailViewState::default()
        });

        draw(&app, &snapshot);
    }

    #[test]
    fn detail_loading_badge_animates_on_same_terminal() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::ui::theme::set_active_theme(0);
        crate::icons::set_icon_mode(IconMode::Plain);

        let mut snapshot = ClusterSnapshot::default();
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });

        let mut app = app_with_view(AppView::Pods);
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Pod("p1".to_string(), "default".to_string())),
            metadata: DetailMetadata {
                name: "p1".to_string(),
                namespace: Some("default".to_string()),
                ..DetailMetadata::default()
            },
            yaml: Some("kind: Pod\nmetadata:\n  name: p1\n".to_string()),
            loading: true,
            ..DetailViewState::default()
        });
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let before = terminal_to_string(&terminal);

        app.advance_spinner();
        draw_in_terminal(&mut terminal, &app, &snapshot);
        let after = terminal_to_string(&terminal);

        assert!(before.contains("Loading..."));
        assert!(after.contains("Loading..."));
        assert_ne!(before, after);
    }

    #[test]
    fn closing_help_overlay_repaints_underlying_view() {
        let snapshot = pods_snapshot_for_render_tests();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
        let app = app_with_view(AppView::Pods);
        draw_in_terminal(&mut terminal, &app, &snapshot);

        let mut with_help = app.clone();
        with_help.help_overlay.open();
        draw_in_terminal(&mut terminal, &with_help, &snapshot);
        let help_render = terminal_to_string(&terminal);
        assert!(help_render.contains("Keybindings"));

        draw_in_terminal(&mut terminal, &app, &snapshot);
        let closed_render = terminal_to_string(&terminal);
        assert!(closed_render.contains("Pods (2)"));
        assert!(!closed_render.contains("Keybindings"));
    }

    #[test]
    fn render_detail_overlay_uses_compact_layout_on_small_terminal() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });

        let mut app = app_with_view(AppView::Pods);
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Pod("p1".to_string(), "default".to_string())),
            metadata: DetailMetadata {
                name: "p1".to_string(),
                namespace: Some("default".to_string()),
                status: Some("Running".to_string()),
                ..DetailMetadata::default()
            },
            yaml: Some("kind: Pod\nmetadata:\n  name: p1\n".to_string()),
            ..DetailViewState::default()
        });

        let rendered = render_to_string_with_size(&app, &snapshot, 40, 10);
        assert!(!rendered.contains("Need at least 40x10"));
        assert!(rendered.contains("Expand terminal for full metadata"));
    }

    /// Verifies Extensions view renders with instance selection cursor without panic.
    #[test]
    fn render_extensions_with_instance_focus_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot
            .custom_resource_definitions
            .push(CustomResourceDefinitionInfo {
                name: "widgets.demo.io".to_string(),
                group: "demo.io".to_string(),
                version: "v1".to_string(),
                kind: "Widget".to_string(),
                plural: "widgets".to_string(),
                scope: "Namespaced".to_string(),
                instances: 2,
            });

        let mut app = app_with_view(AppView::Extensions);
        app.set_extension_instances(
            "widgets.demo.io".to_string(),
            vec![
                CustomResourceInfo {
                    name: "alpha".to_string(),
                    namespace: Some("default".to_string()),
                    ..CustomResourceInfo::default()
                },
                CustomResourceInfo {
                    name: "beta".to_string(),
                    namespace: Some("staging".to_string()),
                    ..CustomResourceInfo::default()
                },
            ],
            None,
        );
        app.extension_in_instances = true;
        app.extension_instance_cursor = 1;

        draw(&app, &snapshot);
    }

    /// Verifies Helm repositories view renders without panic.
    #[test]
    fn render_helm_repos_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot
            .helm_repositories
            .push(crate::k8s::dtos::HelmRepoInfo {
                name: "bitnami".to_string(),
                url: "https://charts.bitnami.com/bitnami".to_string(),
            });

        let app = app_with_view(AppView::HelmCharts);
        draw(&app, &snapshot);
    }

    /// Verifies Helm repos view renders empty state without panic.
    #[test]
    fn render_helm_repos_empty_smoke() {
        let app = app_with_view(AppView::HelmCharts);
        draw(&app, &ClusterSnapshot::default());
    }

    #[test]
    fn helm_repositories_empty_state_is_not_stuck_loading_once_local_data_is_ready() {
        let snapshot = ClusterSnapshot {
            loaded_scope: crate::state::RefreshScope::LOCAL_HELM_REPOSITORIES,
            view_load_states: {
                let mut states = [ViewLoadState::Idle; AppView::COUNT];
                states[AppView::HelmCharts.index()] = ViewLoadState::Ready;
                states
            },
            ..ClusterSnapshot::default()
        };
        let text = render_to_string(&app_with_view(AppView::HelmCharts), &snapshot);
        assert!(text.contains("No Helm repositories configured"));
        assert!(!text.contains("Loading Helm repositories"));
    }

    /// Verifies FluxCD "all" view renders without panic.
    #[test]
    fn render_fluxcd_all_view_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.flux_resources.push(FluxResourceInfo {
            name: "apps".to_string(),
            namespace: Some("flux-system".to_string()),
            kind: "Kustomization".to_string(),
            group: "kustomize.toolkit.fluxcd.io".to_string(),
            status: "Ready".to_string(),
            message: Some("Applied revision main@sha1:abc123".to_string()),
            ..FluxResourceInfo::default()
        });

        let app = app_with_view(AppView::FluxCDAll);
        draw(&app, &snapshot);
    }

    #[test]
    fn render_flux_selected_row_stays_visible_after_far_reorder() {
        let _render_lock = RENDER_INVALIDATION_TEST_LOCK
            .lock()
            .expect("lock should not poison");
        let _theme_guard = ThemeResetGuard(crate::ui::theme::active_theme_index());
        let _icon_mode_lock = crate::icons::icon_mode_test_lock();
        let _icon_guard = IconResetGuard(crate::icons::active_icon_mode());
        crate::ui::theme::set_active_theme(0);
        crate::icons::set_icon_mode(IconMode::Plain);

        fn flux_snapshot(version: u64, target_idx: usize) -> ClusterSnapshot {
            let mut names = (0..60)
                .filter(|idx| *idx != 50)
                .map(|idx| format!("resource-{idx:02}"))
                .collect::<Vec<_>>();
            names.insert(target_idx.min(names.len()), "resource-50".to_string());
            ClusterSnapshot {
                snapshot_version: version,
                flux_resources: names
                    .into_iter()
                    .map(|name| FluxResourceInfo {
                        name,
                        namespace: Some("flux-system".to_string()),
                        kind: "Kustomization".to_string(),
                        group: "kustomize.toolkit.fluxcd.io".to_string(),
                        version: "v1".to_string(),
                        plural: "kustomizations".to_string(),
                        status: "Ready".to_string(),
                        message: Some("Applied".to_string()),
                        ..FluxResourceInfo::default()
                    })
                    .collect(),
                ..ClusterSnapshot::default()
            }
        }

        let mut app = app_with_view(AppView::FluxCDKustomizations);
        app.focus = Focus::Content;
        app.selected_idx = 50;
        let backend = TestBackend::new(120, 10);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        draw_in_terminal(&mut terminal, &app, &flux_snapshot(1, 50));
        let selected_before = selected_lines(&terminal);
        assert!(
            selected_before
                .iter()
                .any(|line| line.contains("resource-50")),
            "selected row should be visible before reorder: {selected_before:?}"
        );

        app.selected_idx = 2;
        draw_in_terminal(&mut terminal, &app, &flux_snapshot(2, 2));
        let selected_after = selected_lines(&terminal);
        assert!(
            selected_after
                .iter()
                .any(|line| line.contains("resource-50")),
            "selected row should be visible after reorder: {selected_after:?}"
        );
        assert!(
            selected_after
                .iter()
                .all(|line| !line.contains("resource-02")),
            "highlight should not flash to raw-index neighbor: {selected_after:?}"
        );
    }

    /// Verifies detail overlay renders for a CustomResource without panic.
    #[test]
    fn render_detail_custom_resource_smoke() {
        let snapshot = ClusterSnapshot::default();
        let mut app = app_with_view(AppView::Extensions);
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::CustomResource {
                name: "my-widget".to_string(),
                namespace: Some("default".to_string()),
                group: "demo.io".to_string(),
                version: "v1".to_string(),
                kind: "Widget".to_string(),
                plural: "widgets".to_string(),
            }),
            metadata: DetailMetadata {
                name: "my-widget".to_string(),
                namespace: Some("default".to_string()),
                status: Some("Widget.demo.io".to_string()),
                ..DetailMetadata::default()
            },
            yaml: Some(
                "apiVersion: demo.io/v1\nkind: Widget\nmetadata:\n  name: my-widget\n".to_string(),
            ),
            sections: vec![
                "CUSTOM RESOURCE".to_string(),
                "kind: Widget".to_string(),
                "apiVersion: demo.io/v1".to_string(),
            ],
            ..DetailViewState::default()
        });

        draw(&app, &snapshot);
    }

    /// Verifies detail overlay renders for a HelmRelease without panic.
    #[test]
    fn render_detail_helm_release_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot
            .helm_releases
            .push(crate::k8s::dtos::HelmReleaseInfo {
                name: "my-app".to_string(),
                namespace: "default".to_string(),
                chart: "nginx".to_string(),
                chart_version: "15.0.0".to_string(),
                status: "deployed".to_string(),
                revision: 3,
                ..crate::k8s::dtos::HelmReleaseInfo::default()
            });

        let mut app = app_with_view(AppView::HelmReleases);
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::HelmRelease(
                "my-app".to_string(),
                "default".to_string(),
            )),
            metadata: DetailMetadata {
                name: "my-app".to_string(),
                namespace: Some("default".to_string()),
                status: Some("deployed".to_string()),
                ..DetailMetadata::default()
            },
            yaml: Some(
                "apiVersion: v1\nkind: Secret\nmetadata:\n  name: sh.helm.release.v1.my-app.v3\n"
                    .to_string(),
            ),
            ..DetailViewState::default()
        });

        draw(&app, &snapshot);
    }

    #[test]
    fn render_detail_cronjob_history_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.cronjobs.push(CronJobInfo {
            name: "nightly".to_string(),
            namespace: "ops".to_string(),
            schedule: "0 2 * * *".to_string(),
            ..CronJobInfo::default()
        });

        let mut app = app_with_view(AppView::CronJobs);
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::CronJob(
                "nightly".to_string(),
                "ops".to_string(),
            )),
            metadata: DetailMetadata {
                name: "nightly".to_string(),
                namespace: Some("ops".to_string()),
                status: Some("Active".to_string()),
                cronjob_suspended: Some(false),
                ..DetailMetadata::default()
            },
            yaml: Some("kind: CronJob\nmetadata:\n  name: nightly\n".to_string()),
            cronjob_history: vec![crate::cronjob::CronJobHistoryEntry {
                job_name: "nightly-001".to_string(),
                namespace: "ops".to_string(),
                status: "Failed".to_string(),
                completions: "0/1".to_string(),
                duration: Some("4s".to_string()),
                pod_count: 1,
                live_pod_count: 1,
                completion_pct: Some(0),
                active_pods: 0,
                failed_pods: 1,
                age: None,
                created_at: None,
                logs_authorized: None,
            }],
            ..DetailViewState::default()
        });

        draw(&app, &snapshot);
    }

    #[test]
    fn pod_derived_cache_separates_sort_variants() {
        let now = now();
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 9_871,
            ..ClusterSnapshot::default()
        };
        snapshot.pods.push(crate::k8s::dtos::PodInfo {
            name: "a".to_string(),
            created_at: Some(now.checked_sub(5.minutes()).expect("timestamp in range")),
            ..crate::k8s::dtos::PodInfo::default()
        });
        snapshot.pods.push(crate::k8s::dtos::PodInfo {
            name: "b".to_string(),
            created_at: Some(now.checked_sub(1.minute()).expect("timestamp in range")),
            ..crate::k8s::dtos::PodInfo::default()
        });

        let now_unix = now_unix_seconds();
        let first = cached_pod_derived(&snapshot, "variant-cache-test", &[0, 1], now_unix, 0);
        let second = cached_pod_derived(&snapshot, "variant-cache-test", &[1, 0], now_unix, 1);

        assert_eq!(
            first[0].age,
            format_age_from_timestamp(snapshot.pods[0].created_at, now_unix)
        );
        assert_eq!(
            second[0].age,
            format_age_from_timestamp(snapshot.pods[1].created_at, now_unix)
        );
    }

    #[test]
    fn utilization_bar_zero_percent() {
        let theme = super::components::default_theme();
        let line = utilization_bar(0, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("░░░░░░░░"));
        assert!(text.contains("0%"));
    }

    #[test]
    fn utilization_bar_hundred_percent() {
        let theme = super::components::default_theme();
        let line = utilization_bar(100, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("▓▓▓▓▓▓▓▓"));
        assert!(text.contains("100%"));
    }

    #[test]
    fn utilization_bar_clamps_above_100() {
        let theme = super::components::default_theme();
        let line = utilization_bar(200, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("100%"));
        assert!(text.contains("▓▓▓▓▓▓▓▓"));
    }

    #[test]
    fn utilization_bar_fifty_percent_fills_half() {
        let theme = super::components::default_theme();
        let line = utilization_bar(50, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        // 50% of 8 = 4 filled, 4 empty
        assert_eq!(text.matches('▓').count(), 4);
        assert_eq!(text.matches('░').count(), 4);
    }

    #[test]
    fn utilization_bar_labeled_includes_prefix() {
        let theme = super::components::default_theme();
        let line = utilization_bar_labeled("250m/4", 6, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("250m/4 "));
        assert!(text.contains("6%"));
    }

    #[test]
    fn resource_table_title_with_icon_no_query() {
        // Icon strings include their own trailing space
        assert_eq!(
            resource_table_title("🔌 ", "Services", 5, 10, "", " [Name ↑]"),
            " 🔌 Services (5) [Name ↑] "
        );
    }

    #[test]
    fn resource_table_title_with_icon_and_query() {
        assert_eq!(
            resource_table_title("🔌 ", "Services", 3, 10, "nginx", " [Name ↑]"),
            " 🔌 Services (3 of 10) [/nginx] [Name ↑]"
        );
    }

    #[test]
    fn resource_table_title_plain_mode_no_icon() {
        // Plain mode passes "" — no double space before label
        assert_eq!(
            resource_table_title("", "Deployments", 42, 42, "", ""),
            " Deployments (42) "
        );
    }

    #[test]
    fn resource_table_title_nerd_icon() {
        assert_eq!(
            resource_table_title("󰜟 ", "Deployments", 42, 42, "", ""),
            " 󰜟 Deployments (42) "
        );
    }

    #[test]
    fn wrap_span_groups_moves_whole_group_to_next_line() {
        let lines = wrap_span_groups(
            &[
                vec![Span::raw("[y] YAML  ")],
                vec![Span::raw("[D] Drift  ")],
                vec![Span::raw("[B] Bookmark")],
            ],
            22,
        );

        assert_eq!(lines.len(), 2);
        let first: String = lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        let second: String = lines[1]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(first, "[y] YAML  [D] Drift  ");
        assert_eq!(second, "[B] Bookmark");
    }
}
