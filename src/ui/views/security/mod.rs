//! Security/RBAC views.

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
