//! Application state machine and keyboard input handling.

pub mod detail_state;
mod input;
pub mod resource_ref;
pub mod sidebar;
pub mod sort;
pub mod views;

pub use detail_state::*;
pub use resource_ref::ResourceRef;
pub use sidebar::{SidebarItem, sidebar_rows};
pub use sort::{
    PodSortColumn, PodSortState, WorkloadSortColumn, WorkloadSortState, filtered_pod_indices,
    filtered_workload_indices,
};
pub use views::{AppView, NavGroup};

use std::{collections::HashMap, collections::HashSet, fs, path::Path, time::Instant};

use serde::{Deserialize, Serialize};

use crate::{
    action_history::{ActionHistoryState, ActionHistoryTarget, ActionKind, ActionStatus},
    bookmarks::{BookmarkEntry, BookmarkToggleResult, selected_bookmark_resource, toggle_bookmark},
    k8s::{client::EventInfo, dtos::CustomResourceInfo},
    policy::DetailAction,
    preferences::{ClusterPreferences, UserPreferences},
    ui::components::{
        CommandPalette, ContextPicker, NamespacePicker,
        port_forward_dialog::PortForwardDialog,
        probe_panel::ProbePanelState as ProbePanelComponentState,
        scale_dialog::{ScaleDialogState, ScaleTargetKind},
    },
    workbench::{
        ActionHistoryTabState, DEFAULT_WORKBENCH_HEIGHT, DecodedSecretTabState, ExecTabState,
        PodLogsTabState, PortForwardTabState, ResourceEventsTabState, ResourceYamlTabState,
        WorkbenchState, WorkbenchTabState, WorkloadLogsTabState,
    },
};

/// Actions emitted by input handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    None,
    RefreshData,
    FluxReconcile,
    Quit,
    OpenDetail(ResourceRef),
    CloseDetail,
    OpenNamespacePicker,
    CloseNamespacePicker,
    SelectNamespace(String),
    OpenContextPicker,
    CloseContextPicker,
    SelectContext(String),
    OpenCommandPalette,
    CloseCommandPalette,
    NavigateTo(AppView),
    ToggleNavGroup(NavGroup),
    EscapePressed,
    LogsViewerOpen,
    LogsViewerClose,
    LogsViewerScrollUp,
    LogsViewerScrollDown,
    LogsViewerScrollTop,
    LogsViewerScrollBottom,
    LogsViewerToggleFollow,
    LogsViewerTogglePrevious,
    LogsViewerToggleTimestamps,
    LogsViewerSelectContainer(String),
    /// User chose "All Containers" in the pod logs picker.
    LogsViewerSelectAllContainers,
    LogsViewerPickerUp,
    LogsViewerPickerDown,
    LogsViewerSearchOpen,
    LogsViewerSearchClose,
    LogsViewerSearchCancel,
    LogsViewerSearchNext,
    LogsViewerSearchPrev,
    OpenResourceYaml,
    OpenDecodedSecret,
    OpenResourceEvents,
    OpenActionHistory,
    OpenExec,
    PortForwardOpen,
    PortForwardCreate(
        (
            crate::k8s::portforward::PortForwardTarget,
            crate::k8s::portforward::PortForwardConfig,
        ),
    ),
    PortForwardRefresh,
    PortForwardStop(String),
    ScaleDialogOpen,
    ScaleDialogClose,
    ScaleDialogUpdateInput(char),
    ScaleDialogBackspace,
    ScaleDialogIncrement,
    ScaleDialogDecrement,
    ScaleDialogSubmit,
    ProbePanelOpen,
    ProbePanelClose,
    ProbeToggleExpand,
    ProbeSelectNext,
    ProbeSelectPrev,
    ToggleWorkbench,
    WorkbenchNextTab,
    WorkbenchPreviousTab,
    WorkbenchCloseActiveTab,
    WorkbenchIncreaseHeight,
    WorkbenchDecreaseHeight,
    WorkbenchToggleMaximize,
    ActionHistoryOpenSelected,
    ExecSelectContainer(String),
    ExecSendInput,
    RolloutRestart,
    EditYaml,
    DeleteResource,
    ForceDeleteResource,
    TriggerCronJob,
    ConfirmCronJobSuspend(bool),
    SetCronJobSuspend(bool),
    CycleTheme,
    CycleIconMode,
    OpenHelp,
    CloseHelp,
    CopyResourceName,
    CopyResourceFullName,
    CopyLogContent,
    ExportLogs,
    ToggleBookmark,
    SaveDecodedSecret,
    PaletteAction {
        action: crate::policy::DetailAction,
        resource: ResourceRef,
    },
    OpenRelationships,
    ConfirmDrainNode,
    CordonNode,
    UncordonNode,
    DrainNode,
    ForceDrainNode,
    ToggleDetailMetadata,
}

/// Which panel currently owns keyboard focus.
///
/// Focus determines how `j`/`k`/`↑`/`↓` are routed:
/// - [`Focus::Sidebar`] → moves `sidebar_cursor` through the nav tree.
/// - [`Focus::Content`] → increments/decrements `selected_idx` in the active list.
///
/// # Transitions
/// - **Sidebar → Content**: `Enter` on a [`SidebarItem::View`] row (via [`AppState::sidebar_activate`]).
/// - **Content → Sidebar**: `Esc` while no detail view is open.
/// - **Tab / BackTab**: cycle through views directly, always lands in Content focus.
/// - **Command palette `NavigateTo`**: jumps to a view, lands in Content focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    /// Sidebar navigation panel has focus (default on startup).
    ///
    /// `j`/`k` move the sidebar cursor. `Enter` activates the highlighted row
    /// (either toggling a [`NavGroup`] or navigating to an [`AppView`]).
    #[default]
    Sidebar,
    /// Main content area has focus.
    ///
    /// `j`/`k` scroll `selected_idx` through the resource list. `Enter` opens
    /// the detail view for the highlighted row. `Esc` returns focus to the sidebar.
    Content,
    /// Bottom workbench owns keyboard input.
    ///
    /// `j`/`k` and related keys are delegated to the active workbench tab while
    /// the main list remains visible in the background.
    Workbench,
}

/// Runtime state for UI interaction and navigation.
///
/// # Navigation model
///
/// The UI has two independently navigable panels: the **sidebar** and the **content area**.
/// [`AppState::focus`] tracks which panel owns keyboard input at any given moment.
///
/// ```text
/// ┌─ Sidebar (Focus::Sidebar) ──┐  ┌─ Content (Focus::Content) ──────────────┐
/// │  ▼ Workloads                │  │  NAME        READY  STATUS  RESTARTS AGE │
/// │    Pods              ←─ Enter activates ──→  row 0  ← selected_idx        │
/// │    Deployments              │  │  row 1                                    │
/// │    ...                      │  │  row 2                                    │
/// └─────────────────────────────┘  └───────────────────────────────────────────┘
///       j/k: sidebar_cursor              j/k: selected_idx
///       Enter: navigate → Content        Enter: open detail view
///                                        Esc: return → Sidebar
/// ```
/// A transient notification that auto-dismisses.
#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub is_error: bool,
    pub created_at: Instant,
}

#[derive(Debug, Clone)]
pub struct AppState {
    /// The currently active top-level view (e.g. Pods, Deployments).
    pub view: AppView,
    /// Zero-based index of the highlighted row in the active content list.
    /// Reset to `0` on every view change.
    pub selected_idx: usize,
    pub search_query: String,
    pub is_search_mode: bool,
    pub should_quit: bool,
    pub confirm_quit: bool,
    pub error_message: Option<String>,
    pub status_message: Option<String>,
    pub detail_view: Option<DetailViewState>,
    pub current_namespace: String,
    pub namespace_picker: NamespacePicker,
    pub context_picker: ContextPicker,
    pub command_palette: CommandPalette,
    pub help_overlay: crate::ui::components::help_overlay::HelpOverlay,
    /// Set of [`NavGroup`]s that are currently collapsed in the sidebar.
    pub collapsed_groups: HashSet<NavGroup>,
    /// Zero-based index of the highlighted row in the sidebar nav tree.
    /// Includes both group headers and view rows; collapsed groups hide their children.
    pub sidebar_cursor: usize,
    /// Which panel currently owns keyboard focus. See [`Focus`] for routing rules.
    pub focus: Focus,
    pub extension_instances: Vec<CustomResourceInfo>,
    pub extension_error: Option<String>,
    pub extension_selected_crd: Option<String>,
    /// When true, keyboard focus is on the instances pane (right) instead of CRD picker (left).
    pub extension_in_instances: bool,
    /// Cursor index within the instances list.
    pub extension_instance_cursor: usize,
    /// Auto-refresh interval in seconds (0 = disabled).
    pub refresh_interval_secs: u64,
    /// Optional shared name/age sort mode for workload list views.
    pub workload_sort: Option<WorkloadSortState>,
    /// Optional sort mode for Pods view.
    pub pod_sort: Option<PodSortState>,
    /// Active port-forward tunnels displayed in the PortForwarding view.
    pub tunnel_registry: crate::state::port_forward::TunnelRegistry,
    /// Canonical mutation/action history.
    pub action_history: ActionHistoryState,
    /// Global user preferences for view sort/column customization.
    pub preferences: Option<UserPreferences>,
    /// Per-cluster preference overrides, keyed by kube context name.
    pub cluster_preferences: Option<HashMap<String, ClusterPreferences>>,
    /// Active kube context name (for per-cluster preferences).
    pub current_context_name: Option<String>,
    /// When true, config should be saved at next convenient point.
    pub needs_config_save: bool,
    /// Persistent bottom workbench state.
    pub workbench: WorkbenchState,
    /// Spinner animation tick counter (0–7), advanced on each UI tick during loading.
    pub spinner_tick: u8,
    /// Stack of timed toast notifications (max 3).
    pub toasts: Vec<Toast>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            view: AppView::Dashboard,
            selected_idx: 0,
            search_query: String::new(),
            is_search_mode: false,
            should_quit: false,
            confirm_quit: false,
            error_message: None,
            status_message: None,
            detail_view: None,
            current_namespace: "all".to_string(),
            namespace_picker: NamespacePicker::new(vec!["all".to_string(), "default".to_string()]),
            context_picker: ContextPicker::default(),
            command_palette: CommandPalette::default(),
            help_overlay: crate::ui::components::help_overlay::HelpOverlay::default(),
            collapsed_groups: {
                // Start with all groups collapsed except Overview (default view's group).
                let mut collapsed = HashSet::new();
                for group in sidebar::all_groups() {
                    if group != NavGroup::Overview {
                        collapsed.insert(group);
                    }
                }
                collapsed
            },
            sidebar_cursor: 0,
            focus: Focus::Sidebar,
            extension_instances: Vec::new(),
            extension_error: None,
            extension_selected_crd: None,
            extension_in_instances: false,
            extension_instance_cursor: 0,
            refresh_interval_secs: 30,
            workload_sort: None,
            pod_sort: None,
            tunnel_registry: crate::state::port_forward::TunnelRegistry::new(),
            action_history: ActionHistoryState::default(),
            preferences: None,
            cluster_preferences: None,
            current_context_name: None,
            needs_config_save: false,
            workbench: WorkbenchState::default(),
            spinner_tick: 0,
            toasts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    namespace: String,
    #[serde(default)]
    theme: Option<String>,
    /// Auto-refresh interval in seconds (0 = disabled, default = 30).
    #[serde(default = "default_refresh_interval")]
    refresh_interval_secs: u64,
    #[serde(default)]
    workbench_open: bool,
    #[serde(default = "default_workbench_height")]
    workbench_height: u16,
    #[serde(default)]
    collapsed_nav_groups: Vec<String>,
    #[serde(default)]
    preferences: Option<UserPreferences>,
    #[serde(default)]
    clusters: Option<HashMap<String, ClusterPreferences>>,
}

fn default_refresh_interval() -> u64 {
    30
}

fn default_workbench_height() -> u16 {
    DEFAULT_WORKBENCH_HEIGHT
}

impl AppState {
    /// Returns the active top-level view.
    pub fn view(&self) -> AppView {
        self.view
    }

    /// Returns the currently selected list index.
    pub fn selected_idx(&self) -> usize {
        self.selected_idx
    }

    /// Returns the active shared sort mode for the given view, if supported.
    pub fn workload_sort_for_view(&self, view: AppView) -> Option<WorkloadSortState> {
        self.workload_sort
            .filter(|sort| view.supports_shared_sort(sort.column))
    }

    /// Returns the active shared sort mode for the current view, if supported.
    pub fn workload_sort(&self) -> Option<WorkloadSortState> {
        self.workload_sort_for_view(self.view)
    }

    /// Returns the active search query.
    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    /// Returns the currently active Pods sort mode, if any.
    pub fn pod_sort(&self) -> Option<PodSortState> {
        self.pod_sort
    }

    /// Returns whether the app is currently in search input mode.
    pub fn is_search_mode(&self) -> bool {
        self.is_search_mode
    }

    /// Returns the current workbench state.
    pub fn workbench(&self) -> &WorkbenchState {
        &self.workbench
    }

    pub fn workbench_mut(&mut self) -> &mut WorkbenchState {
        &mut self.workbench
    }

    pub fn action_history(&self) -> &ActionHistoryState {
        &self.action_history
    }

    pub fn open_action_history_tab(&mut self, focus: bool) {
        let history_key = crate::workbench::WorkbenchTabKey::ActionHistory;
        if focus {
            if !self.workbench.activate_tab(&history_key) {
                self.workbench.open_tab(WorkbenchTabState::ActionHistory(
                    ActionHistoryTabState::default(),
                ));
            }
            self.focus_workbench();
        } else if !self.workbench.has_tab(&history_key) {
            self.workbench
                .ensure_background_tab(WorkbenchTabState::ActionHistory(
                    ActionHistoryTabState::default(),
                ));
        }
    }

    pub fn record_action_pending(
        &mut self,
        kind: ActionKind,
        origin_view: AppView,
        resource: Option<ResourceRef>,
        resource_label: impl Into<String>,
        message: impl Into<String>,
    ) -> u64 {
        self.open_action_history_tab(false);
        let affected_resource = resource.clone();
        let target = resource.map(|resource| ActionHistoryTarget {
            view: origin_view,
            resource,
        });
        let id = self
            .action_history
            .record_pending(kind, resource_label, message, target);
        self.rebuild_timeline_for(affected_resource.as_ref());
        id
    }

    pub fn complete_action_history(
        &mut self,
        entry_id: u64,
        status: ActionStatus,
        message: impl Into<String>,
        keep_target: bool,
    ) {
        // Look up the affected resource before completing (complete may clear target).
        let affected_resource = self
            .action_history
            .find_by_id(entry_id)
            .and_then(|e| e.target.as_ref().map(|t| t.resource.clone()));
        self.action_history
            .complete(entry_id, status, message, keep_target);
        self.rebuild_timeline_for(affected_resource.as_ref());
    }

    /// Rebuild timeline only for the specific resource's tab (or all if resource is None).
    fn rebuild_timeline_for(&mut self, resource: Option<&ResourceRef>) {
        for tab in &mut self.workbench.tabs {
            if let WorkbenchTabState::ResourceEvents(events_tab) = &mut tab.state {
                let dominated = match resource {
                    Some(r) => events_tab.resource == *r,
                    None => true,
                };
                if dominated {
                    events_tab.rebuild_timeline(&self.action_history);
                }
            }
        }
    }

    pub fn selected_action_history_target(&self) -> Option<&ActionHistoryTarget> {
        let tab = self.workbench.active_tab()?;
        let WorkbenchTabState::ActionHistory(history_tab) = &tab.state else {
            return None;
        };
        self.action_history
            .get(history_tab.selected)
            .and_then(|entry| entry.target.as_ref())
    }

    /// Returns whether the event loop should terminate.
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Returns the latest UI-level error, if any.
    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    /// Returns the latest non-error status message, if any.
    pub fn status_message(&self) -> Option<&str> {
        self.status_message.as_deref()
    }

    /// Sets an error message to be shown in the status bar.
    pub fn set_error(&mut self, message: String) {
        self.status_message = None;
        self.error_message = Some(message);
    }

    /// Clears any active error message.
    pub fn clear_error(&mut self) {
        self.error_message = None;
    }

    /// Sets a transient non-error status message in the status bar.
    pub fn set_status(&mut self, message: String) {
        self.error_message = None;
        self.status_message = Some(message);
    }

    /// Clears any active non-error status message.
    pub fn clear_status(&mut self) {
        self.status_message = None;
    }

    /// Advance the spinner animation frame (wraps at 8).
    pub fn advance_spinner(&mut self) {
        self.spinner_tick = self.spinner_tick.wrapping_add(1) % 8;
    }

    /// Returns the current braille spinner character.
    pub fn spinner_char(&self) -> char {
        const FRAMES: [char; 8] = [
            '\u{280B}', '\u{2819}', '\u{2839}', '\u{2838}', '\u{283C}', '\u{2834}', '\u{2826}',
            '\u{2827}',
        ];
        FRAMES[self.spinner_tick as usize % FRAMES.len()]
    }

    /// Push a timed toast notification (max 3 in stack).
    pub fn push_toast(&mut self, message: String, is_error: bool) {
        self.toasts.push(Toast {
            message,
            is_error,
            created_at: Instant::now(),
        });
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
    }

    /// Expire toasts older than 5 seconds. Returns true if any expired.
    pub fn expire_toasts(&mut self) -> bool {
        let before = self.toasts.len();
        self.toasts
            .retain(|t| t.created_at.elapsed() < std::time::Duration::from_secs(5));
        self.toasts.len() != before
    }

    pub fn toggle_workbench(&mut self) {
        self.workbench.toggle_open();
        if !self.workbench.open && self.focus == Focus::Workbench {
            self.focus = Focus::Content;
        }
    }

    pub fn workbench_next_tab(&mut self) {
        self.workbench.next_tab();
    }

    pub fn workbench_previous_tab(&mut self) {
        self.workbench.previous_tab();
    }

    pub fn workbench_close_active_tab(&mut self) {
        self.workbench.close_active_tab();
        self.sync_workbench_focus();
    }

    /// Resets focus to Content when the workbench has no tabs left.
    pub fn sync_workbench_focus(&mut self) {
        if self.workbench.tabs.is_empty() && self.focus == Focus::Workbench {
            self.focus = Focus::Content;
        }
    }

    pub fn workbench_increase_height(&mut self) {
        self.workbench.resize_larger();
    }

    pub fn workbench_decrease_height(&mut self) {
        self.workbench.resize_smaller();
    }

    pub fn workbench_toggle_maximize(&mut self) {
        self.workbench.toggle_maximize();
    }

    /// Sets active namespace for namespaced resource fetches.
    pub fn set_namespace(&mut self, ns: String) {
        self.current_namespace = ns;
        self.selected_idx = 0;
        self.search_query.clear();
        self.is_search_mode = false;
    }

    /// Returns currently active namespace (`all` means cluster-wide listing).
    pub fn get_namespace(&self) -> &str {
        &self.current_namespace
    }

    /// Returns true when namespace picker modal is open.
    pub fn is_namespace_picker_open(&self) -> bool {
        self.namespace_picker.is_open()
    }

    /// Returns true when context picker modal is open.
    pub fn is_context_picker_open(&self) -> bool {
        self.context_picker.is_open()
    }

    /// Opens the context picker modal with the given contexts.
    pub fn open_context_picker(&mut self, contexts: Vec<String>, current: Option<String>) {
        self.context_picker.set_contexts(contexts, current);
        self.context_picker.open();
    }

    /// Closes the context picker modal.
    pub fn close_context_picker(&mut self) {
        self.context_picker.close();
    }

    /// Returns namespace picker state.
    pub fn namespace_picker(&self) -> &NamespacePicker {
        &self.namespace_picker
    }

    /// Replaces available namespace options while preserving current selection if possible.
    pub fn set_available_namespaces(&mut self, mut namespaces: Vec<String>) {
        namespaces.retain(|ns| !ns.is_empty());
        namespaces.sort();
        namespaces.dedup();

        if !namespaces.iter().any(|ns| ns == "all") {
            namespaces.insert(0, "all".to_string());
        }

        if !namespaces.iter().any(|ns| ns == &self.current_namespace) {
            namespaces.push(self.current_namespace.clone());
            namespaces.sort();
            namespaces.dedup();
        }

        self.namespace_picker.set_namespaces(namespaces);
    }

    /// Opens namespace picker modal.
    pub fn open_namespace_picker(&mut self) {
        self.namespace_picker.open();
    }

    /// Closes namespace picker modal.
    pub fn close_namespace_picker(&mut self) {
        self.namespace_picker.close();
    }

    /// Updates the currently displayed custom resource instances for Extensions view.
    pub fn begin_extension_instances_load(&mut self, crd_name: String) {
        self.extension_selected_crd = Some(crd_name);
        self.extension_instances.clear();
        self.extension_error = None;
        self.extension_instance_cursor = 0;
    }

    /// Updates the currently displayed custom resource instances for Extensions view.
    pub fn set_extension_instances(
        &mut self,
        crd_name: String,
        instances: Vec<CustomResourceInfo>,
        error: Option<String>,
    ) {
        self.extension_selected_crd = Some(crd_name);
        self.extension_instances = instances;
        self.extension_error = error;
        self.extension_instance_cursor = 0;
    }

    /// Advances to the next view in [`AppView::ORDER`], wrapping around.
    /// Resets `selected_idx` and syncs `sidebar_cursor` to the new view.
    /// Shared state reset when switching to a different view.
    /// Clears search, resets selection, syncs sidebar, and loads sort preferences.
    pub fn navigate_to_view(&mut self, view: AppView) {
        self.view = view;
        self.selected_idx = 0;
        self.search_query.clear();
        self.is_search_mode = false;
        self.sync_collapsed_to_active_view();
        self.apply_sort_from_preferences(crate::columns::view_key(self.view));
    }

    /// Advances to the next view in [`AppView::ORDER`], wrapping around.
    /// Triggered by `Tab`. Focus is not changed (Tab always targets content).
    fn next_view(&mut self) {
        self.navigate_to_view(self.view.next());
    }

    /// Retreats to the previous view in [`AppView::ORDER`], wrapping around.
    /// Triggered by `Shift+Tab`.
    fn previous_view(&mut self) {
        self.navigate_to_view(self.view.previous());
    }

    /// Moves the content list selection down one row (saturates at `usize::MAX`).
    /// Called when [`Focus::Content`] is active and `j`/`↓` is pressed.
    fn select_next(&mut self) {
        self.selected_idx = self.selected_idx.saturating_add(1);
    }

    /// Moves the content list selection up one row (saturates at `0`).
    /// Called when [`Focus::Content`] is active and `k`/`↑` is pressed.
    fn select_previous(&mut self) {
        self.selected_idx = self.selected_idx.saturating_sub(1);
    }

    fn set_or_toggle_pod_sort(&mut self, column: PodSortColumn) {
        self.selected_idx = 0;
        self.pod_sort = match self.pod_sort {
            Some(current) if current.column == column => {
                Some(PodSortState::new(column, !current.descending))
            }
            _ => Some(PodSortState::new(column, column.default_descending())),
        };
        self.save_sort_to_preferences("pods");
    }

    fn clear_pod_sort(&mut self) {
        self.selected_idx = 0;
        self.pod_sort = None;
        self.save_sort_to_preferences("pods");
    }

    fn set_or_toggle_workload_sort(&mut self, column: WorkloadSortColumn) {
        self.selected_idx = 0;
        self.workload_sort = match self.workload_sort {
            Some(current) if current.column == column => {
                Some(WorkloadSortState::new(column, !current.descending))
            }
            _ => Some(WorkloadSortState::new(column, column.default_descending())),
        };
        let view_key = crate::columns::view_key(self.view);
        self.save_sort_to_preferences(view_key);
    }

    fn clear_workload_sort(&mut self) {
        self.selected_idx = 0;
        self.workload_sort = None;
        let view_key = crate::columns::view_key(self.view);
        self.save_sort_to_preferences(view_key);
    }

    /// Returns a mutable reference to the `ViewPreferences` for the given view key,
    /// writing to cluster-specific prefs when a context is active and cluster prefs
    /// already exist for that context, otherwise writing to global prefs.
    fn view_prefs_mut(&mut self, view_key: &str) -> &mut crate::preferences::ViewPreferences {
        if let Some(ctx) = &self.current_context_name
            && let Some(clusters) = &mut self.cluster_preferences
            && let Some(cluster) = clusters.get_mut(ctx)
        {
            return cluster.views.entry(view_key.to_string()).or_default();
        }
        let global = self.preferences.get_or_insert_with(Default::default);
        global.views.entry(view_key.to_string()).or_default()
    }

    fn cluster_prefs_mut(&mut self) -> Option<&mut ClusterPreferences> {
        let context = self.current_context_name.clone()?;
        let clusters = self
            .cluster_preferences
            .get_or_insert_with(Default::default);
        Some(clusters.entry(context).or_default())
    }

    pub fn bookmarks(&self) -> &[BookmarkEntry] {
        self.current_context_name
            .as_deref()
            .and_then(|ctx| {
                self.cluster_preferences
                    .as_ref()
                    .and_then(|clusters| clusters.get(ctx))
            })
            .map(|prefs| prefs.bookmarks.as_slice())
            .unwrap_or(&[])
    }

    pub fn bookmark_count(&self) -> usize {
        self.bookmarks().len()
    }

    pub fn is_bookmarked(&self, resource: &ResourceRef) -> bool {
        self.bookmarks()
            .iter()
            .any(|bookmark| &bookmark.resource == resource)
    }

    pub fn toggle_bookmark(
        &mut self,
        resource: ResourceRef,
    ) -> Result<BookmarkToggleResult, String> {
        let Some(cluster_prefs) = self.cluster_prefs_mut() else {
            return Err(
                "Current kube context is unavailable; cannot persist cluster bookmarks."
                    .to_string(),
            );
        };
        let result = toggle_bookmark(&mut cluster_prefs.bookmarks, resource)?;
        self.needs_config_save = true;
        Ok(result)
    }

    pub fn selected_bookmark_resource(&self) -> Option<ResourceRef> {
        selected_bookmark_resource(self.bookmarks(), self.selected_idx, self.search_query())
    }

    /// Toggles a column's visibility in user preferences for the current view.
    fn toggle_column_visibility(&mut self, column_id: &str) {
        let view_key = crate::columns::view_key(self.view);
        // Verify the column exists and is hideable
        if let Some(registry) = crate::columns::columns_for_view(self.view) {
            let Some(col) = registry.iter().find(|c| c.id == column_id) else {
                return;
            };
            if !col.hideable {
                return;
            }
        } else {
            return;
        }

        let vp = self.view_prefs_mut(view_key);
        if let Some(pos) = vp.hidden_columns.iter().position(|c| c == column_id) {
            vp.hidden_columns.remove(pos);
        } else {
            vp.hidden_columns.push(column_id.to_string());
        }
        self.needs_config_save = true;

        // Refresh column info in the palette so checkboxes update
        self.refresh_palette_columns();
    }

    /// Populates the command palette with current view's column info.
    pub fn refresh_palette_columns(&mut self) {
        if let Some(registry) = crate::columns::columns_for_view(self.view) {
            let prefs = crate::preferences::resolve_view_preferences(
                crate::columns::view_key(self.view),
                &self.preferences,
                &self.cluster_preferences,
                self.current_context_name.as_deref(),
            );
            let info: Vec<(String, String, bool)> = registry
                .iter()
                .filter(|c| c.hideable)
                .map(|c| {
                    let visible =
                        c.default_visible && !prefs.hidden_columns.iter().any(|h| h == c.id);
                    (c.id.to_string(), c.label.to_string(), visible)
                })
                .collect();
            self.command_palette.set_columns_info(Some(info));
        } else {
            self.command_palette.set_columns_info(None);
        }
    }

    /// Applies persisted sort preferences for the given view key.
    pub fn apply_sort_from_preferences(&mut self, view_key: &str) {
        let prefs = crate::preferences::resolve_view_preferences(
            view_key,
            &self.preferences,
            &self.cluster_preferences,
            self.current_context_name.as_deref(),
        );
        let Some(col_id) = &prefs.sort_column else {
            return;
        };
        let descending = !prefs.sort_ascending;

        match view_key {
            "pods" => {
                let column = match col_id.as_str() {
                    "name" => PodSortColumn::Name,
                    "age" => PodSortColumn::Age,
                    "status" => PodSortColumn::Status,
                    "restarts" => PodSortColumn::Restarts,
                    _ => return,
                };
                self.pod_sort = Some(PodSortState::new(column, descending));
            }
            _ => {
                let column = match col_id.as_str() {
                    "name" => WorkloadSortColumn::Name,
                    "age" => WorkloadSortColumn::Age,
                    _ => return,
                };
                self.workload_sort = Some(WorkloadSortState::new(column, descending));
            }
        }
    }

    /// Saves the current sort state for the given view key into preferences.
    pub fn save_sort_to_preferences(&mut self, view_key: &str) {
        let (sort_column, sort_ascending) = match view_key {
            "pods" => match self.pod_sort {
                Some(s) => (
                    Some(match s.column {
                        PodSortColumn::Name => "name",
                        PodSortColumn::Age => "age",
                        PodSortColumn::Status => "status",
                        PodSortColumn::Restarts => "restarts",
                    }),
                    !s.descending,
                ),
                None => (None, true),
            },
            _ => match self.workload_sort {
                Some(s) => (
                    Some(match s.column {
                        WorkloadSortColumn::Name => "name",
                        WorkloadSortColumn::Age => "age",
                    }),
                    !s.descending,
                ),
                None => (None, true),
            },
        };

        if let Some(col) = sort_column {
            let vp = self.view_prefs_mut(view_key);
            vp.sort_column = Some(col.to_string());
            vp.sort_ascending = sort_ascending;
        } else {
            // Clear sort at the most-specific level only
            let cleared_cluster = if let Some(ctx) = &self.current_context_name
                && let Some(clusters) = &mut self.cluster_preferences
                && let Some(cluster) = clusters.get_mut(ctx)
                && let Some(vp) = cluster.views.get_mut(view_key)
            {
                vp.sort_column = None;
                true
            } else {
                false
            };
            if !cleared_cluster
                && let Some(global) = &mut self.preferences
                && let Some(vp) = global.views.get_mut(view_key)
            {
                vp.sort_column = None;
            }
        }
        self.needs_config_save = true;
    }

    /// Moves the sidebar cursor down one row, wrapping from the last row back to the first.
    /// Only called when [`Focus::Sidebar`] is active and `j`/`↓` is pressed.
    pub fn sidebar_cursor_down(&mut self) {
        let rows = sidebar_rows(&self.collapsed_groups);
        if rows.is_empty() {
            return;
        }
        self.sidebar_cursor = (self.sidebar_cursor + 1) % rows.len();
    }

    /// Moves the sidebar cursor up one row, wrapping from the first row back to the last.
    /// Only called when [`Focus::Sidebar`] is active and `k`/`↑` is pressed.
    pub fn sidebar_cursor_up(&mut self) {
        let rows = sidebar_rows(&self.collapsed_groups);
        if rows.is_empty() {
            return;
        }
        self.sidebar_cursor = if self.sidebar_cursor == 0 {
            rows.len() - 1
        } else {
            self.sidebar_cursor - 1
        };
    }

    /// Activates the currently highlighted sidebar row.
    ///
    /// - [`SidebarItem::Group`] → emits [`AppAction::ToggleNavGroup`] to collapse/expand it.
    /// - [`SidebarItem::View`] → switches `view`, resets `selected_idx` to `0`, and sets
    ///   [`Focus::Content`] so subsequent `j`/`k` scroll the resource list.
    ///
    /// Called from `main.rs` when `Enter` is pressed while [`Focus::Sidebar`] is active.
    pub fn sidebar_activate(&mut self) -> AppAction {
        let rows = sidebar_rows(&self.collapsed_groups);
        match rows.get(self.sidebar_cursor) {
            Some(SidebarItem::Group(g)) => AppAction::ToggleNavGroup(*g),
            Some(SidebarItem::View(v)) => {
                self.focus = Focus::Content;
                AppAction::NavigateTo(*v)
            }
            None => AppAction::None,
        }
    }

    /// Keeps `sidebar_cursor` pointing at the active view row after external navigation.
    ///
    /// Called after `Tab`/`Shift+Tab` view cycling so the sidebar highlight stays in sync
    /// with the active view even when the user didn't navigate via the sidebar cursor.
    /// No-op if the current view is not visible (e.g. its group is collapsed).
    pub fn sync_sidebar_cursor_to_view(&mut self) {
        let rows = sidebar_rows(&self.collapsed_groups);
        if let Some(idx) = rows.iter().position(|r| *r == SidebarItem::View(self.view)) {
            self.sidebar_cursor = idx;
        }
    }

    /// Collapses all sidebar groups except the one containing the active view.
    /// This keeps the sidebar compact so all groups remain visible.
    pub fn sync_collapsed_to_active_view(&mut self) {
        let active_group = sidebar::group_for_view(self.view);
        self.collapsed_groups.clear();
        for group in sidebar::all_groups() {
            if Some(group) != active_group {
                self.collapsed_groups.insert(group);
            }
        }
        self.sync_sidebar_cursor_to_view();
    }

    /// Toggles a nav group collapsed/expanded and clamps the cursor.
    pub fn toggle_nav_group(&mut self, group: NavGroup) {
        if self.collapsed_groups.contains(&group) {
            self.collapsed_groups.remove(&group);
        } else {
            self.collapsed_groups.insert(group);
        }
        let rows = sidebar_rows(&self.collapsed_groups);
        self.sidebar_cursor = self.sidebar_cursor.min(rows.len().saturating_sub(1));
        self.needs_config_save = true;
    }

    /// Returns which detail sub-component is currently active.
    pub fn active_component(&self) -> ActiveComponent {
        if let Some(tab) = self.workbench.active_tab() {
            match tab.state {
                WorkbenchTabState::PodLogs(_) if self.focus == Focus::Workbench => {
                    return ActiveComponent::LogsViewer;
                }
                WorkbenchTabState::PortForward(_) if self.focus == Focus::Workbench => {
                    return ActiveComponent::PortForward;
                }
                _ => {}
            }
        }

        let Some(detail) = &self.detail_view else {
            return ActiveComponent::None;
        };

        if detail.scale_dialog.is_some() {
            ActiveComponent::Scale
        } else if detail.probe_panel.is_some() {
            ActiveComponent::ProbePanel
        } else {
            ActiveComponent::None
        }
    }

    /// Compatibility helper for tests and callers that previously opened the
    /// in-detail logs overlay. This now opens the canonical workbench tab.
    pub fn open_logs_viewer(&mut self) {
        if let Some(detail) = &self.detail_view
            && let Some(resource) = detail.selected_logs_resource()
        {
            match resource {
                pod @ ResourceRef::Pod(_, _) => self.open_pod_logs_tab(pod),
                workload => self.open_workload_logs_tab(workload, 0),
            }
        }
    }

    pub fn close_logs_viewer(&mut self) {
        if matches!(
            self.workbench.active_tab().map(|tab| &tab.state),
            Some(WorkbenchTabState::PodLogs(_))
        ) {
            self.workbench_close_active_tab();
        }
        self.blur_workbench();
    }

    /// Compatibility helper for tests and callers that previously opened the
    /// in-detail port-forward overlay. This now opens the canonical workbench tab.
    pub fn open_port_forward(&mut self) {
        if let Some(detail) = &self.detail_view
            && let Some(ResourceRef::Pod(name, namespace)) = detail.resource.as_ref()
        {
            self.open_port_forward_tab(
                Some(ResourceRef::Pod(name.clone(), namespace.clone())),
                PortForwardDialog::with_target(namespace, name, 0),
            );
        }
    }

    pub fn close_port_forward(&mut self) {
        if matches!(
            self.workbench.active_tab().map(|tab| &tab.state),
            Some(WorkbenchTabState::PortForward(_))
        ) {
            self.workbench_close_active_tab();
        }
        self.blur_workbench();
    }

    pub fn focus_workbench(&mut self) {
        if self.workbench.open && !self.workbench.tabs.is_empty() {
            self.focus = Focus::Workbench;
        }
    }

    pub fn blur_workbench(&mut self) {
        if self.focus == Focus::Workbench {
            self.focus = Focus::Content;
        }
    }

    pub fn open_resource_yaml_tab(
        &mut self,
        resource: ResourceRef,
        yaml: Option<String>,
        error: Option<String>,
        pending_request_id: Option<u64>,
    ) {
        let mut tab = ResourceYamlTabState::new(resource);
        tab.yaml = yaml;
        tab.loading = tab.yaml.is_none() && error.is_none();
        tab.error = error;
        tab.pending_request_id = pending_request_id;
        self.workbench
            .open_tab(WorkbenchTabState::ResourceYaml(tab));
        self.focus = Focus::Workbench;
    }

    pub fn open_decoded_secret_tab(
        &mut self,
        resource: ResourceRef,
        source_yaml: Option<String>,
        error: Option<String>,
        pending_request_id: Option<u64>,
    ) {
        let mut tab = DecodedSecretTabState::new(resource);
        tab.source_yaml = source_yaml;
        tab.loading = tab.source_yaml.is_none() && error.is_none();
        tab.error = error;
        tab.pending_request_id = pending_request_id;
        self.workbench
            .open_tab(WorkbenchTabState::DecodedSecret(tab));
        self.focus = Focus::Workbench;
    }

    pub fn open_resource_events_tab(
        &mut self,
        resource: ResourceRef,
        events: Vec<EventInfo>,
        loading: bool,
        error: Option<String>,
        pending_request_id: Option<u64>,
    ) {
        let mut tab = ResourceEventsTabState::new(resource);
        tab.events = events;
        tab.loading = loading;
        tab.error = error;
        tab.pending_request_id = pending_request_id;
        tab.rebuild_timeline(&self.action_history);
        self.workbench
            .open_tab(WorkbenchTabState::ResourceEvents(tab));
        self.focus = Focus::Workbench;
    }

    pub fn open_pod_logs_tab(&mut self, resource: ResourceRef) {
        self.workbench
            .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(resource)));
        self.focus = Focus::Workbench;
    }

    pub fn open_workload_logs_tab(&mut self, resource: ResourceRef, session_id: u64) {
        self.workbench
            .open_tab(WorkbenchTabState::WorkloadLogs(WorkloadLogsTabState::new(
                resource, session_id,
            )));
        self.focus = Focus::Workbench;
    }

    pub fn open_exec_tab(
        &mut self,
        resource: ResourceRef,
        session_id: u64,
        pod_name: String,
        namespace: String,
    ) {
        self.workbench
            .open_tab(WorkbenchTabState::Exec(ExecTabState::new(
                resource, session_id, pod_name, namespace,
            )));
        self.focus = Focus::Workbench;
    }

    pub fn open_port_forward_tab(
        &mut self,
        target: Option<ResourceRef>,
        dialog: PortForwardDialog,
    ) {
        self.workbench
            .open_tab(WorkbenchTabState::PortForward(PortForwardTabState::new(
                target, dialog,
            )));
        self.focus = Focus::Workbench;
    }

    /// Convenience initializer used by tests and non-runtime callers.
    /// The runtime path in `main.rs` overrides this with snapshot-derived replicas.
    pub fn open_scale_dialog(&mut self) {
        if let Some(detail) = &mut self.detail_view
            && detail.supports_action(DetailAction::Scale)
        {
            let (target_kind, name, namespace, current_replicas) = detail
                .resource
                .as_ref()
                .and_then(|r| match r {
                    ResourceRef::Deployment(name, ns) => {
                        Some((ScaleTargetKind::Deployment, name.clone(), ns.clone(), 1i32))
                    }
                    ResourceRef::StatefulSet(name, ns) => {
                        Some((ScaleTargetKind::StatefulSet, name.clone(), ns.clone(), 1i32))
                    }
                    _ => None,
                })
                .unwrap_or((
                    ScaleTargetKind::Deployment,
                    String::new(),
                    "default".to_string(),
                    1,
                ));
            detail.scale_dialog = Some(ScaleDialogState::new(
                target_kind,
                name,
                namespace,
                current_replicas,
            ));
        }
    }

    pub fn close_scale_dialog(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.scale_dialog = None;
        }
    }

    pub fn open_probe_panel(&mut self) {
        if let Some(detail) = &mut self.detail_view
            && detail.supports_action(DetailAction::Probes)
        {
            let (pod_name, namespace) = detail
                .resource
                .as_ref()
                .and_then(|r| match r {
                    ResourceRef::Pod(name, ns) => Some((name.clone(), ns.clone())),
                    _ => None,
                })
                .unwrap_or_default();
            detail.probe_panel = Some(ProbePanelComponentState::new(
                pod_name,
                namespace,
                Vec::new(),
            ));
        }
    }

    pub fn close_probe_panel(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.probe_panel = None;
        }
    }
}

/// Loads app state config from a given path.
pub fn load_config_from_path(path: &Path) -> AppState {
    let mut app = AppState::default();

    if let Ok(content) = fs::read_to_string(path)
        && let Ok(cfg) = serde_json::from_str::<AppConfig>(&content)
    {
        if !cfg.namespace.trim().is_empty() {
            app.set_namespace(cfg.namespace);
        }
        if let Some(theme_name) = &cfg.theme {
            let idx = match theme_name.to_lowercase().as_str() {
                "nord" => 1,
                "dracula" => 2,
                "catppuccin" | "mocha" => 3,
                "light" => 4,
                _ => 0,
            };
            crate::ui::theme::set_active_theme(idx);
        }
        app.refresh_interval_secs = cfg.refresh_interval_secs;
        app.workbench
            .set_open_and_height(cfg.workbench_open, cfg.workbench_height);
        // Collapsed groups are now auto-managed by sync_collapsed_to_active_view(),
        // so we ignore the saved config and rely on the default (all collapsed
        // except the active view's group).
        let _ = &cfg.collapsed_nav_groups;
        app.preferences = cfg.preferences;
        app.cluster_preferences = cfg.clusters;
    }

    app.current_context_name = kube::config::Kubeconfig::read()
        .ok()
        .and_then(|cfg| cfg.current_context);

    app
}

/// Saves app namespace config to a given path.
pub fn save_config_to_path(app: &AppState, path: &Path) {
    let theme_name = crate::ui::theme::active_theme().name;
    let cfg = AppConfig {
        namespace: app.current_namespace.clone(),
        theme: Some(theme_name.to_string()),
        refresh_interval_secs: app.refresh_interval_secs,
        workbench_open: app.workbench.open,
        workbench_height: app.workbench.height,
        collapsed_nav_groups: Vec::new(),
        preferences: app.preferences.clone(),
        clusters: app.cluster_preferences.clone(),
    };

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let serialized = serde_json::to_string(&cfg).unwrap_or_else(|_| "{}".to_string());
    let tmp = path.with_extension("tmp");
    if fs::write(&tmp, &serialized).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

/// Loads app config from ~/.kube/kubectui-config.json.
pub fn load_config() -> AppState {
    let path = dirs::home_dir()
        .unwrap_or_default()
        .join(".kube")
        .join("kubectui-config.json");
    load_config_from_path(&path)
}

/// Saves app config to ~/.kube/kubectui-config.json.
pub fn save_config(app: &AppState) {
    let path = dirs::home_dir()
        .unwrap_or_default()
        .join(".kube")
        .join("kubectui-config.json");
    save_config_to_path(app, &path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cronjob::CronJobHistoryEntry;
    use crate::k8s::dtos::PodInfo;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// Verifies full forward tab cycle across all views and wraps to Dashboard.
    #[test]
    fn tab_cycles_all_views_forward() {
        let mut app = AppState::default();
        let expected = [
            // Overview
            AppView::Bookmarks,
            AppView::Issues,
            AppView::Nodes,
            AppView::Namespaces,
            AppView::Events,
            // Workloads
            AppView::Pods,
            AppView::Deployments,
            AppView::StatefulSets,
            AppView::DaemonSets,
            AppView::ReplicaSets,
            AppView::ReplicationControllers,
            AppView::Jobs,
            AppView::CronJobs,
            // Network
            AppView::Services,
            AppView::Endpoints,
            AppView::Ingresses,
            AppView::IngressClasses,
            AppView::NetworkPolicies,
            AppView::PortForwarding,
            // Config
            AppView::ConfigMaps,
            AppView::Secrets,
            AppView::ResourceQuotas,
            AppView::LimitRanges,
            AppView::HPAs,
            AppView::PodDisruptionBudgets,
            AppView::PriorityClasses,
            // Storage
            AppView::PersistentVolumeClaims,
            AppView::PersistentVolumes,
            AppView::StorageClasses,
            // Helm
            AppView::HelmCharts,
            AppView::HelmReleases,
            // FluxCD
            AppView::FluxCDAlertProviders,
            AppView::FluxCDAlerts,
            AppView::FluxCDAll,
            AppView::FluxCDArtifacts,
            AppView::FluxCDHelmReleases,
            AppView::FluxCDHelmRepositories,
            AppView::FluxCDImages,
            AppView::FluxCDKustomizations,
            AppView::FluxCDReceivers,
            AppView::FluxCDSources,
            // Access Control
            AppView::ServiceAccounts,
            AppView::ClusterRoles,
            AppView::Roles,
            AppView::ClusterRoleBindings,
            AppView::RoleBindings,
            // Custom Resources
            AppView::Extensions,
            // Wraps back to start
            AppView::Dashboard,
        ];
        for view in expected {
            app.handle_key_event(KeyEvent::from(KeyCode::Tab));
            assert_eq!(app.view(), view);
        }
    }

    /// Verifies reverse tab cycle wraps from Dashboard to Extensions.
    #[test]
    fn shift_tab_cycles_reverse() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::BackTab));
        assert_eq!(app.view(), AppView::Extensions);
    }

    /// Verifies entering search mode and adding/removing characters.
    #[test]
    fn search_query_add_backspace_and_clear() {
        let mut app = AppState::default();

        app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
        app.handle_key_event(KeyEvent::from(KeyCode::Char('a')));
        app.handle_key_event(KeyEvent::from(KeyCode::Char('b')));
        app.handle_key_event(KeyEvent::from(KeyCode::Backspace));

        assert_eq!(app.search_query(), "a");

        app.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(app.search_query(), "");
    }

    /// Verifies pressing Esc in search mode exits mode and clears query.
    #[test]
    fn search_mode_esc_clears_and_exits() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
        app.handle_key_event(KeyEvent::from(KeyCode::Char('x')));

        app.handle_key_event(KeyEvent::from(KeyCode::Esc));

        assert_eq!(app.search_query(), "");
        assert!(!app.is_search_mode());
    }

    /// Verifies refresh actions are emitted for `r` and Ctrl+R.
    #[test]
    fn refresh_action_transitions() {
        let mut app = AppState::default();
        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('r'))),
            AppAction::RefreshData
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::CONTROL)),
            AppAction::RefreshData
        );
    }

    #[test]
    fn flux_view_uppercase_r_triggers_reconcile_without_overriding_ctrl_r() {
        let mut app = AppState::default();
        app.view = AppView::FluxCDKustomizations;

        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('R'))),
            AppAction::FluxReconcile
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::CONTROL)),
            AppAction::RefreshData
        );
    }

    #[test]
    fn flux_detail_uppercase_r_triggers_reconcile_for_supported_resource() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::CustomResource {
                name: "apps".to_string(),
                namespace: Some("flux-system".to_string()),
                group: "kustomize.toolkit.fluxcd.io".to_string(),
                version: "v1".to_string(),
                kind: "Kustomization".to_string(),
                plural: "kustomizations".to_string(),
            }),
            ..DetailViewState::default()
        });

        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('R'))),
            AppAction::FluxReconcile
        );
    }

    #[test]
    fn unsupported_flux_detail_uppercase_r_is_noop() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::CustomResource {
                name: "webhook".to_string(),
                namespace: Some("flux-system".to_string()),
                group: "notification.toolkit.fluxcd.io".to_string(),
                version: "v1beta3".to_string(),
                kind: "Alert".to_string(),
                plural: "alerts".to_string(),
            }),
            ..DetailViewState::default()
        });

        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('R'))),
            AppAction::None
        );
    }

    /// Verifies namespace can be switched through dedicated mutators.
    #[test]
    fn test_appstate_namespace_switching() {
        let mut app = AppState::default();
        assert_eq!(app.get_namespace(), "all");

        app.set_namespace("kube-system".to_string());
        assert_eq!(app.get_namespace(), "kube-system");
    }

    /// Verifies namespace picker shortcut emits open action.
    #[test]
    fn tilde_opens_namespace_picker_action() {
        let mut app = AppState::default();
        let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('~')));
        assert_eq!(action, AppAction::OpenNamespacePicker);
    }

    #[test]
    fn pods_sort_keybindings_toggle_and_clear() {
        let mut app = AppState::default();
        app.view = AppView::Pods;
        app.focus = Focus::Content;

        assert_eq!(app.pod_sort(), None);

        app.handle_key_event(KeyEvent::from(KeyCode::Char('1')));
        assert_eq!(
            app.pod_sort(),
            Some(PodSortState::new(PodSortColumn::Age, true))
        );

        app.handle_key_event(KeyEvent::from(KeyCode::Char('1')));
        assert_eq!(
            app.pod_sort(),
            Some(PodSortState::new(PodSortColumn::Age, false))
        );

        app.handle_key_event(KeyEvent::from(KeyCode::Char('3')));
        assert_eq!(
            app.pod_sort(),
            Some(PodSortState::new(PodSortColumn::Restarts, true))
        );

        app.handle_key_event(KeyEvent::from(KeyCode::Char('0')));
        assert_eq!(app.pod_sort(), None);
    }

    #[test]
    fn pods_name_sort_shortcut_toggles() {
        let mut app = AppState::default();
        app.view = AppView::Pods;
        app.focus = Focus::Content;

        app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
        assert_eq!(
            app.pod_sort(),
            Some(PodSortState::new(PodSortColumn::Name, false))
        );

        app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
        assert_eq!(
            app.pod_sort(),
            Some(PodSortState::new(PodSortColumn::Name, true))
        );
    }

    #[test]
    fn pods_sort_keybindings_are_scoped_to_pods_view() {
        let mut app = AppState::default();
        app.view = AppView::Services;
        app.focus = Focus::Content;

        app.handle_key_event(KeyEvent::from(KeyCode::Char('1')));
        assert_eq!(app.pod_sort(), None);
    }

    #[test]
    fn workload_sort_keybindings_toggle_and_clear() {
        let mut app = AppState::default();
        app.view = AppView::Deployments;
        app.focus = Focus::Content;

        assert_eq!(app.workload_sort(), None);

        app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
        assert_eq!(
            app.workload_sort(),
            Some(WorkloadSortState::new(WorkloadSortColumn::Name, false))
        );

        app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
        assert_eq!(
            app.workload_sort(),
            Some(WorkloadSortState::new(WorkloadSortColumn::Name, true))
        );

        app.handle_key_event(KeyEvent::from(KeyCode::Char('a')));
        assert_eq!(
            app.workload_sort(),
            Some(WorkloadSortState::new(WorkloadSortColumn::Age, true))
        );

        app.handle_key_event(KeyEvent::from(KeyCode::Char('0')));
        assert_eq!(app.workload_sort(), None);
    }

    #[test]
    fn workload_sort_keybindings_are_scoped_to_workload_views() {
        let mut app = AppState::default();
        app.view = AppView::ConfigMaps;
        app.focus = Focus::Content;

        app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
        assert_eq!(app.workload_sort(), None);
    }

    #[test]
    fn workbench_keybindings_emit_expected_actions() {
        use crate::workbench::{ActionHistoryTabState, WorkbenchTabState};

        let mut app = AppState::default();

        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('b'))),
            AppAction::ToggleWorkbench
        );

        // Add a tab (background so open stays false), then toggle open
        app.workbench
            .ensure_background_tab(WorkbenchTabState::ActionHistory(
                ActionHistoryTabState::default(),
            ));
        app.toggle_workbench();
        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char(']'))),
            AppAction::WorkbenchNextTab
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('['))),
            AppAction::WorkbenchPreviousTab
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)),
            AppAction::WorkbenchCloseActiveTab
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL)),
            AppAction::WorkbenchIncreaseHeight
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL)),
            AppAction::WorkbenchDecreaseHeight
        );
    }

    #[test]
    fn workbench_b_key_toggles_from_workbench_focus() {
        use crate::workbench::{ActionHistoryTabState, WorkbenchTabState};

        let mut app = AppState::default();
        app.workbench
            .ensure_background_tab(WorkbenchTabState::ActionHistory(
                ActionHistoryTabState::default(),
            ));
        app.toggle_workbench();
        app.focus = Focus::Workbench;
        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('b'))),
            AppAction::ToggleWorkbench
        );
    }

    #[test]
    fn search_esc_resets_selected_idx() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));
        app.handle_key_event(KeyEvent::from(KeyCode::Char('x')));
        app.selected_idx = 5;
        app.handle_key_event(KeyEvent::from(KeyCode::Esc));
        assert_eq!(app.selected_idx, 0);
        assert!(app.search_query().is_empty());
    }

    #[test]
    fn delete_confirm_accepts_lowercase_d() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Pod("pod-0".into(), "default".into())),
            yaml: Some("kind: Pod".into()),
            confirm_delete: true,
            ..DetailViewState::default()
        });
        let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('d')));
        assert_eq!(action, AppAction::DeleteResource);
    }

    #[test]
    fn sync_workbench_focus_resets_when_tabs_empty() {
        let mut app = AppState::default();
        app.focus = Focus::Workbench;
        app.sync_workbench_focus();
        assert_eq!(app.focus, Focus::Content);
    }

    #[test]
    fn sync_workbench_focus_preserves_when_tabs_exist() {
        use crate::workbench::{ActionHistoryTabState, WorkbenchTabState};
        let mut app = AppState::default();
        app.workbench.open_tab(WorkbenchTabState::ActionHistory(
            ActionHistoryTabState::default(),
        ));
        app.focus = Focus::Workbench;
        app.sync_workbench_focus();
        assert_eq!(app.focus, Focus::Workbench);
    }

    #[test]
    fn pod_logs_search_mode_accepts_shortcut_characters_as_text() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Pod("pod-1".into(), "default".into())),
            ..DetailViewState::default()
        });
        app.open_logs_viewer();

        let Some(tab) = app.workbench.active_tab_mut() else {
            panic!("expected active workbench tab");
        };
        let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state else {
            panic!("expected pod logs tab");
        };
        logs_tab.viewer.searching = true;
        logs_tab.viewer.search_input.clear();

        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('g'))),
            AppAction::None
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('f'))),
            AppAction::None
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::from(KeyCode::Char('t'))),
            AppAction::None
        );

        let Some(tab) = app.workbench.active_tab() else {
            panic!("expected active workbench tab");
        };
        let WorkbenchTabState::PodLogs(logs_tab) = &tab.state else {
            panic!("expected pod logs tab");
        };
        assert_eq!(logs_tab.viewer.search_input, "gft");
    }

    #[test]
    fn workload_logs_filter_mode_supports_ctrl_u_clear() {
        let mut app = AppState::default();
        app.workbench
            .open_tab(WorkbenchTabState::WorkloadLogs(WorkloadLogsTabState::new(
                ResourceRef::Pod("pod-1".into(), "default".into()),
                1,
            )));
        app.focus_workbench();

        let Some(tab) = app.workbench.active_tab_mut() else {
            panic!("expected active workbench tab");
        };
        let WorkbenchTabState::WorkloadLogs(logs_tab) = &mut tab.state else {
            panic!("expected workload logs tab");
        };
        logs_tab.editing_text_filter = true;
        logs_tab.filter_input = "error".into();

        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)),
            AppAction::None
        );

        let Some(tab) = app.workbench.active_tab() else {
            panic!("expected active workbench tab");
        };
        let WorkbenchTabState::WorkloadLogs(logs_tab) = &tab.state else {
            panic!("expected workload logs tab");
        };
        assert!(logs_tab.filter_input.is_empty());
    }

    #[test]
    fn filtered_pod_indices_apply_restarts_sort_with_stable_tie_breakers() {
        let mut pods = vec![
            PodInfo {
                name: "zeta".to_string(),
                namespace: "prod".to_string(),
                status: "Running".to_string(),
                restarts: 2,
                ..PodInfo::default()
            },
            PodInfo {
                name: "alpha".to_string(),
                namespace: "dev".to_string(),
                status: "Pending".to_string(),
                restarts: 2,
                ..PodInfo::default()
            },
            PodInfo {
                name: "beta".to_string(),
                namespace: "prod".to_string(),
                status: "Running".to_string(),
                restarts: 5,
                ..PodInfo::default()
            },
        ];
        // Ensure deterministic age field ordering is not involved in this test.
        for pod in &mut pods {
            pod.created_at = None;
        }

        let sorted = filtered_pod_indices(
            &pods,
            "",
            Some(PodSortState::new(PodSortColumn::Restarts, true)),
        );

        // Highest restarts first, then namespace/name tie-breakers for equal restart count.
        assert_eq!(sorted, vec![2, 1, 0]);
    }

    #[test]
    fn filtered_workload_indices_apply_age_sort_with_name_tie_breaker() {
        #[derive(Clone)]
        struct Item {
            name: String,
            namespace: String,
            age: Option<std::time::Duration>,
        }

        let items = vec![
            Item {
                name: "zeta".to_string(),
                namespace: "prod".to_string(),
                age: Some(std::time::Duration::from_secs(60)),
            },
            Item {
                name: "alpha".to_string(),
                namespace: "dev".to_string(),
                age: Some(std::time::Duration::from_secs(60)),
            },
            Item {
                name: "beta".to_string(),
                namespace: "prod".to_string(),
                age: Some(std::time::Duration::from_secs(120)),
            },
        ];

        let sorted = filtered_workload_indices(
            &items,
            "",
            Some(WorkloadSortState::new(WorkloadSortColumn::Age, true)),
            |item, _| !item.name.is_empty(),
            |item| item.name.as_str(),
            |item| item.namespace.as_str(),
            |item| item.age,
        );

        assert_eq!(sorted, vec![2, 1, 0]);
    }

    /// Verifies namespace persistence round-trip via config helpers.
    #[test]
    fn test_namespace_persistence() {
        use crate::workbench::{ActionHistoryTabState, WorkbenchTabState};

        let path =
            std::env::temp_dir().join(format!("kubectui-config-test-{}.json", std::process::id()));

        let mut app = AppState::default();
        app.set_namespace("demo".to_string());
        app.workbench
            .ensure_background_tab(WorkbenchTabState::ActionHistory(
                ActionHistoryTabState::default(),
            ));
        app.toggle_workbench();
        app.workbench.height = 15;
        save_config_to_path(&app, &path);

        let loaded = load_config_from_path(&path);
        assert_eq!(loaded.get_namespace(), "demo");
        assert!(loaded.workbench.open);
        assert_eq!(loaded.workbench.height, 15);

        let _ = std::fs::remove_file(path);
    }

    /// Verifies quit requires confirmation: first q sets confirm_quit, second q quits.
    #[test]
    fn quit_action_sets_should_quit() {
        let mut app = AppState::default();

        let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('q')));
        assert_eq!(action, AppAction::None);
        assert!(app.confirm_quit);
        assert!(!app.should_quit());

        let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('q')));
        assert_eq!(action, AppAction::Quit);
        assert!(app.should_quit());
    }

    /// Verifies any other key cancels the quit confirmation.
    #[test]
    fn quit_confirm_cancelled_by_other_key() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Char('q')));
        assert!(app.confirm_quit);

        app.handle_key_event(KeyEvent::from(KeyCode::Char('n')));
        assert!(!app.confirm_quit);
        assert!(!app.should_quit());
    }

    /// Verifies Esc closes detail view before requesting app quit.
    #[test]
    fn esc_closes_detail_before_quit() {
        let mut app = AppState {
            detail_view: Some(DetailViewState::default()),
            ..AppState::default()
        };

        let action = app.handle_key_event(KeyEvent::from(KeyCode::Esc));

        assert_eq!(action, AppAction::CloseDetail);
        assert!(!app.should_quit());
    }

    /// Verifies selection index saturates at zero when moving up.
    #[test]
    fn selected_index_never_underflows() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Up));
        assert_eq!(app.selected_idx(), 0);
    }

    /// Verifies j/k move the sidebar cursor (not selected_idx) when no detail view.
    #[test]
    fn selected_index_grows_with_down_events() {
        let mut app = AppState::default();
        for _ in 0..5 {
            app.handle_key_event(KeyEvent::from(KeyCode::Down));
        }
        assert_eq!(app.sidebar_cursor, 5);
    }

    /// Verifies selection resets to zero when switching tabs.
    #[test]
    fn view_switch_resets_selection_index() {
        let mut app = AppState::default();
        app.selected_idx = 2;
        assert_eq!(app.selected_idx(), 2);

        app.handle_key_event(KeyEvent::from(KeyCode::Tab));

        assert_eq!(app.selected_idx(), 0);
    }

    /// Verifies rapid tab switching remains stable.
    #[test]
    fn rapid_tab_switching_is_stable() {
        let mut app = AppState::default();

        for _ in 0..(AppView::tabs().len() * 3) {
            app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        }

        assert_eq!(app.view(), AppView::Dashboard);
    }

    /// Verifies search input ignores Ctrl-modified characters except supported shortcuts.
    #[test]
    fn search_input_ignores_ctrl_characters() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Char('/')));

        app.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));

        assert_eq!(app.search_query(), "");
    }

    /// Verifies error message can be set and cleared.
    #[test]
    fn error_message_set_and_clear() {
        let mut app = AppState::default();
        app.set_error("boom".to_string());
        assert_eq!(app.error_message(), Some("boom"));

        app.clear_error();
        assert_eq!(app.error_message(), None);
    }

    #[test]
    fn status_message_set_and_clear() {
        let mut app = AppState::default();
        app.set_status("working".to_string());
        assert_eq!(app.status_message(), Some("working"));
        assert_eq!(app.error_message(), None);

        app.clear_status();
        assert_eq!(app.status_message(), None);
    }

    /// Verifies resource reference helper methods return expected kind/name/namespace.
    #[test]
    fn resource_ref_helpers_work_for_each_variant() {
        let node = ResourceRef::Node("n1".to_string());
        let pod = ResourceRef::Pod("p1".to_string(), "ns1".to_string());
        let statefulset = ResourceRef::StatefulSet("ss1".to_string(), "ns1".to_string());
        let quota = ResourceRef::ResourceQuota("rq1".to_string(), "ns1".to_string());
        let daemonset = ResourceRef::DaemonSet("ds1".to_string(), "ns1".to_string());
        let pv = ResourceRef::Pv("pv1".to_string());
        let cluster_role = ResourceRef::ClusterRole("cr1".to_string());

        assert_eq!(node.kind(), "Node");
        assert_eq!(node.name(), "n1");
        assert_eq!(node.namespace(), None);

        assert_eq!(pod.kind(), "Pod");
        assert_eq!(pod.name(), "p1");
        assert_eq!(pod.namespace(), Some("ns1"));

        assert_eq!(statefulset.kind(), "StatefulSet");
        assert_eq!(statefulset.name(), "ss1");
        assert_eq!(statefulset.namespace(), Some("ns1"));

        assert_eq!(quota.kind(), "ResourceQuota");
        assert_eq!(quota.name(), "rq1");
        assert_eq!(quota.namespace(), Some("ns1"));

        assert_eq!(daemonset.kind(), "DaemonSet");
        assert_eq!(daemonset.name(), "ds1");
        assert_eq!(daemonset.namespace(), Some("ns1"));

        assert_eq!(pv.kind(), "PersistentVolume");
        assert_eq!(pv.name(), "pv1");
        assert_eq!(pv.namespace(), None);

        assert_eq!(cluster_role.kind(), "ClusterRole");
        assert_eq!(cluster_role.name(), "cr1");
        assert_eq!(cluster_role.namespace(), None);

        let helm = ResourceRef::HelmRelease("my-release".to_string(), "default".to_string());
        assert_eq!(helm.kind(), "HelmRelease");
        assert_eq!(helm.name(), "my-release");
        assert_eq!(helm.namespace(), Some("default"));

        let cr = ResourceRef::CustomResource {
            name: "my-widget".to_string(),
            namespace: Some("prod".to_string()),
            group: "demo.io".to_string(),
            version: "v1".to_string(),
            kind: "Widget".to_string(),
            plural: "widgets".to_string(),
        };
        assert_eq!(cr.kind(), "Widget");
        assert_eq!(cr.name(), "my-widget");
        assert_eq!(cr.namespace(), Some("prod"));

        let cr_cluster = ResourceRef::CustomResource {
            name: "global".to_string(),
            namespace: None,
            group: "infra.io".to_string(),
            version: "v1beta1".to_string(),
            kind: "ClusterWidget".to_string(),
            plural: "clusterwidgets".to_string(),
        };
        assert_eq!(cr_cluster.kind(), "ClusterWidget");
        assert_eq!(cr_cluster.name(), "global");
        assert_eq!(cr_cluster.namespace(), None);
    }

    #[test]
    fn ctrl_y_returns_copy_resource_name() {
        let mut app = AppState::default();
        app.view = AppView::Pods;
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        assert_eq!(action, AppAction::CopyResourceName);
    }

    #[test]
    fn shift_y_returns_copy_full_name() {
        let mut app = AppState::default();
        app.view = AppView::Pods;
        app.focus = Focus::Content;
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT));
        assert_eq!(action, AppAction::CopyResourceFullName);
    }

    #[test]
    fn c_key_returns_cordon_in_node_detail() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            yaml: Some("kind: Node".to_string()),
            ..DetailViewState::default()
        });
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert_eq!(action, AppAction::CordonNode);
    }

    #[test]
    fn u_key_returns_uncordon_in_node_detail() {
        let mut app = AppState::default();
        let mut detail = DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            yaml: Some("kind: Node".to_string()),
            ..DetailViewState::default()
        };
        detail.metadata.node_unschedulable = Some(true);
        app.detail_view = Some(detail);
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
        assert_eq!(action, AppAction::UncordonNode);
    }

    #[test]
    fn d_key_opens_drain_confirmation_in_node_detail() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            yaml: Some("kind: Node".to_string()),
            ..DetailViewState::default()
        });
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT));
        assert_eq!(action, AppAction::None);
        assert!(app.detail_view.as_ref().unwrap().confirm_drain);
    }

    #[test]
    fn drain_confirm_d_returns_drain_node() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            yaml: Some("kind: Node".to_string()),
            confirm_drain: true,
            ..DetailViewState::default()
        });
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT));
        assert_eq!(action, AppAction::DrainNode);
    }

    #[test]
    fn drain_confirm_f_returns_force_drain() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            yaml: Some("kind: Node".to_string()),
            confirm_drain: true,
            ..DetailViewState::default()
        });
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT));
        assert_eq!(action, AppAction::ForceDrainNode);
    }

    #[test]
    fn drain_confirm_esc_cancels() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            yaml: Some("kind: Node".to_string()),
            confirm_drain: true,
            ..DetailViewState::default()
        });
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(action, AppAction::None);
        assert!(!app.detail_view.as_ref().unwrap().confirm_drain);
    }

    #[test]
    fn cronjob_detail_jk_and_enter_follow_selected_job() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::CronJob(
                "nightly".to_string(),
                "ops".to_string(),
            )),
            yaml: Some("kind: CronJob".to_string()),
            cronjob_history: vec![
                CronJobHistoryEntry {
                    job_name: "nightly-001".to_string(),
                    namespace: "ops".to_string(),
                    status: "Succeeded".to_string(),
                    completions: "1/1".to_string(),
                    duration: Some("8s".to_string()),
                    pod_count: 1,
                    live_pod_count: 0,
                    completion_pct: Some(100),
                    active_pods: 0,
                    failed_pods: 0,
                    age: None,
                    created_at: None,
                    logs_authorized: None,
                },
                CronJobHistoryEntry {
                    job_name: "nightly-002".to_string(),
                    namespace: "ops".to_string(),
                    status: "Failed".to_string(),
                    completions: "0/1".to_string(),
                    duration: Some("3s".to_string()),
                    pod_count: 1,
                    live_pod_count: 1,
                    completion_pct: Some(0),
                    active_pods: 0,
                    failed_pods: 1,
                    age: None,
                    created_at: None,
                    logs_authorized: None,
                },
            ],
            ..DetailViewState::default()
        });

        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
            AppAction::None
        );
        assert_eq!(
            app.detail_view.as_ref().unwrap().cronjob_history_selected,
            1
        );
        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            AppAction::OpenDetail(ResourceRef::Job(
                "nightly-002".to_string(),
                "ops".to_string(),
            ))
        );
    }

    #[test]
    fn cronjob_detail_shift_s_opens_suspend_confirmation() {
        let mut app = AppState::default();
        let mut detail = DetailViewState {
            resource: Some(ResourceRef::CronJob(
                "nightly".to_string(),
                "ops".to_string(),
            )),
            yaml: Some("kind: CronJob".to_string()),
            ..DetailViewState::default()
        };
        detail.metadata.cronjob_suspended = Some(false);
        app.detail_view = Some(detail);

        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT)),
            AppAction::ConfirmCronJobSuspend(true)
        );
    }

    #[test]
    fn cronjob_suspend_confirm_enter_dispatches_target_state() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::CronJob(
                "nightly".to_string(),
                "ops".to_string(),
            )),
            yaml: Some("kind: CronJob".to_string()),
            confirm_cronjob_suspend: Some(false),
            ..DetailViewState::default()
        });

        assert_eq!(
            app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            AppAction::SetCronJobSuspend(false)
        );
    }

    #[test]
    fn c_key_does_not_cordon_for_pod_detail() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
            yaml: Some("kind: Pod".to_string()),
            ..DetailViewState::default()
        });
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert_ne!(action, AppAction::CordonNode);
    }

    #[test]
    fn d_key_does_not_drain_for_pod_detail() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
            yaml: Some("kind: Pod".to_string()),
            ..DetailViewState::default()
        });
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT));
        assert_ne!(action, AppAction::DrainNode);
        assert!(!app.detail_view.as_ref().unwrap().confirm_drain);
    }

    #[test]
    fn u_key_does_not_uncordon_for_pod_detail() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Pod("pod-0".to_string(), "ns".to_string())),
            yaml: Some("kind: Pod".to_string()),
            ..DetailViewState::default()
        });
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
        assert_ne!(action, AppAction::UncordonNode);
    }

    #[test]
    fn y_key_in_drain_confirm_dispatches_drain_not_yaml() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            yaml: Some("kind: Node".to_string()),
            confirm_drain: true,
            ..DetailViewState::default()
        });
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(action, AppAction::DrainNode);
    }

    #[test]
    fn palette_blocked_during_drain_confirm() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            yaml: Some("kind: Node".to_string()),
            confirm_drain: true,
            ..DetailViewState::default()
        });
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE));
        assert_ne!(action, AppAction::OpenCommandPalette);
    }

    #[test]
    fn y_key_blocked_during_drain_confirm_does_not_open_yaml() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Node("node-0".to_string())),
            yaml: Some("kind: Node".to_string()),
            confirm_drain: true,
            ..DetailViewState::default()
        });
        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_ne!(action, AppAction::OpenResourceYaml);
    }

    #[test]
    fn o_key_opens_decoded_secret_in_secret_detail() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Secret(
                "app-secret".to_string(),
                "default".to_string(),
            )),
            yaml: Some("kind: Secret".to_string()),
            ..DetailViewState::default()
        });

        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
        assert_eq!(action, AppAction::OpenDecodedSecret);
    }

    #[test]
    fn o_key_opens_decoded_secret_from_secrets_list() {
        let mut app = AppState::default();
        app.view = AppView::Secrets;
        app.focus = Focus::Content;

        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
        assert_eq!(action, AppAction::OpenDecodedSecret);
    }

    #[test]
    fn o_key_does_not_open_decoded_secret_for_non_secret_detail() {
        let mut app = AppState::default();
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::ConfigMap(
                "app-config".to_string(),
                "default".to_string(),
            )),
            yaml: Some("kind: ConfigMap".to_string()),
            ..DetailViewState::default()
        });

        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
        assert_eq!(action, AppAction::None);
    }

    #[test]
    fn uppercase_b_toggles_bookmark_for_selected_resource() {
        let mut app = AppState::default();
        app.view = AppView::Pods;
        app.focus = Focus::Content;

        let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('B'), KeyModifiers::SHIFT));
        assert_eq!(action, AppAction::ToggleBookmark);
    }

    #[test]
    fn toggle_bookmark_persists_per_current_context() {
        let mut app = AppState::default();
        app.current_context_name = Some("prod".to_string());

        let result = app
            .toggle_bookmark(ResourceRef::Secret(
                "app-secret".to_string(),
                "default".to_string(),
            ))
            .expect("bookmark added");
        assert_eq!(result, BookmarkToggleResult::Added);
        assert_eq!(app.bookmark_count(), 1);
        assert!(app.is_bookmarked(&ResourceRef::Secret(
            "app-secret".to_string(),
            "default".to_string(),
        )));
        assert!(app.needs_config_save);
    }

    #[test]
    fn apply_sort_from_preferences_pods() {
        use crate::preferences::{UserPreferences, ViewPreferences};
        let mut app = AppState::default();
        let mut global = UserPreferences::default();
        global.views.insert(
            "pods".into(),
            ViewPreferences {
                sort_column: Some("restarts".into()),
                sort_ascending: false,
                ..Default::default()
            },
        );
        app.preferences = Some(global);
        app.apply_sort_from_preferences("pods");
        let sort = app.pod_sort.unwrap();
        assert_eq!(sort.column, PodSortColumn::Restarts);
        assert!(sort.descending);
    }

    #[test]
    fn apply_sort_from_preferences_workload() {
        use crate::preferences::{UserPreferences, ViewPreferences};
        let mut app = AppState::default();
        let mut global = UserPreferences::default();
        global.views.insert(
            "deployments".into(),
            ViewPreferences {
                sort_column: Some("age".into()),
                sort_ascending: true,
                ..Default::default()
            },
        );
        app.preferences = Some(global);
        app.apply_sort_from_preferences("deployments");
        let sort = app.workload_sort.unwrap();
        assert_eq!(sort.column, WorkloadSortColumn::Age);
        assert!(!sort.descending);
    }

    #[test]
    fn apply_sort_invalid_column_ignored() {
        use crate::preferences::{UserPreferences, ViewPreferences};
        let mut app = AppState::default();
        let mut global = UserPreferences::default();
        global.views.insert(
            "pods".into(),
            ViewPreferences {
                sort_column: Some("nonexistent".into()),
                ..Default::default()
            },
        );
        app.preferences = Some(global);
        app.apply_sort_from_preferences("pods");
        assert!(app.pod_sort.is_none());
    }

    #[test]
    fn save_sort_to_preferences_round_trip() {
        let mut app = AppState::default();
        app.pod_sort = Some(PodSortState::new(PodSortColumn::Status, false));
        app.save_sort_to_preferences("pods");
        let prefs = app.preferences.as_ref().unwrap();
        let vp = prefs.views.get("pods").unwrap();
        assert_eq!(vp.sort_column.as_deref(), Some("status"));
        assert!(vp.sort_ascending); // descending=false → ascending=true
        assert!(app.needs_config_save);
    }

    #[test]
    fn clear_sort_removes_from_preferences() {
        use crate::preferences::{UserPreferences, ViewPreferences};
        let mut app = AppState::default();
        let mut global = UserPreferences::default();
        global.views.insert(
            "pods".into(),
            ViewPreferences {
                sort_column: Some("age".into()),
                ..Default::default()
            },
        );
        app.preferences = Some(global);
        app.pod_sort = None;
        app.save_sort_to_preferences("pods");
        let vp = app.preferences.as_ref().unwrap().views.get("pods").unwrap();
        assert!(vp.sort_column.is_none());
    }

    #[test]
    fn config_round_trip_with_preferences() {
        use crate::preferences::{ClusterPreferences, UserPreferences, ViewPreferences};
        let path = std::env::temp_dir().join("kubectui_test_config_prefs.json");

        let mut app = AppState::default();
        let mut global = UserPreferences::default();
        global.views.insert(
            "pods".into(),
            ViewPreferences {
                sort_column: Some("restarts".into()),
                sort_ascending: false,
                hidden_columns: vec!["namespace".into()],
                ..Default::default()
            },
        );
        app.preferences = Some(global);

        let mut cluster_prefs = ClusterPreferences::default();
        cluster_prefs.views.insert(
            "pods".into(),
            ViewPreferences {
                sort_column: Some("status".into()),
                ..Default::default()
            },
        );
        cluster_prefs.bookmarks.push(BookmarkEntry {
            resource: ResourceRef::Secret("app-secret".to_string(), "default".to_string()),
            bookmarked_at_unix: 42,
        });
        let mut clusters = HashMap::new();
        clusters.insert("prod".into(), cluster_prefs);
        app.cluster_preferences = Some(clusters);

        app.collapsed_groups.insert(NavGroup::FluxCD);
        app.collapsed_groups.insert(NavGroup::AccessControl);

        save_config_to_path(&app, &path);
        let loaded = load_config_from_path(&path);

        let prefs = loaded.preferences.as_ref().unwrap();
        let pod_prefs = prefs.views.get("pods").unwrap();
        assert_eq!(pod_prefs.sort_column.as_deref(), Some("restarts"));
        assert!(!pod_prefs.sort_ascending);
        assert_eq!(pod_prefs.hidden_columns, vec!["namespace"]);

        let clusters = loaded.cluster_preferences.as_ref().unwrap();
        let prod = clusters.get("prod").unwrap();
        let prod_pods = prod.views.get("pods").unwrap();
        assert_eq!(prod_pods.sort_column.as_deref(), Some("status"));
        assert_eq!(prod.bookmarks.len(), 1);
        assert_eq!(prod.bookmarks[0].bookmarked_at_unix, 42);

        // Collapsed groups are now auto-managed — saved config is ignored.
        // Default state: all groups collapsed except the active view's group (Overview).
        assert!(!loaded.collapsed_groups.contains(&NavGroup::Overview));
        assert!(loaded.collapsed_groups.contains(&NavGroup::FluxCD));
        assert!(loaded.collapsed_groups.contains(&NavGroup::AccessControl));
    }

    #[test]
    fn config_backward_compat_no_prefs() {
        let path = std::env::temp_dir().join("kubectui_test_config_compat.json");
        std::fs::write(
            &path,
            r#"{"namespace":"default","workbench_open":true,"workbench_height":14}"#,
        )
        .unwrap();
        let loaded = load_config_from_path(&path);
        assert!(loaded.preferences.is_none());
        assert!(loaded.cluster_preferences.is_none());
        // All groups collapsed except Overview (default view's group).
        assert!(!loaded.collapsed_groups.contains(&NavGroup::Overview));
        assert!(loaded.collapsed_groups.contains(&NavGroup::Workloads));
    }

    #[test]
    fn sidebar_icons_do_not_use_replacement_glyphs() {
        assert!(!NavGroup::Config.icon().contains('\u{fffd}'));
        assert!(!NavGroup::Config.sidebar_text(false).contains('\u{fffd}'));
        assert!(!AppView::Endpoints.icon().contains('\u{fffd}'));
        assert!(!AppView::Endpoints.sidebar_text().contains('\u{fffd}'));
    }
}
