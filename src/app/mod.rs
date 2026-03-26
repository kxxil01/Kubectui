//! Application state machine and keyboard input handling.

mod config_io;
mod core;
pub mod detail_state;
mod input;
mod navigation;
mod preferences;
pub mod resource_ref;
pub mod sidebar;
pub mod sort;
pub mod views;
mod workbench;

pub use config_io::{load_config, load_config_from_path, save_config, save_config_to_path};
pub use detail_state::*;
pub use resource_ref::ResourceRef;
pub use sidebar::{SidebarItem, sidebar_rows};
pub use sort::{
    PodSortColumn, PodSortState, WorkloadSortColumn, WorkloadSortState, filtered_pod_indices,
    filtered_workload_indices,
};
pub use views::{AppView, NavGroup};

use std::{collections::HashMap, collections::HashSet, time::Instant};

use crate::{
    action_history::{ActionHistoryState, ActionHistoryTarget, ActionKind, ActionStatus},
    bookmarks::{BookmarkEntry, BookmarkToggleResult, selected_bookmark_resource, toggle_bookmark},
    k8s::{client::EventInfo, dtos::CustomResourceInfo},
    preferences::{ClusterPreferences, UserPreferences},
    ui::components::{
        CommandPalette, ContextPicker, NamespacePicker, port_forward_dialog::PortForwardDialog,
        probe_panel::ProbePanelState as ProbePanelComponentState, scale_dialog::ScaleDialogState,
    },
    workbench::{
        ActionHistoryTabState, DecodedSecretTabState, ExecTabState, PodLogsTabState,
        PortForwardTabState, ResourceEventsTabState, ResourceYamlTabState, WorkbenchState,
        WorkbenchTabState, WorkloadLogsTabState,
    },
    workspaces::WorkspaceSnapshot,
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
    OpenLogTimeJump,
    ApplyLogTimeJump,
    CancelLogTimeJump,
    ToggleLogRegexMode,
    ToggleLogTimeWindow,
    CycleWorkloadLogLabelFilter,
    ToggleStructuredLogView,
    OpenResourceYaml,
    OpenResourceDiff,
    OpenRollout,
    OpenHelmHistory,
    OpenHelmValuesDiff,
    OpenDecodedSecret,
    OpenResourceEvents,
    OpenActionHistory,
    OpenExec,
    DebugContainerDialogOpen,
    DebugContainerDialogSubmit,
    NodeDebugDialogOpen,
    NodeDebugDialogSubmit,
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
    ToggleRolloutPauseResume,
    ConfirmRolloutUndo,
    ExecuteRolloutUndo,
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
    SaveLogPreset,
    ApplyPreviousLogPreset,
    ApplyNextLogPreset,
    ToggleLogCorrelation,
    SaveWorkspace,
    ApplyPreviousWorkspace,
    ApplyNextWorkspace,
    ApplyWorkspace(String),
    ActivateWorkspaceBank(String),
    OpenResourceTemplateDialog(crate::resource_templates::ResourceTemplateKind),
    SubmitResourceTemplateDialog,
    ToggleBookmark,
    SaveDecodedSecret,
    ExecuteExtension {
        id: String,
        resource: ResourceRef,
    },
    PaletteAction {
        action: crate::policy::DetailAction,
        resource: ResourceRef,
    },
    OpenRelationships,
    OpenNetworkPolicyView,
    OpenNetworkConnectivity,
    OpenTrafficDebug,
    ConfirmDrainNode,
    ConfirmHelmRollback,
    CordonNode,
    UncordonNode,
    DrainNode,
    ForceDrainNode,
    ExecuteHelmRollback,
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
    pub resource_template_dialog: Option<crate::ui::components::ResourceTemplateDialogState>,
    /// Snapshot queued for completion after an async context switch.
    pub pending_workspace_restore: Option<WorkspaceSnapshot>,
    /// Persistent bottom workbench state.
    pub workbench: WorkbenchState,
    /// Spinner animation tick counter (0–7), advanced on each UI tick during loading.
    pub spinner_tick: u8,
    /// Stack of timed toast notifications (max 3).
    pub toasts: Vec<Toast>,
}

#[cfg(test)]
mod tests;
