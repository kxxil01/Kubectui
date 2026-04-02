//! Canonical RBAC action mapping for detail-level resource actions.

use std::collections::BTreeMap;

use crate::{app::ResourceRef, policy::DetailAction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailActionAuthorization {
    Allowed,
    Denied,
    Unknown,
}

pub type ActionAuthorizationMap = BTreeMap<DetailAction, DetailActionAuthorization>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionAccessReview {
    pub action: DetailAction,
    pub authorization: Option<DetailActionAuthorization>,
    pub strict: bool,
    pub checks: Vec<ResourceAccessCheck>,
}

impl DetailActionAuthorization {
    pub const fn from_allowed(allowed: Option<bool>) -> Self {
        match allowed {
            Some(true) => Self::Allowed,
            Some(false) => Self::Denied,
            None => Self::Unknown,
        }
    }

    pub const fn permits(self, action: DetailAction) -> bool {
        match self {
            Self::Allowed => true,
            Self::Denied => false,
            Self::Unknown => !detail_action_requires_strict_authorization(action),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceAccessCheck {
    pub group: Option<String>,
    pub resource: String,
    pub subresource: Option<String>,
    pub verb: String,
    pub namespace: Option<String>,
    pub name: Option<String>,
}

impl ResourceAccessCheck {
    pub fn resource(
        verb: &str,
        group: Option<&str>,
        resource: &str,
        namespace: Option<&str>,
        name: Option<&str>,
    ) -> Self {
        Self {
            group: group.map(str::to_string),
            resource: resource.to_string(),
            subresource: None,
            verb: verb.to_string(),
            namespace: namespace.map(str::to_string),
            name: name.map(str::to_string),
        }
    }

    pub fn subresource(
        verb: &str,
        group: Option<&str>,
        resource: &str,
        subresource: &str,
        namespace: Option<&str>,
        name: Option<&str>,
    ) -> Self {
        Self {
            group: group.map(str::to_string),
            resource: resource.to_string(),
            subresource: Some(subresource.to_string()),
            verb: verb.to_string(),
            namespace: namespace.map(str::to_string),
            name: name.map(str::to_string),
        }
    }
}

#[derive(Debug, Clone)]
struct ResourceAccessTarget {
    group: Option<String>,
    resource: String,
    namespace: Option<String>,
    name: String,
}

pub const fn detail_action_requires_authorization(action: DetailAction) -> bool {
    matches!(
        action,
        DetailAction::ViewYaml
            | DetailAction::ViewConfigDrift
            | DetailAction::ViewDecodedSecret
            | DetailAction::ViewEvents
            | DetailAction::Logs
            | DetailAction::Exec
            | DetailAction::DebugContainer
            | DetailAction::PortForward
            | DetailAction::Probes
            | DetailAction::Scale
            | DetailAction::Restart
            | DetailAction::FluxReconcile
            | DetailAction::EditYaml
            | DetailAction::Delete
            | DetailAction::Trigger
            | DetailAction::SuspendCronJob
            | DetailAction::ResumeCronJob
            | DetailAction::Cordon
            | DetailAction::Uncordon
            | DetailAction::Drain
    )
}

pub const fn detail_action_requires_strict_authorization(action: DetailAction) -> bool {
    matches!(
        action,
        DetailAction::ViewDecodedSecret
            | DetailAction::Exec
            | DetailAction::DebugContainer
            | DetailAction::PortForward
            | DetailAction::Scale
            | DetailAction::Restart
            | DetailAction::FluxReconcile
            | DetailAction::EditYaml
            | DetailAction::Delete
            | DetailAction::Trigger
            | DetailAction::SuspendCronJob
            | DetailAction::ResumeCronJob
            | DetailAction::Cordon
            | DetailAction::Uncordon
            | DetailAction::Drain
    )
}

impl ResourceRef {
    pub fn authorization_checks(&self, action: DetailAction) -> Vec<ResourceAccessCheck> {
        match action {
            DetailAction::ViewYaml | DetailAction::ViewConfigDrift => match self {
                ResourceRef::HelmRelease(_, namespace) => vec![ResourceAccessCheck::resource(
                    "list",
                    None,
                    "secrets",
                    Some(namespace),
                    None,
                )],
                _ => self.base_access_checks("get"),
            },
            DetailAction::ViewAccessReview => Vec::new(),
            DetailAction::ViewRollout
            | DetailAction::ViewHelmHistory
            | DetailAction::ViewTrafficDebug
            | DetailAction::NodeDebugShell => Vec::new(),
            DetailAction::ViewDecodedSecret => self.base_access_checks("get"),
            DetailAction::ViewEvents => {
                if !self.supports_events_tab() {
                    Vec::new()
                } else {
                    self.namespace()
                        .map(|namespace| {
                            vec![ResourceAccessCheck::resource(
                                "list",
                                None,
                                "events",
                                Some(namespace),
                                None,
                            )]
                        })
                        .unwrap_or_default()
                }
            }
            DetailAction::Logs => match self {
                ResourceRef::Pod(name, namespace) => vec![
                    ResourceAccessCheck::resource("get", None, "pods", Some(namespace), Some(name)),
                    ResourceAccessCheck::subresource(
                        "get",
                        None,
                        "pods",
                        "log",
                        Some(namespace),
                        Some(name),
                    ),
                ],
                ResourceRef::Deployment(_, namespace)
                | ResourceRef::StatefulSet(_, namespace)
                | ResourceRef::DaemonSet(_, namespace)
                | ResourceRef::ReplicaSet(_, namespace)
                | ResourceRef::ReplicationController(_, namespace)
                | ResourceRef::Job(_, namespace) => {
                    let mut checks = self.base_access_checks("get");
                    checks.push(ResourceAccessCheck::resource(
                        "list",
                        None,
                        "pods",
                        Some(namespace),
                        None,
                    ));
                    checks.push(ResourceAccessCheck::subresource(
                        "get",
                        None,
                        "pods",
                        "log",
                        Some(namespace),
                        None,
                    ));
                    checks
                }
                _ => Vec::new(),
            },
            DetailAction::Exec => match self {
                ResourceRef::Pod(name, namespace) => vec![
                    ResourceAccessCheck::resource("get", None, "pods", Some(namespace), Some(name)),
                    ResourceAccessCheck::subresource(
                        "create",
                        None,
                        "pods",
                        "exec",
                        Some(namespace),
                        Some(name),
                    ),
                ],
                _ => Vec::new(),
            },
            DetailAction::DebugContainer => match self {
                ResourceRef::Pod(name, namespace) => vec![
                    ResourceAccessCheck::resource("get", None, "pods", Some(namespace), Some(name)),
                    ResourceAccessCheck::subresource(
                        "patch",
                        None,
                        "pods",
                        "ephemeralcontainers",
                        Some(namespace),
                        Some(name),
                    ),
                    ResourceAccessCheck::subresource(
                        "create",
                        None,
                        "pods",
                        "exec",
                        Some(namespace),
                        Some(name),
                    ),
                ],
                _ => Vec::new(),
            },
            DetailAction::PortForward => match self {
                ResourceRef::Pod(name, namespace) => vec![
                    ResourceAccessCheck::resource("get", None, "pods", Some(namespace), Some(name)),
                    ResourceAccessCheck::subresource(
                        "create",
                        None,
                        "pods",
                        "portforward",
                        Some(namespace),
                        Some(name),
                    ),
                ],
                _ => Vec::new(),
            },
            DetailAction::Probes => match self {
                ResourceRef::Pod(_, _) => self.base_access_checks("get"),
                _ => Vec::new(),
            },
            DetailAction::Scale => match self {
                ResourceRef::Deployment(_, _) | ResourceRef::StatefulSet(_, _) => {
                    let mut checks = self.base_access_checks("get");
                    checks.extend(self.base_access_checks("patch"));
                    checks
                }
                _ => Vec::new(),
            },
            DetailAction::Restart => match self {
                ResourceRef::Deployment(_, _)
                | ResourceRef::StatefulSet(_, _)
                | ResourceRef::DaemonSet(_, _) => self.base_access_checks("patch"),
                _ => Vec::new(),
            },
            DetailAction::FluxReconcile => match self {
                ResourceRef::CustomResource { .. } => self.base_access_checks("patch"),
                _ => Vec::new(),
            },
            DetailAction::EditYaml => {
                let mut checks = self.authorization_checks(DetailAction::ViewYaml);
                checks.extend(self.base_access_checks("patch"));
                checks
            }
            DetailAction::Delete => self.base_access_checks("delete"),
            DetailAction::Trigger => match self {
                ResourceRef::CronJob(_, namespace) => {
                    let mut checks = self.base_access_checks("get");
                    checks.push(ResourceAccessCheck::resource(
                        "create",
                        Some("batch"),
                        "jobs",
                        Some(namespace),
                        None,
                    ));
                    checks
                }
                _ => Vec::new(),
            },
            DetailAction::SuspendCronJob | DetailAction::ResumeCronJob => match self {
                ResourceRef::CronJob(_, _) => self.base_access_checks("patch"),
                _ => Vec::new(),
            },
            DetailAction::Cordon | DetailAction::Uncordon => match self {
                ResourceRef::Node(_) => self.base_access_checks("patch"),
                _ => Vec::new(),
            },
            DetailAction::Drain => match self {
                ResourceRef::Node(_) => vec![
                    self.base_resource_check("patch"),
                    ResourceAccessCheck::resource("list", None, "pods", None, None),
                    ResourceAccessCheck::subresource(
                        "create", None, "pods", "eviction", None, None,
                    ),
                ],
                _ => Vec::new(),
            },
            DetailAction::ToggleBookmark
            | DetailAction::ViewNetworkPolicies
            | DetailAction::CheckNetworkConnectivity
            | DetailAction::ViewRelationships => Vec::new(),
        }
    }

    fn base_access_checks(&self, verb: &str) -> Vec<ResourceAccessCheck> {
        self.base_access_target()
            .map(|target| {
                vec![ResourceAccessCheck::resource(
                    verb,
                    target.group.as_deref(),
                    &target.resource,
                    target.namespace.as_deref(),
                    Some(&target.name),
                )]
            })
            .unwrap_or_default()
    }

    fn base_resource_check(&self, verb: &str) -> ResourceAccessCheck {
        let target = self
            .base_access_target()
            .expect("base resource check requires a concrete Kubernetes resource");
        ResourceAccessCheck::resource(
            verb,
            target.group.as_deref(),
            &target.resource,
            target.namespace.as_deref(),
            Some(&target.name),
        )
    }

    fn base_access_target(&self) -> Option<ResourceAccessTarget> {
        let target = match self {
            ResourceRef::Node(name) => (None, "nodes", None, name),
            ResourceRef::Pod(name, namespace) => (None, "pods", Some(namespace.as_str()), name),
            ResourceRef::Service(name, namespace) => {
                (None, "services", Some(namespace.as_str()), name)
            }
            ResourceRef::Deployment(name, namespace) => {
                (Some("apps"), "deployments", Some(namespace.as_str()), name)
            }
            ResourceRef::StatefulSet(name, namespace) => {
                (Some("apps"), "statefulsets", Some(namespace.as_str()), name)
            }
            ResourceRef::DaemonSet(name, namespace) => {
                (Some("apps"), "daemonsets", Some(namespace.as_str()), name)
            }
            ResourceRef::ReplicaSet(name, namespace) => {
                (Some("apps"), "replicasets", Some(namespace.as_str()), name)
            }
            ResourceRef::ReplicationController(name, namespace) => (
                None,
                "replicationcontrollers",
                Some(namespace.as_str()),
                name,
            ),
            ResourceRef::Job(name, namespace) => {
                (Some("batch"), "jobs", Some(namespace.as_str()), name)
            }
            ResourceRef::CronJob(name, namespace) => {
                (Some("batch"), "cronjobs", Some(namespace.as_str()), name)
            }
            ResourceRef::ResourceQuota(name, namespace) => {
                (None, "resourcequotas", Some(namespace.as_str()), name)
            }
            ResourceRef::LimitRange(name, namespace) => {
                (None, "limitranges", Some(namespace.as_str()), name)
            }
            ResourceRef::PodDisruptionBudget(name, namespace) => (
                Some("policy"),
                "poddisruptionbudgets",
                Some(namespace.as_str()),
                name,
            ),
            ResourceRef::Endpoint(name, namespace) => {
                (None, "endpoints", Some(namespace.as_str()), name)
            }
            ResourceRef::Ingress(name, namespace) => (
                Some("networking.k8s.io"),
                "ingresses",
                Some(namespace.as_str()),
                name,
            ),
            ResourceRef::IngressClass(name) => {
                (Some("networking.k8s.io"), "ingressclasses", None, name)
            }
            ResourceRef::NetworkPolicy(name, namespace) => (
                Some("networking.k8s.io"),
                "networkpolicies",
                Some(namespace.as_str()),
                name,
            ),
            ResourceRef::ConfigMap(name, namespace) => {
                (None, "configmaps", Some(namespace.as_str()), name)
            }
            ResourceRef::Secret(name, namespace) => {
                (None, "secrets", Some(namespace.as_str()), name)
            }
            ResourceRef::Hpa(name, namespace) => (
                Some("autoscaling"),
                "horizontalpodautoscalers",
                Some(namespace.as_str()),
                name,
            ),
            ResourceRef::PriorityClass(name) => {
                (Some("scheduling.k8s.io"), "priorityclasses", None, name)
            }
            ResourceRef::Pvc(name, namespace) => (
                None,
                "persistentvolumeclaims",
                Some(namespace.as_str()),
                name,
            ),
            ResourceRef::Pv(name) => (None, "persistentvolumes", None, name),
            ResourceRef::StorageClass(name) => {
                (Some("storage.k8s.io"), "storageclasses", None, name)
            }
            ResourceRef::Namespace(name) => (None, "namespaces", None, name),
            ResourceRef::Event(name, namespace) => (None, "events", Some(namespace.as_str()), name),
            ResourceRef::ServiceAccount(name, namespace) => {
                (None, "serviceaccounts", Some(namespace.as_str()), name)
            }
            ResourceRef::Role(name, namespace) => (
                Some("rbac.authorization.k8s.io"),
                "roles",
                Some(namespace.as_str()),
                name,
            ),
            ResourceRef::RoleBinding(name, namespace) => (
                Some("rbac.authorization.k8s.io"),
                "rolebindings",
                Some(namespace.as_str()),
                name,
            ),
            ResourceRef::ClusterRole(name) => (
                Some("rbac.authorization.k8s.io"),
                "clusterroles",
                None,
                name,
            ),
            ResourceRef::ClusterRoleBinding(name) => (
                Some("rbac.authorization.k8s.io"),
                "clusterrolebindings",
                None,
                name,
            ),
            ResourceRef::HelmRelease(_, _) => return None,
            ResourceRef::CustomResource {
                name,
                namespace,
                group,
                plural,
                ..
            } => (
                Some(group.as_str()),
                plural.as_str(),
                namespace.as_deref(),
                name,
            ),
        };

        Some(ResourceAccessTarget {
            group: target.0.map(str::to_string),
            resource: target.1.to_string(),
            namespace: target.2.map(str::to_string),
            name: target.3.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── from_allowed round-trip ──────────────────────────────────────

    #[test]
    fn from_allowed_maps_all_three_states() {
        assert_eq!(
            DetailActionAuthorization::from_allowed(Some(true)),
            DetailActionAuthorization::Allowed
        );
        assert_eq!(
            DetailActionAuthorization::from_allowed(Some(false)),
            DetailActionAuthorization::Denied
        );
        assert_eq!(
            DetailActionAuthorization::from_allowed(None),
            DetailActionAuthorization::Unknown
        );
    }

    // ── permits() exhaustive edge cases ──────────────────────────────

    #[test]
    fn allowed_permits_every_action() {
        for action in DetailAction::ORDER {
            assert!(
                DetailActionAuthorization::Allowed.permits(action),
                "Allowed should permit {:?}",
                action
            );
        }
    }

    #[test]
    fn denied_blocks_every_action() {
        for action in DetailAction::ORDER {
            assert!(
                !DetailActionAuthorization::Denied.permits(action),
                "Denied should block {:?}",
                action
            );
        }
    }

    #[test]
    fn unknown_blocks_all_strict_actions() {
        let strict_actions = [
            DetailAction::ViewDecodedSecret,
            DetailAction::Exec,
            DetailAction::DebugContainer,
            DetailAction::PortForward,
            DetailAction::Scale,
            DetailAction::Restart,
            DetailAction::FluxReconcile,
            DetailAction::EditYaml,
            DetailAction::Delete,
            DetailAction::Trigger,
            DetailAction::SuspendCronJob,
            DetailAction::ResumeCronJob,
            DetailAction::Cordon,
            DetailAction::Uncordon,
            DetailAction::Drain,
        ];
        for action in strict_actions {
            assert!(
                !DetailActionAuthorization::Unknown.permits(action),
                "Unknown should block strict action {:?}",
                action
            );
        }
    }

    #[test]
    fn unknown_permits_soft_read_actions() {
        let soft_actions = [
            DetailAction::ViewYaml,
            DetailAction::ViewConfigDrift,
            DetailAction::ViewEvents,
            DetailAction::Logs,
            DetailAction::Probes,
        ];
        for action in soft_actions {
            assert!(
                DetailActionAuthorization::Unknown.permits(action),
                "Unknown should permit soft action {:?}",
                action
            );
        }
    }

    #[test]
    fn unknown_permits_non_auth_actions() {
        assert!(DetailActionAuthorization::Unknown.permits(DetailAction::ToggleBookmark));
        assert!(DetailActionAuthorization::Unknown.permits(DetailAction::ViewRelationships));
    }

    // ── strict vs requires_authorization consistency ─────────────────

    #[test]
    fn strict_is_subset_of_requires_authorization() {
        for action in DetailAction::ORDER {
            if detail_action_requires_strict_authorization(action) {
                assert!(
                    detail_action_requires_authorization(action),
                    "{:?} is strict but not in requires_authorization",
                    action
                );
            }
        }
    }

    // ── authorization_checks edge cases ──────────────────────────────

    #[test]
    fn exec_on_non_pod_returns_empty_checks() {
        let deploy = ResourceRef::Deployment("api".into(), "default".into());
        assert!(deploy.authorization_checks(DetailAction::Exec).is_empty());
    }

    #[test]
    fn port_forward_on_non_pod_returns_empty_checks() {
        let node = ResourceRef::Node("node-0".into());
        assert!(
            node.authorization_checks(DetailAction::PortForward)
                .is_empty()
        );
    }

    #[test]
    fn scale_on_daemonset_returns_empty_checks() {
        let ds = ResourceRef::DaemonSet("daemon".into(), "default".into());
        assert!(ds.authorization_checks(DetailAction::Scale).is_empty());
    }

    #[test]
    fn helm_release_base_access_returns_none_so_delete_has_no_checks() {
        let helm = ResourceRef::HelmRelease("release".into(), "default".into());
        assert!(helm.authorization_checks(DetailAction::Delete).is_empty());
    }

    #[test]
    fn helm_release_view_yaml_checks_secrets_list() {
        let helm = ResourceRef::HelmRelease("release".into(), "default".into());
        let checks = helm.authorization_checks(DetailAction::ViewYaml);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].resource, "secrets");
        assert_eq!(checks[0].verb, "list");
    }

    #[test]
    fn bookmark_and_relationships_require_no_authorization_checks() {
        let pod = ResourceRef::Pod("pod-0".into(), "ns".into());
        assert!(
            pod.authorization_checks(DetailAction::ToggleBookmark)
                .is_empty()
        );
        assert!(
            pod.authorization_checks(DetailAction::ViewRelationships)
                .is_empty()
        );
    }

    #[test]
    fn debug_container_requires_get_ephemeral_and_exec_subresources() {
        let pod = ResourceRef::Pod("api-0".into(), "default".into());
        let checks = pod.authorization_checks(DetailAction::DebugContainer);
        assert_eq!(checks.len(), 3);
        assert!(
            checks
                .iter()
                .any(|c| c.verb == "get" && c.resource == "pods")
        );
        assert!(
            checks
                .iter()
                .any(|c| c.verb == "patch"
                    && c.subresource.as_deref() == Some("ephemeralcontainers"))
        );
        assert!(
            checks
                .iter()
                .any(|c| c.verb == "create" && c.subresource.as_deref() == Some("exec"))
        );
    }

    #[test]
    fn drain_checks_include_pod_eviction_subresource() {
        let node = ResourceRef::Node("node-0".into());
        let checks = node.authorization_checks(DetailAction::Drain);
        assert_eq!(checks.len(), 3);
        assert!(
            checks
                .iter()
                .any(|c| c.verb == "patch" && c.resource == "nodes")
        );
        assert!(
            checks
                .iter()
                .any(|c| c.verb == "list" && c.resource == "pods")
        );
        assert!(
            checks
                .iter()
                .any(|c| c.verb == "create" && c.subresource.as_deref() == Some("eviction"))
        );
    }

    #[test]
    fn edit_yaml_combines_view_yaml_and_patch_checks() {
        let pod = ResourceRef::Pod("api-0".into(), "default".into());
        let view_checks = pod.authorization_checks(DetailAction::ViewYaml);
        let edit_checks = pod.authorization_checks(DetailAction::EditYaml);
        assert!(edit_checks.len() > view_checks.len());
        assert!(edit_checks.iter().any(|c| c.verb == "patch"));
        assert!(edit_checks.iter().any(|c| c.verb == "get"));
    }

    #[test]
    fn trigger_cronjob_requires_get_cronjob_and_create_job() {
        let cj = ResourceRef::CronJob("nightly".into(), "ops".into());
        let checks = cj.authorization_checks(DetailAction::Trigger);
        assert!(
            checks
                .iter()
                .any(|c| c.verb == "get" && c.resource == "cronjobs")
        );
        assert!(
            checks
                .iter()
                .any(|c| c.verb == "create" && c.resource == "jobs")
        );
    }

    #[test]
    fn view_events_for_cluster_scoped_resource_returns_empty() {
        let node = ResourceRef::Node("node-0".into());
        assert!(
            node.authorization_checks(DetailAction::ViewEvents)
                .is_empty()
        );
    }

    // ── original tests ───────────────────────────────────────────────

    #[test]
    fn pod_logs_and_exec_use_distinct_subresource_checks() {
        let resource = ResourceRef::Pod("api-0".to_string(), "default".to_string());

        let log_checks = resource.authorization_checks(DetailAction::Logs);
        let exec_checks = resource.authorization_checks(DetailAction::Exec);

        assert!(
            log_checks
                .iter()
                .any(|check| check.subresource.as_deref() == Some("log"))
        );
        assert!(
            exec_checks
                .iter()
                .any(|check| check.subresource.as_deref() == Some("exec"))
        );
        assert!(
            !exec_checks
                .iter()
                .any(|check| check.subresource.as_deref() == Some("log"))
        );
    }

    #[test]
    fn workload_logs_require_pod_listing_in_addition_to_log_access() {
        let resource = ResourceRef::Deployment("api".to_string(), "default".to_string());
        let checks = resource.authorization_checks(DetailAction::Logs);

        assert!(checks.iter().any(|check| {
            check.resource == "pods" && check.verb == "list" && check.subresource.is_none()
        }));
        assert!(checks.iter().any(|check| {
            check.resource == "pods"
                && check.verb == "get"
                && check.subresource.as_deref() == Some("log")
        }));
    }

    #[test]
    fn probes_require_direct_pod_read_access() {
        let resource = ResourceRef::Pod("api-0".to_string(), "default".to_string());
        let checks = resource.authorization_checks(DetailAction::Probes);

        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].resource, "pods");
        assert_eq!(checks[0].verb, "get");
        assert_eq!(checks[0].namespace.as_deref(), Some("default"));
        assert_eq!(checks[0].name.as_deref(), Some("api-0"));
    }
}
