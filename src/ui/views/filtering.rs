//! Canonical per-view filtering helpers shared by rendering, selection, and tests.

use crate::{
    app::{
        AppView, PodSortState, WorkloadSortState, filtered_pod_indices, filtered_workload_indices,
    },
    k8s::dtos::{
        ClusterRoleBindingInfo, ClusterRoleInfo, ConfigMapInfo, CronJobInfo, DaemonSetInfo,
        DeploymentInfo, EndpointInfo, HelmReleaseInfo, HelmRepoInfo, HpaInfo, IngressClassInfo,
        IngressInfo, JobInfo, K8sEventInfo, LimitRangeInfo, NamespaceInfo, NetworkPolicyInfo,
        NodeInfo, PodDisruptionBudgetInfo, PriorityClassInfo, PvInfo, PvcInfo, ReplicaSetInfo,
        ReplicationControllerInfo, ResourceQuotaInfo, RoleBindingInfo, RoleInfo, SecretInfo,
        ServiceAccountInfo, ServiceInfo, StatefulSetInfo, StorageClassInfo,
    },
    state::{
        ClusterSnapshot,
        issues::{compute_issues, filtered_issue_indices},
    },
    ui::contains_ci,
};

use super::flux::filtered_flux_indices_for_view;

fn simple_filtered_indices<T, Match>(items: &[T], query: &str, matches: Match) -> Vec<usize>
where
    Match: Fn(&T, &str) -> bool,
{
    let query = query.trim();
    if query.is_empty() {
        return (0..items.len()).collect();
    }

    items
        .iter()
        .enumerate()
        .filter_map(|(idx, item)| matches(item, query).then_some(idx))
        .collect()
}

pub fn filtered_node_indices(
    items: &[NodeInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |node, needle| contains_ci(&node.name, needle),
        |node| node.name.as_str(),
        |_node| "",
        |node| node.created_at.map(age_duration_now),
    )
}

pub fn filtered_service_indices(
    items: &[ServiceInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |service, needle| {
            contains_ci(&service.name, needle)
                || contains_ci(&service.namespace, needle)
                || contains_ci(&service.type_, needle)
        },
        |service| service.name.as_str(),
        |service| service.namespace.as_str(),
        |service| service.created_at.map(age_duration_now),
    )
}

pub fn filtered_deployment_indices(
    items: &[DeploymentInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |deployment, needle| {
            contains_ci(&deployment.name, needle) || contains_ci(&deployment.namespace, needle)
        },
        |deployment| deployment.name.as_str(),
        |deployment| deployment.namespace.as_str(),
        |deployment| deployment.created_at.map(age_duration_now),
    )
}

pub fn filtered_statefulset_indices(
    items: &[StatefulSetInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |statefulset, needle| {
            contains_ci(&statefulset.name, needle)
                || contains_ci(&statefulset.namespace, needle)
                || contains_ci(statefulset.image.as_deref().unwrap_or_default(), needle)
        },
        |statefulset| statefulset.name.as_str(),
        |statefulset| statefulset.namespace.as_str(),
        |statefulset| statefulset.created_at.map(age_duration_now),
    )
}

pub fn filtered_daemonset_indices(
    items: &[DaemonSetInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |daemonset, needle| {
            contains_ci(&daemonset.name, needle)
                || contains_ci(&daemonset.namespace, needle)
                || contains_ci(&daemonset.selector, needle)
                || contains_ci(daemonset.image.as_deref().unwrap_or_default(), needle)
                || daemonset
                    .labels
                    .iter()
                    .any(|(key, value)| contains_ci(key, needle) || contains_ci(value, needle))
        },
        |daemonset| daemonset.name.as_str(),
        |daemonset| daemonset.namespace.as_str(),
        |daemonset| daemonset.created_at.map(age_duration_now),
    )
}

pub fn filtered_job_indices(
    items: &[JobInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |job, needle| {
            contains_ci(&job.name, needle)
                || contains_ci(&job.namespace, needle)
                || contains_ci(&job.status, needle)
        },
        |job| job.name.as_str(),
        |job| job.namespace.as_str(),
        |job| job.created_at.map(age_duration_now),
    )
}

pub fn filtered_cronjob_indices(
    items: &[CronJobInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |cronjob, needle| {
            contains_ci(&cronjob.name, needle)
                || contains_ci(&cronjob.namespace, needle)
                || contains_ci(&cronjob.schedule, needle)
        },
        |cronjob| cronjob.name.as_str(),
        |cronjob| cronjob.namespace.as_str(),
        |cronjob| cronjob.created_at.map(age_duration_now),
    )
}

pub fn filtered_resource_quota_indices(
    items: &[ResourceQuotaInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |quota, needle| {
            contains_ci(&quota.name, needle)
                || contains_ci(&quota.namespace, needle)
                || quota.hard.keys().any(|key| contains_ci(key, needle))
        },
        |quota| quota.name.as_str(),
        |quota| quota.namespace.as_str(),
        |quota| quota.created_at.map(age_duration_now),
    )
}

pub fn filtered_limit_range_indices(
    items: &[LimitRangeInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |limit_range, needle| {
            contains_ci(&limit_range.name, needle)
                || contains_ci(&limit_range.namespace, needle)
                || limit_range
                    .limits
                    .iter()
                    .any(|spec| contains_ci(&spec.type_, needle))
        },
        |limit_range| limit_range.name.as_str(),
        |limit_range| limit_range.namespace.as_str(),
        |limit_range| limit_range.created_at.map(age_duration_now),
    )
}

pub fn filtered_pdb_indices(
    items: &[PodDisruptionBudgetInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |pdb, needle| {
            contains_ci(&pdb.name, needle)
                || contains_ci(&pdb.namespace, needle)
                || contains_ci(pdb.min_available.as_deref().unwrap_or_default(), needle)
                || contains_ci(pdb.max_unavailable.as_deref().unwrap_or_default(), needle)
        },
        |pdb| pdb.name.as_str(),
        |pdb| pdb.namespace.as_str(),
        |pdb| pdb.created_at.map(age_duration_now),
    )
}

pub fn filtered_ingress_indices(items: &[IngressInfo], query: &str) -> Vec<usize> {
    simple_filtered_indices(items, query, |ingress, needle| {
        contains_ci(&ingress.name, needle)
            || contains_ci(&ingress.namespace, needle)
            || ingress
                .class
                .as_ref()
                .is_some_and(|class| contains_ci(class, needle))
            || ingress
                .address
                .as_ref()
                .is_some_and(|address| contains_ci(address, needle))
            || ingress.hosts.iter().any(|host| contains_ci(host, needle))
    })
}

pub fn filtered_hpa_indices(items: &[HpaInfo], query: &str) -> Vec<usize> {
    simple_filtered_indices(items, query, |hpa, needle| {
        contains_ci(&hpa.name, needle)
            || contains_ci(&hpa.namespace, needle)
            || contains_ci(&hpa.reference, needle)
    })
}

pub fn filtered_event_indices(items: &[K8sEventInfo], query: &str) -> Vec<usize> {
    simple_filtered_indices(items, query, |event, needle| {
        contains_ci(&event.type_, needle)
            || contains_ci(&event.namespace, needle)
            || contains_ci(&event.involved_object, needle)
            || contains_ci(&event.reason, needle)
            || contains_ci(&event.message, needle)
    })
}

pub fn filtered_replicaset_indices(
    items: &[ReplicaSetInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |replicaset, needle| {
            contains_ci(&replicaset.name, needle) || contains_ci(&replicaset.namespace, needle)
        },
        |replicaset| replicaset.name.as_str(),
        |replicaset| replicaset.namespace.as_str(),
        |replicaset| replicaset.created_at.map(age_duration_now),
    )
}

pub fn filtered_replication_controller_indices(
    items: &[ReplicationControllerInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |controller, needle| {
            contains_ci(&controller.name, needle) || contains_ci(&controller.namespace, needle)
        },
        |controller| controller.name.as_str(),
        |controller| controller.namespace.as_str(),
        |controller| controller.created_at.map(age_duration_now),
    )
}

pub fn filtered_service_account_indices(
    items: &[ServiceAccountInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |service_account, needle| {
            contains_ci(&service_account.name, needle)
                || contains_ci(&service_account.namespace, needle)
        },
        |service_account| service_account.name.as_str(),
        |service_account| service_account.namespace.as_str(),
        |service_account| service_account.created_at.map(age_duration_now),
    )
}

pub fn filtered_role_indices(
    items: &[RoleInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |role, needle| contains_ci(&role.name, needle) || contains_ci(&role.namespace, needle),
        |role| role.name.as_str(),
        |role| role.namespace.as_str(),
        |role| role.created_at.map(age_duration_now),
    )
}

pub fn filtered_role_binding_indices(
    items: &[RoleBindingInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |binding, needle| {
            contains_ci(&binding.name, needle)
                || contains_ci(&binding.namespace, needle)
                || contains_ci(&binding.role_ref_name, needle)
        },
        |binding| binding.name.as_str(),
        |binding| binding.namespace.as_str(),
        |binding| binding.created_at.map(age_duration_now),
    )
}

pub fn filtered_cluster_role_indices(
    items: &[ClusterRoleInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |role, needle| contains_ci(&role.name, needle),
        |role| role.name.as_str(),
        |_role| "",
        |role| role.created_at.map(age_duration_now),
    )
}

pub fn filtered_cluster_role_binding_indices(
    items: &[ClusterRoleBindingInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |binding, needle| {
            contains_ci(&binding.name, needle) || contains_ci(&binding.role_ref_name, needle)
        },
        |binding| binding.name.as_str(),
        |_binding| "",
        |binding| binding.created_at.map(age_duration_now),
    )
}

pub fn filtered_pvc_indices(
    items: &[PvcInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |pvc, needle| contains_ci(&pvc.name, needle) || contains_ci(&pvc.namespace, needle),
        |pvc| pvc.name.as_str(),
        |pvc| pvc.namespace.as_str(),
        |_pvc| None,
    )
}

pub fn filtered_pv_indices(
    items: &[PvInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |pv, needle| contains_ci(&pv.name, needle),
        |pv| pv.name.as_str(),
        |_pv| "",
        |_pv| None,
    )
}

pub fn filtered_storage_class_indices(
    items: &[StorageClassInfo],
    query: &str,
    sort: Option<WorkloadSortState>,
) -> Vec<usize> {
    filtered_workload_indices(
        items,
        query,
        sort,
        |storage_class, needle| contains_ci(&storage_class.name, needle),
        |storage_class| storage_class.name.as_str(),
        |_storage_class| "",
        |_storage_class| None,
    )
}

pub fn filtered_endpoint_indices(items: &[EndpointInfo], query: &str) -> Vec<usize> {
    simple_filtered_indices(items, query, |endpoint, needle| {
        contains_ci(&endpoint.name, needle) || contains_ci(&endpoint.namespace, needle)
    })
}

pub fn filtered_ingress_class_indices(items: &[IngressClassInfo], query: &str) -> Vec<usize> {
    simple_filtered_indices(items, query, |ingress_class, needle| {
        contains_ci(&ingress_class.name, needle) || contains_ci(&ingress_class.controller, needle)
    })
}

pub fn filtered_network_policy_indices(items: &[NetworkPolicyInfo], query: &str) -> Vec<usize> {
    simple_filtered_indices(items, query, |policy, needle| {
        contains_ci(&policy.name, needle)
            || contains_ci(&policy.namespace, needle)
            || contains_ci(&policy.pod_selector, needle)
    })
}

pub fn filtered_config_map_indices(items: &[ConfigMapInfo], query: &str) -> Vec<usize> {
    simple_filtered_indices(items, query, |config_map, needle| {
        contains_ci(&config_map.name, needle) || contains_ci(&config_map.namespace, needle)
    })
}

pub fn filtered_secret_indices(items: &[SecretInfo], query: &str) -> Vec<usize> {
    simple_filtered_indices(items, query, |secret, needle| {
        contains_ci(&secret.name, needle)
            || contains_ci(&secret.namespace, needle)
            || contains_ci(&secret.type_, needle)
    })
}

pub fn filtered_namespace_indices(items: &[NamespaceInfo], query: &str) -> Vec<usize> {
    simple_filtered_indices(items, query, |namespace, needle| {
        contains_ci(&namespace.name, needle)
    })
}

pub fn filtered_priority_class_indices(items: &[PriorityClassInfo], query: &str) -> Vec<usize> {
    simple_filtered_indices(items, query, |priority_class, needle| {
        contains_ci(&priority_class.name, needle)
    })
}

pub fn filtered_helm_release_indices(items: &[HelmReleaseInfo], query: &str) -> Vec<usize> {
    simple_filtered_indices(items, query, |release, needle| {
        contains_ci(&release.name, needle)
            || contains_ci(&release.namespace, needle)
            || contains_ci(&release.chart, needle)
    })
}

pub fn filtered_helm_repo_indices(items: &[HelmRepoInfo], query: &str) -> Vec<usize> {
    simple_filtered_indices(items, query, |repo, needle| {
        contains_ci(&repo.name, needle) || contains_ci(&repo.url, needle)
    })
}

pub fn filtered_indices_for_view(
    view: AppView,
    snapshot: &ClusterSnapshot,
    query: &str,
    workload_sort: Option<WorkloadSortState>,
    pod_sort: Option<PodSortState>,
) -> Vec<usize> {
    match view {
        AppView::Dashboard | AppView::Bookmarks | AppView::PortForwarding | AppView::Extensions => {
            Vec::new()
        }
        AppView::Issues => {
            let issues = compute_issues(snapshot);
            filtered_issue_indices(&issues, query.trim())
        }
        AppView::Nodes => filtered_node_indices(&snapshot.nodes, query, workload_sort),
        AppView::Pods => filtered_pod_indices(&snapshot.pods, query, pod_sort),
        AppView::Services => filtered_service_indices(&snapshot.services, query, workload_sort),
        AppView::ResourceQuotas => {
            filtered_resource_quota_indices(&snapshot.resource_quotas, query, workload_sort)
        }
        AppView::LimitRanges => {
            filtered_limit_range_indices(&snapshot.limit_ranges, query, workload_sort)
        }
        AppView::PodDisruptionBudgets => {
            filtered_pdb_indices(&snapshot.pod_disruption_budgets, query, workload_sort)
        }
        AppView::Deployments => {
            filtered_deployment_indices(&snapshot.deployments, query, workload_sort)
        }
        AppView::StatefulSets => {
            filtered_statefulset_indices(&snapshot.statefulsets, query, workload_sort)
        }
        AppView::DaemonSets => {
            filtered_daemonset_indices(&snapshot.daemonsets, query, workload_sort)
        }
        AppView::ReplicaSets => {
            filtered_replicaset_indices(&snapshot.replicasets, query, workload_sort)
        }
        AppView::ReplicationControllers => filtered_replication_controller_indices(
            &snapshot.replication_controllers,
            query,
            workload_sort,
        ),
        AppView::Jobs => filtered_job_indices(&snapshot.jobs, query, workload_sort),
        AppView::CronJobs => filtered_cronjob_indices(&snapshot.cronjobs, query, workload_sort),
        AppView::Endpoints => filtered_endpoint_indices(&snapshot.endpoints, query),
        AppView::Ingresses => filtered_ingress_indices(&snapshot.ingresses, query),
        AppView::IngressClasses => filtered_ingress_class_indices(&snapshot.ingress_classes, query),
        AppView::NetworkPolicies => {
            filtered_network_policy_indices(&snapshot.network_policies, query)
        }
        AppView::ConfigMaps => filtered_config_map_indices(&snapshot.config_maps, query),
        AppView::Secrets => filtered_secret_indices(&snapshot.secrets, query),
        AppView::HPAs => filtered_hpa_indices(&snapshot.hpas, query),
        AppView::PriorityClasses => {
            filtered_priority_class_indices(&snapshot.priority_classes, query)
        }
        AppView::PersistentVolumeClaims => {
            filtered_pvc_indices(&snapshot.pvcs, query, workload_sort)
        }
        AppView::PersistentVolumes => filtered_pv_indices(&snapshot.pvs, query, workload_sort),
        AppView::StorageClasses => {
            filtered_storage_class_indices(&snapshot.storage_classes, query, workload_sort)
        }
        AppView::Namespaces => filtered_namespace_indices(&snapshot.namespace_list, query),
        AppView::Events => filtered_event_indices(&snapshot.events, query),
        AppView::ServiceAccounts => {
            filtered_service_account_indices(&snapshot.service_accounts, query, workload_sort)
        }
        AppView::Roles => filtered_role_indices(&snapshot.roles, query, workload_sort),
        AppView::RoleBindings => {
            filtered_role_binding_indices(&snapshot.role_bindings, query, workload_sort)
        }
        AppView::ClusterRoles => {
            filtered_cluster_role_indices(&snapshot.cluster_roles, query, workload_sort)
        }
        AppView::ClusterRoleBindings => filtered_cluster_role_binding_indices(
            &snapshot.cluster_role_bindings,
            query,
            workload_sort,
        ),
        AppView::HelmCharts => filtered_helm_repo_indices(&snapshot.helm_repositories, query),
        AppView::HelmReleases => filtered_helm_release_indices(&snapshot.helm_releases, query),
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
            filtered_flux_indices_for_view(view, snapshot, query, workload_sort).to_vec()
        }
    }
}

/// Computes a fresh age duration from a creation timestamp, used for sorting.
pub(crate) fn age_duration_now(created_at: chrono::DateTime<chrono::Utc>) -> std::time::Duration {
    let age_secs = (chrono::Utc::now().timestamp() - created_at.timestamp()).max(0) as u64;
    std::time::Duration::from_secs(age_secs)
}
