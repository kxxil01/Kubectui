//! User preference types for view personalization.

use crate::bookmarks::BookmarkEntry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_true() -> bool {
    true
}

/// Per-view sort + column preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewPreferences {
    /// Column ID to sort by (e.g. "age", "status", "restarts").
    #[serde(default)]
    pub sort_column: Option<String>,
    /// Sort direction. `true` = ascending (default).
    #[serde(default = "default_true")]
    pub sort_ascending: bool,
    /// Column IDs to hide.
    #[serde(default)]
    pub hidden_columns: Vec<String>,
    /// Column IDs to explicitly un-hide (overrides `hidden_columns` from lower layers).
    /// Useful for cluster-level prefs to re-show columns hidden globally.
    /// Currently settable via config file only; no UI action writes this yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shown_columns: Vec<String>,
    /// Custom column ordering. `None` = default order.
    #[serde(default)]
    pub column_order: Option<Vec<String>>,
}

impl Default for ViewPreferences {
    fn default() -> Self {
        Self {
            sort_column: None,
            sort_ascending: true,
            hidden_columns: Vec::new(),
            shown_columns: Vec::new(),
            column_order: None,
        }
    }
}

/// Global user preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserPreferences {
    #[serde(default)]
    pub views: HashMap<String, ViewPreferences>,
}

/// Per-cluster preference overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClusterPreferences {
    #[serde(default)]
    pub views: HashMap<String, ViewPreferences>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bookmarks: Vec<BookmarkEntry>,
}

/// Resolves the effective preferences for a view by merging
/// defaults ← global ← cluster-specific (field-level merge).
///
/// Each layer only overrides fields that differ from defaults:
/// - `sort_column`: overridden if `Some`
/// - `sort_ascending`: overridden if not the default (`true`)
/// - `hidden_columns`: union of all layers
/// - `column_order`: overridden if `Some`
pub fn resolve_view_preferences(
    view_key: &str,
    global: &Option<UserPreferences>,
    clusters: &Option<HashMap<String, ClusterPreferences>>,
    current_context: Option<&str>,
) -> ViewPreferences {
    let mut result = ViewPreferences::default();

    // Apply global layer
    if let Some(global) = global
        && let Some(prefs) = global.views.get(view_key)
    {
        merge_view_preferences(&mut result, prefs);
    }

    // Apply cluster-specific layer on top
    if let Some(ctx) = current_context
        && let Some(clusters) = clusters
        && let Some(cluster) = clusters.get(ctx)
        && let Some(prefs) = cluster.views.get(view_key)
    {
        merge_view_preferences(&mut result, prefs);
    }

    result
}

/// Merges `overlay` fields into `base`, overriding only non-default values.
///
/// - `shown_columns` in the overlay removes entries from `base.hidden_columns`
/// - `hidden_columns` in the overlay are unioned into `base.hidden_columns`
fn merge_view_preferences(base: &mut ViewPreferences, overlay: &ViewPreferences) {
    if overlay.sort_column.is_some() {
        base.sort_column = overlay.sort_column.clone();
        base.sort_ascending = overlay.sort_ascending;
    }
    // Apply shown_columns first: un-hide columns from lower layers
    if !overlay.shown_columns.is_empty() {
        base.hidden_columns
            .retain(|c| !overlay.shown_columns.contains(c));
    }
    // Then union hidden_columns
    for col in &overlay.hidden_columns {
        if !base.hidden_columns.contains(col) {
            base.hidden_columns.push(col.clone());
        }
    }
    if overlay.column_order.is_some() {
        base.column_order = overlay.column_order.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_view_prefs() {
        let vp = ViewPreferences::default();
        assert!(vp.sort_column.is_none());
        assert!(vp.sort_ascending);
        assert!(vp.hidden_columns.is_empty());
        assert!(vp.column_order.is_none());
    }

    #[test]
    fn resolve_global_only() {
        let mut global = UserPreferences::default();
        global.views.insert(
            "pods".into(),
            ViewPreferences {
                sort_column: Some("age".into()),
                sort_ascending: false,
                ..Default::default()
            },
        );
        let result = resolve_view_preferences("pods", &Some(global), &None, None);
        assert_eq!(result.sort_column.as_deref(), Some("age"));
        assert!(!result.sort_ascending);
    }

    #[test]
    fn resolve_cluster_overrides_sort_preserves_hidden() {
        let mut global = UserPreferences::default();
        global.views.insert(
            "pods".into(),
            ViewPreferences {
                sort_column: Some("age".into()),
                sort_ascending: false,
                hidden_columns: vec!["namespace".into()],
                ..Default::default()
            },
        );
        let mut cluster = ClusterPreferences::default();
        cluster.views.insert(
            "pods".into(),
            ViewPreferences {
                sort_column: Some("status".into()),
                ..Default::default()
            },
        );
        let mut clusters = HashMap::new();
        clusters.insert("prod".into(), cluster);
        let result = resolve_view_preferences("pods", &Some(global), &Some(clusters), Some("prod"));
        // Cluster overrides sort
        assert_eq!(result.sort_column.as_deref(), Some("status"));
        assert!(result.sort_ascending);
        // Global hidden_columns are preserved via field-level merge
        assert!(result.hidden_columns.contains(&"namespace".to_string()));
    }

    #[test]
    fn resolve_hidden_columns_union_across_layers() {
        let mut global = UserPreferences::default();
        global.views.insert(
            "pods".into(),
            ViewPreferences {
                hidden_columns: vec!["namespace".into()],
                ..Default::default()
            },
        );
        let mut cluster = ClusterPreferences::default();
        cluster.views.insert(
            "pods".into(),
            ViewPreferences {
                hidden_columns: vec!["age".into()],
                ..Default::default()
            },
        );
        let mut clusters = HashMap::new();
        clusters.insert("prod".into(), cluster);
        let result = resolve_view_preferences("pods", &Some(global), &Some(clusters), Some("prod"));
        assert!(result.hidden_columns.contains(&"namespace".to_string()));
        assert!(result.hidden_columns.contains(&"age".to_string()));
        assert_eq!(result.hidden_columns.len(), 2);
    }

    #[test]
    fn resolve_shown_columns_unhides_from_global() {
        let mut global = UserPreferences::default();
        global.views.insert(
            "pods".into(),
            ViewPreferences {
                hidden_columns: vec!["namespace".into(), "age".into()],
                ..Default::default()
            },
        );
        let mut cluster = ClusterPreferences::default();
        cluster.views.insert(
            "pods".into(),
            ViewPreferences {
                shown_columns: vec!["namespace".into()],
                ..Default::default()
            },
        );
        let mut clusters = HashMap::new();
        clusters.insert("prod".into(), cluster);
        let result = resolve_view_preferences("pods", &Some(global), &Some(clusters), Some("prod"));
        // "namespace" was un-hidden by cluster's shown_columns
        assert!(!result.hidden_columns.contains(&"namespace".to_string()));
        // "age" remains hidden
        assert!(result.hidden_columns.contains(&"age".to_string()));
        assert_eq!(result.hidden_columns.len(), 1);
    }

    #[test]
    fn resolve_unknown_cluster_falls_through() {
        let mut global = UserPreferences::default();
        global.views.insert(
            "pods".into(),
            ViewPreferences {
                sort_column: Some("age".into()),
                ..Default::default()
            },
        );
        let result = resolve_view_preferences("pods", &Some(global), &None, Some("unknown"));
        assert_eq!(result.sort_column.as_deref(), Some("age"));
    }

    #[test]
    fn resolve_no_prefs_returns_default() {
        let result = resolve_view_preferences("pods", &None, &None, None);
        assert!(result.sort_column.is_none());
        assert!(result.sort_ascending);
    }

    #[test]
    fn serde_round_trip() {
        let vp = ViewPreferences {
            sort_column: Some("restarts".into()),
            sort_ascending: false,
            hidden_columns: vec!["namespace".into(), "image".into()],
            shown_columns: Vec::new(),
            column_order: Some(vec!["name".into(), "status".into(), "age".into()]),
        };
        let json = serde_json::to_string(&vp).unwrap();
        let back: ViewPreferences = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sort_column, vp.sort_column);
        assert_eq!(back.sort_ascending, vp.sort_ascending);
        assert_eq!(back.hidden_columns, vp.hidden_columns);
        assert_eq!(back.column_order, vp.column_order);
    }

    #[test]
    fn deserialize_empty_json_uses_defaults() {
        let vp: ViewPreferences = serde_json::from_str("{}").unwrap();
        assert!(vp.sort_column.is_none());
        assert!(vp.sort_ascending);
        assert!(vp.hidden_columns.is_empty());
        assert!(vp.column_order.is_none());
    }

    #[test]
    fn cluster_preferences_preserve_bookmarks() {
        let cluster = ClusterPreferences {
            views: HashMap::new(),
            bookmarks: vec![BookmarkEntry {
                resource: crate::app::ResourceRef::Secret(
                    "app-secret".to_string(),
                    "default".to_string(),
                ),
                bookmarked_at_unix: 123,
            }],
        };

        let serialized = serde_json::to_string(&cluster).expect("serialized cluster prefs");
        let decoded: ClusterPreferences =
            serde_json::from_str(&serialized).expect("decoded cluster prefs");

        assert_eq!(decoded.bookmarks.len(), 1);
        assert_eq!(decoded.bookmarks[0].bookmarked_at_unix, 123);
    }
}
