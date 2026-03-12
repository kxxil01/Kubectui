//! Canonical action history model for mutating operations.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};

use crate::app::{AppView, ResourceRef};

pub const MAX_ACTION_HISTORY_ENTRIES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Delete,
    Scale,
    Restart,
    FluxReconcile,
    ApplyYaml,
    Trigger,
    Suspend,
    Resume,
    Cordon,
    Uncordon,
    Drain,
}

impl ActionKind {
    pub const fn label(self) -> &'static str {
        match self {
            ActionKind::Delete => "Delete",
            ActionKind::Scale => "Scale",
            ActionKind::Restart => "Restart",
            ActionKind::FluxReconcile => "Reconcile",
            ActionKind::ApplyYaml => "Apply YAML",
            ActionKind::Trigger => "Trigger",
            ActionKind::Suspend => "Suspend",
            ActionKind::Resume => "Resume",
            ActionKind::Cordon => "Cordon",
            ActionKind::Uncordon => "Uncordon",
            ActionKind::Drain => "Drain",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionStatus {
    Pending,
    Succeeded,
    Failed,
}

impl ActionStatus {
    pub const fn label(self) -> &'static str {
        match self {
            ActionStatus::Pending => "Pending",
            ActionStatus::Succeeded => "Succeeded",
            ActionStatus::Failed => "Failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionHistoryTarget {
    pub view: AppView,
    pub resource: ResourceRef,
}

#[derive(Debug, Clone)]
pub struct ActionHistoryEntry {
    pub id: u64,
    pub kind: ActionKind,
    pub status: ActionStatus,
    pub resource_label: String,
    pub message: String,
    pub target: Option<ActionHistoryTarget>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct ActionHistoryState {
    entries: VecDeque<ActionHistoryEntry>,
    next_id: u64,
}

impl Default for ActionHistoryState {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            next_id: 1,
        }
    }
}

impl ActionHistoryState {
    pub fn record_pending(
        &mut self,
        kind: ActionKind,
        resource_label: impl Into<String>,
        message: impl Into<String>,
        target: Option<ActionHistoryTarget>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.entries.push_front(ActionHistoryEntry {
            id,
            kind,
            status: ActionStatus::Pending,
            resource_label: resource_label.into(),
            message: message.into(),
            target,
            started_at: Utc::now(),
            finished_at: None,
        });
        self.trim_to_limit();
        id
    }

    pub fn complete(
        &mut self,
        id: u64,
        status: ActionStatus,
        message: impl Into<String>,
        keep_target: bool,
    ) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
            entry.status = status;
            entry.message = message.into();
            entry.finished_at = Some(Utc::now());
            if !keep_target {
                entry.target = None;
            }
        }
    }

    pub fn entries(&self) -> &VecDeque<ActionHistoryEntry> {
        &self.entries
    }

    pub fn get(&self, index: usize) -> Option<&ActionHistoryEntry> {
        self.entries.get(index)
    }

    pub fn find_by_id(&self, id: u64) -> Option<&ActionHistoryEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    fn trim_to_limit(&mut self) {
        while self.entries.len() > MAX_ACTION_HISTORY_ENTRIES {
            self.entries.pop_back();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> ActionHistoryTarget {
        ActionHistoryTarget {
            view: AppView::Pods,
            resource: ResourceRef::Pod("pod-0".to_string(), "default".to_string()),
        }
    }

    #[test]
    fn record_pending_prepends_newest_entry() {
        let mut state = ActionHistoryState::default();
        let first = state.record_pending(ActionKind::Delete, "Pod a", "Deleting Pod a", None);
        let second = state.record_pending(ActionKind::Scale, "Deployment b", "Scaling b", None);

        assert_ne!(first, second);
        assert_eq!(
            state.entries().front().map(|entry| entry.kind),
            Some(ActionKind::Scale)
        );
        assert_eq!(state.entries().len(), 2);
    }

    #[test]
    fn complete_updates_entry_status_and_message() {
        let mut state = ActionHistoryState::default();
        let id = state.record_pending(
            ActionKind::Restart,
            "Deployment api",
            "Requesting restart",
            Some(target()),
        );

        state.complete(id, ActionStatus::Succeeded, "Restart requested", true);

        let entry = state.get(0).expect("entry");
        assert_eq!(entry.status, ActionStatus::Succeeded);
        assert_eq!(entry.message, "Restart requested");
        assert!(entry.finished_at.is_some());
        assert!(entry.target.is_some());
    }

    #[test]
    fn complete_can_drop_jump_target() {
        let mut state = ActionHistoryState::default();
        let id = state.record_pending(
            ActionKind::Delete,
            "Pod api-0",
            "Deleting Pod",
            Some(target()),
        );

        state.complete(id, ActionStatus::Succeeded, "Deleted Pod", false);

        let entry = state.get(0).expect("entry");
        assert!(entry.target.is_none());
    }

    #[test]
    fn node_ops_action_kinds_have_labels() {
        assert_eq!(ActionKind::Cordon.label(), "Cordon");
        assert_eq!(ActionKind::Uncordon.label(), "Uncordon");
        assert_eq!(ActionKind::Drain.label(), "Drain");
        assert_eq!(ActionKind::Suspend.label(), "Suspend");
        assert_eq!(ActionKind::Resume.label(), "Resume");
    }

    #[test]
    fn history_is_bounded() {
        let mut state = ActionHistoryState::default();
        for idx in 0..(MAX_ACTION_HISTORY_ENTRIES + 8) {
            state.record_pending(
                ActionKind::ApplyYaml,
                format!("Resource {idx}"),
                "Applying YAML",
                None,
            );
        }

        assert_eq!(state.entries().len(), MAX_ACTION_HISTORY_ENTRIES);
    }
}
