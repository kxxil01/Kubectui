//! User preference types for view personalization.

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
}

/// Resolves the effective preferences for a view by checking
/// cluster-specific → global → defaults.
pub fn resolve_view_preferences(
    view_key: &str,
    global: &Option<UserPreferences>,
    clusters: &Option<HashMap<String, ClusterPreferences>>,
    current_context: Option<&str>,
) -> ViewPreferences {
    // Try cluster-specific first
    if let Some(ctx) = current_context
        && let Some(clusters) = clusters
        && let Some(cluster) = clusters.get(ctx)
        && let Some(prefs) = cluster.views.get(view_key)
    {
        return prefs.clone();
    }
    // Fall through to global
    if let Some(global) = global
        && let Some(prefs) = global.views.get(view_key)
    {
        return prefs.clone();
    }
    ViewPreferences::default()
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
    fn resolve_cluster_overrides_global() {
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
        assert_eq!(result.sort_column.as_deref(), Some("status"));
        // cluster override replaces entire ViewPreferences, not field-merge
        assert!(result.sort_ascending);
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
}
