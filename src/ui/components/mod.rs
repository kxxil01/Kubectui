//! Reusable UI widgets and building blocks.
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, LazyLock, Mutex},
};

pub mod command_palette;
pub mod context_picker;
pub mod debug_container_dialog;
pub mod help_overlay;
pub mod input_field;
pub mod namespace_picker;
pub mod port_forward_dialog;
pub mod probe_panel;
pub mod resource_template_dialog;
pub mod scale_dialog;
pub mod workbench;

pub use command_palette::{CommandPalette, CommandPaletteAction};
pub use context_picker::{ContextPicker, ContextPickerAction};
pub use debug_container_dialog::{
    DebugContainerDialogEvent, DebugContainerDialogState, DebugContainerField,
    render_debug_container_dialog,
};
pub use input_field::InputFieldWidget;
pub use namespace_picker::{NamespacePicker, NamespacePickerAction};
pub use port_forward_dialog::{FormField, PortForwardAction, PortForwardDialog, PortForwardMode};
pub use probe_panel::ProbePanelState;
pub use resource_template_dialog::{
    ResourceTemplateDialogState, ResourceTemplateField, render_resource_template_dialog,
};
pub use scale_dialog::{
    ScaleAction, ScaleDialogState, ScaleField, ScaleTargetKind, render_scale_dialog,
};
pub use workbench::render_workbench;

use ratatui::{
    layout::{Alignment, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Tabs},
};

use crate::{
    app::{AppView, NavGroup},
    state::ConnectionHealth,
    ui::theme::Theme,
};

const MAX_SIDEBAR_CACHE_ENTRIES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HeaderCacheKey {
    theme_index: u8,
    icon_mode: u8,
    title: String,
    cluster_meta: String,
    health: ConnectionHealth,
}

type HeaderCacheValue = Arc<Line<'static>>;
static HEADER_LINE_CACHE: LazyLock<Mutex<Option<(HeaderCacheKey, HeaderCacheValue)>>> =
    LazyLock::new(|| Mutex::new(None));

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct StatusBarCacheKey {
    theme_index: u8,
    message: String,
    is_error: bool,
}

type StatusBarCacheValue = Arc<Line<'static>>;
static STATUS_BAR_LINE_CACHE: LazyLock<Mutex<Option<(StatusBarCacheKey, StatusBarCacheValue)>>> =
    LazyLock::new(|| Mutex::new(None));

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SidebarCacheKey {
    theme_index: u8,
    icon_mode: u8,
    active: AppView,
    sidebar_cursor: usize,
    collapsed_mask: u16,
    sidebar_active: bool,
    counts_hash: u64,
}

type SidebarCacheValue = Arc<Vec<Line<'static>>>;

pub struct SidebarRenderData<'a> {
    pub collapsed: &'a HashSet<NavGroup>,
    pub focus: crate::app::Focus,
    pub counts_hash: u64,
    pub counts: &'a [(AppView, Option<usize>)],
}

#[derive(Debug, Default)]
struct SidebarLineCache {
    map: HashMap<SidebarCacheKey, SidebarCacheValue>,
    order: VecDeque<SidebarCacheKey>,
    last: Option<(SidebarCacheKey, SidebarCacheValue)>,
}

impl SidebarLineCache {
    fn get(&mut self, key: &SidebarCacheKey) -> Option<SidebarCacheValue> {
        if let Some((last_key, value)) = self.last.as_ref()
            && last_key == key
        {
            return Some(value.clone());
        }

        let value = self.map.get(key).cloned();
        if let Some(value) = value.as_ref() {
            self.last = Some((key.clone(), value.clone()));
        }
        value
    }

    fn insert(&mut self, key: SidebarCacheKey, value: SidebarCacheValue) {
        self.last = Some((key.clone(), value.clone()));
        if self.map.contains_key(&key) {
            self.map.insert(key.clone(), value);
            return;
        }

        self.map.insert(key.clone(), value);
        self.order.push_back(key);
        self.evict_if_needed();
    }

    fn evict_if_needed(&mut self) {
        while self.order.len() > MAX_SIDEBAR_CACHE_ENTRIES {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
    }
}

static SIDEBAR_LINE_CACHE: LazyLock<Mutex<SidebarLineCache>> =
    LazyLock::new(|| Mutex::new(SidebarLineCache::default()));

/// Global theme singleton — reads from the active theme setting.
pub fn default_theme() -> Theme {
    crate::ui::theme::active_theme()
}

#[inline]
fn nav_group_bit(group: NavGroup) -> u16 {
    match group {
        NavGroup::Overview => 1 << 0,
        NavGroup::Workloads => 1 << 1,
        NavGroup::Network => 1 << 2,
        NavGroup::Config => 1 << 3,
        NavGroup::Storage => 1 << 4,
        NavGroup::Helm => 1 << 5,
        NavGroup::FluxCD => 1 << 6,
        NavGroup::AccessControl => 1 << 7,
        NavGroup::CustomResources => 1 << 8,
    }
}

fn collapsed_mask(collapsed: &HashSet<NavGroup>) -> u16 {
    collapsed
        .iter()
        .fold(0u16, |mask, group| mask | nav_group_bit(*group))
}

fn cached_header_line(
    theme_index: u8,
    icon_mode: u8,
    title: &str,
    cluster_meta: &str,
    health: ConnectionHealth,
    theme: &Theme,
) -> HeaderCacheValue {
    if let Ok(cache) = HEADER_LINE_CACHE.lock()
        && let Some((cached_key, value)) = cache.as_ref()
        && cached_key.theme_index == theme_index
        && cached_key.icon_mode == icon_mode
        && cached_key.title == title
        && cached_key.cluster_meta == cluster_meta
        && cached_key.health == health
    {
        return value.clone();
    }

    let key = HeaderCacheKey {
        theme_index,
        icon_mode,
        title: title.to_string(),
        cluster_meta: cluster_meta.to_string(),
        health,
    };

    let title_style = theme.title_style();
    let dim_style = Style::default().fg(theme.fg_dim);
    let cluster_icon = crate::icons::chrome_icon("cluster").active();
    let cloud_icon = crate::icons::chrome_icon("cloud").active();
    let mut spans = vec![
        Span::styled(format!(" {cluster_icon}"), title_style),
        Span::styled(title.to_string(), title_style),
        Span::styled("  │  ", theme.muted_style()),
        Span::styled(cloud_icon.to_string(), dim_style),
        Span::styled(cluster_meta.to_string(), dim_style),
    ];

    match health {
        ConnectionHealth::Connected => {
            spans.push(Span::styled(" ●", Style::default().fg(theme.success)));
        }
        ConnectionHealth::Degraded(n) => {
            spans.push(Span::styled(
                format!(" ◐ {n} degraded"),
                Style::default().fg(theme.warning),
            ));
        }
        ConnectionHealth::Disconnected => {
            spans.push(Span::styled(
                " ○ disconnected",
                Style::default().fg(theme.error),
            ));
        }
        ConnectionHealth::Unknown => {}
    }

    let built = Arc::new(Line::from(spans));

    if let Ok(mut cache) = HEADER_LINE_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

fn cached_status_line(
    theme_index: u8,
    message: &str,
    is_error: bool,
    theme: &Theme,
) -> StatusBarCacheValue {
    if let Ok(cache) = STATUS_BAR_LINE_CACHE.lock()
        && let Some((cached_key, value)) = cache.as_ref()
        && cached_key.theme_index == theme_index
        && cached_key.is_error == is_error
        && cached_key.message == message
    {
        return value.clone();
    }

    let key = StatusBarCacheKey {
        theme_index,
        message: message.to_string(),
        is_error,
    };

    let (icon, style) = if is_error {
        ("✗ ", theme.badge_error_style())
    } else {
        ("● ", Style::default().fg(theme.success))
    };

    let built = Arc::new(Line::from(vec![
        Span::styled(icon, style),
        Span::styled(message.to_string(), Style::default().fg(theme.fg_dim)),
    ]));

    if let Ok(mut cache) = STATUS_BAR_LINE_CACHE.lock() {
        *cache = Some((key, built.clone()));
    }

    built
}

fn cached_sidebar_lines(
    theme_index: u8,
    active: AppView,
    sidebar_cursor: usize,
    theme: &Theme,
    data: &SidebarRenderData<'_>,
) -> SidebarCacheValue {
    use crate::app::{SidebarItem, sidebar_rows};

    let sidebar_active = data.focus == crate::app::Focus::Sidebar;
    let key = SidebarCacheKey {
        theme_index,
        icon_mode: crate::icons::active_icon_mode() as u8,
        active,
        sidebar_cursor,
        collapsed_mask: collapsed_mask(data.collapsed),
        sidebar_active,
        counts_hash: data.counts_hash,
    };

    if let Ok(mut cache) = SIDEBAR_LINE_CACHE.lock()
        && let Some(value) = cache.get(&key)
    {
        return value;
    }

    let rows = sidebar_rows(data.collapsed);
    let selected_active_style = Style::default()
        .fg(theme.selection_fg)
        .bg(theme.selection_bg)
        .add_modifier(Modifier::BOLD);
    let selected_inactive_style = Style::default()
        .fg(theme.fg)
        .bg(theme.bg_surface)
        .add_modifier(Modifier::BOLD);
    let group_label_style = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let active_label_style = Style::default().fg(theme.fg).add_modifier(Modifier::BOLD);
    let inactive_view_style = Style::default().fg(theme.fg_dim);

    let built: SidebarCacheValue = Arc::new(
        rows.iter()
            .enumerate()
            .map(|(idx, item)| {
                let is_cursor = idx == sidebar_cursor;
                match item {
                    SidebarItem::Group(group) => {
                        let is_collapsed = data.collapsed.contains(group);
                        let line = group.sidebar_text(is_collapsed);
                        if is_cursor {
                            Line::from(vec![Span::styled(line, selected_active_style)])
                        } else {
                            Line::from(vec![Span::styled(line, group_label_style)])
                        }
                    }
                    SidebarItem::View(view) => {
                        let is_active = *view == active;
                        let base = view.sidebar_text();
                        let line = data
                            .counts
                            .iter()
                            .find(|(count_view, _)| count_view == view)
                            .map_or_else(
                                || base.clone(),
                                |count| match count {
                                    (_, Some(count)) => format!("{base} ({count})"),
                                    (_, None) => format!("{base} (…)"),
                                },
                            );
                        if is_cursor && is_active && sidebar_active {
                            Line::from(vec![Span::styled(line, selected_active_style)])
                        } else if is_cursor && sidebar_active {
                            Line::from(vec![Span::styled(line, selected_inactive_style)])
                        } else if is_active {
                            Line::from(vec![Span::styled(line, active_label_style)])
                        } else {
                            Line::from(vec![Span::styled(line, inactive_view_style)])
                        }
                    }
                }
            })
            .collect::<Vec<_>>(),
    );

    if let Ok(mut cache) = SIDEBAR_LINE_CACHE.lock() {
        cache.insert(key, built.clone());
    }

    built
}

/// Renders the top header bar with app title, version badge, and cluster endpoint.
pub fn render_header(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    cluster_meta: &str,
    health: ConnectionHealth,
) {
    let theme = default_theme();
    let theme_index = crate::ui::theme::active_theme_index();
    let text = cached_header_line(
        theme_index,
        crate::icons::active_icon_mode() as u8,
        title,
        cluster_meta,
        health,
        &theme,
    );

    frame.render_widget(
        Paragraph::new((*text).clone())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(theme.border_type())
                    .border_style(theme.border_active_style())
                    .style(Style::default().bg(theme.header_bg)),
            )
            .alignment(Alignment::Left),
        area,
    );
}

/// Renders the tab navigation bar for all primary app views.
pub fn render_tabs(frame: &mut Frame, area: Rect, views: &[AppView], active: AppView) {
    let theme = default_theme();

    let titles: Vec<Line> = views
        .iter()
        .map(|view| {
            let label = view.label();
            Line::from(Span::raw(format!(" {label} ")))
        })
        .collect();

    let selected = views.iter().position(|view| *view == active).unwrap_or(0);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg_surface));

    let tabs = Tabs::new(titles)
        .block(block)
        .select(selected)
        .style(Style::default().fg(theme.tab_inactive_fg))
        .highlight_style(
            Style::default()
                .fg(theme.tab_active_fg)
                .bg(theme.tab_active_bg)
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::styled("│", theme.muted_style()));

    frame.render_widget(tabs, area);
}

/// Renders the left sidebar navigation with Lens-style collapsible groups.
pub fn render_sidebar(
    frame: &mut Frame,
    area: Rect,
    active: AppView,
    sidebar_cursor: usize,
    data: &SidebarRenderData<'_>,
) {
    use crate::app::Focus;
    use ratatui::layout::Margin;

    let theme = default_theme();
    let theme_index = crate::ui::theme::active_theme_index();
    let sidebar_active = data.focus == Focus::Sidebar;

    let border_style = if sidebar_active {
        theme.border_style()
    } else {
        Style::default().fg(theme.muted)
    };

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .style(Style::default().bg(theme.bg_surface));
    frame.render_widget(outer, area);

    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let lines = cached_sidebar_lines(theme_index, active, sidebar_cursor, &theme, data);

    let buf = frame.buffer_mut();
    for (i, line) in lines.iter().enumerate() {
        if i >= inner.height as usize {
            break;
        }
        buf.set_line(inner.x, inner.y + i as u16, line, inner.width);
    }
}

/// Renders the bottom status bar with context-aware styling.
pub fn render_status_bar(frame: &mut Frame, area: Rect, message: &str, is_error: bool) {
    let theme = default_theme();
    let theme_index = crate::ui::theme::active_theme_index();
    let text = cached_status_line(theme_index, message, is_error, &theme);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .border_style(if is_error {
            theme.badge_error_style()
        } else {
            theme.border_style()
        })
        .style(Style::default().bg(theme.statusbar_bg));

    let widget = Paragraph::new((*text).clone()).block(block);
    frame.render_widget(widget, area);
}

/// Returns a styled bordered block with rounded corners using the default theme.
pub fn default_block(title: &str) -> Block<'static> {
    let theme = default_theme();
    Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg))
}

/// Returns a styled bordered block with active (accent) border — for focused panels.
pub fn active_block(title: &str) -> Block<'static> {
    let theme = default_theme();
    Block::default()
        .title(Span::styled(format!(" {title} "), theme.title_style()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_active_style())
        .style(Style::default().bg(theme.bg))
}

/// Returns an active or default block based on whether the panel is focused.
pub fn content_block(title: &str, focused: bool) -> Block<'static> {
    if focused {
        active_block(title)
    } else {
        default_block(title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::IconMode;

    fn line_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    #[test]
    fn header_cache_respects_icon_mode() {
        crate::icons::set_icon_mode(IconMode::Nerd);
        let theme = default_theme();
        let nerd = cached_header_line(
            0,
            crate::icons::active_icon_mode() as u8,
            "KubecTUI",
            "cluster-a",
            ConnectionHealth::Connected,
            &theme,
        );

        crate::icons::set_icon_mode(IconMode::Plain);
        let plain = cached_header_line(
            0,
            crate::icons::active_icon_mode() as u8,
            "KubecTUI",
            "cluster-a",
            ConnectionHealth::Connected,
            &theme,
        );

        assert_ne!(line_text(&nerd), line_text(&plain));

        crate::icons::set_icon_mode(IconMode::Nerd);
    }
}
