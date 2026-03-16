//! Flux reconciliation helpers extracted from the event loop.
//!
//! These functions check reconcile progress against baseline snapshots and
//! format status summaries for Flux custom resources.

use std::time::Instant;

use crate::async_types::{FluxReconcileObservedState, PendingFluxReconcileVerification};

use kubectui::{
    action_history::ActionStatus,
    app::{AppState, ResourceRef},
    k8s::dtos::FluxResourceInfo,
    state::ClusterSnapshot,
};

/// Returns `true` when `resource` matches `candidate` on all identifying fields.
pub fn flux_resource_matches(resource: &ResourceRef, candidate: &FluxResourceInfo) -> bool {
    let ResourceRef::CustomResource {
        name,
        namespace,
        group,
        version,
        kind,
        plural,
    } = resource
    else {
        return false;
    };

    candidate.name == *name
        && candidate.namespace == *namespace
        && candidate.group == *group
        && candidate.version == *version
        && candidate.kind == *kind
        && candidate.plural == *plural
}

/// Finds the first `FluxResourceInfo` in the snapshot that matches `resource`.
pub fn flux_resource_for_ref<'a>(
    snapshot: &'a ClusterSnapshot,
    resource: &ResourceRef,
) -> Option<&'a FluxResourceInfo> {
    snapshot
        .flux_resources
        .iter()
        .find(|candidate| flux_resource_matches(resource, candidate))
}

/// Captures the reconcile-relevant fields from a `FluxResourceInfo` into a
/// comparable snapshot value.
pub fn flux_observed_state(resource: &FluxResourceInfo) -> FluxReconcileObservedState {
    FluxReconcileObservedState {
        status: resource.status.clone(),
        message: resource.message.clone(),
        last_reconcile_time: resource.last_reconcile_time,
        last_applied_revision: resource.last_applied_revision.clone(),
        last_attempted_revision: resource.last_attempted_revision.clone(),
        observed_generation: resource.observed_generation,
    }
}

/// Convenience wrapper: looks up a resource in the snapshot and returns its
/// observed state.
pub fn flux_observed_state_for_resource(
    snapshot: &ClusterSnapshot,
    resource: &ResourceRef,
) -> Option<FluxReconcileObservedState> {
    flux_resource_for_ref(snapshot, resource).map(flux_observed_state)
}

/// Returns `true` when the current resource state differs from the baseline,
/// indicating that reconciliation has made progress.
pub fn flux_reconcile_progress_observed(
    baseline: Option<&FluxReconcileObservedState>,
    current: &FluxResourceInfo,
) -> bool {
    let Some(baseline) = baseline else {
        return true;
    };

    let current_state = flux_observed_state(current);
    current_state != *baseline
}

/// Builds a human-readable one-line summary of the reconcile status.
pub fn flux_reconcile_status_summary(resource: &FluxResourceInfo) -> String {
    let mut parts = vec![format!("status {}", resource.status)];

    if let Some(revision) = resource
        .last_applied_revision
        .as_ref()
        .or(resource.last_attempted_revision.as_ref())
    {
        parts.push(format!("revision {revision}"));
    }

    parts.join(", ")
}

/// Polls all pending reconcile verifications against the current snapshot.
///
/// Completed or timed-out entries are removed from `pending` and their action
/// history is updated accordingly. The `on_status` callback is invoked for
/// each status message so the caller can route it to the transient status bar.
///
/// Returns `true` if any verification resolved (completed or timed out).
pub fn process_flux_reconcile_verifications(
    app: &mut AppState,
    snapshot: &ClusterSnapshot,
    pending: &mut Vec<PendingFluxReconcileVerification>,
    on_status: &mut dyn FnMut(&mut AppState, String),
) -> bool {
    let mut changed = false;
    let now = Instant::now();
    let mut remaining = Vec::with_capacity(pending.len());

    for verification in pending.drain(..) {
        if let Some(current) = flux_resource_for_ref(snapshot, &verification.resource)
            && flux_reconcile_progress_observed(verification.baseline.as_ref(), current)
        {
            let message = format!(
                "Flux reconcile observed for {} ({})",
                verification.resource_label,
                flux_reconcile_status_summary(current)
            );
            app.complete_action_history(
                verification.action_history_id,
                ActionStatus::Succeeded,
                message.clone(),
                true,
            );
            on_status(app, message);
            changed = true;
            continue;
        }

        if now >= verification.deadline {
            let message = format!(
                "Reconcile requested for {}. Waiting for controller status update.",
                verification.resource_label
            );
            app.complete_action_history(
                verification.action_history_id,
                ActionStatus::Succeeded,
                message.clone(),
                true,
            );
            on_status(app, message);
            changed = true;
            continue;
        }

        remaining.push(verification);
    }

    *pending = remaining;
    changed
}
