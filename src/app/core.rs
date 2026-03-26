use super::*;

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
            pending_workspace_restore: None,
            workbench: WorkbenchState::default(),
            spinner_tick: 0,
            toasts: Vec::new(),
        }
    }
}

impl AppState {
    pub fn view(&self) -> AppView {
        self.view
    }

    pub fn selected_idx(&self) -> usize {
        self.selected_idx
    }

    pub fn workload_sort_for_view(&self, view: AppView) -> Option<WorkloadSortState> {
        self.workload_sort
            .filter(|sort| view.supports_shared_sort(sort.column))
    }

    pub fn workload_sort(&self) -> Option<WorkloadSortState> {
        self.workload_sort_for_view(self.view)
    }

    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    pub fn pod_sort(&self) -> Option<PodSortState> {
        self.pod_sort
    }

    pub fn is_search_mode(&self) -> bool {
        self.is_search_mode
    }

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
        let affected_resource = self
            .action_history
            .find_by_id(entry_id)
            .and_then(|e| e.target.as_ref().map(|t| t.resource.clone()));
        self.action_history
            .complete(entry_id, status, message, keep_target);
        self.rebuild_timeline_for(affected_resource.as_ref());
    }

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

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    pub fn status_message(&self) -> Option<&str> {
        self.status_message.as_deref()
    }

    pub fn set_error(&mut self, message: String) {
        self.status_message = None;
        self.error_message = Some(message);
    }

    pub fn clear_error(&mut self) {
        self.error_message = None;
    }

    pub fn set_status(&mut self, message: String) {
        self.error_message = None;
        self.status_message = Some(message);
    }

    pub fn clear_status(&mut self) {
        self.status_message = None;
    }

    pub fn advance_spinner(&mut self) {
        self.spinner_tick = self.spinner_tick.wrapping_add(1) % 8;
    }

    pub fn spinner_char(&self) -> char {
        const FRAMES: [char; 8] = [
            '\u{280B}', '\u{2819}', '\u{2839}', '\u{2838}', '\u{283C}', '\u{2834}', '\u{2826}',
            '\u{2827}',
        ];
        FRAMES[self.spinner_tick as usize % FRAMES.len()]
    }

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
}
