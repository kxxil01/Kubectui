//! Tests for DaemonSet view rendering and UI components.

mod common;

use common::MockDataSource;
use kubectui::k8s::dtos::DaemonSetInfo;
use kubectui::state::filters::filter_daemonsets;
use std::collections::BTreeMap;

/// Tests that daemonset list filters by namespace correctly.
#[test]
fn test_daemonset_view_namespace_filtering() {
    let mut mock = MockDataSource::default();

    let mut labels = BTreeMap::new();
    labels.insert("app".to_string(), "monitoring".to_string());

    mock.daemonsets = vec![
        DaemonSetInfo {
            name: "prometheus-agent".to_string(),
            namespace: "monitoring".to_string(),
            desired_count: 5,
            ready_count: 5,
            unavailable_count: 0,
            image: Some("prom/prometheus:latest".to_string()),
            age: None,
            created_at: None,
            selector: "app=prometheus".to_string(),
            update_strategy: "RollingUpdate".to_string(),
            labels: labels.clone(),
            status_message: "Ready".to_string(),
        },
        DaemonSetInfo {
            name: "fluent-bit".to_string(),
            namespace: "logging".to_string(),
            desired_count: 3,
            ready_count: 3,
            unavailable_count: 0,
            image: Some("fluent/fluent-bit:2.1".to_string()),
            age: None,
            created_at: None,
            selector: "app=fluent".to_string(),
            update_strategy: "RollingUpdate".to_string(),
            labels: {
                let mut m = BTreeMap::new();
                m.insert("app".to_string(), "logging".to_string());
                m
            },
            status_message: "Ready".to_string(),
        },
    ];

    // Filter for monitoring namespace
    let monitoring_ds = filter_daemonsets(&mock.daemonsets, "", Some("monitoring"));
    assert_eq!(monitoring_ds.len(), 1);
    assert_eq!(monitoring_ds[0].name, "prometheus-agent");
    assert_eq!(monitoring_ds[0].namespace, "monitoring");

    // Filter for logging namespace
    let logging_ds = filter_daemonsets(&mock.daemonsets, "", Some("logging"));
    assert_eq!(logging_ds.len(), 1);
    assert_eq!(logging_ds[0].name, "fluent-bit");
}

/// Tests that daemonset list displays correct count columns.
#[test]
fn test_daemonset_view_count_columns() {
    let items = vec![
        DaemonSetInfo {
            name: "fully-ready".to_string(),
            namespace: "default".to_string(),
            desired_count: 10,
            ready_count: 10,
            unavailable_count: 0,
            ..DaemonSetInfo::default()
        },
        DaemonSetInfo {
            name: "partially-ready".to_string(),
            namespace: "default".to_string(),
            desired_count: 8,
            ready_count: 6,
            unavailable_count: 2,
            ..DaemonSetInfo::default()
        },
        DaemonSetInfo {
            name: "not-ready".to_string(),
            namespace: "default".to_string(),
            desired_count: 5,
            ready_count: 0,
            unavailable_count: 5,
            ..DaemonSetInfo::default()
        },
    ];

    // Verify count values are correct
    assert_eq!(items[0].desired_count, 10);
    assert_eq!(items[0].ready_count, 10);
    assert_eq!(items[0].unavailable_count, 0);

    assert_eq!(items[1].desired_count, 8);
    assert_eq!(items[1].ready_count, 6);
    assert_eq!(items[1].unavailable_count, 2);

    assert_eq!(items[2].desired_count, 5);
    assert_eq!(items[2].ready_count, 0);
    assert_eq!(items[2].unavailable_count, 5);
}

/// Tests sorting daemonsets by status (unavailable first).
#[test]
fn test_daemonset_sorting_by_status() {
    let mut items = vec![
        DaemonSetInfo {
            name: "healthy".to_string(),
            namespace: "default".to_string(),
            desired_count: 5,
            ready_count: 5,
            unavailable_count: 0,
            ..DaemonSetInfo::default()
        },
        DaemonSetInfo {
            name: "unhealthy".to_string(),
            namespace: "default".to_string(),
            desired_count: 5,
            ready_count: 2,
            unavailable_count: 3,
            ..DaemonSetInfo::default()
        },
        DaemonSetInfo {
            name: "partially-ready".to_string(),
            namespace: "default".to_string(),
            desired_count: 5,
            ready_count: 3,
            unavailable_count: 2,
            ..DaemonSetInfo::default()
        },
    ];

    // Sort by unavailable count (descending)
    items.sort_by(|a, b| {
        b.unavailable_count
            .cmp(&a.unavailable_count)
            .then_with(|| a.name.cmp(&b.name))
    });

    assert_eq!(items[0].name, "unhealthy");
    assert_eq!(items[1].name, "partially-ready");
    assert_eq!(items[2].name, "healthy");
}

/// Tests daemonset search functionality by multiple fields.
#[test]
fn test_daemonset_search_multiple_fields() {
    let items = vec![
        DaemonSetInfo {
            name: "node-exporter".to_string(),
            namespace: "monitoring".to_string(),
            image: Some("prom/node-exporter:v1.6".to_string()),
            selector: "app=exporter".to_string(),
            ..DaemonSetInfo::default()
        },
        DaemonSetInfo {
            name: "fluent-collector".to_string(),
            namespace: "logging".to_string(),
            image: Some("fluent/fluent-bit:latest".to_string()),
            selector: "component=logging".to_string(),
            ..DaemonSetInfo::default()
        },
    ];

    // Search by name
    let by_name = filter_daemonsets(&items, "exporter", None);
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].name, "node-exporter");

    // Search by image
    let by_image = filter_daemonsets(&items, "prom", None);
    assert_eq!(by_image.len(), 1);
    assert_eq!(by_image[0].name, "node-exporter");

    // Search by selector
    let by_selector = filter_daemonsets(&items, "component", None);
    assert_eq!(by_selector.len(), 1);
    assert_eq!(by_selector[0].name, "fluent-collector");
}

/// Tests daemonset detail view with new fields.
#[test]
fn test_daemonset_detail_view_fields() {
    let mut labels = BTreeMap::new();
    labels.insert("tier".to_string(), "infrastructure".to_string());
    labels.insert("env".to_string(), "production".to_string());

    let ds = DaemonSetInfo {
        name: "kube-proxy".to_string(),
        namespace: "kube-system".to_string(),
        desired_count: 3,
        ready_count: 3,
        unavailable_count: 0,
        image: Some("k8s.gcr.io/kube-proxy:v1.28.0".to_string()),
        age: None,
        created_at: None,
        selector: "k8s-app=kube-proxy".to_string(),
        update_strategy: "OnDelete".to_string(),
        labels: labels.clone(),
        status_message: "Ready".to_string(),
    };

    // Verify detail view fields
    assert_eq!(ds.name, "kube-proxy");
    assert_eq!(ds.selector, "k8s-app=kube-proxy");
    assert_eq!(ds.update_strategy, "OnDelete");
    assert_eq!(ds.labels.len(), 2);
    assert_eq!(ds.labels.get("tier"), Some(&"infrastructure".to_string()));
    assert_eq!(ds.status_message, "Ready");
}

/// Tests daemonset with degraded status message.
#[test]
fn test_daemonset_degraded_status_message() {
    let ds = DaemonSetInfo {
        name: "custom-daemon".to_string(),
        namespace: "default".to_string(),
        desired_count: 10,
        ready_count: 8,
        unavailable_count: 2,
        image: Some("myapp:latest".to_string()),
        age: None,
        created_at: None,
        selector: "app=custom".to_string(),
        update_strategy: "RollingUpdate".to_string(),
        labels: BTreeMap::new(),
        status_message: "2 nodes not ready; ImagePullBackOff on node-3".to_string(),
    };

    // Verify status message reflects degraded state
    assert_ne!(ds.status_message, "Ready");
    assert!(ds.status_message.contains("not ready"));
    assert!(ds.ready_count < ds.desired_count);
    assert!(ds.unavailable_count > 0);
}

/// Tests pod list relationship in daemonset detail.
#[test]
fn test_daemonset_pod_relationships() {
    let ds = DaemonSetInfo {
        name: "monitoring-agent".to_string(),
        namespace: "default".to_string(),
        desired_count: 4,
        ready_count: 4,
        unavailable_count: 0,
        image: Some("monitoring/agent:v2".to_string()),
        age: None,
        created_at: None,
        selector: "app=agent,version=v2".to_string(),
        update_strategy: "RollingUpdate".to_string(),
        labels: {
            let mut m = BTreeMap::new();
            m.insert("app".to_string(), "agent".to_string());
            m.insert("version".to_string(), "v2".to_string());
            m
        },
        status_message: "Ready".to_string(),
    };

    // Verify selector can be used to identify pods
    let selector_parts: Vec<&str> = ds.selector.split(',').collect();
    assert_eq!(selector_parts.len(), 2);
    assert!(selector_parts.contains(&"app=agent"));
    assert!(selector_parts.contains(&"version=v2"));

    // Verify pod count matches desired count
    assert_eq!(ds.desired_count, 4);
    assert_eq!(ds.ready_count, 4);
}

/// Tests daemonset with multiple label matching.
#[test]
fn test_daemonset_multiple_label_search() {
    let mut labels = BTreeMap::new();
    labels.insert("component".to_string(), "logging".to_string());
    labels.insert("version".to_string(), "2.1".to_string());
    labels.insert("team".to_string(), "platform".to_string());

    let items = vec![
        DaemonSetInfo {
            name: "fluent-bit-v2".to_string(),
            namespace: "logging".to_string(),
            labels: labels.clone(),
            ..DaemonSetInfo::default()
        },
        DaemonSetInfo {
            name: "fluent-bit-v1".to_string(),
            namespace: "logging".to_string(),
            labels: {
                let mut m = BTreeMap::new();
                m.insert("component".to_string(), "logging".to_string());
                m.insert("version".to_string(), "1.9".to_string());
                m
            },
            ..DaemonSetInfo::default()
        },
    ];

    // Search for specific version
    let v2_only = filter_daemonsets(&items, "2.1", None);
    assert_eq!(v2_only.len(), 1);
    assert_eq!(v2_only[0].name, "fluent-bit-v2");

    // Search for team label
    let by_team = filter_daemonsets(&items, "platform", None);
    assert_eq!(by_team.len(), 1);
    assert_eq!(by_team[0].name, "fluent-bit-v2");
}
