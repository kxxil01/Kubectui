use super::*;

impl AppState {
    pub fn set_namespace(&mut self, ns: String) {
        self.current_namespace = ns;
        self.selected_idx = 0;
        self.search_query.clear();
        self.is_search_mode = false;
    }

    pub fn get_namespace(&self) -> &str {
        &self.current_namespace
    }

    pub fn is_namespace_picker_open(&self) -> bool {
        self.namespace_picker.is_open()
    }

    pub fn is_context_picker_open(&self) -> bool {
        self.context_picker.is_open()
    }

    pub fn open_context_picker(&mut self, contexts: Vec<String>, current: Option<String>) {
        self.context_picker.set_contexts(contexts, current);
        self.context_picker.open();
    }

    pub fn close_context_picker(&mut self) {
        self.context_picker.close();
    }

    pub fn namespace_picker(&self) -> &NamespacePicker {
        &self.namespace_picker
    }

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

    pub fn open_namespace_picker(&mut self) {
        self.namespace_picker.open();
    }

    pub fn close_namespace_picker(&mut self) {
        self.namespace_picker.close();
    }

    pub fn begin_extension_instances_load(&mut self, crd_name: String) {
        self.extension_selected_crd = Some(crd_name);
        self.extension_instances.clear();
        self.extension_error = None;
        self.extension_instance_cursor = 0;
    }

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

    pub fn navigate_to_view(&mut self, view: AppView) {
        self.view = view;
        self.selected_idx = 0;
        self.search_query.clear();
        self.is_search_mode = false;
        self.sync_collapsed_to_active_view();
        self.apply_sort_from_preferences(crate::columns::view_key(self.view));
    }

    pub(super) fn next_view(&mut self) {
        self.navigate_to_view(self.view.next());
    }

    pub(super) fn previous_view(&mut self) {
        self.navigate_to_view(self.view.previous());
    }

    pub(super) fn select_next(&mut self) {
        self.selected_idx = self.selected_idx.saturating_add(1);
    }

    pub(super) fn select_previous(&mut self) {
        self.selected_idx = self.selected_idx.saturating_sub(1);
    }

    pub fn sidebar_cursor_down(&mut self) {
        let rows = sidebar_rows(&self.collapsed_groups);
        if rows.is_empty() {
            return;
        }
        self.sidebar_cursor = (self.sidebar_cursor + 1) % rows.len();
    }

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

    pub(super) fn sync_sidebar_cursor_to_view(&mut self) {
        let rows = sidebar_rows(&self.collapsed_groups);
        if let Some(idx) = rows.iter().position(|r| *r == SidebarItem::View(self.view)) {
            self.sidebar_cursor = idx;
        }
    }

    pub fn sync_collapsed_to_active_view(&mut self) {
        if let Some(active_group) = sidebar::group_for_view(self.view) {
            self.collapsed_groups.remove(&active_group);
        }
        self.sync_sidebar_cursor_to_view();
    }

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
}
