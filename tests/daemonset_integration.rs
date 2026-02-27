//! End-to-end integration tests for DaemonSet functionality.

mod common;

use common::MockDataSource;
use kubectui::k8s::dtos::DaemonSetInfo;
use kubectui::state::ClusterSnapshot;
use kubectui::state::filters::filter_daemonsets;
use std::collections::BTreeMap;

/// Tests complete daemonset workflow: fetch, filter, sort, render.
#[test]
fn test_daemonset_complete_workflow() {
    // Setup mock data source with multiple daemonsets
    let mut mock = MockDataSource::default();

    let mut monitoring_labels = BTreeMap::new();
    monitoring_labels.insert("managed-by".to_string(), "platform".to_string());

    mock.daemonsets = vec![
        DaemonSetInfo {
            name: "prometheus-exporter".to_string(),
            namespace: "monitoring".to_string(),
            desired_count: 10,
            ready_count: 10,
            unavailable_count: 0,
            image: Some("prom/node-exporter:v1.6".to_string()),
            age: None,
            created_at: None,
            selector: "app=prometheus-exporter".to_string(),
            update_strategy: "RollingUpdate".to_string(),
            labels: monitoring_labels.clone(),
            status_message: "Ready".to_string(),
        },
        DaemonSetInfo {
            name: "logging-collector".to_string(),
            namespace: "logging".to_string(),
            desired_count: 8,
            ready_count: 7,
            unavailable_count: 1,
            image: Some("fluent/fluent-bit:2.1".to_string()),
            age: None,
            created_at: None,
            selector: "app=fluent-bit,version=2.1".to_string(),
            update_strategy: "RollingUpdate".to_string(),
            labels: {
                let mut m = BTreeMap::new();
                m.insert("managed-by".to_string(), "platform".to_string());
                m
            },
            status_message: "1 node unavailable".to_string(),
        },
        DaemonSetInfo {
            name: "system-agent".to_string(),
            namespace: "kube-system".to_string(),
            desired_count: 3,
            ready_count: 3,
            unavailable_count: 0,
            image: Some("system/agent:stable".to_string()),
            age: None,
            created_at: None,
            selector: "component=system".to_string(),
            update_strategy: "OnDelete".to_string(),
            labels: BTreeMap::new(),
            status_message: "Ready".to_string(),
        },
    ];

    // Simulate filtering for monitoring namespace
    let monitoring_daemonsets = filter_daemonsets(&mock.daemonsets, "", Some("monitoring"));
    assert_eq!(monitoring_daemonsets.len(), 1);
    assert_eq!(monitoring_daemonsets[0].name, "prometheus-exporter");

    // Simulate filtering by search query
    let by_prom = filter_daemonsets(&mock.daemonsets, "prom", None);
    assert_eq!(by_prom.len(), 1);
    assert_eq!(by_prom[0].name, "prometheus-exporter");

    // Simulate sorting by readiness (unavailable first)
    let mut sorted = mock.daemonsets.clone();
    sorted.sort_by_key(|ds| (-ds.unavailable_count, ds.name.clone()));
    assert_eq!(sorted[0].name, "logging-collector");
    assert_eq!(sorted[0].unavailable_count, 1);
}

/// Tests daemonset rendering with snapshot data.
#[test]
fn test_daemonset_rendering_with_snapshot() {
    let mut snapshot = ClusterSnapshot::default();

    let mut labels = BTreeMap::new();
    labels.insert("tier".to_string(), "infrastructure".to_string());

    snapshot.daemonsets = vec![DaemonSetInfo {
        name: "kube-proxy".to_string(),
        namespace: "kube-system".to_string(),
        desired_count: 5,
        ready_count: 5,
        unavailable_count: 0,
        image: Some("k8s.gcr.io/kube-proxy:v1.28".to_string()),
        age: None,
        created_at: None,
        selector: "k8s-app=kube-proxy".to_string(),
        update_strategy: "RollingUpdate".to_string(),
        labels,
        status_message: "Ready".to_string(),
    }];

    // Verify snapshot contains daemonsets
    assert!(!snapshot.daemonsets.is_empty());
    assert_eq!(snapshot.daemonsets.len(), 1);

    // Verify daemonset details in snapshot
    let ds = &snapshot.daemonsets[0];
    assert_eq!(ds.name, "kube-proxy");
    assert_eq!(ds.namespace, "kube-system");
    assert_eq!(ds.desired_count, 5);
    assert_eq!(ds.ready_count, 5);
}

/// Tests namespace filtering with multiple daemonsets.
#[test]
fn test_daemonset_namespace_filtering_comprehensive() {
    let daemonsets = vec![
        DaemonSetInfo {
            name: "monitoring-1".to_string(),
            namespace: "monitoring".to_string(),
            desired_count: 5,
            ready_count: 5,
            ..DaemonSetInfo::default()
        },
        DaemonSetInfo {
            name: "monitoring-2".to_string(),
            namespace: "monitoring".to_string(),
            desired_count: 3,
            ready_count: 3,
            ..DaemonSetInfo::default()
        },
        DaemonSetInfo {
            name: "logging-1".to_string(),
            namespace: "logging".to_string(),
            desired_count: 4,
            ready_count: 4,
            ..DaemonSetInfo::default()
        },
        DaemonSetInfo {
            name: "system-1".to_string(),
            namespace: "kube-system".to_string(),
            desired_count: 2,
            ready_count: 2,
            ..DaemonSetInfo::default()
        },
    ];

    let monitoring = filter_daemonsets(&daemonsets, "", Some("monitoring"));
    assert_eq!(monitoring.len(), 2);

    let logging = filter_daemonsets(&daemonsets, "", Some("logging"));
    assert_eq!(logging.len(), 1);

    let kube_system = filter_daemonsets(&daemonsets, "", Some("kube-system"));
    assert_eq!(kube_system.len(), 1);

    let all = filter_daemonsets(&daemonsets, "", None);
    assert_eq!(all.len(), 4);
}

/// Tests complex search with labels and selectors.
#[test]
fn test_daemonset_complex_search() {
    let daemonsets = vec![
        DaemonSetInfo {
            name: "prometheus-node-exporter".to_string(),
            namespace: "monitoring".to_string(),
            image: Some("prom/node-exporter:v1.6.0".to_string()),
            selector: "app=node-exporter,version=v1".to_string(),
            labels: {
                let mut m = BTreeMap::new();
                m.insert("component".to_string(), "prometheus".to_string());
                m.insert("release".to_string(), "prometheus-stack".to_string());
                m
            },
            ..DaemonSetInfo::default()
        },
        DaemonSetInfo {
            name: "fluent-bit-collector".to_string(),
            namespace: "logging".to_string(),
            image: Some("fluent/fluent-bit:2.1.0".to_string()),
            selector: "app=fluent-bit".to_string(),
            labels: {
                let mut m = BTreeMap::new();
                m.insert("component".to_string(), "logging".to_string());
                m.insert("release".to_string(), "fluent-stack".to_string());
                m
            },
            ..DaemonSetInfo::default()
        },
    ];

    // Search by image registry
    let prom_results = filter_daemonsets(&daemonsets, "prom/", None);
    assert_eq!(prom_results.len(), 1);
    assert_eq!(prom_results[0].name, "prometheus-node-exporter");

    // Search by component label
    let prometheus_results = filter_daemonsets(&daemonsets, "prometheus", None);
    assert_eq!(prometheus_results.len(), 1);
    assert_eq!(prometheus_results[0].name, "prometheus-node-exporter");

    // Search by release label
    let stack_results = filter_daemonsets(&daemonsets, "stack", None);
    assert_eq!(stack_results.len(), 2);

    // Combined namespace and search
    let monitoring_exporter = filter_daemonsets(&daemonsets, "exporter", Some("monitoring"));
    assert_eq!(monitoring_exporter.len(), 1);
    assert_eq!(monitoring_exporter[0].name, "prometheus-node-exporter");
}

/// Tests daemonset update strategy representation.
#[test]
fn test_daemonset_update_strategies() {
    let daemonsets = vec![
        DaemonSetInfo {
            name: "rolling-update".to_string(),
            namespace: "default".to_string(),
            update_strategy: "RollingUpdate".to_string(),
            ..DaemonSetInfo::default()
        },
        DaemonSetInfo {
            name: "on-delete".to_string(),
            namespace: "default".to_string(),
            update_strategy: "OnDelete".to_string(),
            ..DaemonSetInfo::default()
        },
    ];

    assert_eq!(daemonsets[0].update_strategy, "RollingUpdate");
    assert_eq!(daemonsets[1].update_strategy, "OnDelete");

    // Verify strategies are preserved through filtering
    let filtered = filter_daemonsets(&daemonsets, "", None);
    assert_eq!(filtered[0].update_strategy, "RollingUpdate");
    assert_eq!(filtered[1].update_strategy, "OnDelete");
}

/// Tests daemonset selector label extraction.
#[test]
fn test_daemonset_selector_extraction() {
    let daemonsets = [
        DaemonSetInfo {
            name: "app-agent".to_string(),
            namespace: "default".to_string(),
            selector: "app=agent,tier=system".to_string(),
            ..DaemonSetInfo::default()
        },
        DaemonSetInfo {
            name: "no-selector".to_string(),
            namespace: "default".to_string(),
            selector: "-".to_string(),
            ..DaemonSetInfo::default()
        },
    ];

    // Verify selector parsing
    let agent_selectors: Vec<&str> = daemonsets[0].selector.split(',').collect();
    assert_eq!(agent_selectors.len(), 2);
    assert!(agent_selectors.contains(&"app=agent"));
    assert!(agent_selectors.contains(&"tier=system"));

    // Verify empty selector
    assert_eq!(daemonsets[1].selector, "-");
}

/// Tests daemonset status message accuracy.
#[test]
fn test_daemonset_status_messages() {
    let healthy_ds = DaemonSetInfo {
        name: "healthy".to_string(),
        namespace: "default".to_string(),
        desired_count: 10,
        ready_count: 10,
        unavailable_count: 0,
        status_message: "Ready".to_string(),
        ..DaemonSetInfo::default()
    };

    let degraded_ds = DaemonSetInfo {
        name: "degraded".to_string(),
        namespace: "default".to_string(),
        desired_count: 10,
        ready_count: 8,
        unavailable_count: 2,
        status_message: "ImagePullBackOff on 2 nodes".to_string(),
        ..DaemonSetInfo::default()
    };

    let failed_ds = DaemonSetInfo {
        name: "failed".to_string(),
        namespace: "default".to_string(),
        desired_count: 10,
        ready_count: 0,
        unavailable_count: 10,
        status_message: "CrashLoopBackOff; NodeNotReady".to_string(),
        ..DaemonSetInfo::default()
    };

    // Status messages should reflect actual state
    assert_eq!(healthy_ds.status_message, "Ready");
    assert!(degraded_ds.status_message.contains("ImagePullBackOff"));
    assert!(failed_ds.status_message.contains("CrashLoopBackOff"));
    assert!(failed_ds.status_message.contains("NodeNotReady"));
}

/// Tests daemonset label preservation and retrieval.
#[test]
fn test_daemonset_labels_preservation() {
    let mut labels = BTreeMap::new();
    labels.insert("app".to_string(), "monitoring".to_string());
    labels.insert("version".to_string(), "2.1".to_string());
    labels.insert("team".to_string(), "platform".to_string());

    let ds = DaemonSetInfo {
        name: "labeled-agent".to_string(),
        namespace: "default".to_string(),
        labels: labels.clone(),
        ..DaemonSetInfo::default()
    };

    // Verify labels are preserved
    assert_eq!(ds.labels.len(), 3);
    assert_eq!(ds.labels.get("app"), Some(&"monitoring".to_string()));
    assert_eq!(ds.labels.get("version"), Some(&"2.1".to_string()));
    assert_eq!(ds.labels.get("team"), Some(&"platform".to_string()));

    // Verify labels survive filtering
    let daemonsets = vec![ds];
    let filtered = filter_daemonsets(&daemonsets, "", None);
    assert_eq!(filtered[0].labels.len(), 3);
}
