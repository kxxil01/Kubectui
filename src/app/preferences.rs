use super::*;

impl AppState {
    pub(super) fn set_or_toggle_pod_sort(&mut self, column: PodSortColumn) {
        self.selected_idx = 0;
        self.pod_sort = match self.pod_sort {
            Some(current) if current.column == column => {
                Some(PodSortState::new(column, !current.descending))
            }
            _ => Some(PodSortState::new(column, column.default_descending())),
        };
        self.save_sort_to_preferences("pods");
    }

    pub(super) fn clear_pod_sort(&mut self) {
        self.selected_idx = 0;
        self.pod_sort = None;
        self.save_sort_to_preferences("pods");
    }

    pub(super) fn set_or_toggle_workload_sort(&mut self, column: WorkloadSortColumn) {
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

    pub(super) fn clear_workload_sort(&mut self) {
        self.selected_idx = 0;
        self.workload_sort = None;
        let view_key = crate::columns::view_key(self.view);
        self.save_sort_to_preferences(view_key);
    }

    fn view_prefs_mut(&mut self, view_key: &str) -> &mut crate::preferences::ViewPreferences {
        if let Some(context) = self.current_context_name.clone() {
            let clusters = self
                .cluster_preferences
                .get_or_insert_with(Default::default);
            let cluster = clusters.entry(context).or_default();
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

    fn log_prefs_mut(&mut self) -> &mut crate::preferences::LogPresetPreferences {
        &mut self
            .preferences
            .get_or_insert_with(Default::default)
            .log_presets
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

    pub(super) fn toggle_column_visibility(&mut self, column_id: &str) {
        let view_key = crate::columns::view_key(self.view);
        let Some(registry) = crate::columns::columns_for_view(self.view) else {
            return;
        };
        let Some(col) = registry.iter().find(|c| c.id == column_id) else {
            return;
        };
        if !col.hideable {
            return;
        }

        let vp = self.view_prefs_mut(view_key);
        if col.default_visible {
            vp.shown_columns.retain(|c| c != column_id);
            if let Some(pos) = vp.hidden_columns.iter().position(|c| c == column_id) {
                vp.hidden_columns.remove(pos);
            } else {
                vp.hidden_columns.push(column_id.to_string());
            }
        } else {
            vp.hidden_columns.retain(|c| c != column_id);
            if let Some(pos) = vp.shown_columns.iter().position(|c| c == column_id) {
                vp.shown_columns.remove(pos);
            } else {
                vp.shown_columns.push(column_id.to_string());
            }
        }
        self.needs_config_save = true;
        self.refresh_palette_columns();
    }

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
                    let visible = if c.default_visible {
                        !prefs.hidden_columns.iter().any(|hidden| hidden == c.id)
                    } else {
                        prefs.shown_columns.iter().any(|shown| shown == c.id)
                    };
                    (c.id.to_string(), c.label.to_string(), visible)
                })
                .collect();
            self.command_palette.set_columns_info(Some(info));
        } else {
            self.command_palette.set_columns_info(None);
        }
    }

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

    pub fn save_active_log_preset(&mut self) -> Result<String, String> {
        enum PresetSnapshot {
            Pod(crate::log_investigation::PodLogPreset),
            Workload(crate::log_investigation::WorkloadLogPreset),
        }

        let snapshot = match self.workbench.active_tab().map(|tab| &tab.state) {
            Some(WorkbenchTabState::PodLogs(tab)) => {
                PresetSnapshot::Pod(tab.viewer.preset_snapshot())
            }
            Some(WorkbenchTabState::WorkloadLogs(tab)) => {
                PresetSnapshot::Workload(tab.preset_snapshot())
            }
            _ => return Err("Saved log presets are only available from log tabs.".to_string()),
        };

        let saved_name = match snapshot {
            PresetSnapshot::Pod(preset) => {
                let presets = &mut self.log_prefs_mut().pod_logs;
                save_named_pod_preset(presets, preset)
            }
            PresetSnapshot::Workload(preset) => {
                let presets = &mut self.log_prefs_mut().workload_logs;
                save_named_workload_preset(presets, preset)
            }
        };
        self.needs_config_save = true;
        self.set_status(format!("Saved log preset: {saved_name}"));
        Ok(saved_name)
    }

    pub fn cycle_active_log_preset(&mut self, forward: bool) -> Result<String, String> {
        enum PresetCycle {
            Pod {
                current: crate::log_investigation::PodLogPreset,
                presets: Vec<crate::log_investigation::PodLogPreset>,
            },
            Workload {
                current: crate::log_investigation::WorkloadLogPreset,
                presets: Vec<crate::log_investigation::WorkloadLogPreset>,
            },
        }

        let cycle = match self.workbench.active_tab().map(|tab| &tab.state) {
            Some(WorkbenchTabState::PodLogs(tab)) => PresetCycle::Pod {
                current: tab.viewer.preset_snapshot(),
                presets: self
                    .preferences
                    .as_ref()
                    .map(|prefs| prefs.log_presets.pod_logs.clone())
                    .unwrap_or_default(),
            },
            Some(WorkbenchTabState::WorkloadLogs(tab)) => PresetCycle::Workload {
                current: tab.preset_snapshot(),
                presets: self
                    .preferences
                    .as_ref()
                    .map(|prefs| prefs.log_presets.workload_logs.clone())
                    .unwrap_or_default(),
            },
            _ => return Err("Saved log presets are only available from log tabs.".to_string()),
        };

        match cycle {
            PresetCycle::Pod { current, presets } => {
                let preset = cycle_named_pod_preset(&presets, &current, forward)
                    .ok_or_else(|| "No saved pod log presets yet.".to_string())?;
                let Some(active_tab) = self.workbench.active_tab_mut() else {
                    return Err("No active workbench tab.".to_string());
                };
                let WorkbenchTabState::PodLogs(tab) = &mut active_tab.state else {
                    return Err("Pod log preset target is no longer active.".to_string());
                };
                tab.viewer.apply_preset(&preset);
                let label = preset.summary_label();
                self.set_status(format!("Applied pod log preset: {label}"));
                Ok(label)
            }
            PresetCycle::Workload { current, presets } => {
                let preset = cycle_named_workload_preset(&presets, &current, forward)
                    .ok_or_else(|| "No saved workload log presets yet.".to_string())?;
                let Some(active_tab) = self.workbench.active_tab_mut() else {
                    return Err("No active workbench tab.".to_string());
                };
                let WorkbenchTabState::WorkloadLogs(tab) = &mut active_tab.state else {
                    return Err("Workload log preset target is no longer active.".to_string());
                };
                tab.apply_preset(&preset);
                let label = preset.summary_label();
                self.set_status(format!("Applied workload log preset: {label}"));
                Ok(label)
            }
        }
    }
}

const MAX_SAVED_LOG_PRESETS: usize = 12;

fn save_named_pod_preset(
    presets: &mut Vec<crate::log_investigation::PodLogPreset>,
    mut preset: crate::log_investigation::PodLogPreset,
) -> String {
    let base_name = suggested_pod_preset_name(&preset);
    preset.name = unique_preset_name(
        base_name,
        presets.iter().enumerate().filter_map(|(index, existing)| {
            (!same_pod_preset(existing, &preset)).then_some((index, existing.name.as_str()))
        }),
    );
    upsert_pod_preset(presets, preset.clone());
    preset.name
}

fn save_named_workload_preset(
    presets: &mut Vec<crate::log_investigation::WorkloadLogPreset>,
    mut preset: crate::log_investigation::WorkloadLogPreset,
) -> String {
    let base_name = suggested_workload_preset_name(&preset);
    preset.name = unique_preset_name(
        base_name,
        presets.iter().enumerate().filter_map(|(index, existing)| {
            (!same_workload_preset(existing, &preset)).then_some((index, existing.name.as_str()))
        }),
    );
    upsert_workload_preset(presets, preset.clone());
    preset.name
}

fn upsert_pod_preset(
    presets: &mut Vec<crate::log_investigation::PodLogPreset>,
    preset: crate::log_investigation::PodLogPreset,
) {
    if let Some(index) = presets
        .iter()
        .position(|existing| same_pod_preset(existing, &preset))
    {
        presets.remove(index);
    }
    presets.push(preset);
    if presets.len() > MAX_SAVED_LOG_PRESETS {
        let drain = presets.len() - MAX_SAVED_LOG_PRESETS;
        presets.drain(..drain);
    }
}

fn upsert_workload_preset(
    presets: &mut Vec<crate::log_investigation::WorkloadLogPreset>,
    preset: crate::log_investigation::WorkloadLogPreset,
) {
    if let Some(index) = presets
        .iter()
        .position(|existing| same_workload_preset(existing, &preset))
    {
        presets.remove(index);
    }
    presets.push(preset);
    if presets.len() > MAX_SAVED_LOG_PRESETS {
        let drain = presets.len() - MAX_SAVED_LOG_PRESETS;
        presets.drain(..drain);
    }
}

fn cycle_named_pod_preset(
    presets: &[crate::log_investigation::PodLogPreset],
    current: &crate::log_investigation::PodLogPreset,
    forward: bool,
) -> Option<crate::log_investigation::PodLogPreset> {
    cycle_named_preset_index(
        presets.len(),
        presets
            .iter()
            .position(|preset| same_pod_preset(preset, current)),
        forward,
    )
    .and_then(|index| presets.get(index).cloned())
}

fn cycle_named_workload_preset(
    presets: &[crate::log_investigation::WorkloadLogPreset],
    current: &crate::log_investigation::WorkloadLogPreset,
    forward: bool,
) -> Option<crate::log_investigation::WorkloadLogPreset> {
    cycle_named_preset_index(
        presets.len(),
        presets
            .iter()
            .position(|preset| same_workload_preset(preset, current)),
        forward,
    )
    .and_then(|index| presets.get(index).cloned())
}

fn cycle_named_preset_index(
    len: usize,
    current_index: Option<usize>,
    forward: bool,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match (current_index, forward) {
        (Some(index), true) => (index + 1) % len,
        (Some(index), false) => index.checked_sub(1).unwrap_or(len - 1),
        (None, true) => 0,
        (None, false) => len - 1,
    })
}

fn unique_preset_name<'a>(
    base_name: String,
    existing_names: impl Iterator<Item = (usize, &'a str)>,
) -> String {
    let existing = existing_names
        .map(|(_, name)| name.to_string())
        .collect::<std::collections::HashSet<_>>();
    if !existing.contains(&base_name) {
        return base_name;
    }

    for suffix in 2..=MAX_SAVED_LOG_PRESETS + 1 {
        let candidate = format!("{base_name} ({suffix})");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    format!("{base_name} ({})", existing.len() + 1)
}

fn suggested_pod_preset_name(preset: &crate::log_investigation::PodLogPreset) -> String {
    let base = if preset.query.trim().is_empty() {
        "pod logs".to_string()
    } else {
        format!(
            "{} {}",
            if matches!(preset.mode, crate::log_investigation::LogQueryMode::Regex) {
                "regex"
            } else {
                "text"
            },
            summarize_query(&preset.query)
        )
    };
    if preset.structured_view {
        append_window_label(base, preset.time_window)
    } else {
        append_window_label(format!("{base} raw"), preset.time_window)
    }
}

fn suggested_workload_preset_name(preset: &crate::log_investigation::WorkloadLogPreset) -> String {
    let mut parts = Vec::with_capacity(3);
    if preset.query.trim().is_empty() {
        parts.push("workload logs".to_string());
    } else {
        parts.push(format!(
            "{} {}",
            if matches!(preset.mode, crate::log_investigation::LogQueryMode::Regex) {
                "regex"
            } else {
                "text"
            },
            summarize_query(&preset.query)
        ));
    }
    if let Some(pod) = preset.pod_filter.as_deref() {
        parts.push(format!("pod={pod}"));
    }
    if let Some(container) = preset.container_filter.as_deref() {
        parts.push(format!("ctr={container}"));
    }
    if let Some(label) = preset.label_filter.as_deref() {
        parts.push(format!("label={label}"));
    }
    let mut label = parts.join(" ");
    if !preset.structured_view {
        label.push_str(" raw");
    }
    append_window_label(label, preset.time_window)
}

fn summarize_query(query: &str) -> String {
    let trimmed = query.trim();
    let compact = trimmed.chars().take(24).collect::<String>();
    if trimmed.chars().count() > 24 {
        format!("{compact}…")
    } else {
        compact
    }
}

fn same_pod_preset(
    left: &crate::log_investigation::PodLogPreset,
    right: &crate::log_investigation::PodLogPreset,
) -> bool {
    left.query == right.query
        && left.mode == right.mode
        && left.time_window == right.time_window
        && left.structured_view == right.structured_view
}

fn same_workload_preset(
    left: &crate::log_investigation::WorkloadLogPreset,
    right: &crate::log_investigation::WorkloadLogPreset,
) -> bool {
    left.query == right.query
        && left.mode == right.mode
        && left.time_window == right.time_window
        && left.structured_view == right.structured_view
        && left.label_filter == right.label_filter
        && left.pod_filter == right.pod_filter
        && left.container_filter == right.container_filter
}

fn append_window_label(label: String, window: crate::log_investigation::LogTimeWindow) -> String {
    if matches!(window, crate::log_investigation::LogTimeWindow::All) {
        label
    } else {
        format!("{label} {}", window.label())
    }
}
