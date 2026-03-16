//! Copy-to-clipboard and log-export action handlers.

use kubectui::{app::AppState, state::ClusterSnapshot, workbench::WorkbenchTabState};

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
        .and_then(|tab| match &tab.state {
            WorkbenchTabState::PodLogs(logs_tab) => {
                if logs_tab.viewer.lines.is_empty() {
                    None
                } else {
                    Some(logs_tab.viewer.lines.join("\n"))
                }
            }
            WorkbenchTabState::WorkloadLogs(wl_tab) => {
                if wl_tab.lines.is_empty() {
                    None
                } else {
                    Some(
                        wl_tab
                            .lines
                            .iter()
                            .map(|l| format!("{}:{} {}", l.pod_name, l.container_name, l.content))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    )
                }
            }
            _ => None,
        });
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
    let export_data = app
        .workbench()
        .active_tab()
        .and_then(|tab| match &tab.state {
            WorkbenchTabState::PodLogs(logs_tab) => {
                if logs_tab.viewer.lines.is_empty() {
                    None
                } else {
                    let label = format!(
                        "{}-{}",
                        logs_tab.viewer.pod_name, logs_tab.viewer.container_name,
                    );
                    Some((label, logs_tab.viewer.lines.join("\n")))
                }
            }
            WorkbenchTabState::WorkloadLogs(wl_tab) => {
                if wl_tab.lines.is_empty() {
                    None
                } else {
                    let label = tab.state.title().replace(' ', "-");
                    let content = wl_tab
                        .lines
                        .iter()
                        .map(|l| format!("{}:{} {}", l.pod_name, l.container_name, l.content))
                        .collect::<Vec<_>>()
                        .join("\n");
                    Some((label, content))
                }
            }
            _ => None,
        });
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
