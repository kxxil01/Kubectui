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
        self.sync_sidebar_expansion_to_cursor();
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
        self.sync_sidebar_expansion_to_cursor();
    }

    fn sync_sidebar_expansion_to_cursor(&mut self) {
        let rows = sidebar_rows(&self.collapsed_groups);
        let current_item = rows.get(self.sidebar_cursor).copied();
        let expanded_group = current_item.and_then(Self::sidebar_item_group);
        self.set_expanded_group(expanded_group, current_item, true);
    }

    fn sidebar_item_group(item: SidebarItem) -> Option<NavGroup> {
        match item {
            SidebarItem::Group(group) => Some(group),
            SidebarItem::View(view) => sidebar::group_for_view(view),
        }
    }

    fn normalize_expanded_group(expanded_group: Option<NavGroup>) -> Option<NavGroup> {
        match expanded_group {
            Some(NavGroup::Overview) | None => None,
            Some(group) => Some(group),
        }
    }

    fn set_expanded_group(
        &mut self,
        expanded_group: Option<NavGroup>,
        preserve_item: Option<SidebarItem>,
        mark_dirty: bool,
    ) {
        let expanded_group = Self::normalize_expanded_group(expanded_group);
        let collapsed_groups: HashSet<_> = sidebar::all_groups()
            .filter(|group| *group != NavGroup::Overview && Some(*group) != expanded_group)
            .collect();

        if self.collapsed_groups != collapsed_groups {
            self.collapsed_groups = collapsed_groups;
            if mark_dirty {
                self.needs_config_save = true;
            }
        }

        let rows = sidebar_rows(&self.collapsed_groups);
        if let Some(item) = preserve_item
            && let Some(idx) = rows.iter().position(|row| *row == item)
        {
            self.sidebar_cursor = idx;
            return;
        }

        if let Some(group) = preserve_item.and_then(Self::sidebar_item_group)
            && let Some(idx) = rows
                .iter()
                .position(|row| *row == SidebarItem::Group(group))
        {
            self.sidebar_cursor = idx;
            return;
        }

        self.sidebar_cursor = self.sidebar_cursor.min(rows.len().saturating_sub(1));
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
        if let Some(idx) = rows
            .iter()
            .position(|row| *row == SidebarItem::View(self.view))
        {
            self.sidebar_cursor = idx;
            return;
        }

        if let Some(group) = sidebar::group_for_view(self.view)
            && let Some(idx) = rows
                .iter()
                .position(|row| *row == SidebarItem::Group(group))
        {
            self.sidebar_cursor = idx;
            return;
        }

        self.sidebar_cursor = self.sidebar_cursor.min(rows.len().saturating_sub(1));
    }

    pub fn sync_collapsed_to_active_view(&mut self) {
        self.set_expanded_group(
            sidebar::group_for_view(self.view),
            Some(SidebarItem::View(self.view)),
            true,
        );
    }

    pub fn toggle_nav_group(&mut self, group: NavGroup) {
        if self.collapsed_groups.contains(&group) {
            self.set_expanded_group(Some(group), Some(SidebarItem::Group(group)), true);
        } else {
            self.set_expanded_group(None, Some(SidebarItem::Group(group)), true);
        }
    }
}
