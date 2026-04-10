//! Security/RBAC views.

use ratatui::{layout::Rect, prelude::Frame, text::Line};

pub mod cluster_role_bindings;
pub mod cluster_roles;
pub mod role_bindings;
pub mod roles;
pub mod service_accounts;

/// Joins items with ", " or returns "*" for empty slices (RBAC wildcard).
pub(crate) fn join_or_all(items: &[String]) -> String {
    if items.is_empty() {
        "*".to_string()
    } else {
        items.join(", ")
    }
}

pub(crate) fn split_primary_detail(area: Rect) -> (Rect, Rect) {
    crate::ui::vertical_primary_detail_chunks(area, 58, 8, 24)
}

pub(crate) fn render_scrollable_security_detail<'a>(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    focused: bool,
    lines: Vec<Line<'a>>,
    scroll: usize,
) {
    crate::ui::components::render_scrollable_text_block(frame, area, title, focused, lines, scroll);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_primary_detail_compacts_on_short_height() {
        let (primary, detail) = split_primary_detail(Rect::new(0, 0, 90, 18));
        assert_eq!(primary.height, 10);
        assert_eq!(detail.height, 8);
    }

    #[test]
    fn security_primary_detail_uses_full_split_on_tall_height() {
        let (primary, detail) = split_primary_detail(Rect::new(0, 0, 90, 30));
        assert_eq!(primary.height, 17);
        assert_eq!(detail.height, 13);
    }
}
