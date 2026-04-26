//! Resource selection and lookup helpers extracted from the main event loop.

use kubectui::{
    app::{AppState, AppView, DetailViewState, ResourceRef},
    authorization::DetailActionAuthorization,
    bookmarks::{
        bookmark_selected_index, resource_exists, resource_selected_index,
        selected_bookmark_resource,
    },
    cronjob::{cronjob_history_entries, preferred_history_index},
    detail_sections::metadata_for_resource,
    k8s::client::K8sClient,
    policy::{DetailAction, ResourceActionContext},
    state::ClusterSnapshot,
};

use crate::async_types::{DetailAsyncResult, ExtensionFetchResult};

const SELECTION_SEARCH_FALLBACK_STATUS: &str =
    "Selected resource no longer matches search; moved to nearest visible result.";
const SELECTION_SEARCH_NO_VISIBLE_RESULTS_STATUS: &str =
    "Selected resource no longer matches search; no visible results.";

/// Converts the namespace string to `Option`: `"all"` becomes `None`.
pub fn namespace_scope(namespace: &str) -> Option<&str> {
    if namespace == "all" {
        None
    } else {
        Some(namespace)
    }
}

/// Returns the filtered dataset index for the given selection index.
pub fn filtered_index(indices: &[usize], idx: usize) -> Option<usize> {
    indices
        .get(idx.min(indices.len().saturating_sub(1)))
        .copied()
}

/// Resolves the currently selected resource for the active view.
pub fn selected_resource(app: &AppState, snapshot: &ClusterSnapshot) -> Option<ResourceRef> {
    let idx = app.selected_idx();
    match app.view() {
        AppView::Dashboard => None,
        AppView::Projects => {
            let projects = kubectui::projects::compute_projects(snapshot);
            let indices =
                kubectui::projects::filtered_project_indices(&projects, app.search_query());
            filtered_index(&indices, idx).and_then(|i| projects[i].representative.clone())
        }
        AppView::Governance => {
            let summaries = kubectui::governance::compute_governance(snapshot);
            let indices =
                kubectui::governance::filtered_governance_indices(&summaries, app.search_query());
            filtered_index(&indices, idx).and_then(|i| summaries[i].representative.clone())
        }
        AppView::Bookmarks => selected_bookmark_resource(app.bookmarks(), idx, app.search_query()),
        AppView::Vulnerabilities => {
            let findings =
                kubectui::state::vulnerabilities::compute_vulnerability_findings(snapshot);
            let indices = kubectui::state::vulnerabilities::filtered_vulnerability_indices(
                &findings,
                app.search_query(),
            );
            filtered_index(&indices, idx).and_then(|i| findings[i].resource_ref.clone())
        }
        AppView::Issues | AppView::HealthReport => {
            let issues = kubectui::state::issues::compute_issues(snapshot);
            let query = app.search_query().trim();
            let indices = if app.view() == AppView::HealthReport {
                issues
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, issue)| {
                        (issue.source == kubectui::state::issues::ClusterIssueSource::Sanitizer
                            && issue.matches_query(query))
                        .then_some(idx)
                    })
                    .collect()
            } else {
                kubectui::state::issues::filtered_issue_indices(&issues, query)
            };
            filtered_index(&indices, idx).map(|i| issues[i].resource_ref.clone())
        }
        AppView::HelmCharts => None,
        AppView::PortForwarding => None,
        AppView::Extensions => {
            if !app.extension_in_instances {
                return None;
            }
            let crd = app.extension_selected_crd.as_ref().and_then(|crd_name| {
                snapshot
                    .custom_resource_definitions
                    .iter()
                    .find(|c| &c.name == crd_name)
            })?;
            let inst = app.extension_instances.get(
                app.extension_instance_cursor
                    .min(app.extension_instances.len().saturating_sub(1)),
            )?;
            Some(ResourceRef::CustomResource {
                name: inst.name.clone(),
                namespace: inst.namespace.clone(),
                group: crd.group.clone(),
                version: crd.version.clone(),
                kind: crd.kind.clone(),
                plural: crd.plural.clone(),
            })
        }
        AppView::FluxCDAlertProviders
        | AppView::FluxCDAlerts
        | AppView::FluxCDAll
        | AppView::FluxCDArtifacts
        | AppView::FluxCDHelmReleases
        | AppView::FluxCDHelmRepositories
        | AppView::FluxCDImages
        | AppView::FluxCDKustomizations
        | AppView::FluxCDReceivers
        | AppView::FluxCDSources
        | AppView::Nodes
        | AppView::Pods
        | AppView::Services
        | AppView::ResourceQuotas
        | AppView::LimitRanges
        | AppView::PodDisruptionBudgets
        | AppView::Deployments
        | AppView::StatefulSets
        | AppView::DaemonSets
        | AppView::ReplicaSets
        | AppView::ReplicationControllers
        | AppView::Jobs
        | AppView::CronJobs
        | AppView::Endpoints
        | AppView::Ingresses
        | AppView::IngressClasses
        | AppView::GatewayClasses
        | AppView::Gateways
        | AppView::HttpRoutes
        | AppView::GrpcRoutes
        | AppView::ReferenceGrants
        | AppView::NetworkPolicies
        | AppView::ConfigMaps
        | AppView::Secrets
        | AppView::HPAs
        | AppView::PriorityClasses
        | AppView::PersistentVolumeClaims
        | AppView::PersistentVolumes
        | AppView::StorageClasses
        | AppView::Namespaces
        | AppView::Events
        | AppView::ServiceAccounts
        | AppView::Roles
        | AppView::RoleBindings
        | AppView::ClusterRoles
        | AppView::ClusterRoleBindings
        | AppView::HelmReleases => {
            let filtered = kubectui::ui::views::filtering::filtered_indices_for_view(
                app.view(),
                snapshot,
                app.search_query(),
                app.workload_sort(),
                app.pod_sort(),
            );
            let resource_idx = filtered_index(&filtered, idx)?;
            match app.view() {
                AppView::Nodes => {
                    Some(ResourceRef::Node(snapshot.nodes[resource_idx].name.clone()))
                }
                AppView::Pods => {
                    let pod = &snapshot.pods[resource_idx];
                    Some(ResourceRef::Pod(pod.name.clone(), pod.namespace.clone()))
                }
                AppView::Services => {
                    let service = &snapshot.services[resource_idx];
                    Some(ResourceRef::Service(
                        service.name.clone(),
                        service.namespace.clone(),
                    ))
                }
                AppView::ResourceQuotas => {
                    let quota = &snapshot.resource_quotas[resource_idx];
                    Some(ResourceRef::ResourceQuota(
                        quota.name.clone(),
                        quota.namespace.clone(),
                    ))
                }
                AppView::LimitRanges => {
                    let limit_range = &snapshot.limit_ranges[resource_idx];
                    Some(ResourceRef::LimitRange(
                        limit_range.name.clone(),
                        limit_range.namespace.clone(),
                    ))
                }
                AppView::PodDisruptionBudgets => {
                    let pdb = &snapshot.pod_disruption_budgets[resource_idx];
                    Some(ResourceRef::PodDisruptionBudget(
                        pdb.name.clone(),
                        pdb.namespace.clone(),
                    ))
                }
                AppView::Deployments => {
                    let deployment = &snapshot.deployments[resource_idx];
                    Some(ResourceRef::Deployment(
                        deployment.name.clone(),
                        deployment.namespace.clone(),
                    ))
                }
                AppView::StatefulSets => {
                    let statefulset = &snapshot.statefulsets[resource_idx];
                    Some(ResourceRef::StatefulSet(
                        statefulset.name.clone(),
                        statefulset.namespace.clone(),
                    ))
                }
                AppView::DaemonSets => {
                    let daemonset = &snapshot.daemonsets[resource_idx];
                    Some(ResourceRef::DaemonSet(
                        daemonset.name.clone(),
                        daemonset.namespace.clone(),
                    ))
                }
                AppView::ReplicaSets => {
                    let replicaset = &snapshot.replicasets[resource_idx];
                    Some(ResourceRef::ReplicaSet(
                        replicaset.name.clone(),
                        replicaset.namespace.clone(),
                    ))
                }
                AppView::ReplicationControllers => {
                    let controller = &snapshot.replication_controllers[resource_idx];
                    Some(ResourceRef::ReplicationController(
                        controller.name.clone(),
                        controller.namespace.clone(),
                    ))
                }
                AppView::Jobs => {
                    let job = &snapshot.jobs[resource_idx];
                    Some(ResourceRef::Job(job.name.clone(), job.namespace.clone()))
                }
                AppView::CronJobs => {
                    let cronjob = &snapshot.cronjobs[resource_idx];
                    Some(ResourceRef::CronJob(
                        cronjob.name.clone(),
                        cronjob.namespace.clone(),
                    ))
                }
                AppView::Endpoints => {
                    let endpoint = &snapshot.endpoints[resource_idx];
                    Some(ResourceRef::Endpoint(
                        endpoint.name.clone(),
                        endpoint.namespace.clone(),
                    ))
                }
                AppView::Ingresses => {
                    let ingress = &snapshot.ingresses[resource_idx];
                    Some(ResourceRef::Ingress(
                        ingress.name.clone(),
                        ingress.namespace.clone(),
                    ))
                }
                AppView::IngressClasses => Some(ResourceRef::IngressClass(
                    snapshot.ingress_classes[resource_idx].name.clone(),
                )),
                AppView::GatewayClasses => {
                    let class = &snapshot.gateway_classes[resource_idx];
                    Some(ResourceRef::CustomResource {
                        name: class.name.clone(),
                        namespace: None,
                        group: "gateway.networking.k8s.io".to_string(),
                        version: class.version.clone(),
                        kind: "GatewayClass".to_string(),
                        plural: "gatewayclasses".to_string(),
                    })
                }
                AppView::Gateways => {
                    let gateway = &snapshot.gateways[resource_idx];
                    Some(ResourceRef::CustomResource {
                        name: gateway.name.clone(),
                        namespace: Some(gateway.namespace.clone()),
                        group: "gateway.networking.k8s.io".to_string(),
                        version: gateway.version.clone(),
                        kind: "Gateway".to_string(),
                        plural: "gateways".to_string(),
                    })
                }
                AppView::HttpRoutes => {
                    let route = &snapshot.http_routes[resource_idx];
                    Some(ResourceRef::CustomResource {
                        name: route.name.clone(),
                        namespace: Some(route.namespace.clone()),
                        group: "gateway.networking.k8s.io".to_string(),
                        version: route.version.clone(),
                        kind: "HTTPRoute".to_string(),
                        plural: "httproutes".to_string(),
                    })
                }
                AppView::GrpcRoutes => {
                    let route = &snapshot.grpc_routes[resource_idx];
                    Some(ResourceRef::CustomResource {
                        name: route.name.clone(),
                        namespace: Some(route.namespace.clone()),
                        group: "gateway.networking.k8s.io".to_string(),
                        version: route.version.clone(),
                        kind: "GRPCRoute".to_string(),
                        plural: "grpcroutes".to_string(),
                    })
                }
                AppView::ReferenceGrants => {
                    let grant = &snapshot.reference_grants[resource_idx];
                    Some(ResourceRef::CustomResource {
                        name: grant.name.clone(),
                        namespace: Some(grant.namespace.clone()),
                        group: "gateway.networking.k8s.io".to_string(),
                        version: grant.version.clone(),
                        kind: "ReferenceGrant".to_string(),
                        plural: "referencegrants".to_string(),
                    })
                }
                AppView::NetworkPolicies => {
                    let policy = &snapshot.network_policies[resource_idx];
                    Some(ResourceRef::NetworkPolicy(
                        policy.name.clone(),
                        policy.namespace.clone(),
                    ))
                }
                AppView::ConfigMaps => {
                    let config_map = &snapshot.config_maps[resource_idx];
                    Some(ResourceRef::ConfigMap(
                        config_map.name.clone(),
                        config_map.namespace.clone(),
                    ))
                }
                AppView::Secrets => {
                    let secret = &snapshot.secrets[resource_idx];
                    Some(ResourceRef::Secret(
                        secret.name.clone(),
                        secret.namespace.clone(),
                    ))
                }
                AppView::HPAs => {
                    let hpa = &snapshot.hpas[resource_idx];
                    Some(ResourceRef::Hpa(hpa.name.clone(), hpa.namespace.clone()))
                }
                AppView::PriorityClasses => Some(ResourceRef::PriorityClass(
                    snapshot.priority_classes[resource_idx].name.clone(),
                )),
                AppView::PersistentVolumeClaims => {
                    let pvc = &snapshot.pvcs[resource_idx];
                    Some(ResourceRef::Pvc(pvc.name.clone(), pvc.namespace.clone()))
                }
                AppView::PersistentVolumes => {
                    Some(ResourceRef::Pv(snapshot.pvs[resource_idx].name.clone()))
                }
                AppView::StorageClasses => Some(ResourceRef::StorageClass(
                    snapshot.storage_classes[resource_idx].name.clone(),
                )),
                AppView::Namespaces => Some(ResourceRef::Namespace(
                    snapshot.namespace_list[resource_idx].name.clone(),
                )),
                AppView::Events => {
                    let event = &snapshot.events[resource_idx];
                    Some(ResourceRef::Event(
                        event.name.clone(),
                        event.namespace.clone(),
                    ))
                }
                AppView::ServiceAccounts => {
                    let service_account = &snapshot.service_accounts[resource_idx];
                    Some(ResourceRef::ServiceAccount(
                        service_account.name.clone(),
                        service_account.namespace.clone(),
                    ))
                }
                AppView::Roles => {
                    let role = &snapshot.roles[resource_idx];
                    Some(ResourceRef::Role(role.name.clone(), role.namespace.clone()))
                }
                AppView::RoleBindings => {
                    let role_binding = &snapshot.role_bindings[resource_idx];
                    Some(ResourceRef::RoleBinding(
                        role_binding.name.clone(),
                        role_binding.namespace.clone(),
                    ))
                }
                AppView::ClusterRoles => Some(ResourceRef::ClusterRole(
                    snapshot.cluster_roles[resource_idx].name.clone(),
                )),
                AppView::ClusterRoleBindings => Some(ResourceRef::ClusterRoleBinding(
                    snapshot.cluster_role_bindings[resource_idx].name.clone(),
                )),
                AppView::HelmReleases => {
                    let release = &snapshot.helm_releases[resource_idx];
                    Some(ResourceRef::HelmRelease(
                        release.name.clone(),
                        release.namespace.clone(),
                    ))
                }
                AppView::FluxCDAlertProviders
                | AppView::FluxCDAlerts
                | AppView::FluxCDAll
                | AppView::FluxCDArtifacts
                | AppView::FluxCDHelmReleases
                | AppView::FluxCDHelmRepositories
                | AppView::FluxCDImages
                | AppView::FluxCDKustomizations
                | AppView::FluxCDReceivers
                | AppView::FluxCDSources => {
                    snapshot
                        .flux_resources
                        .get(resource_idx)
                        .map(|r| ResourceRef::CustomResource {
                            name: r.name.clone(),
                            namespace: r.namespace.clone(),
                            group: r.group.clone(),
                            version: r.version.clone(),
                            kind: r.kind.clone(),
                            plural: r.plural.clone(),
                        })
                }
                _ => None,
            }
        }
    }
}

/// Preserves the highlighted resource across watch or refresh reorders.
pub fn preserve_selection_identity_after_snapshot_change(
    app: &mut AppState,
    previous: &ClusterSnapshot,
    current: &ClusterSnapshot,
) -> bool {
    let Some(selected) = selected_resource(app, previous) else {
        return clear_selection_search_fallback_status_if_results_visible(app, current);
    };

    let next_idx = resource_selected_index(
        app.view(),
        current,
        &selected,
        app.search_query(),
        app.workload_sort(),
        app.pod_sort(),
    );

    let Some(next_idx) = next_idx else {
        let indices = kubectui::ui::views::filtering::filtered_indices_for_view(
            app.view(),
            current,
            app.search_query(),
            app.workload_sort(),
            app.pod_sort(),
        );
        if resource_exists(current, &selected) && !app.search_query().trim().is_empty() {
            let status = if indices.is_empty() {
                SELECTION_SEARCH_NO_VISIBLE_RESULTS_STATUS
            } else {
                SELECTION_SEARCH_FALLBACK_STATUS
            };
            app.set_status(status.to_string());
        }
        let clamped_idx = app.selected_idx().min(indices.len().saturating_sub(1));
        app.selected_idx = clamped_idx;
        reset_content_detail_scroll_if_selection_changed(app, current, &selected);
        close_stale_detail_after_selection_change(app, current);
        return true;
    };

    let selection_changed = app.selected_idx != next_idx;

    app.selected_idx = next_idx;
    let status_cleared = clear_selection_search_fallback_status(app);
    let detail_changed = close_stale_detail_after_selection_change(app, current);
    selection_changed || detail_changed || status_cleared
}

fn clear_selection_search_fallback_status(app: &mut AppState) -> bool {
    if !matches!(
        app.status_message(),
        Some(SELECTION_SEARCH_FALLBACK_STATUS | SELECTION_SEARCH_NO_VISIBLE_RESULTS_STATUS)
    ) {
        return false;
    }

    app.clear_status();
    true
}

fn clear_selection_search_fallback_status_if_results_visible(
    app: &mut AppState,
    current: &ClusterSnapshot,
) -> bool {
    if !matches!(
        app.status_message(),
        Some(SELECTION_SEARCH_FALLBACK_STATUS | SELECTION_SEARCH_NO_VISIBLE_RESULTS_STATUS)
    ) {
        return false;
    }

    let indices = kubectui::ui::views::filtering::filtered_indices_for_view(
        app.view(),
        current,
        app.search_query(),
        app.workload_sort(),
        app.pod_sort(),
    );
    if indices.is_empty() {
        return false;
    }

    app.clear_status();
    true
}

fn reset_content_detail_scroll_if_selection_changed(
    app: &mut AppState,
    current: &ClusterSnapshot,
    previous_selected: &ResourceRef,
) -> bool {
    if selected_resource(app, current).as_ref() == Some(previous_selected) {
        return false;
    }

    let changed = app.content_detail_scroll != 0;
    app.content_detail_scroll = 0;
    changed
}

fn close_stale_detail_after_selection_change(
    app: &mut AppState,
    current: &ClusterSnapshot,
) -> bool {
    let Some(detail_resource) = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.as_ref())
    else {
        return false;
    };

    let active_view_matches_detail = if app.view().is_fluxcd() {
        is_flux_custom_resource_ref(detail_resource)
    } else {
        detail_resource.primary_view() == Some(app.view())
    };
    if !active_view_matches_detail {
        return false;
    }

    if !resource_exists(current, detail_resource) {
        app.detail_view = None;
        return true;
    }

    if selected_resource(app, current).as_ref() == Some(detail_resource) {
        return false;
    }

    app.detail_view = None;
    true
}

fn is_flux_custom_resource_ref(resource: &ResourceRef) -> bool {
    matches!(
        resource,
        ResourceRef::CustomResource { group, .. } if group.ends_with(".fluxcd.io")
    )
}

/// Returns the resource context (with node/cronjob metadata) for the selection.
pub fn resource_action_context(
    snapshot: &ClusterSnapshot,
    resource: ResourceRef,
) -> ResourceActionContext {
    let (effective_logs_resource, cronjob_history_logs_available) = match &resource {
        ResourceRef::CronJob(name, ns) => snapshot
            .cronjobs
            .iter()
            .find(|cronjob| &cronjob.name == name && &cronjob.namespace == ns)
            .map(|cronjob| {
                let history = cronjob_history_entries(cronjob, &snapshot.jobs, &snapshot.pods);
                let selected = history.get(preferred_history_index(&history)).cloned();
                (
                    selected.as_ref().map(|entry| {
                        ResourceRef::Job(entry.job_name.clone(), entry.namespace.clone())
                    }),
                    selected.is_some_and(|entry| entry.has_log_target()),
                )
            })
            .unwrap_or((None, false)),
        _ => (None, false),
    };
    let node_unschedulable = match &resource {
        ResourceRef::Node(name) => snapshot
            .nodes
            .iter()
            .find(|node| &node.name == name)
            .map(|node| node.unschedulable),
        _ => None,
    };
    let cronjob_suspended = match &resource {
        ResourceRef::CronJob(name, ns) => snapshot
            .cronjobs
            .iter()
            .find(|cronjob| &cronjob.name == name && &cronjob.namespace == ns)
            .map(|cronjob| cronjob.suspend),
        _ => None,
    };
    ResourceActionContext {
        resource,
        node_unschedulable,
        cronjob_suspended,
        cronjob_history_logs_available,
        effective_logs_resource,
        effective_logs_authorization: None,
        action_authorizations: Default::default(),
    }
}

pub fn selected_resource_context(
    app: &AppState,
    snapshot: &ClusterSnapshot,
) -> Option<ResourceActionContext> {
    let resource = selected_resource(app, snapshot)?;
    Some(resource_action_context(snapshot, resource))
}

/// Looks up cached RBAC authorization for a detail action on the given resource.
pub fn cached_detail_action_authorization(
    app: &AppState,
    resource: &ResourceRef,
    action: DetailAction,
) -> Option<DetailActionAuthorization> {
    app.detail_view
        .as_ref()
        .filter(|detail| detail.resource.as_ref() == Some(resource))
        .and_then(|detail| detail.metadata.action_authorizations.get(&action).copied())
}

fn resolve_detail_action_authorization(
    cached: Option<DetailActionAuthorization>,
    live: Option<DetailActionAuthorization>,
) -> Option<DetailActionAuthorization> {
    cached.or(live)
}

/// Resolves the canonical tri-state authorization for a detail action.
pub async fn detail_action_authorization(
    app: &AppState,
    client: &K8sClient,
    resource: &ResourceRef,
    action: DetailAction,
) -> Option<DetailActionAuthorization> {
    let cached = cached_detail_action_authorization(app, resource, action);
    let live = if cached.is_some() {
        None
    } else {
        client.is_detail_action_authorized(resource, action).await
    };

    resolve_detail_action_authorization(cached, live)
}

/// Builds a human-readable denial message for the given action and resource.
pub fn detail_action_denied_message(
    action: DetailAction,
    resource: &ResourceRef,
    status: DetailActionAuthorization,
) -> String {
    match status {
        DetailActionAuthorization::Denied => format!(
            "{} is not allowed for {} '{}'.",
            action.label(),
            resource.kind(),
            resource.name()
        ),
        DetailActionAuthorization::Unknown => format!(
            "{} requires verified authorization for {} '{}', but access could not be confirmed.",
            action.label(),
            resource.kind(),
            resource.name()
        ),
        DetailActionAuthorization::Allowed => format!(
            "{} is already allowed for {} '{}'.",
            action.label(),
            resource.kind(),
            resource.name()
        ),
    }
}

/// Navigates to the bookmarked resource's primary view and returns the ref.
pub fn prepare_resource_target(
    app: &mut AppState,
    snapshot: &ClusterSnapshot,
    resource: &ResourceRef,
) -> Result<(), String> {
    if !resource_exists(snapshot, resource) {
        return Err(format!(
            "{} '{}' is no longer present in the current snapshot.",
            resource.kind(),
            resource.name()
        ));
    }

    app.search_query.clear();
    app.is_search_mode = false;

    let Some(view) = resource.primary_view() else {
        return Err(format!(
            "{} '{}' does not map to a navigable primary view.",
            resource.kind(),
            resource.name()
        ));
    };

    app.navigate_to_view(view);
    app.focus = kubectui::app::Focus::Content;
    app.extension_in_instances = false;
    if let Some(selected_idx) = bookmark_selected_index(
        view,
        snapshot,
        resource,
        app.workload_sort(),
        app.pod_sort(),
    ) {
        app.selected_idx = selected_idx;
    }
    configure_extension_target_selection(app, snapshot, resource);

    Ok(())
}

fn configure_extension_target_selection(
    app: &mut AppState,
    snapshot: &ClusterSnapshot,
    resource: &ResourceRef,
) {
    let ResourceRef::CustomResource {
        name,
        namespace,
        group,
        version,
        kind,
        plural,
    } = resource
    else {
        return;
    };
    if app.view() != AppView::Extensions {
        return;
    }

    let Some((crd_idx, crd)) = snapshot
        .custom_resource_definitions
        .iter()
        .enumerate()
        .find(|(_, crd)| {
            crd.group == *group
                && crd.version == *version
                && crd.kind == *kind
                && crd.plural == *plural
        })
    else {
        return;
    };

    app.selected_idx = crd_idx;
    let same_crd = app.extension_selected_crd.as_deref() == Some(crd.name.as_str());
    app.extension_selected_crd = Some(crd.name.clone());
    app.extension_in_instances = false;
    app.extension_instance_cursor = 0;
    if !same_crd {
        app.extension_instances.clear();
        app.extension_error = None;
        return;
    }

    if let Some(instance_idx) = app
        .extension_instances
        .iter()
        .position(|instance| instance.name == *name && instance.namespace == *namespace)
    {
        app.extension_instance_cursor = instance_idx;
        app.extension_in_instances = true;
    }
}

/// Navigates to the bookmarked resource's primary view and returns the ref.
pub fn prepare_bookmark_target(
    app: &mut AppState,
    snapshot: &ClusterSnapshot,
) -> Result<ResourceRef, String> {
    let resource = app
        .selected_bookmark_resource()
        .ok_or_else(|| "No bookmarked resource is selected.".to_string())?;

    prepare_resource_target(app, snapshot, &resource).map_err(|err| format!("Bookmarked {err}"))?;

    Ok(resource)
}

/// Resolves the selected Flux resource for reconcile, validating it is eligible.
pub fn selected_flux_reconcile_resource(
    app: &AppState,
    snapshot: &ClusterSnapshot,
) -> Result<ResourceRef, String> {
    let resource = app
        .detail_view
        .as_ref()
        .and_then(|detail| detail.resource.clone())
        .or_else(|| selected_resource(app, snapshot))
        .ok_or_else(|| "No Flux resource is selected.".to_string())?;

    if let Some(reason) = resource.flux_reconcile_disabled_reason() {
        return Err(reason.to_string());
    }

    let ResourceRef::CustomResource {
        name,
        namespace,
        group,
        version,
        kind,
        plural,
    } = &resource
    else {
        return Err("Flux reconcile is only available for Flux toolkit resources.".to_string());
    };

    let is_suspended = snapshot.flux_resources.iter().any(|candidate| {
        candidate.name == *name
            && candidate.namespace == *namespace
            && candidate.group == *group
            && candidate.version == *version
            && candidate.kind == *kind
            && candidate.plural == *plural
            && candidate.suspended
    });

    if is_suspended {
        return Err(format!(
            "Flux reconcile is unavailable because {kind} '{name}' is suspended."
        ));
    }

    Ok(resource)
}

/// Creates a loading-state `DetailViewState` for the given resource.
pub fn initial_loading_state(resource: ResourceRef, snapshot: &ClusterSnapshot) -> DetailViewState {
    let metadata = metadata_for_resource(snapshot, &resource);
    DetailViewState {
        resource: Some(resource),
        pending_request_id: None,
        metadata,
        loading: true,
        cronjob_history: Vec::new(),
        cronjob_history_selected: 0,
        confirm_cronjob_suspend: None,
        ..DetailViewState::default()
    }
}

/// Opens the detail view for a resource, spawning the async fetch.
pub fn open_detail_for_resource(
    app: &mut AppState,
    snapshot: &ClusterSnapshot,
    client: &K8sClient,
    detail_tx: &tokio::sync::mpsc::Sender<DetailAsyncResult>,
    resource: ResourceRef,
    detail_request_seq: &mut u64,
) {
    app.record_recent_resource_jump(resource.clone());
    let request_id = super::next_request_id(detail_request_seq);
    let mut state = initial_loading_state(resource.clone(), snapshot);
    state.pending_request_id = Some(request_id);
    app.detail_view = Some(state);
    let client_clone = client.clone();
    let snapshot_clone = snapshot.clone();
    let tx = detail_tx.clone();
    let requested_resource = resource.clone();
    tokio::spawn(async move {
        let result =
            super::fetch_detail_view(&client_clone, &snapshot_clone, requested_resource.clone())
                .await
                .map_err(|err| err.to_string());
        let _ = tx
            .send(DetailAsyncResult {
                request_id,
                resource: requested_resource,
                result,
            })
            .await;
    });
}

/// Selects the extension CRD at the current cursor position.
pub fn selected_extension_crd<'a>(
    app: &AppState,
    snapshot: &'a ClusterSnapshot,
) -> Option<&'a kubectui::k8s::dtos::CustomResourceDefinitionInfo> {
    kubectui::ui::views::extensions::crds::selected_crd(
        &snapshot.custom_resource_definitions,
        app.search_query(),
        app.selected_idx(),
    )
}

/// Spawns an async fetch for extension CRD instances when the Extensions view is active.
pub fn spawn_extensions_fetch(
    client: &K8sClient,
    app: &mut AppState,
    snapshot: &ClusterSnapshot,
    tx: &tokio::sync::mpsc::Sender<ExtensionFetchResult>,
) {
    if app.view() != AppView::Extensions {
        return;
    }

    let Some(crd) = selected_extension_crd(app, snapshot).cloned() else {
        app.extension_instances.clear();
        app.extension_error = None;
        app.extension_selected_crd = None;
        return;
    };

    if app.extension_selected_crd.as_deref() == Some(crd.name.as_str()) {
        return;
    }

    app.begin_extension_instances_load(crd.name.clone());

    let namespace_owned = if crd.scope.eq_ignore_ascii_case("Namespaced") {
        namespace_scope(app.get_namespace()).map(ToString::to_string)
    } else {
        None
    };

    let client = client.clone();
    let crd = crd.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = client
            .fetch_custom_resources(&crd, namespace_owned.as_deref())
            .await
            .map_err(|e| e.to_string());
        let _ = tx
            .send(ExtensionFetchResult {
                crd_name: crd.name.clone(),
                result,
            })
            .await;
    });
}

/// Applies the result of an extension CRD instance fetch to app state.
pub fn apply_extension_fetch_result(app: &mut AppState, result: ExtensionFetchResult) {
    if app.extension_selected_crd.as_deref() != Some(result.crd_name.as_str()) {
        return;
    }

    match result.result {
        Ok(items) => app.set_extension_instances(result.crd_name, items, None),
        Err(err) => app.set_extension_instances(result.crd_name, Vec::new(), Some(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_detail_action_authorization;
    use kubectui::authorization::DetailActionAuthorization;

    #[test]
    fn resolve_authorization_prefers_cached_value() {
        assert_eq!(
            resolve_detail_action_authorization(
                Some(DetailActionAuthorization::Allowed),
                Some(DetailActionAuthorization::Denied),
            ),
            Some(DetailActionAuthorization::Allowed)
        );
        assert_eq!(
            resolve_detail_action_authorization(
                Some(DetailActionAuthorization::Denied),
                Some(DetailActionAuthorization::Allowed),
            ),
            Some(DetailActionAuthorization::Denied)
        );
    }

    #[test]
    fn resolve_authorization_uses_live_when_cache_missing() {
        assert_eq!(
            resolve_detail_action_authorization(None, Some(DetailActionAuthorization::Allowed),),
            Some(DetailActionAuthorization::Allowed)
        );
        assert_eq!(
            resolve_detail_action_authorization(None, Some(DetailActionAuthorization::Unknown),),
            Some(DetailActionAuthorization::Unknown)
        );
    }

    #[test]
    fn resolve_authorization_preserves_unknown_when_no_signal_exists() {
        assert_eq!(resolve_detail_action_authorization(None, None), None);
    }

    #[test]
    fn resolve_authorization_cached_unknown_wins_over_live_allowed() {
        assert_eq!(
            resolve_detail_action_authorization(
                Some(DetailActionAuthorization::Unknown),
                Some(DetailActionAuthorization::Allowed),
            ),
            Some(DetailActionAuthorization::Unknown)
        );
    }

    #[test]
    fn resolve_authorization_cached_denied_wins_over_live_allowed() {
        assert_eq!(
            resolve_detail_action_authorization(
                Some(DetailActionAuthorization::Denied),
                Some(DetailActionAuthorization::Allowed),
            ),
            Some(DetailActionAuthorization::Denied)
        );
    }

    // ── detail_action_denied_message ─────────────────────────────────

    #[test]
    fn denied_message_contains_action_label_and_resource_info() {
        use super::detail_action_denied_message;
        use kubectui::app::ResourceRef;
        use kubectui::policy::DetailAction;

        let resource = ResourceRef::Pod("api-0".to_string(), "default".to_string());
        let msg = detail_action_denied_message(
            DetailAction::Exec,
            &resource,
            DetailActionAuthorization::Denied,
        );
        assert!(msg.contains("not allowed"), "msg = {msg}");
        assert!(msg.contains("api-0"), "msg = {msg}");
    }

    #[test]
    fn unknown_message_mentions_verified_authorization() {
        use super::detail_action_denied_message;
        use kubectui::app::ResourceRef;
        use kubectui::policy::DetailAction;

        let resource = ResourceRef::Node("node-0".to_string());
        let msg = detail_action_denied_message(
            DetailAction::Drain,
            &resource,
            DetailActionAuthorization::Unknown,
        );
        assert!(msg.contains("verified authorization"), "msg = {msg}");
        assert!(msg.contains("node-0"), "msg = {msg}");
    }

    #[test]
    fn allowed_message_is_informational() {
        use super::detail_action_denied_message;
        use kubectui::app::ResourceRef;
        use kubectui::policy::DetailAction;

        let resource = ResourceRef::Pod("api-0".to_string(), "default".to_string());
        let msg = detail_action_denied_message(
            DetailAction::Logs,
            &resource,
            DetailActionAuthorization::Allowed,
        );
        assert!(msg.contains("already allowed"), "msg = {msg}");
    }
}
