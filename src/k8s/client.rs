//! Kubernetes API client wrapper used by KubecTUI.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use anyhow::{Context, Result};
use chrono::Utc;
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
    rbac::v1::{ClusterRole, ClusterRoleBinding, PolicyRule, Role, RoleBinding, Subject},
    scheduling::v1::PriorityClass,
};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::{
    Api, Client, Config,
    api::{
        ApiResource, DynamicObject, GroupVersionKind, ListParams, Patch, PatchParams, PostParams,
    },
    config::KubeConfigOptions,
};

use crate::k8s::{events, yaml};
use crate::{
    app::ResourceRef,
    authorization::{
        ActionAuthorizationMap, DetailActionAuthorization, ResourceAccessCheck,
        detail_action_requires_authorization,
    },
    policy::DetailAction,
};

pub use crate::k8s::{
    dtos::{
        ClusterInfo, ClusterRoleBindingInfo, ClusterRoleInfo, ClusterVersionInfo, ConfigMapInfo,
        CronJobInfo, CustomResourceDefinitionInfo, CustomResourceInfo, DaemonSetInfo,
        DeploymentInfo, EndpointInfo, FluxResourceInfo, HelmReleaseInfo, HpaInfo, IngressClassInfo,
        IngressInfo, JobInfo, K8sEventInfo, LimitRangeInfo, LimitSpec, NamespaceInfo,
        NetworkPolicyInfo, NodeInfo, NodeMetricsInfo, PodDisruptionBudgetInfo, PodInfo,
        PodMetricsInfo, PriorityClassInfo, PvInfo, PvcInfo, RbacRule, ReplicaSetInfo,
        ReplicationControllerInfo, ResourceQuotaInfo, RoleBindingInfo, RoleBindingSubject,
        RoleInfo, SecretInfo, ServiceAccountInfo, ServiceInfo, StatefulSetInfo, StorageClassInfo,
    },
    events::EventInfo,
};

const MAX_EVENTS_LIST_LIMIT: u32 = 1000;
const MAX_RECENT_EVENTS_ITEMS: usize = 250;

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
        let client = Client::try_from(config).context("failed to build kube client")?;

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
        let client = Client::try_from(config).context("failed to build kube client")?;

        Ok(Self {
            client,
            cluster_url,
            cluster_context: Some(context.to_string()),
            cluster_version_cache: Arc::new(tokio::sync::RwLock::new(None)),
            flux_targets_cache: Arc::new(tokio::sync::RwLock::new(None)),
            access_review_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        })
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

    /// Fetches all nodes from the current cluster.
    pub async fn fetch_nodes(&self) -> Result<Vec<NodeInfo>> {
        let nodes_api: Api<Node> = Api::all(self.client.clone());
        let list = list_items_or_empty(&nodes_api, &ListParams::default(), || {
            "failed fetching Kubernetes nodes".to_string()
        })
        .await?;

        let nodes = list
            .into_iter()
            .map(crate::k8s::conversions::node_to_info)
            .collect();

        Ok(nodes)
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
        let list = list_items_or_empty(&ns_api, &ListParams::default(), || {
            "failed fetching namespaces".to_string()
        })
        .await?;

        let names: Vec<String> = list
            .iter()
            .map(|ns| ns.metadata.name.clone().unwrap_or_default())
            .collect();

        Ok(sort_namespaces(names))
    }

    /// Fetches pods from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_pods(&self, namespace: Option<&str>) -> Result<Vec<PodInfo>> {
        let pods_api: Api<Pod> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = list_items_or_empty(&pods_api, &ListParams::default(), || {
            if let Some(ns) = namespace {
                format!("failed fetching pods in namespace '{ns}'")
            } else {
                "failed fetching pods across all namespaces".to_string()
            }
        })
        .await?;

        let pods = list
            .into_iter()
            .map(crate::k8s::conversions::pod_to_info)
            .collect();

        Ok(pods)
    }

    /// Fetches services from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_services(&self, namespace: Option<&str>) -> Result<Vec<ServiceInfo>> {
        let services_api: Api<Service> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = list_items_or_empty(&services_api, &ListParams::default(), || {
            if let Some(ns) = namespace {
                format!("failed fetching services in namespace '{ns}'")
            } else {
                "failed fetching services across all namespaces".to_string()
            }
        })
        .await?;

        let services = list
            .into_iter()
            .map(crate::k8s::conversions::service_to_info)
            .collect();

        Ok(services)
    }

    /// Fetches deployments from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_deployments(&self, namespace: Option<&str>) -> Result<Vec<DeploymentInfo>> {
        let deployments_api: Api<Deployment> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = list_items_or_empty(&deployments_api, &ListParams::default(), || {
            if let Some(ns) = namespace {
                format!("failed fetching deployments in namespace '{ns}'")
            } else {
                "failed fetching deployments across all namespaces".to_string()
            }
        })
        .await?;

        let deployments = list
            .into_iter()
            .map(crate::k8s::conversions::deployment_to_info)
            .collect();

        Ok(deployments)
    }

    /// Fetches statefulsets from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_statefulsets(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<StatefulSetInfo>> {
        let statefulsets_api: Api<StatefulSet> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = list_items_or_empty(&statefulsets_api, &ListParams::default(), || {
            if let Some(ns) = namespace {
                format!("failed fetching statefulsets in namespace '{ns}'")
            } else {
                "failed fetching statefulsets across all namespaces".to_string()
            }
        })
        .await?;

        let statefulsets = list
            .into_iter()
            .map(crate::k8s::conversions::statefulset_to_info)
            .collect();

        Ok(statefulsets)
    }

    /// Fetches daemonsets from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_daemonsets(&self, namespace: Option<&str>) -> Result<Vec<DaemonSetInfo>> {
        let daemonsets_api: Api<DaemonSet> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = list_items_or_empty(&daemonsets_api, &ListParams::default(), || {
            if let Some(ns) = namespace {
                format!("failed fetching daemonsets in namespace '{ns}'")
            } else {
                "failed fetching daemonsets across all namespaces".to_string()
            }
        })
        .await?;

        let daemonsets = list
            .into_iter()
            .map(crate::k8s::conversions::daemonset_to_info)
            .collect();

        Ok(daemonsets)
    }

    /// Fetches replica sets from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_replicasets(&self, namespace: Option<&str>) -> Result<Vec<ReplicaSetInfo>> {
        let api: Api<ReplicaSet> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = list_items_or_empty(&api, &ListParams::default(), || {
            if let Some(ns) = namespace {
                format!("failed fetching replicasets in namespace '{ns}'")
            } else {
                "failed fetching replicasets across all namespaces".to_string()
            }
        })
        .await?;

        let items = list
            .into_iter()
            .map(crate::k8s::conversions::replicaset_to_info)
            .collect();

        Ok(items)
    }

    /// Fetches replication controllers from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_replication_controllers(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<ReplicationControllerInfo>> {
        let api: Api<ReplicationController> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = list_items_or_empty(&api, &ListParams::default(), || {
            if let Some(ns) = namespace {
                format!("failed fetching replicationcontrollers in namespace '{ns}'")
            } else {
                "failed fetching replicationcontrollers across all namespaces".to_string()
            }
        })
        .await?;

        let items = list
            .into_iter()
            .map(crate::k8s::conversions::replication_controller_to_info)
            .collect();

        Ok(items)
    }

    /// Fetches service accounts from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_service_accounts(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<ServiceAccountInfo>> {
        let service_accounts_api: Api<ServiceAccount> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = list_items_or_empty(&service_accounts_api, &ListParams::default(), || {
            if let Some(ns) = namespace {
                format!("failed fetching serviceaccounts in namespace '{ns}'")
            } else {
                "failed fetching serviceaccounts across all namespaces".to_string()
            }
        })
        .await?;

        let now = Utc::now();
        let service_accounts = list
            .into_iter()
            .map(|sa| {
                let created_at = sa.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

                ServiceAccountInfo {
                    name: sa.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: sa
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    secrets_count: sa.secrets.as_ref().map_or(0, |v| v.len()),
                    image_pull_secrets_count: sa.image_pull_secrets.as_ref().map_or(0, |v| v.len()),
                    automount_service_account_token: sa.automount_service_account_token,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect();

        Ok(service_accounts)
    }

    /// Fetches roles from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_roles(&self, namespace: Option<&str>) -> Result<Vec<RoleInfo>> {
        let roles_api: Api<Role> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = list_items_or_empty(&roles_api, &ListParams::default(), || {
            if let Some(ns) = namespace {
                format!("failed fetching roles in namespace '{ns}'")
            } else {
                "failed fetching roles across all namespaces".to_string()
            }
        })
        .await?;

        let now = Utc::now();
        let roles = list
            .into_iter()
            .map(|role| {
                let created_at = role.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

                RoleInfo {
                    name: role
                        .metadata
                        .name
                        .unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: role
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    rules: role
                        .rules
                        .as_ref()
                        .map(|rules| rules.iter().map(rule_from_policy_rule).collect())
                        .unwrap_or_default(),
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect();

        Ok(roles)
    }

    /// Fetches role bindings from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_role_bindings(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<RoleBindingInfo>> {
        let role_bindings_api: Api<RoleBinding> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = list_items_or_empty(&role_bindings_api, &ListParams::default(), || {
            if let Some(ns) = namespace {
                format!("failed fetching rolebindings in namespace '{ns}'")
            } else {
                "failed fetching rolebindings across all namespaces".to_string()
            }
        })
        .await?;

        let now = Utc::now();
        let role_bindings = list
            .into_iter()
            .map(|rb| {
                let created_at = rb.metadata.creation_timestamp.as_ref().map(|ts| ts.0);
                let role_ref = rb.role_ref;

                RoleBindingInfo {
                    name: rb.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: rb
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    role_ref_kind: role_ref.kind,
                    role_ref_name: role_ref.name,
                    subjects: rb
                        .subjects
                        .as_ref()
                        .map(|subjects| subjects.iter().map(subject_from_k8s).collect())
                        .unwrap_or_default(),
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect();

        Ok(role_bindings)
    }

    /// Fetches cluster roles (cluster-wide only).
    pub async fn fetch_cluster_roles(&self) -> Result<Vec<ClusterRoleInfo>> {
        let cluster_roles_api: Api<ClusterRole> = Api::all(self.client.clone());

        let list = list_items_or_empty(&cluster_roles_api, &ListParams::default(), || {
            "failed fetching clusterroles".to_string()
        })
        .await?;

        let now = Utc::now();
        let cluster_roles = list
            .into_iter()
            .map(|cr| {
                let created_at = cr.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

                ClusterRoleInfo {
                    name: cr.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                    rules: cr
                        .rules
                        .as_ref()
                        .map(|rules| rules.iter().map(rule_from_policy_rule).collect())
                        .unwrap_or_default(),
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect();

        Ok(cluster_roles)
    }

    /// Fetches cluster role bindings (cluster-wide only).
    pub async fn fetch_cluster_role_bindings(&self) -> Result<Vec<ClusterRoleBindingInfo>> {
        let cluster_role_bindings_api: Api<ClusterRoleBinding> = Api::all(self.client.clone());

        let list = list_items_or_empty(&cluster_role_bindings_api, &ListParams::default(), || {
            "failed fetching clusterrolebindings".to_string()
        })
        .await?;

        let now = Utc::now();
        let cluster_role_bindings = list
            .into_iter()
            .map(|crb| {
                let created_at = crb.metadata.creation_timestamp.as_ref().map(|ts| ts.0);
                let role_ref = crb.role_ref;

                ClusterRoleBindingInfo {
                    name: crb.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                    role_ref_kind: role_ref.kind,
                    role_ref_name: role_ref.name,
                    subjects: crb
                        .subjects
                        .as_ref()
                        .map(|subjects| subjects.iter().map(subject_from_k8s).collect())
                        .unwrap_or_default(),
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect();

        Ok(cluster_role_bindings)
    }

    /// Fetches jobs from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_jobs(&self, namespace: Option<&str>) -> Result<Vec<JobInfo>> {
        let jobs_api: Api<Job> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = list_items_or_empty(&jobs_api, &ListParams::default(), || {
            if let Some(ns) = namespace {
                format!("failed fetching jobs in namespace '{ns}'")
            } else {
                "failed fetching jobs across all namespaces".to_string()
            }
        })
        .await?;

        let jobs = list
            .into_iter()
            .map(crate::k8s::conversions::job_to_info)
            .collect();

        Ok(jobs)
    }

    /// Fetches cronjobs from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_cronjobs(&self, namespace: Option<&str>) -> Result<Vec<CronJobInfo>> {
        let cronjobs_api: Api<CronJob> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = list_items_or_empty(&cronjobs_api, &ListParams::default(), || {
            if let Some(ns) = namespace {
                format!("failed fetching cronjobs in namespace '{ns}'")
            } else {
                "failed fetching cronjobs across all namespaces".to_string()
            }
        })
        .await?;

        let cronjobs = list
            .into_iter()
            .map(crate::k8s::conversions::cronjob_to_info)
            .collect();

        Ok(cronjobs)
    }

    /// Fetches resource quotas from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_resource_quotas(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<ResourceQuotaInfo>> {
        let api: Api<ResourceQuota> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = list_items_or_empty(&api, &ListParams::default(), || {
            if let Some(ns) = namespace {
                format!("failed fetching resource quotas in namespace '{ns}'")
            } else {
                "failed fetching resource quotas across all namespaces".to_string()
            }
        })
        .await?;

        let now = Utc::now();
        let quotas = list
            .into_iter()
            .map(|quota| {
                let hard = quota
                    .status
                    .as_ref()
                    .and_then(|status| status.hard.as_ref())
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(k, v)| (k, v.0))
                    .collect::<BTreeMap<_, _>>();

                let used = quota
                    .status
                    .as_ref()
                    .and_then(|status| status.used.as_ref())
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(k, v)| (k, v.0))
                    .collect::<BTreeMap<_, _>>();

                let percent_used = quota_percent_used(&hard, &used);
                let created_at = quota.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

                ResourceQuotaInfo {
                    name: quota
                        .metadata
                        .name
                        .unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: quota
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    hard,
                    used,
                    percent_used,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect();

        Ok(quotas)
    }

    /// Fetches limit ranges from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_limit_ranges(&self, namespace: Option<&str>) -> Result<Vec<LimitRangeInfo>> {
        let api: Api<LimitRange> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = list_items_or_empty(&api, &ListParams::default(), || {
            if let Some(ns) = namespace {
                format!("failed fetching limit ranges in namespace '{ns}'")
            } else {
                "failed fetching limit ranges across all namespaces".to_string()
            }
        })
        .await?;

        let now = Utc::now();
        let ranges = list
            .into_iter()
            .map(|range| {
                let limits = range
                    .spec
                    .as_ref()
                    .map(|spec| {
                        spec.limits
                            .iter()
                            .map(|item| LimitSpec {
                                type_: item.type_.clone(),
                                min: quantity_map_to_string_map(item.min.clone()),
                                max: quantity_map_to_string_map(item.max.clone()),
                                default: quantity_map_to_string_map(item.default.clone()),
                                default_request: quantity_map_to_string_map(
                                    item.default_request.clone(),
                                ),
                                max_limit_request_ratio: quantity_map_to_string_map(
                                    item.max_limit_request_ratio.clone(),
                                ),
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                let created_at = range.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

                LimitRangeInfo {
                    name: range
                        .metadata
                        .name
                        .unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: range
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    limits,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect();

        Ok(ranges)
    }

    /// Fetches pod disruption budgets from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_pod_disruption_budgets(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<PodDisruptionBudgetInfo>> {
        let api: Api<PodDisruptionBudget> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = list_items_or_empty(&api, &ListParams::default(), || {
            if let Some(ns) = namespace {
                format!("failed fetching pod disruption budgets in namespace '{ns}'")
            } else {
                "failed fetching pod disruption budgets across all namespaces".to_string()
            }
        })
        .await?;

        let now = Utc::now();
        let pdbs = list
            .into_iter()
            .map(|pdb| {
                let spec = pdb.spec.as_ref();
                let status = pdb.status.as_ref();
                let created_at = pdb.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

                PodDisruptionBudgetInfo {
                    name: pdb.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: pdb
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    min_available: spec
                        .and_then(|s| s.min_available.as_ref())
                        .map(int_or_string_to_string),
                    max_unavailable: spec
                        .and_then(|s| s.max_unavailable.as_ref())
                        .map(int_or_string_to_string),
                    current_healthy: status.map(|s| s.current_healthy).unwrap_or(0),
                    desired_healthy: status.map(|s| s.desired_healthy).unwrap_or(0),
                    disruptions_allowed: status.map(|s| s.disruptions_allowed).unwrap_or(0),
                    expected_pods: status.map(|s| s.expected_pods).unwrap_or(0),
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect();

        Ok(pdbs)
    }

    /// Fetches Endpoints.
    pub async fn fetch_endpoints(&self, namespace: Option<&str>) -> Result<Vec<EndpointInfo>> {
        let api: Api<Endpoints> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };
        let list = list_items_or_empty(&api, &ListParams::default(), || {
            "failed fetching endpoints".to_string()
        })
        .await?;
        let now = Utc::now();
        Ok(list
            .into_iter()
            .map(|ep| {
                let created_at = ep
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts.0.to_rfc3339()).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let mut addresses = Vec::new();
                let mut ports = Vec::new();
                if let Some(subsets) = ep.subsets {
                    for subset in &subsets {
                        if let Some(addrs) = &subset.addresses {
                            for addr in addrs {
                                addresses.push(addr.ip.clone());
                            }
                        }
                        if let Some(ps) = &subset.ports {
                            for p in ps {
                                ports.push(format!(
                                    "{}/{}",
                                    p.port,
                                    p.protocol.as_deref().unwrap_or("TCP")
                                ));
                            }
                        }
                    }
                }
                EndpointInfo {
                    name: ep.metadata.name.unwrap_or_default(),
                    namespace: ep.metadata.namespace.unwrap_or_default(),
                    addresses,
                    ports,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect())
    }

    /// Fetches Ingresses.
    pub async fn fetch_ingresses(&self, namespace: Option<&str>) -> Result<Vec<IngressInfo>> {
        let api: Api<Ingress> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };
        let list = list_items_or_empty(&api, &ListParams::default(), || {
            "failed fetching ingresses".to_string()
        })
        .await?;
        let now = Utc::now();
        Ok(list
            .into_iter()
            .map(|ing| {
                let created_at = ing
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts.0.to_rfc3339()).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let class = ing.spec.as_ref().and_then(|s| s.ingress_class_name.clone());
                let hosts: Vec<String> = ing
                    .spec
                    .as_ref()
                    .and_then(|s| s.rules.as_ref())
                    .map(|rules| rules.iter().filter_map(|r| r.host.clone()).collect())
                    .unwrap_or_default();
                let address = ing
                    .status
                    .as_ref()
                    .and_then(|s| s.load_balancer.as_ref())
                    .and_then(|lb| lb.ingress.as_ref())
                    .and_then(|ingresses| ingresses.first())
                    .and_then(|i| i.ip.clone().or_else(|| i.hostname.clone()));
                let backend_services: Vec<(String, String)> = ing
                    .spec
                    .as_ref()
                    .map(|spec| {
                        let mut backends = Vec::new();
                        if let Some(default_backend) = &spec.default_backend
                            && let Some(svc) = &default_backend.service
                        {
                            let port = svc
                                .port
                                .as_ref()
                                .map(|p| {
                                    p.name.clone().unwrap_or_else(|| {
                                        p.number.map(|n| n.to_string()).unwrap_or_default()
                                    })
                                })
                                .unwrap_or_default();
                            backends.push((svc.name.clone(), port));
                        }
                        for rule in spec.rules.as_deref().unwrap_or_default() {
                            if let Some(http) = &rule.http {
                                for path in &http.paths {
                                    if let Some(svc) = &path.backend.service {
                                        let port = svc
                                            .port
                                            .as_ref()
                                            .map(|p| {
                                                p.name.clone().unwrap_or_else(|| {
                                                    p.number
                                                        .map(|n| n.to_string())
                                                        .unwrap_or_default()
                                                })
                                            })
                                            .unwrap_or_default();
                                        backends.push((svc.name.clone(), port));
                                    }
                                }
                            }
                        }
                        backends.sort();
                        backends.dedup();
                        backends
                    })
                    .unwrap_or_default();
                IngressInfo {
                    name: ing.metadata.name.unwrap_or_default(),
                    namespace: ing.metadata.namespace.unwrap_or_default(),
                    class,
                    hosts,
                    address,
                    ports: vec!["80".to_string(), "443".to_string()],
                    backend_services,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect())
    }

    /// Fetches IngressClasses.
    pub async fn fetch_ingress_classes(&self) -> Result<Vec<IngressClassInfo>> {
        let api: Api<IngressClass> = Api::all(self.client.clone());
        let list = list_items_or_empty(&api, &ListParams::default(), || {
            "failed fetching ingress classes".to_string()
        })
        .await?;
        let now = Utc::now();
        Ok(list
            .into_iter()
            .map(|ic| {
                let created_at = ic
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts.0.to_rfc3339()).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let is_default = ic
                    .metadata
                    .annotations
                    .as_ref()
                    .and_then(|a| a.get("ingressclass.kubernetes.io/is-default-class"))
                    .map(|v| v == "true")
                    .unwrap_or(false);
                IngressClassInfo {
                    name: ic.metadata.name.unwrap_or_default(),
                    controller: ic
                        .spec
                        .as_ref()
                        .map(|s| s.controller.clone().unwrap_or_default())
                        .unwrap_or_default(),
                    is_default,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect())
    }

    /// Fetches NetworkPolicies.
    pub async fn fetch_network_policies(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<NetworkPolicyInfo>> {
        let api: Api<NetworkPolicy> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };
        let list = list_items_or_empty(&api, &ListParams::default(), || {
            "failed fetching network policies".to_string()
        })
        .await?;
        let now = Utc::now();
        Ok(list
            .into_iter()
            .map(|np| {
                let created_at = np
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts.0.to_rfc3339()).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let pod_selector = np
                    .spec
                    .as_ref()
                    .map(|s| {
                        s.pod_selector
                            .match_labels
                            .as_ref()
                            .map(|ml| {
                                ml.iter()
                                    .map(|(k, v)| format!("{k}={v}"))
                                    .collect::<Vec<_>>()
                                    .join(",")
                            })
                            .unwrap_or_else(|| "<all>".to_string())
                    })
                    .unwrap_or_default();
                let ingress_rules = np
                    .spec
                    .as_ref()
                    .and_then(|s| s.ingress.as_ref())
                    .map(|r| r.len())
                    .unwrap_or(0);
                let egress_rules = np
                    .spec
                    .as_ref()
                    .and_then(|s| s.egress.as_ref())
                    .map(|r| r.len())
                    .unwrap_or(0);
                NetworkPolicyInfo {
                    name: np.metadata.name.unwrap_or_default(),
                    namespace: np.metadata.namespace.unwrap_or_default(),
                    pod_selector,
                    ingress_rules,
                    egress_rules,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect())
    }

    /// Fetches ConfigMaps.
    pub async fn fetch_config_maps(&self, namespace: Option<&str>) -> Result<Vec<ConfigMapInfo>> {
        let api: Api<ConfigMap> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };
        let list = list_items_or_empty(&api, &ListParams::default(), || {
            "failed fetching configmaps".to_string()
        })
        .await?;
        let now = Utc::now();
        Ok(list
            .into_iter()
            .map(|cm| {
                let created_at = cm
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts.0.to_rfc3339()).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let data_count = cm.data.as_ref().map(|d| d.len()).unwrap_or(0)
                    + cm.binary_data.as_ref().map(|d| d.len()).unwrap_or(0);
                ConfigMapInfo {
                    name: cm.metadata.name.unwrap_or_default(),
                    namespace: cm.metadata.namespace.unwrap_or_default(),
                    data_count,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect())
    }

    /// Fetches Secrets.
    pub async fn fetch_secrets(&self, namespace: Option<&str>) -> Result<Vec<SecretInfo>> {
        let api: Api<Secret> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };
        let list = list_items_or_empty(&api, &ListParams::default(), || {
            "failed fetching secrets".to_string()
        })
        .await?;
        let now = Utc::now();
        Ok(list
            .into_iter()
            .map(|s| {
                let created_at = s
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts.0.to_rfc3339()).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let data_count = s.data.as_ref().map(|d| d.len()).unwrap_or(0);
                SecretInfo {
                    name: s.metadata.name.unwrap_or_default(),
                    namespace: s.metadata.namespace.unwrap_or_default(),
                    type_: s.type_.unwrap_or_else(|| "Opaque".to_string()),
                    data_count,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect())
    }

    /// Fetches HPAs.
    pub async fn fetch_hpas(&self, namespace: Option<&str>) -> Result<Vec<HpaInfo>> {
        let api: Api<HorizontalPodAutoscaler> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };
        let list = list_items_or_empty(&api, &ListParams::default(), || {
            "failed fetching HPAs".to_string()
        })
        .await?;
        let now = Utc::now();
        Ok(list
            .into_iter()
            .map(|hpa| {
                let created_at = hpa
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts.0.to_rfc3339()).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let spec = hpa.spec.as_ref();
                let status = hpa.status.as_ref();
                let reference = spec
                    .map(|s| format!("{}/{}", s.scale_target_ref.kind, s.scale_target_ref.name))
                    .unwrap_or_default();
                HpaInfo {
                    name: hpa.metadata.name.unwrap_or_default(),
                    namespace: hpa.metadata.namespace.unwrap_or_default(),
                    reference,
                    min_replicas: spec.and_then(|s| s.min_replicas),
                    max_replicas: spec.map(|s| s.max_replicas).unwrap_or(0),
                    current_replicas: status.and_then(|s| s.current_replicas).unwrap_or(0),
                    desired_replicas: status.map(|s| s.desired_replicas).unwrap_or(0),
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect())
    }

    /// Fetches PersistentVolumeClaims.
    pub async fn fetch_pvcs(&self, namespace: Option<&str>) -> Result<Vec<PvcInfo>> {
        let api: Api<PersistentVolumeClaim> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };
        let list = list_items_or_empty(&api, &ListParams::default(), || {
            "failed fetching PVCs".to_string()
        })
        .await?;
        let now = Utc::now();
        Ok(list
            .into_iter()
            .map(|pvc| {
                let created_at = pvc
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts.0.to_rfc3339()).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let spec = pvc.spec.as_ref();
                let status = pvc.status.as_ref();
                let access_modes = spec
                    .and_then(|s| s.access_modes.as_ref())
                    .map(|modes| modes.to_vec())
                    .unwrap_or_default();
                let capacity = status
                    .and_then(|s| s.capacity.as_ref())
                    .and_then(|c| c.get("storage"))
                    .map(|q| q.0.clone());
                PvcInfo {
                    name: pvc.metadata.name.unwrap_or_default(),
                    namespace: pvc.metadata.namespace.unwrap_or_default(),
                    status: status
                        .and_then(|s| s.phase.clone())
                        .unwrap_or_else(|| "Unknown".to_string()),
                    volume: spec.and_then(|s| s.volume_name.clone()),
                    capacity,
                    access_modes,
                    storage_class: spec.and_then(|s| s.storage_class_name.clone()),
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect())
    }

    /// Fetches PersistentVolumes.
    pub async fn fetch_pvs(&self) -> Result<Vec<PvInfo>> {
        let api: Api<PersistentVolume> = Api::all(self.client.clone());
        let list = list_items_or_empty(&api, &ListParams::default(), || {
            "failed fetching PVs".to_string()
        })
        .await?;
        let now = Utc::now();
        Ok(list
            .into_iter()
            .map(|pv| {
                let created_at = pv
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts.0.to_rfc3339()).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let spec = pv.spec.as_ref();
                let access_modes = spec
                    .and_then(|s| s.access_modes.as_ref())
                    .map(|modes| modes.to_vec())
                    .unwrap_or_default();
                let capacity = spec
                    .and_then(|s| s.capacity.as_ref())
                    .and_then(|c| c.get("storage"))
                    .map(|q| q.0.clone());
                let claim = spec.and_then(|s| s.claim_ref.as_ref()).map(|cr| {
                    format!(
                        "{}/{}",
                        cr.namespace.as_deref().unwrap_or(""),
                        cr.name.as_deref().unwrap_or("")
                    )
                });
                PvInfo {
                    name: pv.metadata.name.unwrap_or_default(),
                    capacity,
                    access_modes,
                    reclaim_policy: spec
                        .and_then(|s| s.persistent_volume_reclaim_policy.clone())
                        .unwrap_or_else(|| "Retain".to_string()),
                    status: pv
                        .status
                        .as_ref()
                        .and_then(|s| s.phase.clone())
                        .unwrap_or_else(|| "Unknown".to_string()),
                    claim,
                    storage_class: spec.and_then(|s| s.storage_class_name.clone()),
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect())
    }

    /// Fetches StorageClasses.
    pub async fn fetch_storage_classes(&self) -> Result<Vec<StorageClassInfo>> {
        use k8s_openapi::api::storage::v1::StorageClass;
        let api: Api<StorageClass> = Api::all(self.client.clone());
        let list = list_items_or_empty(&api, &ListParams::default(), || {
            "failed fetching storage classes".to_string()
        })
        .await?;
        let now = Utc::now();
        Ok(list
            .into_iter()
            .map(|sc| {
                let created_at = sc
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts.0.to_rfc3339()).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let is_default = sc
                    .metadata
                    .annotations
                    .as_ref()
                    .and_then(|a| a.get("storageclass.kubernetes.io/is-default-class"))
                    .map(|v| v == "true")
                    .unwrap_or(false);
                StorageClassInfo {
                    name: sc.metadata.name.unwrap_or_default(),
                    provisioner: sc.provisioner,
                    reclaim_policy: sc.reclaim_policy,
                    volume_binding_mode: sc.volume_binding_mode,
                    allow_volume_expansion: sc.allow_volume_expansion.unwrap_or(false),
                    is_default,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect())
    }

    /// Fetches Namespaces as NamespaceInfo.
    pub async fn fetch_namespace_list(&self) -> Result<Vec<NamespaceInfo>> {
        let api: Api<Namespace> = Api::all(self.client.clone());
        let list = list_items_or_empty(&api, &ListParams::default(), || {
            "failed fetching namespaces".to_string()
        })
        .await?;
        let now = Utc::now();
        Ok(list
            .into_iter()
            .map(|ns| {
                let created_at = ns
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts.0.to_rfc3339()).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                NamespaceInfo {
                    name: ns.metadata.name.unwrap_or_default(),
                    status: ns
                        .status
                        .as_ref()
                        .and_then(|s| s.phase.clone())
                        .unwrap_or_else(|| "Active".to_string()),
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect())
    }

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
        let now = Utc::now();
        let mut events: Vec<K8sEventInfo> = list
            .into_iter()
            .map(|ev| {
                let created_at = ev
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts.0.to_rfc3339()).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let last_seen = ev
                    .last_timestamp
                    .as_ref()
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts.0.to_rfc3339()).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let involved = format!(
                    "{}/{}",
                    ev.involved_object.kind.as_deref().unwrap_or(""),
                    ev.involved_object.name.as_deref().unwrap_or("")
                );
                K8sEventInfo {
                    name: ev.metadata.name.unwrap_or_default(),
                    namespace: ev.metadata.namespace.unwrap_or_default(),
                    reason: ev.reason.unwrap_or_default(),
                    message: ev.message.unwrap_or_default(),
                    type_: ev.type_.unwrap_or_else(|| "Normal".to_string()),
                    count: ev.count.unwrap_or(1),
                    involved_object: involved,
                    last_seen,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                }
            })
            .collect();
        // Sort by last_seen descending
        events.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
        events.truncate(MAX_RECENT_EVENTS_ITEMS);
        Ok(events)
    }

    /// Fetches PriorityClasses.
    pub async fn fetch_priority_classes(&self) -> Result<Vec<PriorityClassInfo>> {
        let api: Api<PriorityClass> = Api::all(self.client.clone());
        let list = list_items_or_empty(&api, &ListParams::default(), || {
            "failed fetching priority classes".to_string()
        })
        .await?;
        let now = Utc::now();
        Ok(list
            .into_iter()
            .map(|pc| {
                let created_at = pc
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts.0.to_rfc3339()).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                PriorityClassInfo {
                    name: pc.metadata.name.unwrap_or_default(),
                    value: pc.value,
                    global_default: pc.global_default.unwrap_or(false),
                    description: pc.description.unwrap_or_default(),
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect())
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

            let version = spec
                .versions
                .iter()
                .find(|v| v.storage)
                .or_else(|| spec.versions.iter().find(|v| v.served))
                .or_else(|| spec.versions.first())
                .map(|v| v.name.clone())
                .unwrap_or_else(|| "v1".to_string());

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

        crds.sort_by(|a, b| a.name.cmp(&b.name));
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

        let now = Utc::now();
        let mut resources = list
            .into_iter()
            .map(|item| {
                let created_at = item.metadata.creation_timestamp.as_ref().map(|ts| ts.0);
                CustomResourceInfo {
                    name: item
                        .metadata
                        .name
                        .unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: item.metadata.namespace,
                    created_at,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                }
            })
            .collect::<Vec<_>>();

        resources.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(resources)
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

        for action in DetailAction::ORDER {
            if !detail_action_requires_authorization(action) {
                continue;
            }

            let checks = resource.authorization_checks(action);
            if checks.is_empty() {
                continue;
            }

            match self.evaluate_access_checks(&checks).await {
                Some(true) => {
                    authorizations.insert(action, DetailActionAuthorization::Allowed);
                }
                Some(false) => {
                    authorizations.insert(action, DetailActionAuthorization::Denied);
                }
                None => {}
            }
        }

        authorizations
    }

    pub async fn is_detail_action_authorized(
        &self,
        resource: &ResourceRef,
        action: DetailAction,
    ) -> Option<bool> {
        if !detail_action_requires_authorization(action) {
            return Some(true);
        }

        let checks = resource.authorization_checks(action);
        if checks.is_empty() {
            return None;
        }

        self.evaluate_access_checks(&checks).await
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

        let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
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
        use k8s_openapi::api::core::v1::Secret;
        use kube::api::ListParams;

        let secrets_api: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        let lp = ListParams::default().labels(&format!("owner=helm,name={release_name}"));
        let list = list_items_or_empty(&secrets_api, &lp, || {
            format!("failed fetching Helm release secrets for '{release_name}'")
        })
        .await?;

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
                Ok(yaml::truncate_yaml(rendered))
            }
            None => Ok(format!(
                "# No Helm release secret found for '{release_name}' in namespace '{namespace}'"
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

    async fn evaluate_access_checks(&self, checks: &[ResourceAccessCheck]) -> Option<bool> {
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
                    group: check.group.clone(),
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

fn sort_namespaces(names: Vec<String>) -> Vec<String> {
    let mut names: Vec<String> = names.into_iter().filter(|name| !name.is_empty()).collect();
    names.sort();
    names.dedup();
    names
}

fn custom_resource_api_resource(crd: &CustomResourceDefinitionInfo) -> ApiResource {
    let gvk = GroupVersionKind::gvk(&crd.group, &crd.version, &crd.kind);
    let mut ar = ApiResource::from_gvk(&gvk);
    ar.plural = crd.plural.clone();
    ar
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

fn is_forbidden_error(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(response) if response.code == 403)
}

fn is_metrics_api_unavailable(err: &kube::Error) -> bool {
    match err {
        kube::Error::Api(response) => {
            response.code == 404
                || response.code == 503
                || response.message.contains("metrics.k8s.io")
                || response.reason.eq_ignore_ascii_case("NotFound")
        }
        _ => false,
    }
}

fn rule_from_policy_rule(rule: &PolicyRule) -> RbacRule {
    RbacRule {
        verbs: rule.verbs.clone(),
        api_groups: rule.api_groups.clone().unwrap_or_default(),
        resources: rule.resources.clone().unwrap_or_default(),
        resource_names: rule.resource_names.clone().unwrap_or_default(),
        non_resource_urls: rule.non_resource_urls.clone().unwrap_or_default(),
    }
}

fn subject_from_k8s(subject: &Subject) -> RoleBindingSubject {
    RoleBindingSubject {
        kind: subject.kind.clone(),
        name: subject.name.clone(),
        namespace: subject.namespace.clone(),
        api_group: subject.api_group.clone(),
    }
}

fn quota_percent_used(
    hard: &BTreeMap<String, String>,
    used: &BTreeMap<String, String>,
) -> BTreeMap<String, f64> {
    hard.iter()
        .filter_map(|(key, hard_value)| {
            let used_value = used.get(key)?;
            let used_num = parse_k8s_quantity(used_value)?;
            let hard_num = parse_k8s_quantity(hard_value)?;
            if hard_num <= 0.0 {
                return None;
            }
            Some((key.clone(), (used_num / hard_num) * 100.0))
        })
        .collect()
}

fn quantity_map_to_string_map(
    value: Option<BTreeMap<String, k8s_openapi::apimachinery::pkg::api::resource::Quantity>>,
) -> BTreeMap<String, String> {
    value
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, v.0))
        .collect()
}

fn int_or_string_to_string(value: &IntOrString) -> String {
    match value {
        IntOrString::Int(v) => v.to_string(),
        IntOrString::String(v) => v.clone(),
    }
}

fn parse_k8s_quantity(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let factors = [
        ("Ki", 1024.0),
        ("Mi", 1024.0_f64.powi(2)),
        ("Gi", 1024.0_f64.powi(3)),
        ("Ti", 1024.0_f64.powi(4)),
        ("Pi", 1024.0_f64.powi(5)),
        ("Ei", 1024.0_f64.powi(6)),
        ("n", 1e-9),
        ("u", 1e-6),
        ("m", 1e-3),
        ("K", 1e3),
        ("M", 1e6),
        ("G", 1e9),
        ("T", 1e12),
        ("P", 1e15),
        ("E", 1e18),
    ];

    for (suffix, factor) in factors {
        if let Some(number) = raw.strip_suffix(suffix) {
            let value = number.trim().parse::<f64>().ok()?;
            return Some(value * factor);
        }
    }

    raw.parse::<f64>().ok()
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

fn is_missing_api_error(err: &kube::Error) -> bool {
    if let kube::Error::Api(response) = err
        && response.code == 404
    {
        return true;
    }
    let text = err.to_string();
    text.contains("the server could not find the requested resource")
        || text.contains("could not find the requested resource")
        || text.contains("NotFound")
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
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.with_timezone(&chrono::Utc)),
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

        let now = chrono::Utc::now();
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
                let created_at = secret.metadata.creation_timestamp.as_ref().map(|ts| ts.0);
                let age = created_at.and_then(|ts| (now - ts).to_std().ok());

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
        releases.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
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

        out.sort_by(|left, right| {
            left.namespace
                .cmp(&right.namespace)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(out)
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
        let now = chrono::Utc::now();
        let mut resources = Vec::with_capacity(list.len());
        for item in list {
            let created_at = item.metadata.creation_timestamp.as_ref().map(|ts| ts.0);
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
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));
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
                age: created_at.and_then(|ts| (now - ts).to_std().ok()),
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

    use k8s_openapi::api::{
        core::v1::{NodeCondition, NodeStatus},
        rbac::v1::{PolicyRule, Subject},
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    use super::*;
    use crate::k8s::conversions::{
        format_job_completions, format_job_duration, job_status_from_counts, node_condition_true,
        node_role,
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
        let cfg = kube::Config::new("http://127.0.0.1:1".parse().expect("valid URL"));
        let client = Client::try_from(cfg).expect("client should build for test URL");

        let k8s = K8sClient {
            client,
            cluster_url: "http://127.0.0.1:1".to_string(),
            cluster_context: Some("test".to_string()),
            cluster_version_cache: Arc::new(tokio::sync::RwLock::new(None)),
            flux_targets_cache: Arc::new(tokio::sync::RwLock::new(None)),
            access_review_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        };

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
        let start = Utc::now() - chrono::Duration::seconds(125);
        let out = format_job_duration(Some(start), Some(start + chrono::Duration::seconds(125)));

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
        let err = kube::Error::Api(kube::error::ErrorResponse {
            status: "Failure".to_string(),
            message: "the server could not find the requested resource".to_string(),
            reason: "NotFound".to_string(),
            code: 404,
        });

        assert!(is_metrics_api_unavailable(&err));
    }

    #[test]
    fn metrics_api_unavailable_ignores_unrelated_api_errors() {
        let err = kube::Error::Api(kube::error::ErrorResponse {
            status: "Failure".to_string(),
            message: "forbidden".to_string(),
            reason: "Forbidden".to_string(),
            code: 403,
        });

        assert!(!is_metrics_api_unavailable(&err));
    }

    #[test]
    fn forbidden_error_detection_only_matches_403() {
        let forbidden = kube::Error::Api(kube::error::ErrorResponse {
            status: "Failure".to_string(),
            message: "forbidden".to_string(),
            reason: "Forbidden".to_string(),
            code: 403,
        });
        let timeout = kube::Error::Api(kube::error::ErrorResponse {
            status: "Failure".to_string(),
            message: "timeout".to_string(),
            reason: "Timeout".to_string(),
            code: 504,
        });

        assert!(is_forbidden_error(&forbidden));
        assert!(!is_forbidden_error(&timeout));
    }
}
