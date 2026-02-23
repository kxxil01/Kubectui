//! Reusable UI widgets and building blocks.

pub mod command_palette;
pub mod common;
pub mod context_picker;
pub mod input_field;
pub mod namespace_picker;
pub mod port_forward_dialog;
pub mod probe_panel;
pub mod scale_dialog;

pub use command_palette::{CommandPalette, CommandPaletteAction};
pub use context_picker::{ContextPicker, ContextPickerAction};
pub use input_field::InputFieldWidget;
pub use namespace_picker::{NamespacePicker, NamespacePickerAction};
pub use port_forward_dialog::{FormField, PortForwardAction, PortForwardDialog, PortForwardMode};
pub use probe_panel::ProbePanelState;
pub use scale_dialog::{ScaleAction, ScaleDialogState, ScaleField, render_scale_dialog};

use ratatui::{
    layout::{Alignment, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Tabs},
};

use crate::{app::{AppView, NavGroup}, ui::theme::Theme};

/// Global theme singleton — dark by default, can be overridden via CLI.
pub fn default_theme() -> Theme {
    Theme::dark()
}

/// Renders the top header bar with app title, version badge, and cluster endpoint.
pub fn render_header(frame: &mut Frame, area: Rect, title: &str, cluster_meta: &str) {
    let theme = default_theme();

    let text = Line::from(vec![
        Span::styled(" ⎈ ", theme.title_style()),
        Span::styled(title, theme.title_style()),
        Span::styled("  │  ", theme.muted_style()),
        Span::styled("⛅ ", theme.get_style("fg_dim")),
        Span::styled(cluster_meta, theme.get_style("fg_dim")),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .border_style(theme.border_active_style())
        .style(Style::default().bg(theme.header_bg));

    let widget = Paragraph::new(text)
        .block(block)
        .alignment(Alignment::Left);

    frame.render_widget(widget, area);
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
    collapsed: &std::collections::HashSet<NavGroup>,
    focus: crate::app::Focus,
) {
    use crate::app::{Focus, SidebarItem, sidebar_rows};
    use ratatui::layout::Margin;

    let theme = default_theme();
    let sidebar_active = focus == Focus::Sidebar;

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

    let inner = area.inner(Margin { horizontal: 1, vertical: 1 });
    let rows = sidebar_rows(collapsed);

    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let is_cursor = idx == sidebar_cursor;
            match item {
                SidebarItem::Group(group) => {
                    let collapsed = collapsed.contains(group);
                    let toggle = if collapsed { "▶" } else { "▼" };
                    let label = group.label();
                    if is_cursor {
                        Line::from(vec![Span::styled(
                            format!(" {toggle} {label}"),
                            Style::default()
                                .fg(theme.selection_fg)
                                .bg(theme.selection_bg)
                                .add_modifier(Modifier::BOLD),
                        )])
                    } else {
                        Line::from(vec![
                            Span::styled(
                                format!(" {toggle} "),
                                Style::default().fg(theme.accent),
                            ),
                            Span::styled(
                                label,
                                Style::default()
                                    .fg(theme.accent)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ])
                    }
                }
                SidebarItem::View(view) => {
                    let is_active = *view == active;
                    let label = view.label();
                    if is_cursor && is_active && sidebar_active {
                        Line::from(vec![Span::styled(
                            format!("  ● {label}"),
                            Style::default()
                                .fg(theme.selection_fg)
                                .bg(theme.selection_bg)
                                .add_modifier(Modifier::BOLD),
                        )])
                    } else if is_cursor && sidebar_active {
                        Line::from(vec![Span::styled(
                            format!("    {label}"),
                            Style::default()
                                .fg(theme.fg)
                                .bg(theme.bg_surface)
                                .add_modifier(Modifier::BOLD),
                        )])
                    } else if is_active {
                        Line::from(vec![
                            Span::styled("  ● ", Style::default().fg(theme.accent)),
                            Span::styled(
                                label,
                                Style::default()
                                    .fg(theme.fg)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ])
                    } else {
                        Line::from(vec![Span::styled(
                            format!("    {label}"),
                            Style::default().fg(theme.fg_dim),
                        )])
                    }
                }
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Renders the bottom status bar with context-aware styling.
pub fn render_status_bar(frame: &mut Frame, area: Rect, message: &str, is_error: bool) {
    let theme = default_theme();

    let (icon, style) = if is_error {
        ("✗ ", theme.badge_error_style())
    } else {
        ("● ", Style::default().fg(theme.success))
    };

    let text = Line::from(vec![
        Span::styled(icon, style),
        Span::styled(message, Style::default().fg(theme.fg_dim)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .border_style(if is_error {
            theme.badge_error_style()
        } else {
            theme.border_style()
        })
        .style(Style::default().bg(theme.statusbar_bg));

    let widget = Paragraph::new(text).block(block);
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
        .title(Span::styled(
            format!(" {title} "),
            theme.title_style(),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_active_style())
        .style(Style::default().bg(theme.bg))
}
