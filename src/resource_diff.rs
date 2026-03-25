//! Canonical resource drift computation for live vs. declarative state.

use std::collections::BTreeSet;

use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value};

const LAST_APPLIED_ANNOTATION: &str = "kubectl.kubernetes.io/last-applied-configuration";
const ROLLOUT_RESTART_ANNOTATION: &str = "kubectl.kubernetes.io/restartedAt";
const MAX_SAFE_DIFF_MATRIX_CELLS: usize = 4_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceDiffBaselineKind {
    LastAppliedAnnotation,
    ServerSideApplyManagedFields,
    Missing,
}

impl ResourceDiffBaselineKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::LastAppliedAnnotation => "last-applied",
            Self::ServerSideApplyManagedFields => "ssa-managedFields",
            Self::Missing => "no baseline",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceDiffLineKind {
    Header,
    Hunk,
    Context,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceDiffLine {
    pub kind: ResourceDiffLineKind,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceDiffResult {
    pub baseline_kind: ResourceDiffBaselineKind,
    pub summary: String,
    pub lines: Vec<ResourceDiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SsaApplyEntry {
    manager: String,
    api_version: Option<String>,
    time: Option<String>,
    subresource: Option<String>,
}

pub fn build_resource_diff(live_yaml: &str) -> Result<ResourceDiffResult> {
    let mut live = parse_manifest(
        live_yaml,
        "live manifest YAML is unavailable or is not a Kubernetes object manifest",
    )?;
    let baseline = extract_last_applied(&live);
    let ssa_apply_entries = extract_ssa_apply_entries(&live);
    normalize_resource_value(&mut live);

    let Some(baseline_yaml) = baseline else {
        if !ssa_apply_entries.is_empty() {
            return Ok(build_ssa_managed_fields_notice(&ssa_apply_entries));
        }
        return Ok(ResourceDiffResult {
            baseline_kind: ResourceDiffBaselineKind::Missing,
            summary: "No client-side apply baseline available. Resource may be managed by Helm, server-side apply, or imperative create.".to_string(),
            lines: Vec::new(),
        });
    };

    let mut applied = parse_manifest(
        &baseline_yaml,
        "last-applied annotation does not contain a Kubernetes object manifest",
    )
    .with_context(|| "failed to parse last-applied annotation as YAML/JSON".to_string())?;
    normalize_resource_value(&mut applied);

    let live_text = canonical_yaml(&live)?;
    let applied_text = canonical_yaml(&applied)?;
    if live_text == applied_text {
        return Ok(ResourceDiffResult {
            baseline_kind: ResourceDiffBaselineKind::LastAppliedAnnotation,
            summary: "No drift detected after filtering managed fields.".to_string(),
            lines: Vec::new(),
        });
    }

    if !diff_matrix_fits_budget(&applied_text, &live_text) {
        return Ok(ResourceDiffResult {
            baseline_kind: ResourceDiffBaselineKind::LastAppliedAnnotation,
            summary: "Drift detected, but the normalized manifest is too large for safe inline diff rendering.".to_string(),
            lines: Vec::new(),
        });
    }

    let (lines, added, removed) = build_unified_diff(&applied_text, &live_text);
    Ok(ResourceDiffResult {
        baseline_kind: ResourceDiffBaselineKind::LastAppliedAnnotation,
        summary: format!("Drift detected: {added} added, {removed} removed.",),
        lines,
    })
}

fn parse_manifest(yaml: &str, invalid_message: &str) -> Result<Value> {
    let value: Value = serde_yaml::from_str(yaml).context("invalid YAML")?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(anyhow!("{invalid_message}"))
    }
}

fn extract_last_applied(value: &Value) -> Option<String> {
    value
        .get("metadata")?
        .get("annotations")?
        .get(LAST_APPLIED_ANNOTATION)?
        .as_str()
        .map(str::to_string)
}

fn extract_ssa_apply_entries(value: &Value) -> Vec<SsaApplyEntry> {
    let Some(entries) = value
        .get("metadata")
        .and_then(|metadata| metadata.get("managedFields"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let mut apply_entries = entries
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|entry| {
            let operation = entry.get("operation").and_then(Value::as_str)?;
            if operation != "Apply" {
                return None;
            }

            let fields_type = entry.get("fieldsType").and_then(Value::as_str);
            if let Some(fields_type) = fields_type
                && fields_type != "FieldsV1"
            {
                return None;
            }

            let subresource = entry
                .get("subresource")
                .and_then(Value::as_str)
                .map(str::to_string);
            if matches!(subresource.as_deref(), Some("status")) {
                return None;
            }

            Some(SsaApplyEntry {
                manager: entry
                    .get("manager")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                api_version: entry
                    .get("apiVersion")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                time: entry
                    .get("time")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                subresource,
            })
        })
        .collect::<Vec<_>>();

    apply_entries.sort_unstable_by(|left, right| {
        right
            .time
            .cmp(&left.time)
            .then_with(|| left.manager.cmp(&right.manager))
            .then_with(|| left.subresource.cmp(&right.subresource))
    });

    apply_entries
}

fn build_ssa_managed_fields_notice(entries: &[SsaApplyEntry]) -> ResourceDiffResult {
    let unique_managers = entries
        .iter()
        .map(|entry| entry.manager.as_str())
        .collect::<BTreeSet<_>>();
    let latest = &entries[0];
    let manager_summary = if unique_managers.len() == 1 {
        format!("manager '{}'", latest.manager)
    } else {
        format!(
            "{} apply managers (latest '{}')",
            unique_managers.len(),
            latest.manager
        )
    };

    let mut lines = vec![
        ResourceDiffLine {
            kind: ResourceDiffLineKind::Header,
            content: "# server-side apply ownership metadata".to_string(),
        },
        ResourceDiffLine {
            kind: ResourceDiffLineKind::Context,
            content: format!(" managedFields reports {manager_summary}."),
        },
        ResourceDiffLine {
            kind: ResourceDiffLineKind::Context,
            content: " Kubernetes managedFields record field ownership and update time, not the previously applied field values.".to_string(),
        },
        ResourceDiffLine {
            kind: ResourceDiffLineKind::Context,
            content: " A historical live-vs-applied diff cannot be reconstructed from SSA metadata alone.".to_string(),
        },
    ];

    for entry in entries {
        let mut detail = format!(" manager: {}", entry.manager);
        if let Some(api_version) = &entry.api_version {
            detail.push_str(&format!("  apiVersion: {api_version}"));
        }
        if let Some(time) = &entry.time {
            detail.push_str(&format!("  time: {time}"));
        }
        if let Some(subresource) = &entry.subresource {
            detail.push_str(&format!("  subresource: {subresource}"));
        }
        lines.push(ResourceDiffLine {
            kind: ResourceDiffLineKind::Context,
            content: detail,
        });
    }

    ResourceDiffResult {
        baseline_kind: ResourceDiffBaselineKind::ServerSideApplyManagedFields,
        summary: format!(
            "No client-side apply baseline available. This resource is managed by server-side apply via {manager_summary}, but managedFields does not preserve prior applied values."
        ),
        lines,
    }
}

fn normalize_resource_value(value: &mut Value) {
    let Some(map) = value.as_object_mut() else {
        return;
    };

    if let Some(metadata) = map.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.remove("resourceVersion");
        metadata.remove("uid");
        metadata.remove("managedFields");
        metadata.remove("creationTimestamp");
        metadata.remove("generation");
        metadata.remove("selfLink");

        if let Some(annotations) = metadata
            .get_mut("annotations")
            .and_then(Value::as_object_mut)
        {
            annotations.remove(LAST_APPLIED_ANNOTATION);
            if annotations.is_empty() {
                metadata.remove("annotations");
            }
        }

        if let Some(owner_refs) = metadata
            .get_mut("ownerReferences")
            .and_then(Value::as_array_mut)
        {
            for owner_ref in owner_refs {
                if let Some(owner_ref) = owner_ref.as_object_mut() {
                    owner_ref.remove("uid");
                }
            }
        }

        if metadata.is_empty() {
            map.remove("metadata");
        }
    }

    map.remove("status");
    remove_rollout_restart_annotation(map);
    *value = sort_value(value.take());
}

fn canonical_yaml(value: &Value) -> Result<String> {
    let rendered = serde_yaml::to_string(value).context("failed to serialize normalized YAML")?;
    Ok(rendered.trim_start_matches("---\n").trim_end().to_string())
}

fn sort_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
            let mut sorted = Map::with_capacity(entries.len());
            for (key, value) in entries {
                sorted.insert(key, sort_value(value));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_value).collect()),
        other => other,
    }
}

fn remove_rollout_restart_annotation(map: &mut Map<String, Value>) {
    let Some(spec) = map.get_mut("spec").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(template) = spec.get_mut("template").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(metadata) = template.get_mut("metadata").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(annotations) = metadata
        .get_mut("annotations")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    annotations.remove(ROLLOUT_RESTART_ANNOTATION);
    if annotations.is_empty() {
        metadata.remove("annotations");
    }
    if metadata.is_empty() {
        template.remove("metadata");
    }
}

fn diff_matrix_fits_budget(old_text: &str, new_text: &str) -> bool {
    let old_lines = old_text.lines().count();
    let new_lines = new_text.lines().count();
    old_lines
        .saturating_add(1)
        .saturating_mul(new_lines.saturating_add(1))
        <= MAX_SAFE_DIFF_MATRIX_CELLS
}

fn build_unified_diff(old_text: &str, new_text: &str) -> (Vec<ResourceDiffLine>, usize, usize) {
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();
    let operations = diff_operations(&old_lines, &new_lines);

    let mut lines = Vec::with_capacity(operations.len() + 3);
    lines.push(ResourceDiffLine {
        kind: ResourceDiffLineKind::Header,
        content: "--- applied".to_string(),
    });
    lines.push(ResourceDiffLine {
        kind: ResourceDiffLineKind::Header,
        content: "+++ live".to_string(),
    });
    lines.push(ResourceDiffLine {
        kind: ResourceDiffLineKind::Hunk,
        content: format!("@@ -1,{} +1,{} @@", old_lines.len(), new_lines.len()),
    });

    let mut added = 0usize;
    let mut removed = 0usize;
    for operation in operations {
        let (kind, prefix) = match operation {
            DiffOperation::Context(line) => (ResourceDiffLineKind::Context, format!(" {line}")),
            DiffOperation::Added(line) => {
                added += 1;
                (ResourceDiffLineKind::Added, format!("+{line}"))
            }
            DiffOperation::Removed(line) => {
                removed += 1;
                (ResourceDiffLineKind::Removed, format!("-{line}"))
            }
        };
        lines.push(ResourceDiffLine {
            kind,
            content: prefix,
        });
    }

    (lines, added, removed)
}

enum DiffOperation<'a> {
    Context(&'a str),
    Added(&'a str),
    Removed(&'a str),
}

fn diff_operations<'a>(
    old_lines: &'a [&'a str],
    new_lines: &'a [&'a str],
) -> Vec<DiffOperation<'a>> {
    let old_len = old_lines.len();
    let new_len = new_lines.len();
    let mut lcs = vec![vec![0usize; new_len + 1]; old_len + 1];

    for old_idx in (0..old_len).rev() {
        for new_idx in (0..new_len).rev() {
            lcs[old_idx][new_idx] = if old_lines[old_idx] == new_lines[new_idx] {
                lcs[old_idx + 1][new_idx + 1] + 1
            } else {
                lcs[old_idx + 1][new_idx].max(lcs[old_idx][new_idx + 1])
            };
        }
    }

    let mut old_idx = 0usize;
    let mut new_idx = 0usize;
    let mut operations = Vec::with_capacity(old_len + new_len);
    while old_idx < old_len || new_idx < new_len {
        if old_idx < old_len && new_idx < new_len && old_lines[old_idx] == new_lines[new_idx] {
            operations.push(DiffOperation::Context(old_lines[old_idx]));
            old_idx += 1;
            new_idx += 1;
            continue;
        }

        if old_idx < old_len
            && (new_idx == new_len || lcs[old_idx + 1][new_idx] >= lcs[old_idx][new_idx + 1])
        {
            operations.push(DiffOperation::Removed(old_lines[old_idx]));
            old_idx += 1;
            continue;
        }

        if new_idx < new_len {
            operations.push(DiffOperation::Added(new_lines[new_idx]));
            new_idx += 1;
        }
    }

    operations
}

#[cfg(test)]
mod tests {
    use super::{
        LAST_APPLIED_ANNOTATION, ROLLOUT_RESTART_ANNOTATION, ResourceDiffBaselineKind,
        ResourceDiffLineKind, build_resource_diff, normalize_resource_value,
    };
    use serde_json::json;

    #[test]
    fn reports_missing_baseline_when_annotation_absent() {
        let result = build_resource_diff(
            r#"
apiVersion: v1
kind: ConfigMap
metadata:
  name: demo
data:
  hello: world
"#,
        )
        .expect("diff should build");

        assert_eq!(result.baseline_kind, ResourceDiffBaselineKind::Missing);
        assert!(result.lines.is_empty());
        assert!(
            result
                .summary
                .contains("No client-side apply baseline available")
        );
    }

    #[test]
    fn reports_ssa_notice_when_apply_managed_fields_exist_without_last_applied() {
        let result = build_resource_diff(
            r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: demo
  managedFields:
    - manager: kubectl
      operation: Apply
      apiVersion: apps/v1
      fieldsType: FieldsV1
      time: "2026-03-24T14:20:00Z"
      fieldsV1: {}
spec:
  replicas: 2
"#,
        )
        .expect("diff should build");

        assert_eq!(
            result.baseline_kind,
            ResourceDiffBaselineKind::ServerSideApplyManagedFields
        );
        assert!(
            result
                .summary
                .contains("managedFields does not preserve prior applied values")
        );
        assert!(
            result
                .lines
                .iter()
                .any(|line| line.content.contains("manager: kubectl"))
        );
    }

    #[test]
    fn ignores_status_subresource_apply_entries_for_ssa_fallback() {
        let result = build_resource_diff(
            r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: demo
  managedFields:
    - manager: kube-controller-manager
      operation: Apply
      apiVersion: apps/v1
      fieldsType: FieldsV1
      subresource: status
      time: "2026-03-24T14:20:00Z"
      fieldsV1: {}
spec:
  replicas: 2
"#,
        )
        .expect("diff should build");

        assert_eq!(result.baseline_kind, ResourceDiffBaselineKind::Missing);
        assert!(result.lines.is_empty());
    }

    #[test]
    fn prefers_last_applied_annotation_over_ssa_notice() {
        let result = build_resource_diff(
            r#"
apiVersion: v1
kind: ConfigMap
metadata:
  name: demo
  annotations:
    kubectl.kubernetes.io/last-applied-configuration: |
      {"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"demo"},"data":{"hello":"world"}}
  managedFields:
    - manager: kubectl
      operation: Apply
      apiVersion: v1
      fieldsType: FieldsV1
      time: "2026-03-24T14:20:00Z"
      fieldsV1: {}
data:
  hello: world
"#,
        )
        .expect("diff should build");

        assert_eq!(
            result.baseline_kind,
            ResourceDiffBaselineKind::LastAppliedAnnotation
        );
        assert!(result.summary.contains("No drift detected"));
    }

    #[test]
    fn strips_managed_fields_before_diff() {
        let mut value = json!({
            "metadata": {
                "name": "demo",
                "resourceVersion": "1",
                "uid": "abc",
                "annotations": {
                    "kubectl.kubernetes.io/last-applied-configuration": "{}"
                }
            },
            "status": {
                "phase": "Running"
            }
        });

        normalize_resource_value(&mut value);
        assert_eq!(value, json!({ "metadata": { "name": "demo" } }));
    }

    #[test]
    fn preserves_nested_status_and_metadata() {
        let mut value = json!({
            "apiVersion": "example.io/v1",
            "kind": "Widget",
            "metadata": {
                "name": "demo",
                "annotations": {
                    LAST_APPLIED_ANNOTATION: "{}"
                }
            },
            "spec": {
                "status": {
                    "desired": "enabled"
                },
                "template": {
                    "metadata": {
                        "labels": {
                            "app": "demo"
                        }
                    }
                }
            },
            "status": {
                "observed": "ready"
            }
        });

        normalize_resource_value(&mut value);
        assert_eq!(
            value,
            json!({
                "apiVersion": "example.io/v1",
                "kind": "Widget",
                "metadata": {
                    "name": "demo"
                },
                "spec": {
                    "status": {
                        "desired": "enabled"
                    },
                    "template": {
                        "metadata": {
                            "labels": {
                                "app": "demo"
                            }
                        }
                    }
                }
            })
        );
    }

    #[test]
    fn returns_no_drift_when_only_managed_fields_changed() {
        let result = build_resource_diff(
            r#"
apiVersion: v1
kind: ConfigMap
metadata:
  name: demo
  resourceVersion: "12"
  annotations:
    kubectl.kubernetes.io/last-applied-configuration: |
      {"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"demo"},"data":{"hello":"world"}}
data:
  hello: world
status:
  phase: Active
"#,
        )
        .expect("diff should build");

        assert_eq!(
            result.baseline_kind,
            ResourceDiffBaselineKind::LastAppliedAnnotation
        );
        assert!(result.lines.is_empty());
        assert!(result.summary.contains("No drift detected"));
    }

    #[test]
    fn returns_no_drift_when_only_key_order_differs() {
        let result = build_resource_diff(
            r#"
apiVersion: v1
kind: ConfigMap
metadata:
  name: demo
  annotations:
    kubectl.kubernetes.io/last-applied-configuration: |
      {"kind":"ConfigMap","apiVersion":"v1","data":{"beta":"2","alpha":"1"},"metadata":{"name":"demo"}}
data:
  alpha: "1"
  beta: "2"
"#,
        )
        .expect("diff should build");

        assert!(result.lines.is_empty());
        assert!(result.summary.contains("No drift detected"));
    }

    #[test]
    fn returns_added_and_removed_lines_for_drift() {
        let result = build_resource_diff(
            r#"
apiVersion: v1
kind: ConfigMap
metadata:
  name: demo
  annotations:
    kubectl.kubernetes.io/last-applied-configuration: |
      {"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"demo"},"data":{"hello":"world"}}
data:
  hello: universe
  extra: value
"#,
        )
        .expect("diff should build");

        assert_eq!(
            result.baseline_kind,
            ResourceDiffBaselineKind::LastAppliedAnnotation
        );
        assert!(result.summary.contains("Drift detected"));
        assert!(
            result
                .lines
                .iter()
                .any(|line| line.kind == ResourceDiffLineKind::Removed)
        );
        assert!(
            result
                .lines
                .iter()
                .any(|line| line.kind == ResourceDiffLineKind::Added)
        );
    }

    #[test]
    fn strips_rollout_restart_annotation_from_pod_template() {
        let mut value = json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {
                "name": "demo",
                "annotations": {
                    LAST_APPLIED_ANNOTATION: "{}"
                }
            },
            "spec": {
                "template": {
                    "metadata": {
                        "annotations": {
                            ROLLOUT_RESTART_ANNOTATION: "2026-03-22T15:00:00Z",
                            "team": "platform"
                        }
                    }
                }
            }
        });

        normalize_resource_value(&mut value);
        assert_eq!(
            value,
            json!({
                "apiVersion": "apps/v1",
                "kind": "Deployment",
                "metadata": {
                    "name": "demo"
                },
                "spec": {
                    "template": {
                        "metadata": {
                            "annotations": {
                                "team": "platform"
                            }
                        }
                    }
                }
            })
        );
    }

    #[test]
    fn skips_inline_diff_when_manifest_is_too_large() {
        let live_lines = (0..2_500)
            .map(|idx| format!("  key-{idx}: live"))
            .collect::<Vec<_>>()
            .join("\n");
        let live_yaml = format!(
            "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: giant\n  annotations:\n    kubectl.kubernetes.io/last-applied-configuration: |\n      {{\"apiVersion\":\"v1\",\"kind\":\"ConfigMap\",\"metadata\":{{\"name\":\"giant\"}},\"data\":{{{}}}}}\ndata:\n{}\n",
            (0..2_500)
                .map(|idx| format!("\\\"key-{idx}\\\":\\\"baseline\\\""))
                .collect::<Vec<_>>()
                .join(","),
            live_lines
        );

        let result = build_resource_diff(&live_yaml).expect("diff should build");
        assert_eq!(
            result.baseline_kind,
            ResourceDiffBaselineKind::LastAppliedAnnotation
        );
        assert!(result.lines.is_empty());
        assert!(result.summary.contains("too large for safe inline diff"));
    }

    #[test]
    fn rejects_non_manifest_input() {
        let err = build_resource_diff("# YAML unavailable (RBAC)\n# kind: Pod")
            .expect_err("comments should not be treated as a manifest");

        assert!(
            err.to_string().contains(
                "live manifest YAML is unavailable or is not a Kubernetes object manifest"
            )
        );
    }
}
