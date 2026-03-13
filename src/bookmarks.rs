//! Resource bookmarks persisted per cluster context.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    app::{AppView, PodSortState, ResourceRef, WorkloadSortState},
    state::ClusterSnapshot,
    ui::contains_ci,
};

pub const MAX_BOOKMARKS_PER_CLUSTER: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookmarkToggleResult {
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkEntry {
    pub resource: ResourceRef,
    pub bookmarked_at_unix: i64,
}

impl BookmarkEntry {
    pub fn new(resource: ResourceRef) -> Self {
        Self {
            resource,
            bookmarked_at_unix: Utc::now().timestamp(),
        }
    }

    pub fn matches_query(&self, query: &str) -> bool {
        let query = query.trim();
        query.is_empty()
            || contains_ci(self.resource.kind(), query)
            || contains_ci(self.resource.name(), query)
            || self
                .resource
                .namespace()
                .is_some_and(|namespace| contains_ci(namespace, query))
            || self
                .resource
                .primary_view()
                .is_some_and(|view| contains_ci(view.label(), query))
    }
}

pub fn filtered_bookmark_indices(bookmarks: &[BookmarkEntry], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        (0..bookmarks.len()).collect()
    } else {
        bookmarks
            .iter()
            .enumerate()
            .filter_map(|(idx, bookmark)| bookmark.matches_query(query).then_some(idx))
            .collect()
    }
}

pub fn toggle_bookmark(
    bookmarks: &mut Vec<BookmarkEntry>,
    resource: ResourceRef,
) -> Result<BookmarkToggleResult, String> {
    if let Some(idx) = bookmarks
        .iter()
        .position(|bookmark| bookmark.resource == resource)
    {
        bookmarks.remove(idx);
        return Ok(BookmarkToggleResult::Removed);
    }

    if bookmarks.len() >= MAX_BOOKMARKS_PER_CLUSTER {
        return Err(format!(
            "Bookmark limit reached ({MAX_BOOKMARKS_PER_CLUSTER} per cluster). Remove one before adding another."
        ));
    }

    bookmarks.insert(0, BookmarkEntry::new(resource));
    bookmarks.sort_unstable_by(|left, right| {
        right
            .bookmarked_at_unix
            .cmp(&left.bookmarked_at_unix)
            .then_with(|| left.resource.kind().cmp(right.resource.kind()))
            .then_with(|| left.resource.name().cmp(right.resource.name()))
            .then_with(|| left.resource.namespace().cmp(&right.resource.namespace()))
    });
    Ok(BookmarkToggleResult::Added)
}

pub fn resource_exists(snapshot: &ClusterSnapshot, resource: &ResourceRef) -> bool {
    match resource {
        ResourceRef::Node(name) => snapshot.nodes.iter().any(|item| item.name == *name),
        ResourceRef::Pod(name, namespace) => snapshot
            .pods
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::Service(name, namespace) => snapshot
            .services
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::Deployment(name, namespace) => snapshot
            .deployments
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::StatefulSet(name, namespace) => snapshot
            .statefulsets
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::DaemonSet(name, namespace) => snapshot
            .daemonsets
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::ReplicaSet(name, namespace) => snapshot
            .replicasets
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::ReplicationController(name, namespace) => snapshot
            .replication_controllers
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::Job(name, namespace) => snapshot
            .jobs
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::CronJob(name, namespace) => snapshot
            .cronjobs
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::ResourceQuota(name, namespace) => snapshot
            .resource_quotas
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::LimitRange(name, namespace) => snapshot
            .limit_ranges
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::PodDisruptionBudget(name, namespace) => snapshot
            .pod_disruption_budgets
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::Endpoint(name, namespace) => snapshot
            .endpoints
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::Ingress(name, namespace) => snapshot
            .ingresses
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::IngressClass(name) => snapshot
            .ingress_classes
            .iter()
            .any(|item| item.name == *name),
        ResourceRef::NetworkPolicy(name, namespace) => snapshot
            .network_policies
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::ConfigMap(name, namespace) => snapshot
            .config_maps
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::Secret(name, namespace) => snapshot
            .secrets
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::Hpa(name, namespace) => snapshot
            .hpas
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::PriorityClass(name) => snapshot
            .priority_classes
            .iter()
            .any(|item| item.name == *name),
        ResourceRef::Pvc(name, namespace) => snapshot
            .pvcs
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::Pv(name) => snapshot.pvs.iter().any(|item| item.name == *name),
        ResourceRef::StorageClass(name) => snapshot
            .storage_classes
            .iter()
            .any(|item| item.name == *name),
        ResourceRef::Namespace(name) => snapshot
            .namespace_list
            .iter()
            .any(|item| item.name == *name),
        ResourceRef::Event(name, namespace) => snapshot
            .events
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::ServiceAccount(name, namespace) => snapshot
            .service_accounts
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::Role(name, namespace) => snapshot
            .roles
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::RoleBinding(name, namespace) => snapshot
            .role_bindings
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::ClusterRole(name) => {
            snapshot.cluster_roles.iter().any(|item| item.name == *name)
        }
        ResourceRef::ClusterRoleBinding(name) => snapshot
            .cluster_role_bindings
            .iter()
            .any(|item| item.name == *name),
        ResourceRef::HelmRelease(name, namespace) => snapshot
            .helm_releases
            .iter()
            .any(|item| item.name == *name && item.namespace == *namespace),
        ResourceRef::CustomResource {
            name,
            namespace,
            group,
            version,
            kind,
            plural,
        } => snapshot.flux_resources.iter().any(|item| {
            item.name == *name
                && item.namespace == *namespace
                && item.group == *group
                && item.version == *version
                && item.kind == *kind
                && item.plural == *plural
        }),
    }
}

pub fn selected_bookmark_resource(
    bookmarks: &[BookmarkEntry],
    selected_idx: usize,
    query: &str,
) -> Option<ResourceRef> {
    let filtered = filtered_bookmark_indices(bookmarks, query);
    filtered
        .get(selected_idx.min(filtered.len().saturating_sub(1)))
        .and_then(|idx| bookmarks.get(*idx))
        .map(|bookmark| bookmark.resource.clone())
}

pub fn bookmark_selected_index(
    view: AppView,
    snapshot: &ClusterSnapshot,
    resource: &ResourceRef,
    workload_sort: Option<WorkloadSortState>,
    pod_sort: Option<PodSortState>,
) -> Option<usize> {
    let indices = crate::ui::views::filtering::filtered_indices_for_view(
        view,
        snapshot,
        "",
        workload_sort,
        pod_sort,
    );
    let resource_idx = match (view, resource) {
        (AppView::Nodes, ResourceRef::Node(name)) => {
            snapshot.nodes.iter().position(|item| item.name == *name)?
        }
        (AppView::Pods, ResourceRef::Pod(name, namespace)) => snapshot
            .pods
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::Services, ResourceRef::Service(name, namespace)) => snapshot
            .services
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::Deployments, ResourceRef::Deployment(name, namespace)) => snapshot
            .deployments
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::StatefulSets, ResourceRef::StatefulSet(name, namespace)) => snapshot
            .statefulsets
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::DaemonSets, ResourceRef::DaemonSet(name, namespace)) => snapshot
            .daemonsets
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::ReplicaSets, ResourceRef::ReplicaSet(name, namespace)) => snapshot
            .replicasets
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::ReplicationControllers, ResourceRef::ReplicationController(name, namespace)) => {
            snapshot
                .replication_controllers
                .iter()
                .position(|item| item.name == *name && item.namespace == *namespace)?
        }
        (AppView::Jobs, ResourceRef::Job(name, namespace)) => snapshot
            .jobs
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::CronJobs, ResourceRef::CronJob(name, namespace)) => snapshot
            .cronjobs
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::Endpoints, ResourceRef::Endpoint(name, namespace)) => {
            snapshot
                .endpoints
                .iter()
                .position(|item| item.name == *name && item.namespace == *namespace)?
        }
        (AppView::Ingresses, ResourceRef::Ingress(name, namespace)) => snapshot
            .ingresses
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::IngressClasses, ResourceRef::IngressClass(name)) => snapshot
            .ingress_classes
            .iter()
            .position(|item| item.name == *name)?,
        (AppView::NetworkPolicies, ResourceRef::NetworkPolicy(name, namespace)) => snapshot
            .network_policies
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::ConfigMaps, ResourceRef::ConfigMap(name, namespace)) => snapshot
            .config_maps
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::Secrets, ResourceRef::Secret(name, namespace)) => snapshot
            .secrets
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::ResourceQuotas, ResourceRef::ResourceQuota(name, namespace)) => snapshot
            .resource_quotas
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::LimitRanges, ResourceRef::LimitRange(name, namespace)) => snapshot
            .limit_ranges
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::PodDisruptionBudgets, ResourceRef::PodDisruptionBudget(name, namespace)) => {
            snapshot
                .pod_disruption_budgets
                .iter()
                .position(|item| item.name == *name && item.namespace == *namespace)?
        }
        (AppView::HPAs, ResourceRef::Hpa(name, namespace)) => snapshot
            .hpas
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::PriorityClasses, ResourceRef::PriorityClass(name)) => snapshot
            .priority_classes
            .iter()
            .position(|item| item.name == *name)?,
        (AppView::PersistentVolumeClaims, ResourceRef::Pvc(name, namespace)) => snapshot
            .pvcs
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::PersistentVolumes, ResourceRef::Pv(name)) => {
            snapshot.pvs.iter().position(|item| item.name == *name)?
        }
        (AppView::StorageClasses, ResourceRef::StorageClass(name)) => snapshot
            .storage_classes
            .iter()
            .position(|item| item.name == *name)?,
        (AppView::Namespaces, ResourceRef::Namespace(name)) => snapshot
            .namespace_list
            .iter()
            .position(|item| item.name == *name)?,
        (AppView::Events, ResourceRef::Event(name, namespace)) => snapshot
            .events
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::ServiceAccounts, ResourceRef::ServiceAccount(name, namespace)) => snapshot
            .service_accounts
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::Roles, ResourceRef::Role(name, namespace)) => snapshot
            .roles
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::RoleBindings, ResourceRef::RoleBinding(name, namespace)) => snapshot
            .role_bindings
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (AppView::ClusterRoles, ResourceRef::ClusterRole(name)) => snapshot
            .cluster_roles
            .iter()
            .position(|item| item.name == *name)?,
        (AppView::ClusterRoleBindings, ResourceRef::ClusterRoleBinding(name)) => snapshot
            .cluster_role_bindings
            .iter()
            .position(|item| item.name == *name)?,
        (AppView::HelmReleases, ResourceRef::HelmRelease(name, namespace)) => snapshot
            .helm_releases
            .iter()
            .position(|item| item.name == *name && item.namespace == *namespace)?,
        (
            AppView::FluxCDAll
            | AppView::FluxCDAlertProviders
            | AppView::FluxCDAlerts
            | AppView::FluxCDArtifacts
            | AppView::FluxCDHelmReleases
            | AppView::FluxCDHelmRepositories
            | AppView::FluxCDImages
            | AppView::FluxCDKustomizations
            | AppView::FluxCDReceivers
            | AppView::FluxCDSources,
            ResourceRef::CustomResource {
                name,
                namespace,
                group,
                version,
                kind,
                plural,
            },
        ) => snapshot.flux_resources.iter().position(|item| {
            item.name == *name
                && item.namespace == *namespace
                && item.group == *group
                && item.version == *version
                && item.kind == *kind
                && item.plural == *plural
        })?,
        _ => return None,
    };

    indices.iter().position(|idx| *idx == resource_idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_bookmark_adds_and_removes_same_resource() {
        let resource = ResourceRef::Pod("api".to_string(), "default".to_string());
        let mut bookmarks = Vec::new();

        assert_eq!(
            toggle_bookmark(&mut bookmarks, resource.clone()).expect("bookmark added"),
            BookmarkToggleResult::Added
        );
        assert_eq!(bookmarks.len(), 1);

        assert_eq!(
            toggle_bookmark(&mut bookmarks, resource).expect("bookmark removed"),
            BookmarkToggleResult::Removed
        );
        assert!(bookmarks.is_empty());
    }

    #[test]
    fn filtered_bookmarks_matches_kind_name_and_namespace() {
        let bookmarks = vec![BookmarkEntry {
            resource: ResourceRef::Secret("app-secret".to_string(), "prod".to_string()),
            bookmarked_at_unix: 0,
        }];

        assert_eq!(filtered_bookmark_indices(&bookmarks, "secret"), vec![0]);
        assert_eq!(filtered_bookmark_indices(&bookmarks, "app"), vec![0]);
        assert_eq!(filtered_bookmark_indices(&bookmarks, "prod"), vec![0]);
    }
}
