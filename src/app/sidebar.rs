//! Sidebar navigation state: group/view layout, collapse cache, row enumeration.

use std::collections::HashSet;
use std::sync::LazyLock;

use super::views::{AppView, NavGroup};

/// A row in the sidebar — either a group header or a leaf view item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarItem {
    Group(NavGroup),
    View(AppView),
}

const OVERVIEW_VIEWS: &[AppView] = &[
    AppView::Dashboard,
    AppView::Projects,
    AppView::Governance,
    AppView::Bookmarks,
    AppView::Issues,
    AppView::HealthReport,
    AppView::Vulnerabilities,
    AppView::Nodes,
    AppView::Namespaces,
    AppView::Events,
];

const GROUPED_SIDEBAR_GROUPS: &[(NavGroup, &[AppView])] = &[
    (
        NavGroup::Workloads,
        &[
            AppView::Pods,
            AppView::Deployments,
            AppView::StatefulSets,
            AppView::DaemonSets,
            AppView::ReplicaSets,
            AppView::ReplicationControllers,
            AppView::Jobs,
            AppView::CronJobs,
        ],
    ),
    (
        NavGroup::Network,
        &[
            AppView::Services,
            AppView::Endpoints,
            AppView::Ingresses,
            AppView::IngressClasses,
            AppView::GatewayClasses,
            AppView::Gateways,
            AppView::HttpRoutes,
            AppView::GrpcRoutes,
            AppView::ReferenceGrants,
            AppView::NetworkPolicies,
            AppView::PortForwarding,
        ],
    ),
    (
        NavGroup::Config,
        &[
            AppView::ConfigMaps,
            AppView::Secrets,
            AppView::ResourceQuotas,
            AppView::LimitRanges,
            AppView::HPAs,
            AppView::PodDisruptionBudgets,
            AppView::PriorityClasses,
        ],
    ),
    (
        NavGroup::Storage,
        &[
            AppView::PersistentVolumeClaims,
            AppView::PersistentVolumes,
            AppView::StorageClasses,
        ],
    ),
    (
        NavGroup::Helm,
        &[AppView::HelmCharts, AppView::HelmReleases],
    ),
    (
        NavGroup::FluxCD,
        &[
            AppView::FluxCDAlertProviders,
            AppView::FluxCDAlerts,
            AppView::FluxCDAll,
            AppView::FluxCDArtifacts,
            AppView::FluxCDHelmReleases,
            AppView::FluxCDHelmRepositories,
            AppView::FluxCDImages,
            AppView::FluxCDKustomizations,
            AppView::FluxCDReceivers,
            AppView::FluxCDSources,
        ],
    ),
    (
        NavGroup::AccessControl,
        &[
            AppView::ServiceAccounts,
            AppView::ClusterRoles,
            AppView::Roles,
            AppView::ClusterRoleBindings,
            AppView::RoleBindings,
        ],
    ),
    (NavGroup::CustomResources, &[AppView::Extensions]),
];

const fn nav_group_bit(group: NavGroup) -> u16 {
    match group {
        NavGroup::Overview => 0,
        NavGroup::Workloads => 1 << 0,
        NavGroup::Network => 1 << 1,
        NavGroup::Config => 1 << 2,
        NavGroup::Storage => 1 << 3,
        NavGroup::Helm => 1 << 4,
        NavGroup::FluxCD => 1 << 5,
        NavGroup::AccessControl => 1 << 6,
        NavGroup::CustomResources => 1 << 7,
    }
}

pub(crate) fn collapsed_mask(collapsed: &HashSet<NavGroup>) -> u16 {
    collapsed
        .iter()
        .fold(0u16, |mask, group| mask | nav_group_bit(*group))
}

static SIDEBAR_ROWS_CACHE: LazyLock<Vec<Box<[SidebarItem]>>> = LazyLock::new(|| {
    let num_groups = GROUPED_SIDEBAR_GROUPS.len();
    let combos = 1usize << num_groups;
    let mut cache = Vec::with_capacity(combos);
    for mask in 0u16..(combos as u16) {
        let mut rows = Vec::with_capacity(56);
        for view in OVERVIEW_VIEWS {
            rows.push(SidebarItem::View(*view));
        }
        for (group, views) in GROUPED_SIDEBAR_GROUPS {
            rows.push(SidebarItem::Group(*group));
            if mask & nav_group_bit(*group) == 0 {
                for view in *views {
                    rows.push(SidebarItem::View(*view));
                }
            }
        }
        cache.push(rows.into_boxed_slice());
    }
    cache
});

/// Ordered sidebar rows for the current collapsed state.
pub fn sidebar_rows(collapsed: &HashSet<NavGroup>) -> &'static [SidebarItem] {
    let mask = collapsed_mask(collapsed) as usize;
    &SIDEBAR_ROWS_CACHE[mask]
}

/// Returns the NavGroup that contains the given view, if any.
pub fn group_for_view(view: AppView) -> Option<NavGroup> {
    GROUPED_SIDEBAR_GROUPS
        .iter()
        .find(|(_, views)| views.contains(&view))
        .map(|(group, _)| *group)
        .or_else(|| OVERVIEW_VIEWS.contains(&view).then_some(NavGroup::Overview))
}

/// Returns all collapsible sidebar groups.
pub fn all_groups() -> impl Iterator<Item = NavGroup> {
    GROUPED_SIDEBAR_GROUPS.iter().map(|(g, _)| *g)
}
