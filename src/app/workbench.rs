use super::*;
use crate::{policy::DetailAction, ui::components::scale_dialog::ScaleTargetKind};

impl AppState {
    pub fn active_component(&self) -> ActiveComponent {
        if let Some(tab) = self.workbench.active_tab() {
            match tab.state {
                WorkbenchTabState::PodLogs(_) if self.focus == Focus::Workbench => {
                    return ActiveComponent::LogsViewer;
                }
                WorkbenchTabState::PortForward(_) if self.focus == Focus::Workbench => {
                    return ActiveComponent::PortForward;
                }
                _ => {}
            }
        }

        let Some(detail) = &self.detail_view else {
            return ActiveComponent::None;
        };

        if detail.scale_dialog.is_some() {
            ActiveComponent::Scale
        } else if detail.probe_panel.is_some() {
            ActiveComponent::ProbePanel
        } else {
            ActiveComponent::None
        }
    }

    pub fn open_logs_viewer(&mut self) {
        if let Some(detail) = &self.detail_view
            && let Some(resource) = detail.selected_logs_resource()
        {
            match resource {
                pod @ ResourceRef::Pod(_, _) => self.open_pod_logs_tab(pod),
                workload => self.open_workload_logs_tab(workload, 0),
            }
        }
    }

    pub fn close_logs_viewer(&mut self) {
        if matches!(
            self.workbench.active_tab().map(|tab| &tab.state),
            Some(WorkbenchTabState::PodLogs(_))
        ) {
            self.workbench_close_active_tab();
        }
        self.blur_workbench();
    }

    pub fn open_port_forward(&mut self) {
        if let Some(detail) = &self.detail_view
            && let Some(ResourceRef::Pod(name, namespace)) = detail.resource.as_ref()
        {
            self.open_port_forward_tab(
                Some(ResourceRef::Pod(name.clone(), namespace.clone())),
                PortForwardDialog::with_target(namespace, name, 0),
            );
        }
    }

    pub fn close_port_forward(&mut self) {
        if matches!(
            self.workbench.active_tab().map(|tab| &tab.state),
            Some(WorkbenchTabState::PortForward(_))
        ) {
            self.workbench_close_active_tab();
        }
        self.blur_workbench();
    }

    pub(crate) fn focus_workbench(&mut self) {
        if self.workbench.open && !self.workbench.tabs.is_empty() {
            self.focus = Focus::Workbench;
        }
    }

    pub(crate) fn blur_workbench(&mut self) {
        if self.focus == Focus::Workbench {
            self.focus = Focus::Content;
        }
    }

    pub fn open_resource_yaml_tab(
        &mut self,
        resource: ResourceRef,
        yaml: Option<String>,
        error: Option<String>,
        pending_request_id: Option<u64>,
    ) {
        let mut tab = ResourceYamlTabState::new(resource);
        tab.yaml = yaml;
        tab.loading = tab.yaml.is_none() && error.is_none();
        tab.error = error;
        tab.pending_request_id = pending_request_id;
        self.workbench
            .open_tab(WorkbenchTabState::ResourceYaml(tab));
        self.focus = Focus::Workbench;
    }

    pub fn open_decoded_secret_tab(
        &mut self,
        resource: ResourceRef,
        source_yaml: Option<String>,
        error: Option<String>,
        pending_request_id: Option<u64>,
    ) {
        let mut tab = DecodedSecretTabState::new(resource);
        tab.source_yaml = source_yaml;
        tab.loading = tab.source_yaml.is_none() && error.is_none();
        tab.error = error;
        tab.pending_request_id = pending_request_id;
        self.workbench
            .open_tab(WorkbenchTabState::DecodedSecret(tab));
        self.focus = Focus::Workbench;
    }

    pub fn open_resource_events_tab(
        &mut self,
        resource: ResourceRef,
        events: Vec<EventInfo>,
        loading: bool,
        error: Option<String>,
        pending_request_id: Option<u64>,
    ) {
        let mut tab = ResourceEventsTabState::new(resource);
        tab.events = events;
        tab.loading = loading;
        tab.error = error;
        tab.pending_request_id = pending_request_id;
        tab.rebuild_timeline(&self.action_history);
        self.workbench
            .open_tab(WorkbenchTabState::ResourceEvents(tab));
        self.focus = Focus::Workbench;
    }

    pub fn open_pod_logs_tab(&mut self, resource: ResourceRef) {
        self.workbench
            .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(resource)));
        self.focus = Focus::Workbench;
    }

    pub fn open_workload_logs_tab(&mut self, resource: ResourceRef, session_id: u64) {
        self.workbench
            .open_tab(WorkbenchTabState::WorkloadLogs(WorkloadLogsTabState::new(
                resource, session_id,
            )));
        self.focus = Focus::Workbench;
    }

    pub fn open_exec_tab(
        &mut self,
        resource: ResourceRef,
        session_id: u64,
        pod_name: String,
        namespace: String,
    ) {
        self.workbench
            .open_tab(WorkbenchTabState::Exec(ExecTabState::new(
                resource, session_id, pod_name, namespace,
            )));
        self.focus = Focus::Workbench;
    }

    pub fn open_port_forward_tab(
        &mut self,
        resource: Option<ResourceRef>,
        dialog: PortForwardDialog,
    ) {
        self.workbench
            .open_tab(WorkbenchTabState::PortForward(PortForwardTabState::new(
                resource, dialog,
            )));
        self.focus = Focus::Workbench;
    }

    /// Convenience initializer used by tests and non-runtime callers.
    /// The runtime path in `main.rs` overrides this with snapshot-derived replicas.
    pub fn open_scale_dialog(&mut self) {
        if let Some(detail) = &mut self.detail_view
            && detail.supports_action(DetailAction::Scale)
        {
            let (target_kind, name, namespace, current_replicas) = detail
                .resource
                .as_ref()
                .and_then(|r| match r {
                    ResourceRef::Deployment(name, ns) => {
                        Some((ScaleTargetKind::Deployment, name.clone(), ns.clone(), 1i32))
                    }
                    ResourceRef::StatefulSet(name, ns) => {
                        Some((ScaleTargetKind::StatefulSet, name.clone(), ns.clone(), 1i32))
                    }
                    _ => None,
                })
                .unwrap_or((
                    ScaleTargetKind::Deployment,
                    String::new(),
                    "default".to_string(),
                    1,
                ));
            detail.scale_dialog = Some(ScaleDialogState::new(
                target_kind,
                name,
                namespace,
                current_replicas,
            ));
        }
    }

    pub fn close_scale_dialog(&mut self) {
        if let Some(detail) = self.detail_view.as_mut() {
            detail.scale_dialog = None;
        }
    }

    pub fn open_probe_panel(&mut self) {
        if let Some(detail) = &mut self.detail_view
            && detail.supports_action(DetailAction::Probes)
        {
            let (pod_name, namespace) = detail
                .resource
                .as_ref()
                .and_then(|r| match r {
                    ResourceRef::Pod(name, ns) => Some((name.clone(), ns.clone())),
                    _ => None,
                })
                .unwrap_or_default();
            detail.probe_panel = Some(ProbePanelComponentState::new(
                pod_name,
                namespace,
                Vec::new(),
            ));
        }
    }

    pub fn close_probe_panel(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.probe_panel = None;
        }
    }
}
