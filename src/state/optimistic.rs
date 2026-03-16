//! Optimistic update handlers for immediate UI feedback after mutations.

use std::{collections::HashSet, sync::Arc};

use crate::app::ResourceRef;

use super::{ClusterSnapshot, FluxCounts, GlobalState};

/// Removes items from a vec where `key(item) == expected_name`.
fn remove_named<T, F>(items: &mut Vec<T>, key: F, expected_name: &str) -> bool
where
    F: Fn(&T) -> &String,
{
    let before = items.len();
    items.retain(|item| key(item) != expected_name);
    before != items.len()
}

/// Removes items from a vec where `key(item) == (expected_name, expected_namespace)`.
fn remove_named_in_namespace<T, F>(
    items: &mut Vec<T>,
    key: F,
    expected_name: &str,
    expected_namespace: &str,
) -> bool
where
    F: Fn(&T) -> (&String, &String),
{
    let before = items.len();
    items.retain(|item| {
        let (name, namespace) = key(item);
        name != expected_name || namespace != expected_namespace
    });
    before != items.len()
}

impl GlobalState {
    /// Applies an optimistic node schedulable state change after cordon/uncordon.
    pub fn apply_optimistic_node_schedulable(&mut self, node_name: &str, unschedulable: bool) {
        let snap = Arc::make_mut(&mut self.snapshot);
        if let Some(node) = snap.nodes.iter_mut().find(|n| n.name == node_name) {
            node.unschedulable = unschedulable;
            snap.snapshot_version = snap.snapshot_version.saturating_add(1);
            self.snapshot_dirty = true;
            self.publish_snapshot();
        }
    }

    /// Applies a successful delete locally so the list updates immediately
    /// before the background refresh completes.
    pub fn apply_optimistic_delete(&mut self, resource: &ResourceRef) {
        let snap = Arc::make_mut(&mut self.snapshot);
        let changed = match resource {
            ResourceRef::Node(name) => remove_named(&mut snap.nodes, |item| &item.name, name),
            ResourceRef::Pod(name, ns) => remove_named_in_namespace(
                &mut snap.pods,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Service(name, ns) => remove_named_in_namespace(
                &mut snap.services,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Deployment(name, ns) => remove_named_in_namespace(
                &mut snap.deployments,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::StatefulSet(name, ns) => remove_named_in_namespace(
                &mut snap.statefulsets,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::DaemonSet(name, ns) => remove_named_in_namespace(
                &mut snap.daemonsets,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::ReplicaSet(name, ns) => remove_named_in_namespace(
                &mut snap.replicasets,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::ReplicationController(name, ns) => remove_named_in_namespace(
                &mut snap.replication_controllers,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Job(name, ns) => remove_named_in_namespace(
                &mut snap.jobs,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::CronJob(name, ns) => remove_named_in_namespace(
                &mut snap.cronjobs,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::ResourceQuota(name, ns) => remove_named_in_namespace(
                &mut snap.resource_quotas,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::LimitRange(name, ns) => remove_named_in_namespace(
                &mut snap.limit_ranges,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::PodDisruptionBudget(name, ns) => remove_named_in_namespace(
                &mut snap.pod_disruption_budgets,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Endpoint(name, ns) => remove_named_in_namespace(
                &mut snap.endpoints,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Ingress(name, ns) => remove_named_in_namespace(
                &mut snap.ingresses,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::IngressClass(name) => {
                remove_named(&mut snap.ingress_classes, |item| &item.name, name)
            }
            ResourceRef::NetworkPolicy(name, ns) => remove_named_in_namespace(
                &mut snap.network_policies,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::ConfigMap(name, ns) => remove_named_in_namespace(
                &mut snap.config_maps,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Secret(name, ns) => remove_named_in_namespace(
                &mut snap.secrets,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Hpa(name, ns) => remove_named_in_namespace(
                &mut snap.hpas,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::PriorityClass(name) => {
                remove_named(&mut snap.priority_classes, |item| &item.name, name)
            }
            ResourceRef::Pvc(name, ns) => remove_named_in_namespace(
                &mut snap.pvcs,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Pv(name) => remove_named(&mut snap.pvs, |item| &item.name, name),
            ResourceRef::StorageClass(name) => {
                remove_named(&mut snap.storage_classes, |item| &item.name, name)
            }
            ResourceRef::Namespace(name) => {
                remove_named(&mut snap.namespace_list, |item| &item.name, name)
            }
            ResourceRef::Event(name, ns) => remove_named_in_namespace(
                &mut snap.events,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::ServiceAccount(name, ns) => remove_named_in_namespace(
                &mut snap.service_accounts,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::Role(name, ns) => remove_named_in_namespace(
                &mut snap.roles,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::RoleBinding(name, ns) => remove_named_in_namespace(
                &mut snap.role_bindings,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::ClusterRole(name) => {
                remove_named(&mut snap.cluster_roles, |item| &item.name, name)
            }
            ResourceRef::ClusterRoleBinding(name) => {
                remove_named(&mut snap.cluster_role_bindings, |item| &item.name, name)
            }
            ResourceRef::HelmRelease(name, ns) => remove_named_in_namespace(
                &mut snap.helm_releases,
                |item| (&item.name, &item.namespace),
                name,
                ns,
            ),
            ResourceRef::CustomResource {
                name,
                namespace,
                group,
                version,
                kind,
                plural,
            } => {
                let before = snap.flux_resources.len();
                snap.flux_resources.retain(|item| {
                    item.name != *name
                        || item.namespace != *namespace
                        || item.group != *group
                        || item.version != *version
                        || item.kind != *kind
                        || item.plural != *plural
                });
                before != snap.flux_resources.len()
            }
        };

        if !changed {
            return;
        }

        snap.namespaces_count = count_namespaces(snap);
        snap.flux_counts = FluxCounts::compute(&snap.flux_resources);
        snap.snapshot_version = snap.snapshot_version.saturating_add(1);
        self.snapshot_dirty = true;
        self.publish_snapshot();
    }

    /// Applies a successful scale locally so list views reflect the requested
    /// replica target immediately before the background refresh completes.
    pub fn apply_optimistic_scale(&mut self, resource: &ResourceRef, replicas: i32) {
        let snap = Arc::make_mut(&mut self.snapshot);
        let changed = match resource {
            ResourceRef::Deployment(name, ns) => snap
                .deployments
                .iter_mut()
                .find(|item| item.name == *name && item.namespace == *ns)
                .is_some_and(|deployment| {
                    if deployment.desired_replicas == replicas {
                        return false;
                    }
                    deployment.desired_replicas = replicas;
                    deployment.ready = format!("{}/{}", deployment.ready_replicas, replicas);
                    true
                }),
            ResourceRef::StatefulSet(name, ns) => snap
                .statefulsets
                .iter_mut()
                .find(|item| item.name == *name && item.namespace == *ns)
                .is_some_and(|statefulset| {
                    if statefulset.desired_replicas == replicas {
                        return false;
                    }
                    statefulset.desired_replicas = replicas;
                    true
                }),
            _ => false,
        };

        if !changed {
            return;
        }

        snap.snapshot_version = snap.snapshot_version.saturating_add(1);
        self.snapshot_dirty = true;
        self.publish_snapshot();
    }
}

/// Counts distinct namespaces across pods, services, and deployments.
fn count_namespaces(snap: &ClusterSnapshot) -> usize {
    snap.pods
        .iter()
        .map(|pod| pod.namespace.as_str())
        .chain(
            snap.services
                .iter()
                .map(|service| service.namespace.as_str()),
        )
        .chain(
            snap.deployments
                .iter()
                .map(|deployment| deployment.namespace.as_str()),
        )
        .collect::<HashSet<_>>()
        .len()
}
