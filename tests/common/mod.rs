//! Shared test helpers for integration tests.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use kubectui::{
    k8s::dtos::{
        ClusterInfo, ClusterRoleBindingInfo, ClusterRoleInfo, ConfigMapInfo, CronJobInfo,
        CustomResourceDefinitionInfo, DaemonSetInfo, DeploymentInfo, EndpointInfo, HelmReleaseInfo,
        HpaInfo, IngressClassInfo, IngressInfo, JobInfo, K8sEventInfo, LimitRangeInfo,
        NamespaceInfo, NetworkPolicyInfo, NodeInfo, NodeMetricsInfo, PodDisruptionBudgetInfo,
        PodInfo, PriorityClassInfo, PvInfo, PvcInfo, ReplicaSetInfo, ReplicationControllerInfo,
        ResourceQuotaInfo, RoleBindingInfo, RoleInfo, SecretInfo, ServiceAccountInfo, ServiceInfo,
        StatefulSetInfo, StorageClassInfo,
    },
    state::ClusterDataSource,
};

pub fn make_node(name: &str, ready: bool, role: &str) -> NodeInfo {
    NodeInfo {
        name: name.to_string(),
        ready,
        role: role.to_string(),
        ..NodeInfo::default()
    }
}

pub fn make_service(name: &str, namespace: &str, type_: &str) -> ServiceInfo {
    ServiceInfo {
        name: name.to_string(),
        namespace: namespace.to_string(),
        type_: type_.to_string(),
        service_type: type_.to_string(),
        ports: vec!["80/TCP".to_string()],
        ..ServiceInfo::default()
    }
}

#[allow(dead_code)]
pub fn make_deployment(name: &str, namespace: &str, ready: &str) -> DeploymentInfo {
    DeploymentInfo {
        name: name.to_string(),
        namespace: namespace.to_string(),
        ready: ready.to_string(),
        ..DeploymentInfo::default()
    }
}

#[allow(dead_code)]
pub fn make_pod(name: &str, namespace: &str, status: &str) -> PodInfo {
    PodInfo {
        name: name.to_string(),
        namespace: namespace.to_string(),
        status: status.to_string(),
        ..PodInfo::default()
    }
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct MockDataSource {
    pub url: String,
    pub nodes: Vec<NodeInfo>,
    pub namespaces: Vec<String>,
    pub pods: Vec<PodInfo>,
    pub services: Vec<ServiceInfo>,
    pub deployments: Vec<DeploymentInfo>,
    pub statefulsets: Vec<StatefulSetInfo>,
    pub daemonsets: Vec<DaemonSetInfo>,
    pub replicasets: Vec<ReplicaSetInfo>,
    pub replication_controllers: Vec<ReplicationControllerInfo>,
    pub jobs: Vec<JobInfo>,
    pub cronjobs: Vec<CronJobInfo>,
    pub resource_quotas: Vec<ResourceQuotaInfo>,
    pub limit_ranges: Vec<LimitRangeInfo>,
    pub pod_disruption_budgets: Vec<PodDisruptionBudgetInfo>,
    pub service_accounts: Vec<ServiceAccountInfo>,
    pub roles: Vec<RoleInfo>,
    pub role_bindings: Vec<RoleBindingInfo>,
    pub cluster_roles: Vec<ClusterRoleInfo>,
    pub cluster_role_bindings: Vec<ClusterRoleBindingInfo>,
    pub custom_resource_definitions: Vec<CustomResourceDefinitionInfo>,
    pub fail: bool,
    pub calls: Arc<AtomicUsize>,
}

impl Default for MockDataSource {
    fn default() -> Self {
        Self {
            url: "https://mock.cluster".to_string(),
            nodes: vec![make_node("n1", true, "worker")],
            namespaces: vec!["default".to_string(), "kube-system".to_string()],
            pods: vec![make_pod("p1", "default", "Running")],
            services: vec![make_service("svc1", "default", "ClusterIP")],
            deployments: vec![make_deployment("dep1", "default", "1/1")],
            statefulsets: vec![StatefulSetInfo {
                name: "db".to_string(),
                namespace: "default".to_string(),
                desired_replicas: 1,
                ready_replicas: 1,
                service_name: "db-headless".to_string(),
                pod_management_policy: "OrderedReady".to_string(),
                ..StatefulSetInfo::default()
            }],
            daemonsets: vec![DaemonSetInfo {
                name: "agent".to_string(),
                namespace: "kube-system".to_string(),
                desired_count: 1,
                ready_count: 1,
                ..DaemonSetInfo::default()
            }],
            replicasets: vec![],
            replication_controllers: vec![],
            jobs: vec![JobInfo {
                name: "job1".to_string(),
                namespace: "default".to_string(),
                status: "Running".to_string(),
                ..JobInfo::default()
            }],
            cronjobs: vec![CronJobInfo {
                name: "cron1".to_string(),
                namespace: "default".to_string(),
                schedule: "*/5 * * * *".to_string(),
                ..CronJobInfo::default()
            }],
            resource_quotas: vec![],
            limit_ranges: vec![],
            pod_disruption_budgets: vec![],
            service_accounts: vec![],
            roles: vec![],
            role_bindings: vec![],
            cluster_roles: vec![],
            cluster_role_bindings: vec![],
            custom_resource_definitions: vec![],
            fail: false,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl ClusterDataSource for MockDataSource {
    fn cluster_url(&self) -> &str {
        &self.url
    }

    async fn fetch_nodes(&self) -> Result<Vec<NodeInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock nodes error"));
        }
        Ok(self.nodes.clone())
    }

    async fn fetch_namespaces(&self) -> Result<Vec<String>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock namespaces error"));
        }
        Ok(self.namespaces.clone())
    }

    async fn fetch_pods(&self, _namespace: Option<&str>) -> Result<Vec<PodInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock pods error"));
        }
        Ok(self.pods.clone())
    }

    async fn fetch_services(&self, _namespace: Option<&str>) -> Result<Vec<ServiceInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock services error"));
        }
        Ok(self.services.clone())
    }

    async fn fetch_deployments(&self, _namespace: Option<&str>) -> Result<Vec<DeploymentInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock deployments error"));
        }
        Ok(self.deployments.clone())
    }

    async fn fetch_statefulsets(&self, _namespace: Option<&str>) -> Result<Vec<StatefulSetInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock statefulsets error"));
        }
        Ok(self.statefulsets.clone())
    }

    async fn fetch_daemonsets(&self, _namespace: Option<&str>) -> Result<Vec<DaemonSetInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock daemonsets error"));
        }
        Ok(self.daemonsets.clone())
    }

    async fn fetch_replicasets(&self, _namespace: Option<&str>) -> Result<Vec<ReplicaSetInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock replicasets error"));
        }
        Ok(self.replicasets.clone())
    }

    async fn fetch_replication_controllers(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<ReplicationControllerInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock replicationcontrollers error"));
        }
        Ok(self.replication_controllers.clone())
    }

    async fn fetch_jobs(&self, _namespace: Option<&str>) -> Result<Vec<JobInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock jobs error"));
        }
        Ok(self.jobs.clone())
    }

    async fn fetch_cronjobs(&self, _namespace: Option<&str>) -> Result<Vec<CronJobInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock cronjobs error"));
        }
        Ok(self.cronjobs.clone())
    }

    async fn fetch_resource_quotas(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<ResourceQuotaInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock resourcequotas error"));
        }
        Ok(self.resource_quotas.clone())
    }

    async fn fetch_limit_ranges(&self, _namespace: Option<&str>) -> Result<Vec<LimitRangeInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock limitranges error"));
        }
        Ok(self.limit_ranges.clone())
    }

    async fn fetch_pod_disruption_budgets(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<PodDisruptionBudgetInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock pdb error"));
        }
        Ok(self.pod_disruption_budgets.clone())
    }

    async fn fetch_service_accounts(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<ServiceAccountInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock serviceaccounts error"));
        }
        Ok(self.service_accounts.clone())
    }

    async fn fetch_roles(&self, _namespace: Option<&str>) -> Result<Vec<RoleInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock roles error"));
        }
        Ok(self.roles.clone())
    }

    async fn fetch_role_bindings(&self, _namespace: Option<&str>) -> Result<Vec<RoleBindingInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock rolebindings error"));
        }
        Ok(self.role_bindings.clone())
    }

    async fn fetch_cluster_roles(&self) -> Result<Vec<ClusterRoleInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock clusterroles error"));
        }
        Ok(self.cluster_roles.clone())
    }

    async fn fetch_cluster_role_bindings(&self) -> Result<Vec<ClusterRoleBindingInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock clusterrolebindings error"));
        }
        Ok(self.cluster_role_bindings.clone())
    }

    async fn fetch_custom_resource_definitions(&self) -> Result<Vec<CustomResourceDefinitionInfo>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock crds error"));
        }
        Ok(self.custom_resource_definitions.clone())
    }

    async fn fetch_cluster_info(&self) -> Result<ClusterInfo> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(anyhow!("mock cluster info error"));
        }
        Ok(ClusterInfo {
            context: Some("mock".to_string()),
            server: self.url.clone(),
            git_version: Some("v1.30.0".to_string()),
            platform: Some("linux/amd64".to_string()),
            node_count: self.nodes.len(),
            ready_nodes: self.nodes.iter().filter(|n| n.ready).count(),
            pod_count: self.pods.len(),
        })
    }

    async fn fetch_endpoints(&self, _namespace: Option<&str>) -> Result<Vec<EndpointInfo>> {
        Ok(vec![])
    }
    async fn fetch_ingresses(&self, _namespace: Option<&str>) -> Result<Vec<IngressInfo>> {
        Ok(vec![])
    }
    async fn fetch_ingress_classes(&self) -> Result<Vec<IngressClassInfo>> {
        Ok(vec![])
    }
    async fn fetch_network_policies(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<NetworkPolicyInfo>> {
        Ok(vec![])
    }
    async fn fetch_config_maps(&self, _namespace: Option<&str>) -> Result<Vec<ConfigMapInfo>> {
        Ok(vec![])
    }
    async fn fetch_secrets(&self, _namespace: Option<&str>) -> Result<Vec<SecretInfo>> {
        Ok(vec![])
    }
    async fn fetch_hpas(&self, _namespace: Option<&str>) -> Result<Vec<HpaInfo>> {
        Ok(vec![])
    }
    async fn fetch_pvcs(&self, _namespace: Option<&str>) -> Result<Vec<PvcInfo>> {
        Ok(vec![])
    }
    async fn fetch_pvs(&self) -> Result<Vec<PvInfo>> {
        Ok(vec![])
    }
    async fn fetch_storage_classes(&self) -> Result<Vec<StorageClassInfo>> {
        Ok(vec![])
    }
    async fn fetch_namespace_list(&self) -> Result<Vec<NamespaceInfo>> {
        Ok(vec![])
    }
    async fn fetch_events(&self, _namespace: Option<&str>) -> Result<Vec<K8sEventInfo>> {
        Ok(vec![])
    }
    async fn fetch_priority_classes(&self) -> Result<Vec<PriorityClassInfo>> {
        Ok(vec![])
    }
    async fn fetch_helm_releases(&self, _namespace: Option<&str>) -> Result<Vec<HelmReleaseInfo>> {
        Ok(vec![])
    }
    async fn fetch_all_node_metrics(&self) -> Result<Vec<NodeMetricsInfo>> {
        Ok(vec![])
    }
}
