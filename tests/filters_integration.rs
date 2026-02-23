//! Integration tests for end-to-end filter workflows.

mod common;

use std::time::Instant;

use common::{make_deployment, make_node, make_service};
use kubectui::state::filters::{
    DeploymentHealth, NodeRoleFilter, NodeStatusFilter, filter_deployments, filter_nodes,
    filter_services,
};

/// Validates multi-criteria node filtering workflow.
#[test]
#[ignore = "Optional integration run"]
fn filters_nodes_with_multiple_criteria() {
    let nodes = vec![
        make_node("master-ready", true, "master"),
        make_node("master-notready", false, "master"),
        make_node("worker-ready", true, "worker"),
    ];

    let out = filter_nodes(
        &nodes,
        "master",
        Some(NodeStatusFilter::Ready),
        Some(NodeRoleFilter::Master),
    );

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].name, "master-ready");
}

/// Validates service filtering across namespaces and types.
#[test]
#[ignore = "Optional integration run"]
fn filters_services_across_namespaces() {
    let services = vec![
        make_service("api", "prod", "ClusterIP"),
        make_service("api", "dev", "ClusterIP"),
        make_service("api-lb", "prod", "LoadBalancer"),
    ];

    let prod = filter_services(&services, "api", Some("prod"), None);
    assert_eq!(prod.len(), 2);

    let prod_lb = filter_services(&services, "api", Some("prod"), Some("LoadBalancer"));
    assert_eq!(prod_lb.len(), 1);
}

/// Verifies filtered result counts match expected values.
#[test]
#[ignore = "Optional integration run"]
fn filter_counts_match_expected() {
    let deployments = vec![
        make_deployment("api", "prod", "3/3"),
        make_deployment("api", "dev", "1/3"),
        make_deployment("api", "qa", "0/3"),
    ];

    assert_eq!(
        filter_deployments(&deployments, "api", None, Some(DeploymentHealth::Healthy)).len(),
        1
    );
    assert_eq!(
        filter_deployments(&deployments, "api", None, Some(DeploymentHealth::Degraded)).len(),
        1
    );
    assert_eq!(
        filter_deployments(&deployments, "api", None, Some(DeploymentHealth::Failed)).len(),
        1
    );
}

/// Verifies filtering 1000 items completes within 100ms target.
#[test]
#[ignore = "Optional integration run"]
fn filter_performance_1000_items_under_100ms() {
    let items = (0..1000)
        .map(|i| make_node(&format!("worker-{i}"), i % 2 == 0, "worker"))
        .collect::<Vec<_>>();

    let started = Instant::now();
    let out = filter_nodes(
        &items,
        "worker-9",
        Some(NodeStatusFilter::Ready),
        Some(NodeRoleFilter::Worker),
    );
    let elapsed = started.elapsed();

    assert!(!out.is_empty());
    assert!(
        elapsed.as_millis() < 100,
        "expected <100ms, got {}ms",
        elapsed.as_millis()
    );
}
