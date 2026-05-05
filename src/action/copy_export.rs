//! Copy-to-clipboard and log-export action handlers.

use kubectui::{
    app::AppState, log_investigation::entry_matches_query, state::ClusterSnapshot,
    workbench::WorkbenchTabState,
};

use crate::selection_helpers::selected_resource;

/// Copies the short resource name to the clipboard.
pub fn copy_resource_name(app: &mut AppState, cached_snapshot: &ClusterSnapshot) {
    let name = app
        .detail_view
        .as_ref()
        .and_then(|d| d.resource.as_ref())
        .map(|r| r.name().to_string())
        .or_else(|| selected_resource(app, cached_snapshot).map(|r| r.name().to_string()));
    if let Some(name) = name {
        if let Err(e) = kubectui::clipboard::copy_to_clipboard(&name) {
            app.set_error(format!("Clipboard error: {e}"));
        } else {
            app.status_message = Some(format!("Copied: {name}"));
        }
    }
}

/// Copies the fully-qualified resource name (`namespace/name`) to the clipboard.
pub fn copy_resource_full_name(app: &mut AppState, cached_snapshot: &ClusterSnapshot) {
    let full = app
        .detail_view
        .as_ref()
        .and_then(|d| d.resource.as_ref())
        .map(|r| match r.namespace() {
            Some(ns) => format!("{ns}/{}", r.name()),
            None => r.name().to_string(),
        })
        .or_else(|| {
            selected_resource(app, cached_snapshot).map(|r| match r.namespace() {
                Some(ns) => format!("{ns}/{}", r.name()),
                None => r.name().to_string(),
            })
        });
    if let Some(full) = full {
        if let Err(e) = kubectui::clipboard::copy_to_clipboard(&full) {
            app.set_error(format!("Clipboard error: {e}"));
        } else {
            app.status_message = Some(format!("Copied: {full}"));
        }
    }
}

/// Copies the active log tab content to the clipboard.
pub fn copy_log_content(app: &mut AppState) {
    let Some(tab) = app.workbench().active_tab() else {
        app.set_error("Open a log tab before copying logs.".to_string());
        return;
    };
    let Some(content) = active_log_copy_content(tab) else {
        app.set_error(empty_or_unsupported_log_action_message(tab, "copy"));
        return;
    };

    let line_count = content.lines().count();
    if let Err(e) = kubectui::clipboard::copy_to_clipboard(&content) {
        app.set_error(format!("Clipboard error: {e}"));
    } else {
        app.status_message = Some(format!("Copied {line_count} log lines"));
    }
}

/// Exports the active log tab content to a file.
pub fn export_logs(app: &mut AppState) {
    let Some(tab) = app.workbench().active_tab() else {
        app.set_error("Open a log tab before exporting logs.".to_string());
        return;
    };
    let Some((label, content)) = active_log_export(tab) else {
        app.set_error(empty_or_unsupported_log_action_message(tab, "export"));
        return;
    };

    match kubectui::export::save_logs_to_file(&label, &content) {
        Ok(path) => {
            app.status_message = Some(format!("Saved to {}", path.display()));
        }
        Err(e) => {
            app.set_error(format!("Export error: {e}"));
        }
    }
}

/// Copies the active exec tab output to the clipboard.
pub fn copy_exec_output(app: &mut AppState) {
    let Some(tab) = app.workbench().active_tab() else {
        app.set_error("Open an exec tab before copying exec output.".to_string());
        return;
    };
    let Some(content) = active_exec_output(tab) else {
        app.set_error(empty_or_unsupported_exec_action_message(tab, "copy"));
        return;
    };

    let line_count = content.lines().count();
    if let Err(e) = kubectui::clipboard::copy_to_clipboard(&content) {
        app.set_error(format!("Clipboard error: {e}"));
    } else {
        app.status_message = Some(format!("Copied {line_count} exec output lines"));
    }
}

/// Exports the active exec tab output to a file.
pub fn export_exec_output(app: &mut AppState) {
    let Some(tab) = app.workbench().active_tab() else {
        app.set_error("Open an exec tab before exporting exec output.".to_string());
        return;
    };
    let Some((label, content)) = active_exec_export(tab) else {
        app.set_error(empty_or_unsupported_exec_action_message(tab, "export"));
        return;
    };

    match kubectui::export::save_text_to_file("exec", &label, &content) {
        Ok(path) => {
            app.status_message = Some(format!("Saved to {}", path.display()));
        }
        Err(e) => {
            app.set_error(format!("Export error: {e}"));
        }
    }
}

fn empty_or_unsupported_log_action_message(
    tab: &kubectui::workbench::WorkbenchTab,
    action: &str,
) -> String {
    match &tab.state {
        WorkbenchTabState::PodLogs(_) | WorkbenchTabState::WorkloadLogs(_) => {
            format!("No matching log lines to {action}.")
        }
        _ => format!("Open a log tab before {action}ing logs."),
    }
}

fn empty_or_unsupported_exec_action_message(
    tab: &kubectui::workbench::WorkbenchTab,
    action: &str,
) -> String {
    match &tab.state {
        WorkbenchTabState::Exec(_) => format!("No exec output to {action}."),
        _ => format!("Open an exec tab before {action}ing exec output."),
    }
}

fn active_log_copy_content(tab: &kubectui::workbench::WorkbenchTab) -> Option<String> {
    match &tab.state {
        WorkbenchTabState::PodLogs(logs_tab) => {
            let filtered = visible_pod_log_indices(logs_tab);
            (!filtered.is_empty()).then(|| {
                filtered
                    .iter()
                    .filter_map(|index| logs_tab.viewer.lines.get(*index))
                    .map(|line| {
                        line.display_text(logs_tab.viewer.structured_view)
                            .to_string()
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        }
        WorkbenchTabState::WorkloadLogs(wl_tab) => {
            let content = wl_tab
                .lines
                .iter()
                .filter(|line| wl_tab.matches_filter(line))
                .map(|line| {
                    format!(
                        "{}:{} {}",
                        line.pod_name,
                        line.container_name,
                        line.entry.display_text(wl_tab.structured_view)
                    )
                })
                .collect::<Vec<_>>();
            (!content.is_empty()).then(|| content.join("\n"))
        }
        _ => None,
    }
}

fn active_exec_output(tab: &kubectui::workbench::WorkbenchTab) -> Option<String> {
    match &tab.state {
        WorkbenchTabState::Exec(exec_tab) => exec_tab.output_text(),
        _ => None,
    }
}

fn active_exec_export(tab: &kubectui::workbench::WorkbenchTab) -> Option<(String, String)> {
    match &tab.state {
        WorkbenchTabState::Exec(exec_tab) => exec_tab
            .output_text()
            .map(|content| (exec_tab.output_label(), content)),
        _ => None,
    }
}

fn active_log_export(tab: &kubectui::workbench::WorkbenchTab) -> Option<(String, String)> {
    match &tab.state {
        WorkbenchTabState::PodLogs(logs_tab) => {
            let label = format!(
                "{}-{}",
                logs_tab.viewer.pod_name, logs_tab.viewer.container_name,
            );
            let filtered = visible_pod_log_indices(logs_tab);
            (!filtered.is_empty()).then(|| {
                let content = filtered
                    .iter()
                    .filter_map(|index| logs_tab.viewer.lines.get(*index))
                    .map(|line| {
                        line.display_text(logs_tab.viewer.structured_view)
                            .to_string()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (label, content)
            })
        }
        WorkbenchTabState::WorkloadLogs(_) => active_log_copy_content(tab)
            .map(|content| (tab.state.title().replace(' ', "-"), content)),
        _ => None,
    }
}

fn visible_pod_log_indices(logs_tab: &kubectui::workbench::PodLogsTabState) -> Vec<usize> {
    let viewer = &logs_tab.viewer;
    viewer
        .filtered_indices()
        .into_iter()
        .filter(|index| {
            viewer.lines.get(*index).is_some_and(|line| {
                entry_matches_query(
                    line,
                    &viewer.search_query,
                    viewer.search_mode,
                    viewer.compiled_search.as_ref(),
                    viewer.structured_view,
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kubectui::{
        app::{LogsViewerState, ResourceRef},
        log_investigation::{LogEntry, LogQueryMode, LogTimeWindow, compile_query},
        workbench::{
            ExecTabState, PodLogsTabState, WorkbenchTabState, WorkloadLogLine, WorkloadLogsTabState,
        },
    };

    #[test]
    fn workload_copy_uses_filtered_structured_lines() {
        let mut tab = WorkloadLogsTabState::new(ResourceRef::Pod("pod-0".into(), "ns".into()), 1);
        tab.lines.push(WorkloadLogLine {
            pod_name: "pod-0".into(),
            container_name: "main".into(),
            entry: LogEntry::from_raw(r#"{"level":"info","message":"boot","request_id":"abc"}"#),
            is_stderr: false,
        });
        tab.lines.push(WorkloadLogLine {
            pod_name: "pod-1".into(),
            container_name: "main".into(),
            entry: LogEntry::from_raw("plain line"),
            is_stderr: false,
        });
        tab.text_filter = "req=abc".into();
        tab.text_filter_mode = LogQueryMode::Regex;
        tab.compiled_text_filter =
            compile_query(&tab.text_filter, tab.text_filter_mode).expect("compiled");

        let content = active_log_copy_content(&kubectui::workbench::WorkbenchTab {
            id: 1,
            state: WorkbenchTabState::WorkloadLogs(tab),
        })
        .expect("copy content");

        assert_eq!(content, "pod-0:main INFO req=abc boot");
    }

    #[test]
    fn pod_export_uses_structured_view() {
        let mut viewer = LogsViewerState {
            pod_name: "pod-0".into(),
            container_name: "main".into(),
            ..LogsViewerState::default()
        };
        viewer
            .lines
            .push(LogEntry::from_raw(r#"{"level":"warn","message":"retry"}"#));

        let export = active_log_export(&kubectui::workbench::WorkbenchTab {
            id: 1,
            state: WorkbenchTabState::PodLogs(PodLogsTabState {
                resource: ResourceRef::Pod("pod-0".into(), "ns".into()),
                viewer,
            }),
        })
        .expect("export data");

        assert_eq!(export.0, "pod-0-main");
        assert_eq!(export.1, "WARN retry");
    }

    #[test]
    fn pod_copy_respects_time_window_filter() {
        let mut viewer = LogsViewerState {
            pod_name: "pod-0".into(),
            container_name: "main".into(),
            time_window: LogTimeWindow::Last5Minutes,
            ..LogsViewerState::default()
        };
        viewer
            .lines
            .push(LogEntry::from_raw("2020-01-01T00:00:00Z stale line"));

        let content = active_log_copy_content(&kubectui::workbench::WorkbenchTab {
            id: 1,
            state: WorkbenchTabState::PodLogs(PodLogsTabState {
                resource: ResourceRef::Pod("pod-0".into(), "ns".into()),
                viewer,
            }),
        });

        assert!(content.is_none());
    }

    #[test]
    fn copy_log_content_reports_missing_log_tab() {
        let mut app = AppState::default();

        copy_log_content(&mut app);

        assert_eq!(
            app.error_message(),
            Some("Open a log tab before copying logs.")
        );
    }

    #[test]
    fn export_logs_reports_empty_filtered_log_tab() {
        let mut app = AppState::default();
        app.open_pod_logs_tab(ResourceRef::Pod("pod-0".into(), "ns".into()));

        export_logs(&mut app);

        assert_eq!(
            app.error_message(),
            Some("No matching log lines to export.")
        );
    }

    #[test]
    fn exec_copy_includes_pending_fragment() {
        let mut tab = ExecTabState::new(
            ResourceRef::Pod("pod-0".into(), "ns".into()),
            1,
            "pod-0".into(),
            "ns".into(),
        );
        tab.container_name = "main".into();
        tab.lines.push("first line".into());
        tab.pending_fragment = "partial".into();

        let content = active_exec_output(&kubectui::workbench::WorkbenchTab {
            id: 1,
            state: WorkbenchTabState::Exec(tab),
        })
        .expect("exec output");

        assert_eq!(content, "first line\npartial");
    }

    #[test]
    fn exec_export_uses_resource_label() {
        let mut tab = ExecTabState::new(
            ResourceRef::Pod("pod-0".into(), "ns".into()),
            1,
            "pod-0".into(),
            "ns".into(),
        );
        tab.container_name = "main".into();
        tab.lines.push("ok".into());

        let export = active_exec_export(&kubectui::workbench::WorkbenchTab {
            id: 1,
            state: WorkbenchTabState::Exec(tab),
        })
        .expect("exec export");

        assert_eq!(export.0, "ns-pod-0-main");
        assert_eq!(export.1, "ok");
    }

    #[test]
    fn copy_exec_output_reports_missing_exec_tab() {
        let mut app = AppState::default();

        copy_exec_output(&mut app);

        assert_eq!(
            app.error_message(),
            Some("Open an exec tab before copying exec output.")
        );
    }

    #[test]
    fn export_exec_output_reports_empty_exec_tab() {
        let mut app = AppState::default();
        app.workbench_mut()
            .open_tab(WorkbenchTabState::Exec(ExecTabState::new(
                ResourceRef::Pod("pod-0".into(), "ns".into()),
                1,
                "pod-0".into(),
                "ns".into(),
            )));

        export_exec_output(&mut app);

        assert_eq!(app.error_message(), Some("No exec output to export."));
    }

    #[test]
    fn export_exec_output_uses_exec_file_prefix() {
        let mut app = AppState::default();
        let mut tab = ExecTabState::new(
            ResourceRef::Pod("pod-0".into(), "ns".into()),
            1,
            "pod-0".into(),
            "ns".into(),
        );
        tab.container_name = "main".into();
        tab.lines.push("ok".into());
        app.workbench_mut().open_tab(WorkbenchTabState::Exec(tab));

        export_exec_output(&mut app);

        let status = app.status_message.expect("status message");
        assert!(status.contains("kubectui-exec-ns-pod-0-main-"));
        let path = status.trim_start_matches("Saved to ");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "ok");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn pod_copy_respects_active_search_query() {
        let mut viewer = LogsViewerState {
            pod_name: "pod-0".into(),
            container_name: "main".into(),
            search_query: "request".into(),
            search_mode: LogQueryMode::Substring,
            ..LogsViewerState::default()
        };
        viewer.lines.push(LogEntry::from_raw("startup complete"));
        viewer.lines.push(LogEntry::from_raw("request failed"));

        let content = active_log_copy_content(&kubectui::workbench::WorkbenchTab {
            id: 1,
            state: WorkbenchTabState::PodLogs(PodLogsTabState {
                resource: ResourceRef::Pod("pod-0".into(), "ns".into()),
                viewer,
            }),
        })
        .expect("copy content");

        assert_eq!(content, "request failed");
    }

    #[test]
    fn pod_export_respects_active_search_query() {
        let mut viewer = LogsViewerState {
            pod_name: "pod-0".into(),
            container_name: "main".into(),
            search_query: "warn".into(),
            search_mode: LogQueryMode::Substring,
            ..LogsViewerState::default()
        };
        viewer.lines.push(LogEntry::from_raw("info boot"));
        viewer.lines.push(LogEntry::from_raw("warn retry"));

        let export = active_log_export(&kubectui::workbench::WorkbenchTab {
            id: 1,
            state: WorkbenchTabState::PodLogs(PodLogsTabState {
                resource: ResourceRef::Pod("pod-0".into(), "ns".into()),
                viewer,
            }),
        })
        .expect("export data");

        assert_eq!(export.1, "warn retry");
    }
}
