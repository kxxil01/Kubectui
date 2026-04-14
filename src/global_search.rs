use std::{
    collections::BTreeMap,
    sync::{Arc, LazyLock, Mutex},
};

use crate::{
    app::{AppView, ResourceRef},
    k8s::dtos::{CustomResourceDefinitionInfo, CustomResourceInfo},
    state::ClusterSnapshot,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalResourceSearchEntry {
    pub resource: ResourceRef,
    pub title: String,
    pub subtitle: String,
    pub aliases: Vec<String>,
    pub badge_label: String,
}

type GlobalSearchCacheKey = (u64, usize);
type GlobalSearchCacheValue = Arc<Vec<GlobalResourceSearchEntry>>;

#[allow(clippy::type_complexity)]
static GLOBAL_SEARCH_CACHE: LazyLock<
    Mutex<Option<(GlobalSearchCacheKey, GlobalSearchCacheValue)>>,
> = LazyLock::new(|| Mutex::new(None));

pub fn collect_global_resource_search_entries(
    snapshot: &ClusterSnapshot,
) -> GlobalSearchCacheValue {
    let key = (
        snapshot.snapshot_version,
        std::ptr::from_ref(snapshot) as usize,
    );
    {
        let guard = GLOBAL_SEARCH_CACHE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((cached_key, entries)) = guard.as_ref()
            && *cached_key == key
        {
            return Arc::clone(entries);
        }
    }

    let entries = Arc::new(build_global_resource_search_entries(snapshot));
    {
        let mut guard = GLOBAL_SEARCH_CACHE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *guard = Some((key, Arc::clone(&entries)));
    }
    entries
}

pub fn collect_extension_resource_search_entries(
    crd: &CustomResourceDefinitionInfo,
    instances: &[CustomResourceInfo],
) -> Vec<GlobalResourceSearchEntry> {
    let mut entries = Vec::with_capacity(instances.len());

    for item in instances {
        let resource = ResourceRef::CustomResource {
            name: item.name.clone(),
            namespace: item.namespace.clone(),
            group: crd.group.clone(),
            version: crd.version.clone(),
            kind: crd.kind.clone(),
            plural: crd.plural.clone(),
        };
        let mut aliases = base_aliases(&resource, AppView::Extensions);
        aliases.extend([
            crd.name.to_ascii_lowercase(),
            crd.group.to_ascii_lowercase(),
            crd.version.to_ascii_lowercase(),
            crd.kind.to_ascii_lowercase(),
            crd.plural.to_ascii_lowercase(),
            format!("{}/{}", crd.group, crd.version).to_ascii_lowercase(),
            format!("{} {}", crd.kind, crd.group).to_ascii_lowercase(),
            format!("{}/{}", crd.plural, item.name).to_ascii_lowercase(),
        ]);
        if let Some(namespace) = item.namespace.as_deref() {
            aliases.push(format!("{namespace}/{}", crd.plural).to_ascii_lowercase());
            aliases.push(format!("{namespace}/{}/{}", crd.plural, item.name).to_ascii_lowercase());
            aliases.push(format!("{} {namespace}/{}", crd.kind, item.name).to_ascii_lowercase());
        }
        aliases.sort_unstable();
        aliases.dedup();

        let subtitle = match item.namespace.as_deref() {
            Some(namespace) => format!("{} · {} · {}", crd.kind, namespace, crd.group),
            None => format!("{} · {}", crd.kind, crd.group),
        };
        entries.push(GlobalResourceSearchEntry {
            resource,
            title: item.name.clone(),
            subtitle,
            aliases,
            badge_label: AppView::Extensions.label().to_string(),
        });
    }

    entries
}

fn build_global_resource_search_entries(
    snapshot: &ClusterSnapshot,
) -> Vec<GlobalResourceSearchEntry> {
    let mut entries = Vec::with_capacity(
        snapshot.nodes.len()
            + snapshot.pods.len()
            + snapshot.services.len()
            + snapshot.deployments.len()
            + snapshot.statefulsets.len()
            + snapshot.daemonsets.len()
            + snapshot.replicasets.len()
            + snapshot.replication_controllers.len()
            + snapshot.jobs.len()
            + snapshot.cronjobs.len()
            + snapshot.resource_quotas.len()
            + snapshot.limit_ranges.len()
            + snapshot.pod_disruption_budgets.len()
            + snapshot.endpoints.len()
            + snapshot.ingresses.len()
            + snapshot.ingress_classes.len()
            + snapshot.gateway_classes.len()
            + snapshot.gateways.len()
            + snapshot.http_routes.len()
            + snapshot.grpc_routes.len()
            + snapshot.reference_grants.len()
            + snapshot.network_policies.len()
            + snapshot.config_maps.len()
            + snapshot.secrets.len()
            + snapshot.hpas.len()
            + snapshot.priority_classes.len()
            + snapshot.pvcs.len()
            + snapshot.pvs.len()
            + snapshot.storage_classes.len()
            + snapshot.namespace_list.len()
            + snapshot.events.len()
            + snapshot.service_accounts.len()
            + snapshot.roles.len()
            + snapshot.role_bindings.len()
            + snapshot.cluster_roles.len()
            + snapshot.cluster_role_bindings.len()
            + snapshot.helm_releases.len()
            + snapshot.flux_resources.len(),
    );

    for item in &snapshot.nodes {
        push_cluster_entry(
            &mut entries,
            ResourceRef::Node(item.name.clone()),
            AppView::Nodes,
            None,
            labels_from_pairs([("role", item.role.as_str())]),
        );
    }
    for item in &snapshot.pods {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::Pod(item.name.clone(), item.namespace.clone()),
            AppView::Pods,
            item.namespace.as_str(),
            pair_aliases(&item.labels),
        );
    }
    for item in &snapshot.services {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::Service(item.name.clone(), item.namespace.clone()),
            AppView::Services,
            item.namespace.as_str(),
            map_aliases(&item.labels),
        );
    }
    for item in &snapshot.deployments {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::Deployment(item.name.clone(), item.namespace.clone()),
            AppView::Deployments,
            item.namespace.as_str(),
            map_aliases(&item.pod_template_labels),
        );
    }
    for item in &snapshot.statefulsets {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::StatefulSet(item.name.clone(), item.namespace.clone()),
            AppView::StatefulSets,
            item.namespace.as_str(),
            map_aliases(&item.pod_template_labels),
        );
    }
    for item in &snapshot.daemonsets {
        let mut aliases = map_aliases(&item.labels);
        aliases.extend(map_aliases(&item.pod_template_labels));
        push_namespaced_entry(
            &mut entries,
            ResourceRef::DaemonSet(item.name.clone(), item.namespace.clone()),
            AppView::DaemonSets,
            item.namespace.as_str(),
            aliases,
        );
    }
    for item in &snapshot.replicasets {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::ReplicaSet(item.name.clone(), item.namespace.clone()),
            AppView::ReplicaSets,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.replication_controllers {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::ReplicationController(item.name.clone(), item.namespace.clone()),
            AppView::ReplicationControllers,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.jobs {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::Job(item.name.clone(), item.namespace.clone()),
            AppView::Jobs,
            item.namespace.as_str(),
            map_aliases(&item.pod_template_labels),
        );
    }
    for item in &snapshot.cronjobs {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::CronJob(item.name.clone(), item.namespace.clone()),
            AppView::CronJobs,
            item.namespace.as_str(),
            map_aliases(&item.pod_template_labels),
        );
    }
    for item in &snapshot.resource_quotas {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::ResourceQuota(item.name.clone(), item.namespace.clone()),
            AppView::ResourceQuotas,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.limit_ranges {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::LimitRange(item.name.clone(), item.namespace.clone()),
            AppView::LimitRanges,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.pod_disruption_budgets {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::PodDisruptionBudget(item.name.clone(), item.namespace.clone()),
            AppView::PodDisruptionBudgets,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.endpoints {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::Endpoint(item.name.clone(), item.namespace.clone()),
            AppView::Endpoints,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.ingresses {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::Ingress(item.name.clone(), item.namespace.clone()),
            AppView::Ingresses,
            item.namespace.as_str(),
            map_aliases(&item.labels),
        );
    }
    for item in &snapshot.ingress_classes {
        push_cluster_entry(
            &mut entries,
            ResourceRef::IngressClass(item.name.clone()),
            AppView::IngressClasses,
            None,
            Vec::new(),
        );
    }
    for item in &snapshot.gateway_classes {
        push_cluster_entry(
            &mut entries,
            ResourceRef::CustomResource {
                name: item.name.clone(),
                namespace: None,
                group: "gateway.networking.k8s.io".to_string(),
                version: item.version.clone(),
                kind: "GatewayClass".to_string(),
                plural: "gatewayclasses".to_string(),
            },
            AppView::GatewayClasses,
            None,
            Vec::new(),
        );
    }
    for item in &snapshot.gateways {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::CustomResource {
                name: item.name.clone(),
                namespace: Some(item.namespace.clone()),
                group: "gateway.networking.k8s.io".to_string(),
                version: item.version.clone(),
                kind: "Gateway".to_string(),
                plural: "gateways".to_string(),
            },
            AppView::Gateways,
            item.namespace.as_str(),
            map_aliases(&item.labels),
        );
    }
    for item in &snapshot.http_routes {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::CustomResource {
                name: item.name.clone(),
                namespace: Some(item.namespace.clone()),
                group: "gateway.networking.k8s.io".to_string(),
                version: item.version.clone(),
                kind: "HTTPRoute".to_string(),
                plural: "httproutes".to_string(),
            },
            AppView::HttpRoutes,
            item.namespace.as_str(),
            map_aliases(&item.labels),
        );
    }
    for item in &snapshot.grpc_routes {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::CustomResource {
                name: item.name.clone(),
                namespace: Some(item.namespace.clone()),
                group: "gateway.networking.k8s.io".to_string(),
                version: item.version.clone(),
                kind: "GRPCRoute".to_string(),
                plural: "grpcroutes".to_string(),
            },
            AppView::GrpcRoutes,
            item.namespace.as_str(),
            map_aliases(&item.labels),
        );
    }
    for item in &snapshot.reference_grants {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::CustomResource {
                name: item.name.clone(),
                namespace: Some(item.namespace.clone()),
                group: "gateway.networking.k8s.io".to_string(),
                version: item.version.clone(),
                kind: "ReferenceGrant".to_string(),
                plural: "referencegrants".to_string(),
            },
            AppView::ReferenceGrants,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.network_policies {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::NetworkPolicy(item.name.clone(), item.namespace.clone()),
            AppView::NetworkPolicies,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.config_maps {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::ConfigMap(item.name.clone(), item.namespace.clone()),
            AppView::ConfigMaps,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.secrets {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::Secret(item.name.clone(), item.namespace.clone()),
            AppView::Secrets,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.hpas {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::Hpa(item.name.clone(), item.namespace.clone()),
            AppView::HPAs,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.priority_classes {
        push_cluster_entry(
            &mut entries,
            ResourceRef::PriorityClass(item.name.clone()),
            AppView::PriorityClasses,
            None,
            Vec::new(),
        );
    }
    for item in &snapshot.pvcs {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::Pvc(item.name.clone(), item.namespace.clone()),
            AppView::PersistentVolumeClaims,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.pvs {
        push_cluster_entry(
            &mut entries,
            ResourceRef::Pv(item.name.clone()),
            AppView::PersistentVolumes,
            None,
            Vec::new(),
        );
    }
    for item in &snapshot.storage_classes {
        push_cluster_entry(
            &mut entries,
            ResourceRef::StorageClass(item.name.clone()),
            AppView::StorageClasses,
            None,
            Vec::new(),
        );
    }
    for item in &snapshot.namespace_list {
        push_cluster_entry(
            &mut entries,
            ResourceRef::Namespace(item.name.clone()),
            AppView::Namespaces,
            None,
            map_aliases(&item.labels),
        );
    }
    for item in &snapshot.events {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::Event(item.name.clone(), item.namespace.clone()),
            AppView::Events,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.service_accounts {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::ServiceAccount(item.name.clone(), item.namespace.clone()),
            AppView::ServiceAccounts,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.roles {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::Role(item.name.clone(), item.namespace.clone()),
            AppView::Roles,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.role_bindings {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::RoleBinding(item.name.clone(), item.namespace.clone()),
            AppView::RoleBindings,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.cluster_roles {
        push_cluster_entry(
            &mut entries,
            ResourceRef::ClusterRole(item.name.clone()),
            AppView::ClusterRoles,
            None,
            Vec::new(),
        );
    }
    for item in &snapshot.cluster_role_bindings {
        push_cluster_entry(
            &mut entries,
            ResourceRef::ClusterRoleBinding(item.name.clone()),
            AppView::ClusterRoleBindings,
            None,
            Vec::new(),
        );
    }
    for item in &snapshot.helm_releases {
        push_namespaced_entry(
            &mut entries,
            ResourceRef::HelmRelease(item.name.clone(), item.namespace.clone()),
            AppView::HelmReleases,
            item.namespace.as_str(),
            Vec::new(),
        );
    }
    for item in &snapshot.flux_resources {
        let resource = ResourceRef::CustomResource {
            name: item.name.clone(),
            namespace: item.namespace.clone(),
            group: item.group.clone(),
            version: item.version.clone(),
            kind: item.kind.clone(),
            plural: item.plural.clone(),
        };
        let Some(view) = resource.primary_view() else {
            continue;
        };
        push_flux_entry(&mut entries, resource, view);
    }

    entries
}

fn push_flux_entry(
    entries: &mut Vec<GlobalResourceSearchEntry>,
    resource: ResourceRef,
    view: AppView,
) {
    let kind = resource.kind().to_string();
    let name = resource.name().to_string();
    let namespace = resource.namespace().map(str::to_string);
    let mut aliases = base_aliases(&resource, view);
    if let Some(namespace) = namespace.as_deref() {
        aliases.push(format!("{kind} {namespace}/{name}").to_ascii_lowercase());
    }
    let subtitle = match namespace {
        Some(namespace) => format!("{kind} · {namespace}"),
        None => kind.to_string(),
    };
    entries.push(GlobalResourceSearchEntry {
        resource,
        title: name,
        subtitle,
        aliases,
        badge_label: view.label().to_string(),
    });
}

fn push_cluster_entry(
    entries: &mut Vec<GlobalResourceSearchEntry>,
    resource: ResourceRef,
    view: AppView,
    qualifier: Option<&str>,
    mut aliases: Vec<String>,
) {
    aliases.extend(base_aliases(&resource, view));
    if let Some(qualifier) = qualifier {
        aliases.push(qualifier.to_ascii_lowercase());
    }
    entries.push(GlobalResourceSearchEntry {
        title: resource.name().to_string(),
        subtitle: format!(
            "{}{}",
            resource.kind(),
            qualifier
                .map(|value| format!(" · {value}"))
                .unwrap_or_default()
        ),
        resource,
        aliases,
        badge_label: view.label().to_string(),
    });
}

fn push_namespaced_entry(
    entries: &mut Vec<GlobalResourceSearchEntry>,
    resource: ResourceRef,
    view: AppView,
    namespace: &str,
    mut aliases: Vec<String>,
) {
    aliases.extend(base_aliases(&resource, view));
    entries.push(GlobalResourceSearchEntry {
        title: resource.name().to_string(),
        subtitle: format!("{} · {namespace}", resource.kind()),
        resource,
        aliases,
        badge_label: view.label().to_string(),
    });
}

fn base_aliases(resource: &ResourceRef, view: AppView) -> Vec<String> {
    let mut aliases = vec![
        resource.kind().to_ascii_lowercase(),
        resource.name().to_ascii_lowercase(),
        view.label().to_ascii_lowercase(),
        format!("{} {}", resource.kind(), resource.name()).to_ascii_lowercase(),
    ];
    if let Some(namespace) = resource.namespace() {
        aliases.push(namespace.to_ascii_lowercase());
        aliases.push(format!("{namespace}/{}", resource.name()).to_ascii_lowercase());
        aliases.push(format!("{} {namespace}", resource.kind()).to_ascii_lowercase());
    }
    aliases
}

fn map_aliases(labels: &BTreeMap<String, String>) -> Vec<String> {
    labels
        .iter()
        .flat_map(|(key, value)| {
            [
                key.to_ascii_lowercase(),
                format!("{key}={value}").to_ascii_lowercase(),
                value.to_ascii_lowercase(),
            ]
        })
        .collect()
}

fn pair_aliases(labels: &[(String, String)]) -> Vec<String> {
    labels
        .iter()
        .flat_map(|(key, value)| {
            [
                key.to_ascii_lowercase(),
                format!("{key}={value}").to_ascii_lowercase(),
                value.to_ascii_lowercase(),
            ]
        })
        .collect()
}

fn labels_from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<String> {
    pairs
        .into_iter()
        .flat_map(|(key, value)| {
            [
                key.to_ascii_lowercase(),
                value.to_ascii_lowercase(),
                format!("{key}={value}").to_ascii_lowercase(),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        collect_extension_resource_search_entries, collect_global_resource_search_entries,
    };
    use crate::{
        app::ResourceRef,
        k8s::dtos::{
            CustomResourceDefinitionInfo, CustomResourceInfo, DeploymentInfo, FluxResourceInfo,
            NamespaceInfo, PodInfo,
        },
        state::ClusterSnapshot,
    };
    use std::{collections::BTreeMap, sync::Arc};

    #[test]
    fn global_search_indexes_namespaced_resources_and_labels() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.snapshot_version = 1;
        snapshot.pods.push(PodInfo {
            name: "api-0".into(),
            namespace: "prod".into(),
            labels: vec![
                ("app".into(), "api".into()),
                ("team".into(), "platform".into()),
            ],
            ..PodInfo::default()
        });

        let entries = collect_global_resource_search_entries(&snapshot);
        let entry = entries
            .iter()
            .find(|entry| entry.resource == ResourceRef::Pod("api-0".into(), "prod".into()))
            .expect("pod entry");

        assert_eq!(entry.title, "api-0");
        assert_eq!(entry.subtitle, "Pod · prod");
        assert!(entry.aliases.iter().any(|alias| alias == "app=api"));
        assert!(entry.aliases.iter().any(|alias| alias == "platform"));
    }

    #[test]
    fn global_search_indexes_cluster_scoped_resources() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.snapshot_version = 2;
        snapshot.namespace_list.push(NamespaceInfo {
            name: "platform".into(),
            labels: BTreeMap::from([("team".into(), "platform".into())]),
            ..NamespaceInfo::default()
        });
        snapshot.deployments.push(DeploymentInfo {
            name: "api".into(),
            namespace: "platform".into(),
            pod_template_labels: BTreeMap::from([("app".into(), "api".into())]),
            ..DeploymentInfo::default()
        });

        let entries = collect_global_resource_search_entries(&snapshot);

        assert!(entries.iter().any(|entry| {
            entry.resource == ResourceRef::Namespace("platform".into())
                && entry.aliases.iter().any(|alias| alias == "team=platform")
        }));
        assert!(entries.iter().any(|entry| {
            entry.resource == ResourceRef::Deployment("api".into(), "platform".into())
                && entry.aliases.iter().any(|alias| alias == "app=api")
        }));
    }

    #[test]
    fn global_search_flux_entries_include_kind_name_aliases() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.snapshot_version = 3;
        snapshot.flux_resources.push(FluxResourceInfo {
            name: "backend".into(),
            namespace: Some("flux-system".into()),
            kind: "HelmRelease".into(),
            group: "helm.toolkit.fluxcd.io".into(),
            version: "v2".into(),
            plural: "helmreleases".into(),
            ..FluxResourceInfo::default()
        });

        let entries = collect_global_resource_search_entries(&snapshot);
        let entry = entries
            .iter()
            .find(|entry| {
                entry.resource
                    == ResourceRef::CustomResource {
                        name: "backend".into(),
                        namespace: Some("flux-system".into()),
                        group: "helm.toolkit.fluxcd.io".into(),
                        version: "v2".into(),
                        kind: "HelmRelease".into(),
                        plural: "helmreleases".into(),
                    }
            })
            .expect("flux entry");

        assert!(
            entry
                .aliases
                .iter()
                .any(|alias| alias == "helmrelease backend")
        );
        assert!(
            entry
                .aliases
                .iter()
                .any(|alias| alias == "helmrelease flux-system/backend")
        );
    }

    #[test]
    fn global_search_cache_reuses_entries_for_same_snapshot_version() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.snapshot_version = 9;
        snapshot.pods.push(PodInfo {
            name: "api-0".into(),
            namespace: "prod".into(),
            ..PodInfo::default()
        });

        let first = collect_global_resource_search_entries(&snapshot);
        let second = collect_global_resource_search_entries(&snapshot);
        assert!(Arc::ptr_eq(&first, &second));

        snapshot.snapshot_version = 10;
        let third = collect_global_resource_search_entries(&snapshot);
        assert!(!Arc::ptr_eq(&first, &third));
    }

    #[test]
    fn extension_search_entries_include_crd_aliases_for_namespaced_instances() {
        let crd = CustomResourceDefinitionInfo {
            name: "widgets.demo.io".into(),
            group: "demo.io".into(),
            version: "v1".into(),
            kind: "Widget".into(),
            plural: "widgets".into(),
            scope: "Namespaced".into(),
            instances: 1,
        };
        let entries = collect_extension_resource_search_entries(
            &crd,
            &[CustomResourceInfo {
                name: "redis".into(),
                namespace: Some("prod".into()),
                ..CustomResourceInfo::default()
            }],
        );
        let entry = entries.first().expect("custom resource entry");

        assert_eq!(
            entry.resource,
            ResourceRef::CustomResource {
                name: "redis".into(),
                namespace: Some("prod".into()),
                group: "demo.io".into(),
                version: "v1".into(),
                kind: "Widget".into(),
                plural: "widgets".into(),
            }
        );
        assert_eq!(entry.title, "redis");
        assert_eq!(entry.subtitle, "Widget · prod · demo.io");
        assert!(entry.aliases.iter().any(|alias| alias == "widgets.demo.io"));
        assert!(
            entry
                .aliases
                .iter()
                .any(|alias| alias == "prod/widgets/redis")
        );
        assert!(
            entry
                .aliases
                .iter()
                .any(|alias| alias == "widget prod/redis")
        );
    }

    #[test]
    fn extension_search_entries_support_cluster_scoped_instances() {
        let crd = CustomResourceDefinitionInfo {
            name: "clusters.demo.io".into(),
            group: "demo.io".into(),
            version: "v1".into(),
            kind: "ClusterWidget".into(),
            plural: "clusterwidgets".into(),
            scope: "Cluster".into(),
            instances: 1,
        };
        let entries = collect_extension_resource_search_entries(
            &crd,
            &[CustomResourceInfo {
                name: "redis-global".into(),
                namespace: None,
                ..CustomResourceInfo::default()
            }],
        );
        let entry = entries.first().expect("cluster resource entry");

        assert_eq!(
            entry.subtitle, "ClusterWidget · demo.io",
            "cluster-scoped subtitle must omit namespace"
        );
        assert!(
            entry
                .aliases
                .iter()
                .any(|alias| alias == "clusterwidgets/redis-global")
        );
        assert!(
            entry
                .aliases
                .iter()
                .any(|alias| alias == "clusterwidget demo.io")
        );
    }
}
