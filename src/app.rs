//! Application state machine and keyboard input handling.

use std::{fs, path::Path};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

use crate::{
    k8s::{
        client::EventInfo,
        dtos::{CustomResourceInfo, NodeMetricsInfo, PodMetricsInfo},
    },
    ui::components::{CommandPalette, CommandPaletteAction, ContextPicker, ContextPickerAction, NamespacePicker, NamespacePickerAction},
};

/// Sidebar navigation groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavGroup {
    Overview,
    Workloads,
    Networking,
    Security,
    Governance,
    Extensions,
}

impl NavGroup {
    pub const fn label(self) -> &'static str {
        match self {
            NavGroup::Overview => "Overview",
            NavGroup::Workloads => "Workloads",
            NavGroup::Networking => "Networking",
            NavGroup::Security => "Security",
            NavGroup::Governance => "Governance",
            NavGroup::Extensions => "Extensions",
        }
    }

    pub const fn icon(self) -> &'static str {
        match self {
            NavGroup::Overview => "󰋗",
            NavGroup::Workloads => "󰆧",
            NavGroup::Networking => "󰛳",
            NavGroup::Security => "󰒃",
            NavGroup::Governance => "󰒓",
            NavGroup::Extensions => "󰏗",
        }
    }
}

/// Top-level views displayed by KubecTUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppView {
    Dashboard,
    Nodes,
    Pods,
    Services,
    Deployments,
    StatefulSets,
    DaemonSets,
    Jobs,
    CronJobs,
    ServiceAccounts,
    Roles,
    RoleBindings,
    ClusterRoles,
    ClusterRoleBindings,
    ResourceQuotas,
    LimitRanges,
    PodDisruptionBudgets,
    Extensions,
}

impl AppView {
    const ORDER: [AppView; 18] = [
        AppView::Dashboard,
        AppView::Nodes,
        AppView::Pods,
        AppView::Services,
        AppView::Deployments,
        AppView::StatefulSets,
        AppView::DaemonSets,
        AppView::Jobs,
        AppView::CronJobs,
        AppView::ServiceAccounts,
        AppView::Roles,
        AppView::RoleBindings,
        AppView::ClusterRoles,
        AppView::ClusterRoleBindings,
        AppView::ResourceQuotas,
        AppView::LimitRanges,
        AppView::PodDisruptionBudgets,
        AppView::Extensions,
    ];

    /// Returns a static display label for this view.
    pub const fn label(self) -> &'static str {
        match self {
            AppView::Dashboard => "Dashboard",
            AppView::Nodes => "Nodes",
            AppView::Pods => "Pods",
            AppView::Services => "Services",
            AppView::Deployments => "Deployments",
            AppView::StatefulSets => "StatefulSets",
            AppView::DaemonSets => "DaemonSets",
            AppView::Jobs => "Jobs",
            AppView::CronJobs => "CronJobs",
            AppView::ServiceAccounts => "ServiceAccounts",
            AppView::Roles => "Roles",
            AppView::RoleBindings => "RoleBindings",
            AppView::ClusterRoles => "ClusterRoles",
            AppView::ClusterRoleBindings => "ClusterRoleBindings",
            AppView::ResourceQuotas => "ResourceQuotas",
            AppView::LimitRanges => "LimitRanges",
            AppView::PodDisruptionBudgets => "PodDisruptionBudgets",
            AppView::Extensions => "Extensions",
        }
    }

    /// Returns the sidebar icon for this view.
    pub const fn icon(self) -> &'static str {
        match self {
            AppView::Dashboard => "󰋗",
            AppView::Nodes => "󰒋",
            AppView::Pods => "󰠳",
            AppView::Services => "󰛳",
            AppView::Deployments => "󰆧",
            AppView::StatefulSets => "󰆼",
            AppView::DaemonSets => "󰒓",
            AppView::Jobs => "󰃰",
            AppView::CronJobs => "󰔠",
            AppView::ServiceAccounts => "󰀄",
            AppView::Roles => "󰒃",
            AppView::RoleBindings => "󰌋",
            AppView::ClusterRoles => "󰒃",
            AppView::ClusterRoleBindings => "󰌋",
            AppView::ResourceQuotas => "󰏗",
            AppView::LimitRanges => "󰳗",
            AppView::PodDisruptionBudgets => "󰦕",
            AppView::Extensions => "󰏗",
        }
    }

    /// Returns the NavGroup this view belongs to.
    pub const fn group(self) -> NavGroup {
        match self {
            AppView::Dashboard | AppView::Nodes => NavGroup::Overview,
            AppView::Pods
            | AppView::Deployments
            | AppView::StatefulSets
            | AppView::DaemonSets
            | AppView::Jobs
            | AppView::CronJobs => NavGroup::Workloads,
            AppView::Services => NavGroup::Networking,
            AppView::ServiceAccounts
            | AppView::Roles
            | AppView::RoleBindings
            | AppView::ClusterRoles
            | AppView::ClusterRoleBindings => NavGroup::Security,
            AppView::ResourceQuotas
            | AppView::LimitRanges
            | AppView::PodDisruptionBudgets => NavGroup::Governance,
            AppView::Extensions => NavGroup::Extensions,
        }
    }

    fn index(self) -> usize {
        Self::ORDER
            .iter()
            .position(|view| *view == self)
            .expect("AppView::ORDER must contain all enum variants")
    }

    fn from_index(index: usize) -> Self {
        Self::ORDER[index % Self::ORDER.len()]
    }

    fn next(self) -> Self {
        Self::from_index(self.index() + 1)
    }

    fn previous(self) -> Self {
        let current = self.index();
        let next_idx = if current == 0 {
            Self::ORDER.len() - 1
        } else {
            current - 1
        };
        Self::from_index(next_idx)
    }

    /// Enumerates all available top-level tabs in stable order.
    pub const fn tabs() -> &'static [AppView; 18] {
        &Self::ORDER
    }
}

/// Logical pointer to a resource selected in the current view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceRef {
    Node(String),
    Pod(String, String),
    Service(String, String),
    Deployment(String, String),
    StatefulSet(String, String),
    ResourceQuota(String, String),
    LimitRange(String, String),
    PodDisruptionBudget(String, String),
}

impl ResourceRef {
    /// Returns resource kind label used by UI and fetch routing.
    pub fn kind(&self) -> &'static str {
        match self {
            ResourceRef::Node(_) => "Node",
            ResourceRef::Pod(_, _) => "Pod",
            ResourceRef::Service(_, _) => "Service",
            ResourceRef::Deployment(_, _) => "Deployment",
            ResourceRef::StatefulSet(_, _) => "StatefulSet",
            ResourceRef::ResourceQuota(_, _) => "ResourceQuota",
            ResourceRef::LimitRange(_, _) => "LimitRange",
            ResourceRef::PodDisruptionBudget(_, _) => "PodDisruptionBudget",
        }
    }

    /// Returns resource name.
    pub fn name(&self) -> &str {
        match self {
            ResourceRef::Node(name)
            | ResourceRef::Pod(name, _)
            | ResourceRef::Service(name, _)
            | ResourceRef::Deployment(name, _)
            | ResourceRef::StatefulSet(name, _)
            | ResourceRef::ResourceQuota(name, _)
            | ResourceRef::LimitRange(name, _)
            | ResourceRef::PodDisruptionBudget(name, _) => name,
        }
    }

    /// Returns namespace when this is a namespaced resource.
    pub fn namespace(&self) -> Option<&str> {
        match self {
            ResourceRef::Node(_) => None,
            ResourceRef::Pod(_, ns)
            | ResourceRef::Service(_, ns)
            | ResourceRef::Deployment(_, ns)
            | ResourceRef::StatefulSet(_, ns)
            | ResourceRef::ResourceQuota(_, ns)
            | ResourceRef::LimitRange(_, ns)
            | ResourceRef::PodDisruptionBudget(_, ns) => Some(ns),
        }
    }
}

/// Human-readable metadata displayed in the detail modal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetailMetadata {
    pub name: String,
    pub namespace: Option<String>,
    pub status: Option<String>,
    pub node: Option<String>,
    pub ip: Option<String>,
    pub created: Option<String>,
    pub labels: Vec<(String, String)>,
}

/// Top-level active component when detail modal is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveComponent {
    None,
    LogsViewer,
    PortForward,
    Scale,
    ProbePanel,
}

/// In-detail logs viewer state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogsViewerState {
    pub scroll_offset: usize,
    pub follow_mode: bool,
    pub lines: Vec<String>,
    pub pod_name: String,
    pub pod_namespace: String,
    pub loading: bool,
    pub error: Option<String>,
}

/// Active form field in the lightweight port-forward dialog state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortForwardField {
    LocalPort,
    RemotePort,
    TunnelList,
}

/// In-detail port-forward dialog state used by keyboard routing tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortForwardDialogState {
    pub active_field: PortForwardField,
    pub local_port: String,
    pub remote_port: String,
}

impl Default for PortForwardDialogState {
    fn default() -> Self {
        Self {
            active_field: PortForwardField::LocalPort,
            local_port: String::new(),
            remote_port: String::new(),
        }
    }
}

/// In-detail scale dialog state used by keyboard routing tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaleDialogInputState {
    pub replica_input: String,
    pub target_replicas: i32,
}

impl Default for ScaleDialogInputState {
    fn default() -> Self {
        Self {
            replica_input: String::new(),
            target_replicas: 0,
        }
    }
}

/// In-detail probe panel state used by keyboard routing tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProbePanelState {
    pub probes: Vec<String>,
    pub expanded: Vec<bool>,
    pub selected_idx: usize,
}

/// Detail modal state for the currently focused resource.
#[derive(Debug, Clone, Default)]
pub struct DetailViewState {
    pub resource: Option<ResourceRef>,
    pub metadata: DetailMetadata,
    pub yaml: Option<String>,
    pub events: Vec<EventInfo>,
    pub sections: Vec<String>,
    pub pod_metrics: Option<PodMetricsInfo>,
    pub node_metrics: Option<NodeMetricsInfo>,
    pub metrics_unavailable_message: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub logs_viewer: Option<LogsViewerState>,
    pub port_forward_dialog: Option<PortForwardDialogState>,
    pub scale_dialog: Option<ScaleDialogInputState>,
    pub probe_panel: Option<ProbePanelState>,
}

/// Actions emitted by input handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    None,
    RefreshData,
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
    EscapePressed,
    LogsViewerOpen,
    LogsViewerClose,
    LogsViewerScrollUp,
    LogsViewerScrollDown,
    LogsViewerScrollTop,
    LogsViewerScrollBottom,
    LogsViewerToggleFollow,
    PortForwardOpen,
    PortForwardClose,
    PortForwardNextField,
    PortForwardPrevField,
    PortForwardUpdateLocalPort(String),
    PortForwardUpdateRemotePort(String),
    PortForwardBackspace,
    ScaleDialogOpen,
    ScaleDialogClose,
    ScaleDialogUpdateInput(char),
    ScaleDialogBackspace,
    ProbePanelOpen,
    ProbePanelClose,
    ProbeToggleExpand(usize),
    ProbeSelectNext,
    ProbeSelectPrev,
}

/// Runtime state for UI interaction and navigation.
#[derive(Debug, Clone)]
pub struct AppState {
    pub view: AppView,
    pub selected_idx: usize,
    pub search_query: String,
    pub is_search_mode: bool,
    pub should_quit: bool,
    pub error_message: Option<String>,
    pub detail_view: Option<DetailViewState>,
    pub current_namespace: String,
    pub namespace_picker: NamespacePicker,
    pub context_picker: ContextPicker,
    pub command_palette: CommandPalette,
    pub extension_instances: Vec<CustomResourceInfo>,
    pub extension_error: Option<String>,
    pub extension_selected_crd: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            view: AppView::Dashboard,
            selected_idx: 0,
            search_query: String::new(),
            is_search_mode: false,
            should_quit: false,
            error_message: None,
            detail_view: None,
            current_namespace: "default".to_string(),
            namespace_picker: NamespacePicker::new(vec!["all".to_string(), "default".to_string()]),
            context_picker: ContextPicker::default(),
            command_palette: CommandPalette::default(),
            extension_instances: Vec::new(),
            extension_error: None,
            extension_selected_crd: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    namespace: String,
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

    /// Returns the active search query.
    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    /// Returns whether the app is currently in search input mode.
    pub fn is_search_mode(&self) -> bool {
        self.is_search_mode
    }

    /// Returns whether the event loop should terminate.
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Returns the latest UI-level error, if any.
    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    /// Sets an error message to be shown in the status bar.
    pub fn set_error(&mut self, message: String) {
        self.error_message = Some(message);
    }

    /// Clears any active error message.
    pub fn clear_error(&mut self) {
        self.error_message = None;
    }

    /// Sets active namespace for namespaced resource fetches.
    pub fn set_namespace(&mut self, ns: String) {
        self.current_namespace = ns;
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
    pub fn set_extension_instances(
        &mut self,
        crd_name: String,
        instances: Vec<CustomResourceInfo>,
        error: Option<String>,
    ) {
        self.extension_selected_crd = Some(crd_name);
        self.extension_instances = instances;
        self.extension_error = error;
    }

    fn next_view(&mut self) {
        self.view = self.view.next();
        self.selected_idx = 0;
    }

    fn previous_view(&mut self) {
        self.view = self.view.previous();
        self.selected_idx = 0;
    }

    fn select_next(&mut self) {
        self.selected_idx = self.selected_idx.saturating_add(1);
    }

    fn select_previous(&mut self) {
        self.selected_idx = self.selected_idx.saturating_sub(1);
    }

    /// Returns which detail sub-component is currently active.
    pub fn active_component(&self) -> ActiveComponent {
        let Some(detail) = &self.detail_view else {
            return ActiveComponent::None;
        };

        if detail.logs_viewer.is_some() {
            ActiveComponent::LogsViewer
        } else if detail.port_forward_dialog.is_some() {
            ActiveComponent::PortForward
        } else if detail.scale_dialog.is_some() {
            ActiveComponent::Scale
        } else if detail.probe_panel.is_some() {
            ActiveComponent::ProbePanel
        } else {
            ActiveComponent::None
        }
    }

    pub fn open_logs_viewer(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.logs_viewer = Some(LogsViewerState::default());
        }
    }

    pub fn close_logs_viewer(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.logs_viewer = None;
        }
    }

    pub fn open_port_forward(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.port_forward_dialog = Some(PortForwardDialogState::default());
        }
    }

    pub fn close_port_forward(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.port_forward_dialog = None;
        }
    }

    pub fn open_scale_dialog(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.scale_dialog = Some(ScaleDialogInputState::default());
        }
    }

    pub fn close_scale_dialog(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.scale_dialog = None;
        }
    }

    pub fn open_probe_panel(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.probe_panel = Some(ProbePanelState::default());
        }
    }

    pub fn close_probe_panel(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.probe_panel = None;
        }
    }

    /// Handles a keyboard event and updates app state.
    pub fn handle_key_event(&mut self, key: KeyEvent) -> AppAction {
        if self.command_palette.is_open() {
            return match self.command_palette.handle_key(key) {
                CommandPaletteAction::None => AppAction::None,
                CommandPaletteAction::Navigate(view) => AppAction::NavigateTo(view),
                CommandPaletteAction::Close => AppAction::CloseCommandPalette,
            };
        }

        if self.context_picker.is_open() {
            return match self.context_picker.handle_key(key) {
                ContextPickerAction::None => AppAction::None,
                ContextPickerAction::Select(ctx) => AppAction::SelectContext(ctx),
                ContextPickerAction::Close => AppAction::CloseContextPicker,
            };
        }

        if self.namespace_picker.is_open() {
            return match self.namespace_picker.handle_key(key) {
                NamespacePickerAction::None => AppAction::None,
                NamespacePickerAction::Select(ns) => AppAction::SelectNamespace(ns),
                NamespacePickerAction::Close => AppAction::CloseNamespacePicker,
            };
        }

        if self.is_search_mode {
            return self.handle_search_input(key);
        }

        // Component-level routing priority:
        // LogsViewer > PortForward > Scale > ProbePanel > DetailView > MainView
        match self.active_component() {
            ActiveComponent::LogsViewer => {
                return match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Char('k') | KeyCode::Up => AppAction::LogsViewerScrollUp,
                    KeyCode::Char('j') | KeyCode::Down => AppAction::LogsViewerScrollDown,
                    KeyCode::Char('g') => AppAction::LogsViewerScrollTop,
                    KeyCode::Char('G') => AppAction::LogsViewerScrollBottom,
                    KeyCode::Char('f') => AppAction::LogsViewerToggleFollow,
                    _ => AppAction::None,
                };
            }
            ActiveComponent::PortForward => {
                return match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Tab => AppAction::PortForwardNextField,
                    KeyCode::BackTab => AppAction::PortForwardPrevField,
                    KeyCode::Backspace => AppAction::PortForwardBackspace,
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        let field = self
                            .detail_view
                            .as_ref()
                            .and_then(|d| d.port_forward_dialog.as_ref())
                            .map(|pf| pf.active_field)
                            .unwrap_or(PortForwardField::LocalPort);

                        match field {
                            PortForwardField::LocalPort => {
                                AppAction::PortForwardUpdateLocalPort(c.to_string())
                            }
                            PortForwardField::RemotePort => {
                                AppAction::PortForwardUpdateRemotePort(c.to_string())
                            }
                            PortForwardField::TunnelList => AppAction::None,
                        }
                    }
                    _ => AppAction::None,
                };
            }
            ActiveComponent::Scale => {
                return match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Backspace => AppAction::ScaleDialogBackspace,
                    KeyCode::Char(c) if c.is_ascii_digit() => AppAction::ScaleDialogUpdateInput(c),
                    _ => AppAction::None,
                };
            }
            ActiveComponent::ProbePanel => {
                return match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Char(' ') => {
                        let idx = self
                            .detail_view
                            .as_ref()
                            .and_then(|d| d.probe_panel.as_ref())
                            .map(|p| p.selected_idx)
                            .unwrap_or(0);
                        AppAction::ProbeToggleExpand(idx)
                    }
                    KeyCode::Char('j') | KeyCode::Down => AppAction::ProbeSelectNext,
                    KeyCode::Char('k') | KeyCode::Up => AppAction::ProbeSelectPrev,
                    _ => AppAction::None,
                };
            }
            ActiveComponent::None => {}
        }

        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                AppAction::Quit
            }
            KeyCode::Esc if self.detail_view.is_some() => AppAction::CloseDetail,
            KeyCode::Esc => {
                self.should_quit = true;
                AppAction::Quit
            }
            KeyCode::Char('l') | KeyCode::Char('L') if self.detail_view.is_some() => {
                AppAction::LogsViewerOpen
            }
            KeyCode::Char('f') if self.detail_view.is_some() => AppAction::PortForwardOpen,
            KeyCode::Char('s') if self.detail_view.is_some() => AppAction::ScaleDialogOpen,
            KeyCode::Tab => {
                self.next_view();
                AppAction::None
            }
            KeyCode::BackTab => {
                self.previous_view();
                AppAction::None
            }
            KeyCode::Down => {
                self.select_next();
                AppAction::None
            }
            KeyCode::Up => {
                self.select_previous();
                AppAction::None
            }
            KeyCode::Char('/') => {
                self.is_search_mode = true;
                AppAction::None
            }
            KeyCode::Char('~') => AppAction::OpenNamespacePicker,
            KeyCode::Char('c') if self.detail_view.is_none() => AppAction::OpenContextPicker,
            KeyCode::Char(':') if self.detail_view.is_none() => AppAction::OpenCommandPalette,
            KeyCode::Char('r') => AppAction::RefreshData,
            KeyCode::Char('R') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                AppAction::RefreshData
            }
            _ => AppAction::None,
        }
    }

    fn handle_search_input(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.search_query.clear();
                self.is_search_mode = false;
            }
            KeyCode::Enter => {
                self.is_search_mode = false;
            }
            KeyCode::Backspace => {
                self.search_query.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_query.clear();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_query.push(c);
            }
            _ => {}
        }
        AppAction::None
    }
}

/// Loads app state config from a given path.
pub fn load_config_from_path(path: &Path) -> AppState {
    let mut app = AppState::default();

    if let Ok(content) = fs::read_to_string(path)
        && let Ok(cfg) = serde_json::from_str::<AppConfig>(&content)
        && !cfg.namespace.trim().is_empty()
    {
        app.set_namespace(cfg.namespace);
    }

    app
}

/// Saves app namespace config to a given path.
pub fn save_config_to_path(app: &AppState, path: &Path) {
    let cfg = AppConfig {
        namespace: app.current_namespace.clone(),
    };

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let serialized = serde_json::to_string(&cfg).unwrap_or_else(|_| "{}".to_string());
    let _ = fs::write(path, serialized);
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

    /// Verifies full forward tab cycle across all views and wraps to Dashboard.
    #[test]
    fn tab_cycles_all_views_forward() {
        let mut app = AppState::default();

        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::Nodes);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::Pods);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::Services);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::Deployments);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::StatefulSets);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::DaemonSets);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::Jobs);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::CronJobs);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::ServiceAccounts);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::Roles);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::RoleBindings);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::ClusterRoles);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::ClusterRoleBindings);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::ResourceQuotas);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::LimitRanges);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::PodDisruptionBudgets);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::Extensions);
        app.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.view(), AppView::Dashboard);
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

    /// Verifies namespace can be switched through dedicated mutators.
    #[test]
    fn test_appstate_namespace_switching() {
        let mut app = AppState::default();
        assert_eq!(app.get_namespace(), "default");

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

    /// Verifies namespace persistence round-trip via config helpers.
    #[test]
    fn test_namespace_persistence() {
        let path =
            std::env::temp_dir().join(format!("kubectui-config-test-{}.json", std::process::id()));

        let mut app = AppState::default();
        app.set_namespace("demo".to_string());
        save_config_to_path(&app, &path);

        let loaded = load_config_from_path(&path);
        assert_eq!(loaded.get_namespace(), "demo");

        let _ = std::fs::remove_file(path);
    }

    /// Verifies quit action and should_quit state transition.
    #[test]
    fn quit_action_sets_should_quit() {
        let mut app = AppState::default();

        let action = app.handle_key_event(KeyEvent::from(KeyCode::Char('q')));

        assert_eq!(action, AppAction::Quit);
        assert!(app.should_quit());
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

    /// Verifies selection can grow with repeated down events.
    #[test]
    fn selected_index_grows_with_down_events() {
        let mut app = AppState::default();
        for _ in 0..5 {
            app.handle_key_event(KeyEvent::from(KeyCode::Down));
        }
        assert_eq!(app.selected_idx(), 5);
    }

    /// Verifies selection resets to zero when switching tabs.
    #[test]
    fn view_switch_resets_selection_index() {
        let mut app = AppState::default();
        app.handle_key_event(KeyEvent::from(KeyCode::Down));
        app.handle_key_event(KeyEvent::from(KeyCode::Down));
        assert_eq!(app.selected_idx(), 2);

        app.handle_key_event(KeyEvent::from(KeyCode::Tab));

        assert_eq!(app.selected_idx(), 0);
    }

    /// Verifies rapid tab switching remains stable.
    #[test]
    fn rapid_tab_switching_is_stable() {
        let mut app = AppState::default();

        for _ in 0..108 {
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

    /// Verifies resource reference helper methods return expected kind/name/namespace.
    #[test]
    fn resource_ref_helpers_work_for_each_variant() {
        let node = ResourceRef::Node("n1".to_string());
        let pod = ResourceRef::Pod("p1".to_string(), "ns1".to_string());
        let statefulset = ResourceRef::StatefulSet("ss1".to_string(), "ns1".to_string());
        let quota = ResourceRef::ResourceQuota("rq1".to_string(), "ns1".to_string());

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
    }
}
