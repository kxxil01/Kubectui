use super::*;
use crate::{
    authorization::ActionAccessReview,
    k8s::helm::HelmHistoryResult,
    k8s::rollout::RolloutInspection,
    network_policy_analysis::NetworkPolicyAnalysis,
    policy::DetailAction,
    rbac_subjects::SubjectAccessReview,
    resource_diff::ResourceDiffResult,
    runbooks::LoadedRunbook,
    traffic_debug::TrafficDebugAnalysis,
    ui::components::scale_dialog::ScaleTargetKind,
    workbench::{
        AccessReviewTabState, AiAnalysisTabState, AttemptedActionReview, ConnectivityTabState,
        ConnectivityTargetOption, ExtensionOutputTabState, HelmHistoryTabState,
        NetworkPolicyTabState, ResourceDiffTabState, RolloutTabState, RunbookTabState,
        TrafficDebugTabState,
    },
};

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

        if detail.debug_dialog.is_some() {
            ActiveComponent::DebugContainer
        } else if detail.node_debug_dialog.is_some() {
            ActiveComponent::NodeDebug
        } else if detail.scale_dialog.is_some() {
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
        let key = WorkbenchTabKey::ResourceYaml(resource.clone());
        if let Some(tab) = self.workbench.find_tab_mut(&key)
            && let WorkbenchTabState::ResourceYaml(tab) = &mut tab.state
        {
            tab.update_content(yaml, error, pending_request_id);
            self.workbench.activate_tab(&key);
            self.focus = Focus::Workbench;
            return;
        }
        let mut tab = ResourceYamlTabState::new(resource);
        tab.yaml = yaml;
        tab.loading = tab.yaml.is_none() && error.is_none();
        tab.error = error;
        tab.pending_request_id = pending_request_id;
        self.workbench
            .open_tab(WorkbenchTabState::ResourceYaml(tab));
        self.focus = Focus::Workbench;
    }

    pub fn open_access_review_tab(
        &mut self,
        resource: ResourceRef,
        context_name: Option<String>,
        namespace_scope: String,
        entries: Vec<ActionAccessReview>,
        subject_review: Option<SubjectAccessReview>,
        attempted_review: Option<AttemptedActionReview>,
    ) {
        let key = WorkbenchTabKey::AccessReview(resource.clone());
        if let Some(tab) = self.workbench.find_tab_mut(&key)
            && let WorkbenchTabState::AccessReview(tab) = &mut tab.state
        {
            tab.refresh_payload(
                context_name,
                namespace_scope,
                entries,
                subject_review,
                attempted_review,
            );
            self.workbench.activate_tab(&key);
            self.focus = Focus::Workbench;
            return;
        }
        self.workbench
            .open_tab(WorkbenchTabState::AccessReview(AccessReviewTabState::new(
                resource,
                context_name,
                namespace_scope,
                entries,
                subject_review,
                attempted_review,
            )));
        self.focus = Focus::Workbench;
    }

    pub fn open_resource_diff_tab(
        &mut self,
        resource: ResourceRef,
        diff: Option<ResourceDiffResult>,
        error: Option<String>,
        pending_request_id: Option<u64>,
    ) {
        let key = WorkbenchTabKey::ResourceDiff(resource.clone());
        if let Some(tab) = self.workbench.find_tab_mut(&key)
            && let WorkbenchTabState::ResourceDiff(tab) = &mut tab.state
        {
            if let Some(diff) = diff {
                tab.apply_result(diff);
            } else if let Some(error) = error {
                tab.set_error(error);
            } else {
                tab.loading = true;
                tab.error = None;
                tab.pending_request_id = pending_request_id;
            }
            self.workbench.activate_tab(&key);
            self.focus = Focus::Workbench;
            return;
        }
        let mut tab = ResourceDiffTabState::new(resource);
        tab.loading = diff.is_none() && error.is_none();
        tab.error = error;
        tab.pending_request_id = pending_request_id;
        if let Some(diff) = diff {
            tab.apply_result(diff);
        }
        self.workbench
            .open_tab(WorkbenchTabState::ResourceDiff(tab));
        self.focus = Focus::Workbench;
    }

    pub fn open_rollout_tab(
        &mut self,
        resource: ResourceRef,
        inspection: Option<RolloutInspection>,
        error: Option<String>,
        pending_request_id: Option<u64>,
    ) {
        let key = WorkbenchTabKey::Rollout(resource.clone());
        if let Some(tab) = self.workbench.find_tab_mut(&key)
            && let WorkbenchTabState::Rollout(tab) = &mut tab.state
        {
            if let Some(inspection) = inspection {
                tab.apply_inspection(inspection);
            } else if let Some(error) = error {
                tab.set_error(error);
            } else {
                tab.loading = true;
                tab.error = None;
                tab.pending_request_id = pending_request_id;
            }
            self.workbench.activate_tab(&key);
            self.focus = Focus::Workbench;
            return;
        }
        let mut tab = RolloutTabState::new(resource);
        tab.loading = inspection.is_none() && error.is_none();
        tab.error = error;
        tab.pending_request_id = pending_request_id;
        if let Some(inspection) = inspection {
            tab.apply_inspection(inspection);
        }
        self.workbench.open_tab(WorkbenchTabState::Rollout(tab));
        self.focus = Focus::Workbench;
    }

    pub fn open_helm_history_tab(
        &mut self,
        resource: ResourceRef,
        history: Option<HelmHistoryResult>,
        error: Option<String>,
        pending_request_id: Option<u64>,
    ) {
        let key = WorkbenchTabKey::HelmHistory(resource.clone());
        if let Some(tab) = self.workbench.find_tab_mut(&key)
            && let WorkbenchTabState::HelmHistory(tab) = &mut tab.state
        {
            if let Some(history) = history {
                tab.apply_history(history);
            } else if let Some(error) = error {
                tab.set_history_error(error);
            } else {
                tab.loading = true;
                tab.error = None;
                tab.pending_history_request_id = pending_request_id;
            }
            self.workbench.activate_tab(&key);
            self.focus = Focus::Workbench;
            return;
        }
        let mut tab = HelmHistoryTabState::new(resource);
        tab.loading = history.is_none() && error.is_none();
        tab.error = error;
        tab.pending_history_request_id = pending_request_id;
        if let Some(history) = history {
            tab.apply_history(history);
        }
        self.workbench.open_tab(WorkbenchTabState::HelmHistory(tab));
        self.focus = Focus::Workbench;
    }

    pub fn open_decoded_secret_tab(
        &mut self,
        resource: ResourceRef,
        source_yaml: Option<String>,
        error: Option<String>,
        pending_request_id: Option<u64>,
    ) {
        let key = WorkbenchTabKey::DecodedSecret(resource.clone());
        if let Some(tab) = self.workbench.find_tab_mut(&key)
            && let WorkbenchTabState::DecodedSecret(tab) = &mut tab.state
        {
            if !tab.has_local_edit_state() {
                tab.source_yaml = source_yaml;
                tab.loading = tab.source_yaml.is_none() && error.is_none();
                tab.error = error;
                tab.pending_request_id = pending_request_id;
                if tab.source_yaml.is_none() && tab.error.is_some() {
                    tab.entries.clear();
                    tab.clamp_selected();
                }
            }
            self.workbench.activate_tab(&key);
            self.focus = Focus::Workbench;
            return;
        }
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
        let key = WorkbenchTabKey::ResourceEvents(resource.clone());
        if let Some(tab) = self.workbench.find_tab_mut(&key)
            && let WorkbenchTabState::ResourceEvents(tab) = &mut tab.state
        {
            if !loading || error.is_some() || !events.is_empty() {
                tab.events = events;
                tab.rebuild_timeline(&self.action_history);
            }
            tab.loading = loading;
            tab.error = error;
            tab.pending_request_id = pending_request_id;
            self.workbench.activate_tab(&key);
            self.focus = Focus::Workbench;
            return;
        }
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

    pub fn open_network_policy_tab(
        &mut self,
        resource: ResourceRef,
        analysis: Option<NetworkPolicyAnalysis>,
        error: Option<String>,
    ) {
        let key = WorkbenchTabKey::NetworkPolicy(resource.clone());
        if let Some(tab) = self.workbench.find_tab_mut(&key)
            && let WorkbenchTabState::NetworkPolicy(tab) = &mut tab.state
        {
            if let Some(analysis) = analysis {
                tab.apply_analysis(analysis);
            } else if let Some(error) = error {
                tab.set_error(error);
            }
            self.workbench.activate_tab(&key);
            self.focus = Focus::Workbench;
            return;
        }
        let mut tab = NetworkPolicyTabState::new(resource);
        if let Some(analysis) = analysis {
            tab.apply_analysis(analysis);
        } else if let Some(error) = error {
            tab.set_error(error);
        }
        self.workbench
            .open_tab(WorkbenchTabState::NetworkPolicy(tab));
        self.focus = Focus::Workbench;
    }

    pub fn open_connectivity_tab(
        &mut self,
        source: ResourceRef,
        targets: Vec<ConnectivityTargetOption>,
    ) {
        if let Some((idx, existing_tab)) =
            self.workbench.tabs.iter_mut().enumerate().find(|(_, tab)| {
                matches!(
                    &tab.state,
                    WorkbenchTabState::Connectivity(existing) if existing.source == source
                )
            })
        {
            let WorkbenchTabState::Connectivity(tab) = &mut existing_tab.state else {
                unreachable!("connectivity tab lookup must return connectivity state");
            };
            tab.apply_targets(targets);
            self.workbench.active_tab = idx;
            self.workbench.open = true;
            self.focus = Focus::Workbench;
            return;
        }
        self.workbench
            .open_tab(WorkbenchTabState::Connectivity(ConnectivityTabState::new(
                source, targets,
            )));
        self.focus = Focus::Workbench;
    }

    pub fn open_traffic_debug_tab(
        &mut self,
        resource: ResourceRef,
        analysis: Option<TrafficDebugAnalysis>,
        error: Option<String>,
    ) {
        let key = WorkbenchTabKey::TrafficDebug(resource.clone());
        if let Some(tab) = self.workbench.find_tab_mut(&key)
            && let WorkbenchTabState::TrafficDebug(tab) = &mut tab.state
        {
            if let Some(analysis) = analysis {
                tab.apply_analysis(analysis);
            } else if let Some(error) = error {
                tab.set_error(error);
            }
            self.workbench.activate_tab(&key);
            self.focus = Focus::Workbench;
            return;
        }
        let mut tab = TrafficDebugTabState::new(resource);
        if let Some(analysis) = analysis {
            tab.apply_analysis(analysis);
        } else if let Some(error) = error {
            tab.set_error(error);
        }
        self.workbench
            .open_tab(WorkbenchTabState::TrafficDebug(tab));
        self.focus = Focus::Workbench;
    }

    pub fn open_runbook_tab(&mut self, runbook: LoadedRunbook, resource: Option<ResourceRef>) {
        let key = WorkbenchTabKey::Runbook(runbook.id.clone(), resource.clone());
        if let Some(tab) = self.workbench.find_tab_mut(&key)
            && let WorkbenchTabState::Runbook(tab) = &mut tab.state
        {
            tab.refresh_runbook(runbook);
            self.workbench.activate_tab(&key);
            self.focus = Focus::Workbench;
            return;
        }
        self.workbench
            .open_tab(WorkbenchTabState::Runbook(Box::new(RunbookTabState::new(
                runbook, resource,
            ))));
        self.focus = Focus::Workbench;
    }

    pub fn open_pod_logs_tab(&mut self, resource: ResourceRef) {
        let key = WorkbenchTabKey::PodLogs(resource.clone());
        if self.workbench.activate_tab(&key) {
            self.focus = Focus::Workbench;
            return;
        }
        self.workbench
            .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(resource)));
        self.focus = Focus::Workbench;
    }

    pub fn open_workload_logs_tab(&mut self, resource: ResourceRef, session_id: u64) {
        let key = WorkbenchTabKey::WorkloadLogs(resource.clone());
        if let Some(tab) = self.workbench.find_tab_mut(&key)
            && let WorkbenchTabState::WorkloadLogs(tab) = &mut tab.state
        {
            tab.restart_session(session_id);
            self.workbench.activate_tab(&key);
            self.focus = Focus::Workbench;
            return;
        }
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
        let key = WorkbenchTabKey::Exec(resource.clone());
        if let Some(tab) = self.workbench.find_tab_mut(&key)
            && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
        {
            exec_tab.restart_session(session_id, pod_name, namespace, None);
            self.workbench.activate_tab(&key);
            self.focus = Focus::Workbench;
            return;
        }
        self.workbench
            .open_tab(WorkbenchTabState::Exec(ExecTabState::new(
                resource, session_id, pod_name, namespace,
            )));
        self.focus = Focus::Workbench;
    }

    pub fn open_exec_tab_for_container(
        &mut self,
        resource: ResourceRef,
        session_id: u64,
        pod_name: String,
        namespace: String,
        container_name: String,
    ) {
        let key = WorkbenchTabKey::Exec(resource.clone());
        if let Some(tab) = self.workbench.find_tab_mut(&key)
            && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
        {
            exec_tab.restart_session(session_id, pod_name, namespace, Some(container_name));
            self.workbench.activate_tab(&key);
            self.focus = Focus::Workbench;
            return;
        }
        let mut tab = ExecTabState::new(resource, session_id, pod_name, namespace);
        tab.preset_container(container_name);
        self.workbench.open_tab(WorkbenchTabState::Exec(tab));
        self.focus = Focus::Workbench;
    }

    pub fn append_exec_banner(
        &mut self,
        resource: &ResourceRef,
        session_id: u64,
        lines: &[String],
    ) {
        if let Some(tab) = self
            .workbench_mut()
            .find_tab_mut(&crate::workbench::WorkbenchTabKey::Exec(resource.clone()))
            && let WorkbenchTabState::Exec(exec_tab) = &mut tab.state
            && exec_tab.session_id == session_id
        {
            exec_tab.append_banner(lines);
        }
    }

    pub fn open_port_forward_tab(
        &mut self,
        resource: Option<ResourceRef>,
        dialog: PortForwardDialog,
    ) {
        if let Some(tab) = self.workbench.find_tab_mut(&WorkbenchTabKey::PortForward)
            && let WorkbenchTabState::PortForward(existing) = &mut tab.state
            && existing.target == resource
        {
            self.workbench.activate_tab(&WorkbenchTabKey::PortForward);
            self.focus = Focus::Workbench;
            return;
        }
        self.workbench
            .open_tab(WorkbenchTabState::PortForward(PortForwardTabState::new(
                resource, dialog,
            )));
        self.focus = Focus::Workbench;
    }

    pub fn open_extension_output_tab(
        &mut self,
        execution_id: u64,
        title: impl Into<String>,
        resource: Option<ResourceRef>,
        mode_label: impl Into<String>,
        command_preview: impl Into<String>,
    ) {
        self.workbench.open_tab(WorkbenchTabState::ExtensionOutput(
            ExtensionOutputTabState::new(
                execution_id,
                title,
                resource,
                mode_label,
                command_preview,
            ),
        ));
        self.focus = Focus::Workbench;
    }

    pub fn open_ai_analysis_tab(
        &mut self,
        execution_id: u64,
        title: impl Into<String>,
        resource: ResourceRef,
    ) {
        self.workbench
            .open_tab(WorkbenchTabState::AiAnalysis(Box::new(
                AiAnalysisTabState::new(execution_id, title, resource),
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

    pub fn begin_probe_panel_refresh(&mut self, request_id: u64) {
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
            match detail.probe_panel.as_mut() {
                Some(panel) if panel.pod_name == pod_name && panel.namespace == namespace => {
                    panel.begin_refresh(request_id);
                }
                _ => {
                    let mut panel = ProbePanelComponentState::new(pod_name, namespace, Vec::new());
                    panel.begin_refresh(request_id);
                    detail.probe_panel = Some(panel);
                }
            }
        }
    }

    pub fn close_probe_panel(&mut self) {
        if let Some(detail) = &mut self.detail_view {
            detail.probe_panel = None;
        }
    }

    pub fn toggle_active_log_correlation(&mut self) -> Result<String, String> {
        let message = {
            let Some(tab) = self.workbench.active_tab_mut() else {
                return Err("No active workbench tab.".to_string());
            };

            match &mut tab.state {
                WorkbenchTabState::PodLogs(tab) => {
                    match tab.viewer.toggle_correlation_on_current_line()? {
                        Some(request_id) => format!("Correlating pod logs on req={request_id}"),
                        None => "Cleared pod log correlation".to_string(),
                    }
                }
                WorkbenchTabState::WorkloadLogs(tab) => match tab
                    .toggle_correlation_on_current_line()?
                {
                    Some(request_id) => format!("Correlating workload logs on req={request_id}"),
                    None => "Cleared workload log correlation".to_string(),
                },
                _ => return Err("Log correlation is only available from log tabs.".to_string()),
            }
        };
        self.set_status(message.clone());
        Ok(message)
    }
}
