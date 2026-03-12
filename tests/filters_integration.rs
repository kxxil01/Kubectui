//! Integration tests for end-to-end filter workflows.

mod common;

use std::time::Instant;

use common::{make_deployment, make_node, make_service};
use kubectui::ui::{
    views::deployments::{DeploymentHealth, deployment_health_from_ready},
    views::filtering::{
        filtered_deployment_indices, filtered_node_indices, filtered_service_indices,
    },
};

/// Validates node query matching uses production filtering.
#[test]
#[ignore = "Optional integration run"]
fn filters_nodes_by_query() {
    let nodes = vec![
        make_node("master-ready", true, "master"),
        make_node("master-notready", false, "master"),
        make_node("worker-ready", true, "worker"),
    ];

    let out = filtered_node_indices(&nodes, "MASTER", None);

    assert_eq!(out.len(), 2);
    assert_eq!(nodes[out[0]].name, "master-ready");
    assert_eq!(nodes[out[1]].name, "master-notready");
}

/// Validates service query matches namespace and type through the production path.
#[test]
#[ignore = "Optional integration run"]
fn filters_services_across_fields() {
    let services = vec![
        make_service("api", "prod", "ClusterIP"),
        make_service("api", "dev", "ClusterIP"),
        make_service("api-lb", "prod", "LoadBalancer"),
    ];

    let prod = filtered_service_indices(&services, "prod", None);
    assert_eq!(prod.len(), 2);

    let prod_lb = filtered_service_indices(&services, "LoadBalancer", None);
    assert_eq!(prod_lb.len(), 1);
}

/// Verifies deployment health mapping stays aligned with the render path.
#[test]
#[ignore = "Optional integration run"]
fn deployment_health_classification_matches_expected() {
    let deployments = vec![
        make_deployment("api", "prod", "3/3"),
        make_deployment("api", "dev", "1/3"),
        make_deployment("api", "qa", "0/3"),
    ];

    let out = filtered_deployment_indices(&deployments, "api", None);
    assert_eq!(out.len(), 3);

    assert_eq!(
        deployment_health_from_ready(&deployments[out[0]].ready),
        DeploymentHealth::Healthy
    );
    assert_eq!(
        deployment_health_from_ready(&deployments[out[1]].ready),
        DeploymentHealth::Degraded
    );
    assert_eq!(
        deployment_health_from_ready(&deployments[out[2]].ready),
        DeploymentHealth::Failed
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
    let out = filtered_node_indices(&items, "worker-9", None);
    let elapsed = started.elapsed();

    assert!(!out.is_empty());
    assert!(
        elapsed.as_millis() < 100,
        "expected <100ms, got {}ms",
        elapsed.as_millis()
    );
}
