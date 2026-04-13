//! Copy-to-clipboard and log-export action handlers.

use kubectui::{
    app::AppState,
    log_investigation::entry_matches_query,
    state::ClusterSnapshot,
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
    let content = app
        .workbench()
        .active_tab()
        .and_then(active_log_copy_content);
    if let Some(content) = content {
        let line_count = content.lines().count();
        if let Err(e) = kubectui::clipboard::copy_to_clipboard(&content) {
            app.set_error(format!("Clipboard error: {e}"));
        } else {
            app.status_message = Some(format!("Copied {line_count} log lines"));
        }
    }
}

/// Exports the active log tab content to a file.
pub fn export_logs(app: &mut AppState) {
    let export_data = app.workbench().active_tab().and_then(active_log_export);
    if let Some((label, content)) = export_data {
        match kubectui::export::save_logs_to_file(&label, &content) {
            Ok(path) => {
                app.status_message = Some(format!("Saved to {}", path.display()));
            }
            Err(e) => {
                app.set_error(format!("Export error: {e}"));
            }
        }
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
        workbench::{PodLogsTabState, WorkbenchTabState, WorkloadLogLine, WorkloadLogsTabState},
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
