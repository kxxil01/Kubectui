//! Kubernetes API client wrapper used by KubecTUI.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::Utc;
use k8s_openapi::api::{
    apps::v1::{DaemonSet, Deployment, StatefulSet},
    batch::v1::{CronJob, Job},
    core::v1::{LimitRange, Namespace, Node, Pod, PodSpec, ResourceQuota, Service, ServiceAccount},
    policy::v1::PodDisruptionBudget,
    rbac::v1::{ClusterRole, ClusterRoleBinding, PolicyRule, Role, RoleBinding, Subject},
};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::{
    Api, Client, Config,
    api::{ApiResource, DynamicObject, GroupVersionKind, ListParams},
    config::KubeConfigOptions,
};

use crate::k8s::{events, yaml};

pub use crate::k8s::{
    dtos::{
        ClusterInfo, ClusterRoleBindingInfo, ClusterRoleInfo, CronJobInfo,
        CustomResourceDefinitionInfo, CustomResourceInfo, DaemonSetInfo, DeploymentInfo, JobInfo,
        LimitRangeInfo, LimitSpec, NodeInfo, NodeMetricsInfo, PodDisruptionBudgetInfo, PodInfo,
        PodMetricsInfo, RbacRule, ResourceQuotaInfo, RoleBindingInfo, RoleBindingSubject, RoleInfo,
        ServiceAccountInfo, ServiceInfo, StatefulSetInfo,
    },
    events::EventInfo,
};

/// Configured Kubernetes client wrapper.
#[derive(Clone)]
pub struct K8sClient {
    client: Client,
    cluster_url: String,
    cluster_context: Option<String>,
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
        })
    }

    /// Returns the configured Kubernetes cluster API endpoint.
    pub fn cluster_url(&self) -> &str {
        &self.cluster_url
    }

    /// Returns reference to the underlying Kubernetes client.
    pub fn get_client(&self) -> Client {
        self.client.clone()
    }

    /// Fetches all nodes from the current cluster.
    pub async fn fetch_nodes(&self) -> Result<Vec<NodeInfo>> {
        let nodes_api: Api<Node> = Api::all(self.client.clone());
        let list = nodes_api
            .list(&ListParams::default())
            .await
            .context("failed fetching Kubernetes nodes")?;

        let nodes = list
            .into_iter()
            .map(|node| {
                let alloc = node
                    .status
                    .as_ref()
                    .and_then(|status| status.allocatable.as_ref());
                let name = node
                    .metadata
                    .name
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| "<unknown>".to_string());

                NodeInfo {
                    name,
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
                    cpu_allocatable: alloc.and_then(|a| a.get("cpu").map(|q| q.0.clone())),
                    memory_allocatable: alloc.and_then(|a| a.get("memory").map(|q| q.0.clone())),
                    created_at: node.metadata.creation_timestamp.as_ref().map(|ts| ts.0),
                    memory_pressure: node_condition_true(&node, "MemoryPressure"),
                    disk_pressure: node_condition_true(&node, "DiskPressure"),
                    pid_pressure: node_condition_true(&node, "PIDPressure"),
                    network_unavailable: node_condition_true(&node, "NetworkUnavailable"),
                }
            })
            .collect();

        Ok(nodes)
    }

    /// Fetches available namespaces sorted alphabetically.
    pub async fn fetch_namespaces(&self) -> Result<Vec<String>> {
        let ns_api: Api<Namespace> = Api::all(self.client.clone());
        let list = ns_api
            .list(&ListParams::default())
            .await
            .context("failed fetching namespaces")?;

        let names: Vec<String> = list
            .items
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

        let list = pods_api
            .list(&ListParams::default())
            .await
            .with_context(|| {
                if let Some(ns) = namespace {
                    format!("failed fetching pods in namespace '{ns}'")
                } else {
                    "failed fetching pods across all namespaces".to_string()
                }
            })?;

        let pods = list
            .into_iter()
            .map(|pod| {
                let container_statuses = pod
                    .status
                    .as_ref()
                    .and_then(|status| status.container_statuses.as_ref())
                    .cloned()
                    .unwrap_or_default();

                let waiting_reasons = container_statuses
                    .iter()
                    .filter_map(|status| status.state.as_ref())
                    .filter_map(|state| state.waiting.as_ref())
                    .filter_map(|waiting| waiting.reason.clone())
                    .collect::<Vec<_>>();

                let restarts = container_statuses.iter().map(|s| s.restart_count).sum();

                PodInfo {
                    name: pod.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: pod
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    status: pod
                        .status
                        .as_ref()
                        .and_then(|status| status.phase.clone())
                        .unwrap_or_else(|| "Unknown".to_string()),
                    node: pod.spec.as_ref().and_then(|spec| spec.node_name.clone()),
                    pod_ip: pod.status.as_ref().and_then(|status| status.pod_ip.clone()),
                    restarts,
                    created_at: pod.metadata.creation_timestamp.as_ref().map(|ts| ts.0),
                    labels: pod
                        .metadata
                        .labels
                        .unwrap_or_default()
                        .into_iter()
                        .collect(),
                    waiting_reasons,
                }
            })
            .collect();

        Ok(pods)
    }

    /// Fetches services from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_services(&self, namespace: Option<&str>) -> Result<Vec<ServiceInfo>> {
        let services_api: Api<Service> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = services_api
            .list(&ListParams::default())
            .await
            .with_context(|| {
                if let Some(ns) = namespace {
                    format!("failed fetching services in namespace '{ns}'")
                } else {
                    "failed fetching services across all namespaces".to_string()
                }
            })?;

        let now = Utc::now();
        let services = list
            .into_iter()
            .map(|svc| {
                let ports = svc
                    .spec
                    .as_ref()
                    .and_then(|spec| spec.ports.as_ref())
                    .map(|ports| {
                        ports
                            .iter()
                            .map(|p| {
                                format!(
                                    "{}/{}",
                                    p.port,
                                    p.protocol.clone().unwrap_or_else(|| "TCP".to_string())
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let service_type = svc
                    .spec
                    .as_ref()
                    .and_then(|spec| spec.type_.clone())
                    .unwrap_or_else(|| "ClusterIP".to_string());

                let created_at = svc.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

                ServiceInfo {
                    name: svc.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: svc
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    type_: service_type.clone(),
                    service_type,
                    cluster_ip: svc.spec.as_ref().and_then(|spec| spec.cluster_ip.clone()),
                    ports,
                    created_at,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                }
            })
            .collect();

        Ok(services)
    }

    /// Fetches deployments from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_deployments(&self, namespace: Option<&str>) -> Result<Vec<DeploymentInfo>> {
        let deployments_api: Api<Deployment> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = deployments_api
            .list(&ListParams::default())
            .await
            .with_context(|| {
                if let Some(ns) = namespace {
                    format!("failed fetching deployments in namespace '{ns}'")
                } else {
                    "failed fetching deployments across all namespaces".to_string()
                }
            })?;

        let now = Utc::now();
        let deployments = list
            .into_iter()
            .map(|dep| {
                let desired_replicas = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
                let ready_replicas = dep
                    .status
                    .as_ref()
                    .and_then(|s| s.ready_replicas)
                    .unwrap_or(0);
                let available_replicas = dep
                    .status
                    .as_ref()
                    .and_then(|s| s.available_replicas)
                    .unwrap_or(0);
                let updated_replicas = dep
                    .status
                    .as_ref()
                    .and_then(|s| s.updated_replicas)
                    .unwrap_or(0);

                let created_at = dep.metadata.creation_timestamp.as_ref().map(|ts| ts.0);
                let image = dep
                    .spec
                    .as_ref()
                    .and_then(|spec| spec.template.spec.as_ref())
                    .and_then(|spec| spec.containers.first())
                    .and_then(|container| container.image.clone());

                DeploymentInfo {
                    name: dep.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: dep
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    desired_replicas,
                    ready_replicas,
                    available_replicas,
                    updated_replicas,
                    created_at,
                    ready: format!("{ready_replicas}/{desired_replicas}"),
                    updated: updated_replicas,
                    available: available_replicas,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    image,
                }
            })
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

        let list = statefulsets_api
            .list(&ListParams::default())
            .await
            .with_context(|| {
                if let Some(ns) = namespace {
                    format!("failed fetching statefulsets in namespace '{ns}'")
                } else {
                    "failed fetching statefulsets across all namespaces".to_string()
                }
            })?;

        let now = Utc::now();
        let statefulsets = list
            .into_iter()
            .map(|ss| {
                let spec = ss.spec.as_ref();
                let status = ss.status.as_ref();
                let created_at = ss.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

                StatefulSetInfo {
                    name: ss.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: ss
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    desired_replicas: spec.and_then(|s| s.replicas).unwrap_or(1),
                    ready_replicas: status.and_then(|s| s.ready_replicas).unwrap_or(0),
                    service_name: spec
                        .map(|s| s.service_name.clone())
                        .unwrap_or_else(|| "<none>".to_string()),
                    pod_management_policy: spec
                        .and_then(|s| s.pod_management_policy.clone())
                        .unwrap_or_else(|| "OrderedReady".to_string()),
                    image: self
                        .extract_image_from_pod_spec(spec.and_then(|s| s.template.spec.as_ref())),
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect();

        Ok(statefulsets)
    }

    /// Fetches daemonsets from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_daemonsets(&self, namespace: Option<&str>) -> Result<Vec<DaemonSetInfo>> {
        let daemonsets_api: Api<DaemonSet> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = daemonsets_api
            .list(&ListParams::default())
            .await
            .with_context(|| {
                if let Some(ns) = namespace {
                    format!("failed fetching daemonsets in namespace '{ns}'")
                } else {
                    "failed fetching daemonsets across all namespaces".to_string()
                }
            })?;

        let now = Utc::now();
        let daemonsets = list
            .into_iter()
            .map(|ds| {
                let spec = ds.spec.as_ref();
                let status = ds.status.as_ref();
                let created_at = ds.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

                let desired_count = status.map(|s| s.desired_number_scheduled).unwrap_or(0);
                let ready_count = status.map(|s| s.number_ready).unwrap_or(0);
                let unavailable_count = status.and_then(|s| s.number_unavailable).unwrap_or(0);

                DaemonSetInfo {
                    name: ds.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: ds
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    desired_count,
                    ready_count,
                    unavailable_count,
                    selector: spec
                        .and_then(|s| s.selector.match_labels.as_ref())
                        .map(|labels| {
                            labels
                                .iter()
                                .map(|(k, v)| format!("{k}={v}"))
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_else(|| "<none>".to_string()),
                    update_strategy: spec
                        .and_then(|s| s.update_strategy.as_ref())
                        .and_then(|us| us.type_.clone())
                        .unwrap_or_else(|| "RollingUpdate".to_string()),
                    labels: ds
                        .metadata
                        .labels
                        .clone()
                        .unwrap_or_default()
                        .into_iter()
                        .collect(),
                    status_message: if unavailable_count == 0 {
                        "Ready".to_string()
                    } else {
                        format!("{unavailable_count} pods unavailable")
                    },
                    image: self
                        .extract_image_from_pod_spec(spec.and_then(|s| s.template.spec.as_ref())),
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect();

        Ok(daemonsets)
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

        let list = service_accounts_api
            .list(&ListParams::default())
            .await
            .with_context(|| {
                if let Some(ns) = namespace {
                    format!("failed fetching serviceaccounts in namespace '{ns}'")
                } else {
                    "failed fetching serviceaccounts across all namespaces".to_string()
                }
            })?;

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

        let list = roles_api
            .list(&ListParams::default())
            .await
            .with_context(|| {
                if let Some(ns) = namespace {
                    format!("failed fetching roles in namespace '{ns}'")
                } else {
                    "failed fetching roles across all namespaces".to_string()
                }
            })?;

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

        let list = role_bindings_api
            .list(&ListParams::default())
            .await
            .with_context(|| {
                if let Some(ns) = namespace {
                    format!("failed fetching rolebindings in namespace '{ns}'")
                } else {
                    "failed fetching rolebindings across all namespaces".to_string()
                }
            })?;

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

        let list = cluster_roles_api
            .list(&ListParams::default())
            .await
            .context("failed fetching clusterroles")?;

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

        let list = cluster_role_bindings_api
            .list(&ListParams::default())
            .await
            .context("failed fetching clusterrolebindings")?;

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

        let list = jobs_api
            .list(&ListParams::default())
            .await
            .with_context(|| {
                if let Some(ns) = namespace {
                    format!("failed fetching jobs in namespace '{ns}'")
                } else {
                    "failed fetching jobs across all namespaces".to_string()
                }
            })?;

        let now = Utc::now();
        let jobs = list
            .into_iter()
            .map(|job| {
                let spec = job.spec.as_ref();
                let status = job.status.as_ref();

                let succeeded = status.and_then(|s| s.succeeded).unwrap_or(0);
                let failed = status.and_then(|s| s.failed).unwrap_or(0);
                let active = status.and_then(|s| s.active).unwrap_or(0);
                let parallelism = spec.and_then(|s| s.parallelism).unwrap_or(1);
                let start_time = status.and_then(|s| s.start_time.as_ref()).map(|ts| ts.0);
                let completion_time = status
                    .and_then(|s| s.completion_time.as_ref())
                    .map(|ts| ts.0);
                let created_at = job.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

                JobInfo {
                    name: job.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: job
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    status: job_status_from_counts(succeeded, failed, active),
                    completions: format_job_completions(succeeded, parallelism),
                    duration: format_job_duration(start_time, completion_time),
                    parallelism,
                    active_pods: active,
                    failed_pods: failed,
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
            .collect();

        Ok(jobs)
    }

    /// Fetches cronjobs from a namespace or all namespaces when `namespace` is `None`.
    pub async fn fetch_cronjobs(&self, namespace: Option<&str>) -> Result<Vec<CronJobInfo>> {
        let cronjobs_api: Api<CronJob> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let list = cronjobs_api
            .list(&ListParams::default())
            .await
            .with_context(|| {
                if let Some(ns) = namespace {
                    format!("failed fetching cronjobs in namespace '{ns}'")
                } else {
                    "failed fetching cronjobs across all namespaces".to_string()
                }
            })?;

        let now = Utc::now();
        let cronjobs = list
            .into_iter()
            .map(|cj| {
                let spec = cj.spec.as_ref();
                let status = cj.status.as_ref();
                let created_at = cj.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

                CronJobInfo {
                    name: cj.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                    namespace: cj
                        .metadata
                        .namespace
                        .unwrap_or_else(|| "default".to_string()),
                    schedule: spec
                        .map(|s| s.schedule.clone())
                        .unwrap_or_else(|| "<none>".to_string()),
                    timezone: spec.and_then(|s| s.time_zone.clone()),
                    last_schedule_time: status
                        .and_then(|s| s.last_schedule_time.as_ref())
                        .map(|ts| ts.0),
                    next_schedule_time: None,
                    last_successful_time: status
                        .and_then(|s| s.last_successful_time.as_ref())
                        .map(|ts| ts.0),
                    suspend: spec.and_then(|s| s.suspend).unwrap_or(false),
                    active_jobs: status
                        .and_then(|s| s.active.as_ref())
                        .map(|v| v.len() as i32)
                        .unwrap_or(0),
                    age: created_at.and_then(|ts| (now - ts).to_std().ok()),
                    created_at,
                }
            })
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

        let list = api.list(&ListParams::default()).await.with_context(|| {
            if let Some(ns) = namespace {
                format!("failed fetching resource quotas in namespace '{ns}'")
            } else {
                "failed fetching resource quotas across all namespaces".to_string()
            }
        })?;

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

        let list = api.list(&ListParams::default()).await.with_context(|| {
            if let Some(ns) = namespace {
                format!("failed fetching limit ranges in namespace '{ns}'")
            } else {
                "failed fetching limit ranges across all namespaces".to_string()
            }
        })?;

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

        let list = api.list(&ListParams::default()).await.with_context(|| {
            if let Some(ns) = namespace {
                format!("failed fetching pod disruption budgets in namespace '{ns}'")
            } else {
                "failed fetching pod disruption budgets across all namespaces".to_string()
            }
        })?;

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

    /// Fetches CustomResourceDefinitions cluster-wide and includes instance counts.
    pub async fn fetch_custom_resource_definitions(
        &self,
    ) -> Result<Vec<CustomResourceDefinitionInfo>> {
        let crd_api: Api<CustomResourceDefinition> = Api::all(self.client.clone());
        let list = crd_api
            .list(&ListParams::default())
            .await
            .context("failed fetching custom resource definitions")?;

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

            let info = CustomResourceDefinitionInfo {
                name: crd.metadata.name.unwrap_or_else(|| "<unknown>".to_string()),
                group: spec.group.clone(),
                version,
                kind: spec.names.kind.clone(),
                plural: spec.names.plural.clone(),
                scope: spec.scope,
                instances: 0,
            };

            let instances = self
                .count_custom_resource_instances(&info)
                .await
                .unwrap_or(0);

            crds.push(CustomResourceDefinitionInfo { instances, ..info });
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

        let list = api
            .list(&ListParams::default())
            .await
            .with_context(|| format!("failed fetching custom resources for CRD '{}'", crd.name))?;

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
            Err(err) if is_metrics_api_unavailable(&err) => return Ok(None),
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
            Err(err) if is_metrics_api_unavailable(&err) => return Ok(None),
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed fetching node metrics for node '{name}'"));
            }
        };

        Ok(NodeMetricsInfo::from_json(name.to_string(), &obj.data))
    }

    async fn count_custom_resource_instances(
        &self,
        crd: &CustomResourceDefinitionInfo,
    ) -> Result<usize> {
        let ar = custom_resource_api_resource(crd);
        let api: Api<DynamicObject> = Api::all_with(self.client.clone(), &ar);
        let list = api.list(&ListParams::default()).await?;
        Ok(list.items.len())
    }

    /// Fetches cluster summary information.
    pub async fn fetch_cluster_info(&self) -> Result<ClusterInfo> {
        let nodes = self
            .fetch_nodes()
            .await
            .context("failed gathering node list for cluster summary")?;
        let pods = self
            .fetch_pods(None)
            .await
            .context("failed gathering pod list for cluster summary")?;

        let ready_nodes = nodes.iter().filter(|node| node.ready).count();
        let version = self
            .client
            .apiserver_version()
            .await
            .context("failed fetching API server version")?;

        Ok(ClusterInfo {
            context: self.cluster_context.clone(),
            server: self.cluster_url.clone(),
            git_version: Some(version.git_version),
            platform: Some(version.platform),
            node_count: nodes.len(),
            ready_nodes,
            pod_count: pods.len(),
        })
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

    /// Fetches pod events and degrades gracefully when RBAC denies access.
    pub async fn fetch_pod_events(&self, name: &str, namespace: &str) -> Result<Vec<EventInfo>> {
        events::fetch_pod_events(&self.client, name, namespace)
            .await
            .with_context(|| format!("failed preparing events for pod '{namespace}/{name}'"))
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

    fn extract_image_from_pod_spec(&self, pod_spec: Option<&PodSpec>) -> Option<String> {
        pod_spec
            .and_then(|spec| spec.containers.first())
            .and_then(|container| container.image.clone())
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
            .map(|ports| ports.iter().map(|p| p.container_port as u16).collect())
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

fn node_condition_true(node: &Node, condition_type: &str) -> bool {
    node.status
        .as_ref()
        .and_then(|status| status.conditions.as_ref())
        .and_then(|conditions| {
            conditions
                .iter()
                .find(|condition| condition.type_ == condition_type)
        })
        .is_some_and(|condition| condition.status == "True")
}

fn node_role(node: &Node) -> String {
    let labels = node.metadata.labels.as_ref();

    let is_control_plane = labels.is_some_and(|labels| {
        labels.contains_key("node-role.kubernetes.io/control-plane")
            || labels.contains_key("node-role.kubernetes.io/master")
    });

    if is_control_plane {
        "master".to_string()
    } else {
        "worker".to_string()
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

fn job_status_from_counts(succeeded: i32, failed: i32, active: i32) -> String {
    if succeeded > 0 && active == 0 {
        "Succeeded".to_string()
    } else if failed > 0 {
        "Failed".to_string()
    } else if active > 0 {
        "Running".to_string()
    } else {
        "Pending".to_string()
    }
}

fn format_job_completions(succeeded: i32, parallelism: i32) -> String {
    format!("{}/{}", succeeded.max(0), parallelism.max(1))
}

fn format_job_duration(
    start_time: Option<chrono::DateTime<Utc>>,
    completion_time: Option<chrono::DateTime<Utc>>,
) -> Option<String> {
    let start = start_time?;
    let end = completion_time.unwrap_or_else(Utc::now);
    let delta = end.signed_duration_since(start);

    let secs = delta.num_seconds().max(0);
    let mins = secs / 60;
    let rem_secs = secs % 60;

    if mins > 0 {
        Some(format!("{mins}m{rem_secs}s"))
    } else {
        Some(format!("{rem_secs}s"))
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::api::{
        core::v1::{NodeCondition, NodeStatus},
        rbac::v1::{PolicyRule, Subject},
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    use super::*;

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
}
