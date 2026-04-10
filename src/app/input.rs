//! Keyboard input handling for AppState.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::views::AppView;
use super::{
    ActiveComponent, AppAction, AppState, DetailViewState, Focus, PodSortColumn, WorkloadSortColumn,
};
use crate::{
    policy::{DetailAction, ViewAction},
    ui::components::{CommandPaletteAction, ContextPickerAction, NamespacePickerAction},
    workbench::{AccessReviewFocus, ConnectivityTabFocus, WorkbenchTabState},
};

impl AppState {
    fn handle_workbench_key_event(&mut self, key: KeyEvent) -> AppAction {
        use crate::ui::components::port_forward_dialog::PortForwardAction;

        let access_review_input_active = self.workbench.active_tab().is_some_and(|tab| {
            matches!(
                &tab.state,
                WorkbenchTabState::AccessReview(tab)
                    if matches!(tab.focus, AccessReviewFocus::SubjectInput)
            )
        });

        // Common workbench keys (apply to all tab types)
        if !access_review_input_active && key.code == KeyCode::Char('z') {
            return AppAction::WorkbenchToggleMaximize;
        }
        if !access_review_input_active && key.code == KeyCode::Char('b') {
            return AppAction::ToggleWorkbench;
        }

        let action_history_len = self.visible_action_history_entries().len();
        let Some(tab) = self.workbench.active_tab_mut() else {
            return AppAction::None;
        };

        match &mut tab.state {
            WorkbenchTabState::ActionHistory(tab) => match key.code {
                KeyCode::Esc => AppAction::EscapePressed,
                KeyCode::Char('j') | KeyCode::Down => {
                    tab.select_next(action_history_len);
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    tab.select_previous();
                    AppAction::None
                }
                KeyCode::Char('g') => {
                    tab.select_top();
                    AppAction::None
                }
                KeyCode::Char('G') => {
                    tab.select_bottom(action_history_len);
                    AppAction::None
                }
                KeyCode::PageDown => {
                    for _ in 0..10 {
                        tab.select_next(action_history_len);
                    }
                    AppAction::None
                }
                KeyCode::PageUp => {
                    for _ in 0..10 {
                        tab.select_previous();
                    }
                    AppAction::None
                }
                KeyCode::Enter => AppAction::ActionHistoryOpenSelected,
                _ => AppAction::None,
            },
            WorkbenchTabState::AccessReview(tab) => {
                let max_scroll = tab.line_count().saturating_sub(1);
                match tab.focus {
                    AccessReviewFocus::Summary => match key.code {
                        KeyCode::Esc => AppAction::EscapePressed,
                        KeyCode::Tab | KeyCode::Char('s') | KeyCode::Char('/') => {
                            tab.start_subject_input();
                            AppAction::None
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            tab.scroll = tab.scroll.saturating_add(1).min(max_scroll);
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            tab.scroll = tab.scroll.saturating_sub(1);
                            AppAction::None
                        }
                        KeyCode::Char('g') => {
                            tab.scroll = 0;
                            AppAction::None
                        }
                        KeyCode::Char('G') => {
                            tab.scroll = max_scroll;
                            AppAction::None
                        }
                        KeyCode::PageDown => {
                            tab.scroll = tab.scroll.saturating_add(10).min(max_scroll);
                            AppAction::None
                        }
                        KeyCode::PageUp => {
                            tab.scroll = tab.scroll.saturating_sub(10);
                            AppAction::None
                        }
                        _ => AppAction::None,
                    },
                    AccessReviewFocus::SubjectInput => match key.code {
                        KeyCode::Esc => {
                            tab.stop_subject_input();
                            AppAction::None
                        }
                        KeyCode::Tab | KeyCode::BackTab => {
                            tab.stop_subject_input();
                            AppAction::None
                        }
                        KeyCode::Backspace | KeyCode::Delete => {
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
                        KeyCode::Enter => AppAction::ApplyAccessReviewSubject,
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            tab.subject_input.clear();
                            tab.subject_input_error = None;
                            AppAction::None
                        }
                        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
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
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Char('j') | KeyCode::Down => {
                        tab.scroll = tab.scroll.saturating_add(1).min(max_scroll);
                        AppAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        tab.scroll = tab.scroll.saturating_sub(1);
                        AppAction::None
                    }
                    KeyCode::Char('g') => {
                        tab.scroll = 0;
                        AppAction::None
                    }
                    KeyCode::Char('G') => {
                        tab.scroll = max_scroll;
                        AppAction::None
                    }
                    KeyCode::PageDown => {
                        tab.scroll = tab.scroll.saturating_add(10).min(max_scroll);
                        AppAction::None
                    }
                    KeyCode::PageUp => {
                        tab.scroll = tab.scroll.saturating_sub(10);
                        AppAction::None
                    }
                    _ => AppAction::None,
                }
            }
            WorkbenchTabState::ResourceDiff(tab) => {
                let max_scroll = tab.lines.len().saturating_sub(1);
                match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Char('j') | KeyCode::Down => {
                        tab.scroll = tab.scroll.saturating_add(1).min(max_scroll);
                        AppAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        tab.scroll = tab.scroll.saturating_sub(1);
                        AppAction::None
                    }
                    KeyCode::Char('g') => {
                        tab.scroll = 0;
                        AppAction::None
                    }
                    KeyCode::Char('G') => {
                        tab.scroll = max_scroll;
                        AppAction::None
                    }
                    KeyCode::PageDown => {
                        tab.scroll = tab.scroll.saturating_add(10).min(max_scroll);
                        AppAction::None
                    }
                    KeyCode::PageUp => {
                        tab.scroll = tab.scroll.saturating_sub(10);
                        AppAction::None
                    }
                    _ => AppAction::None,
                }
            }
            WorkbenchTabState::ExtensionOutput(tab) => {
                let max_scroll = tab.lines.len().saturating_sub(1);
                match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Char('j') | KeyCode::Down => {
                        tab.scroll = tab.scroll.saturating_add(1).min(max_scroll);
                        AppAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        tab.scroll = tab.scroll.saturating_sub(1);
                        AppAction::None
                    }
                    KeyCode::Char('g') => {
                        tab.scroll = 0;
                        AppAction::None
                    }
                    KeyCode::Char('G') => {
                        tab.scroll = max_scroll;
                        AppAction::None
                    }
                    KeyCode::PageDown => {
                        tab.scroll = tab.scroll.saturating_add(10).min(max_scroll);
                        AppAction::None
                    }
                    KeyCode::PageUp => {
                        tab.scroll = tab.scroll.saturating_sub(10);
                        AppAction::None
                    }
                    _ => AppAction::None,
                }
            }
            WorkbenchTabState::AiAnalysis(tab) => match key.code {
                KeyCode::Esc => AppAction::EscapePressed,
                KeyCode::Char('j') | KeyCode::Down => {
                    tab.scroll = tab.scroll.saturating_add(1);
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    tab.scroll = tab.scroll.saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('g') => {
                    tab.scroll = 0;
                    AppAction::None
                }
                KeyCode::Char('G') => {
                    tab.scroll = usize::MAX;
                    AppAction::None
                }
                KeyCode::PageDown => {
                    tab.scroll = tab.scroll.saturating_add(10);
                    AppAction::None
                }
                KeyCode::PageUp => {
                    tab.scroll = tab.scroll.saturating_sub(10);
                    AppAction::None
                }
                _ => AppAction::None,
            },
            WorkbenchTabState::Runbook(tab) => match key.code {
                KeyCode::Esc => AppAction::EscapePressed,
                KeyCode::Char('j') | KeyCode::Down
                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    tab.scroll_detail_down(1);
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up
                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    tab.scroll_detail_up(1);
                    AppAction::None
                }
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    tab.scroll_detail_down(10);
                    AppAction::None
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    tab.scroll_detail_up(10);
                    AppAction::None
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    tab.select_next();
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    tab.select_previous();
                    AppAction::None
                }
                KeyCode::Char('g') => {
                    tab.select_top();
                    AppAction::None
                }
                KeyCode::Char('G') => {
                    tab.select_bottom();
                    AppAction::None
                }
                KeyCode::PageDown => {
                    for _ in 0..10 {
                        tab.select_next();
                    }
                    AppAction::None
                }
                KeyCode::PageUp => {
                    for _ in 0..10 {
                        tab.select_previous();
                    }
                    AppAction::None
                }
                KeyCode::Char('d') => AppAction::RunbookToggleStepDone,
                KeyCode::Char('s') => AppAction::RunbookToggleStepSkipped,
                KeyCode::Enter => AppAction::RunbookExecuteSelectedStep,
                _ => AppAction::None,
            },
            WorkbenchTabState::HelmHistory(tab) => {
                if tab.rollback_pending {
                    return match key.code {
                        KeyCode::Char('j') | KeyCode::Down => {
                            tab.scroll = tab.scroll.saturating_add(1);
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            tab.scroll = tab.scroll.saturating_sub(1);
                            AppAction::None
                        }
                        KeyCode::PageDown | KeyCode::Char('d')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            tab.scroll = tab.scroll.saturating_add(10);
                            AppAction::None
                        }
                        KeyCode::PageUp | KeyCode::Char('u')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            tab.scroll = tab.scroll.saturating_sub(10);
                            AppAction::None
                        }
                        KeyCode::Esc => AppAction::None,
                        _ => AppAction::None,
                    };
                }

                if tab.confirm_rollback_revision.is_some() {
                    return match key.code {
                        KeyCode::Esc => {
                            tab.cancel_rollback_confirm();
                            AppAction::None
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            tab.scroll = tab.scroll.saturating_add(1);
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            tab.scroll = tab.scroll.saturating_sub(1);
                            AppAction::None
                        }
                        KeyCode::PageDown | KeyCode::Char('d')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            tab.scroll = tab.scroll.saturating_add(10);
                            AppAction::None
                        }
                        KeyCode::PageUp | KeyCode::Char('u')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            tab.scroll = tab.scroll.saturating_sub(10);
                            AppAction::None
                        }
                        KeyCode::Char('R') | KeyCode::Char('y') | KeyCode::Enter => {
                            AppAction::ExecuteHelmRollback
                        }
                        _ => AppAction::None,
                    };
                }

                if let Some(diff) = tab.diff.as_mut() {
                    let max_scroll = diff.lines.len().saturating_sub(1);
                    return match key.code {
                        KeyCode::Esc => {
                            tab.close_diff();
                            AppAction::None
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            diff.scroll = diff.scroll.saturating_add(1).min(max_scroll);
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            diff.scroll = diff.scroll.saturating_sub(1);
                            AppAction::None
                        }
                        KeyCode::Char('g') => {
                            diff.scroll = 0;
                            AppAction::None
                        }
                        KeyCode::Char('G') => {
                            diff.scroll = max_scroll;
                            AppAction::None
                        }
                        KeyCode::PageDown => {
                            diff.scroll = diff.scroll.saturating_add(10).min(max_scroll);
                            AppAction::None
                        }
                        KeyCode::PageUp => {
                            diff.scroll = diff.scroll.saturating_sub(10);
                            AppAction::None
                        }
                        KeyCode::Char('R') if tab.selected_target_revision().is_some() => {
                            AppAction::ConfirmHelmRollback
                        }
                        _ => AppAction::None,
                    };
                }

                match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Char('j') | KeyCode::Down => {
                        tab.select_next();
                        AppAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        tab.select_previous();
                        AppAction::None
                    }
                    KeyCode::Char('g') => {
                        tab.select_top();
                        AppAction::None
                    }
                    KeyCode::Char('G') => {
                        tab.select_bottom();
                        AppAction::None
                    }
                    KeyCode::PageDown => {
                        for _ in 0..10 {
                            tab.select_next();
                        }
                        AppAction::None
                    }
                    KeyCode::PageUp => {
                        for _ in 0..10 {
                            tab.select_previous();
                        }
                        AppAction::None
                    }
                    KeyCode::Enter if tab.selected_target_revision().is_some() => {
                        AppAction::OpenHelmValuesDiff
                    }
                    KeyCode::Char('R') if tab.selected_target_revision().is_some() => {
                        AppAction::ConfirmHelmRollback
                    }
                    _ => AppAction::None,
                }
            }
            WorkbenchTabState::Rollout(tab) => {
                if tab.mutation_pending.is_some() {
                    return AppAction::None;
                }

                if tab.confirm_undo_revision.is_some() {
                    return match key.code {
                        KeyCode::Esc => {
                            tab.cancel_undo_confirm();
                            AppAction::None
                        }
                        KeyCode::Char('U') | KeyCode::Char('y') | KeyCode::Enter => {
                            AppAction::ExecuteRolloutUndo
                        }
                        _ => AppAction::None,
                    };
                }

                match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Char('j') | KeyCode::Down => {
                        tab.select_next();
                        AppAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        tab.select_previous();
                        AppAction::None
                    }
                    KeyCode::Char('g') => {
                        tab.select_top();
                        AppAction::None
                    }
                    KeyCode::Char('G') => {
                        tab.select_bottom();
                        AppAction::None
                    }
                    KeyCode::PageDown => {
                        for _ in 0..10 {
                            tab.select_next();
                        }
                        AppAction::None
                    }
                    KeyCode::PageUp => {
                        for _ in 0..10 {
                            tab.select_previous();
                        }
                        AppAction::None
                    }
                    KeyCode::Char('R') => AppAction::RolloutRestart,
                    KeyCode::Char('P')
                        if tab.kind
                            == Some(crate::k8s::rollout::RolloutWorkloadKind::Deployment) =>
                    {
                        AppAction::ToggleRolloutPauseResume
                    }
                    KeyCode::Char('U') if tab.selected_undo_revision().is_some() => {
                        AppAction::ConfirmRolloutUndo
                    }
                    _ => AppAction::None,
                }
            }
            WorkbenchTabState::DecodedSecret(tab) => {
                if tab.editing {
                    match key.code {
                        KeyCode::Esc => {
                            tab.editing = false;
                            tab.edit_input.clear();
                            AppAction::None
                        }
                        KeyCode::Enter => {
                            let edited = std::mem::take(&mut tab.edit_input);
                            if let Some(entry) = tab.selected_entry_mut() {
                                entry.commit_edit(edited);
                            }
                            tab.editing = false;
                            AppAction::None
                        }
                        KeyCode::Backspace => {
                            tab.edit_input.pop();
                            AppAction::None
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            tab.edit_input.clear();
                            AppAction::None
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            tab.edit_input.push(c);
                            AppAction::None
                        }
                        _ => AppAction::None,
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => AppAction::EscapePressed,
                        KeyCode::Char('j') | KeyCode::Down => {
                            if !tab.entries.is_empty() {
                                tab.selected =
                                    (tab.selected + 1).min(tab.entries.len().saturating_sub(1));
                                tab.scroll = tab.scroll.max(tab.selected.saturating_sub(1));
                            }
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            tab.selected = tab.selected.saturating_sub(1);
                            tab.scroll = tab.scroll.min(tab.selected);
                            AppAction::None
                        }
                        KeyCode::Char('g') => {
                            tab.selected = 0;
                            tab.scroll = 0;
                            AppAction::None
                        }
                        KeyCode::Char('G') => {
                            let max = tab.entries.len().saturating_sub(1);
                            tab.selected = max;
                            tab.scroll = max;
                            AppAction::None
                        }
                        KeyCode::Char('m') => {
                            tab.masked = !tab.masked;
                            AppAction::None
                        }
                        KeyCode::Char('e') | KeyCode::Enter => {
                            if let Some(entry) = tab.selected_entry()
                                && let Some(value) = entry.editable_text()
                            {
                                tab.edit_input = value.to_string();
                                tab.editing = true;
                            }
                            AppAction::None
                        }
                        KeyCode::Char('s') if tab.has_unsaved_changes() => {
                            AppAction::SaveDecodedSecret
                        }
                        _ => AppAction::None,
                    }
                }
            }
            WorkbenchTabState::ResourceEvents(tab) => {
                let max_scroll = tab.timeline.len().saturating_sub(1);
                match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Char('j') | KeyCode::Down => {
                        tab.scroll = tab.scroll.saturating_add(1).min(max_scroll);
                        AppAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        tab.scroll = tab.scroll.saturating_sub(1);
                        AppAction::None
                    }
                    KeyCode::Char('g') => {
                        tab.scroll = 0;
                        AppAction::None
                    }
                    KeyCode::Char('G') => {
                        tab.scroll = max_scroll;
                        AppAction::None
                    }
                    KeyCode::PageDown => {
                        tab.scroll = tab.scroll.saturating_add(10).min(max_scroll);
                        AppAction::None
                    }
                    KeyCode::PageUp => {
                        tab.scroll = tab.scroll.saturating_sub(10);
                        AppAction::None
                    }
                    _ => AppAction::None,
                }
            }
            WorkbenchTabState::PodLogs(tab) => {
                if tab.viewer.searching {
                    match key.code {
                        KeyCode::Esc => AppAction::LogsViewerSearchCancel,
                        KeyCode::Enter => AppAction::LogsViewerSearchClose,
                        KeyCode::Backspace => {
                            tab.viewer.search_input.pop();
                            AppAction::None
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            tab.viewer.search_input.clear();
                            AppAction::None
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            tab.viewer.search_input.push(c);
                            AppAction::None
                        }
                        _ => AppAction::None,
                    }
                } else if tab.viewer.jumping_to_time {
                    match key.code {
                        KeyCode::Esc => AppAction::CancelLogTimeJump,
                        KeyCode::Enter => AppAction::ApplyLogTimeJump,
                        KeyCode::Backspace => {
                            tab.viewer.time_jump_input.pop();
                            AppAction::None
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            tab.viewer.time_jump_input.clear();
                            AppAction::None
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            tab.viewer.time_jump_input.push(c);
                            AppAction::None
                        }
                        _ => AppAction::None,
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => AppAction::EscapePressed,
                        KeyCode::Char('k') | KeyCode::Up => {
                            if tab.viewer.picking_container {
                                AppAction::LogsViewerPickerUp
                            } else {
                                AppAction::LogsViewerScrollUp
                            }
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            if tab.viewer.picking_container {
                                AppAction::LogsViewerPickerDown
                            } else {
                                AppAction::LogsViewerScrollDown
                            }
                        }
                        KeyCode::Enter if tab.viewer.picking_container => {
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
                        KeyCode::Char('g') => AppAction::LogsViewerScrollTop,
                        KeyCode::Char('G') => AppAction::LogsViewerScrollBottom,
                        KeyCode::Char('f') => AppAction::LogsViewerToggleFollow,
                        KeyCode::Char('P') if !tab.viewer.picking_container => {
                            AppAction::LogsViewerTogglePrevious
                        }
                        KeyCode::Char('t') if !tab.viewer.picking_container => {
                            AppAction::LogsViewerToggleTimestamps
                        }
                        KeyCode::Char('/') if !tab.viewer.picking_container => {
                            AppAction::LogsViewerSearchOpen
                        }
                        KeyCode::Char('n') if !tab.viewer.picking_container => {
                            AppAction::LogsViewerSearchNext
                        }
                        KeyCode::Char('N') if !tab.viewer.picking_container => {
                            AppAction::LogsViewerSearchPrev
                        }
                        KeyCode::Char('R') if !tab.viewer.picking_container => {
                            AppAction::ToggleLogRegexMode
                        }
                        KeyCode::Char('W') if !tab.viewer.picking_container => {
                            AppAction::ToggleLogTimeWindow
                        }
                        KeyCode::Char('T') if !tab.viewer.picking_container => {
                            AppAction::OpenLogTimeJump
                        }
                        KeyCode::Char('C') if !tab.viewer.picking_container => {
                            AppAction::ToggleLogCorrelation
                        }
                        KeyCode::Char('J') if !tab.viewer.picking_container => {
                            AppAction::ToggleStructuredLogView
                        }
                        KeyCode::Char('y') if !tab.viewer.picking_container => {
                            AppAction::CopyLogContent
                        }
                        KeyCode::Char('S') if !tab.viewer.picking_container => {
                            AppAction::ExportLogs
                        }
                        KeyCode::Char('M') if !tab.viewer.picking_container => {
                            AppAction::SaveLogPreset
                        }
                        KeyCode::Char('[') if !tab.viewer.picking_container => {
                            AppAction::ApplyPreviousLogPreset
                        }
                        KeyCode::Char(']') if !tab.viewer.picking_container => {
                            AppAction::ApplyNextLogPreset
                        }
                        _ => AppAction::None,
                    }
                }
            }
            WorkbenchTabState::WorkloadLogs(tab) => {
                let filtered_len = tab
                    .lines
                    .iter()
                    .filter(|line| tab.matches_filter(line))
                    .count();
                if tab.editing_text_filter {
                    match key.code {
                        KeyCode::Esc => {
                            tab.editing_text_filter = false;
                            tab.filter_input.clear();
                            AppAction::None
                        }
                        KeyCode::Enter => {
                            tab.commit_text_filter();
                            AppAction::None
                        }
                        KeyCode::Backspace => {
                            tab.filter_input.pop();
                            AppAction::None
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            tab.filter_input.clear();
                            AppAction::None
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            tab.filter_input.push(c);
                            AppAction::None
                        }
                        _ => AppAction::None,
                    }
                } else if tab.jumping_to_time {
                    match key.code {
                        KeyCode::Esc => AppAction::CancelLogTimeJump,
                        KeyCode::Enter => AppAction::ApplyLogTimeJump,
                        KeyCode::Backspace => {
                            tab.time_jump_input.pop();
                            AppAction::None
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            tab.time_jump_input.clear();
                            AppAction::None
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            tab.time_jump_input.push(c);
                            AppAction::None
                        }
                        _ => AppAction::None,
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => AppAction::EscapePressed,
                        KeyCode::Char('j') | KeyCode::Down => {
                            tab.scroll = (tab.scroll + 1).min(filtered_len.saturating_sub(1));
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            tab.scroll = tab.scroll.saturating_sub(1);
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::Char('g') => {
                            tab.scroll = 0;
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::Char('G') => {
                            tab.scroll = filtered_len.saturating_sub(1);
                            tab.follow_mode = true;
                            AppAction::None
                        }
                        KeyCode::PageDown => {
                            tab.scroll = (tab.scroll + 10).min(filtered_len.saturating_sub(1));
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::PageUp => {
                            tab.scroll = tab.scroll.saturating_sub(10);
                            tab.follow_mode = false;
                            AppAction::None
                        }
                        KeyCode::Char('f') => {
                            tab.follow_mode = !tab.follow_mode;
                            if tab.follow_mode {
                                tab.scroll = filtered_len.saturating_sub(1);
                            }
                            AppAction::None
                        }
                        KeyCode::Char('/') => {
                            tab.editing_text_filter = true;
                            tab.filter_input = tab.text_filter.clone();
                            AppAction::None
                        }
                        KeyCode::Char('p') => {
                            tab.cycle_pod_filter();
                            AppAction::None
                        }
                        KeyCode::Char('c') => {
                            tab.cycle_container_filter();
                            AppAction::None
                        }
                        KeyCode::Char('R') => AppAction::ToggleLogRegexMode,
                        KeyCode::Char('W') => AppAction::ToggleLogTimeWindow,
                        KeyCode::Char('T') => AppAction::OpenLogTimeJump,
                        KeyCode::Char('L') => AppAction::CycleWorkloadLogLabelFilter,
                        KeyCode::Char('C') => AppAction::ToggleLogCorrelation,
                        KeyCode::Char('J') => AppAction::ToggleStructuredLogView,
                        KeyCode::Char('y') if !tab.editing_text_filter => AppAction::CopyLogContent,
                        KeyCode::Char('S') if !tab.editing_text_filter => AppAction::ExportLogs,
                        KeyCode::Char('M') if !tab.editing_text_filter => AppAction::SaveLogPreset,
                        KeyCode::Char('[') if !tab.editing_text_filter => {
                            AppAction::ApplyPreviousLogPreset
                        }
                        KeyCode::Char(']') if !tab.editing_text_filter => {
                            AppAction::ApplyNextLogPreset
                        }
                        _ => AppAction::None,
                    }
                }
            }
            WorkbenchTabState::Exec(tab) => {
                if tab.picking_container {
                    match key.code {
                        KeyCode::Esc => {
                            // Exit container picker back to command input,
                            // don't close the entire workbench.
                            tab.picking_container = false;
                            AppAction::None
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            tab.container_cursor = tab.container_cursor.saturating_sub(1);
                            AppAction::None
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            let max = tab.containers.len().saturating_sub(1);
                            tab.container_cursor = (tab.container_cursor + 1).min(max);
                            AppAction::None
                        }
                        KeyCode::Enter => tab
                            .containers
                            .get(tab.container_cursor)
                            .cloned()
                            .map(AppAction::ExecSelectContainer)
                            .unwrap_or(AppAction::None),
                        _ => AppAction::None,
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => AppAction::EscapePressed,
                        KeyCode::Enter => AppAction::ExecSendInput,
                        KeyCode::Backspace => {
                            tab.input.pop();
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
                        KeyCode::PageUp => {
                            tab.scroll = tab.scroll.saturating_sub(10);
                            AppAction::None
                        }
                        KeyCode::PageDown => {
                            tab.scroll = (tab.scroll + 10).min(tab.lines.len().saturating_sub(1));
                            AppAction::None
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            tab.input.push(c);
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
                KeyCode::Char('j') | KeyCode::Down => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    if !flat.is_empty() {
                        tab.cursor = (tab.cursor + 1).min(flat.len().saturating_sub(1));
                    }
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    tab.cursor = tab.cursor.saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('g') => {
                    tab.cursor = 0;
                    AppAction::None
                }
                KeyCode::Char('G') => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    tab.cursor = flat.len().saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('l') | KeyCode::Right => {
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
                KeyCode::Char('h') | KeyCode::Left => {
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
                KeyCode::Enter => {
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
                _ => AppAction::None,
            },
            WorkbenchTabState::NetworkPolicy(tab) => match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    if !flat.is_empty() {
                        tab.cursor = (tab.cursor + 1).min(flat.len().saturating_sub(1));
                    }
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    tab.cursor = tab.cursor.saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('g') => {
                    tab.cursor = 0;
                    AppAction::None
                }
                KeyCode::Char('G') => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    tab.cursor = flat.len().saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('l') | KeyCode::Right => {
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
                KeyCode::Char('h') | KeyCode::Left => {
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
                KeyCode::Enter => {
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
                KeyCode::Esc => AppAction::EscapePressed,
                _ => AppAction::None,
            },
            WorkbenchTabState::TrafficDebug(tab) => match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    if !flat.is_empty() {
                        tab.cursor = (tab.cursor + 1).min(flat.len().saturating_sub(1));
                    }
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    tab.cursor = tab.cursor.saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('g') => {
                    tab.cursor = 0;
                    AppAction::None
                }
                KeyCode::Char('G') => {
                    let flat = crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                    tab.cursor = flat.len().saturating_sub(1);
                    AppAction::None
                }
                KeyCode::Char('l') | KeyCode::Right => {
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
                KeyCode::Char('h') | KeyCode::Left => {
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
                KeyCode::Enter => {
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
                KeyCode::Esc => AppAction::EscapePressed,
                _ => AppAction::None,
            },
            WorkbenchTabState::Connectivity(tab) => match tab.focus {
                ConnectivityTabFocus::Filter => match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Tab => {
                        tab.focus = ConnectivityTabFocus::Targets;
                        AppAction::None
                    }
                    KeyCode::BackTab => {
                        tab.focus = ConnectivityTabFocus::Result;
                        AppAction::None
                    }
                    KeyCode::Backspace | KeyCode::Delete => {
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
                    KeyCode::Enter => {
                        tab.focus = ConnectivityTabFocus::Targets;
                        AppAction::None
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        tab.filter.clear();
                        tab.refresh_filter();
                        AppAction::None
                    }
                    KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        tab.filter.add_char(ch);
                        tab.refresh_filter();
                        AppAction::None
                    }
                    _ => AppAction::None,
                },
                ConnectivityTabFocus::Targets => match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Tab => {
                        tab.focus = ConnectivityTabFocus::Result;
                        AppAction::None
                    }
                    KeyCode::BackTab => {
                        tab.focus = ConnectivityTabFocus::Filter;
                        AppAction::None
                    }
                    KeyCode::Char('/') => {
                        tab.focus = ConnectivityTabFocus::Filter;
                        AppAction::None
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        if !tab.filtered_target_indices.is_empty() {
                            tab.selected_target = (tab.selected_target + 1)
                                .min(tab.filtered_target_indices.len().saturating_sub(1));
                        }
                        AppAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        tab.selected_target = tab.selected_target.saturating_sub(1);
                        AppAction::None
                    }
                    KeyCode::Char('g') => {
                        tab.selected_target = 0;
                        AppAction::None
                    }
                    KeyCode::Char('G') => {
                        tab.selected_target = tab.filtered_target_indices.len().saturating_sub(1);
                        AppAction::None
                    }
                    KeyCode::Enter => AppAction::OpenNetworkConnectivity,
                    _ => AppAction::None,
                },
                ConnectivityTabFocus::Result => match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Tab => {
                        tab.focus = ConnectivityTabFocus::Filter;
                        AppAction::None
                    }
                    KeyCode::BackTab => {
                        tab.focus = ConnectivityTabFocus::Targets;
                        AppAction::None
                    }
                    KeyCode::Char('/') => {
                        tab.focus = ConnectivityTabFocus::Filter;
                        AppAction::None
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        let flat =
                            crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                        if !flat.is_empty() {
                            tab.tree_cursor =
                                (tab.tree_cursor + 1).min(flat.len().saturating_sub(1));
                        }
                        AppAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        tab.tree_cursor = tab.tree_cursor.saturating_sub(1);
                        AppAction::None
                    }
                    KeyCode::Char('g') => {
                        tab.tree_cursor = 0;
                        AppAction::None
                    }
                    KeyCode::Char('G') => {
                        let flat =
                            crate::k8s::relationships::flatten_tree(&tab.tree, &tab.expanded);
                        tab.tree_cursor = flat.len().saturating_sub(1);
                        AppAction::None
                    }
                    KeyCode::Char('l') | KeyCode::Right => {
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
                    KeyCode::Char('h') | KeyCode::Left => {
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
                    KeyCode::Enter => {
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
            KeyCode::Char('r') if allow_plain_r => Some(AppAction::RefreshData),
            KeyCode::Char('R')
                if key.modifiers.contains(KeyModifiers::CONTROL) && allow_plain_r =>
            {
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
    /// 6. **Quit confirmation** — after `q`/`Esc`, `q`/`y`/`Enter` confirms; any other key cancels.
    /// 7. **Main navigation** (see table below).
    ///
    /// # Main navigation keys
    ///
    /// | Key | Condition | Effect |
    /// |-----|-----------|--------|
    /// | `q` | — | Enter quit confirmation |
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
                KeyCode::Esc | KeyCode::Char('?') => AppAction::CloseHelp,
                KeyCode::Char('j') | KeyCode::Down => {
                    self.help_overlay.scroll_down();
                    AppAction::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
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
                KeyCode::Esc => {
                    self.resource_template_dialog = None;
                    AppAction::None
                }
                KeyCode::Enter
                    if dialog.focus_field
                        == crate::ui::components::ResourceTemplateField::CreateBtn =>
                {
                    AppAction::SubmitResourceTemplateDialog
                }
                KeyCode::Enter
                    if dialog.focus_field
                        == crate::ui::components::ResourceTemplateField::CancelBtn =>
                {
                    self.resource_template_dialog = None;
                    AppAction::None
                }
                KeyCode::Tab | KeyCode::Down => {
                    dialog.next_field();
                    AppAction::None
                }
                KeyCode::BackTab | KeyCode::Up => {
                    dialog.prev_field();
                    AppAction::None
                }
                KeyCode::Backspace => {
                    dialog.backspace();
                    AppAction::None
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
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
                return match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Enter => AppAction::ScaleDialogSubmit,
                    KeyCode::Backspace => AppAction::ScaleDialogBackspace,
                    KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Up => {
                        AppAction::ScaleDialogIncrement
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') | KeyCode::Down => {
                        AppAction::ScaleDialogDecrement
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() => AppAction::ScaleDialogUpdateInput(c),
                    _ => AppAction::None,
                };
            }
            ActiveComponent::ProbePanel => {
                return match key.code {
                    KeyCode::Esc => AppAction::EscapePressed,
                    KeyCode::Enter | KeyCode::Char(' ') => AppAction::ProbeToggleExpand,
                    KeyCode::Char('j') | KeyCode::Down => AppAction::ProbeSelectNext,
                    KeyCode::Char('k') | KeyCode::Up => AppAction::ProbeSelectPrev,
                    _ => AppAction::None,
                };
            }
            ActiveComponent::None => {}
        }

        if self.confirm_quit {
            return match key.code {
                KeyCode::Char('q') | KeyCode::Char('y') | KeyCode::Enter => {
                    self.should_quit = true;
                    AppAction::Quit
                }
                _ => {
                    self.confirm_quit = false;
                    AppAction::None
                }
            };
        }

        match key.code {
            KeyCode::Char('q') => {
                self.confirm_quit = true;
                AppAction::None
            }
            KeyCode::Esc
                if self
                    .detail_view
                    .as_ref()
                    .map(|d| d.confirm_delete)
                    .unwrap_or(false) =>
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
                    .unwrap_or(false) =>
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
                    .is_some() =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.confirm_cronjob_suspend = None;
                }
                AppAction::None
            }
            KeyCode::Esc if self.detail_view.is_some() => AppAction::CloseDetail,
            KeyCode::Esc if self.focus == Focus::Content => {
                self.focus = Focus::Sidebar;
                AppAction::None
            }
            KeyCode::Esc if self.focus == Focus::Workbench => {
                self.focus = Focus::Content;
                AppAction::None
            }
            KeyCode::Esc => {
                self.confirm_quit = true;
                AppAction::None
            }
            KeyCode::Char('l') | KeyCode::Char('L')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Logs))
                    || (self.detail_view.is_none() && self.focus == Focus::Content) =>
            {
                AppAction::LogsViewerOpen
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                AppAction::CopyResourceName
            }
            KeyCode::Char('y')
                if (self.detail_view.as_ref().is_some_and(|detail| {
                    detail.supports_action(DetailAction::ViewYaml)
                        && !detail.has_confirmation_dialog()
                }) || (self.detail_view.is_none() && self.focus == Focus::Content)) =>
            {
                AppAction::OpenResourceYaml
            }
            KeyCode::Char('D')
                if self.detail_view.as_ref().is_some_and(|detail| {
                    detail.supports_action(DetailAction::ViewConfigDrift)
                        && !detail.supports_action(DetailAction::Drain)
                        && !detail.has_confirmation_dialog()
                }) =>
            {
                AppAction::OpenResourceDiff
            }
            KeyCode::Char('O')
                if self.detail_view.as_ref().is_some_and(|detail| {
                    detail.supports_action(DetailAction::ViewRollout)
                        && !detail.has_confirmation_dialog()
                }) =>
            {
                AppAction::OpenRollout
            }
            KeyCode::Char('h')
                if self.detail_view.as_ref().is_some_and(|detail| {
                    detail.supports_action(DetailAction::ViewHelmHistory)
                        && !detail.has_confirmation_dialog()
                }) || (self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && self.view == AppView::HelmReleases) =>
            {
                AppAction::OpenHelmHistory
            }
            KeyCode::Char('A')
                if (self.detail_view.as_ref().is_some_and(|detail| {
                    detail.supports_action(DetailAction::ViewAccessReview)
                }) || (self.detail_view.is_none() && self.focus == Focus::Content))
                    && !self
                        .detail_view
                        .as_ref()
                        .is_some_and(DetailViewState::has_confirmation_dialog) =>
            {
                AppAction::OpenAccessReview
            }
            KeyCode::Char('N')
                if self.detail_view.as_ref().is_some_and(|detail| {
                    detail.supports_action(DetailAction::ViewNetworkPolicies)
                }) && !self
                    .detail_view
                    .as_ref()
                    .is_some_and(DetailViewState::has_confirmation_dialog) =>
            {
                AppAction::OpenNetworkPolicyView
            }
            KeyCode::Char('C')
                if self.detail_view.as_ref().is_some_and(|detail| {
                    detail.supports_action(DetailAction::CheckNetworkConnectivity)
                }) && !self
                    .detail_view
                    .as_ref()
                    .is_some_and(DetailViewState::has_confirmation_dialog) =>
            {
                AppAction::OpenNetworkConnectivity
            }
            KeyCode::Char('t')
                if (self.detail_view.as_ref().is_some_and(|detail| {
                    detail.supports_action(DetailAction::ViewTrafficDebug)
                }) || (self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && matches!(
                        self.view,
                        AppView::Services | AppView::Endpoints | AppView::Ingresses | AppView::Pods
                    )))
                    && !self
                        .detail_view
                        .as_ref()
                        .is_some_and(DetailViewState::has_confirmation_dialog) =>
            {
                AppAction::OpenTrafficDebug
            }
            KeyCode::Char('o')
                if self.detail_view.as_ref().is_some_and(|detail| {
                    detail.supports_action(DetailAction::ViewDecodedSecret)
                }) || (self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && self.view == AppView::Secrets) =>
            {
                AppAction::OpenDecodedSecret
            }
            KeyCode::Char('B')
                if self
                    .detail_view
                    .as_ref()
                    .and_then(|detail| detail.resource.as_ref())
                    .is_some()
                    || (self.detail_view.is_none()
                        && self.focus == Focus::Content
                        && !matches!(
                            self.view,
                            AppView::Dashboard
                                | AppView::HelmCharts
                                | AppView::PortForwarding
                                | AppView::Extensions
                        )) =>
            {
                AppAction::ToggleBookmark
            }
            KeyCode::Char('Y') if self.detail_view.is_none() && self.focus == Focus::Content => {
                AppAction::CopyResourceFullName
            }
            KeyCode::Char('v')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::ViewEvents))
                    || (self.detail_view.is_none() && self.focus == Focus::Content) =>
            {
                AppAction::OpenResourceEvents
            }
            KeyCode::Char('H')
                if !self
                    .detail_view
                    .as_ref()
                    .is_some_and(DetailViewState::has_confirmation_dialog) =>
            {
                AppAction::OpenActionHistory
            }
            KeyCode::Char('x')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Exec))
                    || (self.detail_view.is_none() && self.focus == Focus::Content) =>
            {
                AppAction::OpenExec
            }
            KeyCode::Char('g')
                if self.detail_view.as_ref().is_some_and(|detail| {
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
            KeyCode::Char('f')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::PortForward))
                    || (self.detail_view.is_none() && self.focus == Focus::Content) =>
            {
                AppAction::PortForwardOpen
            }
            KeyCode::Char('s')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Scale)) =>
            {
                AppAction::ScaleDialogOpen
            }
            KeyCode::Char('p')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Probes)) =>
            {
                AppAction::ProbePanelOpen
            }
            KeyCode::Char('R')
                if self.detail_view.is_some() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
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
                    .is_some_and(|detail| detail.supports_action(DetailAction::EditYaml)) =>
            {
                AppAction::EditYaml
            }
            KeyCode::Char('m') if self.detail_view.is_some() => AppAction::ToggleDetailMetadata,
            KeyCode::Char('d')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Delete)) =>
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
            KeyCode::Char('j') | KeyCode::Down
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| !detail.has_confirmation_dialog())
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.scroll_top_panels_down(1);
                }
                AppAction::None
            }
            KeyCode::Char('j') | KeyCode::Down
                if self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(
                        self.view,
                        AppView::Dashboard
                            | AppView::Projects
                            | AppView::Governance
                            | AppView::RoleBindings
                            | AppView::ClusterRoleBindings
                            | AppView::Roles
                            | AppView::ClusterRoles
                    ) =>
            {
                self.content_detail_scroll = self.content_detail_scroll.saturating_add(1);
                AppAction::None
            }
            KeyCode::Char('k') | KeyCode::Up
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| !detail.has_confirmation_dialog())
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.scroll_top_panels_up(1);
                }
                AppAction::None
            }
            KeyCode::Char('k') | KeyCode::Up
                if self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(
                        self.view,
                        AppView::Dashboard
                            | AppView::Projects
                            | AppView::Governance
                            | AppView::RoleBindings
                            | AppView::ClusterRoleBindings
                            | AppView::Roles
                            | AppView::ClusterRoles
                    ) =>
            {
                self.content_detail_scroll = self.content_detail_scroll.saturating_sub(1);
                AppAction::None
            }
            KeyCode::Char('d')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| !detail.has_confirmation_dialog())
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.scroll_top_panels_down(10);
                }
                AppAction::None
            }
            KeyCode::Char('d')
                if self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(
                        self.view,
                        AppView::Dashboard
                            | AppView::Projects
                            | AppView::Governance
                            | AppView::RoleBindings
                            | AppView::ClusterRoleBindings
                            | AppView::Roles
                            | AppView::ClusterRoles
                    ) =>
            {
                self.content_detail_scroll = self.content_detail_scroll.saturating_add(10);
                AppAction::None
            }
            KeyCode::Char('u')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| !detail.has_confirmation_dialog())
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.scroll_top_panels_up(10);
                }
                AppAction::None
            }
            KeyCode::Char('u')
                if self.detail_view.is_none()
                    && self.focus == Focus::Content
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(
                        self.view,
                        AppView::Dashboard
                            | AppView::Projects
                            | AppView::Governance
                            | AppView::RoleBindings
                            | AppView::ClusterRoleBindings
                            | AppView::Roles
                            | AppView::ClusterRoles
                    ) =>
            {
                self.content_detail_scroll = self.content_detail_scroll.saturating_sub(10);
                AppAction::None
            }
            KeyCode::Char('j') | KeyCode::Down
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| !detail.has_confirmation_dialog())
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
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
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if let Some(detail) = &mut self.detail_view {
                    detail.select_prev_cronjob_history();
                }
                AppAction::None
            }
            KeyCode::Tab if self.detail_view.is_none() => {
                self.next_view();
                AppAction::None
            }
            KeyCode::BackTab if self.detail_view.is_none() => {
                self.previous_view();
                AppAction::None
            }
            KeyCode::Char('j') | KeyCode::Down
                if self.detail_view.is_none() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
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
                if self.detail_view.is_none() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
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
            KeyCode::Down
                if self.detail_view.is_none() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.select_next();
                AppAction::None
            }
            KeyCode::Up
                if self.detail_view.is_none() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.select_previous();
                AppAction::None
            }
            KeyCode::Char('n') if self.detail_view.is_none() && self.view == AppView::Pods => {
                self.set_or_toggle_pod_sort(PodSortColumn::Name);
                AppAction::None
            }
            KeyCode::Char('n')
                if self.detail_view.is_none()
                    && self.view.supports_shared_sort(WorkloadSortColumn::Name) =>
            {
                self.set_or_toggle_workload_sort(WorkloadSortColumn::Name);
                AppAction::None
            }
            KeyCode::Char('a') if self.detail_view.is_none() && self.view == AppView::Pods => {
                self.set_or_toggle_pod_sort(PodSortColumn::Age);
                AppAction::None
            }
            KeyCode::Char('a')
                if self.detail_view.is_none()
                    && self.view.supports_shared_sort(WorkloadSortColumn::Age) =>
            {
                self.set_or_toggle_workload_sort(WorkloadSortColumn::Age);
                AppAction::None
            }
            KeyCode::Char('1') if self.detail_view.is_none() && self.view == AppView::Pods => {
                self.set_or_toggle_pod_sort(PodSortColumn::Age);
                AppAction::None
            }
            KeyCode::Char('1')
                if self.detail_view.is_none()
                    && self.view.supports_shared_sort(WorkloadSortColumn::Age) =>
            {
                self.set_or_toggle_workload_sort(WorkloadSortColumn::Age);
                AppAction::None
            }
            KeyCode::Char('2') if self.detail_view.is_none() && self.view == AppView::Pods => {
                self.set_or_toggle_pod_sort(PodSortColumn::Status);
                AppAction::None
            }
            KeyCode::Char('3') if self.detail_view.is_none() && self.view == AppView::Pods => {
                self.set_or_toggle_pod_sort(PodSortColumn::Restarts);
                AppAction::None
            }
            KeyCode::Char('0') if self.detail_view.is_none() && self.view == AppView::Pods => {
                self.clear_pod_sort();
                AppAction::None
            }
            KeyCode::Char('0')
                if self.detail_view.is_none()
                    && !self.view.shared_sort_capabilities().is_empty() =>
            {
                self.clear_workload_sort();
                AppAction::None
            }
            KeyCode::Char('/') => {
                self.is_search_mode = true;
                AppAction::None
            }
            KeyCode::Char('~') => AppAction::OpenNamespacePicker,
            KeyCode::Char('W') if self.detail_view.is_none() => AppAction::SaveWorkspace,
            KeyCode::Char('{') if self.detail_view.is_none() => AppAction::ApplyPreviousWorkspace,
            KeyCode::Char('}') if self.detail_view.is_none() => AppAction::ApplyNextWorkspace,
            KeyCode::Char('b') if self.detail_view.is_none() => AppAction::ToggleWorkbench,
            KeyCode::Char('[') if self.detail_view.is_none() && self.workbench.open => {
                AppAction::WorkbenchPreviousTab
            }
            KeyCode::Char(']') if self.detail_view.is_none() && self.workbench.open => {
                AppAction::WorkbenchNextTab
            }
            KeyCode::Char('w')
                if self.detail_view.is_none()
                    && self.workbench.open
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                AppAction::WorkbenchCloseActiveTab
            }
            KeyCode::Up
                if self.detail_view.is_none()
                    && self.workbench.open
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                AppAction::WorkbenchIncreaseHeight
            }
            KeyCode::Down
                if self.detail_view.is_none()
                    && self.workbench.open
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                AppAction::WorkbenchDecreaseHeight
            }
            KeyCode::Char('c') if self.detail_view.is_none() => AppAction::OpenContextPicker,
            KeyCode::Char(':')
                if !self
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
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                AppAction::FluxReconcile
            }
            KeyCode::Char('r')
                if !self
                    .detail_view
                    .as_ref()
                    .is_some_and(DetailViewState::has_confirmation_dialog) =>
            {
                AppAction::RefreshData
            }
            KeyCode::Char('R')
                if key.modifiers.contains(KeyModifiers::CONTROL)
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
                }) =>
            {
                AppAction::OpenRelationships
            }
            KeyCode::Char('T')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Trigger)) =>
            {
                AppAction::TriggerCronJob
            }
            KeyCode::Char('S')
                if self.detail_view.as_ref().is_some_and(|detail| {
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
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Cordon)) =>
            {
                AppAction::CordonNode
            }
            KeyCode::Char('u')
                if self
                    .detail_view
                    .as_ref()
                    .is_some_and(|detail| detail.supports_action(DetailAction::Uncordon)) =>
            {
                AppAction::UncordonNode
            }
            KeyCode::Char('D')
                if self
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
            KeyCode::Char('T') if self.detail_view.is_none() => AppAction::CycleTheme,
            KeyCode::Char('I') if self.detail_view.is_none() => AppAction::CycleIconMode,
            KeyCode::Char('?')
                if !self
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
        match key.code {
            KeyCode::Esc => {
                self.search_query.clear();
                self.is_search_mode = false;
                // Reset selection so the user doesn't land on a stale filtered index.
                self.selected_idx = 0;
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
