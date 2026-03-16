use serde::{Deserialize, Serialize};

use super::views::AppView;

/// Logical pointer to a resource selected in the current view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceRef {
    Node(String),
    Pod(String, String),
    Service(String, String),
    Deployment(String, String),
    StatefulSet(String, String),
    DaemonSet(String, String),
    ReplicaSet(String, String),
    ReplicationController(String, String),
    Job(String, String),
    CronJob(String, String),
    ResourceQuota(String, String),
    LimitRange(String, String),
    PodDisruptionBudget(String, String),
    Endpoint(String, String),
    Ingress(String, String),
    IngressClass(String),
    NetworkPolicy(String, String),
    ConfigMap(String, String),
    Secret(String, String),
    Hpa(String, String),
    PriorityClass(String),
    Pvc(String, String),
    Pv(String),
    StorageClass(String),
    Namespace(String),
    Event(String, String),
    ServiceAccount(String, String),
    Role(String, String),
    RoleBinding(String, String),
    ClusterRole(String),
    ClusterRoleBinding(String),
    HelmRelease(String, String),
    /// A custom resource instance identified by its CRD coordinates.
    /// Fields: (name, namespace_opt, group, version, kind, plural)
    CustomResource {
        name: String,
        namespace: Option<String>,
        group: String,
        version: String,
        kind: String,
        plural: String,
    },
}

impl ResourceRef {
    /// Returns resource kind label used by UI and fetch routing.
    pub fn kind(&self) -> &str {
        match self {
            ResourceRef::Node(_) => "Node",
            ResourceRef::Pod(_, _) => "Pod",
            ResourceRef::Service(_, _) => "Service",
            ResourceRef::Deployment(_, _) => "Deployment",
            ResourceRef::StatefulSet(_, _) => "StatefulSet",
            ResourceRef::DaemonSet(_, _) => "DaemonSet",
            ResourceRef::ReplicaSet(_, _) => "ReplicaSet",
            ResourceRef::ReplicationController(_, _) => "ReplicationController",
            ResourceRef::Job(_, _) => "Job",
            ResourceRef::CronJob(_, _) => "CronJob",
            ResourceRef::ResourceQuota(_, _) => "ResourceQuota",
            ResourceRef::LimitRange(_, _) => "LimitRange",
            ResourceRef::PodDisruptionBudget(_, _) => "PodDisruptionBudget",
            ResourceRef::Endpoint(_, _) => "Endpoints",
            ResourceRef::Ingress(_, _) => "Ingress",
            ResourceRef::IngressClass(_) => "IngressClass",
            ResourceRef::NetworkPolicy(_, _) => "NetworkPolicy",
            ResourceRef::ConfigMap(_, _) => "ConfigMap",
            ResourceRef::Secret(_, _) => "Secret",
            ResourceRef::Hpa(_, _) => "HorizontalPodAutoscaler",
            ResourceRef::PriorityClass(_) => "PriorityClass",
            ResourceRef::Pvc(_, _) => "PersistentVolumeClaim",
            ResourceRef::Pv(_) => "PersistentVolume",
            ResourceRef::StorageClass(_) => "StorageClass",
            ResourceRef::Namespace(_) => "Namespace",
            ResourceRef::Event(_, _) => "Event",
            ResourceRef::ServiceAccount(_, _) => "ServiceAccount",
            ResourceRef::Role(_, _) => "Role",
            ResourceRef::RoleBinding(_, _) => "RoleBinding",
            ResourceRef::ClusterRole(_) => "ClusterRole",
            ResourceRef::ClusterRoleBinding(_) => "ClusterRoleBinding",
            ResourceRef::HelmRelease(_, _) => "HelmRelease",
            ResourceRef::CustomResource { kind, .. } => kind.as_str(),
        }
    }

    /// Returns resource name.
    pub fn name(&self) -> &str {
        match self {
            ResourceRef::Node(name)
            | ResourceRef::Pod(name, _)
            | ResourceRef::Service(name, _)
            | ResourceRef::Deployment(name, _)
            | ResourceRef::StatefulSet(name, _)
            | ResourceRef::DaemonSet(name, _)
            | ResourceRef::ReplicaSet(name, _)
            | ResourceRef::ReplicationController(name, _)
            | ResourceRef::Job(name, _)
            | ResourceRef::CronJob(name, _)
            | ResourceRef::ResourceQuota(name, _)
            | ResourceRef::LimitRange(name, _)
            | ResourceRef::PodDisruptionBudget(name, _)
            | ResourceRef::Endpoint(name, _)
            | ResourceRef::Ingress(name, _)
            | ResourceRef::IngressClass(name)
            | ResourceRef::NetworkPolicy(name, _)
            | ResourceRef::ConfigMap(name, _)
            | ResourceRef::Secret(name, _)
            | ResourceRef::Hpa(name, _)
            | ResourceRef::PriorityClass(name)
            | ResourceRef::Pvc(name, _)
            | ResourceRef::Pv(name)
            | ResourceRef::StorageClass(name)
            | ResourceRef::Namespace(name)
            | ResourceRef::Event(name, _)
            | ResourceRef::ServiceAccount(name, _)
            | ResourceRef::Role(name, _)
            | ResourceRef::RoleBinding(name, _)
            | ResourceRef::ClusterRole(name)
            | ResourceRef::ClusterRoleBinding(name) => name,
            ResourceRef::HelmRelease(name, _) => name,
            ResourceRef::CustomResource { name, .. } => name,
        }
    }

    /// Returns namespace when this is a namespaced resource.
    pub fn namespace(&self) -> Option<&str> {
        match self {
            ResourceRef::Node(_)
            | ResourceRef::IngressClass(_)
            | ResourceRef::PriorityClass(_)
            | ResourceRef::Pv(_)
            | ResourceRef::StorageClass(_)
            | ResourceRef::Namespace(_)
            | ResourceRef::ClusterRole(_)
            | ResourceRef::ClusterRoleBinding(_) => None,
            ResourceRef::Pod(_, ns)
            | ResourceRef::Service(_, ns)
            | ResourceRef::Deployment(_, ns)
            | ResourceRef::StatefulSet(_, ns)
            | ResourceRef::DaemonSet(_, ns)
            | ResourceRef::ReplicaSet(_, ns)
            | ResourceRef::ReplicationController(_, ns)
            | ResourceRef::Job(_, ns)
            | ResourceRef::CronJob(_, ns)
            | ResourceRef::ResourceQuota(_, ns)
            | ResourceRef::LimitRange(_, ns)
            | ResourceRef::PodDisruptionBudget(_, ns)
            | ResourceRef::Endpoint(_, ns)
            | ResourceRef::Ingress(_, ns)
            | ResourceRef::NetworkPolicy(_, ns)
            | ResourceRef::ConfigMap(_, ns)
            | ResourceRef::Secret(_, ns)
            | ResourceRef::Hpa(_, ns)
            | ResourceRef::Pvc(_, ns)
            | ResourceRef::Event(_, ns)
            | ResourceRef::ServiceAccount(_, ns)
            | ResourceRef::Role(_, ns)
            | ResourceRef::RoleBinding(_, ns) => Some(ns),
            ResourceRef::HelmRelease(_, ns) => Some(ns),
            ResourceRef::CustomResource { namespace, .. } => namespace.as_deref(),
        }
    }

    pub fn primary_view(&self) -> Option<AppView> {
        match self {
            ResourceRef::Node(_) => Some(AppView::Nodes),
            ResourceRef::Pod(_, _) => Some(AppView::Pods),
            ResourceRef::Service(_, _) => Some(AppView::Services),
            ResourceRef::Deployment(_, _) => Some(AppView::Deployments),
            ResourceRef::StatefulSet(_, _) => Some(AppView::StatefulSets),
            ResourceRef::DaemonSet(_, _) => Some(AppView::DaemonSets),
            ResourceRef::ReplicaSet(_, _) => Some(AppView::ReplicaSets),
            ResourceRef::ReplicationController(_, _) => Some(AppView::ReplicationControllers),
            ResourceRef::Job(_, _) => Some(AppView::Jobs),
            ResourceRef::CronJob(_, _) => Some(AppView::CronJobs),
            ResourceRef::ResourceQuota(_, _) => Some(AppView::ResourceQuotas),
            ResourceRef::LimitRange(_, _) => Some(AppView::LimitRanges),
            ResourceRef::PodDisruptionBudget(_, _) => Some(AppView::PodDisruptionBudgets),
            ResourceRef::Endpoint(_, _) => Some(AppView::Endpoints),
            ResourceRef::Ingress(_, _) => Some(AppView::Ingresses),
            ResourceRef::IngressClass(_) => Some(AppView::IngressClasses),
            ResourceRef::NetworkPolicy(_, _) => Some(AppView::NetworkPolicies),
            ResourceRef::ConfigMap(_, _) => Some(AppView::ConfigMaps),
            ResourceRef::Secret(_, _) => Some(AppView::Secrets),
            ResourceRef::Hpa(_, _) => Some(AppView::HPAs),
            ResourceRef::PriorityClass(_) => Some(AppView::PriorityClasses),
            ResourceRef::Pvc(_, _) => Some(AppView::PersistentVolumeClaims),
            ResourceRef::Pv(_) => Some(AppView::PersistentVolumes),
            ResourceRef::StorageClass(_) => Some(AppView::StorageClasses),
            ResourceRef::Namespace(_) => Some(AppView::Namespaces),
            ResourceRef::Event(_, _) => Some(AppView::Events),
            ResourceRef::ServiceAccount(_, _) => Some(AppView::ServiceAccounts),
            ResourceRef::Role(_, _) => Some(AppView::Roles),
            ResourceRef::RoleBinding(_, _) => Some(AppView::RoleBindings),
            ResourceRef::ClusterRole(_) => Some(AppView::ClusterRoles),
            ResourceRef::ClusterRoleBinding(_) => Some(AppView::ClusterRoleBindings),
            ResourceRef::HelmRelease(_, _) => Some(AppView::HelmReleases),
            ResourceRef::CustomResource { group, .. } if group.ends_with(".fluxcd.io") => {
                Some(AppView::FluxCDAll)
            }
            ResourceRef::CustomResource { .. } => None,
        }
    }
}
