//! Keyboard input handling for AppState.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::views::AppView;
use super::{
    ActiveComponent, AppAction, AppState, ContentPaneFocus, DetailViewState, Focus, PodSortColumn,
    WorkloadSortColumn,
};
use crate::{
    policy::{DetailAction, ViewAction},
    ui::components::{
        CommandPaletteAction, ContextPickerAction, NamespacePickerAction, scale_dialog::ScaleField,
    },
    ui::{
        clear_input_at_cursor, delete_char_left_at_cursor, delete_char_right_at_cursor,
        insert_char_at_cursor, move_cursor_end, move_cursor_home, move_cursor_left,
        move_cursor_right,
    },
    workbench::{AccessReviewFocus, ConnectivityTabFocus, WorkbenchTabState},
};

fn plain_shortcut(key: KeyEvent) -> bool {
    key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
}

fn copy_resource_name_shortcut(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && key
            .modifiers
            .difference(KeyModifiers::CONTROL | KeyModifiers::SHIFT)
            .is_empty()
}

fn ctrl_shortcut(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && key
            .modifiers
            .difference(KeyModifiers::CONTROL | KeyModifiers::SHIFT)
            .is_empty()
}

fn view_supports_content_detail_scroll(view: AppView) -> bool {
    view.supports_secondary_pane_scroll()
}

fn view_supports_selected_resource_shortcut(view: AppView, extension_in_instances: bool) -> bool {
    match view {
        AppView::Dashboard | AppView::HelmCharts | AppView::PortForwarding => false,
        AppView::Extensions => extension_in_instances,
        _ => true,
    }
}

fn app_supports_selected_resource_action_shortcut(app: &AppState) -> bool {
    app.detail_view
        .as_ref()
        .and_then(|detail| detail.resource.as_ref())
        .is_some()
        || (app.detail_view.is_none()
            && app.focus == Focus::Content
            && view_supports_selected_resource_shortcut(app.view, app.extension_in_instances))
}

fn view_supports_resource_events_shortcut(view: AppView) -> bool {
    matches!(
        view,
        AppView::Pods
            | AppView::Deployments
            | AppView::StatefulSets
            | AppView::DaemonSets
            | AppView::ReplicaSets
            | AppView::Jobs
            | AppView::CronJobs
            | AppView::Services
            | AppView::Ingresses
            | AppView::ConfigMaps
            | AppView::PersistentVolumeClaims
            | AppView::HelmReleases
    )
}

fn view_supports_logs_shortcut(view: AppView) -> bool {
    matches!(
        view,
        AppView::Pods
            | AppView::Deployments
            | AppView::StatefulSets
            | AppView::DaemonSets
            | AppView::ReplicaSets
            | AppView::ReplicationControllers
            | AppView::Jobs
    )
}

fn view_supports_pod_only_shortcut(view: AppView) -> bool {
    matches!(view, AppView::Pods)
}

impl AppState {
    fn handle_detail_confirmation_key(&mut self, key: KeyEvent) -> Option<AppAction> {
        let detail = self.detail_view.as_mut()?;
        if !detail.has_confirmation_dialog() {
            return None;
        }

        let action = match key.code {
            KeyCode::Esc if plain_shortcut(key) => {
                detail.confirm_delete = false;
                detail.confirm_drain = false;
                detail.confirm_cronjob_suspend = None;
                AppAction::None
            }
            KeyCode::Char('F') if detail.confirm_drain && plain_shortcut(key) => {
                AppAction::ForceDrainNode
            }
            KeyCode::Char('D') | KeyCode::Char('y') | KeyCode::Enter
                if detail.confirm_drain && plain_shortcut(key) =>
            {
                AppAction::DrainNode
            }
            KeyCode::Char('F') if detail.confirm_delete && plain_shortcut(key) => {
                AppAction::ForceDeleteResource
            }
            KeyCode::Char('D') | KeyCode::Char('d') | KeyCode::Char('y') | KeyCode::Enter
                if detail.confirm_delete && plain_shortcut(key) =>
            {
                AppAction::DeleteResource
            }
            KeyCode::Char('S') | KeyCode::Char('y') | KeyCode::Enter
                if detail.confirm_cronjob_suspend.is_some() && plain_shortcut(key) =>
            {
                AppAction::SetCronJobSuspend(detail.confirm_cronjob_suspend.unwrap_or(false))
            }
            _ => AppAction::None,
        };

        Some(action)
    }

    fn workbench_local_editor_active(&self) -> bool {
        if self.focus != Focus::Workbench || !self.workbench.open {
            return false;
        }

        let Some(tab) = self.workbench.active_tab() else {
            return false;
        };

        match &tab.state {
            WorkbenchTabState::AccessReview(tab) => {
                matches!(tab.focus, AccessReviewFocus::SubjectInput)
            }
            WorkbenchTabState::Connectivity(tab) => {
                matches!(tab.focus, ConnectivityTabFocus::Filter)
            }
            WorkbenchTabState::DecodedSecret(tab) => tab.editing,
            WorkbenchTabState::PodLogs(tab) => tab.viewer.searching || tab.viewer.jumping_to_time,
            WorkbenchTabState::WorkloadLogs(tab) => tab.editing_text_filter || tab.jumping_to_time,
            WorkbenchTabState::Exec(tab) => !tab.picking_container,
            _ => false,
        }
    }

    fn workbench_global_overlay_action(&self, key: KeyEvent) -> Option<AppAction> {
        if self.focus != Focus::Workbench
            || !self.workbench.open
            || self.workbench_local_editor_active()
            || self
                .detail_view
                .as_ref()
                .is_some_and(DetailViewState::has_confirmation_dialog)
        {
            return None;
        }

        match key.code {
            KeyCode::Char(':') if plain_shortcut(key) => Some(AppAction::OpenCommandPalette),
            KeyCode::Char('?') if plain_shortcut(key) => Some(AppAction::OpenHelp),
            KeyCode::Char('~') if plain_shortcut(key) => Some(AppAction::OpenNamespacePicker),
            _ => None,
        }
    }

    fn handle_workbench_key_event(&mut self, key: KeyEvent) -> AppAction {
        use crate::ui::components::port_forward_dialog::PortForwardAction;

        let local_editor_active = self.workbench_local_editor_active();
        // Common workbench keys (apply to all tab types)
        if !local_editor_active {
            match key.code {
                KeyCode::Char('z') if plain_shortcut(key) => {
                    return AppAction::WorkbenchToggleMaximize;
                }
                KeyCode::Char('b') if plain_shortcut(key) => {
                    return AppAction::ToggleWorkbench;
                }
                KeyCode::Char(',') if plain_shortcut(key) => {
                    return AppAction::WorkbenchPreviousTab;
                }
                KeyCode::Char('.') if plain_shortcut(key) => {
                    return AppAction::WorkbenchNextTab;
                }
                KeyCode::Char('w') if ctrl_shortcut(key) => {
                    return AppAction::WorkbenchCloseActiveTab;
                }
                KeyCode::Up if ctrl_shortcut(key) => {
                    return AppAction::WorkbenchIncreaseHeight;
                }
                KeyCode::Down if ctrl_shortcut(key) => {
                    return AppAction::WorkbenchDecreaseHeight;
                }
                _ => {}
            }
        }

        let action_history_ids = self
            .visible_action_history_entries()
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        let Some(tab) = self.workbench.active_tab_mut() else {
            return AppAction::None;
        };

        match &mut tab.state {
            WorkbenchTabState::ActionHistory(tab) => match key.code {
                KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                    tab.select_next(&action_history_ids);
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                    tab.select_previous(&action_history_ids);
                    AppAction::None
                }
                KeyCode::Char('g') if plain_shortcut(key) => {
                    tab.select_top(&action_history_ids);
                    AppAction::None
                }
                KeyCode::Char('G') if plain_shortcut(key) => {
                    tab.select_bottom(&action_history_ids);
                    AppAction::None
                }
                KeyCode::PageDown if plain_shortcut(key) => {
                    for _ in 0..10 {
                        tab.select_next(&action_history_ids);
                    }
                    AppAction::None
                }
                KeyCode::PageUp if plain_shortcut(key) => {
                    for _ in 0..10 {
                        tab.select_previous(&action_history_ids);
                    }
                    AppAction::None
                }
                KeyCode::Enter if plain_shortcut(key) => AppAction::ActionHistoryOpenSelected,
                _ => AppAction::None,
            },
            WorkbenchTabState::AccessReview(tab) => {
                let max_scroll = tab.line_count().saturating_sub(1);
                match tab.focus {
                    AccessReviewFocus::Summary => match key.code {
                        KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                        KeyCode::Tab | KeyCode::Char('s') | KeyCode::Char('/')
                            if plain_shortcut(key) =>
                        {
                            tab.start_subject_input();
                            AppAction::None
                        }
                        KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                            tab.scroll = tab.scroll.saturating_add(1).min(max_scroll);
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                            tab.scroll = tab.scroll.saturating_sub(1);
                            AppAction::None
                        }
                        KeyCode::Char('g') if plain_shortcut(key) => {
                            tab.scroll = 0;
                            AppAction::None
                        }
                        KeyCode::Char('G') if plain_shortcut(key) => {
                            tab.scroll = max_scroll;
                            AppAction::None
                        }
                        KeyCode::PageDown if plain_shortcut(key) => {
                            tab.scroll = tab.scroll.saturating_add(10).min(max_scroll);
                            AppAction::None
                        }
                        KeyCode::PageUp if plain_shortcut(key) => {
                            tab.scroll = tab.scroll.saturating_sub(10);
                            AppAction::None
                        }
                        _ => AppAction::None,
                    },
                    AccessReviewFocus::SubjectInput => match key.code {
                        KeyCode::Esc if plain_shortcut(key) => {
                            tab.stop_subject_input();
                            AppAction::None
                        }
                        KeyCode::Tab | KeyCode::BackTab if plain_shortcut(key) => {
                            tab.stop_subject_input();
                            AppAction::None
                        }
                        KeyCode::Backspace => {
                            tab.subject_input.backspace_char();
                            tab.subject_input_error = None;
                            AppAction::None
                        }
                        KeyCode::Delete => {
                            tab.subject_input.delete_char();
                            tab.subject_input_error = None;
                            AppAction::None
                        }
                        KeyCode::Left => {
                            tab.subject_input.cursor_left();
                            AppAction::None
                        }
                        KeyCode::Right => {
                            tab.subject_input.cursor_right();
                            AppAction::None
                        }
                        KeyCode::Home => {
                            tab.subject_input.cursor_home();
                            AppAction::None
                        }
                        KeyCode::End => {
                            tab.subject_input.cursor_end();
                            AppAction::None
                        }
                        KeyCode::Enter if plain_shortcut(key) => {
                            AppAction::ApplyAccessReviewSubject
                        }
                        KeyCode::Char('u') if ctrl_shortcut(key) => {
                            tab.subject_input.clear();
                            tab.subject_input_error = None;
                            AppAction::None
                        }
                        KeyCode::Char(ch) if plain_shortcut(key) => {
                            tab.subject_input.add_char(ch);
                            tab.subject_input_error = None;
                            AppAction::None
                        }
                        _ => AppAction::None,
                    },
                }
            }
            WorkbenchTabState::ResourceYaml(tab) => {
                let max_scroll = tab
                    .yaml
                    .as_ref()
                    .map(|yaml| yaml.lines().count().saturating_sub(1))
                    .unwrap_or(0);
                match key.code {
                    KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                    KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                        tab.scroll = tab.scroll.saturating_add(1).min(max_scroll);
                        AppAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                        tab.scroll = tab.scroll.saturating_sub(1);
                        AppAction::None
                    }
                    KeyCode::Char('g') if plain_shortcut(key) => {
                        tab.scroll = 0;
                        AppAction::None
                    }
                    KeyCode::Char('G') if plain_shortcut(key) => {
                        tab.scroll = max_scroll;
                        AppAction::None
                    }
                    KeyCode::PageDown if plain_shortcut(key) => {
                        tab.scroll = tab.scroll.saturating_add(10).min(max_scroll);
                        AppAction::None
                    }
                    KeyCode::PageUp if plain_shortcut(key) => {
                        tab.scroll = tab.scroll.saturating_sub(10);
                        AppAction::None
                    }
                    _ => AppAction::None,
                }
            }
            WorkbenchTabState::ResourceDiff(tab) => {
                let max_scroll = tab.lines.len().saturating_sub(1);
                match key.code {
                    KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                    KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                        tab.scroll = tab.scroll.saturating_add(1).min(max_scroll);
                        AppAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                        tab.scroll = tab.scroll.saturating_sub(1);
                        AppAction::None
                    }
                    KeyCode::Char('g') if plain_shortcut(key) => {
                        tab.scroll = 0;
                        AppAction::None
                    }
                    KeyCode::Char('G') if plain_shortcut(key) => {
                        tab.scroll = max_scroll;
                        AppAction::None
                    }
                    KeyCode::PageDown if plain_shortcut(key) => {
                        tab.scroll = tab.scroll.saturating_add(10).min(max_scroll);
                        AppAction::None
                    }
                    KeyCode::PageUp if plain_shortcut(key) => {
                        tab.scroll = tab.scroll.saturating_sub(10);
                        AppAction::None
                    }
                    _ => AppAction::None,
                }
            }
            WorkbenchTabState::ExtensionOutput(tab) => match key.code {
                KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                    tab.scroll = tab.scroll.saturating_add(1);
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                    tab.scroll = tab.scroll.saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('g') if plain_shortcut(key) => {
                    tab.scroll = 0;
                    AppAction::None
                }
                KeyCode::Char('G') if plain_shortcut(key) => {
                    tab.scroll = usize::MAX;
                    AppAction::None
                }
                KeyCode::PageDown if plain_shortcut(key) => {
                    tab.scroll = tab.scroll.saturating_add(10);
                    AppAction::None
                }
                KeyCode::PageUp if plain_shortcut(key) => {
                    tab.scroll = tab.scroll.saturating_sub(10);
                    AppAction::None
                }
                _ => AppAction::None,
            },
            WorkbenchTabState::AiAnalysis(tab) => match key.code {
                KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                    tab.scroll = tab.scroll.saturating_add(1);
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                    tab.scroll = tab.scroll.saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('g') if plain_shortcut(key) => {
                    tab.scroll = 0;
                    AppAction::None
                }
                KeyCode::Char('G') if plain_shortcut(key) => {
                    tab.scroll = usize::MAX;
                    AppAction::None
                }
                KeyCode::PageDown if plain_shortcut(key) => {
                    tab.scroll = tab.scroll.saturating_add(10);
                    AppAction::None
                }
                KeyCode::PageUp if plain_shortcut(key) => {
                    tab.scroll = tab.scroll.saturating_sub(10);
                    AppAction::None
                }
                _ => AppAction::None,
            },
            WorkbenchTabState::Runbook(tab) => match key.code {
                KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                KeyCode::Char('j') | KeyCode::Down if ctrl_shortcut(key) => {
                    tab.scroll_detail_down(1);
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up if ctrl_shortcut(key) => {
                    tab.scroll_detail_up(1);
                    AppAction::None
                }
                KeyCode::Char('d') | KeyCode::Char('D') if ctrl_shortcut(key) => {
                    tab.scroll_detail_down(10);
                    AppAction::None
                }
                KeyCode::Char('u') | KeyCode::Char('U') if ctrl_shortcut(key) => {
                    tab.scroll_detail_up(10);
                    AppAction::None
                }
                KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                    tab.select_next();
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                    tab.select_previous();
                    AppAction::None
                }
                KeyCode::Char('g') if plain_shortcut(key) => {
                    tab.select_top();
                    AppAction::None
                }
                KeyCode::Char('G') if plain_shortcut(key) => {
                    tab.select_bottom();
                    AppAction::None
                }
                KeyCode::PageDown if plain_shortcut(key) => {
                    for _ in 0..10 {
                        tab.select_next();
                    }
                    AppAction::None
                }
                KeyCode::PageUp if plain_shortcut(key) => {
                    for _ in 0..10 {
                        tab.select_previous();
                    }
                    AppAction::None
                }
                KeyCode::Char('d') if plain_shortcut(key) => AppAction::RunbookToggleStepDone,
                KeyCode::Char('s') if plain_shortcut(key) => AppAction::RunbookToggleStepSkipped,
                KeyCode::Enter if plain_shortcut(key) => AppAction::RunbookExecuteSelectedStep,
                _ => AppAction::None,
            },
            WorkbenchTabState::HelmHistory(tab) => {
                if tab.rollback_pending {
                    return match key.code {
                        KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                            tab.scroll = tab.scroll.saturating_add(1);
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                            tab.scroll = tab.scroll.saturating_sub(1);
                            AppAction::None
                        }
                        KeyCode::PageDown | KeyCode::Char('d') | KeyCode::Char('D')
                            if ctrl_shortcut(key) =>
                        {
                            tab.scroll = tab.scroll.saturating_add(10);
                            AppAction::None
                        }
                        KeyCode::PageUp | KeyCode::Char('u') | KeyCode::Char('U')
                            if ctrl_shortcut(key) =>
                        {
                            tab.scroll = tab.scroll.saturating_sub(10);
                            AppAction::None
                        }
                        KeyCode::Esc if plain_shortcut(key) => AppAction::None,
                        _ => AppAction::None,
                    };
                }

                if tab.confirm_rollback_revision.is_some() {
                    return match key.code {
                        KeyCode::Esc if plain_shortcut(key) => {
                            tab.cancel_rollback_confirm();
                            AppAction::None
                        }
                        KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                            tab.scroll = tab.scroll.saturating_add(1);
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                            tab.scroll = tab.scroll.saturating_sub(1);
                            AppAction::None
                        }
                        KeyCode::PageDown | KeyCode::Char('d') | KeyCode::Char('D')
                            if ctrl_shortcut(key) =>
                        {
                            tab.scroll = tab.scroll.saturating_add(10);
                            AppAction::None
                        }
                        KeyCode::PageUp | KeyCode::Char('u') | KeyCode::Char('U')
                            if ctrl_shortcut(key) =>
                        {
                            tab.scroll = tab.scroll.saturating_sub(10);
                            AppAction::None
                        }
                        KeyCode::Char('R') | KeyCode::Char('y') | KeyCode::Enter
                            if plain_shortcut(key) =>
                        {
                            AppAction::ExecuteHelmRollback
                        }
                        _ => AppAction::None,
                    };
                }

                if let Some(diff) = tab.diff.as_mut() {
                    let max_scroll = diff.lines.len().saturating_sub(1);
                    return match key.code {
                        KeyCode::Esc if plain_shortcut(key) => {
                            tab.close_diff();
                            AppAction::None
                        }
                        KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                            diff.scroll = diff.scroll.saturating_add(1).min(max_scroll);
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                            diff.scroll = diff.scroll.saturating_sub(1);
                            AppAction::None
                        }
                        KeyCode::Char('g') if plain_shortcut(key) => {
                            diff.scroll = 0;
                            AppAction::None
                        }
                        KeyCode::Char('G') if plain_shortcut(key) => {
                            diff.scroll = max_scroll;
                            AppAction::None
                        }
                        KeyCode::PageDown if plain_shortcut(key) => {
                            diff.scroll = diff.scroll.saturating_add(10).min(max_scroll);
                            AppAction::None
                        }
                        KeyCode::PageUp if plain_shortcut(key) => {
                            diff.scroll = diff.scroll.saturating_sub(10);
                            AppAction::None
                        }
                        KeyCode::Char('R')
                            if tab.selected_target_revision().is_some() && plain_shortcut(key) =>
                        {
                            AppAction::ConfirmHelmRollback
                        }
                        _ => AppAction::None,
                    };
                }

                match key.code {
                    KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                    KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                        tab.select_next();
                        AppAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                        tab.select_previous();
                        AppAction::None
                    }
                    KeyCode::Char('g') if plain_shortcut(key) => {
                        tab.select_top();
                        AppAction::None
                    }
                    KeyCode::Char('G') if plain_shortcut(key) => {
                        tab.select_bottom();
                        AppAction::None
                    }
                    KeyCode::PageDown if plain_shortcut(key) => {
                        for _ in 0..10 {
                            tab.select_next();
                        }
                        AppAction::None
                    }
                    KeyCode::PageUp if plain_shortcut(key) => {
                        for _ in 0..10 {
                            tab.select_previous();
                        }
                        AppAction::None
                    }
                    KeyCode::Enter
                        if tab.selected_target_revision().is_some() && plain_shortcut(key) =>
                    {
                        AppAction::OpenHelmValuesDiff
                    }
                    KeyCode::Char('R')
                        if tab.selected_target_revision().is_some() && plain_shortcut(key) =>
                    {
                        AppAction::ConfirmHelmRollback
                    }
                    _ => AppAction::None,
                }
            }
            WorkbenchTabState::Rollout(tab) => {
                if tab.mutation_pending.is_some() {
                    return match key.code {
                        KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                            tab.detail_scroll = tab.detail_scroll.saturating_add(1);
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                            tab.detail_scroll = tab.detail_scroll.saturating_sub(1);
                            AppAction::None
                        }
                        KeyCode::PageDown | KeyCode::Char('d') | KeyCode::Char('D')
                            if ctrl_shortcut(key) =>
                        {
                            tab.detail_scroll = tab.detail_scroll.saturating_add(10);
                            AppAction::None
                        }
                        KeyCode::PageUp | KeyCode::Char('u') | KeyCode::Char('U')
                            if ctrl_shortcut(key) =>
                        {
                            tab.detail_scroll = tab.detail_scroll.saturating_sub(10);
                            AppAction::None
                        }
                        _ => AppAction::None,
                    };
                }

                if tab.confirm_undo_revision.is_some() {
                    return match key.code {
                        KeyCode::Esc if plain_shortcut(key) => {
                            tab.cancel_undo_confirm();
                            AppAction::None
                        }
                        KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                            tab.detail_scroll = tab.detail_scroll.saturating_add(1);
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                            tab.detail_scroll = tab.detail_scroll.saturating_sub(1);
                            AppAction::None
                        }
                        KeyCode::PageDown | KeyCode::Char('d') | KeyCode::Char('D')
                            if ctrl_shortcut(key) =>
                        {
                            tab.detail_scroll = tab.detail_scroll.saturating_add(10);
                            AppAction::None
                        }
                        KeyCode::PageUp | KeyCode::Char('u') | KeyCode::Char('U')
                            if ctrl_shortcut(key) =>
                        {
                            tab.detail_scroll = tab.detail_scroll.saturating_sub(10);
                            AppAction::None
                        }
                        KeyCode::Char('U') | KeyCode::Char('y') | KeyCode::Enter
                            if plain_shortcut(key) =>
                        {
                            AppAction::ExecuteRolloutUndo
                        }
                        _ => AppAction::None,
                    };
                }

                match key.code {
                    KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                    KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                        tab.select_next();
                        AppAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                        tab.select_previous();
                        AppAction::None
                    }
                    KeyCode::Char('g') if plain_shortcut(key) => {
                        tab.select_top();
                        AppAction::None
                    }
                    KeyCode::Char('G') if plain_shortcut(key) => {
                        tab.select_bottom();
                        AppAction::None
                    }
                    KeyCode::PageDown if plain_shortcut(key) => {
                        for _ in 0..10 {
                            tab.select_next();
                        }
                        AppAction::None
                    }
                    KeyCode::PageUp if plain_shortcut(key) => {
                        for _ in 0..10 {
                            tab.select_previous();
                        }
                        AppAction::None
                    }
                    KeyCode::Char('R') if plain_shortcut(key) => AppAction::RolloutRestart,
                    KeyCode::Char('P')
                        if tab.kind
                            == Some(crate::k8s::rollout::RolloutWorkloadKind::Deployment)
                            && plain_shortcut(key) =>
                    {
                        AppAction::ToggleRolloutPauseResume
                    }
                    KeyCode::Char('U')
                        if tab.selected_undo_revision().is_some() && plain_shortcut(key) =>
                    {
                        AppAction::ConfirmRolloutUndo
                    }
                    _ => AppAction::None,
                }
            }
            WorkbenchTabState::DecodedSecret(tab) => {
                if tab.editing && tab.masked {
                    tab.editing = false;
                    tab.edit_input.clear();
                    tab.edit_cursor = 0;
                }

                if tab.editing {
                    match key.code {
                        KeyCode::Esc if plain_shortcut(key) => {
                            tab.editing = false;
                            tab.edit_input.clear();
                            AppAction::None
                        }
                        KeyCode::Enter if plain_shortcut(key) => {
                            let edited = std::mem::take(&mut tab.edit_input);
                            if let Some(entry) = tab.selected_entry_mut() {
                                entry.commit_edit(edited);
                            }
                            tab.editing = false;
                            tab.edit_cursor = 0;
                            AppAction::None
                        }
                        KeyCode::Backspace => {
                            delete_char_left_at_cursor(&mut tab.edit_input, &mut tab.edit_cursor);
                            AppAction::None
                        }
                        KeyCode::Delete => {
                            delete_char_right_at_cursor(&mut tab.edit_input, tab.edit_cursor);
                            AppAction::None
                        }
                        KeyCode::Left => {
                            move_cursor_left(&mut tab.edit_cursor);
                            AppAction::None
                        }
                        KeyCode::Right => {
                            move_cursor_right(&mut tab.edit_cursor, &tab.edit_input);
                            AppAction::None
                        }
                        KeyCode::Home => {
                            tab.edit_cursor = 0;
                            AppAction::None
                        }
                        KeyCode::End => {
                            move_cursor_end(&mut tab.edit_cursor, &tab.edit_input);
                            AppAction::None
                        }
                        KeyCode::Char('u') if ctrl_shortcut(key) => {
                            clear_input_at_cursor(&mut tab.edit_input, &mut tab.edit_cursor);
                            AppAction::None
                        }
                        KeyCode::Char(c) if plain_shortcut(key) => {
                            insert_char_at_cursor(&mut tab.edit_input, &mut tab.edit_cursor, c);
                            AppAction::None
                        }
                        _ => AppAction::None,
                    }
                } else {
                    match key.code {
                        KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                        KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                            tab.select_next();
                            tab.scroll = tab.scroll.max(tab.selected.saturating_sub(1));
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                            tab.select_previous();
                            tab.scroll = tab.scroll.min(tab.selected);
                            AppAction::None
                        }
                        KeyCode::Char('g') if plain_shortcut(key) => {
                            tab.select_top();
                            tab.scroll = 0;
                            AppAction::None
                        }
                        KeyCode::Char('G') if plain_shortcut(key) => {
                            tab.select_bottom();
                            tab.scroll = tab.selected;
                            AppAction::None
                        }
                        KeyCode::Char('m') if plain_shortcut(key) => {
                            tab.masked = !tab.masked;
                            AppAction::None
                        }
                        KeyCode::Char('e') | KeyCode::Enter
                            if !tab.masked && plain_shortcut(key) =>
                        {
                            if let Some(entry) = tab.selected_entry()
                                && let Some(value) = entry.editable_text()
                            {
                                tab.edit_input = value.to_string();
                                tab.edit_cursor = tab.edit_input.chars().count();
                                tab.editing = true;
                            }
                            AppAction::None
                        }
                        KeyCode::Char('s') if tab.has_unsaved_changes() && plain_shortcut(key) => {
                            AppAction::SaveDecodedSecret
                        }
                        _ => AppAction::None,
                    }
                }
            }
            WorkbenchTabState::ResourceEvents(tab) => match key.code {
                KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                    tab.scroll = tab.scroll.saturating_add(1);
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                    tab.scroll = tab.scroll.saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('g') if plain_shortcut(key) => {
                    tab.scroll = 0;
                    AppAction::None
                }
                KeyCode::Char('G') if plain_shortcut(key) => {
                    tab.scroll = usize::MAX;
                    AppAction::None
                }
                KeyCode::PageDown if plain_shortcut(key) => {
                    tab.scroll = tab.scroll.saturating_add(10);
                    AppAction::None
                }
                KeyCode::PageUp if plain_shortcut(key) => {
                    tab.scroll = tab.scroll.saturating_sub(10);
                    AppAction::None
                }
                _ => AppAction::None,
            },
            WorkbenchTabState::PodLogs(tab) => {
                if tab.viewer.searching {
                    match key.code {
                        KeyCode::Esc if plain_shortcut(key) => AppAction::LogsViewerSearchCancel,
                        KeyCode::Enter if plain_shortcut(key) => AppAction::LogsViewerSearchClose,
                        KeyCode::Backspace => {
                            delete_char_left_at_cursor(
                                &mut tab.viewer.search_input,
                                &mut tab.viewer.search_cursor,
                            );
                            AppAction::None
                        }
                        KeyCode::Delete => {
                            delete_char_right_at_cursor(
                                &mut tab.viewer.search_input,
                                tab.viewer.search_cursor,
                            );
                            AppAction::None
                        }
                        KeyCode::Left => {
                            move_cursor_left(&mut tab.viewer.search_cursor);
                            AppAction::None
                        }
                        KeyCode::Right => {
                            move_cursor_right(
                                &mut tab.viewer.search_cursor,
                                &tab.viewer.search_input,
                            );
                            AppAction::None
                        }
                        KeyCode::Home => {
                            move_cursor_home(&mut tab.viewer.search_cursor);
                            AppAction::None
                        }
                        KeyCode::End => {
                            move_cursor_end(
                                &mut tab.viewer.search_cursor,
                                &tab.viewer.search_input,
                            );
                            AppAction::None
                        }
                        KeyCode::Char('u') if ctrl_shortcut(key) => {
                            clear_input_at_cursor(
                                &mut tab.viewer.search_input,
                                &mut tab.viewer.search_cursor,
                            );
                            AppAction::None
                        }
                        KeyCode::Char(c) if plain_shortcut(key) => {
                            insert_char_at_cursor(
                                &mut tab.viewer.search_input,
                                &mut tab.viewer.search_cursor,
                                c,
                            );
                            AppAction::None
                        }
                        _ => AppAction::None,
                    }
                } else if tab.viewer.jumping_to_time {
                    match key.code {
                        KeyCode::Esc if plain_shortcut(key) => AppAction::CancelLogTimeJump,
                        KeyCode::Enter if plain_shortcut(key) => AppAction::ApplyLogTimeJump,
                        KeyCode::Backspace => {
                            delete_char_left_at_cursor(
                                &mut tab.viewer.time_jump_input,
                                &mut tab.viewer.time_jump_cursor,
                            );
                            AppAction::None
                        }
                        KeyCode::Delete => {
                            delete_char_right_at_cursor(
                                &mut tab.viewer.time_jump_input,
                                tab.viewer.time_jump_cursor,
                            );
                            AppAction::None
                        }
                        KeyCode::Left => {
                            move_cursor_left(&mut tab.viewer.time_jump_cursor);
                            AppAction::None
                        }
                        KeyCode::Right => {
                            move_cursor_right(
                                &mut tab.viewer.time_jump_cursor,
                                &tab.viewer.time_jump_input,
                            );
                            AppAction::None
                        }
                        KeyCode::Home => {
                            tab.viewer.time_jump_cursor = 0;
                            AppAction::None
                        }
                        KeyCode::End => {
                            move_cursor_end(
                                &mut tab.viewer.time_jump_cursor,
                                &tab.viewer.time_jump_input,
                            );
                            AppAction::None
                        }
                        KeyCode::Char('u') if ctrl_shortcut(key) => {
                            clear_input_at_cursor(
                                &mut tab.viewer.time_jump_input,
                                &mut tab.viewer.time_jump_cursor,
                            );
                            AppAction::None
                        }
                        KeyCode::Char(c) if plain_shortcut(key) => {
                            insert_char_at_cursor(
                                &mut tab.viewer.time_jump_input,
                                &mut tab.viewer.time_jump_cursor,
                                c,
                            );
                            AppAction::None
                        }
                        _ => AppAction::None,
                    }
                } else {
                    match key.code {
                        KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                        KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                            if tab.viewer.picking_container {
                                AppAction::LogsViewerPickerUp
                            } else {
                                AppAction::LogsViewerScrollUp
                            }
                        }
                        KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                            if tab.viewer.picking_container {
                                AppAction::LogsViewerPickerDown
                            } else {
                                AppAction::LogsViewerScrollDown
                            }
                        }
                        KeyCode::Enter if tab.viewer.picking_container && plain_shortcut(key) => {
                            if tab.viewer.container_cursor == 0 && tab.viewer.containers.len() > 1 {
                                // "All Containers" entry at index 0
                                AppAction::LogsViewerSelectAllContainers
                            } else {
                                // Single container: offset by 1 to skip the "All" entry
                                let real_idx = if tab.viewer.containers.len() > 1 {
                                    tab.viewer.container_cursor.saturating_sub(1)
                                } else {
                                    tab.viewer.container_cursor
                                };
                                tab.viewer
                                    .containers
                                    .get(real_idx)
                                    .cloned()
                                    .map(AppAction::LogsViewerSelectContainer)
                                    .unwrap_or(AppAction::None)
                            }
                        }
                        KeyCode::Char('g') if plain_shortcut(key) => AppAction::LogsViewerScrollTop,
                        KeyCode::Char('G') if plain_shortcut(key) => {
                            AppAction::LogsViewerScrollBottom
                        }
                        KeyCode::Char('f') if plain_shortcut(key) => {
                            AppAction::LogsViewerToggleFollow
                        }
                        KeyCode::Char('P')
                            if !tab.viewer.picking_container && plain_shortcut(key) =>
                        {
                            AppAction::LogsViewerTogglePrevious
                        }
                        KeyCode::Char('t')
                            if !tab.viewer.picking_container && plain_shortcut(key) =>
                        {
                            AppAction::LogsViewerToggleTimestamps
                        }
                        KeyCode::Char('/')
                            if !tab.viewer.picking_container && plain_shortcut(key) =>
                        {
                            AppAction::LogsViewerSearchOpen
                        }
                        KeyCode::Char('n')
                            if !tab.viewer.picking_container && plain_shortcut(key) =>
                        {
                            AppAction::LogsViewerSearchNext
                        }
                        KeyCode::Char('N')
                            if !tab.viewer.picking_container && plain_shortcut(key) =>
                        {
                            AppAction::LogsViewerSearchPrev
                        }
                        KeyCode::Char('R')
                            if !tab.viewer.picking_container && plain_shortcut(key) =>
                        {
                            AppAction::ToggleLogRegexMode
                        }
                        KeyCode::Char('W')
                            if !tab.viewer.picking_container && plain_shortcut(key) =>
                        {
                            AppAction::ToggleLogTimeWindow
                        }
                        KeyCode::Char('T')
                            if !tab.viewer.picking_container && plain_shortcut(key) =>
                        {
                            AppAction::OpenLogTimeJump
                        }
                        KeyCode::Char('C')
                            if !tab.viewer.picking_container && plain_shortcut(key) =>
                        {
                            AppAction::ToggleLogCorrelation
                        }
                        KeyCode::Char('J')
                            if !tab.viewer.picking_container && plain_shortcut(key) =>
                        {
                            AppAction::ToggleStructuredLogView
                        }
                        KeyCode::Char('y')
                            if !tab.viewer.picking_container && plain_shortcut(key) =>
                        {
                            AppAction::CopyLogContent
                        }
                        KeyCode::Char('S') | KeyCode::Char('s')
                            if !tab.viewer.picking_container && plain_shortcut(key) =>
                        {
                            AppAction::ExportLogs
                        }
                        KeyCode::Char('M') | KeyCode::Char('m')
                            if !tab.viewer.picking_container && plain_shortcut(key) =>
                        {
                            AppAction::SaveLogPreset
                        }
                        KeyCode::Char('[')
                            if !tab.viewer.picking_container && plain_shortcut(key) =>
                        {
                            AppAction::ApplyPreviousLogPreset
                        }
                        KeyCode::Char(']')
                            if !tab.viewer.picking_container && plain_shortcut(key) =>
                        {
                            AppAction::ApplyNextLogPreset
                        }
                        _ => AppAction::None,
                    }
                }
            }
            WorkbenchTabState::WorkloadLogs(tab) => {
                if tab.editing_text_filter {
                    match key.code {
                        KeyCode::Esc if plain_shortcut(key) => {
                            tab.editing_text_filter = false;
                            tab.filter_input.clear();
                            AppAction::None
                        }
                        KeyCode::Enter if plain_shortcut(key) => {
                            tab.commit_text_filter();
                            AppAction::None
                        }
                        KeyCode::Backspace => {
                            delete_char_left_at_cursor(
                                &mut tab.filter_input,
                                &mut tab.filter_input_cursor,
                            );
                            AppAction::None
                        }
                        KeyCode::Delete => {
                            delete_char_right_at_cursor(
                                &mut tab.filter_input,
                                tab.filter_input_cursor,
                            );
                            AppAction::None
                        }
                        KeyCode::Left => {
                            move_cursor_left(&mut tab.filter_input_cursor);
                            AppAction::None
                        }
                        KeyCode::Right => {
                            move_cursor_right(&mut tab.filter_input_cursor, &tab.filter_input);
                            AppAction::None
                        }
                        KeyCode::Home => {
                            tab.filter_input_cursor = 0;
                            AppAction::None
                        }
                        KeyCode::End => {
                            move_cursor_end(&mut tab.filter_input_cursor, &tab.filter_input);
                            AppAction::None
                        }
                        KeyCode::Char('u') if ctrl_shortcut(key) => {
                            clear_input_at_cursor(
                                &mut tab.filter_input,
                                &mut tab.filter_input_cursor,
                            );
                            AppAction::None
                        }
                        KeyCode::Char(c) if plain_shortcut(key) => {
                            insert_char_at_cursor(
                                &mut tab.filter_input,
                                &mut tab.filter_input_cursor,
                                c,
                            );
                            AppAction::None
                        }
                        _ => AppAction::None,
                    }
                } else if tab.jumping_to_time {
                    match key.code {
                        KeyCode::Esc if plain_shortcut(key) => AppAction::CancelLogTimeJump,
                        KeyCode::Enter if plain_shortcut(key) => AppAction::ApplyLogTimeJump,
                        KeyCode::Backspace => {
                            delete_char_left_at_cursor(
                                &mut tab.time_jump_input,
                                &mut tab.time_jump_cursor,
                            );
                            AppAction::None
                        }
                        KeyCode::Delete => {
                            delete_char_right_at_cursor(
                                &mut tab.time_jump_input,
                                tab.time_jump_cursor,
                            );
                            AppAction::None
                        }
                        KeyCode::Left => {
                            move_cursor_left(&mut tab.time_jump_cursor);
                            AppAction::None
                        }
                        KeyCode::Right => {
                            move_cursor_right(&mut tab.time_jump_cursor, &tab.time_jump_input);
                            AppAction::None
                        }
                        KeyCode::Home => {
                            tab.time_jump_cursor = 0;
                            AppAction::None
                        }
                        KeyCode::End => {
                            move_cursor_end(&mut tab.time_jump_cursor, &tab.time_jump_input);
                            AppAction::None
                        }
                        KeyCode::Char('u') if ctrl_shortcut(key) => {
                            clear_input_at_cursor(
                                &mut tab.time_jump_input,
                                &mut tab.time_jump_cursor,
                            );
                            AppAction::None
                        }
                        KeyCode::Char(c) if plain_shortcut(key) => {
                            insert_char_at_cursor(
                                &mut tab.time_jump_input,
                                &mut tab.time_jump_cursor,
                                c,
                            );
                            AppAction::None
                        }
                        _ => AppAction::None,
                    }
                } else {
                    match key.code {
                        KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                        KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                            let filtered_len = tab.filtered_len();
                            if filtered_len <= 1 {
                                if filtered_len == 0 {
                                    tab.scroll = 0;
                                } else {
                                    tab.scroll = tab.scroll.saturating_add(1);
                                }
                            } else {
                                tab.scroll = (tab.scroll + 1).min(filtered_len.saturating_sub(1));
                            }
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                            let filtered_len = tab.filtered_len();
                            if filtered_len <= 1 {
                                if filtered_len == 0 {
                                    tab.scroll = 0;
                                } else {
                                    tab.scroll = tab.scroll.saturating_sub(1);
                                }
                            } else {
                                tab.scroll = tab.scroll.saturating_sub(1);
                            }
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::Char('g') if plain_shortcut(key) => {
                            tab.scroll = 0;
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::Char('G') if plain_shortcut(key) => {
                            let filtered_len = tab.filtered_len();
                            tab.scroll = if filtered_len <= 1 && filtered_len > 0 {
                                usize::MAX
                            } else {
                                filtered_len.saturating_sub(1)
                            };
                            tab.follow_mode = true;
                            AppAction::None
                        }
                        KeyCode::PageDown if plain_shortcut(key) => {
                            let filtered_len = tab.filtered_len();
                            if filtered_len <= 1 {
                                if filtered_len == 0 {
                                    tab.scroll = 0;
                                } else {
                                    tab.scroll = tab.scroll.saturating_add(10);
                                }
                            } else {
                                tab.scroll = (tab.scroll + 10).min(filtered_len.saturating_sub(1));
                            }
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::PageUp if plain_shortcut(key) => {
                            tab.scroll = tab.scroll.saturating_sub(10);
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::Char('f') if plain_shortcut(key) => {
                            tab.follow_mode = !tab.follow_mode;
                            if tab.follow_mode {
                                let filtered_len = tab.filtered_len();
                                tab.scroll = filtered_len.saturating_sub(1);
                            }
                            AppAction::None
                        }
                        KeyCode::Char('/') if plain_shortcut(key) => {
                            tab.editing_text_filter = true;
                            tab.filter_input = tab.text_filter.clone();
                            tab.filter_input_cursor = tab.filter_input.chars().count();
                            AppAction::None
                        }
                        KeyCode::Char('p') if plain_shortcut(key) => {
                            tab.cycle_pod_filter();
                            AppAction::None
                        }
                        KeyCode::Char('c') if plain_shortcut(key) => {
                            tab.cycle_container_filter();
                            AppAction::None
                        }
                        KeyCode::Char('R') if plain_shortcut(key) => AppAction::ToggleLogRegexMode,
                        KeyCode::Char('W') if plain_shortcut(key) => AppAction::ToggleLogTimeWindow,
                        KeyCode::Char('T') if plain_shortcut(key) => AppAction::OpenLogTimeJump,
                        KeyCode::Char('L') if plain_shortcut(key) => {
                            AppAction::CycleWorkloadLogLabelFilter
                        }
                        KeyCode::Char('C') if plain_shortcut(key) => {
                            AppAction::ToggleLogCorrelation
                        }
                        KeyCode::Char('J') if plain_shortcut(key) => {
                            AppAction::ToggleStructuredLogView
                        }
                        KeyCode::Char('y') if !tab.editing_text_filter && plain_shortcut(key) => {
                            AppAction::CopyLogContent
                        }
                        KeyCode::Char('S') | KeyCode::Char('s')
                            if !tab.editing_text_filter && plain_shortcut(key) =>
                        {
                            AppAction::ExportLogs
                        }
                        KeyCode::Char('M') | KeyCode::Char('m')
                            if !tab.editing_text_filter && plain_shortcut(key) =>
                        {
                            AppAction::SaveLogPreset
                        }
                        KeyCode::Char('[') if !tab.editing_text_filter && plain_shortcut(key) => {
                            AppAction::ApplyPreviousLogPreset
                        }
                        KeyCode::Char(']') if !tab.editing_text_filter && plain_shortcut(key) => {
                            AppAction::ApplyNextLogPreset
                        }
                        _ => AppAction::None,
                    }
                }
            }
            WorkbenchTabState::Exec(tab) => {
                if tab.picking_container {
                    match key.code {
                        KeyCode::Esc if plain_shortcut(key) => {
                            // Exit container picker back to command input,
                            // don't close the entire workbench.
                            tab.picking_container = false;
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                            tab.container_cursor = tab.container_cursor.saturating_sub(1);
                            AppAction::None
                        }
                        KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                            let max = tab.containers.len().saturating_sub(1);
                            tab.container_cursor = (tab.container_cursor + 1).min(max);
                            AppAction::None
                        }
                        KeyCode::Enter if plain_shortcut(key) => tab
                            .containers
                            .get(tab.container_cursor)
                            .cloned()
                            .map(AppAction::ExecSelectContainer)
                            .unwrap_or(AppAction::None),
                        _ => AppAction::None,
                    }
                } else {
                    match key.code {
                        KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                        KeyCode::Enter if plain_shortcut(key) => AppAction::ExecSendInput,
                        KeyCode::Backspace => {
                            delete_char_left_at_cursor(&mut tab.input, &mut tab.input_cursor);
                            AppAction::None
                        }
                        KeyCode::Delete => {
                            delete_char_right_at_cursor(&mut tab.input, tab.input_cursor);
                            AppAction::None
                        }
                        KeyCode::Left => {
                            move_cursor_left(&mut tab.input_cursor);
                            AppAction::None
                        }
                        KeyCode::Right => {
                            move_cursor_right(&mut tab.input_cursor, &tab.input);
                            AppAction::None
                        }
                        KeyCode::Home => {
                            tab.input_cursor = 0;
                            AppAction::None
                        }
                        KeyCode::End => {
                            move_cursor_end(&mut tab.input_cursor, &tab.input);
                            AppAction::None
                        }
                        KeyCode::Up => {
                            tab.scroll = tab.scroll.saturating_sub(1);
                            AppAction::None
                        }
                        KeyCode::Down => {
                            tab.scroll = (tab.scroll + 1).min(tab.lines.len().saturating_sub(1));
                            AppAction::None
                        }
                        KeyCode::PageUp if plain_shortcut(key) => {
                            tab.scroll = tab.scroll.saturating_sub(10);
                            AppAction::None
                        }
                        KeyCode::PageDown if plain_shortcut(key) => {
                            tab.scroll = (tab.scroll + 10).min(tab.lines.len().saturating_sub(1));
                            AppAction::None
                        }
                        KeyCode::Char(c) if plain_shortcut(key) => {
                            insert_char_at_cursor(&mut tab.input, &mut tab.input_cursor, c);
                            AppAction::None
                        }
                        KeyCode::Char('u') if ctrl_shortcut(key) => {
                            clear_input_at_cursor(&mut tab.input, &mut tab.input_cursor);
                            AppAction::None
                        }
                        _ => AppAction::None,
                    }
                }
            }
            WorkbenchTabState::PortForward(tab) => match tab.dialog.handle_key(key) {
                PortForwardAction::None => AppAction::None,
                PortForwardAction::Refresh => AppAction::PortForwardRefresh,
                PortForwardAction::Close => AppAction::EscapePressed,
                PortForwardAction::Create(args) => AppAction::PortForwardCreate(args),
                PortForwardAction::Stop(tunnel_id) => AppAction::PortForwardStop(tunnel_id),
            },
            WorkbenchTabState::Relations(tab) => match key.code {
                KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    if !flat.is_empty() {
                        tab.cursor = (tab.cursor + 1).min(flat.len().saturating_sub(1));
                    }
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                    tab.cursor = tab.cursor.saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('g') if plain_shortcut(key) => {
                    tab.cursor = 0;
                    AppAction::None
                }
                KeyCode::Char('G') if plain_shortcut(key) => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    tab.cursor = flat.len().saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('l') | KeyCode::Right if plain_shortcut(key) => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    if let Some(node) = flat.get(tab.cursor)
                        && node.has_children
                        && !node.expanded
                    {
                        tab.expanded.insert(node.tree_index);
                        let flat =
                            crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                        tab.cursor = tab.cursor.min(flat.len().saturating_sub(1));
                    }
                    AppAction::None
                }
                KeyCode::Char('h') | KeyCode::Left if plain_shortcut(key) => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    if let Some(node) = flat.get(tab.cursor) {
                        if node.expanded {
                            tab.expanded.remove(&node.tree_index);
                            let flat =
                                crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                            tab.cursor = tab.cursor.min(flat.len().saturating_sub(1));
                        } else if tab.cursor > 0 {
                            for i in (0..tab.cursor).rev() {
                                if flat[i].depth < node.depth {
                                    tab.cursor = i;
                                    break;
                                }
                            }
                        }
                    }
                    AppAction::None
                }
                KeyCode::Enter if plain_shortcut(key) => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    if let Some(node) = flat.get(tab.cursor)
                        && let Some(resource) = &node.resource
                        && !node.not_found
                        && node.relation != crate::k8s::relationships::RelationKind::SectionHeader
                    {
                        return AppAction::OpenDetail(resource.clone());
                    }
                    AppAction::None
                }
                KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                _ => AppAction::None,
            },
            WorkbenchTabState::NetworkPolicy(tab) => match key.code {
                KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    if !flat.is_empty() {
                        tab.cursor = (tab.cursor + 1).min(flat.len().saturating_sub(1));
                    }
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                    tab.cursor = tab.cursor.saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('g') if plain_shortcut(key) => {
                    tab.cursor = 0;
                    AppAction::None
                }
                KeyCode::Char('G') if plain_shortcut(key) => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    tab.cursor = flat.len().saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('l') | KeyCode::Right if plain_shortcut(key) => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    if let Some(node) = flat.get(tab.cursor)
                        && node.has_children
                        && !node.expanded
                    {
                        tab.expanded.insert(node.tree_index);
                        // Re-clamp cursor after tree shape change.
                        let flat =
                            crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                        tab.cursor = tab.cursor.min(flat.len().saturating_sub(1));
                    }
                    AppAction::None
                }
                KeyCode::Char('h') | KeyCode::Left if plain_shortcut(key) => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    if let Some(node) = flat.get(tab.cursor) {
                        if node.expanded {
                            tab.expanded.remove(&node.tree_index);
                            // Re-clamp cursor after tree shape change.
                            let flat =
                                crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                            tab.cursor = tab.cursor.min(flat.len().saturating_sub(1));
                        } else if tab.cursor > 0 {
                            for i in (0..tab.cursor).rev() {
                                if flat[i].depth < node.depth {
                                    tab.cursor = i;
                                    break;
                                }
                            }
                        }
                    }
                    AppAction::None
                }
                KeyCode::Enter if plain_shortcut(key) => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    if let Some(node) = flat.get(tab.cursor)
                        && let Some(resource) = &node.resource
                        && !node.not_found
                        && node.relation != crate::k8s::relationships::RelationKind::SectionHeader
                    {
                        return AppAction::OpenDetail(resource.clone());
                    }
                    AppAction::None
                }
                KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                _ => AppAction::None,
            },
            WorkbenchTabState::TrafficDebug(tab) => match key.code {
                KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    if !flat.is_empty() {
                        tab.cursor = (tab.cursor + 1).min(flat.len().saturating_sub(1));
                    }
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                    tab.cursor = tab.cursor.saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('g') if plain_shortcut(key) => {
                    tab.cursor = 0;
                    AppAction::None
                }
                KeyCode::Char('G') if plain_shortcut(key) => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    tab.cursor = flat.len().saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('l') | KeyCode::Right if plain_shortcut(key) => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    if let Some(node) = flat.get(tab.cursor)
                        && node.has_children
                        && !node.expanded
                    {
                        tab.expanded.insert(node.tree_index);
                        let flat =
                            crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                        tab.cursor = tab.cursor.min(flat.len().saturating_sub(1));
                    }
                    AppAction::None
                }
                KeyCode::Char('h') | KeyCode::Left if plain_shortcut(key) => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    if let Some(node) = flat.get(tab.cursor) {
                        if node.expanded {
                            tab.expanded.remove(&node.tree_index);
                            let flat =
                                crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                            tab.cursor = tab.cursor.min(flat.len().saturating_sub(1));
                        } else if tab.cursor > 0 {
                            for i in (0..tab.cursor).rev() {
                                if flat[i].depth < node.depth {
                                    tab.cursor = i;
                                    break;
                                }
                            }
                        }
                    }
                    AppAction::None
                }
                KeyCode::Enter if plain_shortcut(key) => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    if let Some(node) = flat.get(tab.cursor)
                        && let Some(resource) = &node.resource
                        && !node.not_found
                        && node.relation != crate::k8s::relationships::RelationKind::SectionHeader
                    {
                        return AppAction::OpenDetail(resource.clone());
                    }
                    AppAction::None
                }
                KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                _ => AppAction::None,
            },
            WorkbenchTabState::Connectivity(tab) => match tab.focus {
                ConnectivityTabFocus::Filter => match key.code {
                    KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                    KeyCode::Tab if plain_shortcut(key) => {
                        tab.focus = ConnectivityTabFocus::Targets;
                        AppAction::None
                    }
                    KeyCode::BackTab if plain_shortcut(key) => {
                        tab.focus = ConnectivityTabFocus::Result;
                        AppAction::None
                    }
                    KeyCode::Backspace => {
                        tab.filter.backspace_char();
                        tab.refresh_filter();
                        AppAction::None
                    }
                    KeyCode::Delete => {
                        tab.filter.delete_char();
                        tab.refresh_filter();
                        AppAction::None
                    }
                    KeyCode::Left => {
                        tab.filter.cursor_left();
                        AppAction::None
                    }
                    KeyCode::Right => {
                        tab.filter.cursor_right();
                        AppAction::None
                    }
                    KeyCode::Home => {
                        tab.filter.cursor_home();
                        AppAction::None
                    }
                    KeyCode::End => {
                        tab.filter.cursor_end();
                        AppAction::None
                    }
                    KeyCode::Enter if plain_shortcut(key) => {
                        tab.focus = ConnectivityTabFocus::Targets;
                        AppAction::None
                    }
                    KeyCode::Char('u') if ctrl_shortcut(key) => {
                        tab.filter.clear();
                        tab.refresh_filter();
                        AppAction::None
                    }
                    KeyCode::Char(ch) if plain_shortcut(key) => {
                        tab.filter.add_char(ch);
                        tab.refresh_filter();
                        AppAction::None
                    }
                    _ => AppAction::None,
                },
                ConnectivityTabFocus::Targets => match key.code {
                    KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                    KeyCode::Tab if plain_shortcut(key) => {
                        tab.focus = ConnectivityTabFocus::Result;
                        AppAction::None
                    }
                    KeyCode::BackTab if plain_shortcut(key) => {
                        tab.focus = ConnectivityTabFocus::Filter;
                        AppAction::None
                    }
                    KeyCode::Char('/') if plain_shortcut(key) => {
                        tab.focus = ConnectivityTabFocus::Filter;
                        AppAction::None
                    }
                    KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                        tab.select_next_target();
                        AppAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                        tab.select_previous_target();
                        AppAction::None
                    }
                    KeyCode::Char('g') if plain_shortcut(key) => {
                        tab.select_top_target();
                        AppAction::None
                    }
                    KeyCode::Char('G') if plain_shortcut(key) => {
                        tab.select_bottom_target();
                        AppAction::None
                    }
                    KeyCode::Enter if plain_shortcut(key) => AppAction::OpenNetworkConnectivity,
                    _ => AppAction::None,
                },
                ConnectivityTabFocus::Result => match key.code {
                    KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                    KeyCode::Tab if plain_shortcut(key) => {
                        tab.focus = ConnectivityTabFocus::Filter;
                        AppAction::None
                    }
                    KeyCode::BackTab if plain_shortcut(key) => {
                        tab.focus = ConnectivityTabFocus::Targets;
                        AppAction::None
                    }
                    KeyCode::Char('/') if plain_shortcut(key) => {
                        tab.focus = ConnectivityTabFocus::Filter;
                        AppAction::None
                    }
                    KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                        let flat =
                            crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                        if !flat.is_empty() {
                            tab.tree_cursor =
                                (tab.tree_cursor + 1).min(flat.len().saturating_sub(1));
                        }
                        AppAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                        tab.tree_cursor = tab.tree_cursor.saturating_sub(1);
                        AppAction::None
                    }
                    KeyCode::Char('g') if plain_shortcut(key) => {
                        tab.tree_cursor = 0;
                        AppAction::None
                    }
                    KeyCode::Char('G') if plain_shortcut(key) => {
                        let flat =
                            crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                        tab.tree_cursor = flat.len().saturating_sub(1);
                        AppAction::None
                    }
                    KeyCode::Char('l') | KeyCode::Right if plain_shortcut(key) => {
                        let flat =
                            crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                        if let Some(node) = flat.get(tab.tree_cursor)
                            && node.has_children
                            && !node.expanded
                        {
                            tab.expanded.insert(node.tree_index);
                            let flat =
                                crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                            tab.tree_cursor = tab.tree_cursor.min(flat.len().saturating_sub(1));
                        }
                        AppAction::None
                    }
                    KeyCode::Char('h') | KeyCode::Left if plain_shortcut(key) => {
                        let flat =
                            crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                        if let Some(node) = flat.get(tab.tree_cursor) {
                            if node.expanded {
                                tab.expanded.remove(&node.tree_index);
                                let flat = crate::k8s::relationships::flatten_tree(
                                    &tab.tree,
                                    &tab.expanded,
                                );
                                tab.tree_cursor = tab.tree_cursor.min(flat.len().saturating_sub(1));
                            } else if tab.tree_cursor > 0 {
                                for i in (0..tab.tree_cursor).rev() {
                                    if flat[i].depth < node.depth {
                                        tab.tree_cursor = i;
                                        break;
                                    }
                                }
                            }
                        }
                        AppAction::None
                    }
                    KeyCode::Enter if plain_shortcut(key) => {
                        let flat =
                            crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                        if let Some(node) = flat.get(tab.tree_cursor)
                            && let Some(resource) = &node.resource
                            && !node.not_found
                            && node.relation
                                != crate::k8s::relationships::RelationKind::SectionHeader
                        {
                            return AppAction::OpenDetail(resource.clone());
                        }
                        AppAction::None
                    }
                    _ => AppAction::None,
                },
            },
        }
    }

    fn workbench_refresh_action(&self, key: KeyEvent) -> Option<AppAction> {
        if self.focus != Focus::Workbench
            || !self.workbench.open
            || self
                .detail_view
                .as_ref()
                .is_some_and(DetailViewState::has_confirmation_dialog)
        {
            return None;
        }

        let tab = self.workbench.active_tab()?;

        let allow_plain_r = match &tab.state {
            WorkbenchTabState::ActionHistory(_)
            | WorkbenchTabState::ResourceYaml(_)
            | WorkbenchTabState::ResourceDiff(_)
            | WorkbenchTabState::Rollout(crate::workbench::RolloutTabState {
                confirm_undo_revision: None,
                mutation_pending: None,
                ..
            })
            | WorkbenchTabState::DecodedSecret(crate::workbench::DecodedSecretTabState {
                editing: false,
                ..
            })
            | WorkbenchTabState::ResourceEvents(_)
            | WorkbenchTabState::Runbook(_)
            | WorkbenchTabState::Relations(_)
            | WorkbenchTabState::NetworkPolicy(_)
            | WorkbenchTabState::TrafficDebug(_) => true,
            WorkbenchTabState::AccessReview(tab) => {
                !matches!(tab.focus, AccessReviewFocus::SubjectInput)
            }
            WorkbenchTabState::Connectivity(tab) => tab.focus != ConnectivityTabFocus::Filter,
            WorkbenchTabState::PodLogs(tab) => {
                !tab.viewer.searching
                    && !tab.viewer.jumping_to_time
                    && !tab.viewer.picking_container
            }
            WorkbenchTabState::WorkloadLogs(tab) => {
                !tab.editing_text_filter && !tab.jumping_to_time
            }
            WorkbenchTabState::DecodedSecret(_) => false,
            WorkbenchTabState::HelmHistory(tab) => {
                !tab.rollback_pending && tab.confirm_rollback_revision.is_none()
            }
            WorkbenchTabState::Rollout(_) => false,
            WorkbenchTabState::Exec(_)
            | WorkbenchTabState::ExtensionOutput(_)
            | WorkbenchTabState::AiAnalysis(_)
            | WorkbenchTabState::PortForward(_) => false,
        };

        match key.code {
            KeyCode::Char('r') if allow_plain_r && plain_shortcut(key) => {
                Some(AppAction::RefreshData)
            }
            KeyCode::Char('R') if ctrl_shortcut(key) && allow_plain_r => {
                Some(AppAction::RefreshData)
            }
            _ => None,
        }
    }

    /// Routes a raw keyboard event to the appropriate handler and returns the resulting action.
    ///
    /// # Input routing priority (highest → lowest)
    ///
    /// 1. **Command palette** — when open, all keys are consumed by the palette.
    /// 2. **Context picker** — when open, all keys are consumed by the picker.
    /// 3. **Namespace picker** — when open, all keys are consumed by the picker.
    /// 4. **Search mode** — `/` activates it; `Esc`/`Enter` exits; all printable chars append to query.
    /// 5. **Active sub-component** (detail overlay):
    ///    - `LogsViewer`: `j`/`k` scroll lines, `g`/`G` jump to top/bottom, `f` toggles follow.
    ///    - `PortForward`: `Tab`/`BackTab` cycle fields, digits update port inputs.
    ///    - `Scale`: digits update replica count, `Backspace` deletes.
    ///    - `ProbePanel`: `j`/`k` select probe, `Space` toggles expand.
    /// 6. **Quit confirmation** — after root `Esc`, only `Enter` confirms; any other key cancels.
    /// 7. **Main navigation** (see table below).
    ///
    /// # Main navigation keys
    ///
    /// | Key | Condition | Effect |
    /// |-----|-----------|--------|
    /// | `q` | — | No-op at root |
    /// | `Esc` | detail view open | Close detail view |
    /// | `Esc` | `focus == Content` | Return focus to sidebar |
    /// | `Esc` | — | Enter quit confirmation |
    /// | `Tab` | — | Next view in [`AppView::ORDER`], sync sidebar cursor |
    /// | `Shift+Tab` | — | Previous view in [`AppView::ORDER`], sync sidebar cursor |
    /// | `j` / `↓` | no detail, `focus == Sidebar` | Move sidebar cursor down |
    /// | `j` / `↓` | no detail, `focus == Content` | Move content selection down |
    /// | `k` / `↑` | no detail, `focus == Sidebar` | Move sidebar cursor up |
    /// | `k` / `↑` | no detail, `focus == Content` | Move content selection up |
    /// | `n` | workload view, no detail | Sort by Name (toggle asc/desc on repeat) |
    /// | `a` | workload view, no detail | Sort by Age (toggle asc/desc on repeat) |
    /// | `1` | Pods view, no detail | Sort pods by Age (toggle asc/desc on repeat) |
    /// | `2` | Pods view, no detail | Sort pods by Status (toggle asc/desc on repeat) |
    /// | `3` | Pods view, no detail | Sort pods by Restarts (toggle asc/desc on repeat) |
    /// | `0` | workload view, no detail | Clear active sort and return to default order |
    /// | `/` | — | Enter search mode |
    /// | `~` | — | Open namespace picker |
    /// | `c` | no detail | Open context picker |
    /// | `:` | no detail | Open command palette |
    /// | `r` / `Ctrl+R` | — | Trigger data refresh |
    /// | `Shift+R` | Flux view or Flux detail | Reconcile selected Flux resource |
    ///
    /// `Enter` is **not** handled here — it is intercepted in `main.rs` before this method
    /// is called, because its behaviour depends on both `focus` and `detail_view`.
    pub fn handle_key_event(&mut self, key: KeyEvent) -> AppAction {
        if self.help_overlay.is_open() {
            return match key.code {
                KeyCode::Esc | KeyCode::Char('?') if plain_shortcut(key) => AppAction::CloseHelp,
                KeyCode::PageDown if plain_shortcut(key) => {
                    self.help_overlay.scroll_page_down();
                    AppAction::None
                }
                KeyCode::PageUp if plain_shortcut(key) => {
                    self.help_overlay.scroll_page_up();
                    AppAction::None
                }
                KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                    self.help_overlay.scroll_down();
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                    self.help_overlay.scroll_up();
                    AppAction::None
                }
                _ => AppAction::None,
            };
        }

        if self.command_palette.is_open() {
            return match self.command_palette.handle_key(key) {
                CommandPaletteAction::None => AppAction::None,
                CommandPaletteAction::Navigate(view) => AppAction::NavigateTo(view),
                CommandPaletteAction::JumpToResource(resource) => {
                    AppAction::JumpToResource(resource)
                }
                CommandPaletteAction::ActivateWorkbenchTab(key) => {
                    AppAction::ActivateWorkbenchTab(key)
                }
                CommandPaletteAction::Execute(action, resource) => {
                    AppAction::PaletteAction { action, resource }
                }
                CommandPaletteAction::ExecuteAi(id, resource) => {
                    AppAction::ExecuteAi { id, resource }
                }
                CommandPaletteAction::ExecuteExtension(id, resource) => {
                    AppAction::ExecuteExtension { id, resource }
                }
                CommandPaletteAction::OpenRunbook(id, resource) => {
                    AppAction::OpenRunbook { id, resource }
                }
                CommandPaletteAction::ToggleColumn(column_id) => {
                    self.toggle_column_visibility(&column_id);
                    AppAction::None
                }
                CommandPaletteAction::SaveWorkspace => AppAction::SaveWorkspace,
                CommandPaletteAction::ApplyWorkspace(name) => AppAction::ApplyWorkspace(name),
                CommandPaletteAction::ActivateWorkspaceBank(name) => {
                    AppAction::ActivateWorkspaceBank(name)
                }
                CommandPaletteAction::OpenTemplateDialog(kind) => {
                    AppAction::OpenResourceTemplateDialog(kind)
                }
                CommandPaletteAction::Close => AppAction::CloseCommandPalette,
            };
        }

        if let Some(dialog) = &mut self.resource_template_dialog {
            return match key.code {
                KeyCode::Esc if plain_shortcut(key) => {
                    self.resource_template_dialog = None;
                    AppAction::None
                }
                KeyCode::Enter
                    if dialog.focus_field
                        == crate::ui::components::ResourceTemplateField::CreateBtn
                        && plain_shortcut(key) =>
                {
                    AppAction::SubmitResourceTemplateDialog
                }
                KeyCode::Enter
                    if dialog.focus_field
                        == crate::ui::components::ResourceTemplateField::CancelBtn
                        && plain_shortcut(key) =>
                {
                    self.resource_template_dialog = None;
                    AppAction::None
                }
                KeyCode::Tab | KeyCode::Down if plain_shortcut(key) => {
                    dialog.next_field();
                    AppAction::None
                }
                KeyCode::BackTab | KeyCode::Up if plain_shortcut(key) => {
                    dialog.prev_field();
                    AppAction::None
                }
                KeyCode::Left => {
                    dialog.cursor_left();
                    AppAction::None
                }
                KeyCode::Right => {
                    dialog.cursor_right();
                    AppAction::None
                }
                KeyCode::Home => {
                    dialog.cursor_home();
                    AppAction::None
                }
                KeyCode::End => {
                    dialog.cursor_end();
                    AppAction::None
                }
                KeyCode::Backspace => {
                    dialog.backspace();
                    AppAction::None
                }
                KeyCode::Delete => {
                    dialog.delete_char();
                    AppAction::None
                }
                KeyCode::Char('u') if ctrl_shortcut(key) => {
                    dialog.clear_active();
                    AppAction::None
                }
                KeyCode::Char(c) if plain_shortcut(key) => {
                    dialog.add_char(c);
                    AppAction::None
                }
                _ => AppAction::None,
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

        if let Some(action) = self.workbench_refresh_action(key) {
            return action;
        }

        if let Some(action) = self.workbench_global_overlay_action(key) {
            return action;
        }

        if self.detail_view.is_none()
            && !self.confirm_quit
            && self.focus != Focus::Workbench
            && let Some(action) = self.matching_workspace_hotkey_action(key)
        {
            return action;
        }

        if self.focus == Focus::Workbench && self.workbench.open {
            return self.handle_workbench_key_event(key);
        }

        // Component-level routing priority:
        // Scale > ProbePanel > DetailView > MainView
        match self.active_component() {
            ActiveComponent::LogsViewer | ActiveComponent::PortForward => {
                return self.handle_workbench_key_event(key);
            }
            ActiveComponent::DebugContainer => {
                if let Some(detail) = &mut self.detail_view
                    && let Some(dialog) = &mut detail.debug_dialog
                {
                    return match dialog.handle_key(key) {
                        crate::ui::components::DebugContainerDialogEvent::None => AppAction::None,
                        crate::ui::components::DebugContainerDialogEvent::Submit => {
                            AppAction::DebugContainerDialogSubmit
                        }
                        crate::ui::components::DebugContainerDialogEvent::Close => {
                            detail.debug_dialog = None;
                            AppAction::None
                        }
                    };
                }
                return AppAction::None;
            }
            ActiveComponent::NodeDebug => {
                if let Some(detail) = &mut self.detail_view
                    && let Some(dialog) = &mut detail.node_debug_dialog
                {
                    return match dialog.handle_key(key) {
                        crate::ui::components::NodeDebugDialogEvent::None => AppAction::None,
                        crate::ui::components::NodeDebugDialogEvent::Submit => {
                            AppAction::NodeDebugDialogSubmit
                        }
                        crate::ui::components::NodeDebugDialogEvent::Close => {
                            detail.node_debug_dialog = None;
                            AppAction::None
                        }
                    };
                }
                return AppAction::None;
            }
            ActiveComponent::Scale => {
                let scale_focus = self
                    .detail_view
                    .as_ref()
                    .and_then(|detail| detail.scale_dialog.as_ref())
                    .map(|scale| scale.focus_field);
                return match key.code {
                    KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                    KeyCode::Enter
                        if plain_shortcut(key) && scale_focus == Some(ScaleField::CancelBtn) =>
                    {
                        AppAction::EscapePressed
                    }
                    KeyCode::Enter if plain_shortcut(key) => AppAction::ScaleDialogSubmit,
                    KeyCode::Tab if plain_shortcut(key) => AppAction::ScaleDialogNextField,
                    KeyCode::BackTab if plain_shortcut(key) => AppAction::ScaleDialogPrevField,
                    KeyCode::Backspace => AppAction::ScaleDialogBackspace,
                    KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Up
                        if plain_shortcut(key) =>
                    {
                        AppAction::ScaleDialogIncrement
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') | KeyCode::Down
                        if plain_shortcut(key) =>
                    {
                        AppAction::ScaleDialogDecrement
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() && plain_shortcut(key) => {
                        AppAction::ScaleDialogUpdateInput(c)
                    }
                    _ => AppAction::None,
                };
            }
            ActiveComponent::ProbePanel => {
                return match key.code {
                    KeyCode::Esc if plain_shortcut(key) => AppAction::EscapePressed,
                    KeyCode::Enter | KeyCode::Char(' ') if plain_shortcut(key) => {
                        AppAction::ProbeToggleExpand
                    }
                    KeyCode::Char('j') | KeyCode::Down if plain_shortcut(key) => {
                        AppAction::ProbeSelectNext
                    }
                    KeyCode::Char('k') | KeyCode::Up if plain_shortcut(key) => {
                        AppAction::ProbeSelectPrev
                    }
                    _ => AppAction::None,
                };
            }
            ActiveComponent::None => {}
        }

        if self.confirm_quit {
            return match key.code {
                KeyCode::Enter if plain_shortcut(key) => {
                    self.should_quit = true;
                    AppAction::Quit
                }
                _ => {
                    self.confirm_quit = false;
                    AppAction::None
                }
            };
        }

        if let Some(action) = self.handle_detail_confirmation_key(key) {
            return action;
        }

        match key.code {
            KeyCode::Char('q') => AppAction::None,
            KeyCode::Esc
                if self
                    .detail_view
                    .as_ref()
                    .map(|d| d.confirm_delete)
                    .unwrap_or(false)
                    && plain_shortcut(key) =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.confirm_delete = false;
                }
                AppAction::None
            }
            KeyCode::Esc
                if self
                    .detail_view
                    .as_ref()
                    .map(|d| d.confirm_drain)
                    .unwrap_or(false)
                    && plain_shortcut(key) =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.confirm_drain = false;
                }
                AppAction::None
            }
            KeyCode::Esc
                if self
                    .detail_view
                    .as_ref()
                    .and_then(|d| d.confirm_cronjob_suspend)
                    .is_some()
                    && plain_shortcut(key) =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.confirm_cronjob_suspend = None;
                }
                AppAction::None
            }
            KeyCode::Esc if self.detail_view.is_some() && plain_shortcut(key) => {
                AppAction::CloseDetail
            }
            KeyCode::Esc if self.focus == Focus::Content && plain_shortcut(key) => {
                self.focus = Focus::Sidebar;
                AppAction::None
            }
            KeyCode::Esc if self.focus == Focus::Workbench && plain_shortcut(key) => {
                self.focus = Focus::Content;
                AppAction::None
            }
            KeyCode::Esc if plain_shortcut(key) => {
                self.confirm_quit = true;
                AppAction::None
            }
            KeyCode::Char('l') | KeyCode::Char('L')
                if plain_shortcut(key)
                    && (self
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::Logs))
                        || (self.detail_view.is_none()
                            && self.focus == Focus::Content
                            && view_supports_logs_shortcut(self.view))) =>
            {
                AppAction::LogsViewerOpen
            }
            KeyCode::Char('y') | KeyCode::Char('Y')
                if copy_resource_name_shortcut(key)
                    && app_supports_selected_resource_action_shortcut(self) =>
            {
                AppAction::CopyResourceName
            }
            KeyCode::Char('y')
                if plain_shortcut(key)
                    && (self.detail_view.as_ref().is_some_and(|detail| {
                        detail.supports_action(DetailAction::ViewYaml)
                            && !detail.has_confirmation_dialog()
                    }) || (self.detail_view.is_none()
                        && self.focus == Focus::Content
                        && view_supports_selected_resource_shortcut(
                            self.view,
                            self.extension_in_instances,
                        ))) =>
            {
                AppAction::OpenResourceYaml
            }
            KeyCode::Char('D')
                if plain_shortcut(key)
                    && self.detail_view.as_ref().is_some_and(|detail| {
                        detail.supports_action(DetailAction::ViewConfigDrift)
                            && !detail.supports_action(DetailAction::Drain)
                            && !detail.has_confirmation_dialog()
                    }) =>
            {
                AppAction::OpenResourceDiff
            }
            KeyCode::Char('O')
                if plain_shortcut(key)
                    && self.detail_view.as_ref().is_some_and(|detail| {
                        detail.supports_action(DetailAction::ViewRollout)
                            && !detail.has_confirmation_dialog()
                    }) =>
            {
                AppAction::OpenRollout
            }
            KeyCode::Char('h')
                if plain_shortcut(key)
                    && (self.detail_view.as_ref().is_some_and(|detail| {
                        detail.supports_action(DetailAction::ViewHelmHistory)
                            && !detail.has_confirmation_dialog()
                    }) || (self.detail_view.is_none()
                        && self.focus == Focus::Content
                        && self.view == AppView::HelmReleases)) =>
            {
                AppAction::OpenHelmHistory
            }
            KeyCode::Char('A')
                if plain_shortcut(key)
                    && (self.detail_view.as_ref().is_some_and(|detail| {
                        detail.supports_action(DetailAction::ViewAccessReview)
                    }) || (self.detail_view.is_none()
                        && self.focus == Focus::Content
                        && view_supports_selected_resource_shortcut(
                            self.view,
                            self.extension_in_instances,
                        )))
                    && !self
                        .detail_view
                        .as_ref()
                        .is_some_and(DetailViewState::has_confirmation_dialog) =>
            {
                AppAction::OpenAccessReview
            }
            KeyCode::Char('N')
                if plain_shortcut(key)
                    && self.detail_view.as_ref().is_some_and(|detail| {
                        detail.supports_action(DetailAction::ViewNetworkPolicies)
                    })
                    && !self
                        .detail_view
                        .as_ref()
                        .is_some_and(DetailViewState::has_confirmation_dialog) =>
            {
                AppAction::OpenNetworkPolicyView
            }
            KeyCode::Char('C')
                if plain_shortcut(key)
                    && self.detail_view.as_ref().is_some_and(|detail| {
                        detail.supports_action(DetailAction::CheckNetworkConnectivity)
                    })
                    && !self
                        .detail_view
                        .as_ref()
                        .is_some_and(DetailViewState::has_confirmation_dialog) =>
            {
                AppAction::OpenNetworkConnectivity
            }
            KeyCode::Char('t')
                if plain_shortcut(key)
                    && (self.detail_view.as_ref().is_some_and(|detail| {
                        detail.supports_action(DetailAction::ViewTrafficDebug)
                    }) || (self.detail_view.is_none()
                        && self.focus == Focus::Content
                        && matches!(
                            self.view,
                            AppView::Services
                                | AppView::Endpoints
                                | AppView::Ingresses
                                | AppView::Pods
                        )))
                    && !self
                        .detail_view
                        .as_ref()
                        .is_some_and(DetailViewState::has_confirmation_dialog) =>
            {
                AppAction::OpenTrafficDebug
            }
            KeyCode::Char('o')
                if plain_shortcut(key)
                    && (self.detail_view.as_ref().is_some_and(|detail| {
                        detail.supports_action(DetailAction::ViewDecodedSecret)
                    }) || (self.detail_view.is_none()
                        && self.focus == Focus::Content
                        && self.view == AppView::Secrets)) =>
            {
                AppAction::OpenDecodedSecret
            }
            KeyCode::Char('B')
                if plain_shortcut(key) && app_supports_selected_resource_action_shortcut(self) =>
            {
                AppAction::ToggleBookmark
            }
            KeyCode::Char('Y')
                if plain_shortcut(key) && app_supports_selected_resource_action_shortcut(self) =>
            {
                AppAction::CopyResourceFullName
            }
            KeyCode::Char('v')
                if plain_shortcut(key)
                    && (self.detail_view.as_ref().is_some_and(|detail| {
                        detail.supports_action(DetailAction::ViewEvents)
                    }) || (self.detail_view.is_none()
                        && self.focus == Focus::Content
                        && view_supports_resource_events_shortcut(self.view))) =>
            {
                AppAction::OpenResourceEvents
            }
            KeyCode::Char('H')
                if plain_shortcut(key)
                    && !self
                        .detail_view
                        .as_ref()
                        .is_some_and(DetailViewState::has_confirmation_dialog) =>
            {
                AppAction::OpenActionHistory
            }
            KeyCode::Char('x')
                if plain_shortcut(key)
                    && (self
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::Exec))
                        || (self.detail_view.is_none()
                            && self.focus == Focus::Content
                            && view_supports_pod_only_shortcut(self.view))) =>
            {
                AppAction::OpenExec
            }
            KeyCode::Char('g')
                if plain_shortcut(key)
                    && self.detail_view.as_ref().is_some_and(|detail| {
                        detail.supports_action(DetailAction::DebugContainer)
                            || detail.supports_action(DetailAction::NodeDebugShell)
                    }) =>
            {
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::NodeDebugShell))
                {
                    AppAction::NodeDebugDialogOpen
                } else {
                    AppAction::DebugContainerDialogOpen
                }
            }
            KeyCode::Char('f') | KeyCode::Char('F')
                if self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && ctrl_shortcut(key)
                    && view_supports_content_detail_scroll(self.view) =>
            {
                self.content_detail_scroll = self.content_detail_scroll.saturating_add(10);
                AppAction::None
            }
            KeyCode::Char('f')
                if plain_shortcut(key)
                    && (self.detail_view.as_ref().is_some_and(|detail| {
                        detail.supports_action(DetailAction::PortForward)
                    }) || (self.detail_view.is_none()
                        && self.focus == Focus::Content
                        && view_supports_pod_only_shortcut(self.view))) =>
            {
                AppAction::PortForwardOpen
            }
            KeyCode::Char('s')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Scale))
                    && plain_shortcut(key) =>
            {
                AppAction::ScaleDialogOpen
            }
            KeyCode::Char('p')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Probes))
                    && plain_shortcut(key) =>
            {
                AppAction::ProbePanelOpen
            }
            KeyCode::Char('R') if self.detail_view.is_some() && plain_shortcut(key) => {
                match self.detail_view.as_ref() {
                    Some(detail) if detail.supports_action(DetailAction::Restart) => {
                        AppAction::RolloutRestart
                    }
                    Some(detail) if detail.supports_action(DetailAction::FluxReconcile) => {
                        AppAction::FluxReconcile
                    }
                    _ => AppAction::None,
                }
            }
            KeyCode::Char('e')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::EditYaml))
                    && plain_shortcut(key) =>
            {
                AppAction::EditYaml
            }
            KeyCode::Char('m') if self.detail_view.is_some() && plain_shortcut(key) => {
                AppAction::ToggleDetailMetadata
            }
            KeyCode::Char('d')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Delete))
                    && plain_shortcut(key) =>
            {
                // Toggle delete confirmation prompt
                if let Some(detail) = &mut self.detail_view {
                    detail.confirm_delete = true;
                }
                AppAction::None
            }
            KeyCode::Char('F')
                if self
                    .detail_view
                    .as_ref()
                    .map(|d| d.confirm_drain)
                    .unwrap_or(false) =>
            {
                AppAction::ForceDrainNode
            }
            KeyCode::Char('D') | KeyCode::Char('y') | KeyCode::Enter
                if self
                    .detail_view
                    .as_ref()
                    .map(|d| d.confirm_drain)
                    .unwrap_or(false) =>
            {
                AppAction::DrainNode
            }
            KeyCode::Char('F')
                if self
                    .detail_view
                    .as_ref()
                    .map(|d| d.confirm_delete)
                    .unwrap_or(false) =>
            {
                AppAction::ForceDeleteResource
            }
            KeyCode::Char('D') | KeyCode::Char('d') | KeyCode::Char('y') | KeyCode::Enter
                if self
                    .detail_view
                    .as_ref()
                    .map(|d| d.confirm_delete)
                    .unwrap_or(false) =>
            {
                AppAction::DeleteResource
            }
            KeyCode::Char('S') | KeyCode::Char('y') | KeyCode::Enter
                if self
                    .detail_view
                    .as_ref()
                    .and_then(|d| d.confirm_cronjob_suspend)
                    .is_some() =>
            {
                AppAction::SetCronJobSuspend(
                    self.detail_view
                        .as_ref()
                        .and_then(|detail| detail.confirm_cronjob_suspend)
                        .unwrap_or(false),
                )
            }
            KeyCode::Enter
                if self
                    .detail_view
                    .as_ref()
                    .filter(|detail| !detail.has_confirmation_dialog())
                    .and_then(DetailViewState::selected_detail_resource)
                    .is_some() =>
            {
                self.detail_view
                    .as_ref()
                    .filter(|detail| !detail.has_confirmation_dialog())
                    .and_then(DetailViewState::selected_detail_resource)
                    .map(AppAction::OpenDetail)
                    .unwrap_or(AppAction::None)
            }
            KeyCode::Char('j') | KeyCode::Char('J') | KeyCode::Down
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| !detail.has_confirmation_dialog())
                    && ctrl_shortcut(key) =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.scroll_top_panels_down(1);
                }
                AppAction::None
            }
            KeyCode::Char('j') | KeyCode::Char('J') | KeyCode::Down
                if self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && ctrl_shortcut(key)
                    && view_supports_content_detail_scroll(self.view) =>
            {
                self.content_detail_scroll = self.content_detail_scroll.saturating_add(1);
                AppAction::None
            }
            KeyCode::Char('k') | KeyCode::Char('K') | KeyCode::Up
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| !detail.has_confirmation_dialog())
                    && ctrl_shortcut(key) =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.scroll_top_panels_up(1);
                }
                AppAction::None
            }
            KeyCode::Char('k') | KeyCode::Char('K') | KeyCode::Up
                if self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && ctrl_shortcut(key)
                    && view_supports_content_detail_scroll(self.view) =>
            {
                self.content_detail_scroll = self.content_detail_scroll.saturating_sub(1);
                AppAction::None
            }
            KeyCode::Char('d') | KeyCode::Char('D')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| !detail.has_confirmation_dialog())
                    && ctrl_shortcut(key) =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.scroll_top_panels_down(10);
                }
                AppAction::None
            }
            KeyCode::Char('d') | KeyCode::Char('D')
                if self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && ctrl_shortcut(key)
                    && view_supports_content_detail_scroll(self.view) =>
            {
                self.content_detail_scroll = self.content_detail_scroll.saturating_add(10);
                AppAction::None
            }
            KeyCode::Char('u') | KeyCode::Char('U')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| !detail.has_confirmation_dialog())
                    && ctrl_shortcut(key) =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.scroll_top_panels_up(10);
                }
                AppAction::None
            }
            KeyCode::Char('u') | KeyCode::Char('U')
                if self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && ctrl_shortcut(key)
                    && view_supports_content_detail_scroll(self.view) =>
            {
                self.content_detail_scroll = self.content_detail_scroll.saturating_sub(10);
                AppAction::None
            }
            KeyCode::Char('b') | KeyCode::Char('B')
                if self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && ctrl_shortcut(key)
                    && view_supports_content_detail_scroll(self.view) =>
            {
                self.content_detail_scroll = self.content_detail_scroll.saturating_sub(10);
                AppAction::None
            }
            KeyCode::PageDown
                if self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && view_supports_content_detail_scroll(self.view) =>
            {
                self.content_detail_scroll = self.content_detail_scroll.saturating_add(10);
                AppAction::None
            }
            KeyCode::PageUp
                if self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && view_supports_content_detail_scroll(self.view) =>
            {
                self.content_detail_scroll = self.content_detail_scroll.saturating_sub(10);
                AppAction::None
            }
            KeyCode::Char(';')
                if self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && plain_shortcut(key)
                    && view_supports_content_detail_scroll(self.view) =>
            {
                self.content_pane_focus = match self.content_pane_focus() {
                    ContentPaneFocus::List => ContentPaneFocus::Secondary,
                    ContentPaneFocus::Secondary => ContentPaneFocus::List,
                };
                AppAction::None
            }
            KeyCode::Char('j') | KeyCode::Down
                if self.content_secondary_pane_active() && plain_shortcut(key) =>
            {
                self.content_detail_scroll = self.content_detail_scroll.saturating_add(1);
                AppAction::None
            }
            KeyCode::Char('k') | KeyCode::Up
                if self.content_secondary_pane_active() && plain_shortcut(key) =>
            {
                self.content_detail_scroll = self.content_detail_scroll.saturating_sub(1);
                AppAction::None
            }
            KeyCode::Char('d') if self.content_secondary_pane_active() && plain_shortcut(key) => {
                self.content_detail_scroll = self.content_detail_scroll.saturating_add(10);
                AppAction::None
            }
            KeyCode::Char('u') if self.content_secondary_pane_active() && plain_shortcut(key) => {
                self.content_detail_scroll = self.content_detail_scroll.saturating_sub(10);
                AppAction::None
            }
            KeyCode::Char('j') | KeyCode::Down
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| !detail.has_confirmation_dialog())
                    && plain_shortcut(key) =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.select_next_cronjob_history();
                }
                AppAction::None
            }
            KeyCode::Char('k') | KeyCode::Up
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| !detail.has_confirmation_dialog())
                    && plain_shortcut(key) =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.select_prev_cronjob_history();
                }
                AppAction::None
            }
            KeyCode::Tab if self.detail_view.is_none() && plain_shortcut(key) => {
                self.next_view();
                AppAction::None
            }
            KeyCode::BackTab if self.detail_view.is_none() && plain_shortcut(key) => {
                self.previous_view();
                AppAction::None
            }
            KeyCode::Char('j') | KeyCode::Down
                if self.detail_view.is_none() && plain_shortcut(key) =>
            {
                match self.focus {
                    Focus::Sidebar => self.sidebar_cursor_down(),
                    Focus::Content
                        if self.view == AppView::Extensions && self.extension_in_instances =>
                    {
                        if !self.extension_instances.is_empty() {
                            self.extension_instance_cursor = (self.extension_instance_cursor + 1)
                                % self.extension_instances.len();
                        }
                    }
                    Focus::Content => self.select_next(),
                    Focus::Workbench => {}
                }
                AppAction::None
            }
            KeyCode::Char('k') | KeyCode::Up
                if self.detail_view.is_none() && plain_shortcut(key) =>
            {
                match self.focus {
                    Focus::Sidebar => self.sidebar_cursor_up(),
                    Focus::Content
                        if self.view == AppView::Extensions && self.extension_in_instances =>
                    {
                        if !self.extension_instances.is_empty() {
                            self.extension_instance_cursor = if self.extension_instance_cursor == 0
                            {
                                self.extension_instances.len() - 1
                            } else {
                                self.extension_instance_cursor - 1
                            };
                        }
                    }
                    Focus::Content => self.select_previous(),
                    Focus::Workbench => {}
                }
                AppAction::None
            }
            KeyCode::Down if self.detail_view.is_none() && plain_shortcut(key) => {
                self.select_next();
                AppAction::None
            }
            KeyCode::Up if self.detail_view.is_none() && plain_shortcut(key) => {
                self.select_previous();
                AppAction::None
            }
            KeyCode::Char('n')
                if self.detail_view.is_none()
                    && self.view == AppView::Pods
                    && plain_shortcut(key) =>
            {
                self.set_or_toggle_pod_sort(PodSortColumn::Name);
                AppAction::None
            }
            KeyCode::Char('n')
                if self.detail_view.is_none()
                    && self.view.supports_shared_sort(WorkloadSortColumn::Name)
                    && plain_shortcut(key) =>
            {
                self.set_or_toggle_workload_sort(WorkloadSortColumn::Name);
                AppAction::None
            }
            KeyCode::Char('a')
                if self.detail_view.is_none()
                    && self.view == AppView::Pods
                    && plain_shortcut(key) =>
            {
                self.set_or_toggle_pod_sort(PodSortColumn::Age);
                AppAction::None
            }
            KeyCode::Char('a')
                if self.detail_view.is_none()
                    && self.view.supports_shared_sort(WorkloadSortColumn::Age)
                    && plain_shortcut(key) =>
            {
                self.set_or_toggle_workload_sort(WorkloadSortColumn::Age);
                AppAction::None
            }
            KeyCode::Char('1')
                if self.detail_view.is_none()
                    && self.view == AppView::Pods
                    && plain_shortcut(key) =>
            {
                self.set_or_toggle_pod_sort(PodSortColumn::Age);
                AppAction::None
            }
            KeyCode::Char('1')
                if self.detail_view.is_none()
                    && self.view.supports_shared_sort(WorkloadSortColumn::Age)
                    && plain_shortcut(key) =>
            {
                self.set_or_toggle_workload_sort(WorkloadSortColumn::Age);
                AppAction::None
            }
            KeyCode::Char('2')
                if self.detail_view.is_none()
                    && self.view == AppView::Pods
                    && plain_shortcut(key) =>
            {
                self.set_or_toggle_pod_sort(PodSortColumn::Status);
                AppAction::None
            }
            KeyCode::Char('3')
                if self.detail_view.is_none()
                    && self.view == AppView::Pods
                    && plain_shortcut(key) =>
            {
                self.set_or_toggle_pod_sort(PodSortColumn::Restarts);
                AppAction::None
            }
            KeyCode::Char('0')
                if self.detail_view.is_none()
                    && self.view == AppView::Pods
                    && plain_shortcut(key) =>
            {
                self.clear_pod_sort();
                AppAction::None
            }
            KeyCode::Char('0')
                if self.detail_view.is_none()
                    && !self.view.shared_sort_capabilities().is_empty()
                    && plain_shortcut(key) =>
            {
                self.clear_workload_sort();
                AppAction::None
            }
            KeyCode::Char('/') if plain_shortcut(key) => {
                self.is_search_mode = true;
                move_cursor_end(&mut self.search_cursor, &self.search_query);
                AppAction::None
            }
            KeyCode::Char('~') if plain_shortcut(key) => AppAction::OpenNamespacePicker,
            KeyCode::Char('W') if self.detail_view.is_none() && plain_shortcut(key) => {
                AppAction::SaveWorkspace
            }
            KeyCode::Char('{') if self.detail_view.is_none() && plain_shortcut(key) => {
                AppAction::ApplyPreviousWorkspace
            }
            KeyCode::Char('}') if self.detail_view.is_none() && plain_shortcut(key) => {
                AppAction::ApplyNextWorkspace
            }
            KeyCode::Char('b') if self.detail_view.is_none() && plain_shortcut(key) => {
                AppAction::ToggleWorkbench
            }
            KeyCode::Char(',')
                if self.detail_view.is_none() && self.workbench.open && plain_shortcut(key) =>
            {
                AppAction::WorkbenchPreviousTab
            }
            KeyCode::Char('.')
                if self.detail_view.is_none() && self.workbench.open && plain_shortcut(key) =>
            {
                AppAction::WorkbenchNextTab
            }
            KeyCode::Char('w')
                if self.detail_view.is_none() && self.workbench.open && ctrl_shortcut(key) =>
            {
                AppAction::WorkbenchCloseActiveTab
            }
            KeyCode::Up
                if self.detail_view.is_none() && self.workbench.open && ctrl_shortcut(key) =>
            {
                AppAction::WorkbenchIncreaseHeight
            }
            KeyCode::Down
                if self.detail_view.is_none() && self.workbench.open && ctrl_shortcut(key) =>
            {
                AppAction::WorkbenchDecreaseHeight
            }
            KeyCode::Char('c') if self.detail_view.is_none() && plain_shortcut(key) => {
                AppAction::OpenContextPicker
            }
            KeyCode::Char(':')
                if plain_shortcut(key)
                    && !self
                        .detail_view
                        .as_ref()
                        .is_some_and(DetailViewState::has_confirmation_dialog) =>
            {
                AppAction::OpenCommandPalette
            }
            KeyCode::Char('R')
                if self.detail_view.is_none()
                    && self
                        .view
                        .supports_view_action(ViewAction::SelectedFluxReconcile)
                    && plain_shortcut(key) =>
            {
                AppAction::FluxReconcile
            }
            KeyCode::Char('r')
                if plain_shortcut(key)
                    && !self
                        .detail_view
                        .as_ref()
                        .is_some_and(DetailViewState::has_confirmation_dialog) =>
            {
                AppAction::RefreshData
            }
            KeyCode::Char('R')
                if ctrl_shortcut(key)
                    && !self
                        .detail_view
                        .as_ref()
                        .is_some_and(DetailViewState::has_confirmation_dialog) =>
            {
                AppAction::RefreshData
            }
            KeyCode::Char('w')
                if self.detail_view.as_ref().is_some_and(|detail| {
                    detail.supports_action(DetailAction::ViewRelationships)
                }) && plain_shortcut(key) =>
            {
                AppAction::OpenRelationships
            }
            KeyCode::Char('T')
                if plain_shortcut(key)
                    && self
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::Trigger)) =>
            {
                AppAction::TriggerCronJob
            }
            KeyCode::Char('S')
                if plain_shortcut(key)
                    && self.detail_view.as_ref().is_some_and(|detail| {
                        detail.supports_action(DetailAction::SuspendCronJob)
                            || detail.supports_action(DetailAction::ResumeCronJob)
                    }) =>
            {
                AppAction::ConfirmCronJobSuspend(
                    self.detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::SuspendCronJob)),
                )
            }
            KeyCode::Char('c')
                if plain_shortcut(key)
                    && self
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::Cordon)) =>
            {
                AppAction::CordonNode
            }
            KeyCode::Char('u')
                if plain_shortcut(key)
                    && self
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::Uncordon)) =>
            {
                AppAction::UncordonNode
            }
            KeyCode::Char('D')
                if plain_shortcut(key)
                    && self
                        .detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.supports_action(DetailAction::Drain)) =>
            {
                // Open drain confirmation prompt
                if let Some(detail) = &mut self.detail_view {
                    detail.confirm_drain = true;
                }
                AppAction::None
            }
            KeyCode::Char('T') if self.detail_view.is_none() && plain_shortcut(key) => {
                AppAction::CycleTheme
            }
            KeyCode::Char('I') if self.detail_view.is_none() && plain_shortcut(key) => {
                AppAction::CycleIconMode
            }
            KeyCode::Char('?')
                if plain_shortcut(key)
                    && !self
                        .detail_view
                        .as_ref()
                        .is_some_and(DetailViewState::has_confirmation_dialog) =>
            {
                AppAction::OpenHelp
            }
            _ => AppAction::None,
        }
    }

    fn handle_search_input(&mut self, key: KeyEvent) -> AppAction {
        let previous_query = self.search_query.clone();

        match key.code {
            KeyCode::Esc if plain_shortcut(key) => {
                self.clear_search_query();
                self.is_search_mode = false;
                // Reset selection so the user doesn't land on a stale filtered index.
                self.selected_idx = 0;
                self.detail_view = None;
                self.clear_selection_search_status();
            }
            KeyCode::Enter if plain_shortcut(key) => {
                self.is_search_mode = false;
            }
            KeyCode::Backspace => {
                delete_char_left_at_cursor(&mut self.search_query, &mut self.search_cursor);
            }
            KeyCode::Delete => {
                delete_char_right_at_cursor(&mut self.search_query, self.search_cursor);
            }
            KeyCode::Left => {
                move_cursor_left(&mut self.search_cursor);
            }
            KeyCode::Right => {
                move_cursor_right(&mut self.search_cursor, &self.search_query);
            }
            KeyCode::Home => {
                move_cursor_home(&mut self.search_cursor);
            }
            KeyCode::End => {
                move_cursor_end(&mut self.search_cursor, &self.search_query);
            }
            KeyCode::Char('u') if ctrl_shortcut(key) => {
                self.clear_search_query();
            }
            KeyCode::Char(c) if plain_shortcut(key) => {
                insert_char_at_cursor(&mut self.search_query, &mut self.search_cursor, c);
            }
            _ => {}
        }
        if self.search_query != previous_query {
            self.selected_idx = 0;
            self.detail_view = None;
            self.clear_selection_search_status();
        }
        AppAction::None
    }
}
