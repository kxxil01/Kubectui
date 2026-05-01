//! Canonical workbench state for long-lived bottom-pane surfaces.

use std::collections::{BTreeMap, HashSet};

use crate::{
    action_history::ActionHistoryState,
    app::{LogsViewerState, ResourceRef},
    authorization::{ActionAccessReview, DetailActionAuthorization, ResourceAccessCheck},
    icons::tab_icon,
    k8s::{
        client::EventInfo,
        dtos::HelmReleaseRevisionInfo,
        rollout::{RolloutInspection, RolloutRevisionInfo, RolloutWorkloadKind},
        workload_logs::MAX_WORKLOAD_LOG_STREAMS,
    },
    log_investigation::{
        LogEntry, LogFilterSpec, LogQueryMode, LogTimeWindow, WorkloadLogPreset, compile_query,
        entry_matches_filters, format_jump_target, nearest_timestamp_index, parse_jump_target,
    },
    network_policy_analysis::NetworkPolicyAnalysis,
    network_policy_connectivity::ConnectivityAnalysis,
    rbac_subjects::{SubjectAccessReview, SubjectBindingResolution},
    resource_diff::{
        ResourceDiffBaselineKind, ResourceDiffLine, ResourceDiffResult, YamlDocumentDiffResult,
    },
    runbooks::{LoadedRunbook, LoadedRunbookStep},
    secret::DecodedSecretEntry,
    timeline::{TimelineEntry, build_timeline},
    traffic_debug::TrafficDebugAnalysis,
    ui::{
        clear_input_at_cursor,
        components::{input_field::InputFieldWidget, port_forward_dialog::PortForwardDialog},
        move_cursor_end,
    },
};

pub const DEFAULT_WORKBENCH_HEIGHT: u16 = 12;
pub const MIN_WORKBENCH_HEIGHT: u16 = 8;
pub const MAX_WORKBENCH_HEIGHT: u16 = 40;
pub const MAX_WORKLOAD_LOG_LINES: usize = 5_000;
pub const MAX_EXEC_OUTPUT_LINES: usize = 5_000;
pub const MAX_EXTENSION_OUTPUT_LINES: usize = 5_000;
pub const MAX_TIMELINE_EVENTS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkbenchTabKind {
    ActionHistory,
    AccessReview,
    ResourceYaml,
    ResourceDiff,
    Rollout,
    Helm,
    DecodedSecret,
    ResourceEvents,
    PodLogs,
    WorkloadLogs,
    Exec,
    Extension,
    AiAnalysis,
    Runbook,
    PortForward,
    Relations,
    NetworkPolicy,
    Connectivity,
    TrafficDebug,
}

impl WorkbenchTabKind {
    pub const fn title(self) -> &'static str {
        match self {
            WorkbenchTabKind::ActionHistory => "History",
            WorkbenchTabKind::AccessReview => "Access",
            WorkbenchTabKind::ResourceYaml => "YAML",
            WorkbenchTabKind::ResourceDiff => "Drift",
            WorkbenchTabKind::Rollout => "Rollout",
            WorkbenchTabKind::Helm => "Helm",
            WorkbenchTabKind::DecodedSecret => "Decoded",
            WorkbenchTabKind::ResourceEvents => "Timeline",
            WorkbenchTabKind::PodLogs => "Logs",
            WorkbenchTabKind::WorkloadLogs => "Workload Logs",
            WorkbenchTabKind::Exec => "Exec",
            WorkbenchTabKind::Extension => "Extension",
            WorkbenchTabKind::AiAnalysis => "AI",
            WorkbenchTabKind::Runbook => "Runbook",
            WorkbenchTabKind::PortForward => "Port-Forward",
            WorkbenchTabKind::Relations => "Relations",
            WorkbenchTabKind::NetworkPolicy => "NetPol",
            WorkbenchTabKind::Connectivity => "Reach",
            WorkbenchTabKind::TrafficDebug => "Traffic",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WorkbenchTabKey {
    ActionHistory,
    AccessReview(ResourceRef),
    ResourceYaml(ResourceRef),
    ResourceDiff(ResourceRef),
    Rollout(ResourceRef),
    HelmHistory(ResourceRef),
    DecodedSecret(ResourceRef),
    ResourceEvents(ResourceRef),
    PodLogs(ResourceRef),
    WorkloadLogs(ResourceRef),
    Exec(ResourceRef),
    ExtensionOutput(u64),
    AiAnalysis(u64),
    Runbook(String, Option<ResourceRef>),
    PortForward,
    Relations(ResourceRef),
    NetworkPolicy(ResourceRef),
    Connectivity(ResourceRef),
    TrafficDebug(ResourceRef),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelmValuesDiffState {
    pub current_revision: i32,
    pub target_revision: i32,
    pub summary: Option<String>,
    pub lines: Vec<ResourceDiffLine>,
    pub scroll: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub pending_request_id: Option<u64>,
}

impl HelmValuesDiffState {
    pub fn new(current_revision: i32, target_revision: i32, request_id: u64) -> Self {
        Self {
            current_revision,
            target_revision,
            summary: None,
            lines: Vec::new(),
            scroll: 0,
            loading: true,
            error: None,
            pending_request_id: Some(request_id),
        }
    }

    pub fn apply_result(&mut self, diff: YamlDocumentDiffResult) {
        self.summary = Some(diff.summary);
        self.lines = diff.lines;
        self.scroll = 0;
        self.loading = false;
        self.error = None;
        self.pending_request_id = None;
    }

    pub fn set_error(&mut self, error: String) {
        self.summary = None;
        self.lines.clear();
        self.scroll = 0;
        self.loading = false;
        self.error = Some(error);
        self.pending_request_id = None;
    }
}

#[derive(Debug, Clone)]
pub struct HelmHistoryTabState {
    pub resource: ResourceRef,
    pub pending_history_request_id: Option<u64>,
    pub pending_rollback_action_history_id: Option<u64>,
    pub cli_version: Option<String>,
    pub revisions: Vec<HelmReleaseRevisionInfo>,
    pub scroll: usize,
    pub selected: usize,
    pub current_revision: Option<i32>,
    pub loading: bool,
    pub error: Option<String>,
    pub diff: Option<HelmValuesDiffState>,
    pub confirm_rollback_revision: Option<i32>,
    pub rollback_pending: bool,
}

impl HelmHistoryTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            pending_history_request_id: None,
            pending_rollback_action_history_id: None,
            cli_version: None,
            revisions: Vec::new(),
            scroll: 0,
            selected: 0,
            current_revision: None,
            loading: true,
            error: None,
            diff: None,
            confirm_rollback_revision: None,
            rollback_pending: false,
        }
    }

    pub fn apply_history(&mut self, history: crate::k8s::helm::HelmHistoryResult) {
        let selected_revision = self.selected_revision().map(|entry| entry.revision);
        let scroll = self.scroll;
        self.cli_version = Some(history.cli_version);
        self.revisions = history.revisions;
        self.current_revision = self.revisions.iter().map(|entry| entry.revision).max();
        self.selected = selected_revision
            .and_then(|revision| {
                self.revisions
                    .iter()
                    .position(|entry| entry.revision == revision)
            })
            .unwrap_or(0)
            .min(self.revisions.len().saturating_sub(1));
        self.scroll = scroll.min(self.revisions.len().saturating_sub(1));
        self.loading = false;
        self.error = None;
        self.pending_history_request_id = None;
        self.pending_rollback_action_history_id = None;
        self.confirm_rollback_revision = None;
        self.rollback_pending = false;
        self.diff = None;
    }

    pub fn set_history_error(&mut self, error: String) {
        self.revisions.clear();
        self.scroll = 0;
        self.selected = 0;
        self.current_revision = None;
        self.loading = false;
        self.error = Some(error);
        self.pending_history_request_id = None;
        self.pending_rollback_action_history_id = None;
        self.diff = None;
        self.confirm_rollback_revision = None;
        self.rollback_pending = false;
    }

    pub fn refresh(&mut self, request_id: u64) {
        self.scroll = 0;
        self.loading = true;
        self.error = None;
        self.pending_history_request_id = Some(request_id);
        self.confirm_rollback_revision = None;
        self.diff = None;
    }

    pub fn select_next(&mut self) {
        if !self.revisions.is_empty() {
            self.selected = (self.selected + 1).min(self.revisions.len().saturating_sub(1));
        }
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_top(&mut self) {
        self.selected = 0;
    }

    pub fn select_bottom(&mut self) {
        self.selected = self.revisions.len().saturating_sub(1);
    }

    pub fn selected_revision(&self) -> Option<&HelmReleaseRevisionInfo> {
        self.revisions
            .get(self.selected.min(self.revisions.len().saturating_sub(1)))
    }

    pub fn selected_target_revision(&self) -> Option<i32> {
        let selected = self.selected_revision()?.revision;
        (Some(selected) != self.current_revision).then_some(selected)
    }

    pub fn begin_diff(&mut self, current_revision: i32, target_revision: i32, request_id: u64) {
        self.scroll = 0;
        self.confirm_rollback_revision = None;
        self.diff = Some(HelmValuesDiffState::new(
            current_revision,
            target_revision,
            request_id,
        ));
    }

    pub fn close_diff(&mut self) {
        self.scroll = 0;
        self.diff = None;
    }

    pub fn begin_rollback_confirm(&mut self, revision: i32) {
        self.scroll = 0;
        self.confirm_rollback_revision = Some(revision);
    }

    pub fn cancel_rollback_confirm(&mut self) {
        self.scroll = 0;
        self.confirm_rollback_revision = None;
    }

    pub fn begin_rollback(&mut self, action_history_id: u64) {
        self.scroll = 0;
        self.rollback_pending = true;
        self.pending_rollback_action_history_id = Some(action_history_id);
        self.confirm_rollback_revision = None;
        self.diff = None;
    }

    pub fn clear_rollback_if_matches(&mut self, action_history_id: u64) {
        if self.pending_rollback_action_history_id == Some(action_history_id) {
            self.rollback_pending = false;
            self.pending_rollback_action_history_id = None;
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActionHistoryTabState {
    pub selected: usize,
    selected_entry_id: Option<u64>,
}

impl ActionHistoryTabState {
    pub fn selected_index(&self, visible_ids: &[u64]) -> usize {
        if visible_ids.is_empty() {
            return 0;
        }
        self.selected_entry_id
            .and_then(|id| visible_ids.iter().position(|entry_id| *entry_id == id))
            .unwrap_or_else(|| self.selected.min(visible_ids.len().saturating_sub(1)))
    }

    pub fn sync_selection(&mut self, visible_ids: &[u64]) {
        let selected = self.selected_index(visible_ids);
        self.selected = selected;
        self.selected_entry_id = visible_ids.get(selected).copied();
    }

    pub fn select_next(&mut self, visible_ids: &[u64]) {
        self.sync_selection(visible_ids);
        if visible_ids.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(visible_ids.len().saturating_sub(1));
        self.selected_entry_id = visible_ids.get(self.selected).copied();
    }

    pub fn select_previous(&mut self, visible_ids: &[u64]) {
        self.sync_selection(visible_ids);
        self.selected = self.selected.saturating_sub(1);
        self.selected_entry_id = visible_ids.get(self.selected).copied();
    }

    pub fn select_top(&mut self, visible_ids: &[u64]) {
        self.selected = 0;
        self.selected_entry_id = visible_ids.first().copied();
    }

    pub fn select_bottom(&mut self, visible_ids: &[u64]) {
        self.selected = visible_ids.len().saturating_sub(1);
        self.selected_entry_id = visible_ids.get(self.selected).copied();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessReviewFocus {
    Summary,
    SubjectInput,
}

#[derive(Debug, Clone)]
pub struct AccessReviewTabState {
    pub resource: ResourceRef,
    pub context_name: Option<String>,
    pub namespace_scope: String,
    pub entries: Vec<ActionAccessReview>,
    pub subject_review: Option<SubjectAccessReview>,
    pub attempted_review: Option<AttemptedActionReview>,
    pub focus: AccessReviewFocus,
    pub subject_input: InputFieldWidget,
    pub subject_input_error: Option<String>,
    pub scroll: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptedActionReview {
    pub action: crate::policy::DetailAction,
    pub authorization: Option<DetailActionAuthorization>,
    pub strict: bool,
    pub checks: Vec<ResourceAccessCheck>,
    pub note: Option<String>,
}

impl AccessReviewTabState {
    pub fn new(
        resource: ResourceRef,
        context_name: Option<String>,
        namespace_scope: String,
        entries: Vec<ActionAccessReview>,
        subject_review: Option<SubjectAccessReview>,
        attempted_review: Option<AttemptedActionReview>,
    ) -> Self {
        let subject_input_value = subject_review
            .as_ref()
            .map(|review| review.subject.spec())
            .unwrap_or_default();
        let mut tab = Self {
            resource,
            context_name,
            namespace_scope,
            entries,
            subject_review,
            attempted_review,
            focus: AccessReviewFocus::Summary,
            subject_input: InputFieldWidget::with_value(&subject_input_value, 128),
            subject_input_error: None,
            scroll: 0,
        };
        if tab.attempted_review.is_some() {
            tab.scroll = 3;
        }
        tab
    }

    pub fn line_count(&self) -> usize {
        let header_lines = 4usize;
        let attempted_lines = self
            .attempted_review
            .as_ref()
            .map_or(0, Self::attempted_review_line_count);
        let subject_input_lines = self.subject_input_line_count();
        let subject_lines = self
            .subject_review
            .as_ref()
            .map_or(0, Self::subject_review_line_count);
        let entry_lines = self
            .entries
            .iter()
            .map(Self::entry_line_count)
            .sum::<usize>();
        header_lines + attempted_lines + subject_input_lines + subject_lines + entry_lines
    }

    pub fn refresh_payload(
        &mut self,
        context_name: Option<String>,
        namespace_scope: String,
        entries: Vec<ActionAccessReview>,
        subject_review: Option<SubjectAccessReview>,
        attempted_review: Option<AttemptedActionReview>,
    ) {
        self.context_name = context_name;
        self.namespace_scope = namespace_scope;
        self.entries = entries;
        self.subject_review = subject_review;
        self.attempted_review = attempted_review;
        self.scroll = self.scroll.min(self.line_count().saturating_sub(1));
    }

    #[cfg(test)]
    fn action_line_offset(&self, action: crate::policy::DetailAction) -> Option<usize> {
        let mut offset = 4usize;
        if let Some(review) = &self.attempted_review {
            offset += Self::attempted_review_line_count(review);
        }
        offset += self.subject_input_line_count();
        if let Some(review) = &self.subject_review {
            offset += Self::subject_review_line_count(review);
        }
        for entry in &self.entries {
            if entry.action == action {
                return Some(offset.saturating_sub(1));
            }
            offset += Self::entry_line_count(entry);
        }
        None
    }

    pub fn subject_review_offset(&self) -> usize {
        let mut offset = 4usize;
        if let Some(review) = &self.attempted_review {
            offset += Self::attempted_review_line_count(review);
        }
        offset + self.subject_input_line_count()
    }

    pub fn subject_input_offset(&self) -> usize {
        let mut offset = 4usize;
        if let Some(review) = &self.attempted_review {
            offset += Self::attempted_review_line_count(review);
        }
        offset
    }

    fn attempted_review_line_count(review: &AttemptedActionReview) -> usize {
        let grouped_scope_headers =
            usize::from(review.checks.iter().any(|check| check.namespace.is_some()))
                + usize::from(review.checks.iter().any(|check| check.namespace.is_none()));
        3 + usize::from(review.note.is_some())
            + if review.checks.is_empty() {
                1
            } else {
                1 + review.checks.len() + grouped_scope_headers
            }
    }

    fn subject_input_line_count(&self) -> usize {
        3usize + usize::from(self.subject_input_error.is_some())
    }

    fn subject_review_line_count(review: &SubjectAccessReview) -> usize {
        3 + review
            .bindings
            .iter()
            .map(Self::subject_binding_line_count)
            .sum::<usize>()
    }

    fn subject_binding_line_count(binding: &SubjectBindingResolution) -> usize {
        3 + binding.role.rules.len().max(1)
    }

    fn entry_line_count(entry: &ActionAccessReview) -> usize {
        let grouped_scope_headers =
            usize::from(entry.checks.iter().any(|check| check.namespace.is_some()))
                + usize::from(entry.checks.iter().any(|check| check.namespace.is_none()));

        2 + if entry.checks.is_empty() {
            1
        } else {
            entry.checks.len() + grouped_scope_headers
        }
    }

    pub fn start_subject_input(&mut self) {
        self.focus = AccessReviewFocus::SubjectInput;
        self.subject_input.focused = true;
        self.subject_input.cursor_end();
        self.scroll = self.subject_input_offset();
    }

    pub fn stop_subject_input(&mut self) {
        self.focus = AccessReviewFocus::Summary;
        self.subject_input.focused = false;
    }
}

#[derive(Debug, Clone)]
pub struct ResourceYamlTabState {
    pub resource: ResourceRef,
    pub pending_request_id: Option<u64>,
    pub yaml: Option<String>,
    pub scroll: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl ResourceYamlTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            pending_request_id: None,
            yaml: None,
            scroll: 0,
            loading: true,
            error: None,
        }
    }

    pub fn update_content(
        &mut self,
        yaml: Option<String>,
        error: Option<String>,
        pending_request_id: Option<u64>,
    ) {
        let content_updated = yaml.is_some() || error.is_some() || pending_request_id.is_none();
        if content_updated {
            self.yaml = yaml;
            let total_lines = self
                .yaml
                .as_ref()
                .map(|yaml| yaml.lines().count())
                .unwrap_or(0);
            self.scroll = self.scroll.min(total_lines.saturating_sub(1));
        }
        self.loading = pending_request_id.is_some() || (self.yaml.is_none() && error.is_none());
        self.error = error;
        self.pending_request_id = pending_request_id;
    }
}

#[derive(Debug, Clone)]
pub struct ResourceDiffTabState {
    pub resource: ResourceRef,
    pub pending_request_id: Option<u64>,
    pub baseline_kind: Option<ResourceDiffBaselineKind>,
    pub summary: Option<String>,
    pub lines: Vec<ResourceDiffLine>,
    pub scroll: usize,
    pub loading: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RolloutMutationState {
    Restart,
    Pause,
    Resume,
    Undo(i64),
}

#[derive(Debug, Clone)]
pub struct RolloutTabState {
    pub resource: ResourceRef,
    pub pending_request_id: Option<u64>,
    pub kind: Option<RolloutWorkloadKind>,
    pub strategy: Option<String>,
    pub paused: bool,
    pub current_revision: Option<i64>,
    pub update_target_revision: Option<i64>,
    pub summary_lines: Vec<String>,
    pub conditions: Vec<crate::k8s::rollout::RolloutConditionInfo>,
    pub revisions: Vec<RolloutRevisionInfo>,
    pub selected: usize,
    pub detail_scroll: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub confirm_undo_revision: Option<i64>,
    pub mutation_pending: Option<RolloutMutationState>,
    pub pending_mutation_action_history_id: Option<u64>,
}

impl RolloutTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            pending_request_id: None,
            kind: None,
            strategy: None,
            paused: false,
            current_revision: None,
            update_target_revision: None,
            summary_lines: Vec::new(),
            conditions: Vec::new(),
            revisions: Vec::new(),
            selected: 0,
            detail_scroll: 0,
            loading: true,
            error: None,
            confirm_undo_revision: None,
            mutation_pending: None,
            pending_mutation_action_history_id: None,
        }
    }

    pub fn apply_inspection(&mut self, inspection: RolloutInspection) {
        let selected_revision = self.selected_revision().map(|entry| entry.revision);
        let detail_scroll = self.detail_scroll;
        self.kind = Some(inspection.kind);
        self.strategy = Some(inspection.strategy);
        self.paused = inspection.paused;
        self.current_revision = inspection.current_revision;
        self.update_target_revision = inspection.update_target_revision;
        self.summary_lines = inspection.summary_lines;
        self.conditions = inspection.conditions;
        self.revisions = inspection.revisions;
        self.selected = selected_revision
            .and_then(|revision| {
                self.revisions
                    .iter()
                    .position(|entry| entry.revision == revision)
            })
            .unwrap_or(0)
            .min(self.revisions.len().saturating_sub(1));
        self.detail_scroll = detail_scroll;
        self.loading = false;
        self.error = None;
        self.pending_request_id = None;
        self.confirm_undo_revision = None;
        self.mutation_pending = None;
        self.pending_mutation_action_history_id = None;
    }

    pub fn set_error(&mut self, error: String) {
        self.kind = None;
        self.strategy = None;
        self.paused = false;
        self.current_revision = None;
        self.update_target_revision = None;
        self.summary_lines.clear();
        self.conditions.clear();
        self.revisions.clear();
        self.selected = 0;
        self.detail_scroll = 0;
        self.loading = false;
        self.error = Some(error);
        self.pending_request_id = None;
        self.confirm_undo_revision = None;
        self.mutation_pending = None;
        self.pending_mutation_action_history_id = None;
    }

    pub fn refresh(&mut self, request_id: u64) {
        self.detail_scroll = 0;
        self.loading = true;
        self.error = None;
        self.pending_request_id = Some(request_id);
        self.confirm_undo_revision = None;
    }

    pub fn select_next(&mut self) {
        if !self.revisions.is_empty() {
            self.selected = (self.selected + 1).min(self.revisions.len().saturating_sub(1));
        }
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_top(&mut self) {
        self.selected = 0;
    }

    pub fn select_bottom(&mut self) {
        self.selected = self.revisions.len().saturating_sub(1);
    }

    pub fn selected_revision(&self) -> Option<&RolloutRevisionInfo> {
        self.revisions
            .get(self.selected.min(self.revisions.len().saturating_sub(1)))
    }

    pub fn selected_undo_revision(&self) -> Option<i64> {
        let selected = self.selected_revision()?.revision;
        (Some(selected) != self.current_revision).then_some(selected)
    }

    pub fn begin_undo_confirm(&mut self, revision: i64) {
        self.detail_scroll = 0;
        self.confirm_undo_revision = Some(revision);
    }

    pub fn cancel_undo_confirm(&mut self) {
        self.detail_scroll = 0;
        self.confirm_undo_revision = None;
    }

    pub fn begin_mutation(&mut self, mutation: RolloutMutationState, action_history_id: u64) {
        self.detail_scroll = 0;
        self.confirm_undo_revision = None;
        self.mutation_pending = Some(mutation);
        self.pending_mutation_action_history_id = Some(action_history_id);
    }

    pub fn clear_mutation_if_matches(&mut self, action_history_id: u64) {
        if self.pending_mutation_action_history_id == Some(action_history_id) {
            self.mutation_pending = None;
            self.pending_mutation_action_history_id = None;
        }
    }
}

impl ResourceDiffTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            pending_request_id: None,
            baseline_kind: None,
            summary: None,
            lines: Vec::new(),
            scroll: 0,
            loading: true,
            error: None,
        }
    }

    pub fn apply_result(&mut self, diff: ResourceDiffResult) {
        let scroll = self.scroll;
        self.baseline_kind = Some(diff.baseline_kind);
        self.summary = Some(diff.summary);
        self.lines = diff.lines;
        self.scroll = scroll.min(self.lines.len().saturating_sub(1));
        self.loading = false;
        self.error = None;
        self.pending_request_id = None;
    }

    pub fn set_error(&mut self, error: String) {
        self.baseline_kind = None;
        self.summary = None;
        self.lines.clear();
        self.scroll = 0;
        self.loading = false;
        self.error = Some(error);
        self.pending_request_id = None;
    }

    pub fn refresh(&mut self, request_id: Option<u64>) {
        self.baseline_kind = None;
        self.summary = None;
        self.lines.clear();
        self.scroll = 0;
        self.loading = true;
        self.error = None;
        self.pending_request_id = request_id;
    }
}

#[derive(Debug, Clone)]
pub struct DecodedSecretTabState {
    pub resource: ResourceRef,
    pub pending_request_id: Option<u64>,
    pub source_yaml: Option<String>,
    pub entries: Vec<DecodedSecretEntry>,
    pub selected: usize,
    selected_key: Option<String>,
    pub scroll: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub masked: bool,
    pub editing: bool,
    pub edit_input: String,
    pub edit_cursor: usize,
}

impl DecodedSecretTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            pending_request_id: None,
            source_yaml: None,
            entries: Vec::new(),
            selected: 0,
            selected_key: None,
            scroll: 0,
            loading: true,
            error: None,
            masked: true,
            editing: false,
            edit_input: String::new(),
            edit_cursor: 0,
        }
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.entries.iter().any(DecodedSecretEntry::is_dirty)
    }

    pub fn has_local_edit_state(&self) -> bool {
        self.editing || self.has_unsaved_changes()
    }

    pub fn selected_index(&self) -> usize {
        if self.entries.is_empty() {
            return 0;
        }
        self.selected_key
            .as_ref()
            .and_then(|key| self.entries.iter().position(|entry| &entry.key == key))
            .unwrap_or_else(|| self.selected.min(self.entries.len().saturating_sub(1)))
    }

    pub fn selected_entry(&self) -> Option<&DecodedSecretEntry> {
        self.entries.get(self.selected_index())
    }

    pub fn selected_entry_mut(&mut self) -> Option<&mut DecodedSecretEntry> {
        let selected = self.selected_index();
        self.selected = selected;
        self.selected_key = self.entries.get(selected).map(|entry| entry.key.clone());
        self.entries.get_mut(selected)
    }

    pub fn select_next(&mut self) {
        if self.entries.is_empty() {
            self.selected = 0;
            self.selected_key = None;
            return;
        }
        let selected = self.selected_index();
        self.selected = (selected + 1).min(self.entries.len().saturating_sub(1));
        self.selected_key = self
            .entries
            .get(self.selected)
            .map(|entry| entry.key.clone());
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected_index().saturating_sub(1);
        self.selected_key = self
            .entries
            .get(self.selected)
            .map(|entry| entry.key.clone());
    }

    pub fn select_top(&mut self) {
        self.selected = 0;
        self.selected_key = self.entries.first().map(|entry| entry.key.clone());
    }

    pub fn select_bottom(&mut self) {
        self.selected = self.entries.len().saturating_sub(1);
        self.selected_key = self
            .entries
            .get(self.selected)
            .map(|entry| entry.key.clone());
    }

    pub fn clamp_selected(&mut self) {
        let max = self.entries.len().saturating_sub(1);
        self.selected = self.selected_index().min(max);
        self.selected_key = self
            .entries
            .get(self.selected)
            .map(|entry| entry.key.clone());
        self.scroll = self.scroll.min(max);
    }
}

#[derive(Debug, Clone)]
pub struct ResourceEventsTabState {
    pub resource: ResourceRef,
    pub pending_request_id: Option<u64>,
    pub events: Vec<EventInfo>,
    pub timeline: Vec<TimelineEntry>,
    pub scroll: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl ResourceEventsTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            pending_request_id: None,
            events: Vec::new(),
            timeline: Vec::new(),
            scroll: 0,
            loading: true,
            error: None,
        }
    }

    /// Rebuild the merged timeline from current events + action history.
    pub fn rebuild_timeline(&mut self, history: &ActionHistoryState) {
        // Cap events to avoid unbounded growth from noisy controllers.
        if self.events.len() > MAX_TIMELINE_EVENTS {
            let drain = self.events.len() - MAX_TIMELINE_EVENTS;
            self.events.drain(..drain);
        }
        self.timeline = build_timeline(&self.events, history.entries(), &self.resource);
        self.scroll = self.scroll.min(self.timeline.len().saturating_sub(1));
    }
}

#[derive(Debug, Clone)]
pub struct PodLogsTabState {
    pub resource: ResourceRef,
    pub viewer: LogsViewerState,
}

impl PodLogsTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            viewer: LogsViewerState::default(),
        }
    }

    pub fn restart_viewer_for_pod(
        &mut self,
        pod_name: String,
        pod_namespace: String,
        request_id: u64,
    ) {
        self.viewer
            .restart_for_pod(pod_name, pod_namespace, request_id);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadLogLine {
    pub pod_name: String,
    pub container_name: String,
    pub entry: LogEntry,
    pub is_stderr: bool,
}

#[derive(Debug, Clone)]
pub struct WorkloadLogsTabState {
    pub resource: ResourceRef,
    pub session_id: u64,
    pub sources: Vec<(String, String, String)>,
    pub pod_labels: BTreeMap<String, Vec<String>>,
    pub lines: Vec<WorkloadLogLine>,
    pub scroll: usize,
    pub follow_mode: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub text_filter: String,
    pub text_filter_mode: LogQueryMode,
    pub compiled_text_filter: Option<regex::Regex>,
    pub text_filter_error: Option<String>,
    pub time_window: LogTimeWindow,
    pub correlation_request_id: Option<String>,
    pub filter_input: String,
    pub filter_input_cursor: usize,
    pub editing_text_filter: bool,
    pub time_jump_input: String,
    pub time_jump_cursor: usize,
    pub jumping_to_time: bool,
    pub time_jump_error: Option<String>,
    pub structured_view: bool,
    pub label_filter: Option<String>,
    pub pod_filter: Option<String>,
    pub container_filter: Option<String>,
    pub available_labels: Vec<String>,
    pub matching_label_pods: Option<HashSet<String>>,
    pub available_pods: Vec<String>,
    pub available_containers: Vec<String>,
    filtered_line_anchor: Option<WorkloadLogLine>,
}

impl WorkloadLogsTabState {
    pub fn new(resource: ResourceRef, session_id: u64) -> Self {
        Self {
            resource,
            session_id,
            sources: Vec::new(),
            pod_labels: BTreeMap::new(),
            lines: Vec::new(),
            scroll: 0,
            follow_mode: true,
            loading: true,
            error: None,
            notice: None,
            text_filter: String::new(),
            text_filter_mode: LogQueryMode::Substring,
            compiled_text_filter: None,
            text_filter_error: None,
            time_window: LogTimeWindow::All,
            correlation_request_id: None,
            filter_input: String::new(),
            filter_input_cursor: 0,
            editing_text_filter: false,
            time_jump_input: String::new(),
            time_jump_cursor: 0,
            jumping_to_time: false,
            time_jump_error: None,
            structured_view: true,
            label_filter: None,
            pod_filter: None,
            container_filter: None,
            available_labels: Vec::new(),
            matching_label_pods: None,
            available_pods: Vec::new(),
            available_containers: Vec::new(),
            filtered_line_anchor: None,
        }
    }

    pub fn restart_session(&mut self, session_id: u64) {
        self.session_id = session_id;
        self.sources.clear();
        self.pod_labels.clear();
        self.lines.clear();
        self.scroll = 0;
        self.loading = true;
        self.error = None;
        self.notice = None;
        self.correlation_request_id = None;
        self.filter_input = self.text_filter.clone();
        move_cursor_end(&mut self.filter_input_cursor, &self.filter_input);
        self.editing_text_filter = false;
        clear_input_at_cursor(&mut self.time_jump_input, &mut self.time_jump_cursor);
        self.jumping_to_time = false;
        self.time_jump_error = None;
        self.available_labels.clear();
        self.matching_label_pods = None;
        self.available_pods.clear();
        self.available_containers.clear();
        self.filtered_line_anchor = None;
    }

    pub fn update_targets(&mut self, targets: &[crate::k8s::workload_logs::WorkloadLogTarget]) {
        self.pod_labels.clear();
        self.available_labels.clear();
        self.available_pods.clear();
        self.available_containers.clear();
        let mut seen_labels = HashSet::new();
        let mut seen_containers = HashSet::new();

        for target in targets {
            self.available_pods.push(target.pod_name.clone());
            let labels = target
                .labels
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>();
            for label in &labels {
                if seen_labels.insert(label.clone()) {
                    self.available_labels.push(label.clone());
                }
            }
            for container in &target.containers {
                if seen_containers.insert(container.clone()) {
                    self.available_containers.push(container.clone());
                }
            }
            self.pod_labels.insert(target.pod_name.clone(), labels);
        }
        self.available_labels.sort();
        self.available_pods.sort();
        self.available_containers.sort();
        if self.label_filter.as_ref().is_some_and(|label| {
            !self
                .available_labels
                .iter()
                .any(|candidate| candidate == label)
        }) {
            self.label_filter = None;
        }
        if self
            .pod_filter
            .as_ref()
            .is_some_and(|pod| !self.available_pods.iter().any(|candidate| candidate == pod))
        {
            self.pod_filter = None;
        }
        if self.container_filter.as_ref().is_some_and(|container| {
            !self
                .available_containers
                .iter()
                .any(|candidate| candidate == container)
        }) {
            self.container_filter = None;
        }
        self.refresh_matching_label_pods();
    }

    pub fn apply_bootstrap_targets(
        &mut self,
        targets: Vec<crate::k8s::workload_logs::WorkloadLogTarget>,
    ) -> Vec<(String, String, String)> {
        self.update_targets(&targets);
        self.error = None;
        self.notice = None;

        let mut sources = Vec::new();
        for target in targets {
            for container in target.containers {
                if sources.len() >= MAX_WORKLOAD_LOG_STREAMS {
                    self.notice = Some(format!(
                        "Stream cap reached at {MAX_WORKLOAD_LOG_STREAMS} pod/container streams."
                    ));
                    break;
                }
                sources.push((target.pod_name.clone(), target.namespace.clone(), container));
            }
        }

        self.loading = false;
        if sources.is_empty() {
            self.sources.clear();
            self.error = Some("No pod/container streams were resolved.".to_string());
            return Vec::new();
        }

        self.sources = sources.clone();
        sources
    }

    pub fn apply_bootstrap_error(&mut self, error: String) {
        self.sources.clear();
        self.loading = false;
        self.notice = None;
        self.error = Some(error);
    }

    pub fn push_line(&mut self, line: WorkloadLogLine) {
        if !self.available_pods.iter().any(|pod| pod == &line.pod_name) {
            self.available_pods.push(line.pod_name.clone());
            self.available_pods.sort();
        }
        if !self
            .available_containers
            .iter()
            .any(|container| container == &line.container_name)
        {
            self.available_containers.push(line.container_name.clone());
            self.available_containers.sort();
        }

        self.lines.push(line);
        if self.lines.len() > MAX_WORKLOAD_LOG_LINES {
            let excess = self.lines.len() - MAX_WORKLOAD_LOG_LINES;
            self.lines.drain(..excess);
            self.scroll = self.scroll.saturating_sub(excess);
        }
        if self.follow_mode {
            // Set scroll past the end; the renderer's scroll_window clamps it
            // to the last visible filtered line. This avoids an O(n) filter
            // scan on every push.
            self.scroll = self.lines.len();
        }
    }

    pub fn commit_text_filter(&mut self) {
        if self.filter_input == self.text_filter {
            self.editing_text_filter = false;
            self.text_filter_error = None;
            self.time_jump_error = None;
            return;
        }
        let preserved_line = self.selected_filtered_line_anchor();
        self.text_filter = self.filter_input.clone();
        self.text_filter_error = None;
        self.time_jump_error = None;
        match compile_query(&self.text_filter, self.text_filter_mode) {
            Ok(compiled) => self.compiled_text_filter = compiled,
            Err(err) => {
                self.compiled_text_filter = None;
                self.text_filter_error = Some(err);
            }
        }
        self.editing_text_filter = false;
        self.restore_filtered_scroll(preserved_line);
    }

    pub fn open_time_jump(&mut self) {
        self.jumping_to_time = true;
        self.editing_text_filter = false;
        self.time_jump_error = None;
        self.time_jump_input = self
            .current_filtered_line()
            .and_then(|line| line.entry.timestamp())
            .map(format_jump_target)
            .unwrap_or_default();
        move_cursor_end(&mut self.time_jump_cursor, &self.time_jump_input);
    }

    pub fn commit_time_jump(&mut self) {
        self.time_jump_error = None;
        let target = match parse_jump_target(&self.time_jump_input) {
            Ok(target) => target,
            Err(err) => {
                self.time_jump_error = Some(err);
                return;
            }
        };
        let now = crate::time::now();
        let mut visible_ordinal = 0usize;
        let Some(index) = nearest_timestamp_index(
            self.lines.iter().filter_map(|line| {
                if !self.matches_filter_at(line, now) {
                    return None;
                }

                let index = visible_ordinal;
                visible_ordinal += 1;
                Some((index, &line.entry))
            }),
            target,
        ) else {
            self.time_jump_error = Some(
                "No workload log lines in the current investigation view have timestamps."
                    .to_string(),
            );
            return;
        };
        self.scroll = index;
        self.follow_mode = false;
        self.jumping_to_time = false;
    }

    pub fn cancel_time_jump(&mut self) {
        self.jumping_to_time = false;
        self.time_jump_error = None;
        self.time_jump_input.clear();
    }

    pub fn toggle_text_filter_mode(&mut self) {
        let preserved_line = self.selected_filtered_line_anchor();
        self.text_filter_mode = self.text_filter_mode.toggle();
        self.text_filter_error = None;
        match compile_query(&self.text_filter, self.text_filter_mode) {
            Ok(compiled) => self.compiled_text_filter = compiled,
            Err(err) => {
                self.compiled_text_filter = None;
                self.text_filter_error = Some(err);
            }
        }
        self.restore_filtered_scroll(preserved_line);
    }

    pub fn cycle_time_window(&mut self) {
        let preserved_line = self.selected_filtered_line_anchor();
        self.time_window = self.time_window.next();
        self.restore_filtered_scroll(preserved_line);
    }

    pub fn cycle_pod_filter(&mut self) {
        let preserved_line = self.selected_filtered_line_anchor();
        self.pod_filter = cycle_filter_value(&self.available_pods, self.pod_filter.as_deref());
        self.restore_filtered_scroll(preserved_line);
    }

    pub fn cycle_container_filter(&mut self) {
        let preserved_line = self.selected_filtered_line_anchor();
        self.container_filter =
            cycle_filter_value(&self.available_containers, self.container_filter.as_deref());
        self.restore_filtered_scroll(preserved_line);
    }

    pub fn cycle_label_filter(&mut self) {
        let preserved_line = self.selected_filtered_line_anchor();
        self.label_filter =
            cycle_filter_value(&self.available_labels, self.label_filter.as_deref());
        self.refresh_matching_label_pods();
        self.restore_filtered_scroll(preserved_line);
    }

    pub fn matches_filter(&self, line: &WorkloadLogLine) -> bool {
        self.matches_filter_at(line, crate::time::now())
    }

    pub fn matches_filter_at(
        &self,
        line: &WorkloadLogLine,
        now: crate::time::AppTimestamp,
    ) -> bool {
        self.pod_filter
            .as_ref()
            .is_none_or(|pod| pod == &line.pod_name)
            && self
                .matching_label_pods
                .as_ref()
                .is_none_or(|pods| pods.contains(&line.pod_name))
            && self
                .container_filter
                .as_ref()
                .is_none_or(|container| container == &line.container_name)
            && entry_matches_filters(
                &line.entry,
                LogFilterSpec {
                    query: &self.text_filter,
                    mode: self.text_filter_mode,
                    compiled: self.compiled_text_filter.as_ref(),
                    structured: self.structured_view,
                    time_window: self.time_window,
                    correlation_request_id: self.correlation_request_id.as_deref(),
                },
                now,
            )
    }

    pub fn preset_snapshot(&self) -> WorkloadLogPreset {
        WorkloadLogPreset {
            name: String::new(),
            query: self.text_filter.clone(),
            mode: self.text_filter_mode,
            time_window: self.time_window,
            structured_view: self.structured_view,
            label_filter: self.label_filter.clone(),
            pod_filter: self.pod_filter.clone(),
            container_filter: self.container_filter.clone(),
        }
    }

    pub fn apply_preset(&mut self, preset: &WorkloadLogPreset) {
        let preserved_line = self.selected_filtered_line_anchor();
        self.editing_text_filter = false;
        self.jumping_to_time = false;
        self.text_filter = preset.query.clone();
        self.filter_input = self.text_filter.clone();
        move_cursor_end(&mut self.filter_input_cursor, &self.filter_input);
        clear_input_at_cursor(&mut self.time_jump_input, &mut self.time_jump_cursor);
        self.time_jump_error = None;
        self.text_filter_mode = preset.mode;
        self.time_window = preset.time_window;
        self.correlation_request_id = None;
        self.structured_view = preset.structured_view;
        self.label_filter = preset
            .label_filter
            .as_ref()
            .filter(|label| {
                self.available_labels
                    .iter()
                    .any(|candidate| candidate == *label)
            })
            .cloned();
        self.pod_filter = preset
            .pod_filter
            .as_ref()
            .filter(|pod| {
                self.available_pods
                    .iter()
                    .any(|candidate| candidate == *pod)
            })
            .cloned();
        self.container_filter = preset
            .container_filter
            .as_ref()
            .filter(|container| {
                self.available_containers
                    .iter()
                    .any(|candidate| candidate == *container)
            })
            .cloned();
        self.text_filter_error = None;
        match compile_query(&self.text_filter, self.text_filter_mode) {
            Ok(compiled) => self.compiled_text_filter = compiled,
            Err(err) => {
                self.compiled_text_filter = None;
                self.text_filter_error = Some(err);
            }
        }
        self.refresh_matching_label_pods();
        self.follow_mode = false;
        self.restore_filtered_scroll(preserved_line);
    }

    pub fn filtered_indices(&self) -> Vec<usize> {
        let now = crate::time::now();
        self.lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| self.matches_filter_at(line, now).then_some(index))
            .collect()
    }

    pub fn filtered_len(&self) -> usize {
        let now = crate::time::now();
        self.lines
            .iter()
            .filter(|line| self.matches_filter_at(line, now))
            .count()
    }

    pub fn current_filtered_line(&self) -> Option<&WorkloadLogLine> {
        let now = crate::time::now();
        let mut last = None;
        for (matched, line) in self
            .lines
            .iter()
            .filter(|line| self.matches_filter_at(line, now))
            .enumerate()
        {
            if matched == self.scroll {
                return Some(line);
            }
            last = Some(line);
        }
        last
    }

    fn selected_filtered_line_anchor(&self) -> Option<WorkloadLogLine> {
        self.current_filtered_line()
            .cloned()
            .or_else(|| self.filtered_line_anchor.clone())
    }

    fn restore_filtered_scroll(&mut self, preserved_line: Option<WorkloadLogLine>) {
        let now = crate::time::now();
        let target_scroll = self.scroll;
        let mut total = 0usize;
        let mut anchor_at_target = None;
        let mut last_anchor = None;

        for line in &self.lines {
            if !self.matches_filter_at(line, now) {
                continue;
            }

            if preserved_line
                .as_ref()
                .is_some_and(|preserved| preserved == line)
            {
                self.scroll = total;
                self.filtered_line_anchor = Some(line.clone());
                return;
            }

            if total == target_scroll {
                anchor_at_target = Some(line.clone());
            }
            last_anchor = Some(line.clone());
            total += 1;
        }

        if total == 0 {
            self.scroll = 0;
            self.filtered_line_anchor = preserved_line;
            return;
        }

        self.scroll = target_scroll.min(total.saturating_sub(1));
        self.filtered_line_anchor = anchor_at_target.or(last_anchor);
    }

    pub fn toggle_correlation_on_current_line(&mut self) -> Result<Option<String>, String> {
        if self.correlation_request_id.is_some() {
            self.correlation_request_id = None;
            self.scroll = 0;
            return Ok(None);
        }
        let Some(request_id) = self
            .current_filtered_line()
            .and_then(|line| line.entry.request_id())
            .map(str::to_string)
        else {
            return Err(
                "The current workload log line does not contain a request token.".to_string(),
            );
        };
        self.correlation_request_id = Some(request_id.clone());
        self.scroll = 0;
        Ok(Some(request_id))
    }

    fn refresh_matching_label_pods(&mut self) {
        self.matching_label_pods = self.label_filter.as_ref().map(|label| {
            self.pod_labels
                .iter()
                .filter_map(|(pod, labels)| {
                    labels
                        .iter()
                        .any(|candidate| candidate == label)
                        .then_some(pod.clone())
                })
                .collect()
        });
    }
}

#[derive(Debug, Clone)]
pub struct ExecTabState {
    pub resource: ResourceRef,
    pub session_id: u64,
    pub pod_name: String,
    pub namespace: String,
    pub container_name: String,
    pub containers: Vec<String>,
    pub picking_container: bool,
    pub container_cursor: usize,
    pub input: String,
    pub input_cursor: usize,
    pub lines: Vec<String>,
    pub scroll: usize,
    pub loading: bool,
    pub shell_name: Option<String>,
    pub error: Option<String>,
    pub exited: bool,
    pub pending_fragment: String,
}

#[derive(Debug, Clone)]
pub struct ExtensionOutputTabState {
    pub execution_id: u64,
    pub title: String,
    pub resource: Option<ResourceRef>,
    pub command_preview: String,
    pub mode_label: String,
    pub lines: Vec<String>,
    pub scroll: usize,
    pub loading: bool,
    pub success: Option<bool>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
}

impl ExtensionOutputTabState {
    pub fn new(
        execution_id: u64,
        title: impl Into<String>,
        resource: Option<ResourceRef>,
        mode_label: impl Into<String>,
        command_preview: impl Into<String>,
    ) -> Self {
        Self {
            execution_id,
            title: title.into(),
            resource,
            command_preview: command_preview.into(),
            mode_label: mode_label.into(),
            lines: Vec::new(),
            scroll: 0,
            loading: true,
            success: None,
            exit_code: None,
            error: None,
        }
    }

    pub fn apply_output(
        &mut self,
        lines: Vec<String>,
        success: bool,
        exit_code: Option<i32>,
        error: Option<String>,
    ) {
        self.lines = truncate_extension_lines(lines);
        self.scroll = 0;
        self.loading = false;
        self.success = Some(success);
        self.exit_code = exit_code;
        self.error = error;
    }
}

#[derive(Debug, Clone)]
pub struct AiAnalysisContent {
    pub provider_label: String,
    pub model: String,
    pub summary: String,
    pub likely_causes: Vec<String>,
    pub next_steps: Vec<String>,
    pub uncertainty: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AiAnalysisTabState {
    pub execution_id: u64,
    pub title: String,
    pub resource: ResourceRef,
    pub scroll: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub content: Option<Box<AiAnalysisContent>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunbookStepState {
    Pending,
    Done,
    Skipped,
}

#[derive(Debug, Clone)]
pub struct RunbookStepRuntime {
    pub step: LoadedRunbookStep,
    pub state: RunbookStepState,
}

#[derive(Debug, Clone)]
pub struct RunbookTabState {
    pub runbook: LoadedRunbook,
    pub resource: Option<ResourceRef>,
    pub selected: usize,
    pub banner: Option<String>,
    pub detail_scroll: usize,
    pub steps: Vec<RunbookStepRuntime>,
}

impl RunbookTabState {
    pub fn new(runbook: LoadedRunbook, resource: Option<ResourceRef>) -> Self {
        let steps = runbook
            .steps
            .iter()
            .cloned()
            .map(|step| RunbookStepRuntime {
                step,
                state: RunbookStepState::Pending,
            })
            .collect();
        Self {
            runbook,
            resource,
            selected: 0,
            banner: None,
            detail_scroll: 0,
            steps,
        }
    }

    pub fn refresh_runbook(&mut self, runbook: LoadedRunbook) {
        let selected_step = self
            .steps
            .get(self.selected.min(self.steps.len().saturating_sub(1)))
            .map(|step| (step.step.title.clone(), step.step.kind.clone()));
        let previous_states = self
            .steps
            .iter()
            .map(|step| {
                (
                    (step.step.title.clone(), step.step.kind.clone()),
                    step.state,
                )
            })
            .collect::<Vec<_>>();

        self.steps = runbook
            .steps
            .iter()
            .cloned()
            .map(|step| {
                let state = previous_states
                    .iter()
                    .find_map(|((title, kind), state)| {
                        (title == &step.title && kind == &step.kind).then_some(*state)
                    })
                    .unwrap_or(RunbookStepState::Pending);
                RunbookStepRuntime { step, state }
            })
            .collect();
        self.runbook = runbook;
        self.selected = selected_step
            .and_then(|(title, kind)| {
                self.steps
                    .iter()
                    .position(|step| step.step.title == title && step.step.kind == kind)
            })
            .unwrap_or(0)
            .min(self.steps.len().saturating_sub(1));
    }

    pub fn select_next(&mut self) {
        if !self.steps.is_empty() {
            self.selected = (self.selected + 1).min(self.steps.len().saturating_sub(1));
            self.detail_scroll = 0;
        }
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.detail_scroll = 0;
    }

    pub fn select_top(&mut self) {
        self.selected = 0;
        self.detail_scroll = 0;
    }

    pub fn select_bottom(&mut self) {
        self.selected = self.steps.len().saturating_sub(1);
        self.detail_scroll = 0;
    }

    pub fn scroll_detail_down(&mut self, step: usize) {
        self.detail_scroll = self.detail_scroll.saturating_add(step);
    }

    pub fn scroll_detail_up(&mut self, step: usize) {
        self.detail_scroll = self.detail_scroll.saturating_sub(step);
    }

    pub fn selected_step(&self) -> Option<&RunbookStepRuntime> {
        self.steps
            .get(self.selected.min(self.steps.len().saturating_sub(1)))
    }

    pub fn selected_step_mut(&mut self) -> Option<&mut RunbookStepRuntime> {
        let index = self.selected.min(self.steps.len().saturating_sub(1));
        self.steps.get_mut(index)
    }

    pub fn toggle_done(&mut self) {
        if let Some(step) = self.selected_step_mut() {
            step.state = match step.state {
                RunbookStepState::Done => RunbookStepState::Pending,
                _ => RunbookStepState::Done,
            };
        }
    }

    pub fn toggle_skipped(&mut self) {
        if let Some(step) = self.selected_step_mut() {
            step.state = match step.state {
                RunbookStepState::Skipped => RunbookStepState::Pending,
                _ => RunbookStepState::Skipped,
            };
        }
    }

    pub fn progress_label(&self) -> String {
        let done = self
            .steps
            .iter()
            .filter(|step| step.state == RunbookStepState::Done)
            .count();
        format!("{done}/{}", self.steps.len())
    }
}

impl AiAnalysisTabState {
    pub fn new(execution_id: u64, title: impl Into<String>, resource: ResourceRef) -> Self {
        Self {
            execution_id,
            title: title.into(),
            resource,
            scroll: 0,
            loading: true,
            error: None,
            content: None,
        }
    }

    pub fn rendered_line_count(&self) -> usize {
        let mut lines = 3usize;
        if self.loading || self.error.is_some() {
            return lines + 1;
        }
        if let Some(content) = &self.content {
            lines += 3;
            for section in [
                &content.likely_causes,
                &content.next_steps,
                &content.uncertainty,
            ] {
                if !section.is_empty() {
                    lines += section.len() + 2;
                }
            }
        }
        lines
    }

    pub fn apply_result(
        &mut self,
        provider_label: impl Into<String>,
        model: impl Into<String>,
        summary: String,
        likely_causes: Vec<String>,
        next_steps: Vec<String>,
        uncertainty: Vec<String>,
    ) {
        self.scroll = 0;
        self.loading = false;
        self.error = None;
        self.content = Some(Box::new(AiAnalysisContent {
            provider_label: provider_label.into(),
            model: model.into(),
            summary,
            likely_causes,
            next_steps,
            uncertainty,
        }));
    }

    pub fn apply_error(&mut self, error: String) {
        self.scroll = 0;
        self.loading = false;
        self.error = Some(error);
        self.content = None;
    }
}

impl ExecTabState {
    pub fn new(
        resource: ResourceRef,
        session_id: u64,
        pod_name: String,
        namespace: String,
    ) -> Self {
        Self {
            resource,
            session_id,
            pod_name,
            namespace,
            container_name: String::new(),
            containers: Vec::new(),
            picking_container: false,
            container_cursor: 0,
            input: String::new(),
            input_cursor: 0,
            lines: Vec::new(),
            scroll: 0,
            loading: true,
            shell_name: None,
            error: None,
            exited: false,
            pending_fragment: String::new(),
        }
    }

    pub fn set_containers(&mut self, containers: Vec<String>) {
        let selected_container = if self.picking_container {
            self.containers.get(self.container_cursor).cloned()
        } else if self.container_name.is_empty() {
            None
        } else {
            Some(self.container_name.clone())
        };
        self.containers = containers;
        self.exited = false;
        self.error = None;
        if self.containers.is_empty() {
            self.container_cursor = 0;
            self.picking_container = false;
            self.loading = false;
            self.error = Some("No containers found in this pod.".to_string());
        } else if self.containers.len() > 1 {
            self.container_cursor = selected_container
                .and_then(|name| {
                    self.containers
                        .iter()
                        .position(|container| container == &name)
                })
                .unwrap_or(0);
            self.picking_container = true;
            self.loading = false;
        } else if let Some(container) = self.containers.first() {
            self.container_cursor = 0;
            self.container_name = container.clone();
            self.picking_container = false;
        }
    }

    pub fn restart_session(
        &mut self,
        session_id: u64,
        pod_name: String,
        namespace: String,
        preset_container: Option<String>,
    ) {
        self.session_id = session_id;
        self.pod_name = pod_name;
        self.namespace = namespace;
        self.lines.clear();
        self.scroll = 0;
        self.loading = true;
        self.shell_name = None;
        self.error = None;
        self.exited = false;
        self.pending_fragment.clear();
        self.picking_container = false;
        self.container_cursor = 0;
        if let Some(container_name) = preset_container {
            self.container_name = container_name.clone();
            self.containers = vec![container_name];
        } else {
            self.containers.clear();
        }
    }

    pub fn preset_container(&mut self, container_name: impl Into<String>) {
        let container_name = container_name.into();
        self.container_name = container_name.clone();
        self.containers = vec![container_name];
        self.container_cursor = 0;
        self.picking_container = false;
        self.loading = true;
        self.exited = false;
        self.error = None;
    }

    pub fn append_banner(&mut self, lines: &[String]) {
        for line in lines {
            self.lines.push(line.clone());
        }
        if !lines.is_empty() {
            self.lines.push(String::new());
        }
    }

    /// Max pending fragment size before force-flushing (1 MB).
    const MAX_PENDING_FRAGMENT: usize = 1_048_576;

    pub fn append_output(&mut self, chunk: &str) {
        for segment in chunk.split_inclusive('\n') {
            if segment.ends_with('\n') {
                self.pending_fragment
                    .push_str(segment.trim_end_matches('\n'));
                self.lines.push(std::mem::take(&mut self.pending_fragment));
            } else {
                self.pending_fragment.push_str(segment);
            }
        }
        // Force-flush oversized fragments (e.g. binary output without newlines).
        if self.pending_fragment.len() > Self::MAX_PENDING_FRAGMENT {
            self.lines.push(std::mem::take(&mut self.pending_fragment));
        }
        if self.lines.len() > MAX_EXEC_OUTPUT_LINES {
            let excess = self.lines.len() - MAX_EXEC_OUTPUT_LINES;
            self.lines.drain(..excess);
            self.scroll = self.scroll.saturating_sub(excess);
        }
        // Auto-follow: keep scroll at bottom if user was already at (or near) the end.
        let max = self.lines.len().saturating_sub(1);
        if self.scroll >= max.saturating_sub(1) {
            self.scroll = max;
        }
    }
}

fn truncate_extension_lines(mut lines: Vec<String>) -> Vec<String> {
    if lines.len() > MAX_EXTENSION_OUTPUT_LINES {
        let omitted = lines.len() - MAX_EXTENSION_OUTPUT_LINES;
        lines.truncate(MAX_EXTENSION_OUTPUT_LINES);
        lines.push(format!("... truncated {omitted} additional lines"));
    }
    lines
}

#[derive(Debug, Clone)]
pub struct PortForwardTabState {
    pub target: Option<ResourceRef>,
    pub dialog: PortForwardDialog,
}

impl PortForwardTabState {
    pub fn new(target: Option<ResourceRef>, dialog: PortForwardDialog) -> Self {
        Self { target, dialog }
    }
}

#[derive(Debug, Clone)]
pub struct RelationsTabState {
    pub resource: ResourceRef,
    pub pending_request_id: Option<u64>,
    pub tree: Vec<crate::k8s::relationships::RelationNode>,
    pub cursor: usize,
    pub expanded: std::collections::HashSet<usize>,
    pub loading: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NetworkPolicyTabState {
    pub resource: ResourceRef,
    pub summary_lines: Vec<String>,
    pub tree: Vec<crate::k8s::relationships::RelationNode>,
    pub cursor: usize,
    pub expanded: std::collections::HashSet<usize>,
    pub loading: bool,
    pub error: Option<String>,
}

fn default_expanded_relation_tree(
    tree: &[crate::k8s::relationships::RelationNode],
) -> std::collections::HashSet<usize> {
    let mut expanded = std::collections::HashSet::new();
    let mut counter = 0usize;
    for section in tree {
        expanded.insert(counter);
        counter += 1;
        for child in &section.children {
            expanded.insert(counter);
            counter += 1;
            crate::k8s::relationships::count_descendants(&child.children, &mut counter);
        }
    }
    expanded
}

fn preserved_relation_cursor(
    previous_tree: &[crate::k8s::relationships::RelationNode],
    previous_expanded: &std::collections::HashSet<usize>,
    previous_cursor: usize,
    next_tree: &[crate::k8s::relationships::RelationNode],
    next_expanded: &std::collections::HashSet<usize>,
) -> usize {
    let previous_flat = crate::k8s::relationships::flatten_tree(previous_tree, previous_expanded);
    let selected = previous_flat.get(previous_cursor.min(previous_flat.len().saturating_sub(1)));
    let next_flat = crate::k8s::relationships::flatten_tree(next_tree, next_expanded);

    selected
        .and_then(|node| {
            if let Some(resource) = &node.resource {
                next_flat
                    .iter()
                    .position(|candidate| candidate.resource.as_ref() == Some(resource))
            } else {
                next_flat.iter().position(|candidate| {
                    candidate.resource.is_none()
                        && candidate.relation == node.relation
                        && candidate.namespace == node.namespace
                        && candidate.label == node.label
                })
            }
        })
        .unwrap_or_else(|| previous_cursor.min(next_flat.len().saturating_sub(1)))
}

impl NetworkPolicyTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            summary_lines: Vec::new(),
            tree: Vec::new(),
            cursor: 0,
            expanded: std::collections::HashSet::new(),
            loading: false,
            error: None,
        }
    }

    pub fn apply_analysis(&mut self, analysis: NetworkPolicyAnalysis) {
        let expanded = default_expanded_relation_tree(&analysis.tree);
        let cursor = preserved_relation_cursor(
            &self.tree,
            &self.expanded,
            self.cursor,
            &analysis.tree,
            &expanded,
        );
        self.summary_lines = analysis.summary_lines;
        self.tree = analysis.tree;
        self.expanded = expanded;
        self.cursor = cursor;
        self.loading = false;
        self.error = None;
    }

    pub fn set_error(&mut self, error: String) {
        self.summary_lines.clear();
        self.tree.clear();
        self.cursor = 0;
        self.expanded.clear();
        self.loading = false;
        self.error = Some(error);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectivityTargetOption {
    pub resource: ResourceRef,
    pub display: String,
    pub status: String,
    pub pod_ip: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectivityTabFocus {
    Filter,
    Targets,
    Result,
}

#[derive(Debug, Clone)]
pub struct ConnectivityTabState {
    pub source: ResourceRef,
    pub focus: ConnectivityTabFocus,
    pub filter: InputFieldWidget,
    pub targets: Vec<ConnectivityTargetOption>,
    pub filtered_target_indices: Vec<usize>,
    pub selected_target: usize,
    selected_target_resource: Option<ResourceRef>,
    pub current_target: Option<ResourceRef>,
    pub summary_lines: Vec<String>,
    pub tree: Vec<crate::k8s::relationships::RelationNode>,
    pub tree_cursor: usize,
    pub expanded: std::collections::HashSet<usize>,
    pub error: Option<String>,
}

impl ConnectivityTabState {
    pub fn new(source: ResourceRef, targets: Vec<ConnectivityTargetOption>) -> Self {
        let mut tab = Self {
            source,
            focus: ConnectivityTabFocus::Targets,
            filter: InputFieldWidget::new(64),
            targets,
            filtered_target_indices: Vec::new(),
            selected_target: 0,
            selected_target_resource: None,
            current_target: None,
            summary_lines: default_connectivity_summary(),
            tree: Vec::new(),
            tree_cursor: 0,
            expanded: std::collections::HashSet::new(),
            error: None,
        };
        tab.refresh_filter();
        tab
    }

    pub fn apply_targets(&mut self, targets: Vec<ConnectivityTargetOption>) {
        let preserved_target = self.selected_target_resource.clone().or_else(|| {
            self.selected_target_option()
                .map(|target| target.resource.clone())
        });
        self.targets = targets;
        self.selected_target_resource = preserved_target;
        if self
            .current_target
            .as_ref()
            .is_some_and(|target| !self.targets.iter().any(|entry| entry.resource == *target))
        {
            self.current_target = None;
            self.summary_lines = default_connectivity_summary();
            self.tree.clear();
            self.tree_cursor = 0;
            self.expanded.clear();
            self.error = None;
            self.focus = ConnectivityTabFocus::Targets;
        } else if self.current_target.is_none() && self.error.is_some() {
            self.summary_lines = default_connectivity_summary();
            self.tree.clear();
            self.tree_cursor = 0;
            self.expanded.clear();
            self.error = None;
            self.focus = ConnectivityTabFocus::Targets;
        }
        self.refresh_filter();
    }

    pub fn refresh_filter(&mut self) {
        let preserved_target = self.selected_target_resource.clone().or_else(|| {
            self.selected_target_option()
                .map(|target| target.resource.clone())
        });
        let query = self.filter.value.to_ascii_lowercase();
        self.filtered_target_indices = self
            .targets
            .iter()
            .enumerate()
            .filter_map(|(idx, target)| {
                (query.is_empty()
                    || target.display.to_ascii_lowercase().contains(&query)
                    || target.status.to_ascii_lowercase().contains(&query)
                    || target
                        .pod_ip
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains(&query))
                .then_some(idx)
            })
            .collect();
        if self.filtered_target_indices.is_empty() {
            self.selected_target = 0;
            return;
        }
        self.selected_target = preserved_target
            .as_ref()
            .and_then(|resource| {
                self.filtered_target_indices
                    .iter()
                    .position(|idx| self.targets[*idx].resource == *resource)
            })
            .unwrap_or_else(|| {
                self.selected_target
                    .min(self.filtered_target_indices.len().saturating_sub(1))
            });
        self.selected_target_resource = self
            .selected_target_option()
            .map(|target| target.resource.clone());
    }

    pub fn selected_target_option(&self) -> Option<&ConnectivityTargetOption> {
        self.filtered_target_indices
            .get(self.selected_target)
            .and_then(|idx| self.targets.get(*idx))
    }

    pub fn select_next_target(&mut self) {
        if self.filtered_target_indices.is_empty() {
            self.selected_target = 0;
            self.selected_target_resource = None;
            return;
        }
        self.selected_target =
            (self.selected_target + 1).min(self.filtered_target_indices.len().saturating_sub(1));
        self.selected_target_resource = self
            .selected_target_option()
            .map(|target| target.resource.clone());
    }

    pub fn select_previous_target(&mut self) {
        self.selected_target = self.selected_target.saturating_sub(1);
        self.selected_target_resource = self
            .selected_target_option()
            .map(|target| target.resource.clone());
    }

    pub fn select_top_target(&mut self) {
        self.selected_target = 0;
        self.selected_target_resource = self
            .selected_target_option()
            .map(|target| target.resource.clone());
    }

    pub fn select_bottom_target(&mut self) {
        self.selected_target = self.filtered_target_indices.len().saturating_sub(1);
        self.selected_target_resource = self
            .selected_target_option()
            .map(|target| target.resource.clone());
    }

    pub fn apply_analysis(&mut self, target: ResourceRef, analysis: ConnectivityAnalysis) {
        self.current_target = Some(target);
        self.summary_lines = analysis.summary_lines;
        self.tree = analysis.tree;
        self.focus = ConnectivityTabFocus::Result;
        self.tree_cursor = 0;
        self.error = None;
        self.expanded.clear();

        let mut counter = 0usize;
        for section in &self.tree {
            self.expanded.insert(counter);
            counter += 1;
            for child in &section.children {
                self.expanded.insert(counter);
                counter += 1;
                crate::k8s::relationships::count_descendants(&child.children, &mut counter);
            }
        }
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
        self.current_target = None;
        self.summary_lines = vec![
            "Connectivity query could not be evaluated from the current snapshot.".to_string(),
        ];
        self.tree.clear();
        self.tree_cursor = 0;
        self.expanded.clear();
        self.focus = ConnectivityTabFocus::Targets;
    }
}

fn default_connectivity_summary() -> Vec<String> {
    vec![
        "Select a target pod, then press [Enter] to evaluate whether any traffic is allowed by policy intent."
            .to_string(),
        "Result shows intent only; CNI enforcement/runtime packet filters may still differ."
            .to_string(),
    ]
}

#[derive(Debug, Clone)]
pub struct TrafficDebugTabState {
    pub resource: ResourceRef,
    pub summary_lines: Vec<String>,
    pub tree: Vec<crate::k8s::relationships::RelationNode>,
    pub cursor: usize,
    pub expanded: std::collections::HashSet<usize>,
    pub error: Option<String>,
}

impl TrafficDebugTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            summary_lines: Vec::new(),
            tree: Vec::new(),
            cursor: 0,
            expanded: std::collections::HashSet::new(),
            error: None,
        }
    }

    pub fn apply_analysis(&mut self, analysis: TrafficDebugAnalysis) {
        let expanded = default_expanded_relation_tree(&analysis.tree);
        let cursor = preserved_relation_cursor(
            &self.tree,
            &self.expanded,
            self.cursor,
            &analysis.tree,
            &expanded,
        );
        self.summary_lines = analysis.summary_lines;
        self.tree = analysis.tree;
        self.expanded = expanded;
        self.cursor = cursor;
        self.error = None;
    }

    pub fn set_error(&mut self, error: String) {
        self.summary_lines.clear();
        self.tree.clear();
        self.cursor = 0;
        self.expanded.clear();
        self.error = Some(error);
    }
}

impl RelationsTabState {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            pending_request_id: None,
            tree: Vec::new(),
            cursor: 0,
            expanded: std::collections::HashSet::new(),
            loading: true,
            error: None,
        }
    }

    /// Populate the tree and auto-expand section headers and their immediate
    /// children so the user sees a useful overview on first open.
    pub fn set_tree(&mut self, tree: Vec<crate::k8s::relationships::RelationNode>) {
        let expanded = default_expanded_relation_tree(&tree);
        let cursor =
            preserved_relation_cursor(&self.tree, &self.expanded, self.cursor, &tree, &expanded);
        self.expanded = expanded;
        self.tree = tree;
        self.cursor = cursor;
        self.pending_request_id = None;
        self.loading = false;
        self.error = None;
    }

    pub fn set_error(&mut self, error: String) {
        self.pending_request_id = None;
        self.tree.clear();
        self.cursor = 0;
        self.expanded.clear();
        self.loading = false;
        self.error = Some(error);
    }
}

#[derive(Debug, Clone)]
pub enum WorkbenchTabState {
    ActionHistory(ActionHistoryTabState),
    AccessReview(AccessReviewTabState),
    ResourceYaml(ResourceYamlTabState),
    ResourceDiff(ResourceDiffTabState),
    Rollout(RolloutTabState),
    HelmHistory(HelmHistoryTabState),
    DecodedSecret(DecodedSecretTabState),
    ResourceEvents(ResourceEventsTabState),
    PodLogs(PodLogsTabState),
    WorkloadLogs(WorkloadLogsTabState),
    Exec(ExecTabState),
    ExtensionOutput(ExtensionOutputTabState),
    AiAnalysis(Box<AiAnalysisTabState>),
    Runbook(Box<RunbookTabState>),
    PortForward(PortForwardTabState),
    Relations(RelationsTabState),
    NetworkPolicy(NetworkPolicyTabState),
    Connectivity(ConnectivityTabState),
    TrafficDebug(TrafficDebugTabState),
}

impl WorkbenchTabState {
    pub const fn kind(&self) -> WorkbenchTabKind {
        match self {
            Self::ActionHistory(_) => WorkbenchTabKind::ActionHistory,
            Self::AccessReview(_) => WorkbenchTabKind::AccessReview,
            Self::ResourceYaml(_) => WorkbenchTabKind::ResourceYaml,
            Self::ResourceDiff(_) => WorkbenchTabKind::ResourceDiff,
            Self::Rollout(_) => WorkbenchTabKind::Rollout,
            Self::HelmHistory(_) => WorkbenchTabKind::Helm,
            Self::DecodedSecret(_) => WorkbenchTabKind::DecodedSecret,
            Self::ResourceEvents(_) => WorkbenchTabKind::ResourceEvents,
            Self::PodLogs(_) => WorkbenchTabKind::PodLogs,
            Self::WorkloadLogs(_) => WorkbenchTabKind::WorkloadLogs,
            Self::Exec(_) => WorkbenchTabKind::Exec,
            Self::ExtensionOutput(_) => WorkbenchTabKind::Extension,
            Self::AiAnalysis(_) => WorkbenchTabKind::AiAnalysis,
            Self::Runbook(_) => WorkbenchTabKind::Runbook,
            Self::PortForward(_) => WorkbenchTabKind::PortForward,
            Self::Relations(_) => WorkbenchTabKind::Relations,
            Self::NetworkPolicy(_) => WorkbenchTabKind::NetworkPolicy,
            Self::Connectivity(_) => WorkbenchTabKind::Connectivity,
            Self::TrafficDebug(_) => WorkbenchTabKind::TrafficDebug,
        }
    }

    pub fn key(&self) -> WorkbenchTabKey {
        match self {
            Self::ActionHistory(_) => WorkbenchTabKey::ActionHistory,
            Self::AccessReview(tab) => WorkbenchTabKey::AccessReview(tab.resource.clone()),
            Self::ResourceYaml(tab) => WorkbenchTabKey::ResourceYaml(tab.resource.clone()),
            Self::ResourceDiff(tab) => WorkbenchTabKey::ResourceDiff(tab.resource.clone()),
            Self::Rollout(tab) => WorkbenchTabKey::Rollout(tab.resource.clone()),
            Self::HelmHistory(tab) => WorkbenchTabKey::HelmHistory(tab.resource.clone()),
            Self::DecodedSecret(tab) => WorkbenchTabKey::DecodedSecret(tab.resource.clone()),
            Self::ResourceEvents(tab) => WorkbenchTabKey::ResourceEvents(tab.resource.clone()),
            Self::PodLogs(tab) => WorkbenchTabKey::PodLogs(tab.resource.clone()),
            Self::WorkloadLogs(tab) => WorkbenchTabKey::WorkloadLogs(tab.resource.clone()),
            Self::Exec(tab) => WorkbenchTabKey::Exec(tab.resource.clone()),
            Self::ExtensionOutput(tab) => WorkbenchTabKey::ExtensionOutput(tab.execution_id),
            Self::AiAnalysis(tab) => WorkbenchTabKey::AiAnalysis(tab.execution_id),
            Self::Runbook(tab) => {
                WorkbenchTabKey::Runbook(tab.runbook.id.clone(), tab.resource.clone())
            }
            Self::PortForward(_) => WorkbenchTabKey::PortForward,
            Self::Relations(tab) => WorkbenchTabKey::Relations(tab.resource.clone()),
            Self::NetworkPolicy(tab) => WorkbenchTabKey::NetworkPolicy(tab.resource.clone()),
            Self::Connectivity(tab) => WorkbenchTabKey::Connectivity(tab.source.clone()),
            Self::TrafficDebug(tab) => WorkbenchTabKey::TrafficDebug(tab.resource.clone()),
        }
    }

    pub fn title(&self) -> String {
        let kind_label = self.kind().title();
        let icon = tab_icon(kind_label).active();
        match self {
            Self::ActionHistory(_) => format!("{icon}{kind_label}"),
            Self::AccessReview(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
            Self::ResourceYaml(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
            Self::ResourceDiff(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
            Self::Rollout(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
            Self::HelmHistory(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
            Self::DecodedSecret(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
            Self::ResourceEvents(tab) => {
                format!("{icon}Events {}", resource_title(&tab.resource))
            }
            Self::PodLogs(tab) => format!("{icon}Logs {}", resource_title(&tab.resource)),
            Self::WorkloadLogs(tab) => format!("{icon}Logs {}", resource_title(&tab.resource)),
            Self::Exec(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
            Self::ExtensionOutput(tab) => match &tab.resource {
                Some(resource) => format!("{icon}Ext {} {}", tab.title, resource_title(resource)),
                None => format!("{icon}Ext {}", tab.title),
            },
            Self::AiAnalysis(tab) => {
                format!("{icon}AI {} {}", tab.title, resource_title(&tab.resource))
            }
            Self::Runbook(tab) => match &tab.resource {
                Some(resource) => format!(
                    "{icon}Runbook {} {}",
                    tab.runbook.title,
                    resource_title(resource)
                ),
                None => format!("{icon}Runbook {}", tab.runbook.title),
            },
            Self::PortForward(tab) => match &tab.target {
                Some(resource) => {
                    format!("{icon}{kind_label} {}", resource_title(resource))
                }
                None => format!("{icon}{kind_label} Sessions"),
            },
            Self::Relations(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
            Self::NetworkPolicy(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
            Self::Connectivity(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.source))
            }
            Self::TrafficDebug(tab) => {
                format!("{icon}{kind_label} {}", resource_title(&tab.resource))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkbenchTab {
    pub id: u64,
    pub state: WorkbenchTabState,
}

impl WorkbenchTab {
    pub fn new(id: u64, state: WorkbenchTabState) -> Self {
        Self { id, state }
    }
}

#[derive(Debug, Clone)]
pub struct WorkbenchState {
    pub open: bool,
    pub height: u16,
    pub maximized: bool,
    pub active_tab: usize,
    pub tabs: Vec<WorkbenchTab>,
    next_tab_id: u64,
}

impl Default for WorkbenchState {
    fn default() -> Self {
        Self {
            open: false,
            height: DEFAULT_WORKBENCH_HEIGHT,
            maximized: false,
            active_tab: 0,
            tabs: Vec::new(),
            next_tab_id: 1,
        }
    }
}

impl WorkbenchState {
    pub fn open_tab(&mut self, state: WorkbenchTabState) -> usize {
        self.ensure_tab(state, true)
    }

    pub fn ensure_background_tab(&mut self, state: WorkbenchTabState) -> usize {
        self.ensure_tab(state, false)
    }

    fn ensure_tab(&mut self, state: WorkbenchTabState, focus: bool) -> usize {
        let key = state.key();
        let was_open = self.open;
        let previous_active = self.active_tab;
        if let Some(idx) = self.tabs.iter().position(|tab| tab.state.key() == key) {
            self.tabs[idx].state = state;
            if focus {
                self.active_tab = idx;
                self.open = true;
            } else {
                self.active_tab = previous_active.min(self.tabs.len().saturating_sub(1));
                self.open = was_open;
            }
            return idx;
        }

        let id = self.next_tab_id;
        self.next_tab_id = self.next_tab_id.saturating_add(1);
        self.tabs.push(WorkbenchTab::new(id, state));
        let idx = self.tabs.len().saturating_sub(1);
        if focus {
            self.active_tab = idx;
            self.open = true;
        } else {
            self.active_tab = previous_active.min(self.tabs.len().saturating_sub(1));
            self.open = was_open;
        }
        idx
    }

    pub fn toggle_open(&mut self) {
        if self.open {
            self.open = false;
            self.maximized = false;
        } else if !self.tabs.is_empty() {
            self.open = true;
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.maximized = false;
    }

    pub fn toggle_maximize(&mut self) {
        if !self.tabs.is_empty() {
            self.maximized = !self.maximized;
        }
    }

    pub fn close_active_tab(&mut self) {
        if self.tabs.is_empty() {
            self.open = false;
            self.maximized = false;
            self.active_tab = 0;
            return;
        }

        let idx = self.active_tab.min(self.tabs.len().saturating_sub(1));
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            self.open = false;
            self.maximized = false;
            self.active_tab = 0;
            return;
        }

        self.active_tab = idx.min(self.tabs.len().saturating_sub(1));
    }

    pub fn next_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        self.active_tab = (self.active_tab + 1) % self.tabs.len();
    }

    pub fn previous_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        self.active_tab = if self.active_tab == 0 {
            self.tabs.len() - 1
        } else {
            self.active_tab - 1
        };
    }

    pub fn resize_larger(&mut self) {
        self.height = self
            .height
            .saturating_add(1)
            .clamp(MIN_WORKBENCH_HEIGHT, MAX_WORKBENCH_HEIGHT);
    }

    pub fn resize_smaller(&mut self) {
        self.height = self
            .height
            .saturating_sub(1)
            .clamp(MIN_WORKBENCH_HEIGHT, MAX_WORKBENCH_HEIGHT);
    }

    pub fn active_tab(&self) -> Option<&WorkbenchTab> {
        self.tabs
            .get(self.active_tab.min(self.tabs.len().saturating_sub(1)))
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut WorkbenchTab> {
        let idx = self.active_tab.min(self.tabs.len().saturating_sub(1));
        self.tabs.get_mut(idx)
    }

    pub fn find_tab_mut(&mut self, key: &WorkbenchTabKey) -> Option<&mut WorkbenchTab> {
        self.tabs.iter_mut().find(|tab| tab.state.key() == *key)
    }

    pub fn exec_session_id(&self, resource: &ResourceRef) -> Option<u64> {
        self.tabs.iter().find_map(|tab| match &tab.state {
            WorkbenchTabState::Exec(exec_tab) if &exec_tab.resource == resource => {
                Some(exec_tab.session_id)
            }
            _ => None,
        })
    }

    pub fn has_tab(&self, key: &WorkbenchTabKey) -> bool {
        self.tabs.iter().any(|tab| tab.state.key() == *key)
    }

    pub fn close_tab_by_key(&mut self, key: &WorkbenchTabKey) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.state.key() == *key) else {
            return false;
        };
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.open = false;
            self.maximized = false;
            self.active_tab = 0;
        } else if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        } else if index <= self.active_tab && self.active_tab > 0 {
            self.active_tab -= 1;
        }
        true
    }

    /// Remove all resource-bound tabs (YAML, Drift, Events, Logs, Exec) that become
    /// stale after a context or namespace switch.  ActionHistory and PortForward
    /// are retained since they are not resource-scoped.
    pub fn close_resource_tabs(&mut self) {
        self.tabs.retain(|tab| {
            matches!(
                tab.state.kind(),
                WorkbenchTabKind::ActionHistory | WorkbenchTabKind::PortForward
            )
        });
        if self.tabs.is_empty() {
            self.open = false;
            self.maximized = false;
            self.active_tab = 0;
        } else {
            self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
        }
    }

    /// Reset workbench state to the subset represented in saved workspace snapshots.
    ///
    /// Saved workspaces do not persist live resource sessions or port-forward tabs, so
    /// workspace restore must clear them authoritatively before applying the snapshot.
    pub fn close_tabs_for_workspace_restore(&mut self) {
        self.tabs
            .retain(|tab| matches!(tab.state.kind(), WorkbenchTabKind::ActionHistory));
        if self.tabs.is_empty() {
            self.open = false;
            self.maximized = false;
            self.active_tab = 0;
        } else {
            self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
        }
    }

    pub fn activate_tab(&mut self, key: &WorkbenchTabKey) -> bool {
        if let Some(idx) = self.tabs.iter().position(|tab| tab.state.key() == *key) {
            self.active_tab = idx;
            self.open = true;
            return true;
        }
        false
    }

    pub fn set_open_and_height(&mut self, open: bool, height: u16) {
        self.height = height.clamp(MIN_WORKBENCH_HEIGHT, MAX_WORKBENCH_HEIGHT);
        self.open = open;
    }
}

fn resource_title(resource: &ResourceRef) -> String {
    match resource.namespace() {
        Some(namespace) => format!("{}/{}", namespace, resource.name()),
        None => resource.name().to_string(),
    }
}

fn cycle_filter_value(values: &[String], current: Option<&str>) -> Option<String> {
    if values.is_empty() {
        return None;
    }
    match current {
        None => values.first().cloned(),
        Some(current) => values
            .iter()
            .position(|value| value == current)
            .and_then(|idx| values.get(idx + 1))
            .cloned(),
        // Wrap back to "All"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_diff::ResourceDiffLineKind;

    fn pod(name: &str) -> ResourceRef {
        ResourceRef::Pod(name.to_string(), "ns".to_string())
    }

    #[test]
    fn default_workbench_starts_closed_and_empty() {
        let state = WorkbenchState::default();
        assert!(!state.open);
        assert!(state.tabs.is_empty());
        assert_eq!(state.active_tab, 0);
    }

    #[test]
    fn open_tab_reuses_existing_resource_tab() {
        let mut state = WorkbenchState::default();
        let first = state.open_tab(WorkbenchTabState::ResourceYaml(ResourceYamlTabState::new(
            pod("pod-0"),
        )));
        let second = state.open_tab(WorkbenchTabState::ResourceYaml(ResourceYamlTabState::new(
            pod("pod-0"),
        )));

        assert_eq!(first, second);
        assert_eq!(state.tabs.len(), 1);
        assert!(state.open);
    }

    #[test]
    fn resource_diff_tab_deduplicates_by_resource() {
        let mut state = WorkbenchState::default();
        let first = state.open_tab(WorkbenchTabState::ResourceDiff(ResourceDiffTabState::new(
            pod("pod-0"),
        )));
        let second = state.open_tab(WorkbenchTabState::ResourceDiff(ResourceDiffTabState::new(
            pod("pod-0"),
        )));

        assert_eq!(first, second);
        assert_eq!(state.tabs.len(), 1);
        assert!(state.open);
    }

    #[test]
    fn helm_history_tab_deduplicates_by_resource() {
        let mut state = WorkbenchState::default();
        let resource = ResourceRef::HelmRelease("release".to_string(), "default".to_string());
        let first = state.open_tab(WorkbenchTabState::HelmHistory(HelmHistoryTabState::new(
            resource.clone(),
        )));
        let second = state.open_tab(WorkbenchTabState::HelmHistory(HelmHistoryTabState::new(
            resource,
        )));

        assert_eq!(first, second);
        assert_eq!(state.tabs.len(), 1);
        assert!(state.open);
    }

    #[test]
    fn runbook_tab_deduplicates_by_id_and_resource() {
        let mut state = WorkbenchState::default();
        let runbook = LoadedRunbook {
            id: "pod_failure".into(),
            title: "Pod Failure Triage".into(),
            description: None,
            aliases: vec!["incident".into()],
            resource_kinds: vec!["Pod".into()],
            shortcut: None,
            steps: vec![LoadedRunbookStep {
                title: "Open logs".into(),
                description: None,
                kind: crate::runbooks::LoadedRunbookStepKind::DetailAction {
                    action: crate::runbooks::RunbookDetailAction::Logs,
                },
            }],
        };
        let resource = pod("pod-0");
        let first = state.open_tab(WorkbenchTabState::Runbook(Box::new(RunbookTabState::new(
            runbook.clone(),
            Some(resource.clone()),
        ))));
        let second = state.open_tab(WorkbenchTabState::Runbook(Box::new(RunbookTabState::new(
            runbook,
            Some(resource),
        ))));

        assert_eq!(first, second);
        assert_eq!(state.tabs.len(), 1);
    }

    #[test]
    fn runbook_progress_counts_done_steps_only() {
        let runbook = LoadedRunbook {
            id: "pod_failure".into(),
            title: "Pod Failure Triage".into(),
            description: None,
            aliases: vec!["incident".into()],
            resource_kinds: vec!["Pod".into()],
            shortcut: None,
            steps: vec![
                LoadedRunbookStep {
                    title: "Checklist".into(),
                    description: None,
                    kind: crate::runbooks::LoadedRunbookStepKind::Checklist {
                        items: vec!["Inspect events".into()],
                    },
                },
                LoadedRunbookStep {
                    title: "Open logs".into(),
                    description: None,
                    kind: crate::runbooks::LoadedRunbookStepKind::DetailAction {
                        action: crate::runbooks::RunbookDetailAction::Logs,
                    },
                },
            ],
        };
        let mut tab = RunbookTabState::new(runbook, Some(pod("pod-0")));
        assert_eq!(tab.progress_label(), "0/2");

        tab.toggle_done();
        assert_eq!(tab.progress_label(), "1/2");

        tab.select_next();
        tab.toggle_skipped();
        assert_eq!(tab.progress_label(), "1/2");
    }

    #[test]
    fn runbook_detail_scroll_resets_after_selection_change() {
        let runbook = LoadedRunbook {
            id: "scroll".into(),
            title: "Scroll".into(),
            description: None,
            aliases: Vec::new(),
            resource_kinds: Vec::new(),
            shortcut: None,
            steps: vec![
                LoadedRunbookStep {
                    title: "First".into(),
                    description: None,
                    kind: crate::runbooks::LoadedRunbookStepKind::Checklist {
                        items: vec!["One".into()],
                    },
                },
                LoadedRunbookStep {
                    title: "Second".into(),
                    description: None,
                    kind: crate::runbooks::LoadedRunbookStepKind::Checklist {
                        items: vec!["Two".into()],
                    },
                },
            ],
        };
        let mut tab = RunbookTabState::new(runbook, Some(pod("pod-0")));
        tab.scroll_detail_down(9);
        assert_eq!(tab.detail_scroll, 9);
        tab.select_next();
        assert_eq!(tab.detail_scroll, 0);
    }

    #[test]
    fn extension_output_tab_keeps_last_lines_bounded() {
        let mut tab = ExtensionOutputTabState::new(
            7,
            "Describe Pod",
            Some(pod("pod-0")),
            "BG",
            "kubectl get pod",
        );
        let lines = (0..(MAX_EXTENSION_OUTPUT_LINES + 2))
            .map(|idx| format!("line-{idx}"))
            .collect::<Vec<_>>();

        tab.apply_output(lines, true, Some(0), None);

        assert_eq!(tab.lines.len(), MAX_EXTENSION_OUTPUT_LINES + 1);
        assert_eq!(
            tab.lines.last().map(String::as_str),
            Some("... truncated 2 additional lines")
        );
    }

    #[test]
    fn helm_history_apply_history_preserves_selected_revision_when_still_present() {
        let mut tab = HelmHistoryTabState::new(ResourceRef::HelmRelease(
            "release".to_string(),
            "default".to_string(),
        ));
        tab.revisions = vec![
            HelmReleaseRevisionInfo {
                revision: 5,
                ..HelmReleaseRevisionInfo::default()
            },
            HelmReleaseRevisionInfo {
                revision: 4,
                ..HelmReleaseRevisionInfo::default()
            },
        ];
        tab.selected = 1;

        tab.apply_history(crate::k8s::helm::HelmHistoryResult {
            cli_version: "v4.1.3".to_string(),
            revisions: vec![
                HelmReleaseRevisionInfo {
                    revision: 6,
                    ..HelmReleaseRevisionInfo::default()
                },
                HelmReleaseRevisionInfo {
                    revision: 5,
                    ..HelmReleaseRevisionInfo::default()
                },
                HelmReleaseRevisionInfo {
                    revision: 4,
                    ..HelmReleaseRevisionInfo::default()
                },
            ],
        });

        assert_eq!(tab.selected_revision().map(|entry| entry.revision), Some(4));
        assert_eq!(tab.current_revision, Some(6));
        assert!(tab.diff.is_none());
    }

    #[test]
    fn helm_history_scroll_resets_when_mode_changes() {
        let mut tab = HelmHistoryTabState::new(ResourceRef::HelmRelease(
            "release".to_string(),
            "default".to_string(),
        ));
        tab.scroll = 7;

        tab.begin_rollback_confirm(3);
        assert_eq!(tab.scroll, 0);

        tab.scroll = 5;
        tab.begin_diff(4, 3, 99);
        assert_eq!(tab.scroll, 0);

        tab.scroll = 9;
        tab.begin_rollback(41);
        assert_eq!(tab.scroll, 0);
    }

    #[test]
    fn helm_history_clear_rollback_ignores_stale_action() {
        let mut tab = HelmHistoryTabState::new(ResourceRef::HelmRelease(
            "release".to_string(),
            "default".to_string(),
        ));
        tab.begin_rollback(41);

        tab.clear_rollback_if_matches(42);

        assert!(tab.rollback_pending);
        assert_eq!(tab.pending_rollback_action_history_id, Some(41));

        tab.clear_rollback_if_matches(41);

        assert!(!tab.rollback_pending);
        assert!(tab.pending_rollback_action_history_id.is_none());
    }

    #[test]
    fn helm_history_error_clears_stale_payload() {
        let mut tab = HelmHistoryTabState::new(ResourceRef::HelmRelease(
            "release".to_string(),
            "default".to_string(),
        ));
        tab.revisions = vec![HelmReleaseRevisionInfo {
            revision: 5,
            ..HelmReleaseRevisionInfo::default()
        }];
        tab.selected = 3;
        tab.scroll = 8;
        tab.current_revision = Some(5);
        tab.diff = Some(HelmValuesDiffState::new(5, 4, 42));
        tab.confirm_rollback_revision = Some(4);
        tab.rollback_pending = true;

        tab.set_history_error("boom".to_string());

        assert!(tab.revisions.is_empty());
        assert_eq!(tab.selected, 0);
        assert_eq!(tab.scroll, 0);
        assert!(tab.current_revision.is_none());
        assert!(tab.diff.is_none());
        assert!(tab.confirm_rollback_revision.is_none());
        assert!(!tab.rollback_pending);
        assert_eq!(tab.error.as_deref(), Some("boom"));
    }

    #[test]
    fn rollout_detail_scroll_resets_when_mode_changes() {
        let mut tab = RolloutTabState::new(ResourceRef::Deployment(
            "api".to_string(),
            "default".to_string(),
        ));
        tab.detail_scroll = 6;

        tab.begin_undo_confirm(3);
        assert_eq!(tab.detail_scroll, 0);

        tab.detail_scroll = 4;
        tab.begin_mutation(RolloutMutationState::Restart, 21);
        assert_eq!(tab.detail_scroll, 0);

        tab.detail_scroll = 8;
        tab.set_error("boom".to_string());
        assert_eq!(tab.detail_scroll, 0);
    }

    #[test]
    fn rollout_clear_mutation_ignores_stale_action() {
        let mut tab = RolloutTabState::new(ResourceRef::Deployment(
            "api".to_string(),
            "default".to_string(),
        ));
        tab.begin_mutation(RolloutMutationState::Restart, 21);

        tab.clear_mutation_if_matches(22);

        assert_eq!(tab.mutation_pending, Some(RolloutMutationState::Restart));
        assert_eq!(tab.pending_mutation_action_history_id, Some(21));

        tab.clear_mutation_if_matches(21);

        assert!(tab.mutation_pending.is_none());
        assert!(tab.pending_mutation_action_history_id.is_none());
    }

    #[test]
    fn rollout_tab_deduplicates_by_resource() {
        let mut state = WorkbenchState::default();
        let resource = ResourceRef::Deployment("api".to_string(), "default".to_string());
        let first = state.open_tab(WorkbenchTabState::Rollout(RolloutTabState::new(
            resource.clone(),
        )));
        let second = state.open_tab(WorkbenchTabState::Rollout(RolloutTabState::new(resource)));

        assert_eq!(first, second);
        assert_eq!(state.tabs.len(), 1);
        assert!(state.open);
    }

    #[test]
    fn rollout_apply_inspection_preserves_selected_revision_when_still_present() {
        let mut tab = RolloutTabState::new(ResourceRef::Deployment(
            "api".to_string(),
            "default".to_string(),
        ));
        tab.revisions = vec![
            RolloutRevisionInfo {
                revision: 5,
                name: "api-5".to_string(),
                created: None,
                summary: "5/5 ready".to_string(),
                change_cause: None,
                is_current: true,
                is_update_target: true,
            },
            RolloutRevisionInfo {
                revision: 4,
                name: "api-4".to_string(),
                created: None,
                summary: "5/5 ready".to_string(),
                change_cause: None,
                is_current: false,
                is_update_target: false,
            },
        ];
        tab.selected = 1;

        tab.apply_inspection(RolloutInspection {
            kind: RolloutWorkloadKind::Deployment,
            strategy: "RollingUpdate".to_string(),
            paused: false,
            current_revision: Some(6),
            update_target_revision: Some(6),
            summary_lines: vec!["Desired 5".to_string()],
            conditions: Vec::new(),
            revisions: vec![
                RolloutRevisionInfo {
                    revision: 6,
                    name: "api-6".to_string(),
                    created: None,
                    summary: "2/5 ready".to_string(),
                    change_cause: None,
                    is_current: true,
                    is_update_target: true,
                },
                RolloutRevisionInfo {
                    revision: 5,
                    name: "api-5".to_string(),
                    created: None,
                    summary: "5/5 ready".to_string(),
                    change_cause: None,
                    is_current: false,
                    is_update_target: false,
                },
                RolloutRevisionInfo {
                    revision: 4,
                    name: "api-4".to_string(),
                    created: None,
                    summary: "5/5 ready".to_string(),
                    change_cause: None,
                    is_current: false,
                    is_update_target: false,
                },
            ],
        });

        assert_eq!(tab.selected_revision().map(|entry| entry.revision), Some(4));
        assert_eq!(tab.current_revision, Some(6));
        assert!(!tab.loading);
        assert!(tab.confirm_undo_revision.is_none());
        assert!(tab.mutation_pending.is_none());
    }

    #[test]
    fn rollout_error_clears_stale_payload() {
        let mut tab = RolloutTabState::new(ResourceRef::Deployment(
            "api".to_string(),
            "default".to_string(),
        ));
        tab.kind = Some(RolloutWorkloadKind::Deployment);
        tab.strategy = Some("RollingUpdate".to_string());
        tab.paused = true;
        tab.current_revision = Some(7);
        tab.update_target_revision = Some(8);
        tab.summary_lines = vec!["healthy".to_string()];
        tab.conditions = vec![crate::k8s::rollout::RolloutConditionInfo {
            type_: "Available".to_string(),
            status: "True".to_string(),
            reason: Some("Ok".to_string()),
            message: None,
        }];
        tab.revisions = vec![RolloutRevisionInfo {
            revision: 7,
            name: "api-7".to_string(),
            created: None,
            summary: "ready".to_string(),
            change_cause: None,
            is_current: true,
            is_update_target: true,
        }];
        tab.selected = 4;
        tab.confirm_undo_revision = Some(6);
        tab.mutation_pending = Some(RolloutMutationState::Restart);
        tab.pending_mutation_action_history_id = Some(21);

        tab.set_error("boom".to_string());

        assert!(tab.kind.is_none());
        assert!(tab.strategy.is_none());
        assert!(!tab.paused);
        assert!(tab.current_revision.is_none());
        assert!(tab.update_target_revision.is_none());
        assert!(tab.summary_lines.is_empty());
        assert!(tab.conditions.is_empty());
        assert!(tab.revisions.is_empty());
        assert_eq!(tab.selected, 0);
        assert!(tab.confirm_undo_revision.is_none());
        assert!(tab.mutation_pending.is_none());
        assert!(tab.pending_mutation_action_history_id.is_none());
        assert_eq!(tab.error.as_deref(), Some("boom"));
    }

    #[test]
    fn resource_diff_error_clears_stale_payload() {
        let mut tab =
            ResourceDiffTabState::new(ResourceRef::Pod("api".to_string(), "default".to_string()));
        tab.baseline_kind = Some(ResourceDiffBaselineKind::LastAppliedAnnotation);
        tab.summary = Some("changed".to_string());
        tab.lines = vec![ResourceDiffLine {
            kind: ResourceDiffLineKind::Context,
            content: "a".to_string(),
        }];
        tab.scroll = 9;

        tab.set_error("boom".to_string());

        assert!(tab.baseline_kind.is_none());
        assert!(tab.summary.is_none());
        assert!(tab.lines.is_empty());
        assert_eq!(tab.scroll, 0);
        assert_eq!(tab.error.as_deref(), Some("boom"));
    }

    #[test]
    fn network_policy_error_clears_stale_payload() {
        let mut tab =
            NetworkPolicyTabState::new(ResourceRef::Pod("api".to_string(), "default".to_string()));
        tab.summary_lines = vec!["reachable".to_string()];
        tab.tree = vec![crate::k8s::relationships::RelationNode {
            resource: None,
            label: "Policy Summary".to_string(),
            status: None,
            namespace: None,
            relation: crate::k8s::relationships::RelationKind::SectionHeader,
            not_found: false,
            children: Vec::new(),
        }];
        tab.cursor = 5;
        tab.expanded.insert(0);
        tab.loading = true;

        tab.set_error("boom".to_string());

        assert!(tab.summary_lines.is_empty());
        assert!(tab.tree.is_empty());
        assert_eq!(tab.cursor, 0);
        assert!(tab.expanded.is_empty());
        assert!(!tab.loading);
        assert_eq!(tab.error.as_deref(), Some("boom"));
    }

    #[test]
    fn traffic_debug_error_clears_stale_payload() {
        let mut tab =
            TrafficDebugTabState::new(ResourceRef::Pod("api".to_string(), "default".to_string()));
        tab.summary_lines = vec!["reachable".to_string()];
        tab.tree = vec![crate::k8s::relationships::RelationNode {
            resource: None,
            label: "Traffic".to_string(),
            status: None,
            namespace: None,
            relation: crate::k8s::relationships::RelationKind::SectionHeader,
            not_found: false,
            children: Vec::new(),
        }];
        tab.cursor = 5;
        tab.expanded.insert(0);

        tab.set_error("boom".to_string());

        assert!(tab.summary_lines.is_empty());
        assert!(tab.tree.is_empty());
        assert_eq!(tab.cursor, 0);
        assert!(tab.expanded.is_empty());
        assert_eq!(tab.error.as_deref(), Some("boom"));
    }

    #[test]
    fn relations_error_clears_stale_payload() {
        let mut tab =
            RelationsTabState::new(ResourceRef::Pod("api".to_string(), "default".to_string()));
        tab.pending_request_id = Some(42);
        tab.tree = vec![crate::k8s::relationships::RelationNode {
            resource: None,
            label: "Owner Chain".to_string(),
            status: None,
            namespace: None,
            relation: crate::k8s::relationships::RelationKind::SectionHeader,
            not_found: false,
            children: Vec::new(),
        }];
        tab.cursor = 5;
        tab.expanded.insert(0);
        tab.loading = true;

        tab.set_error("boom".to_string());

        assert!(tab.pending_request_id.is_none());
        assert!(tab.tree.is_empty());
        assert_eq!(tab.cursor, 0);
        assert!(tab.expanded.is_empty());
        assert!(!tab.loading);
        assert_eq!(tab.error.as_deref(), Some("boom"));
    }

    #[test]
    fn network_policy_tab_deduplicates_by_resource() {
        let mut state = WorkbenchState::default();
        let first = state.open_tab(WorkbenchTabState::NetworkPolicy(
            NetworkPolicyTabState::new(pod("pod-0")),
        ));
        let second = state.open_tab(WorkbenchTabState::NetworkPolicy(
            NetworkPolicyTabState::new(pod("pod-0")),
        ));

        assert_eq!(first, second);
        assert_eq!(state.tabs.len(), 1);
        assert!(state.open);
    }

    #[test]
    fn connectivity_tab_deduplicates_by_source_resource() {
        let mut state = WorkbenchState::default();
        let first = state.open_tab(WorkbenchTabState::Connectivity(ConnectivityTabState::new(
            pod("pod-0"),
            Vec::new(),
        )));
        let second = state.open_tab(WorkbenchTabState::Connectivity(ConnectivityTabState::new(
            pod("pod-0"),
            Vec::new(),
        )));

        assert_eq!(first, second);
        assert_eq!(state.tabs.len(), 1);
        assert!(state.open);
    }

    #[test]
    fn traffic_debug_tab_deduplicates_by_resource() {
        let mut state = WorkbenchState::default();
        let first = state.open_tab(WorkbenchTabState::TrafficDebug(TrafficDebugTabState::new(
            pod("pod-0"),
        )));
        let second = state.open_tab(WorkbenchTabState::TrafficDebug(TrafficDebugTabState::new(
            pod("pod-0"),
        )));

        assert_eq!(first, second);
        assert_eq!(state.tabs.len(), 1);
        assert!(state.open);
    }

    #[test]
    fn ai_analysis_rendered_line_count_tracks_sections_and_resets_scroll() {
        let mut tab = AiAnalysisTabState::new(7, "Ask AI", pod("pod-0"));
        assert_eq!(tab.rendered_line_count(), 4);

        tab.scroll = 9;
        tab.apply_result(
            "AI",
            "gpt-test",
            "summary".to_string(),
            vec!["cause".to_string()],
            vec!["step".to_string(), "step2".to_string()],
            vec!["uncertain".to_string()],
        );

        assert_eq!(tab.scroll, 0);
        assert_eq!(tab.rendered_line_count(), 16);

        tab.scroll = 4;
        tab.apply_error("boom".to_string());
        assert_eq!(tab.scroll, 0);
        assert_eq!(tab.rendered_line_count(), 4);
    }

    #[test]
    fn access_review_line_count_and_offsets_include_scope_headers() {
        let tab = AccessReviewTabState::new(
            ResourceRef::ServiceAccount("api".into(), "payments".into()),
            Some("prod".into()),
            "payments".into(),
            vec![
                ActionAccessReview {
                    action: crate::policy::DetailAction::ViewYaml,
                    authorization: Some(crate::authorization::DetailActionAuthorization::Allowed),
                    strict: false,
                    checks: vec![],
                },
                ActionAccessReview {
                    action: crate::policy::DetailAction::Delete,
                    authorization: Some(crate::authorization::DetailActionAuthorization::Denied),
                    strict: true,
                    checks: vec![
                        crate::authorization::ResourceAccessCheck::resource(
                            "get",
                            None,
                            "namespaces",
                            None,
                            Some("payments"),
                        ),
                        crate::authorization::ResourceAccessCheck::resource(
                            "delete",
                            None,
                            "pods",
                            Some("payments"),
                            Some("api-0"),
                        ),
                    ],
                },
            ],
            Some(crate::rbac_subjects::SubjectAccessReview {
                subject: crate::rbac_subjects::AccessReviewSubject::ServiceAccount {
                    name: "api".into(),
                    namespace: "payments".into(),
                },
                bindings: vec![
                    crate::rbac_subjects::SubjectBindingResolution {
                        binding: ResourceRef::RoleBinding(
                            "payments-view".into(),
                            "payments".into(),
                        ),
                        role: crate::rbac_subjects::SubjectRoleResolution {
                            resource: Some(ResourceRef::Role(
                                "payments-reader".into(),
                                "payments".into(),
                            )),
                            kind: "Role".into(),
                            name: "payments-reader".into(),
                            namespace: Some("payments".into()),
                            rules: vec![crate::k8s::dtos::RbacRule {
                                verbs: vec!["get".into()],
                                resources: vec!["pods".into()],
                                ..crate::k8s::dtos::RbacRule::default()
                            }],
                            missing: false,
                        },
                    },
                    crate::rbac_subjects::SubjectBindingResolution {
                        binding: ResourceRef::ClusterRoleBinding("api-admin".into()),
                        role: crate::rbac_subjects::SubjectRoleResolution {
                            resource: Some(ResourceRef::ClusterRole("ops-admin".into())),
                            kind: "ClusterRole".into(),
                            name: "ops-admin".into(),
                            namespace: None,
                            rules: vec![crate::k8s::dtos::RbacRule {
                                verbs: vec!["*".into()],
                                resources: vec!["*".into()],
                                ..crate::k8s::dtos::RbacRule::default()
                            }],
                            missing: false,
                        },
                    },
                ],
            }),
            Some(AttemptedActionReview {
                action: crate::policy::DetailAction::Delete,
                authorization: Some(crate::authorization::DetailActionAuthorization::Denied),
                strict: true,
                checks: vec![crate::authorization::ResourceAccessCheck::resource(
                    "delete",
                    Some("apps"),
                    "deployments",
                    Some("payments"),
                    Some("api"),
                )],
                note: None,
            }),
        );

        assert_eq!(tab.line_count(), 33);
        assert_eq!(
            tab.action_line_offset(crate::policy::DetailAction::Delete),
            Some(26)
        );
        assert_eq!(tab.scroll, 3);
    }

    #[test]
    fn tab_navigation_wraps() {
        let mut state = WorkbenchState::default();
        state.open_tab(WorkbenchTabState::ActionHistory(
            ActionHistoryTabState::default(),
        ));
        state.open_tab(WorkbenchTabState::ResourceYaml(ResourceYamlTabState::new(
            pod("pod-0"),
        )));
        state.open_tab(WorkbenchTabState::ResourceEvents(
            ResourceEventsTabState::new(pod("pod-0")),
        ));

        state.next_tab();
        assert_eq!(
            state.active_tab().map(|tab| tab.state.kind()),
            Some(WorkbenchTabKind::ActionHistory)
        );

        state.previous_tab();
        assert_eq!(
            state.active_tab().map(|tab| tab.state.kind()),
            Some(WorkbenchTabKind::ResourceEvents)
        );
    }

    #[test]
    fn close_active_tab_closes_workbench_when_last_tab_removed() {
        let mut state = WorkbenchState::default();
        state.open_tab(WorkbenchTabState::ResourceYaml(ResourceYamlTabState::new(
            pod("pod-0"),
        )));

        state.close_active_tab();

        assert!(!state.open);
        assert!(state.tabs.is_empty());
    }

    #[test]
    fn resize_clamps_to_supported_range() {
        let mut state = WorkbenchState::default();
        state.height = MIN_WORKBENCH_HEIGHT;
        state.resize_smaller();
        assert_eq!(state.height, MIN_WORKBENCH_HEIGHT);

        state.height = MAX_WORKBENCH_HEIGHT;
        state.resize_larger();
        assert_eq!(state.height, MAX_WORKBENCH_HEIGHT);
    }

    #[test]
    fn background_tab_preserves_focus_and_open_state() {
        let mut state = WorkbenchState::default();
        state.open_tab(WorkbenchTabState::ResourceYaml(ResourceYamlTabState::new(
            pod("pod-0"),
        )));

        state.ensure_background_tab(WorkbenchTabState::ActionHistory(
            ActionHistoryTabState::default(),
        ));

        assert_eq!(
            state.active_tab().map(|tab| tab.state.kind()),
            Some(WorkbenchTabKind::ResourceYaml)
        );
        assert!(state.open);
        assert_eq!(state.tabs.len(), 2);
    }

    #[test]
    fn resource_yaml_update_content_clamps_scroll_after_shrink() {
        let mut tab = ResourceYamlTabState::new(pod("pod-0"));
        tab.yaml = Some("a\nb\nc\nd".into());
        tab.loading = false;
        tab.scroll = 99;

        tab.update_content(Some("a\nb".into()), None, None);

        assert_eq!(tab.scroll, 1);
        assert_eq!(tab.yaml.as_deref(), Some("a\nb"));
    }

    #[test]
    fn toggle_maximize() {
        let mut state = WorkbenchState::default();
        state.open_tab(WorkbenchTabState::ActionHistory(
            ActionHistoryTabState::default(),
        ));
        assert!(!state.maximized);
        state.toggle_maximize();
        assert!(state.maximized);
        state.toggle_maximize();
        assert!(!state.maximized);
    }

    #[test]
    fn close_clears_maximized() {
        let mut state = WorkbenchState::default();
        state.maximized = true;
        state.close();
        assert!(!state.maximized);
    }

    #[test]
    fn toggle_open_clears_maximized_on_close() {
        let mut state = WorkbenchState::default();
        state.open = true;
        state.maximized = true;
        state.toggle_open();
        assert!(!state.open);
        assert!(!state.maximized);
    }

    #[test]
    fn toggle_open_does_not_open_empty_workbench() {
        let mut state = WorkbenchState::default();
        assert!(!state.open);
        state.toggle_open();
        assert!(!state.open);
    }

    #[test]
    fn toggle_maximize_does_nothing_when_empty() {
        let mut state = WorkbenchState::default();
        state.toggle_maximize();
        assert!(!state.maximized);
    }

    #[test]
    fn relations_tab_deduplicates_by_resource() {
        let mut state = WorkbenchState::default();
        let first = state.open_tab(WorkbenchTabState::Relations(RelationsTabState::new(pod(
            "pod-0",
        ))));
        let second = state.open_tab(WorkbenchTabState::Relations(RelationsTabState::new(pod(
            "pod-0",
        ))));
        assert_eq!(first, second);
        assert_eq!(state.tabs.len(), 1);
    }

    #[test]
    fn relations_tab_set_tree_preserves_selected_resource_identity() {
        let mut tab = RelationsTabState::new(pod("pod-0"));
        tab.set_tree(vec![crate::k8s::relationships::RelationNode {
            resource: None,
            label: "Owned".to_string(),
            status: None,
            namespace: None,
            relation: crate::k8s::relationships::RelationKind::SectionHeader,
            not_found: false,
            children: vec![
                crate::k8s::relationships::RelationNode {
                    resource: Some(pod("pod-a")),
                    label: "Pod pod-a".to_string(),
                    status: None,
                    namespace: Some("default".to_string()),
                    relation: crate::k8s::relationships::RelationKind::Owned,
                    not_found: false,
                    children: Vec::new(),
                },
                crate::k8s::relationships::RelationNode {
                    resource: Some(pod("pod-b")),
                    label: "Pod pod-b".to_string(),
                    status: None,
                    namespace: Some("default".to_string()),
                    relation: crate::k8s::relationships::RelationKind::Owned,
                    not_found: false,
                    children: Vec::new(),
                },
            ],
        }]);
        tab.cursor = 2;

        tab.set_tree(vec![crate::k8s::relationships::RelationNode {
            resource: None,
            label: "Owned".to_string(),
            status: None,
            namespace: None,
            relation: crate::k8s::relationships::RelationKind::SectionHeader,
            not_found: false,
            children: vec![
                crate::k8s::relationships::RelationNode {
                    resource: Some(pod("pod-b")),
                    label: "Pod pod-b".to_string(),
                    status: None,
                    namespace: Some("default".to_string()),
                    relation: crate::k8s::relationships::RelationKind::Owned,
                    not_found: false,
                    children: Vec::new(),
                },
                crate::k8s::relationships::RelationNode {
                    resource: Some(pod("pod-a")),
                    label: "Pod pod-a".to_string(),
                    status: None,
                    namespace: Some("default".to_string()),
                    relation: crate::k8s::relationships::RelationKind::Owned,
                    not_found: false,
                    children: Vec::new(),
                },
            ],
        }]);

        let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
        assert_eq!(flat[tab.cursor].resource, Some(pod("pod-b")));
    }

    #[test]
    fn relations_tab_set_tree_clears_loading_and_stale_error() {
        let mut tab = RelationsTabState::new(pod("pod-0"));
        tab.pending_request_id = Some(42);
        tab.loading = true;
        tab.error = Some("previous relationship fetch failed".to_string());

        tab.set_tree(vec![crate::k8s::relationships::RelationNode {
            resource: Some(pod("pod-a")),
            label: "Pod pod-a".to_string(),
            status: None,
            namespace: Some("default".to_string()),
            relation: crate::k8s::relationships::RelationKind::Owned,
            not_found: false,
            children: Vec::new(),
        }]);

        assert!(tab.pending_request_id.is_none());
        assert!(!tab.loading);
        assert!(tab.error.is_none());
        assert_eq!(tab.tree.len(), 1);
    }

    #[test]
    fn close_active_tab_clears_maximized() {
        let mut state = WorkbenchState::default();
        state.open_tab(WorkbenchTabState::ActionHistory(
            ActionHistoryTabState::default(),
        ));
        state.maximized = true;
        state.close_active_tab();
        assert!(!state.maximized);
        assert!(!state.open);
    }

    #[test]
    fn close_resource_tabs_clears_maximized() {
        let mut state = WorkbenchState::default();
        state.open_tab(WorkbenchTabState::ResourceYaml(ResourceYamlTabState {
            resource: pod("pod-0"),
            yaml: None,
            scroll: 0,
            pending_request_id: None,
            loading: false,
            error: None,
        }));
        state.maximized = true;
        state.close_resource_tabs();
        assert!(!state.maximized);
        assert!(!state.open);
    }

    #[test]
    fn close_tabs_for_workspace_restore_drops_port_forward() {
        let mut state = WorkbenchState::default();
        state.open_tab(WorkbenchTabState::ActionHistory(
            ActionHistoryTabState::default(),
        ));
        state.open_tab(WorkbenchTabState::PortForward(PortForwardTabState::new(
            None,
            PortForwardDialog::new(),
        )));
        state.maximized = true;

        state.close_tabs_for_workspace_restore();

        assert!(state.has_tab(&WorkbenchTabKey::ActionHistory));
        assert!(!state.has_tab(&WorkbenchTabKey::PortForward));
        assert_eq!(state.tabs.len(), 1);
        assert!(state.maximized);
    }

    #[test]
    fn exec_set_containers_clears_exited_and_error() {
        let mut tab = ExecTabState::new(pod("pod-0"), 1, "pod-0".into(), "default".into());
        tab.exited = true;
        tab.error = Some("connection lost".into());
        tab.set_containers(vec!["main".into()]);
        assert!(!tab.exited);
        assert!(tab.error.is_none());
        assert_eq!(tab.container_name, "main");
    }

    #[test]
    fn exec_set_containers_preserves_selected_container_identity() {
        let mut tab = ExecTabState::new(pod("pod-0"), 1, "pod-0".into(), "default".into());
        tab.set_containers(vec!["main".into(), "sidecar".into(), "metrics".into()]);
        tab.container_cursor = 1;
        tab.picking_container = true;

        tab.set_containers(vec!["sidecar".into(), "metrics".into(), "main".into()]);

        assert!(tab.picking_container);
        assert_eq!(tab.container_cursor, 0);
        assert_eq!(tab.containers[tab.container_cursor], "sidecar");
    }

    #[test]
    fn exec_restart_session_preserves_selected_container_identity() {
        let mut tab = ExecTabState::new(pod("pod-0"), 1, "pod-0".into(), "default".into());
        tab.input = "echo hi".into();
        tab.input_cursor = tab.input.chars().count();
        tab.set_containers(vec!["main".into(), "sidecar".into(), "metrics".into()]);
        tab.container_name = "sidecar".into();
        tab.lines = vec!["old".into()];
        tab.scroll = 3;
        tab.loading = false;

        tab.restart_session(9, "pod-0".into(), "default".into(), None);

        assert_eq!(tab.session_id, 9);
        assert_eq!(tab.container_name, "sidecar");
        assert!(tab.containers.is_empty());
        assert!(tab.lines.is_empty());
        assert_eq!(tab.scroll, 0);
        assert!(tab.loading);
        assert_eq!(tab.input, "echo hi");
        assert_eq!(tab.input_cursor, "echo hi".chars().count());
    }

    #[test]
    fn exec_append_output_follows_when_at_bottom() {
        let mut tab = ExecTabState::new(pod("pod-0"), 1, "pod-0".into(), "default".into());
        tab.append_output("line1\nline2\n");
        assert_eq!(tab.scroll, 1); // at bottom (2 lines, index 1)
        tab.append_output("line3\n");
        assert_eq!(tab.scroll, 2); // followed to new bottom
    }

    #[test]
    fn exec_append_output_does_not_follow_when_scrolled_up() {
        let mut tab = ExecTabState::new(pod("pod-0"), 1, "pod-0".into(), "default".into());
        tab.append_output("line1\nline2\nline3\n");
        tab.scroll = 0; // user scrolled to top
        tab.append_output("line4\n");
        assert_eq!(tab.scroll, 0); // stays at top
    }

    #[test]
    fn exec_session_id_returns_matching_tab_session() {
        let mut state = WorkbenchState::default();
        state.open_tab(WorkbenchTabState::Exec(ExecTabState::new(
            pod("pod-0"),
            41,
            "pod-0".into(),
            "ns".into(),
        )));

        assert_eq!(state.exec_session_id(&pod("pod-0")), Some(41));
        assert_eq!(state.exec_session_id(&pod("pod-1")), None);
    }

    #[test]
    fn workload_log_follow_mode_sets_scroll_past_end() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.follow_mode = true;
        tab.push_line(WorkloadLogLine {
            pod_name: "pod-0".into(),
            container_name: "main".into(),
            entry: LogEntry::from_raw("hello"),
            is_stderr: false,
        });
        // follow mode sets scroll past end for renderer to clamp
        assert!(tab.scroll >= tab.lines.len());
    }

    #[test]
    fn workload_logs_bootstrap_success_clears_stale_error() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.error = Some("old failure".into());

        let sources =
            tab.apply_bootstrap_targets(vec![crate::k8s::workload_logs::WorkloadLogTarget {
                pod_name: "pod-0".into(),
                namespace: "default".into(),
                containers: vec!["main".into()],
                labels: Vec::new(),
            }]);

        assert_eq!(
            sources,
            vec![("pod-0".into(), "default".into(), "main".into())]
        );
        assert!(tab.error.is_none());
        assert!(!tab.loading);
        assert_eq!(tab.sources, sources);
    }

    #[test]
    fn workload_logs_bootstrap_error_clears_stale_sources() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.sources = vec![("pod-0".into(), "default".into(), "main".into())];
        tab.notice = Some("old notice".into());

        tab.apply_bootstrap_error("boom".into());

        assert!(tab.sources.is_empty());
        assert_eq!(tab.error.as_deref(), Some("boom"));
        assert!(tab.notice.is_none());
        assert!(!tab.loading);
    }

    #[test]
    fn events_timeline_rebuild_clamps_scroll() {
        let mut tab = ResourceEventsTabState::new(pod("pod-0"));
        tab.scroll = 100;
        tab.rebuild_timeline(&crate::action_history::ActionHistoryState::default());
        // timeline is empty, scroll should be 0
        assert_eq!(tab.scroll, 0);
    }

    #[test]
    fn events_timeline_rebuild_clamps_scroll_when_timeline_shrinks() {
        let mut tab = ResourceEventsTabState::new(pod("pod-0"));
        tab.scroll = 99;
        tab.events.push(crate::k8s::events::EventInfo {
            event_type: "Normal".into(),
            reason: "Scheduled".into(),
            message: "long event body".into(),
            first_timestamp: crate::time::now(),
            last_timestamp: crate::time::now(),
            count: 1,
        });

        tab.rebuild_timeline(&crate::action_history::ActionHistoryState::default());

        assert!(!tab.timeline.is_empty());
        assert_eq!(tab.scroll, tab.timeline.len().saturating_sub(1));
    }

    #[test]
    fn workload_log_filter_matches_case_insensitively() {
        let tab = WorkloadLogsTabState {
            text_filter: "error".to_string(),
            ..WorkloadLogsTabState::new(pod("pod-0"), 1)
        };

        assert!(tab.matches_filter(&WorkloadLogLine {
            pod_name: "pod-0".to_string(),
            container_name: "main".to_string(),
            entry: LogEntry::from_raw("ERROR: probe failed"),
            is_stderr: true,
        }));
    }

    #[test]
    fn workload_log_regex_filter_matches_structured_summary() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.filter_input = "req=abc-\\d+".to_string();
        tab.text_filter_mode = LogQueryMode::Regex;
        tab.commit_text_filter();

        assert!(tab.matches_filter(&WorkloadLogLine {
            pod_name: "pod-0".to_string(),
            container_name: "main".to_string(),
            entry: LogEntry::from_raw(
                r#"{"level":"info","message":"startup complete","request_id":"abc-7"}"#,
            ),
            is_stderr: false,
        }));
    }

    #[test]
    fn workload_log_apply_preset_syncs_edit_cursors() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.filter_input = "old".into();
        tab.filter_input_cursor = 1;
        tab.time_jump_input = "2026-01-01T00:00:00Z".into();
        tab.time_jump_cursor = 5;

        tab.apply_preset(&WorkloadLogPreset {
            name: String::new(),
            query: "needle".into(),
            mode: LogQueryMode::Regex,
            time_window: LogTimeWindow::Last1Hour,
            structured_view: false,
            label_filter: None,
            pod_filter: None,
            container_filter: None,
        });

        assert_eq!(tab.filter_input, "needle");
        assert_eq!(tab.filter_input_cursor, "needle".chars().count());
        assert!(tab.time_jump_input.is_empty());
        assert_eq!(tab.time_jump_cursor, 0);
    }

    #[test]
    fn workload_log_label_filter_matches_precomputed_pod_set() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.update_targets(&[
            crate::k8s::workload_logs::WorkloadLogTarget {
                pod_name: "api-0".into(),
                namespace: "default".into(),
                containers: vec!["main".into()],
                labels: vec![("app".into(), "api".into())],
            },
            crate::k8s::workload_logs::WorkloadLogTarget {
                pod_name: "worker-0".into(),
                namespace: "default".into(),
                containers: vec!["main".into()],
                labels: vec![("app".into(), "worker".into())],
            },
        ]);
        tab.cycle_label_filter();

        assert_eq!(tab.label_filter.as_deref(), Some("app=api"));
        assert!(tab.matches_filter(&WorkloadLogLine {
            pod_name: "api-0".to_string(),
            container_name: "main".to_string(),
            entry: LogEntry::from_raw("2026-03-26T10:00:00Z api"),
            is_stderr: false,
        }));
        assert!(!tab.matches_filter(&WorkloadLogLine {
            pod_name: "worker-0".to_string(),
            container_name: "main".to_string(),
            entry: LogEntry::from_raw("2026-03-26T10:00:00Z worker"),
            is_stderr: false,
        }));
    }

    #[test]
    fn workload_log_target_refresh_rebuilds_filter_inventories() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.update_targets(&[crate::k8s::workload_logs::WorkloadLogTarget {
            pod_name: "api-0".into(),
            namespace: "default".into(),
            containers: vec!["main".into(), "sidecar".into()],
            labels: vec![("app".into(), "api".into())],
        }]);
        tab.pod_filter = Some("api-0".into());
        tab.container_filter = Some("sidecar".into());
        tab.label_filter = Some("app=api".into());

        tab.update_targets(&[crate::k8s::workload_logs::WorkloadLogTarget {
            pod_name: "worker-0".into(),
            namespace: "default".into(),
            containers: vec!["main".into()],
            labels: vec![("app".into(), "worker".into())],
        }]);

        assert_eq!(tab.available_pods, vec!["worker-0".to_string()]);
        assert_eq!(tab.available_containers, vec!["main".to_string()]);
        assert_eq!(tab.available_labels, vec!["app=worker".to_string()]);
        assert!(tab.pod_filter.is_none());
        assert!(tab.container_filter.is_none());
        assert!(tab.label_filter.is_none());
    }

    #[test]
    fn workload_log_time_window_filters_stale_entries() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.time_window = LogTimeWindow::Last5Minutes;

        assert!(!tab.matches_filter(&WorkloadLogLine {
            pod_name: "pod-0".to_string(),
            container_name: "main".to_string(),
            entry: LogEntry::from_raw("2020-01-01T00:00:00Z old line"),
            is_stderr: false,
        }));
        assert!(tab.matches_filter(&WorkloadLogLine {
            pod_name: "pod-0".to_string(),
            container_name: "main".to_string(),
            entry: LogEntry::from_raw("2099-01-01T00:00:00Z future line"),
            is_stderr: false,
        }));
    }

    #[test]
    fn workload_log_filter_roundtrip_preserves_selected_line_across_zero_matches() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.lines = vec![
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("alpha line"),
                is_stderr: false,
            },
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("beta line"),
                is_stderr: false,
            },
        ];
        tab.scroll = 1;

        tab.filter_input = "zzz".into();
        tab.commit_text_filter();
        assert!(tab.filtered_indices().is_empty());
        assert_eq!(tab.scroll, 0);

        tab.filter_input.clear();
        tab.commit_text_filter();

        assert_eq!(
            tab.current_filtered_line().map(|line| line.entry.raw()),
            Some("beta line")
        );
        assert_eq!(tab.scroll, 1);
    }

    #[test]
    fn workload_log_unchanged_filter_commit_keeps_scroll() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.lines = vec![
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("ready first"),
                is_stderr: false,
            },
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("ready current"),
                is_stderr: false,
            },
        ];
        tab.text_filter = "ready".to_string();
        tab.filter_input = "ready".to_string();
        tab.compiled_text_filter = compile_query("ready", LogQueryMode::Substring)
            .expect("substring filter should compile");
        tab.scroll = 1;
        tab.editing_text_filter = true;
        tab.text_filter_error = Some("stale".to_string());
        tab.time_jump_error = Some("stale jump".to_string());

        tab.commit_text_filter();

        assert_eq!(tab.text_filter, "ready");
        assert_eq!(tab.scroll, 1);
        assert!(!tab.editing_text_filter);
        assert!(tab.text_filter_error.is_none());
        assert!(tab.time_jump_error.is_none());
    }

    #[test]
    fn workload_log_filtered_len_matches_filtered_indices() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.lines = vec![
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("ready first"),
                is_stderr: false,
            },
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("skip this"),
                is_stderr: false,
            },
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("ready second"),
                is_stderr: false,
            },
        ];
        tab.text_filter = "ready".to_string();
        tab.compiled_text_filter = compile_query("ready", LogQueryMode::Substring)
            .expect("substring filter should compile");

        assert_eq!(tab.filtered_len(), tab.filtered_indices().len());
        assert_eq!(tab.filtered_len(), 2);
    }

    #[test]
    fn workload_log_restore_scroll_updates_anchor_from_existing_filtered_pass() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.lines = vec![
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("ready first"),
                is_stderr: false,
            },
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("skip this"),
                is_stderr: false,
            },
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("ready second"),
                is_stderr: false,
            },
        ];
        tab.text_filter = "ready".to_string();
        tab.compiled_text_filter = compile_query("ready", LogQueryMode::Substring)
            .expect("substring filter should compile");
        tab.scroll = 1;

        tab.restore_filtered_scroll(None);

        assert_eq!(tab.scroll, 1);
        assert_eq!(
            tab.filtered_line_anchor
                .as_ref()
                .map(|line| line.entry.raw()),
            Some("ready second")
        );
    }

    #[test]
    fn workload_log_restore_scroll_preserves_visible_line_identity() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        let selected = WorkloadLogLine {
            pod_name: "pod-0".to_string(),
            container_name: "main".to_string(),
            entry: LogEntry::from_raw("ready selected"),
            is_stderr: false,
        };
        tab.lines = vec![
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("ready first"),
                is_stderr: false,
            },
            selected.clone(),
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("ready third"),
                is_stderr: false,
            },
        ];
        tab.text_filter = "ready".to_string();
        tab.compiled_text_filter = compile_query("ready", LogQueryMode::Substring)
            .expect("substring filter should compile");
        tab.scroll = 0;

        tab.restore_filtered_scroll(Some(selected));

        assert_eq!(tab.scroll, 1);
        assert_eq!(
            tab.filtered_line_anchor
                .as_ref()
                .map(|line| line.entry.raw()),
            Some("ready selected")
        );
    }

    #[test]
    fn workload_log_restore_scroll_clamps_to_last_visible_anchor() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.lines = vec![
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("ready first"),
                is_stderr: false,
            },
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("skip this"),
                is_stderr: false,
            },
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("ready second"),
                is_stderr: false,
            },
        ];
        tab.text_filter = "ready".to_string();
        tab.compiled_text_filter = compile_query("ready", LogQueryMode::Substring)
            .expect("substring filter should compile");
        tab.scroll = usize::MAX;

        tab.restore_filtered_scroll(None);

        assert_eq!(tab.scroll, 1);
        assert_eq!(
            tab.filtered_line_anchor
                .as_ref()
                .map(|line| line.entry.raw()),
            Some("ready second")
        );
    }

    #[test]
    fn workload_log_current_filtered_line_clamps_without_filtered_vector() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.lines = vec![
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("ready first"),
                is_stderr: false,
            },
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("skip this"),
                is_stderr: false,
            },
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("ready second"),
                is_stderr: false,
            },
        ];
        tab.text_filter = "ready".to_string();
        tab.compiled_text_filter = compile_query("ready", LogQueryMode::Substring)
            .expect("substring filter should compile");
        tab.scroll = usize::MAX;

        assert_eq!(
            tab.current_filtered_line().map(|line| line.entry.raw()),
            Some("ready second")
        );
    }

    #[test]
    fn workload_log_time_jump_uses_visible_filtered_ordinal() {
        let mut tab = WorkloadLogsTabState::new(pod("pod-0"), 1);
        tab.lines = vec![
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("2026-03-26T10:00:00Z ready first"),
                is_stderr: false,
            },
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("2026-03-26T10:06:00Z hidden nearer"),
                is_stderr: false,
            },
            WorkloadLogLine {
                pod_name: "pod-0".to_string(),
                container_name: "main".to_string(),
                entry: LogEntry::from_raw("2026-03-26T10:08:00Z ready second"),
                is_stderr: false,
            },
        ];
        tab.text_filter = "ready".to_string();
        tab.compiled_text_filter = compile_query("ready", LogQueryMode::Substring)
            .expect("substring filter should compile");
        tab.open_time_jump();
        tab.time_jump_input = "2026-03-26T10:06:30Z".into();

        tab.commit_time_jump();

        assert_eq!(tab.scroll, 1);
        assert!(!tab.jumping_to_time);
        assert!(tab.time_jump_error.is_none());
        assert_eq!(
            tab.current_filtered_line().map(|line| line.entry.raw()),
            Some("2026-03-26T10:08:00Z ready second")
        );
    }

    #[test]
    fn action_history_tab_preserves_selected_entry_identity_when_entries_prepend() {
        let mut tab = ActionHistoryTabState::default();
        let initial = [41_u64, 40_u64];
        tab.select_bottom(&initial);

        let refreshed = [42_u64, 41_u64, 40_u64];
        tab.sync_selection(&refreshed);

        assert_eq!(tab.selected_index(&refreshed), 2);
    }

    #[test]
    fn action_history_tab_clamps_when_selected_entry_disappears() {
        let mut tab = ActionHistoryTabState::default();
        let initial = [11_u64, 10_u64];
        tab.select_bottom(&initial);

        let refreshed = [11_u64];
        tab.sync_selection(&refreshed);

        assert_eq!(tab.selected_index(&refreshed), 0);
    }

    #[test]
    fn connectivity_target_refresh_preserves_selected_target_identity() {
        let mut tab = ConnectivityTabState::new(
            ResourceRef::Pod("source".into(), "default".into()),
            vec![
                ConnectivityTargetOption {
                    resource: ResourceRef::Pod("api-0".into(), "default".into()),
                    display: "api-0".into(),
                    status: "ready".into(),
                    pod_ip: Some("10.0.0.2".into()),
                },
                ConnectivityTargetOption {
                    resource: ResourceRef::Pod("api-1".into(), "default".into()),
                    display: "api-1".into(),
                    status: "ready".into(),
                    pod_ip: Some("10.0.0.3".into()),
                },
            ],
        );
        tab.select_bottom_target();

        tab.apply_targets(vec![
            ConnectivityTargetOption {
                resource: ResourceRef::Pod("api-00".into(), "default".into()),
                display: "api-00".into(),
                status: "ready".into(),
                pod_ip: Some("10.0.0.1".into()),
            },
            ConnectivityTargetOption {
                resource: ResourceRef::Pod("api-0".into(), "default".into()),
                display: "api-0".into(),
                status: "ready".into(),
                pod_ip: Some("10.0.0.2".into()),
            },
            ConnectivityTargetOption {
                resource: ResourceRef::Pod("api-1".into(), "default".into()),
                display: "api-1".into(),
                status: "ready".into(),
                pod_ip: Some("10.0.0.3".into()),
            },
        ]);

        assert_eq!(
            tab.selected_target_option()
                .map(|target| target.resource.clone()),
            Some(ResourceRef::Pod("api-1".into(), "default".into()))
        );
    }

    #[test]
    fn connectivity_target_refresh_clears_stale_error_without_result() {
        let mut tab = ConnectivityTabState::new(
            ResourceRef::Pod("source".into(), "default".into()),
            vec![ConnectivityTargetOption {
                resource: ResourceRef::Pod("api-0".into(), "default".into()),
                display: "api-0".into(),
                status: "ready".into(),
                pod_ip: Some("10.0.0.2".into()),
            }],
        );
        tab.set_error("previous query failed".into());

        tab.apply_targets(vec![ConnectivityTargetOption {
            resource: ResourceRef::Pod("api-0".into(), "default".into()),
            display: "api-0".into(),
            status: "ready".into(),
            pod_ip: Some("10.0.0.2".into()),
        }]);

        assert!(tab.error.is_none());
        assert!(tab.current_target.is_none());
        assert!(tab.tree.is_empty());
        assert_eq!(tab.summary_lines, default_connectivity_summary());
        assert_eq!(tab.focus, ConnectivityTabFocus::Targets);
    }

    #[test]
    fn connectivity_filter_zero_results_preserves_selected_identity_for_restore() {
        let selected = ResourceRef::Pod("api-1".into(), "default".into());
        let mut tab = ConnectivityTabState::new(
            ResourceRef::Pod("source".into(), "default".into()),
            vec![
                ConnectivityTargetOption {
                    resource: ResourceRef::Pod("api-0".into(), "default".into()),
                    display: "api-0".into(),
                    status: "ready".into(),
                    pod_ip: Some("10.0.0.2".into()),
                },
                ConnectivityTargetOption {
                    resource: selected.clone(),
                    display: "api-1".into(),
                    status: "ready".into(),
                    pod_ip: Some("10.0.0.3".into()),
                },
            ],
        );
        tab.select_bottom_target();

        tab.filter.value = "zzz".into();
        tab.refresh_filter();
        assert!(tab.selected_target_option().is_none());

        tab.filter.value.clear();
        tab.refresh_filter();

        assert_eq!(
            tab.selected_target_option()
                .map(|target| target.resource.clone()),
            Some(selected)
        );
    }

    #[test]
    fn decoded_secret_clamp_preserves_selected_key_identity() {
        let mut tab =
            DecodedSecretTabState::new(ResourceRef::Secret("app-secret".into(), "default".into()));
        tab.entries = vec![
            DecodedSecretEntry {
                key: "alpha".into(),
                value: crate::secret::DecodedSecretValue::Text {
                    original: "a".into(),
                    current: "a".into(),
                },
            },
            DecodedSecretEntry {
                key: "beta".into(),
                value: crate::secret::DecodedSecretValue::Text {
                    original: "b".into(),
                    current: "b".into(),
                },
            },
        ];
        tab.select_bottom();

        tab.entries = vec![
            DecodedSecretEntry {
                key: "aardvark".into(),
                value: crate::secret::DecodedSecretValue::Text {
                    original: "aa".into(),
                    current: "aa".into(),
                },
            },
            DecodedSecretEntry {
                key: "alpha".into(),
                value: crate::secret::DecodedSecretValue::Text {
                    original: "a".into(),
                    current: "a".into(),
                },
            },
            DecodedSecretEntry {
                key: "beta".into(),
                value: crate::secret::DecodedSecretValue::Text {
                    original: "b".into(),
                    current: "b".into(),
                },
            },
        ];
        tab.clamp_selected();

        assert_eq!(
            tab.selected_entry().map(|entry| entry.key.as_str()),
            Some("beta")
        );
    }
}
