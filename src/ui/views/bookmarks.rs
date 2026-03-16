//! Bookmarks view for persisted per-cluster resource shortcuts.

use chrono::Utc;
use ratatui::{
    layout::{Constraint, Margin, Rect},
    prelude::{Frame, Modifier, Style},
    text::Span,
    widgets::{
        Cell, HighlightSpacing, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState,
    },
};

use crate::{
    bookmarks::{BookmarkEntry, filtered_bookmark_indices, resource_exists},
    state::ClusterSnapshot,
    ui::{
        components::{content_block, default_block, default_theme},
        format_age, responsive_table_widths, table_viewport_rows, table_window,
    },
};

pub fn render_bookmarks(
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
    let indices = filtered_bookmark_indices(bookmarks, query);

    if indices.is_empty() {
        let message = if query.is_empty() {
            "  No bookmarks saved for this cluster"
        } else {
            "  No bookmarks match the search query"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(message, theme.inactive_style()))
                .block(default_block("Bookmarks")),
            area,
        );
        return;
    }

    let total = indices.len();
    let selected = selected_idx.min(total.saturating_sub(1));
    let window = table_window(total, selected, table_viewport_rows(area));
    let now = Utc::now().timestamp();

    let header = Row::new([
        Cell::from(Span::styled("STATE", theme.header_style())),
        Cell::from(Span::styled("KIND", theme.header_style())),
        Cell::from(Span::styled("NAME", theme.header_style())),
        Cell::from(Span::styled("NAMESPACE", theme.header_style())),
        Cell::from(Span::styled("VIEW", theme.header_style())),
        Cell::from(Span::styled("SAVED", theme.header_style())),
    ])
    .style(theme.header_style())
    .height(1);

    let rows: Vec<Row> = indices[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(local_idx, bookmark_idx)| {
            let bookmark = &bookmarks[*bookmark_idx];
            let exists = resource_exists(cluster, &bookmark.resource);
            let age_secs = now.saturating_sub(bookmark.bookmarked_at_unix).max(0) as u64;
            let saved_age = format_age(Some(std::time::Duration::from_secs(age_secs)));
            let namespace = bookmark.resource.namespace().unwrap_or("-");
            let primary_view = bookmark
                .resource
                .primary_view()
                .map(|view| view.label())
                .unwrap_or("Detail");

            let mut row_style = if (window.start + local_idx).is_multiple_of(2) {
                Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };
            if !exists {
                row_style = row_style
                    .fg(theme.fg_dim)
                    .add_modifier(Modifier::CROSSED_OUT);
            }

            let status = if exists { "★" } else { "✗" };
            let status_style = if exists {
                theme.badge_warning_style()
            } else {
                theme.inactive_style()
            };

            Row::new(vec![
                Cell::from(Span::styled(status, status_style)),
                Cell::from(bookmark.resource.kind()),
                Cell::from(bookmark.resource.name()),
                Cell::from(namespace),
                Cell::from(primary_view),
                Cell::from(saved_age),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(window.selected));
    let title = if query.is_empty() {
        format!(" Bookmarks ({total}) ")
    } else {
        format!(" Bookmarks ({total} of {}) [/{query}]", bookmarks.len())
    };

    let table = Table::new(
        rows,
        responsive_table_widths(
            area.width,
            [
                Constraint::Length(5),
                Constraint::Length(20),
                Constraint::Min(24),
                Constraint::Length(18),
                Constraint::Length(18),
                Constraint::Length(10),
            ],
        ),
    )
    .header(header)
    .block(content_block(&title, focused))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ResourceRef;

    #[test]
    fn render_bookmarks_empty_smoke() {
        let backend = ratatui::backend::TestBackend::new(100, 20);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                render_bookmarks(
                    frame,
                    frame.area(),
                    &ClusterSnapshot::default(),
                    &[],
                    0,
                    "",
                    true,
                );
            })
            .expect("draw bookmarks");
    }

    #[test]
    fn render_bookmarks_with_rows_smoke() {
        let backend = ratatui::backend::TestBackend::new(100, 20);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        let mut snapshot = ClusterSnapshot::default();
        snapshot.secrets.push(crate::k8s::dtos::SecretInfo {
            name: "app-secret".to_string(),
            namespace: "default".to_string(),
            ..Default::default()
        });
        let bookmarks = vec![BookmarkEntry {
            resource: ResourceRef::Secret("app-secret".to_string(), "default".to_string()),
            bookmarked_at_unix: Utc::now().timestamp(),
        }];
        terminal
            .draw(|frame| {
                render_bookmarks(frame, frame.area(), &snapshot, &bookmarks, 0, "", true);
            })
            .expect("draw bookmarks");
    }
}
