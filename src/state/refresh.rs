//! Refresh pipeline: parallel resource fetching and snapshot assembly.

use anyhow::{Result, anyhow};
use std::{collections::HashSet, sync::Arc, time::Duration};

use super::fetch::{
    CORE_FETCH_SEMAPHORE, SECONDARY_FETCH_SEMAPHORE, apply_optional_fetch_result,
    apply_vec_fetch_result, maybe_fetch,
};
use super::{
    ClusterDataSource, ConnectionHealth, DataPhase, FluxCounts, GlobalState, RefreshOptions,
    RefreshScope, ViewLoadState,
};
use crate::app::AppView;
use crate::k8s::dtos::{ClusterInfo, ClusterVersionInfo, NamespaceInfo, NodeInfo};
use crate::time::now;

impl GlobalState {
    fn mark_refresh_completed(&mut self, options: RefreshOptions, target_view: Option<AppView>) {
        let completed_scope = Self::completed_scope_for_refresh(options, target_view);
        let loaded_scope = self.snapshot.loaded_scope.union(completed_scope);
        let mut changed = false;
        changed |= AppView::tabs().iter().fold(false, |acc, &view| {
            let required_scope = Self::view_ready_scope(view);
            if required_scope.is_empty() || !loaded_scope.contains(required_scope) {
                return acc;
            }
            self.set_view_load_state(view, ViewLoadState::Ready) || acc
        });
        if let Some(view) = target_view {
            let required_scope = Self::view_ready_scope(view);
            if Self::scope_uses_targeted_fetch(required_scope)
                || required_scope.is_empty()
                || loaded_scope.contains(required_scope)
            {
                changed |= self.set_view_load_state(view, ViewLoadState::Ready);
            }
        }
        changed |= self.set_view_load_state(AppView::PortForwarding, ViewLoadState::Ready);

        if changed {
            self.snapshot_dirty = true;
        }
    }

    fn completed_scope_for_refresh(
        options: RefreshOptions,
        target_view: Option<AppView>,
    ) -> RefreshScope {
        let completed = options.completed_scope();
        let Some(view) = target_view else {
            return completed;
        };
        let view_scope = Self::view_ready_scope(view);
        if Self::scope_uses_targeted_fetch(view_scope) {
            completed.without(view_scope)
        } else {
            completed
        }
    }

    const fn scope_uses_targeted_fetch(scope: RefreshScope) -> bool {
        scope.0 == RefreshScope::NETWORK.0
            || scope.0 == RefreshScope::CONFIG.0
            || scope.0 == RefreshScope::STORAGE.0
            || scope.0 == RefreshScope::SECURITY.0
    }

    fn fetch_for_view(scope_enabled: bool, target_view: Option<AppView>, view: AppView) -> bool {
        if !scope_enabled {
            return false;
        }
        let Some(target) = target_view else {
            return true;
        };
        let target_scope = Self::view_ready_scope(target);
        let view_scope = Self::view_ready_scope(view);
        !Self::scope_uses_targeted_fetch(target_scope)
            || target_scope != view_scope
            || target == view
    }

    fn build_cluster_info(
        client: &impl ClusterDataSource,
        nodes: &[NodeInfo],
        pod_count: usize,
        version: ClusterVersionInfo,
    ) -> ClusterInfo {
        ClusterInfo {
            context: client.cluster_context().map(str::to_string),
            server: client.cluster_url().to_string(),
            git_version: Some(version.git_version),
            platform: Some(version.platform),
            node_count: nodes.len(),
            ready_nodes: nodes.iter().filter(|node| node.ready).count(),
            pod_count,
        }
    }

    fn filter_namespace<T, F>(items: Vec<T>, namespace: Option<&str>, namespace_of: F) -> Vec<T>
    where
        F: Fn(&T) -> &str,
    {
        match namespace {
            Some(ns) => items
                .into_iter()
                .filter(|item| namespace_of(item) == ns)
                .collect(),
            None => items,
        }
    }

    pub(super) fn namespace_names_from_list(namespace_list: &[NamespaceInfo]) -> Vec<String> {
        let mut names: Vec<String> = namespace_list
            .iter()
            .map(|ns| ns.name.clone())
            .filter(|name| !name.is_empty())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Refreshes core resources in parallel, updating status and timestamps.
    ///
    /// Production hardening behavior:
    /// - Per-resource timeout protection (10s)
    /// - Global refresh timeout (60s) prevents indefinite hangs
    /// - Graceful degradation for partial API failures
    /// - Returns error only when all critical resources fail
    pub async fn refresh<D>(&mut self, client: &D, namespace: Option<&str>) -> Result<()>
    where
        D: ClusterDataSource + Sync,
    {
        self.refresh_with_options(client, namespace, RefreshOptions::default())
            .await
    }

    /// Refreshes core resources with runtime options for expensive view-specific data.
    pub async fn refresh_with_options<D>(
        &mut self,
        client: &D,
        namespace: Option<&str>,
        options: RefreshOptions,
    ) -> Result<()>
    where
        D: ClusterDataSource + Sync,
    {
        self.refresh_view_with_options(client, namespace, options, None)
            .await
    }

    pub async fn refresh_view_with_options<D>(
        &mut self,
        client: &D,
        namespace: Option<&str>,
        options: RefreshOptions,
        target_view: Option<AppView>,
    ) -> Result<()>
    where
        D: ClusterDataSource + Sync,
    {
        match tokio::time::timeout(
            Duration::from_secs(60),
            self.refresh_inner(client, namespace, options, target_view),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                let snap = Arc::make_mut(&mut self.snapshot);
                snap.phase = DataPhase::Error;
                snap.last_error = Some("Global refresh timed out (60s)".to_string());
                self.snapshot_dirty = true;
                self.publish_snapshot();
                Err(anyhow!("Global refresh timed out (60s)"))
            }
        }
    }

    async fn refresh_inner<D>(
        &mut self,
        client: &D,
        namespace: Option<&str>,
        options: RefreshOptions,
        target_view: Option<AppView>,
    ) -> Result<()>
    where
        D: ClusterDataSource + Sync,
    {
        if self.snapshot.phase == DataPhase::Loading {
            return Ok(());
        }

        // Trigger copy-on-write — the deep clone happens here (asynchronously,
        // off the main event loop) rather than at the GlobalState::clone() site.
        let snap = Arc::make_mut(&mut self.snapshot);
        snap.phase = DataPhase::Loading;
        snap.last_error = None;
        snap.cluster_url = Some(client.cluster_url().to_string());

        let fetch_nodes = options.scope.intersects(RefreshScope::NODES);
        let fetch_pods = options.scope.intersects(RefreshScope::PODS);
        let fetch_services = options.scope.intersects(RefreshScope::SERVICES);
        let fetch_deployments = options.scope.intersects(RefreshScope::DEPLOYMENTS);
        let fetch_statefulsets = options.scope.intersects(RefreshScope::STATEFULSETS);
        let fetch_daemonsets = options.scope.intersects(RefreshScope::DAEMONSETS);
        let fetch_replicasets = options.scope.intersects(RefreshScope::REPLICASETS);
        let fetch_replication_controllers = options
            .scope
            .intersects(RefreshScope::REPLICATION_CONTROLLERS);
        let fetch_jobs = options.scope.intersects(RefreshScope::JOBS);
        let fetch_cronjobs = options.scope.intersects(RefreshScope::CRONJOBS);
        let fetch_namespaces = options.scope.intersects(RefreshScope::NAMESPACES);
        let fetch_metrics = options.scope.intersects(RefreshScope::METRICS);
        let fetch_network = options.scope.intersects(RefreshScope::NETWORK);
        let fetch_config = options.scope.intersects(RefreshScope::CONFIG);
        let fetch_storage = options.scope.intersects(RefreshScope::STORAGE);
        let fetch_security = options.scope.intersects(RefreshScope::SECURITY);
        let fetch_helm = options.scope.intersects(RefreshScope::HELM);
        let fetch_extensions = options.scope.intersects(RefreshScope::EXTENSIONS);
        let fetch_flux = options.scope.intersects(RefreshScope::FLUX);
        let fetch_local_helm_repositories = options
            .scope
            .intersects(RefreshScope::LOCAL_HELM_REPOSITORIES);
        let fetch_cluster_info = options.include_cluster_info;
        let skip_core = options.skip_core;
        let (wave1, wave2) = tokio::join!(
            async {
                tokio::join!(
                    maybe_fetch(
                        fetch_nodes && !skip_core,
                        "nodes",
                        &CORE_FETCH_SEMAPHORE,
                        || client.fetch_nodes()
                    ),
                    maybe_fetch(
                        fetch_pods && !skip_core,
                        "pods",
                        &CORE_FETCH_SEMAPHORE,
                        || client.fetch_pods(namespace)
                    ),
                    maybe_fetch(
                        fetch_services && !skip_core,
                        "services",
                        &CORE_FETCH_SEMAPHORE,
                        || client.fetch_services(namespace)
                    ),
                    maybe_fetch(
                        fetch_deployments && !skip_core,
                        "deployments",
                        &CORE_FETCH_SEMAPHORE,
                        || client.fetch_deployments(namespace)
                    ),
                    maybe_fetch(
                        fetch_statefulsets && !skip_core,
                        "statefulsets",
                        &CORE_FETCH_SEMAPHORE,
                        || client.fetch_statefulsets(namespace)
                    ),
                    maybe_fetch(
                        fetch_daemonsets && !skip_core,
                        "daemonsets",
                        &CORE_FETCH_SEMAPHORE,
                        || client.fetch_daemonsets(namespace)
                    ),
                    maybe_fetch(
                        fetch_replicasets && !skip_core,
                        "replicasets",
                        &CORE_FETCH_SEMAPHORE,
                        || client.fetch_replicasets(namespace)
                    ),
                    maybe_fetch(
                        fetch_replication_controllers && !skip_core,
                        "replicationcontrollers",
                        &CORE_FETCH_SEMAPHORE,
                        || client.fetch_replication_controllers(namespace)
                    ),
                    maybe_fetch(
                        fetch_jobs && !skip_core,
                        "jobs",
                        &CORE_FETCH_SEMAPHORE,
                        || client.fetch_jobs(namespace)
                    ),
                    maybe_fetch(
                        fetch_cronjobs && !skip_core,
                        "cronjobs",
                        &CORE_FETCH_SEMAPHORE,
                        || client.fetch_cronjobs(namespace)
                    ),
                    maybe_fetch(
                        fetch_namespaces && !skip_core,
                        "namespacelist",
                        &CORE_FETCH_SEMAPHORE,
                        || { client.fetch_namespace_list() }
                    ),
                    maybe_fetch(fetch_flux, "fluxresources", &CORE_FETCH_SEMAPHORE, || {
                        client.fetch_flux_resources(namespace)
                    }),
                    maybe_fetch(fetch_metrics, "cluster info", &CORE_FETCH_SEMAPHORE, || {
                        client.fetch_cluster_version()
                    }),
                    maybe_fetch(
                        fetch_cluster_info && namespace.is_some(),
                        "cluster pod count",
                        &CORE_FETCH_SEMAPHORE,
                        || client.fetch_cluster_pod_count()
                    ),
                )
            },
            async {
                tokio::join!(
                    maybe_fetch(
                        Self::fetch_for_view(fetch_config, target_view, AppView::ResourceQuotas),
                        "resourcequotas",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_resource_quotas(namespace)
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_config, target_view, AppView::LimitRanges),
                        "limitranges",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_limit_ranges(namespace)
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(
                            fetch_config,
                            target_view,
                            AppView::PodDisruptionBudgets
                        ),
                        "pdbs",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || { client.fetch_pod_disruption_budgets(namespace) }
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_security, target_view, AppView::ServiceAccounts),
                        "serviceaccounts",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_service_accounts(namespace)
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_security, target_view, AppView::Roles),
                        "roles",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || { client.fetch_roles(namespace) }
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_security, target_view, AppView::RoleBindings),
                        "rolebindings",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_role_bindings(namespace)
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_security, target_view, AppView::ClusterRoles),
                        "clusterroles",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_cluster_roles()
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(
                            fetch_security,
                            target_view,
                            AppView::ClusterRoleBindings
                        ),
                        "clusterrolebindings",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_cluster_role_bindings()
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_security, target_view, AppView::Vulnerabilities),
                        "vulnerabilityreports",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_vulnerability_reports(namespace)
                    ),
                    maybe_fetch(fetch_extensions, "crds", &SECONDARY_FETCH_SEMAPHORE, || {
                        client.fetch_custom_resource_definitions()
                    }),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_network, target_view, AppView::Endpoints),
                        "endpoints",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || { client.fetch_endpoints(namespace) }
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_network, target_view, AppView::Ingresses),
                        "ingresses",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || { client.fetch_ingresses(namespace) }
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_network, target_view, AppView::IngressClasses),
                        "ingressclasses",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_ingress_classes()
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_network, target_view, AppView::GatewayClasses),
                        "gatewayclasses",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_gateway_classes()
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_network, target_view, AppView::Gateways),
                        "gateways",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_gateways(namespace)
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_network, target_view, AppView::HttpRoutes),
                        "httproutes",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_http_routes(namespace)
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_network, target_view, AppView::GrpcRoutes),
                        "grpcroutes",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_grpc_routes(namespace)
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_network, target_view, AppView::ReferenceGrants),
                        "referencegrants",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_reference_grants(namespace)
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_network, target_view, AppView::NetworkPolicies),
                        "networkpolicies",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_network_policies(namespace)
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_config, target_view, AppView::ConfigMaps),
                        "configmaps",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || { client.fetch_config_maps(namespace) }
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_config, target_view, AppView::Secrets),
                        "secrets",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || { client.fetch_secrets(namespace) }
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_config, target_view, AppView::HPAs),
                        "hpas",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || { client.fetch_hpas(namespace) }
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(
                            fetch_storage,
                            target_view,
                            AppView::PersistentVolumeClaims
                        ),
                        "pvcs",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || { client.fetch_pvcs(namespace) }
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(
                            fetch_storage,
                            target_view,
                            AppView::PersistentVolumes
                        ),
                        "pvs",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || { client.fetch_pvs() }
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_storage, target_view, AppView::StorageClasses),
                        "storageclasses",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_storage_classes()
                    ),
                    maybe_fetch(
                        Self::fetch_for_view(fetch_config, target_view, AppView::PriorityClasses),
                        "priorityclasses",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || client.fetch_priority_classes()
                    ),
                    maybe_fetch(
                        fetch_helm,
                        "helmreleases",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || { client.fetch_helm_releases(namespace) }
                    ),
                    maybe_fetch(
                        fetch_metrics,
                        "nodemetrics",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || { client.fetch_all_node_metrics() }
                    ),
                    maybe_fetch(
                        fetch_metrics,
                        "podmetrics",
                        &SECONDARY_FETCH_SEMAPHORE,
                        || { client.fetch_all_pod_metrics(namespace) }
                    ),
                )
            },
        );

        let (
            (
                nodes_res,
                pods_res,
                services_res,
                deployments_res,
                statefulsets_res,
                daemonsets_res,
                replicasets_res,
                replication_controllers_res,
                jobs_res,
                cronjobs_res,
                namespace_list_res,
                flux_resources_res,
                cluster_info_res,
                cluster_pod_count_res,
            ),
            (
                resource_quotas_res,
                limit_ranges_res,
                pod_disruption_budgets_res,
                service_accounts_res,
                roles_res,
                role_bindings_res,
                cluster_roles_res,
                cluster_role_bindings_res,
                vulnerability_reports_res,
                custom_resource_definitions_res,
                endpoints_res,
                ingresses_res,
                ingress_classes_res,
                gateway_classes_res,
                gateways_res,
                http_routes_res,
                grpc_routes_res,
                reference_grants_res,
                network_policies_res,
                config_maps_res,
                secrets_res,
                hpas_res,
                pvcs_res,
                pvs_res,
                storage_classes_res,
                priority_classes_res,
                helm_releases_res,
                node_metrics_res,
                pod_metrics_res,
            ),
        ) = (wave1, wave2);

        let primary_resource_fetch_succeeded = matches!(nodes_res.as_ref(), Some(Ok(_)))
            || matches!(pods_res.as_ref(), Some(Ok(_)))
            || matches!(services_res.as_ref(), Some(Ok(_)))
            || matches!(deployments_res.as_ref(), Some(Ok(_)))
            || matches!(statefulsets_res.as_ref(), Some(Ok(_)))
            || matches!(daemonsets_res.as_ref(), Some(Ok(_)))
            || matches!(replicasets_res.as_ref(), Some(Ok(_)))
            || matches!(replication_controllers_res.as_ref(), Some(Ok(_)))
            || matches!(jobs_res.as_ref(), Some(Ok(_)))
            || matches!(cronjobs_res.as_ref(), Some(Ok(_)))
            || matches!(namespace_list_res.as_ref(), Some(Ok(_)))
            || matches!(flux_resources_res.as_ref(), Some(Ok(_)));
        let primary_resource_fetch_attempted = nodes_res.is_some()
            || pods_res.is_some()
            || services_res.is_some()
            || deployments_res.is_some()
            || statefulsets_res.is_some()
            || daemonsets_res.is_some()
            || replicasets_res.is_some()
            || replication_controllers_res.is_some()
            || jobs_res.is_some()
            || cronjobs_res.is_some()
            || namespace_list_res.is_some()
            || flux_resources_res.is_some();

        let mut errors = Vec::new();
        let mut total_fetches: usize = 0;

        {
            let snap = Arc::make_mut(&mut self.snapshot);
            apply_vec_fetch_result(
                &mut snap.nodes,
                nodes_res,
                "nodes",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.pods,
                pods_res,
                "pods",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.services,
                services_res,
                "services",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.deployments,
                deployments_res,
                "deployments",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.statefulsets,
                statefulsets_res,
                "statefulsets",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.daemonsets,
                daemonsets_res,
                "daemonsets",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.replicasets,
                replicasets_res,
                "replicasets",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.replication_controllers,
                replication_controllers_res,
                "replicationcontrollers",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.jobs,
                jobs_res,
                "jobs",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.cronjobs,
                cronjobs_res,
                "cronjobs",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.namespace_list,
                namespace_list_res,
                "namespacelist",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.resource_quotas,
                resource_quotas_res,
                "resourcequotas",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.limit_ranges,
                limit_ranges_res,
                "limitranges",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.pod_disruption_budgets,
                pod_disruption_budgets_res,
                "pdbs",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.service_accounts,
                service_accounts_res,
                "serviceaccounts",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.roles,
                roles_res,
                "roles",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.role_bindings,
                role_bindings_res,
                "rolebindings",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.cluster_roles,
                cluster_roles_res,
                "clusterroles",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.cluster_role_bindings,
                cluster_role_bindings_res,
                "clusterrolebindings",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.vulnerability_reports,
                vulnerability_reports_res,
                "vulnerabilityreports",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.custom_resource_definitions,
                custom_resource_definitions_res,
                "crds",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.endpoints,
                endpoints_res,
                "endpoints",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.ingresses,
                ingresses_res,
                "ingresses",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.ingress_classes,
                ingress_classes_res,
                "ingressclasses",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.gateway_classes,
                gateway_classes_res,
                "gatewayclasses",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.gateways,
                gateways_res,
                "gateways",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.http_routes,
                http_routes_res,
                "httproutes",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.grpc_routes,
                grpc_routes_res,
                "grpcroutes",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.reference_grants,
                reference_grants_res,
                "referencegrants",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.network_policies,
                network_policies_res,
                "networkpolicies",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.config_maps,
                config_maps_res,
                "configmaps",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.secrets,
                secrets_res,
                "secrets",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.hpas,
                hpas_res,
                "hpas",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.pvcs,
                pvcs_res,
                "pvcs",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.pvs,
                pvs_res,
                "pvs",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.storage_classes,
                storage_classes_res,
                "storageclasses",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.priority_classes,
                priority_classes_res,
                "priorityclasses",
                &mut errors,
                &mut total_fetches,
            );
            if let Some(helm_releases) = apply_optional_fetch_result(
                helm_releases_res,
                "helmreleases",
                &mut errors,
                &mut total_fetches,
            ) {
                snap.helm_releases = Self::filter_namespace(helm_releases, namespace, |release| {
                    release.namespace.as_str()
                });
            }
            apply_vec_fetch_result(
                &mut snap.flux_resources,
                flux_resources_res,
                "fluxresources",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.node_metrics,
                node_metrics_res,
                "nodemetrics",
                &mut errors,
                &mut total_fetches,
            );
            apply_vec_fetch_result(
                &mut snap.pod_metrics,
                pod_metrics_res,
                "podmetrics",
                &mut errors,
                &mut total_fetches,
            );
            if let Some(cluster_version) = apply_optional_fetch_result(
                cluster_info_res,
                "cluster info",
                &mut errors,
                &mut total_fetches,
            ) && !snap.nodes.is_empty()
            {
                let cluster_pod_count = if namespace.is_some() {
                    apply_optional_fetch_result(
                        cluster_pod_count_res,
                        "cluster pod count",
                        &mut errors,
                        &mut total_fetches,
                    )
                } else {
                    None
                };
                if let Some(pod_count) =
                    cluster_pod_count.or_else(|| namespace.is_none().then_some(snap.pods.len()))
                {
                    snap.cluster_info = Some(Self::build_cluster_info(
                        client,
                        &snap.nodes,
                        pod_count,
                        cluster_version,
                    ));
                }
            }
        }

        self.namespaces = Self::namespace_names_from_list(&self.snapshot.namespace_list);

        let all_failed = !skip_core
            && total_fetches > 0
            && if primary_resource_fetch_attempted {
                !primary_resource_fetch_succeeded
            } else {
                errors.len() >= total_fetches
            };

        if all_failed {
            let message = if errors.is_empty() {
                "failed to refresh cluster state".to_string()
            } else if errors.len() <= 3 {
                errors.join(" | ")
            } else {
                format!("{} (+{} more)", errors[..3].join(" | "), errors.len() - 3)
            };
            let snap = Arc::make_mut(&mut self.snapshot);
            snap.phase = DataPhase::Error;
            snap.last_error = Some(message.clone());
            snap.connection_health = ConnectionHealth::Disconnected;
            snap.failed_resource_count = errors.len();
            self.snapshot_dirty = true;
            self.publish_snapshot();
            return Err(anyhow!(message));
        }

        let namespaces_count = self
            .snapshot
            .pods
            .iter()
            .map(|pod| pod.namespace.as_str())
            .chain(
                self.snapshot
                    .services
                    .iter()
                    .map(|service| service.namespace.as_str()),
            )
            .chain(
                self.snapshot
                    .deployments
                    .iter()
                    .map(|deployment| deployment.namespace.as_str()),
            )
            .collect::<HashSet<_>>()
            .len();

        let prev_loaded_scope = self.snapshot.loaded_scope;
        let prev_connection_health = self.snapshot.connection_health;
        {
            let snap = Arc::make_mut(&mut self.snapshot);
            snap.namespaces_count = namespaces_count;
            snap.flux_counts = FluxCounts::compute(&snap.flux_resources);
            if fetch_local_helm_repositories {
                snap.helm_repositories = crate::k8s::helm::read_helm_repositories();
            }
            snap.loaded_scope =
                prev_loaded_scope.union(Self::completed_scope_for_refresh(options, target_view));
        }
        self.mark_refresh_completed(options, target_view);
        // Arc refcount is 1 here — make_mut is a no-op pointer return.
        let snap = Arc::make_mut(&mut self.snapshot);
        snap.snapshot_version = snap.snapshot_version.saturating_add(1);
        snap.phase = DataPhase::Ready;
        snap.last_updated = Some(now());
        snap.failed_resource_count = errors.len();
        snap.connection_health = if total_fetches == 0 {
            prev_connection_health
        } else if errors.is_empty() {
            ConnectionHealth::Connected
        } else if errors.len() >= total_fetches {
            ConnectionHealth::Disconnected
        } else {
            ConnectionHealth::Degraded(errors.len())
        };
        snap.last_error = if errors.is_empty() {
            None
        } else if errors.len() <= 3 {
            Some(errors.join(" | "))
        } else {
            Some(format!(
                "{} (+{} more)",
                errors[..3].join(" | "),
                errors.len() - 3
            ))
        };
        self.snapshot_dirty = true;

        self.publish_snapshot();

        Ok(())
    }
}
