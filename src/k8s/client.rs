//! Kubernetes API client wrapper used by KubecTUI.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use crate::time::{now, parse_timestamp};
use anyhow::{Context, Result};
use futures::{StreamExt, stream};
use k8s_openapi::api::{
    apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet},
    authorization::v1::{ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec},
    autoscaling::v2::HorizontalPodAutoscaler,
    batch::v1::{CronJob, Job},
    core::v1::{
        ConfigMap, Endpoints, LimitRange, Namespace, Node, PersistentVolume, PersistentVolumeClaim,
        Pod, ReplicationController, ResourceQuota, Secret, Service, ServiceAccount,
    },
    networking::v1::{Ingress, IngressClass, NetworkPolicy},
    policy::v1::PodDisruptionBudget,
    rbac::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding},
    scheduling::v1::PriorityClass,
    storage::v1::StorageClass,
};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::{
    CustomResourceDefinition, CustomResourceDefinitionVersion,
};
use kube::{
    Api, Client, Config,
    api::{
        ApiResource, DynamicObject, GroupVersionKind, ListParams, PartialObjectMeta, Patch,
        PatchParams, PostParams,
    },
    client::{ClientBuilder, retry::RetryPolicy},
    config::KubeConfigOptions,
};
use tower::{buffer::BufferLayer, retry::RetryLayer};

use crate::k8s::{events, gateway_api, yaml};
use crate::{
    app::ResourceRef,
    authorization::{
        ActionAuthorizationMap, DetailActionAuthorization, ResourceAccessCheck,
        detail_action_requires_authorization, helm_release_storage_access_checks,
    },
    k8s::{
        conversions::{
            age_from_created_at, app_timestamp_from_k8s_timestamp, namespace_metadata_to_info,
        },
        exec::{DebugContainerLaunchRequest, DebugContainerLaunchResult, launch_debug_container},
        node_debug::{
            NodeDebugLaunchRequest, NodeDebugLaunchResult, delete_node_debug_pod,
            launch_node_debug_pod,
        },
    },
    policy::DetailAction,
};

pub use crate::k8s::{
    dtos::{
        ClusterInfo, ClusterRoleBindingInfo, ClusterRoleInfo, ClusterVersionInfo, ConfigMapInfo,
        CronJobInfo, CustomResourceDefinitionInfo, CustomResourceInfo, DaemonSetInfo,
        DeploymentInfo, EndpointInfo, FluxResourceInfo, GatewayClassInfo, GatewayInfo,
        GrpcRouteInfo, HelmReleaseInfo, HpaInfo, HttpRouteInfo, IngressClassInfo, IngressInfo,
        JobInfo, K8sEventInfo, LimitRangeInfo, LimitSpec, NamespaceInfo, NetworkPolicyInfo,
        NodeInfo, NodeMetricsInfo, PodDisruptionBudgetInfo, PodInfo, PodMetricsInfo,
        PriorityClassInfo, PvInfo, PvcInfo, RbacRule, ReferenceGrantInfo, ReplicaSetInfo,
        ReplicationControllerInfo, ResourceQuotaInfo, RoleBindingInfo, RoleBindingSubject,
        RoleInfo, SecretInfo, ServiceAccountInfo, ServiceInfo, StatefulSetInfo, StorageClassInfo,
        VulnerabilityReportInfo, VulnerabilitySummaryCounts,
    },
    events::EventInfo,
};

const MAX_EVENTS_LIST_LIMIT: u32 = 1000;
const MAX_RECENT_EVENTS_ITEMS: usize = 250;
const CLIENT_RETRY_BUFFER_SIZE: usize = 1024;
const EPHEMERAL_CONTAINERS_MIN_MINOR: u32 = 25;
const TRIVY_OPERATOR_GROUP: &str = "aquasecurity.github.io";
const TRIVY_OPERATOR_VERSION: &str = "v1alpha1";

/// Configured Kubernetes client wrapper.
#[derive(Clone)]
pub struct K8sClient {
    client: Client,
    cluster_url: String,
    cluster_context: Option<String>,
    cluster_version_cache: Arc<tokio::sync::RwLock<Option<ClusterVersionInfo>>>,
    flux_targets_cache: Arc<tokio::sync::RwLock<Option<Vec<FluxApiTarget>>>>,
    access_review_cache: Arc<tokio::sync::RwLock<HashMap<ResourceAccessCheck, bool>>>,
}

/// Generates a namespace-scoped fetch method on `K8sClient`.
macro_rules! fetch_namespaced {
    ($(#[$meta:meta])* $method:ident, $k8s_type:ty, $info_type:ty, $converter:path, $resource_name:literal) => {
        $(#[$meta])*
        pub async fn $method(&self, namespace: Option<&str>) -> Result<Vec<$info_type>> {
            let api: Api<$k8s_type> = match namespace {
                Some(ns) => Api::namespaced(self.client.clone(), ns),
                None => Api::all(self.client.clone()),
            };
            let list = list_items_or_empty(&api, &ListParams::default(), || {
                if let Some(ns) = namespace {
                    format!(
                        concat!("failed fetching ", $resource_name, " in namespace '{}'"),
                        ns
                    )
                } else {
                    concat!("failed fetching ", $resource_name, " across all namespaces")
                        .to_string()
                }
            })
            .await?;
            Ok(list.into_iter().map($converter).collect())
        }
    };
}

/// Generates a cluster-scoped fetch method on `K8sClient`.
macro_rules! fetch_cluster {
    ($(#[$meta:meta])* $method:ident, $k8s_type:ty, $info_type:ty, $converter:path, $resource_name:literal) => {
        $(#[$meta])*
        pub async fn $method(&self) -> Result<Vec<$info_type>> {
            let api: Api<$k8s_type> = Api::all(self.client.clone());
            let list = list_items_or_empty(&api, &ListParams::default(), || {
                concat!("failed fetching ", $resource_name).to_string()
            })
            .await?;
            Ok(list.into_iter().map($converter).collect())
        }
    };
}

impl K8sClient {
    /// Creates a Kubernetes client from `~/.kube/config` when available,
    /// then falls back to ambient/in-cluster configuration.
    pub async fn connect() -> Result<Self> {
        let cluster_context = kube::config::Kubeconfig::read()
            .ok()
            .and_then(|cfg| cfg.current_context);

        let config = match Config::from_kubeconfig(&KubeConfigOptions::default()).await {
            Ok(cfg) => cfg,
            Err(kubeconfig_err) => Config::infer().await.with_context(|| {
                format!(
                    "failed loading kubeconfig from ~/.kube/config and failed inferring config: {kubeconfig_err}"
                )
            })?,
        };

        let cluster_url = config.cluster_url.to_string();
        let client = build_kube_client(config)?;

        Ok(Self {
            client,
            cluster_url,
            cluster_context,
            cluster_version_cache: Arc::new(tokio::sync::RwLock::new(None)),
            flux_targets_cache: Arc::new(tokio::sync::RwLock::new(None)),
            access_review_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        })
    }

    /// Creates a Kubernetes client pinned to a specific kubeconfig context.
    pub async fn connect_with_context(context: &str) -> Result<Self> {
        let opts = KubeConfigOptions {
            context: Some(context.to_string()),
            ..Default::default()
        };
        let config = Config::from_kubeconfig(&opts)
            .await
            .with_context(|| format!("failed loading kubeconfig for context '{context}'"))?;

        let cluster_url = config.cluster_url.to_string();
        let client = build_kube_client(config)?;

        Ok(Self {
            client,
            cluster_url,
            cluster_context: Some(context.to_string()),
            cluster_version_cache: Arc::new(tokio::sync::RwLock::new(None)),
            flux_targets_cache: Arc::new(tokio::sync::RwLock::new(None)),
            access_review_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        })
    }

    /// Creates a dummy client for unit tests that don't hit the K8s API.
    ///
    /// The underlying HTTP client points at an unreachable address so any
    /// actual API call will fail immediately — use only for tests that
    /// exercise local logic.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn dummy() -> Self {
        let cfg = kube::Config::new("http://127.0.0.1:1".parse().expect("valid URL"));
        let client = Client::try_from(cfg).expect("client should build for test URL");
        Self {
            client,
            cluster_url: "http://127.0.0.1:1".to_string(),
            cluster_context: Some("test".to_string()),
            cluster_version_cache: Arc::new(tokio::sync::RwLock::new(None)),
            flux_targets_cache: Arc::new(tokio::sync::RwLock::new(None)),
            access_review_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    /// Returns all context names from `~/.kube/config`, sorted alphabetically.
    /// The current context (if any) is returned first.
    pub fn list_contexts() -> Vec<String> {
        let Ok(kubeconfig) = kube::config::Kubeconfig::read() else {
            return Vec::new();
        };

        let current = kubeconfig.current_context.clone();
        let mut names: Vec<String> = kubeconfig
            .contexts
            .into_iter()
            .filter_map(|nc| nc.name.into())
            .collect();
        names.sort();

        if let Some(cur) = current {
            names.retain(|n| n != &cur);
            names.insert(0, cur);
        }

        names
    }

    /// Returns the configured Kubernetes cluster API endpoint.
    pub fn cluster_url(&self) -> &str {
        &self.cluster_url
    }

    pub fn cluster_context(&self) -> Option<&str> {
        self.cluster_context.as_deref()
    }

    /// Returns reference to the underlying Kubernetes client.
    pub fn get_client(&self) -> Client {
        self.client.clone()
    }

    /// Cordons a node by setting `spec.unschedulable = true`.
    pub async fn cordon_node(&self, name: &str) -> Result<()> {
        let nodes_api: Api<Node> = Api::all(self.client.clone());
        let patch = serde_json::json!({"spec": {"unschedulable": true}});
        let pp = PatchParams {
            field_manager: Some("kubectui".to_string()),
            ..Default::default()
        };
        nodes_api
            .patch(name, &pp, &Patch::Merge(patch))
            .await
            .with_context(|| format!("failed to cordon node '{name}'"))?;
        Ok(())
    }

    /// Uncordons a node by setting `spec.unschedulable = false`.
    pub async fn uncordon_node(&self, name: &str) -> Result<()> {
        let nodes_api: Api<Node> = Api::all(self.client.clone());
        let patch = serde_json::json!({"spec": {"unschedulable": false}});
        let pp = PatchParams {
            field_manager: Some("kubectui".to_string()),
            ..Default::default()
        };
        nodes_api
            .patch(name, &pp, &Patch::Merge(patch))
            .await
            .with_context(|| format!("failed to uncordon node '{name}'"))?;
        Ok(())
    }

    /// Drains a node by cordoning it then evicting all non-DaemonSet, non-mirror pods.
    ///
    /// If `force` is true, pods that cannot be evicted (PDB violations) are deleted directly.
    pub async fn drain_node(
        &self,
        name: &str,
        timeout_secs: u64,
        grace_period_secs: u32,
        force: bool,
    ) -> Result<()> {
        // Cordon first to prevent new pods from being scheduled during drain.
        self.cordon_node(name).await?;

        let pods_api: Api<k8s_openapi::api::core::v1::Pod> = Api::all(self.client.clone());
        let lp = ListParams::default().fields(&format!("spec.nodeName={name}"));
        let pod_list = pods_api
            .list(&lp)
            .await
            .with_context(|| format!("failed to list pods on node '{name}'"))?;

        let mut to_evict = Vec::new();
        for pod in pod_list {
            let meta = &pod.metadata;
            // Skip mirror pods (created by kubelet from static manifests).
            if meta
                .annotations
                .as_ref()
                .is_some_and(|a| a.contains_key("kubernetes.io/config.mirror"))
            {
                continue;
            }
            // Skip DaemonSet-owned pods.
            if pod
                .metadata
                .owner_references
                .as_ref()
                .is_some_and(|refs| refs.iter().any(|r| r.kind == "DaemonSet"))
            {
                continue;
            }
            let pod_name = meta.name.clone().unwrap_or_default();
            let pod_ns = meta.namespace.clone().unwrap_or_default();
            if !pod_name.is_empty() && !pod_ns.is_empty() {
                to_evict.push((pod_name, pod_ns));
            }
        }

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

        let evict_params = kube::api::EvictParams {
            delete_options: Some(kube::api::DeleteParams {
                grace_period_seconds: Some(grace_period_secs),
                ..Default::default()
            }),
            ..Default::default()
        };

        for (pod_name, pod_ns) in &to_evict {
            let ns_pods: Api<k8s_openapi::api::core::v1::Pod> =
                Api::namespaced(self.client.clone(), pod_ns);
            loop {
                if tokio::time::Instant::now() >= deadline {
                    anyhow::bail!(
                        "drain timed out after {timeout_secs}s while evicting pod '{pod_name}' in '{pod_ns}'"
                    );
                }
                let result = ns_pods.evict(pod_name, &evict_params).await;
                match result {
                    Ok(_) => break,
                    Err(kube::Error::Api(ref status))
                        if (status.code == 429 || status.code == 409) && force =>
                    {
                        // PDB violation — force delete if requested.
                        let dp = kube::api::DeleteParams {
                            grace_period_seconds: Some(0),
                            ..Default::default()
                        };
                        ns_pods.delete(pod_name, &dp).await.with_context(|| {
                            format!("failed to force-delete pod '{pod_name}' in '{pod_ns}'")
                        })?;
                        break;
                    }
                    Err(kube::Error::Api(ref status))
                        if (status.code == 429 || status.code == 409) =>
                    {
                        // PDB violation, non-force — retry with backoff until deadline.
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                    Err(kube::Error::Api(ref status)) if status.code == 404 => break,
                    Err(e) => {
                        return Err(e).with_context(|| {
                            format!("failed to evict pod '{pod_name}' in '{pod_ns}'")
                        });
                    }
                }
            }
        }

        // Wait for pods to terminate.
        const MAX_CONSECUTIVE_ERRORS: u32 = 5;
        for (pod_name, pod_ns) in &to_evict {
            let ns_pods: Api<k8s_openapi::api::core::v1::Pod> =
                Api::namespaced(self.client.clone(), pod_ns);
            let mut consecutive_errors: u32 = 0;
            loop {
                if tokio::time::Instant::now() >= deadline {
                    anyhow::bail!(
                        "drain timed out after {timeout_secs}s waiting for pod '{pod_name}' in '{pod_ns}' to terminate"
                    );
                }
                match ns_pods.get_opt(pod_name).await {
                    Ok(None) => break,
                    Ok(Some(_)) => {
                        consecutive_errors = 0;
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                    Err(kube::Error::Api(ref status)) if status.code == 404 => break,
                    Err(e) => {
                        consecutive_errors += 1;
                        if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                            return Err(e).context(format!(
                                "repeated errors waiting for pod '{pod_name}' in '{pod_ns}' to terminate"
                            ));
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        }

        Ok(())
    }

    /// Fetches available namespaces sorted alphabetically.
    pub async fn fetch_namespaces(&self) -> Result<Vec<String>> {
        let ns_api: Api<Namespace> = Api::all(self.client.clone());
        let list = list_metadata_items_or_empty(&ns_api, &ListParams::default(), || {
            "failed fetching namespaces".to_string()
        })
        .await?;

        let names: Vec<String> = list
            .iter()
            .map(|ns| ns.metadata.name.clone().unwrap_or_default())
            .collect();

        Ok(sort_namespaces(names))
    }

    /// Fetches Namespaces as NamespaceInfo using metadata-only list payloads.
    pub async fn fetch_namespace_list(&self) -> Result<Vec<NamespaceInfo>> {
        let api: Api<Namespace> = Api::all(self.client.clone());
        let mut items: Vec<NamespaceInfo> =
            list_metadata_items_or_empty(&api, &ListParams::default(), || {
                "failed fetching namespaces".to_string()
            })
            .await?
            .into_iter()
            .map(namespace_metadata_to_info)
            .collect();
        items.sort_unstable_by(|left, right| left.name.cmp(&right.name));
        Ok(items)
    }

    // ── Namespace-scoped resources ──────────────────────────────────
    fetch_namespaced!(
        /// Fetches pods from a namespace or all namespaces.
        fetch_pods, Pod, PodInfo,
        crate::k8s::conversions::pod_to_info, "pods"
    );
    fetch_namespaced!(
        /// Fetches services from a namespace or all namespaces.
        fetch_services, Service, ServiceInfo,
        crate::k8s::conversions::service_to_info, "services"
    );
    fetch_namespaced!(
        /// Fetches deployments from a namespace or all namespaces.
        fetch_deployments, Deployment, DeploymentInfo,
        crate::k8s::conversions::deployment_to_info, "deployments"
    );
    fetch_namespaced!(
        /// Fetches statefulsets from a namespace or all namespaces.
        fetch_statefulsets, StatefulSet, StatefulSetInfo,
        crate::k8s::conversions::statefulset_to_info, "statefulsets"
    );
    fetch_namespaced!(
        /// Fetches daemonsets from a namespace or all namespaces.
        fetch_daemonsets, DaemonSet, DaemonSetInfo,
        crate::k8s::conversions::daemonset_to_info, "daemonsets"
    );
    fetch_namespaced!(
        /// Fetches replica sets from a namespace or all namespaces.
        fetch_replicasets, ReplicaSet, ReplicaSetInfo,
        crate::k8s::conversions::replicaset_to_info, "replicasets"
    );
    fetch_namespaced!(
        /// Fetches replication controllers from a namespace or all namespaces.
        fetch_replication_controllers, ReplicationController, ReplicationControllerInfo,
        crate::k8s::conversions::replication_controller_to_info, "replicationcontrollers"
    );
    fetch_namespaced!(
        /// Fetches service accounts from a namespace or all namespaces.
        fetch_service_accounts, ServiceAccount, ServiceAccountInfo,
        crate::k8s::conversions::service_account_to_info, "serviceaccounts"
    );
    fetch_namespaced!(
        /// Fetches roles from a namespace or all namespaces.
        fetch_roles, Role, RoleInfo,
        crate::k8s::conversions::role_to_info, "roles"
    );
    fetch_namespaced!(
        /// Fetches role bindings from a namespace or all namespaces.
        fetch_role_bindings, RoleBinding, RoleBindingInfo,
        crate::k8s::conversions::role_binding_to_info, "rolebindings"
    );
    fetch_namespaced!(
        /// Fetches jobs from a namespace or all namespaces.
        fetch_jobs, Job, JobInfo,
        crate::k8s::conversions::job_to_info, "jobs"
    );
    fetch_namespaced!(
        /// Fetches cronjobs from a namespace or all namespaces.
        fetch_cronjobs, CronJob, CronJobInfo,
        crate::k8s::conversions::cronjob_to_info, "cronjobs"
    );
    fetch_namespaced!(
        /// Fetches resource quotas from a namespace or all namespaces.
        fetch_resource_quotas, ResourceQuota, ResourceQuotaInfo,
        crate::k8s::conversions::resource_quota_to_info, "resource quotas"
    );
    fetch_namespaced!(
        /// Fetches limit ranges from a namespace or all namespaces.
        fetch_limit_ranges, LimitRange, LimitRangeInfo,
        crate::k8s::conversions::limit_range_to_info, "limit ranges"
    );
    fetch_namespaced!(
        /// Fetches pod disruption budgets from a namespace or all namespaces.
        fetch_pod_disruption_budgets, PodDisruptionBudget, PodDisruptionBudgetInfo,
        crate::k8s::conversions::pdb_to_info, "pod disruption budgets"
    );
    fetch_namespaced!(
        /// Fetches Endpoints.
        fetch_endpoints, Endpoints, EndpointInfo,
        crate::k8s::conversions::endpoint_to_info, "endpoints"
    );
    fetch_namespaced!(
        /// Fetches Ingresses.
        fetch_ingresses, Ingress, IngressInfo,
        crate::k8s::conversions::ingress_to_info, "ingresses"
    );
    /// Fetches Gateway API Gateways when the CRD is installed.
    pub async fn fetch_gateways(&self, namespace: Option<&str>) -> Result<Vec<GatewayInfo>> {
        gateway_api::fetch_gateways(&self.client, namespace).await
    }
    /// Fetches Gateway API HTTPRoutes when the CRD is installed.
    pub async fn fetch_http_routes(&self, namespace: Option<&str>) -> Result<Vec<HttpRouteInfo>> {
        gateway_api::fetch_http_routes(&self.client, namespace).await
    }
    /// Fetches Gateway API GRPCRoutes when the CRD is installed.
    pub async fn fetch_grpc_routes(&self, namespace: Option<&str>) -> Result<Vec<GrpcRouteInfo>> {
        gateway_api::fetch_grpc_routes(&self.client, namespace).await
    }
    /// Fetches Gateway API ReferenceGrants when the CRD is installed.
    pub async fn fetch_reference_grants(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<ReferenceGrantInfo>> {
        gateway_api::fetch_reference_grants(&self.client, namespace).await
    }
    fetch_namespaced!(
        /// Fetches NetworkPolicies.
        fetch_network_policies, NetworkPolicy, NetworkPolicyInfo,
        crate::k8s::conversions::network_policy_to_info, "network policies"
    );
    fetch_namespaced!(
        /// Fetches ConfigMaps.
        fetch_config_maps, ConfigMap, ConfigMapInfo,
        crate::k8s::conversions::config_map_to_info, "configmaps"
    );
    fetch_namespaced!(
        /// Fetches Secrets.
        fetch_secrets, Secret, SecretInfo,
        crate::k8s::conversions::secret_to_info, "secrets"
    );
    fetch_namespaced!(
        /// Fetches HPAs.
        fetch_hpas, HorizontalPodAutoscaler, HpaInfo,
        crate::k8s::conversions::hpa_to_info, "HPAs"
    );
    fetch_namespaced!(
        /// Fetches PersistentVolumeClaims.
        fetch_pvcs, PersistentVolumeClaim, PvcInfo,
        crate::k8s::conversions::pvc_to_info, "PVCs"
    );

    // ── Cluster-scoped resources ────────────────────────────────────
    fetch_cluster!(
        /// Fetches all nodes from the cluster.
        fetch_nodes, Node, NodeInfo,
        crate::k8s::conversions::node_to_info, "Kubernetes nodes"
    );
    fetch_cluster!(
        /// Fetches cluster roles.
        fetch_cluster_roles, ClusterRole, ClusterRoleInfo,
        crate::k8s::conversions::cluster_role_to_info, "clusterroles"
    );
    fetch_cluster!(
        /// Fetches cluster role bindings.
        fetch_cluster_role_bindings, ClusterRoleBinding, ClusterRoleBindingInfo,
        crate::k8s::conversions::cluster_role_binding_to_info, "clusterrolebindings"
    );
    fetch_cluster!(
        /// Fetches IngressClasses.
        fetch_ingress_classes, IngressClass, IngressClassInfo,
        crate::k8s::conversions::ingress_class_to_info, "ingress classes"
    );
    /// Fetches Gateway API GatewayClasses when the CRD is installed.
    pub async fn fetch_gateway_classes(&self) -> Result<Vec<GatewayClassInfo>> {
        gateway_api::fetch_gateway_classes(&self.client).await
    }
    fetch_cluster!(
        /// Fetches PersistentVolumes.
        fetch_pvs, PersistentVolume, PvInfo,
        crate::k8s::conversions::pv_to_info, "PVs"
    );
    fetch_cluster!(
        /// Fetches StorageClasses.
        fetch_storage_classes, StorageClass, StorageClassInfo,
        crate::k8s::conversions::storage_class_to_info, "storage classes"
    );
    fetch_cluster!(
        /// Fetches PriorityClasses.
        fetch_priority_classes, PriorityClass, PriorityClassInfo,
        crate::k8s::conversions::priority_class_to_info, "priority classes"
    );

    /// Fetches cluster-wide Events.
    pub async fn fetch_events(&self, namespace: Option<&str>) -> Result<Vec<K8sEventInfo>> {
        use k8s_openapi::api::core::v1::Event;
        let api: Api<Event> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };
        let lp = ListParams::default().limit(MAX_EVENTS_LIST_LIMIT);
        let list = list_items_or_empty(&api, &lp, || {
            if let Some(ns) = namespace {
                format!("failed fetching events in namespace '{ns}'")
            } else {
                "failed fetching events across all namespaces".to_string()
            }
        })
        .await?;
        let mut events: Vec<K8sEventInfo> = list
            .into_iter()
            .map(crate::k8s::conversions::event_to_info)
            .collect();
        // Sort by last_seen descending
        events.sort_unstable_by(|a, b| b.last_seen.cmp(&a.last_seen));
        events.truncate(MAX_RECENT_EVENTS_ITEMS);
        Ok(events)
    }

    /// Fetches CustomResourceDefinitions cluster-wide.
    ///
    /// Instance lists are fetched lazily when entering the Extensions detail pane.
    /// This keeps global refresh fast on large clusters with many CRDs.
    pub async fn fetch_custom_resource_definitions(
        &self,
    ) -> Result<Vec<CustomResourceDefinitionInfo>> {
        let crd_api: Api<CustomResourceDefinition> = Api::all(self.client.clone());
        let list = list_items_or_empty(&crd_api, &ListParams::default(), || {
            "failed fetching custom resource definitions".to_string()
        })
        .await?;

        let mut crds = Vec::new();
        for crd in list {
            let spec = crd.spec;
            let version = choose_crd_request_version(&spec.versions);

            crds.push(CustomResourceDefinitionInfo {
                name: crd.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                group: spec.group.clone(),
                version,
                kind: spec.names.kind.clone(),
                plural: spec.names.plural.clone(),
                scope: spec.scope,
                instances: 0,
            });
        }

        crds.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        Ok(crds)
    }

    /// Fetches custom resources for a selected CRD.
    pub async fn fetch_custom_resources(
        &self,
        crd: &CustomResourceDefinitionInfo,
        namespace: Option<&str>,
    ) -> Result<Vec<CustomResourceInfo>> {
        let ar = custom_resource_api_resource(crd);

        let api: Api<DynamicObject> = if crd.scope.eq_ignore_ascii_case("Namespaced") {
            match namespace {
                Some(ns) => Api::namespaced_with(self.client.clone(), ns, &ar),
                None => Api::all_with(self.client.clone(), &ar),
            }
        } else {
            Api::all_with(self.client.clone(), &ar)
        };

        let list = list_items_or_empty(&api, &ListParams::default(), || {
            format!("failed fetching custom resources for CRD '{}'", crd.name)
        })
        .await?;

        let now = now();
        let mut resources = list
            .into_iter()
            .map(|item| {
                let created_at = item
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0));
                CustomResourceInfo {
                    name: item
                        .metadata
                        .name
                        .unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: item.metadata.namespace,
                    created_at,
                    age: age_from_created_at(created_at, now),
                }
            })
            .collect::<Vec<_>>();

        resources.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        Ok(resources)
    }

    /// Fetches Trivy Operator vulnerability reports. Missing CRDs or RBAC simply yield no rows.
    pub async fn fetch_vulnerability_reports(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<VulnerabilityReportInfo>> {
        let mut reports = self
            .fetch_namespaced_vulnerability_reports(namespace)
            .await?;
        reports.extend(self.fetch_cluster_vulnerability_reports().await?);
        reports.sort_unstable_by(|left, right| {
            vulnerability_severity_rank(right.counts.highest_severity())
                .cmp(&vulnerability_severity_rank(left.counts.highest_severity()))
                .then_with(|| right.counts.total().cmp(&left.counts.total()))
                .then_with(|| left.resource_namespace.cmp(&right.resource_namespace))
                .then_with(|| left.resource_kind.cmp(&right.resource_kind))
                .then_with(|| left.resource_name.cmp(&right.resource_name))
                .then_with(|| left.container_name.cmp(&right.container_name))
        });
        Ok(reports)
    }

    async fn fetch_namespaced_vulnerability_reports(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<VulnerabilityReportInfo>> {
        let gvk = GroupVersionKind::gvk(
            TRIVY_OPERATOR_GROUP,
            TRIVY_OPERATOR_VERSION,
            "VulnerabilityReport",
        );
        let mut ar = ApiResource::from_gvk(&gvk);
        ar.plural = "vulnerabilityreports".to_string();
        let api: Api<DynamicObject> = match namespace {
            Some(ns) => Api::namespaced_with(self.client.clone(), ns, &ar),
            None => Api::all_with(self.client.clone(), &ar),
        };

        let items = match api.list(&ListParams::default()).await {
            Ok(list) => list.items,
            Err(err) if is_forbidden_error(&err) || is_missing_api_error(&err) => {
                return Ok(Vec::new());
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    if let Some(ns) = namespace {
                        format!("failed fetching vulnerability reports in namespace '{ns}'")
                    } else {
                        "failed fetching vulnerability reports across all namespaces".to_string()
                    }
                });
            }
        };

        Ok(items
            .into_iter()
            .filter_map(|item| parse_vulnerability_report(item, false))
            .collect())
    }

    async fn fetch_cluster_vulnerability_reports(&self) -> Result<Vec<VulnerabilityReportInfo>> {
        let gvk = GroupVersionKind::gvk(
            TRIVY_OPERATOR_GROUP,
            TRIVY_OPERATOR_VERSION,
            "ClusterVulnerabilityReport",
        );
        let mut ar = ApiResource::from_gvk(&gvk);
        ar.plural = "clustervulnerabilityreports".to_string();
        let api: Api<DynamicObject> = Api::all_with(self.client.clone(), &ar);

        let items = match api.list(&ListParams::default()).await {
            Ok(list) => list.items,
            Err(err) if is_forbidden_error(&err) || is_missing_api_error(&err) => {
                return Ok(Vec::new());
            }
            Err(err) => return Err(err).context("failed fetching cluster vulnerability reports"),
        };

        Ok(items
            .into_iter()
            .filter_map(|item| parse_vulnerability_report(item, true))
            .collect())
    }

    /// Fetches pod metrics via metrics.k8s.io (returns None when unavailable).
    pub async fn fetch_pod_metrics(
        &self,
        name: &str,
        namespace: &str,
    ) -> Result<Option<PodMetricsInfo>> {
        let gvk = GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "PodMetrics");
        let mut ar = ApiResource::from_gvk(&gvk);
        ar.plural = "pods".to_string();
        let api: Api<DynamicObject> = Api::namespaced_with(self.client.clone(), namespace, &ar);

        let obj = match api.get(name).await {
            Ok(value) => value,
            Err(err) if is_metrics_api_unavailable(&err) || is_forbidden_error(&err) => {
                return Ok(None);
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("failed fetching pod metrics for {namespace}/{name}")
                });
            }
        };

        Ok(PodMetricsInfo::from_json(
            name.to_string(),
            namespace.to_string(),
            &obj.data,
        ))
    }

    /// Fetches node metrics via metrics.k8s.io (returns None when unavailable).
    pub async fn fetch_node_metrics(&self, name: &str) -> Result<Option<NodeMetricsInfo>> {
        let gvk = GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "NodeMetrics");
        let mut ar = ApiResource::from_gvk(&gvk);
        ar.plural = "nodes".to_string();
        let api: Api<DynamicObject> = Api::all_with(self.client.clone(), &ar);

        let obj = match api.get(name).await {
            Ok(value) => value,
            Err(err) if is_metrics_api_unavailable(&err) || is_forbidden_error(&err) => {
                return Ok(None);
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed fetching node metrics for node '{name}'"));
            }
        };

        Ok(NodeMetricsInfo::from_json(name.to_string(), &obj.data))
    }

    /// Fetches metrics for all nodes at once via metrics.k8s.io list.
    /// Returns empty vec (not an error) when metrics-server is absent.
    pub async fn fetch_all_node_metrics(&self) -> Result<Vec<NodeMetricsInfo>> {
        let gvk = GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "NodeMetrics");
        let mut ar = ApiResource::from_gvk(&gvk);
        ar.plural = "nodes".to_string();
        let api: Api<DynamicObject> = Api::all_with(self.client.clone(), &ar);

        let list = match api.list(&ListParams::default()).await {
            Ok(list) => list.items,
            Err(err) if is_metrics_api_unavailable(&err) || is_forbidden_error(&err) => {
                return Ok(Vec::new());
            }
            Err(err) => return Err(err).context("failed listing node metrics"),
        };

        Ok(list
            .into_iter()
            .filter_map(|obj| {
                let name = obj.metadata.name.clone().unwrap_or_default();
                NodeMetricsInfo::from_json(name, &obj.data)
            })
            .collect())
    }

    /// Fetches metrics for all pods at once via metrics.k8s.io list.
    /// Returns empty vec (not an error) when metrics-server is absent.
    pub async fn fetch_all_pod_metrics(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<PodMetricsInfo>> {
        let gvk = GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "PodMetrics");
        let mut ar = ApiResource::from_gvk(&gvk);
        ar.plural = "pods".to_string();
        let api: Api<DynamicObject> = match namespace {
            Some(ns) => Api::namespaced_with(self.client.clone(), ns, &ar),
            None => Api::all_with(self.client.clone(), &ar),
        };

        let list = match api.list(&ListParams::default()).await {
            Ok(list) => list.items,
            Err(err) if is_metrics_api_unavailable(&err) || is_forbidden_error(&err) => {
                return Ok(Vec::new());
            }
            Err(err) => return Err(err).context("failed listing pod metrics"),
        };

        Ok(list
            .into_iter()
            .filter_map(|obj| {
                let name = obj.metadata.name.clone().unwrap_or_default();
                let ns = obj.metadata.namespace.clone().unwrap_or_default();
                PodMetricsInfo::from_json(name, ns, &obj.data)
            })
            .collect())
    }

    /// Fetches and caches API server version metadata for the current context.
    pub async fn fetch_cluster_version(&self) -> Result<ClusterVersionInfo> {
        if let Some(version) = self.cluster_version_cache.read().await.clone() {
            return Ok(version);
        }

        let version = self
            .client
            .apiserver_version()
            .await
            .context("failed fetching API server version")?;
        let info = ClusterVersionInfo {
            git_version: version.git_version,
            platform: version.platform,
        };

        let mut cache = self.cluster_version_cache.write().await;
        if let Some(version) = cache.clone() {
            return Ok(version);
        }
        *cache = Some(info.clone());
        Ok(info)
    }

    /// Fetches the cluster-wide pod count regardless of the active namespace scope.
    pub async fn fetch_cluster_pod_count(&self) -> Result<usize> {
        let pods: Api<Pod> = Api::all(self.client.clone());
        let list = list_items_or_empty(&pods, &ListParams::default(), || {
            "failed fetching pod count".to_string()
        })
        .await?;
        Ok(list.len())
    }

    pub async fn fetch_detail_action_authorizations(
        &self,
        resource: &ResourceRef,
    ) -> ActionAuthorizationMap {
        let mut authorizations = ActionAuthorizationMap::new();

        for &action in DetailAction::ALL {
            if !detail_action_requires_authorization(action) {
                continue;
            }

            let checks = resource.authorization_checks(action);
            if checks.is_empty() {
                continue;
            }

            let status = DetailActionAuthorization::from_allowed(
                self.evaluate_access_checks_cached(&checks).await,
            );
            authorizations.insert(action, status);
        }

        authorizations
    }

    pub async fn is_detail_action_authorized(
        &self,
        resource: &ResourceRef,
        action: DetailAction,
    ) -> Option<DetailActionAuthorization> {
        if !detail_action_requires_authorization(action) {
            return Some(DetailActionAuthorization::Allowed);
        }

        let checks = resource.authorization_checks(action);
        if checks.is_empty() {
            return None;
        }

        Some(DetailActionAuthorization::from_allowed(
            self.evaluate_access_checks_cached(&checks).await,
        ))
    }

    pub async fn evaluate_access_checks(&self, checks: &[ResourceAccessCheck]) -> Option<bool> {
        self.evaluate_access_checks_cached(checks).await
    }

    pub async fn helm_rollback_access_checks(
        &self,
        release_name: &str,
        namespace: &str,
        current_revision: i32,
        target_revision: i32,
    ) -> Result<Vec<ResourceAccessCheck>> {
        let current_manifest = crate::k8s::helm::fetch_release_manifest(
            release_name,
            namespace,
            self.cluster_context.clone(),
            current_revision,
        )
        .await?;
        let target_manifest = crate::k8s::helm::fetch_release_manifest(
            release_name,
            namespace,
            self.cluster_context.clone(),
            target_revision,
        )
        .await?;

        let mut checks = helm_release_storage_access_checks(namespace);
        checks.extend(
            crate::k8s::yaml::manifest_access_checks_for_transition(
                &self.client,
                &current_manifest,
                &target_manifest,
                Some(namespace),
            )
            .await?,
        );
        Ok(checks)
    }

    /// Fetches a concrete resource and renders it as YAML.
    pub async fn fetch_resource_yaml(
        &self,
        kind: &str,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<String> {
        yaml::get_resource_yaml(&self.client, kind, name, namespace)
            .await
            .with_context(|| {
                format!(
                    "failed preparing YAML for kind='{kind}' name='{name}' namespace='{}'",
                    namespace.unwrap_or("<cluster-scope>")
                )
            })
    }

    /// Fetches a concrete resource as a full, untruncated manifest for diffing.
    pub async fn fetch_resource_yaml_for_diff(
        &self,
        kind: &str,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<String> {
        yaml::get_resource_yaml_for_diff(&self.client, kind, name, namespace)
            .await
            .with_context(|| {
                format!(
                    "failed preparing diff source YAML for kind='{kind}' name='{name}' namespace='{}'",
                    namespace.unwrap_or("<cluster-scope>")
                )
            })
    }

    /// Fetches YAML for a custom resource using explicit CRD API coordinates.
    pub async fn fetch_custom_resource_yaml(
        &self,
        group: &str,
        version: &str,
        kind: &str,
        plural: &str,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<String> {
        yaml::get_custom_resource_yaml(&self.client, group, version, kind, plural, name, namespace)
            .await
            .with_context(|| {
                format!(
                    "failed preparing YAML for CRD {group}/{version}/{kind} name='{name}' namespace='{}'",
                    namespace.unwrap_or("<cluster-scope>")
                )
            })
    }

    /// Fetches a custom-resource manifest as a full, untruncated YAML document.
    pub async fn fetch_custom_resource_yaml_for_diff(
        &self,
        group: &str,
        version: &str,
        kind: &str,
        plural: &str,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<String> {
        yaml::get_custom_resource_yaml_for_diff(
            &self.client,
            group,
            version,
            kind,
            plural,
            name,
            namespace,
        )
        .await
        .with_context(|| {
            format!(
                "failed preparing diff source YAML for CRD {group}/{version}/{kind} name='{name}' namespace='{}'",
                namespace.unwrap_or("<cluster-scope>")
            )
        })
    }

    /// Fetches the canonical live manifest source for drift inspection.
    pub async fn fetch_resource_diff_source_yaml(&self, resource: &ResourceRef) -> Result<String> {
        match resource {
            ResourceRef::CustomResource {
                group,
                version,
                kind,
                plural,
                name,
                namespace,
            } => {
                self.fetch_custom_resource_yaml_for_diff(
                    group,
                    version,
                    kind,
                    plural,
                    name,
                    namespace.as_deref(),
                )
                .await
            }
            ResourceRef::HelmRelease(name, namespace) => {
                self.fetch_helm_release_yaml_for_diff(name, namespace).await
            }
            _ => {
                let kind = resource.kind().to_ascii_lowercase();
                self.fetch_resource_yaml_for_diff(&kind, resource.name(), resource.namespace())
                    .await
            }
        }
    }

    /// Launches an ephemeral debug container in a running Pod.
    pub async fn launch_debug_container(
        &self,
        request: &DebugContainerLaunchRequest,
    ) -> Result<DebugContainerLaunchResult> {
        self.ensure_ephemeral_containers_supported().await?;
        launch_debug_container(self, request).await
    }

    /// Applies edited YAML back to the cluster (server-side apply).
    pub async fn apply_resource_yaml(
        &self,
        yaml_str: &str,
        kind: &str,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<()> {
        yaml::apply_resource_yaml(&self.client, yaml_str, kind, name, namespace).await
    }

    pub async fn apply_yaml_documents(&self, yaml_str: &str) -> Result<usize> {
        yaml::apply_yaml_documents(&self.client, yaml_str).await
    }

    pub async fn launch_node_debug_pod(
        &self,
        request: &NodeDebugLaunchRequest,
    ) -> Result<NodeDebugLaunchResult> {
        launch_node_debug_pod(self, request).await
    }

    pub async fn delete_node_debug_pod(&self, namespace: &str, pod_name: &str) -> Result<()> {
        delete_node_debug_pod(self, namespace, pod_name).await
    }

    /// Deletes a Kubernetes resource by kind, name, and optional namespace.
    pub async fn delete_resource(
        &self,
        kind: &str,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<()> {
        yaml::delete_resource(&self.client, kind, name, namespace).await
    }

    /// Force-deletes a Kubernetes resource by setting grace period to 0.
    pub async fn force_delete_resource(
        &self,
        kind: &str,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<()> {
        yaml::force_delete_resource(&self.client, kind, name, namespace).await
    }

    /// Deletes a custom resource using explicit CRD coordinates.
    pub async fn delete_custom_resource(
        &self,
        group: &str,
        version: &str,
        kind: &str,
        plural: &str,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<()> {
        yaml::delete_custom_resource(&self.client, group, version, kind, plural, name, namespace)
            .await
    }

    /// Requests Flux reconciliation for a custom resource using Flux's
    /// standard `reconcile.fluxcd.io/requestedAt` annotation.
    pub async fn request_flux_reconcile(
        &self,
        group: &str,
        version: &str,
        kind: &str,
        plural: &str,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<()> {
        yaml::request_flux_reconcile(&self.client, group, version, kind, plural, name, namespace)
            .await
    }

    /// Creates a Job from a CronJob spec, effectively triggering a manual run.
    pub async fn trigger_cronjob(&self, name: &str, namespace: &str) -> Result<String> {
        use kube::api::PostParams;

        let cronjobs: Api<CronJob> = Api::namespaced(self.client.clone(), namespace);
        let cronjob = cronjobs
            .get(name)
            .await
            .with_context(|| format!("failed to get CronJob '{name}' in '{namespace}'"))?;

        let job_template = cronjob
            .spec
            .as_ref()
            .map(|s| &s.job_template)
            .context("CronJob has no spec")?;

        let timestamp = now().strftime("%Y%m%d%H%M%S").to_string();
        let job_name = format!("{name}-manual-{timestamp}");

        let job = Job {
            metadata: kube::api::ObjectMeta {
                name: Some(job_name.clone()),
                namespace: Some(namespace.to_string()),
                labels: job_template
                    .metadata
                    .as_ref()
                    .and_then(|m| m.labels.clone()),
                annotations: {
                    let mut ann = BTreeMap::new();
                    ann.insert(
                        "cronjob.kubernetes.io/instantiate".to_string(),
                        "manual".to_string(),
                    );
                    Some(ann)
                },
                ..Default::default()
            },
            spec: job_template.spec.clone(),
            ..Default::default()
        };

        let jobs: Api<Job> = Api::namespaced(self.client.clone(), namespace);
        jobs.create(&PostParams::default(), &job)
            .await
            .with_context(|| format!("failed to create Job from CronJob '{name}'"))?;

        Ok(job_name)
    }

    /// Sets `spec.suspend` on a CronJob.
    pub async fn set_cronjob_suspend(
        &self,
        name: &str,
        namespace: &str,
        suspend: bool,
    ) -> Result<()> {
        let cronjobs: Api<CronJob> = Api::namespaced(self.client.clone(), namespace);
        let patch = serde_json::json!({
            "spec": {
                "suspend": suspend
            }
        });
        let pp = PatchParams {
            field_manager: Some("kubectui".to_string()),
            ..PatchParams::default()
        };

        cronjobs
            .patch(name, &pp, &Patch::Merge(&patch))
            .await
            .with_context(|| {
                format!(
                    "failed to {} CronJob '{name}' in namespace '{namespace}'",
                    if suspend { "suspend" } else { "resume" }
                )
            })?;

        Ok(())
    }

    /// Fetches the Helm release secret as YAML.
    ///
    /// Helm v3 stores releases as Secrets named `sh.helm.release.v1.{name}.v{revision}`.
    /// This finds the latest revision secret for the given release name.
    pub async fn fetch_helm_release_yaml(
        &self,
        release_name: &str,
        namespace: &str,
    ) -> Result<String> {
        self.fetch_helm_release_secret_yaml(release_name, namespace, true, true)
            .await
    }

    /// Fetches the Helm release secret as a full, untruncated YAML document.
    pub async fn fetch_helm_release_yaml_for_diff(
        &self,
        release_name: &str,
        namespace: &str,
    ) -> Result<String> {
        self.fetch_helm_release_secret_yaml(release_name, namespace, false, false)
            .await
    }

    async fn fetch_helm_release_secret_yaml(
        &self,
        release_name: &str,
        namespace: &str,
        truncate: bool,
        allow_missing_comment: bool,
    ) -> Result<String> {
        use k8s_openapi::api::core::v1::Secret;
        use kube::api::ListParams;

        let secrets_api: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        let lp = ListParams::default().labels(&format!("owner=helm,name={release_name}"));
        let list = if allow_missing_comment {
            list_items_or_empty(&secrets_api, &lp, || {
                format!("failed fetching Helm release secrets for '{release_name}'")
            })
            .await?
        } else {
            match secrets_api.list(&lp).await {
                Ok(list) => list.items,
                Err(err) if is_forbidden_error(&err) => {
                    return Err(anyhow::anyhow!(
                        "RBAC denied reading Helm release secrets for '{release_name}' in namespace '{namespace}'"
                    ));
                }
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("failed fetching Helm release secrets for '{release_name}'")
                    });
                }
            }
        };

        // Find the latest revision (highest version label)
        let latest = list.into_iter().max_by_key(|s| {
            s.metadata
                .labels
                .as_ref()
                .and_then(|l| l.get("version"))
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or(0)
        });

        match latest {
            Some(secret) => {
                let rendered = serde_yaml::to_string(&secret)
                    .context("failed serializing Helm release secret to YAML")?;
                Ok(if truncate {
                    yaml::truncate_yaml(rendered)
                } else {
                    rendered
                })
            }
            None if allow_missing_comment => Ok(format!(
                "# No Helm release secret found for '{release_name}' in namespace '{namespace}'"
            )),
            None => Err(anyhow::anyhow!(
                "no Helm release secret found for '{release_name}' in namespace '{namespace}'"
            )),
        }
    }

    /// Fetches pod events and degrades gracefully when RBAC denies access.
    pub async fn fetch_pod_events(&self, name: &str, namespace: &str) -> Result<Vec<EventInfo>> {
        events::fetch_pod_events(&self.client, name, namespace)
            .await
            .with_context(|| format!("failed preparing events for pod '{namespace}/{name}'"))
    }

    /// Fetches events for any namespaced resource kind. Degrades gracefully on RBAC denial.
    pub async fn fetch_resource_events(
        &self,
        kind: &str,
        name: &str,
        namespace: &str,
    ) -> Result<Vec<EventInfo>> {
        events::fetch_resource_events(&self.client, kind, name, namespace)
            .await
            .with_context(|| format!("failed preparing events for {kind} '{namespace}/{name}'"))
    }

    /// Gets the current and desired replica counts for a deployment.
    pub async fn get_deployment_replicas(&self, name: &str, namespace: &str) -> Result<(i32, i32)> {
        let deployments_api: Api<Deployment> = Api::namespaced(self.client.clone(), namespace);
        let deployment = deployments_api.get(name).await.with_context(|| {
            format!(
                "deployment '{}' not found in namespace '{}'",
                name, namespace
            )
        })?;

        let desired_replicas = deployment
            .spec
            .as_ref()
            .and_then(|s| s.replicas)
            .unwrap_or(1);
        let current_replicas = deployment
            .status
            .as_ref()
            .and_then(|s| s.ready_replicas)
            .unwrap_or(0);

        Ok((current_replicas, desired_replicas))
    }

    /// Polls deployment replicas until target is reached or timeout occurs.
    ///
    /// Polls every 500ms and returns when current_replicas == target_replicas or timeout is reached.
    pub async fn wait_for_replicas(
        &self,
        name: &str,
        namespace: &str,
        target_replicas: i32,
        timeout_secs: u64,
    ) -> Result<()> {
        use std::time::{Duration, Instant};
        use tokio::time::sleep;

        let start = Instant::now();
        let timeout = Duration::from_secs(timeout_secs);

        loop {
            let (current, _) = self
                .get_deployment_replicas(name, namespace)
                .await
                .with_context(|| {
                    format!(
                        "failed polling deployment '{}' in namespace '{}'",
                        name, namespace
                    )
                })?;

            if current == target_replicas {
                return Ok(());
            }

            if start.elapsed() > timeout {
                return Err(anyhow::anyhow!(
                    "timeout waiting for {} replicas in deployment '{}' (namespace '{}')",
                    target_replicas,
                    name,
                    namespace
                ));
            }

            sleep(Duration::from_millis(500)).await;
        }
    }

    /// Creates a port-forward tunnel to a pod's port.
    ///
    /// Returns a tunnel ID on success. The tunnel is managed by PortForwarderService.
    pub async fn create_port_forward(
        &self,
        target: &crate::k8s::portforward::PortForwardTarget,
        config: &crate::k8s::portforward::PortForwardConfig,
    ) -> Result<
        crate::k8s::portforward::PortForwardTunnelInfo,
        crate::k8s::portforward_errors::PortForwardError,
    > {
        use crate::k8s::portforward_errors::PortForwardError;

        // 1. Verify pod exists
        let pods_api: Api<Pod> = Api::namespaced(self.client.clone(), &target.namespace);
        let pod =
            pods_api
                .get(&target.pod_name)
                .await
                .map_err(|_| PortForwardError::PodNotFound {
                    namespace: target.namespace.clone(),
                    pod_name: target.pod_name.clone(),
                })?;

        // 2. Check if port is exposed in pod spec
        let container_ports: Vec<u16> = pod
            .spec
            .as_ref()
            .and_then(|spec| spec.containers.first())
            .and_then(|container| container.ports.as_ref())
            .map(|ports| {
                ports
                    .iter()
                    .filter_map(|p| u16::try_from(p.container_port).ok())
                    .collect()
            })
            .unwrap_or_default();

        if !container_ports.is_empty() && !container_ports.contains(&target.remote_port) {
            return Err(PortForwardError::PortNotExposed {
                pod_name: target.pod_name.clone(),
                port: target.remote_port,
                available_ports: container_ports,
            });
        }

        // 3. Check local port availability
        let local_port = if config.local_port == 0 {
            // Auto-assign a port
            self.find_available_port()
                .await
                .map_err(|_| PortForwardError::PortInUse {
                    port: 0,
                    process_name: Some("auto-assignment failed".to_string()),
                })?
        } else {
            // Verify specific port is available
            self.check_port_available(config.local_port)
                .await
                .map_err(|_| PortForwardError::PortInUse {
                    port: config.local_port,
                    process_name: None,
                })?;
            config.local_port
        };

        // 4. Create the tunnel info
        use std::net::SocketAddr;
        use std::str::FromStr;

        let local_addr = SocketAddr::from_str(&format!("{}:{}", config.bind_address, local_port))
            .map_err(|_| PortForwardError::InvalidPort {
            port: local_port,
            reason: "invalid bind address".to_string(),
        })?;

        let tunnel = crate::k8s::portforward::PortForwardTunnelInfo {
            id: target.id(),
            target: target.clone(),
            local_addr,
            state: crate::k8s::portforward::TunnelState::Active,
        };

        Ok(tunnel)
    }

    /// Checks if a local port is available for binding.
    async fn check_port_available(&self, port: u16) -> Result<()> {
        use tokio::net::TcpListener;

        let bind_addr = format!("127.0.0.1:{}", port);
        let _listener = TcpListener::bind(&bind_addr)
            .await
            .with_context(|| format!("Port {} is not available", port))?;

        Ok(())
    }

    /// Finds an available port on the system.
    async fn find_available_port(&self) -> Result<u16> {
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("failed to find available port")?;

        let port = listener
            .local_addr()
            .context("failed to get local address")?
            .port();

        Ok(port)
    }

    async fn evaluate_access_checks_cached(&self, checks: &[ResourceAccessCheck]) -> Option<bool> {
        let mut saw_unknown = false;

        for check in checks {
            match self.review_access(check).await {
                Some(true) => {}
                Some(false) => return Some(false),
                None => saw_unknown = true,
            }
        }

        if saw_unknown { None } else { Some(true) }
    }

    async fn review_access(&self, check: &ResourceAccessCheck) -> Option<bool> {
        if let Some(cached) = self.access_review_cache.read().await.get(check).copied() {
            return Some(cached);
        }

        let api: Api<SelfSubjectAccessReview> = Api::all(self.client.clone());
        let review = SelfSubjectAccessReview {
            spec: SelfSubjectAccessReviewSpec {
                resource_attributes: Some(ResourceAttributes {
                    field_selector: None,
                    group: check.group.clone(),
                    label_selector: None,
                    name: check.name.clone(),
                    namespace: check.namespace.clone(),
                    resource: Some(check.resource.clone()),
                    subresource: check.subresource.clone(),
                    verb: Some(check.verb.clone()),
                    version: None,
                }),
                ..SelfSubjectAccessReviewSpec::default()
            },
            ..SelfSubjectAccessReview::default()
        };

        let allowed = match api.create(&PostParams::default(), &review).await {
            Ok(response) => response.status.as_ref().map(|status| status.allowed),
            Err(err) if is_forbidden_error(&err) || is_missing_api_error(&err) => None,
            Err(_) => None,
        }?;

        self.access_review_cache
            .write()
            .await
            .insert(check.clone(), allowed);
        Some(allowed)
    }
}

impl K8sClient {
    async fn ensure_ephemeral_containers_supported(&self) -> Result<()> {
        let Ok(version) = self.fetch_cluster_version().await else {
            return Ok(());
        };
        if let Some((major, minor)) = parse_kubernetes_minor_version(version.git_version.as_str())
            && (major < 1 || (major == 1 && minor < EPHEMERAL_CONTAINERS_MIN_MINOR))
        {
            anyhow::bail!(
                "Cluster {} does not support stable ephemeral debug containers. Kubernetes v1.{}+ is required.",
                version.git_version,
                EPHEMERAL_CONTAINERS_MIN_MINOR
            );
        }
        Ok(())
    }
}

fn parse_kubernetes_minor_version(git_version: &str) -> Option<(u32, u32)> {
    let version = git_version.strip_prefix('v').unwrap_or(git_version);
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor_raw = parts.next()?;
    let minor_digits: String = minor_raw
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    let minor = minor_digits.parse().ok()?;
    Some((major, minor))
}

fn build_kube_client(config: Config) -> Result<Client> {
    Ok(ClientBuilder::try_from(config)
        .context("failed to build kube client")?
        .with_layer(&BufferLayer::new(CLIENT_RETRY_BUFFER_SIZE))
        .with_layer(&RetryLayer::new(default_retry_policy()))
        .build())
}

fn default_retry_policy() -> RetryPolicy {
    RetryPolicy::default()
}

fn sort_namespaces(names: Vec<String>) -> Vec<String> {
    let mut names: Vec<String> = names.into_iter().filter(|name| !name.is_empty()).collect();
    names.sort();
    names.dedup();
    names
}

fn custom_resource_api_resource(crd: &CustomResourceDefinitionInfo) -> ApiResource {
    let gvk = GroupVersionKind::gvk(&crd.group, &crd.version, &crd.kind);
    ApiResource::from_gvk_with_plural(&gvk, &crd.plural)
}

fn choose_crd_request_version(versions: &[CustomResourceDefinitionVersion]) -> String {
    versions
        .iter()
        .find(|version| version.served && version.storage)
        .or_else(|| versions.iter().find(|version| version.served))
        .or_else(|| versions.iter().find(|version| version.storage))
        .or_else(|| versions.first())
        .map(|version| version.name.clone())
        .unwrap_or_else(|| "v1".to_string())
}

async fn list_items_or_empty<K, C>(api: &Api<K>, params: &ListParams, context: C) -> Result<Vec<K>>
where
    K: Clone + std::fmt::Debug + serde::de::DeserializeOwned,
    C: FnOnce() -> String,
{
    match api.list(params).await {
        Ok(list) => Ok(list.items),
        Err(err) if is_forbidden_error(&err) => Ok(Vec::new()),
        Err(err) => Err(err).with_context(context),
    }
}

async fn list_metadata_items_or_empty<K, C>(
    api: &Api<K>,
    params: &ListParams,
    context: C,
) -> Result<Vec<PartialObjectMeta<K>>>
where
    K: Clone + std::fmt::Debug + serde::de::DeserializeOwned + kube::Resource,
    C: FnOnce() -> String,
{
    match api.list_metadata(params).await {
        Ok(list) => Ok(list.items),
        Err(err) if is_forbidden_error(&err) => Ok(Vec::new()),
        Err(err) => Err(err).with_context(context),
    }
}

pub(crate) fn is_forbidden_error(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(response) if response.is_forbidden())
}

fn is_metrics_api_unavailable(err: &kube::Error) -> bool {
    match err {
        kube::Error::Api(response) => {
            response.is_not_found()
                || response.code == 503
                || response.message.contains("metrics.k8s.io")
                || response.reason.eq_ignore_ascii_case("NotFound")
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy)]
struct FluxResourceKindSpec {
    kind: &'static str,
    group: &'static str,
    plural: &'static str,
    versions: &'static [&'static str],
    namespaced: bool,
}

#[derive(Debug, Clone, Copy)]
struct FluxApiTarget {
    spec: FluxResourceKindSpec,
    version: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FluxWatchTarget {
    pub group: &'static str,
    pub version: &'static str,
    pub kind: &'static str,
    pub plural: &'static str,
    pub namespaced: bool,
}

const FLUX_RESOURCE_KIND_SPECS: &[FluxResourceKindSpec] = &[
    FluxResourceKindSpec {
        kind: "Kustomization",
        group: "kustomize.toolkit.fluxcd.io",
        plural: "kustomizations",
        versions: &["v1", "v1beta2"],
        namespaced: true,
    },
    FluxResourceKindSpec {
        kind: "HelmRelease",
        group: "helm.toolkit.fluxcd.io",
        plural: "helmreleases",
        versions: &["v2", "v2beta2", "v2beta1"],
        namespaced: true,
    },
    FluxResourceKindSpec {
        kind: "GitRepository",
        group: "source.toolkit.fluxcd.io",
        plural: "gitrepositories",
        versions: &["v1", "v1beta2", "v1beta1"],
        namespaced: true,
    },
    FluxResourceKindSpec {
        kind: "HelmRepository",
        group: "source.toolkit.fluxcd.io",
        plural: "helmrepositories",
        versions: &["v1", "v1beta2", "v1beta1"],
        namespaced: true,
    },
    FluxResourceKindSpec {
        kind: "OCIRepository",
        group: "source.toolkit.fluxcd.io",
        plural: "ocirepositories",
        versions: &["v1", "v1beta2"],
        namespaced: true,
    },
    FluxResourceKindSpec {
        kind: "Bucket",
        group: "source.toolkit.fluxcd.io",
        plural: "buckets",
        versions: &["v1", "v1beta2", "v1beta1"],
        namespaced: true,
    },
    FluxResourceKindSpec {
        kind: "HelmChart",
        group: "source.toolkit.fluxcd.io",
        plural: "helmcharts",
        versions: &["v1", "v1beta2", "v1beta1"],
        namespaced: true,
    },
    FluxResourceKindSpec {
        kind: "AlertProvider",
        group: "notification.toolkit.fluxcd.io",
        plural: "alertproviders",
        versions: &["v1beta3", "v1beta2", "v1beta1"],
        namespaced: true,
    },
    FluxResourceKindSpec {
        kind: "Alert",
        group: "notification.toolkit.fluxcd.io",
        plural: "alerts",
        versions: &["v1beta3", "v1beta2", "v1beta1"],
        namespaced: true,
    },
    FluxResourceKindSpec {
        kind: "Receiver",
        group: "notification.toolkit.fluxcd.io",
        plural: "receivers",
        versions: &["v1", "v1beta3", "v1beta2", "v1beta1"],
        namespaced: true,
    },
    FluxResourceKindSpec {
        kind: "ImageRepository",
        group: "image.toolkit.fluxcd.io",
        plural: "imagerepositories",
        versions: &["v1beta2", "v1beta1"],
        namespaced: true,
    },
    FluxResourceKindSpec {
        kind: "ImagePolicy",
        group: "image.toolkit.fluxcd.io",
        plural: "imagepolicies",
        versions: &["v1beta2", "v1beta1"],
        namespaced: true,
    },
    FluxResourceKindSpec {
        kind: "ImageUpdateAutomation",
        group: "image.toolkit.fluxcd.io",
        plural: "imageupdateautomations",
        versions: &["v1beta2", "v1beta1"],
        namespaced: true,
    },
];

pub(crate) fn is_missing_api_error(err: &kube::Error) -> bool {
    if let kube::Error::Api(response) = err
        && response.is_not_found()
    {
        return true;
    }
    let text = err.to_string();
    text.contains("the server could not find the requested resource")
        || text.contains("could not find the requested resource")
        || text.contains("NotFound")
}

fn vulnerability_severity_rank(severity: crate::k8s::dtos::AlertSeverity) -> u8 {
    match severity {
        crate::k8s::dtos::AlertSeverity::Error => 3,
        crate::k8s::dtos::AlertSeverity::Warning => 2,
        crate::k8s::dtos::AlertSeverity::Info => 1,
    }
}

fn parse_vulnerability_report(
    item: DynamicObject,
    cluster_scoped: bool,
) -> Option<VulnerabilityReportInfo> {
    let metadata = item.metadata;
    let data = item.data;
    let labels = metadata.labels.unwrap_or_default();
    let report = data.get("report")?;
    let summary = report.get("summary")?;
    let vulnerabilities = report
        .get("vulnerabilities")
        .and_then(|value| value.as_array());

    let fixed_version_count = vulnerabilities
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    item.get("fixedVersion")
                        .and_then(|value| value.as_str())
                        .is_some_and(|value| !value.trim().is_empty())
                })
                .count()
        })
        .unwrap_or_default();

    let name = metadata.name?;
    let namespace = metadata.namespace.unwrap_or_default();
    let resource_kind = labels
        .get("trivy-operator.resource.kind")
        .cloned()
        .unwrap_or_else(|| "Unknown".to_string());
    let resource_name = labels
        .get("trivy-operator.resource.name")
        .cloned()
        .unwrap_or_else(|| name.clone());
    let resource_namespace = labels
        .get("trivy-operator.resource.namespace")
        .cloned()
        .or_else(|| (!namespace.is_empty()).then_some(namespace.clone()))
        .unwrap_or_default();

    Some(VulnerabilityReportInfo {
        name,
        namespace,
        resource_kind,
        resource_name,
        resource_namespace,
        container_name: labels.get("trivy-operator.container.name").cloned(),
        artifact_repository: report
            .pointer("/artifact/repository")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        artifact_tag: report
            .pointer("/artifact/tag")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        registry_server: report
            .pointer("/registry/server")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        fixable_count: fixed_version_count,
        counts: VulnerabilitySummaryCounts {
            critical: summary_count(summary, "criticalCount"),
            high: summary_count(summary, "highCount"),
            medium: summary_count(summary, "mediumCount"),
            low: summary_count(summary, "lowCount"),
            unknown: summary_count(summary, "unknownCount"),
        },
        scanner_name: report
            .pointer("/scanner/name")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        scanner_vendor: report
            .pointer("/scanner/vendor")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        scanner_version: report
            .pointer("/scanner/version")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        update_timestamp: report
            .pointer("/updateTimestamp")
            .and_then(|value| value.as_str())
            .and_then(parse_timestamp),
        cluster_scoped,
    })
}

fn summary_count(summary: &serde_json::Value, key: &str) -> usize {
    summary
        .get(key)
        .and_then(|value| value.as_u64())
        .unwrap_or_default() as usize
}

fn flux_ready_details(data: &serde_json::Value) -> (Option<bool>, Option<String>) {
    let Some(conditions) = data
        .pointer("/status/conditions")
        .and_then(|value| value.as_array())
    else {
        return (None, None);
    };

    let ready_condition = conditions.iter().find(|item| {
        item.get("type")
            .and_then(|value| value.as_str())
            .is_some_and(|ty| ty.eq_ignore_ascii_case("Ready"))
    });

    let Some(condition) = ready_condition else {
        return (None, None);
    };

    let ready = condition
        .get("status")
        .and_then(|value| value.as_str())
        .map(|status| status.eq_ignore_ascii_case("True"));
    let message = condition
        .get("message")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .or_else(|| {
            condition
                .get("reason")
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
        });

    (ready, message)
}

fn flux_artifact_details(data: &serde_json::Value) -> Option<String> {
    let revision = data
        .pointer("/status/artifact/revision")
        .and_then(|value| value.as_str());
    let digest = data
        .pointer("/status/artifact/digest")
        .and_then(|value| value.as_str());

    if let Some(revision) = revision {
        if let Some(digest) = digest {
            return Some(format!("{revision} ({digest})"));
        }
        return Some(revision.to_string());
    }

    data.pointer("/status/artifact/url")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

fn flux_source_url(data: &serde_json::Value) -> Option<String> {
    data.pointer("/spec/url")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .or_else(|| {
            data.pointer("/spec/endpoint")
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
        })
}

fn flux_parse_conditions(data: &serde_json::Value) -> Vec<crate::k8s::dtos::FluxCondition> {
    data.pointer("/status/conditions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|item| crate::k8s::dtos::FluxCondition {
                    type_: item
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    status: item
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    reason: item
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    message: item
                        .get("message")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    timestamp: item
                        .get("lastTransitionTime")
                        .and_then(|v| v.as_str())
                        .and_then(parse_timestamp),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn flux_source_ref(data: &serde_json::Value) -> Option<String> {
    let source_ref = data.pointer("/spec/sourceRef")?;
    let kind = source_ref.get("kind").and_then(|v| v.as_str())?;
    let name = source_ref.get("name").and_then(|v| v.as_str())?;
    let ns = source_ref
        .get("namespace")
        .and_then(|v| v.as_str())
        .map(|ns| format!(" ({ns})"))
        .unwrap_or_default();
    Some(format!("{kind}/{name}{ns}"))
}

impl K8sClient {
    /// Fetches Helm releases by reading Helm-managed Secrets (owner=helm, type=helm.sh/release.v1).
    /// Decodes the release metadata from the secret's labels without requiring the Helm CLI.
    pub async fn fetch_helm_releases(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<crate::k8s::dtos::HelmReleaseInfo>> {
        use k8s_openapi::api::core::v1::Secret;
        use kube::api::ListParams;

        let secrets_api: Api<Secret> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        // Helm v3 stores releases as secrets with label owner=helm
        let lp = ListParams::default().labels("owner=helm");
        let list = list_items_or_empty(&secrets_api, &lp, || {
            "failed fetching Helm release secrets".to_string()
        })
        .await?;

        let now = now();
        let mut releases: Vec<crate::k8s::dtos::HelmReleaseInfo> = list
            .into_iter()
            .filter_map(|secret| {
                let labels = secret.metadata.labels.as_ref()?;
                // Only process helm release secrets
                if labels.get("owner")?.as_str() != "helm" {
                    return None;
                }
                let release_name = labels.get("name")?.clone();
                let status = labels
                    .get("status")
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                let revision: i32 = labels
                    .get("version")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);

                let ns = secret.metadata.namespace.clone().unwrap_or_default();
                let created_at = secret
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0));
                let age = age_from_created_at(created_at, now);

                // Try to get chart info from the "helmrelease" label pattern
                let chart_label = labels.get("chart").cloned().unwrap_or_default();
                let (chart_name, chart_version) = if let Some(pos) = chart_label.rfind('-') {
                    let (name, ver) = chart_label.split_at(pos);
                    (name.to_string(), ver.trim_start_matches('-').to_string())
                } else if !chart_label.is_empty() {
                    (chart_label, String::new())
                } else {
                    (release_name.clone(), String::new())
                };

                Some(crate::k8s::dtos::HelmReleaseInfo {
                    name: release_name,
                    namespace: ns,
                    chart: chart_name,
                    chart_version,
                    app_version: String::new(), // not available from secret labels alone
                    status,
                    revision,
                    updated: created_at,
                    age,
                })
            })
            .collect();

        // Sort by namespace then name
        releases.sort_unstable_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
        Ok(releases)
    }

    /// Fetches common Flux resources for the dedicated Flux view.
    ///
    /// Resources are loaded directly from Flux CRDs (if installed). Missing CRDs
    /// are treated as empty lists so clusters without Flux remain healthy.
    pub async fn fetch_flux_resources(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<crate::k8s::dtos::FluxResourceInfo>> {
        const FLUX_FETCH_CONCURRENCY: usize = 3;

        let targets = self.discover_flux_targets().await?;
        let mut out = Vec::new();
        let mut needs_rediscovery = false;
        let mut fetches = stream::iter(targets.into_iter().map(|target| async move {
            (
                target,
                self.fetch_flux_resources_for_version(target.spec, target.version, namespace)
                    .await,
            )
        }))
        .buffer_unordered(FLUX_FETCH_CONCURRENCY);

        while let Some((target, result)) = fetches.next().await {
            match result {
                Ok(mut items) => out.append(&mut items),
                Err(err) if is_missing_api_error(&err) => {
                    // Flux CRDs changed while running: invalidate and rediscover next refresh.
                    needs_rediscovery = true;
                }
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!(
                            "failed fetching Flux {} resources ({}/{})",
                            target.spec.kind, target.spec.group, target.version
                        )
                    });
                }
            }
        }

        if needs_rediscovery {
            self.invalidate_flux_targets_cache().await;
        }

        out.sort_unstable_by(|left, right| {
            left.namespace
                .cmp(&right.namespace)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(out)
    }

    pub(crate) async fn discover_flux_watch_targets(&self) -> Result<Vec<FluxWatchTarget>> {
        Ok(self
            .discover_flux_targets()
            .await?
            .into_iter()
            .map(|target| FluxWatchTarget {
                group: target.spec.group,
                version: target.version,
                kind: target.spec.kind,
                plural: target.spec.plural,
                namespaced: target.spec.namespaced,
            })
            .collect())
    }

    async fn invalidate_flux_targets_cache(&self) {
        *self.flux_targets_cache.write().await = None;
    }

    async fn discover_flux_targets(&self) -> Result<Vec<FluxApiTarget>> {
        if let Some(cached) = self.flux_targets_cache.read().await.as_ref() {
            return Ok(cached.clone());
        }

        let mut discovered = Vec::new();
        for spec in FLUX_RESOURCE_KIND_SPECS {
            for &version in spec.versions {
                match self.probe_flux_target(*spec, version).await {
                    Ok(()) => {
                        discovered.push(FluxApiTarget {
                            spec: *spec,
                            version,
                        });
                        break;
                    }
                    Err(err) if is_missing_api_error(&err) => continue,
                    Err(err) => {
                        return Err(err).with_context(|| {
                            format!(
                                "failed discovering Flux {} resources ({}/{})",
                                spec.kind, spec.group, version
                            )
                        });
                    }
                }
            }
        }

        let mut guard = self.flux_targets_cache.write().await;
        if let Some(cached) = guard.as_ref() {
            return Ok(cached.clone());
        }
        *guard = Some(discovered.clone());
        Ok(discovered)
    }

    async fn probe_flux_target(
        &self,
        spec: FluxResourceKindSpec,
        version: &'static str,
    ) -> std::result::Result<(), kube::Error> {
        let gvk = GroupVersionKind::gvk(spec.group, version, spec.kind);
        let mut ar = ApiResource::from_gvk(&gvk);
        ar.plural = spec.plural.to_string();
        let api: Api<DynamicObject> = Api::all_with(self.client.clone(), &ar);
        match api.list(&ListParams::default().limit(1)).await {
            Ok(_) => {}
            Err(err) if is_forbidden_error(&err) => {}
            Err(err) => return Err(err),
        }
        Ok(())
    }

    async fn fetch_flux_resources_for_version(
        &self,
        spec: FluxResourceKindSpec,
        version: &str,
        namespace: Option<&str>,
    ) -> std::result::Result<Vec<crate::k8s::dtos::FluxResourceInfo>, kube::Error> {
        let gvk = GroupVersionKind::gvk(spec.group, version, spec.kind);
        let mut ar = ApiResource::from_gvk(&gvk);
        ar.plural = spec.plural.to_string();

        let api: Api<DynamicObject> = if spec.namespaced {
            match namespace {
                Some(ns) => Api::namespaced_with(self.client.clone(), ns, &ar),
                None => Api::all_with(self.client.clone(), &ar),
            }
        } else {
            Api::all_with(self.client.clone(), &ar)
        };

        let list = match api.list(&ListParams::default()).await {
            Ok(list) => list.items,
            Err(err) if is_forbidden_error(&err) => Vec::new(),
            Err(err) => return Err(err),
        };
        let now = now();
        let mut resources = Vec::with_capacity(list.len());
        for item in list {
            let created_at = item
                .metadata
                .creation_timestamp
                .as_ref()
                .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0));
            let suspended = item
                .data
                .pointer("/spec/suspend")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let (ready, message) = flux_ready_details(&item.data);
            let conditions = flux_parse_conditions(&item.data);
            let artifact = flux_artifact_details(&item.data);
            let source_url = flux_source_url(&item.data);
            let source_ref = flux_source_ref(&item.data);
            let is_stalled = conditions.iter().any(|c| {
                c.type_.eq_ignore_ascii_case("Stalled") && c.status.eq_ignore_ascii_case("True")
            });
            let status = if suspended {
                "Suspended".to_string()
            } else if is_stalled {
                "Stalled".to_string()
            } else {
                match ready {
                    Some(true) => "Ready".to_string(),
                    Some(false) => "NotReady".to_string(),
                    None => "Unknown".to_string(),
                }
            };
            let last_reconcile_time = item
                .data
                .pointer("/status/lastHandledReconcileAt")
                .and_then(|v| v.as_str())
                .and_then(parse_timestamp);
            let last_applied_revision = item
                .data
                .pointer("/status/lastAppliedRevision")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let last_attempted_revision = item
                .data
                .pointer("/status/lastAttemptedRevision")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let observed_generation = item
                .data
                .pointer("/status/observedGeneration")
                .and_then(|v| v.as_i64());
            let generation = item.metadata.generation;
            let interval = item
                .data
                .pointer("/spec/interval")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let timeout = item
                .data
                .pointer("/spec/timeout")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            resources.push(crate::k8s::dtos::FluxResourceInfo {
                name: item
                    .metadata
                    .name
                    .unwrap_or_else(|| "<unknown>".to_string()),
                namespace: item.metadata.namespace,
                kind: spec.kind.to_string(),
                group: spec.group.to_string(),
                version: version.to_string(),
                plural: spec.plural.to_string(),
                source_url,
                status,
                message,
                artifact,
                suspended,
                created_at,
                age: age_from_created_at(created_at, now),
                conditions,
                last_reconcile_time,
                last_applied_revision,
                last_attempted_revision,
                observed_generation,
                generation,
                source_ref,
                interval,
                timeout,
            });
        }

        Ok(resources)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use jiff::ToSpan;
    use k8s_openapi::api::{
        core::v1::{NodeCondition, NodeStatus},
        rbac::v1::{PolicyRule, Subject},
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
    use kube::core::Status;

    use super::*;
    use crate::k8s::conversions::{
        format_job_completions, format_job_duration, int_or_string_to_string,
        job_status_from_counts, node_condition_true, node_role, parse_k8s_quantity,
        quota_percent_used, rule_from_policy_rule, subject_from_k8s,
    };

    fn node_with_condition(condition_type: &str, status: &str) -> Node {
        Node {
            metadata: ObjectMeta::default(),
            status: Some(NodeStatus {
                conditions: Some(vec![NodeCondition {
                    type_: condition_type.to_string(),
                    status: status.to_string(),
                    ..NodeCondition::default()
                }]),
                ..NodeStatus::default()
            }),
            ..Node::default()
        }
    }

    fn api_error(code: u16, reason: &str, message: &str) -> kube::Error {
        kube::Error::Api(Status::failure(message, reason).with_code(code).boxed())
    }

    fn crd_version(name: &str, served: bool, storage: bool) -> CustomResourceDefinitionVersion {
        CustomResourceDefinitionVersion {
            name: name.to_string(),
            served,
            storage,
            ..CustomResourceDefinitionVersion::default()
        }
    }

    /// Verifies node readiness helper returns true only for matching True condition.
    #[test]
    fn node_condition_true_matches_expected_condition() {
        let ready_node = node_with_condition("Ready", "True");
        let not_ready_node = node_with_condition("Ready", "False");

        assert!(node_condition_true(&ready_node, "Ready"));
        assert!(!node_condition_true(&not_ready_node, "Ready"));
    }

    /// Verifies unknown condition types are treated as false.
    #[test]
    fn node_condition_true_unknown_type_is_false() {
        let node = node_with_condition("DiskPressure", "True");
        assert!(!node_condition_true(&node, "Ready"));
    }

    /// Verifies namespace names are sorted and deduplicated.
    #[test]
    fn test_fetch_namespaces_sorted() {
        let sorted = sort_namespaces(vec![
            "zeta".to_string(),
            "default".to_string(),
            "".to_string(),
            "alpha".to_string(),
            "default".to_string(),
        ]);

        assert_eq!(sorted, vec!["alpha", "default", "zeta"]);
    }

    #[tokio::test]
    async fn build_kube_client_supports_retry_layer_stack() {
        let config = Config::new("http://127.0.0.1:6443".parse().expect("valid uri"));
        build_kube_client(config).expect("client should build with retry layer");
    }

    /// Verifies control-plane labels map to master role.
    #[test]
    fn node_role_detects_master_from_control_plane_label() {
        let mut labels = BTreeMap::new();
        labels.insert(
            "node-role.kubernetes.io/control-plane".to_string(),
            "".to_string(),
        );

        let node = Node {
            metadata: ObjectMeta {
                labels: Some(labels),
                ..ObjectMeta::default()
            },
            ..Node::default()
        };

        assert_eq!(node_role(&node), "master");
    }

    /// Verifies nodes without control-plane labels default to worker role.
    #[test]
    fn node_role_defaults_to_worker() {
        let node = Node::default();
        assert_eq!(node_role(&node), "worker");
    }

    #[test]
    fn parse_vulnerability_report_reads_summary_labels_and_fixable_count() {
        let obj = DynamicObject {
            types: None,
            metadata: ObjectMeta {
                name: Some("api-abc123".to_string()),
                namespace: Some("default".to_string()),
                labels: Some(BTreeMap::from([
                    (
                        "trivy-operator.resource.kind".to_string(),
                        "Deployment".to_string(),
                    ),
                    (
                        "trivy-operator.resource.name".to_string(),
                        "api".to_string(),
                    ),
                    (
                        "trivy-operator.container.name".to_string(),
                        "web".to_string(),
                    ),
                ])),
                ..ObjectMeta::default()
            },
            data: serde_json::json!({
                "report": {
                    "artifact": { "repository": "ghcr.io/demo/api", "tag": "1.2.3" },
                    "scanner": { "name": "Trivy", "vendor": "Aqua", "version": "0.59.1" },
                    "summary": {
                        "criticalCount": 1,
                        "highCount": 2,
                        "mediumCount": 3,
                        "lowCount": 4,
                        "unknownCount": 5
                    },
                    "updateTimestamp": "2026-03-26T06:00:00Z",
                    "vulnerabilities": [
                        { "vulnerabilityID": "CVE-1", "fixedVersion": "2.0.0" },
                        { "vulnerabilityID": "CVE-2", "fixedVersion": "" },
                        { "vulnerabilityID": "CVE-3", "fixedVersion": "1.3.0" }
                    ]
                }
            }),
        };

        let report = parse_vulnerability_report(obj, false).expect("report should parse");
        assert_eq!(report.resource_kind, "Deployment");
        assert_eq!(report.resource_name, "api");
        assert_eq!(report.container_name.as_deref(), Some("web"));
        assert_eq!(
            report.artifact_repository.as_deref(),
            Some("ghcr.io/demo/api")
        );
        assert_eq!(report.artifact_tag.as_deref(), Some("1.2.3"));
        assert_eq!(report.counts.critical, 1);
        assert_eq!(report.counts.high, 2);
        assert_eq!(report.counts.medium, 3);
        assert_eq!(report.counts.low, 4);
        assert_eq!(report.counts.unknown, 5);
        assert_eq!(report.fixable_count, 2);
        assert!(report.update_timestamp.is_some());
    }

    /// Verifies node mapping preserves defaults for missing metadata fields.
    #[test]
    fn fetch_nodes_mapping_handles_missing_fields() {
        let node = Node::default();
        let info = NodeInfo {
            name: node
                .metadata
                .name
                .clone()
                .unwrap_or_else(|| "<unknown>".to_string()),
            ready: node_condition_true(&node, "Ready"),
            kubelet_version: node
                .status
                .as_ref()
                .and_then(|status| status.node_info.as_ref())
                .map(|info| info.kubelet_version.clone())
                .unwrap_or_else(|| "unknown".to_string()),
            os_image: node
                .status
                .as_ref()
                .and_then(|status| status.node_info.as_ref())
                .map(|info| info.os_image.clone())
                .unwrap_or_else(|| "unknown".to_string()),
            role: node_role(&node),
            cpu_allocatable: None,
            memory_allocatable: None,
            created_at: None,
            memory_pressure: node_condition_true(&node, "MemoryPressure"),
            disk_pressure: node_condition_true(&node, "DiskPressure"),
            pid_pressure: node_condition_true(&node, "PIDPressure"),
            network_unavailable: node_condition_true(&node, "NetworkUnavailable"),
            unschedulable: node
                .spec
                .as_ref()
                .and_then(|s| s.unschedulable)
                .unwrap_or(false),
        };

        assert_eq!(info.name, "<unknown>");
        assert_eq!(info.kubelet_version, "unknown");
        assert_eq!(info.os_image, "unknown");
        assert_eq!(info.role, "worker");
    }

    /// Verifies invalid resource kind in YAML fetch returns descriptive error.
    #[tokio::test]
    async fn fetch_resource_yaml_invalid_kind_has_clear_error() {
        let k8s = K8sClient::dummy();

        let err = k8s
            .fetch_resource_yaml("unsupported", "name", None)
            .await
            .expect_err("invalid kind should error");

        let err_text = format!("{err:#}");
        assert!(
            err_text.contains("failed preparing YAML") && err_text.contains("unsupported"),
            "error should include context and root cause, got: {err_text}"
        );
    }

    #[test]
    fn job_status_determination_matches_expected_priority() {
        assert_eq!(job_status_from_counts(1, 0, 0), "Succeeded");
        assert_eq!(job_status_from_counts(0, 1, 0), "Failed");
        assert_eq!(job_status_from_counts(0, 0, 2), "Running");
        assert_eq!(job_status_from_counts(0, 0, 0), "Pending");
    }

    #[test]
    fn job_completions_format_uses_succeeded_over_parallelism() {
        assert_eq!(format_job_completions(3, 10), "3/10");
        assert_eq!(format_job_completions(0, 0), "0/1");
        assert_eq!(format_job_completions(-1, -2), "0/1");
    }

    #[test]
    fn job_duration_is_human_readable() {
        let start = now()
            .checked_sub(125.seconds())
            .expect("timestamp in range");
        let end = start
            .checked_add(125.seconds())
            .expect("timestamp in range");
        let out = format_job_duration(Some(start), Some(end));

        assert_eq!(out.as_deref(), Some("2m5s"));
    }

    #[test]
    fn policy_rule_mapping_extracts_all_fields() {
        let input = PolicyRule {
            verbs: vec!["get".to_string(), "list".to_string()],
            api_groups: Some(vec!["apps".to_string()]),
            resources: Some(vec!["deployments".to_string()]),
            resource_names: Some(vec!["api".to_string()]),
            non_resource_urls: Some(vec!["/healthz".to_string()]),
        };

        let mapped = rule_from_policy_rule(&input);
        assert_eq!(mapped.verbs, vec!["get", "list"]);
        assert_eq!(mapped.api_groups, vec!["apps"]);
        assert_eq!(mapped.resources, vec!["deployments"]);
        assert_eq!(mapped.resource_names, vec!["api"]);
        assert_eq!(mapped.non_resource_urls, vec!["/healthz"]);
    }

    #[test]
    fn role_binding_subject_mapping_keeps_namespace_and_api_group() {
        let input = Subject {
            kind: "ServiceAccount".to_string(),
            name: "builder".to_string(),
            namespace: Some("default".to_string()),
            api_group: Some("rbac.authorization.k8s.io".to_string()),
        };

        let mapped = subject_from_k8s(&input);
        assert_eq!(mapped.kind, "ServiceAccount");
        assert_eq!(mapped.name, "builder");
        assert_eq!(mapped.namespace.as_deref(), Some("default"));
        assert_eq!(
            mapped.api_group.as_deref(),
            Some("rbac.authorization.k8s.io")
        );
    }

    #[test]
    fn job_duration_none_without_start_time() {
        assert!(format_job_duration(None, None).is_none());
    }

    #[test]
    fn parse_k8s_quantity_understands_cpu_and_memory_units() {
        assert_eq!(parse_k8s_quantity("500m"), Some(0.5));
        assert_eq!(parse_k8s_quantity("1"), Some(1.0));
        assert_eq!(parse_k8s_quantity("1Gi"), Some(1024.0_f64.powi(3)));
    }

    #[test]
    fn quota_percent_used_computes_expected_ratio() {
        let mut hard = BTreeMap::new();
        let mut used = BTreeMap::new();
        hard.insert("pods".to_string(), "10".to_string());
        used.insert("pods".to_string(), "4".to_string());

        let result = quota_percent_used(&hard, &used);
        assert_eq!(result.get("pods").copied(), Some(40.0));
    }

    #[test]
    fn int_or_string_to_string_handles_both_variants() {
        assert_eq!(int_or_string_to_string(&IntOrString::Int(2)), "2");
        assert_eq!(
            int_or_string_to_string(&IntOrString::String("50%".to_string())),
            "50%"
        );
    }

    #[test]
    fn metrics_api_unavailable_detects_not_found_errors() {
        let err = api_error(
            404,
            "NotFound",
            "the server could not find the requested resource",
        );

        assert!(is_metrics_api_unavailable(&err));
    }

    #[test]
    fn metrics_api_unavailable_ignores_unrelated_api_errors() {
        let err = api_error(403, "Forbidden", "forbidden");

        assert!(!is_metrics_api_unavailable(&err));
    }

    #[test]
    fn forbidden_error_detection_only_matches_403() {
        let forbidden = api_error(403, "Forbidden", "forbidden");
        let timeout = api_error(504, "Timeout", "timeout");

        assert!(is_forbidden_error(&forbidden));
        assert!(!is_forbidden_error(&timeout));
    }

    #[test]
    fn choose_crd_request_version_prefers_served_storage() {
        let versions = vec![
            crd_version("v1beta1", true, false),
            crd_version("v1", true, true),
        ];
        assert_eq!(choose_crd_request_version(&versions), "v1");
    }

    #[test]
    fn choose_crd_request_version_avoids_unserved_storage_versions() {
        let versions = vec![
            crd_version("v1beta1", false, true),
            crd_version("v1", true, false),
        ];
        assert_eq!(choose_crd_request_version(&versions), "v1");
    }

    #[test]
    fn choose_crd_request_version_falls_back_to_storage_when_needed() {
        let versions = vec![
            crd_version("v1beta1", false, false),
            crd_version("v1", false, true),
        ];
        assert_eq!(choose_crd_request_version(&versions), "v1");
    }

    #[test]
    fn choose_crd_request_version_defaults_when_empty() {
        assert_eq!(choose_crd_request_version(&[]), "v1");
    }

    #[test]
    fn custom_resource_api_resource_uses_explicit_plural() {
        let crd = CustomResourceDefinitionInfo {
            name: "policies.demo.io".to_string(),
            group: "demo.io".to_string(),
            version: "v1".to_string(),
            kind: "Policy".to_string(),
            plural: "policies".to_string(),
            scope: "Namespaced".to_string(),
            instances: 0,
        };
        let api_resource = custom_resource_api_resource(&crd);

        assert_eq!(api_resource.group, "demo.io");
        assert_eq!(api_resource.version, "v1");
        assert_eq!(api_resource.kind, "Policy");
        assert_eq!(api_resource.plural, "policies");
    }
}
