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
}
