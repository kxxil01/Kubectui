# Milestone 8: Enhanced Resource Detail — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Bring detail view information density closer to Lens — show annotations, owner references, force delete, and CronJob trigger.

**Architecture:** Extends existing `DetailMetadata` with `annotations` and `owner_references` fields, populates them from the existing K8s API response in `fetch_pods`/`metadata_for_resource`. Adds a collapsible labels/annotations panel in the detail view renderer. Force delete adds a `force: bool` parameter to `delete_resource`. CronJob trigger creates a Job from the CronJob spec via the K8s API.

**Tech Stack:** Rust, ratatui, kube-rs (Api<Pod/Job/CronJob>), k8s-openapi (DeleteParams with grace_period_seconds=0 + propagation_policy)

---

## Task 1: Add Annotations to DetailMetadata and Detail View

**Files:**
- Modify: `src/app.rs` (add `annotations` field to `DetailMetadata`)
- Modify: `src/k8s/client.rs` (populate annotations in `fetch_pods`)
- Modify: `src/k8s/dtos.rs` (add `annotations` to `PodInfo`)
- Modify: `src/main.rs` (populate annotations in `metadata_for_resource`)
- Modify: `src/ui/views/detail.rs` (render labels and annotations sections)

### Step 1: Add annotations to PodInfo DTO

In `src/k8s/dtos.rs`, add to `PodInfo` struct:
```rust
pub annotations: Vec<(String, String)>,
```

### Step 2: Populate annotations in fetch_pods

In `src/k8s/client.rs` `fetch_pods`, in the `PodInfo { ... }` construction (around line 244), add:
```rust
annotations: pod
    .metadata
    .annotations
    .unwrap_or_default()
    .into_iter()
    .collect(),
```

### Step 3: Add annotations to DetailMetadata

In `src/app.rs`, add to `DetailMetadata` struct:
```rust
pub annotations: Vec<(String, String)>,
```

### Step 4: Populate annotations in metadata_for_resource

In `src/main.rs` `metadata_for_resource`, in the `ResourceRef::Pod` arm, add `annotations: pod.annotations.clone()` to the `DetailMetadata` construction. For other resource types that don't have annotations yet, use `annotations: Vec::new()`.

### Step 5: Render expanded labels and annotations in detail view

In `src/ui/views/detail.rs` `render_metadata_panel`, the current labels display only shows 3 labels truncated. Replace this with a proper expandable display:

Change the labels rendering to show ALL labels (not just 3), and add an annotations section below:

```rust
// Labels section — show all labels
if !detail_state.metadata.labels.is_empty() {
    lines.push(Line::from(vec![
        Span::styled(" Labels    ", theme.inactive_style()),
        Span::styled(
            format!("({})", detail_state.metadata.labels.len()),
            Style::default().fg(theme.muted),
        ),
    ]));
    for (k, v) in &detail_state.metadata.labels {
        lines.push(Line::from(vec![
            Span::styled("   ", theme.inactive_style()),
            Span::styled(format!("{k}"), Style::default().fg(theme.accent)),
            Span::styled("=", Style::default().fg(theme.muted)),
            Span::styled(format!("{v}"), Style::default().fg(theme.fg_dim)),
        ]));
    }
}

// Annotations section
if !detail_state.metadata.annotations.is_empty() {
    lines.push(Line::from(vec![
        Span::styled(" Annotations ", theme.inactive_style()),
        Span::styled(
            format!("({})", detail_state.metadata.annotations.len()),
            Style::default().fg(theme.muted),
        ),
    ]));
    for (k, v) in detail_state.metadata.annotations.iter().take(5) {
        lines.push(Line::from(vec![
            Span::styled("   ", theme.inactive_style()),
            Span::styled(format!("{k}"), Style::default().fg(theme.accent)),
            Span::styled("=", Style::default().fg(theme.muted)),
            Span::styled(
                if v.len() > 60 { format!("{}…", &v[..60]) } else { v.clone() },
                Style::default().fg(theme.fg_dim),
            ),
        ]));
    }
    if detail_state.metadata.annotations.len() > 5 {
        lines.push(Line::from(Span::styled(
            format!("   … and {} more", detail_state.metadata.annotations.len() - 5),
            Style::default().fg(theme.muted),
        )));
    }
}
```

The metadata panel height constraint in `render_detail` needs to grow. Currently `Constraint::Length(9)` — change to `Constraint::Min(9)` or increase to accommodate labels/annotations.

Actually, a better approach: move the labels/annotations rendering into the `render_details_panel` (the middle column), which already has `Constraint::Min(6)`. This avoids breaking the metadata panel layout.

### Step 6: Verify and commit

```
cargo fmt --all
command cargo clippy --all-targets --all-features -- -D warnings
command cargo test --all-targets --all-features
git add src/app.rs src/k8s/dtos.rs src/k8s/client.rs src/main.rs src/ui/views/detail.rs
git commit -m "feat: show annotations in detail view"
```

---

## Task 2: Add Owner References to Detail View

**Files:**
- Modify: `src/k8s/dtos.rs` (add `OwnerRefInfo` struct and field to `PodInfo`)
- Modify: `src/k8s/client.rs` (populate owner_references in `fetch_pods`)
- Modify: `src/app.rs` (add `owner_references` to `DetailMetadata`)
- Modify: `src/main.rs` (populate owner_references in `metadata_for_resource`)
- Modify: `src/ui/views/detail.rs` (render owner references in details panel)

### Step 1: Add OwnerRefInfo to DTOs

In `src/k8s/dtos.rs`:
```rust
/// Lightweight owner reference for display in detail views.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OwnerRefInfo {
    pub kind: String,
    pub name: String,
}
```

Add to `PodInfo`:
```rust
pub owner_references: Vec<OwnerRefInfo>,
```

### Step 2: Populate in fetch_pods

In `src/k8s/client.rs` `fetch_pods`, add:
```rust
owner_references: pod
    .metadata
    .owner_references
    .unwrap_or_default()
    .into_iter()
    .map(|oref| crate::k8s::dtos::OwnerRefInfo {
        kind: oref.kind,
        name: oref.name,
    })
    .collect(),
```

### Step 3: Add to DetailMetadata

In `src/app.rs`, add:
```rust
pub owner_references: Vec<crate::k8s::dtos::OwnerRefInfo>,
```

### Step 4: Populate in metadata_for_resource

In `metadata_for_resource`, Pod arm: `owner_references: pod.owner_references.clone()`. Other arms: `owner_references: Vec::new()`.

### Step 5: Render in detail view

In `render_details_panel` in `src/ui/views/detail.rs`, add BEFORE the events section:

```rust
if !detail_state.metadata.owner_references.is_empty() {
    lines.push(Line::from(Span::styled(
        " OWNERS",
        theme.section_title_style(),
    )));
    for oref in &detail_state.metadata.owner_references {
        lines.push(Line::from(vec![
            Span::styled(format!("  {}", oref.kind), Style::default().fg(theme.accent)),
            Span::styled("/", Style::default().fg(theme.muted)),
            Span::styled(oref.name.clone(), Style::default().fg(theme.fg)),
        ]));
    }
}
```

### Step 6: Verify and commit

```
cargo fmt --all
command cargo clippy --all-targets --all-features -- -D warnings
command cargo test --all-targets --all-features
git add src/k8s/dtos.rs src/k8s/client.rs src/app.rs src/main.rs src/ui/views/detail.rs
git commit -m "feat: show owner references in detail view"
```

---

## Task 3: Force Delete Option

**Files:**
- Modify: `src/app.rs` (add `ForceDeleteResource` action)
- Modify: `src/events/input.rs` (handle action)
- Modify: `src/k8s/yaml.rs` (add `force_delete_resource` function)
- Modify: `src/k8s/client.rs` (add `force_delete_resource` method)
- Modify: `src/main.rs` (handle ForceDeleteResource action)
- Modify: `src/ui/views/detail.rs` (show force delete hint in delete confirm dialog)

### Step 1: Add force delete K8s function

In `src/k8s/yaml.rs`, add after `delete_resource`:
```rust
/// Force-deletes a resource by setting grace period to 0 and removing finalizers.
pub async fn force_delete_resource(
    client: &Client,
    kind: &str,
    name: &str,
    namespace: Option<&str>,
) -> Result<()> {
    let (api_resource, namespaced) = api_resource_for_kind(kind)
        .with_context(|| format!("unsupported resource kind '{kind}'"))?;

    let api: Api<DynamicObject> = if namespaced {
        match namespace {
            Some(ns) => Api::namespaced_with(client.clone(), ns, &api_resource),
            None => return Err(anyhow::anyhow!("namespace required for {kind}")),
        }
    } else {
        Api::all_with(client.clone(), &api_resource)
    };

    let dp = kube::api::DeleteParams {
        grace_period_seconds: Some(0),
        propagation_policy: Some(kube::api::PropagationPolicy::Background),
        ..Default::default()
    };

    api.delete(name, &dp)
        .await
        .with_context(|| format!("force-delete {kind}/{name} failed"))?;

    Ok(())
}
```

### Step 2: Add client method

In `src/k8s/client.rs`:
```rust
pub async fn force_delete_resource(
    &self,
    kind: &str,
    name: &str,
    namespace: Option<&str>,
) -> Result<()> {
    yaml::force_delete_resource(&self.client, kind, name, namespace).await
}
```

### Step 3: Add action variant

In `src/app.rs` AppAction:
```rust
ForceDeleteResource,
```

### Step 4: Add `F` key in delete confirmation dialog

In `handle_key_event`, find where `confirm_delete` is handled. There should be a section handling key input when `detail.confirm_delete` is true. Add:
```rust
KeyCode::Char('F') => AppAction::ForceDeleteResource,
```

### Step 5: Handle in events/input.rs

```rust
AppAction::ForceDeleteResource => {
    // Handled in main.rs (needs async K8s call)
    true
}
```

### Step 6: Handle in main.rs

Near the `DeleteResource` handler, add `ForceDeleteResource` handler following the same pattern but calling `client.force_delete_resource(...)` instead of `client.delete_resource(...)`. Record in action history as `ActionKind::Delete` with message "Force deleting...".

### Step 7: Update delete confirmation UI

In `src/ui/views/detail.rs` `render_delete_confirm`, add a line showing `[F] Force delete (removes finalizers)` in the dialog.

### Step 8: Verify and commit

```
cargo fmt --all
command cargo clippy --all-targets --all-features -- -D warnings
command cargo test --all-targets --all-features
git add src/app.rs src/events/input.rs src/k8s/yaml.rs src/k8s/client.rs src/main.rs src/ui/views/detail.rs
git commit -m "feat: add force delete option with F key in delete confirmation"
```

---

## Task 4: CronJob Manual Trigger

**Files:**
- Modify: `src/app.rs` (add `TriggerCronJob` action, add key binding)
- Modify: `src/events/input.rs` (handle action)
- Modify: `src/k8s/client.rs` (add `trigger_cronjob` method)
- Modify: `src/main.rs` (handle TriggerCronJob action)
- Modify: `src/policy.rs` (add `Trigger` to DetailAction)
- Modify: `src/ui/views/detail.rs` (show trigger hint in footer)

### Step 1: Add trigger_cronjob to K8s client

In `src/k8s/client.rs`:
```rust
/// Creates a Job from a CronJob spec, effectively triggering a manual run.
pub async fn trigger_cronjob(&self, name: &str, namespace: &str) -> Result<String> {
    use k8s_openapi::api::batch::v1::{CronJob, Job, JobSpec};
    use kube::api::PostParams;

    let cronjobs: Api<CronJob> = Api::namespaced(self.client.clone(), namespace);
    let cronjob = cronjobs
        .get(name)
        .await
        .with_context(|| format!("failed to get CronJob '{name}' in '{namespace}'"))?;

    let job_template = cronjob
        .spec
        .as_ref()
        .map(|s| &s.job_template)
        .context("CronJob has no spec")?;

    let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    let job_name = format!("{name}-manual-{timestamp}");

    let job = Job {
        metadata: kube::api::ObjectMeta {
            name: Some(job_name.clone()),
            namespace: Some(namespace.to_string()),
            labels: job_template.metadata.as_ref().and_then(|m| m.labels.clone()),
            annotations: {
                let mut ann = std::collections::BTreeMap::new();
                ann.insert("cronjob.kubernetes.io/instantiate".to_string(), "manual".to_string());
                Some(ann)
            },
            ..Default::default()
        },
        spec: job_template.spec.clone(),
        ..Default::default()
    };

    let jobs: Api<Job> = Api::namespaced(self.client.clone(), namespace);
    jobs.create(&PostParams::default(), &job)
        .await
        .with_context(|| format!("failed to create Job from CronJob '{name}'"))?;

    Ok(job_name)
}
```

### Step 2: Add DetailAction::Trigger to policy

In `src/policy.rs`, add to `DetailAction` enum:
```rust
Trigger,
```

Update the match in `supports_action` / the policy configuration to return true for `Trigger` only when the resource is a CronJob.

### Step 3: Add action variant

In `src/app.rs` AppAction:
```rust
TriggerCronJob,
```

### Step 4: Add key binding

In `handle_key_event`, add (gated by `Trigger` policy):
```rust
KeyCode::Char('T') if self.detail_view.as_ref().is_some_and(|d| d.supports_action(DetailAction::Trigger)) => {
    AppAction::TriggerCronJob
}
```

Note: `T` (uppercase) is already used for `CycleTheme` when `detail_view.is_none()`. Since this binding only activates when `detail_view.is_some()` and supports Trigger, there's no conflict.

### Step 5: Handle in events/input.rs

```rust
AppAction::TriggerCronJob => {
    // Handled in main.rs (needs async K8s call)
    true
}
```

### Step 6: Handle in main.rs

```rust
AppAction::TriggerCronJob => {
    let cronjob_info = app.detail_view.as_ref().and_then(|d| {
        if let Some(ResourceRef::CronJob(name, ns)) = &d.resource {
            Some((name.clone(), ns.clone()))
        } else {
            None
        }
    });
    if let Some((name, namespace)) = cronjob_info {
        let resource_label = format!("CronJob '{name}'");
        let origin_view = app.view();
        let action_id = app.record_action_pending(
            ActionKind::Trigger,
            origin_view,
            app.detail_view.as_ref().and_then(|d| d.resource.clone()),
            resource_label.clone(),
            format!("Triggering {resource_label}..."),
        );
        let client_clone = client.clone();
        let action_tx_clone = action_result_tx.clone();
        tokio::spawn(async move {
            let result = client_clone.trigger_cronjob(&name, &namespace).await;
            let _ = action_tx_clone.send((action_id, result.map(|job_name| {
                format!("Created Job '{job_name}'")
            }).map_err(|e| e.to_string()))).await;
        });
    }
}
```

NOTE: You'll need to add `ActionKind::Trigger` to the action history model. Check `src/action_history.rs` for the enum and add a `Trigger` variant with label "Trigger".

### Step 7: Add Trigger hint to detail footer

The footer actions are generated by `footer_actions()` method on `DetailViewState`. Add `Trigger` to the list when the resource is a CronJob. Update the hint text to `[T] Trigger`.

### Step 8: Verify and commit

```
cargo fmt --all
command cargo clippy --all-targets --all-features -- -D warnings
command cargo test --all-targets --all-features
git add src/app.rs src/events/input.rs src/k8s/client.rs src/main.rs src/policy.rs src/action_history.rs src/ui/views/detail.rs
git commit -m "feat: add T to manually trigger CronJob as a new Job"
```

---

## Quality Gate

After all 4 tasks:

```bash
cargo fmt --all
command cargo clippy --all-targets --all-features -- -D warnings
command cargo test --all-targets --all-features
cargo build --release
```

---

## Task Summary

| Task | Feature | Key Files | Complexity |
|------|---------|-----------|------------|
| 1 | Annotations in detail view | dtos.rs, client.rs, app.rs, detail.rs | Medium |
| 2 | Owner references in detail view | dtos.rs, client.rs, app.rs, detail.rs | Medium |
| 3 | Force delete (F key) | yaml.rs, client.rs, app.rs, main.rs, detail.rs | Medium |
| 4 | CronJob manual trigger (T key) | client.rs, app.rs, main.rs, policy.rs, action_history.rs | Large |

Tasks 1 and 2 are independent (both add DTO fields + detail rendering). Tasks 3 and 4 are independent of each other. Execute sequentially: 1, 2, 3, 4.

Note: The original M8 plan included "container environment variables" — this is deferred because it requires significant DTO expansion (env vars per container, secret refs, configmap refs) and the YAML view already provides this data. Labels/annotations + owner refs + force delete + CronJob trigger deliver more operator value per effort.
