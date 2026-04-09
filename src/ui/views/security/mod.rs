//! Security/RBAC views.

use ratatui::layout::Rect;

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
