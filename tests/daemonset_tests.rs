//! Comprehensive tests for DaemonSet DTO and fetch operations.

use kubectui::k8s::dtos::DaemonSetInfo;
use kubectui::ui::views::filtering::filtered_daemonset_indices;
use std::collections::BTreeMap;

fn filtered_daemonsets<'a>(items: &'a [DaemonSetInfo], query: &str) -> Vec<&'a DaemonSetInfo> {
    filtered_daemonset_indices(items, query, None)
        .into_iter()
        .map(|idx| &items[idx])
        .collect()
}

/// Tests DaemonSetInfo DTO creation with all fields.
#[test]
fn test_daemonset_info_dto_complete() {
    let mut labels = BTreeMap::new();
    labels.insert("app".to_string(), "monitoring".to_string());
    labels.insert("version".to_string(), "v1".to_string());

    let ds = DaemonSetInfo {
        name: "node-exporter".to_string(),
        namespace: "monitoring".to_string(),
        desired_count: 10,
        ready_count: 10,
        unavailable_count: 0,
        image: Some("prom/node-exporter:latest".to_string()),
        age: None,
        created_at: None,
        selector: "app=node-exporter".to_string(),
        update_strategy: "RollingUpdate".to_string(),
        labels: labels.clone(),
        status_message: "Ready".to_string(),
    };

    assert_eq!(ds.name, "node-exporter");
    assert_eq!(ds.namespace, "monitoring");
    assert_eq!(ds.desired_count, 10);
    assert_eq!(ds.ready_count, 10);
    assert_eq!(ds.unavailable_count, 0);
    assert_eq!(ds.selector, "app=node-exporter");
    assert_eq!(ds.update_strategy, "RollingUpdate");
    assert_eq!(ds.labels.len(), 2);
    assert_eq!(ds.status_message, "Ready");
}

/// Tests DaemonSetInfo DTO with default values.
#[test]
fn test_daemonset_info_dto_defaults() {
    let ds = DaemonSetInfo::default();

    assert_eq!(ds.name, "");
    assert_eq!(ds.namespace, "");
    assert_eq!(ds.desired_count, 0);
    assert_eq!(ds.ready_count, 0);
    assert_eq!(ds.unavailable_count, 0);
    assert_eq!(ds.image, None);
    assert_eq!(ds.selector, "");
    assert_eq!(ds.update_strategy, "");
    assert!(ds.labels.is_empty());
    assert_eq!(ds.status_message, "");
}

/// Tests DaemonSetInfo serialization/deserialization.
#[test]
fn test_daemonset_info_serialization() {
    let mut labels = BTreeMap::new();
    labels.insert("tier".to_string(), "system".to_string());

    let original = DaemonSetInfo {
        name: "kube-proxy".to_string(),
        namespace: "kube-system".to_string(),
        desired_count: 3,
        ready_count: 3,
        unavailable_count: 0,
        image: Some("k8s.gcr.io/kube-proxy:v1.28.0".to_string()),
        age: None,
        created_at: None,
        selector: "k8s-app=kube-proxy".to_string(),
        update_strategy: "RollingUpdate".to_string(),
        labels: labels.clone(),
        status_message: "Ready".to_string(),
    };

    // Test serialization to JSON
    let serialized = serde_json::to_string(&original).expect("should serialize");
    let deserialized: DaemonSetInfo =
        serde_json::from_str(&serialized).expect("should deserialize");

    assert_eq!(original.name, deserialized.name);
    assert_eq!(original.namespace, deserialized.namespace);
    assert_eq!(original.desired_count, deserialized.desired_count);
    assert_eq!(original.selector, deserialized.selector);
    assert_eq!(original.update_strategy, deserialized.update_strategy);
    assert_eq!(original.labels, deserialized.labels);
}

/// Tests DaemonSetInfo with multiple labels.
#[test]
fn test_daemonset_info_multiple_labels() {
    let mut labels = BTreeMap::new();
    labels.insert("app".to_string(), "logging".to_string());
    labels.insert("version".to_string(), "2.1".to_string());
    labels.insert("team".to_string(), "platform".to_string());

    let ds = DaemonSetInfo {
        name: "fluent-bit".to_string(),
        namespace: "logging".to_string(),
        desired_count: 5,
        ready_count: 5,
        unavailable_count: 0,
        image: Some("fluent/fluent-bit:2.1".to_string()),
        age: None,
        created_at: None,
        selector: "app=fluent-bit,version=2.1".to_string(),
        update_strategy: "RollingUpdate".to_string(),
        labels,
        status_message: "Ready".to_string(),
    };

    assert_eq!(ds.labels.len(), 3);
    assert_eq!(ds.labels.get("app").unwrap(), "logging");
    assert_eq!(ds.labels.get("version").unwrap(), "2.1");
    assert_eq!(ds.labels.get("team").unwrap(), "platform");
}

/// Tests DaemonSetInfo with degraded status.
#[test]
fn test_daemonset_info_degraded_status() {
    let ds = DaemonSetInfo {
        name: "custom-agent".to_string(),
        namespace: "default".to_string(),
        desired_count: 8,
        ready_count: 6,
        unavailable_count: 2,
        image: Some("myrepo/agent:latest".to_string()),
        age: None,
        created_at: None,
        selector: "component=agent".to_string(),
        update_strategy: "RollingUpdate".to_string(),
        labels: BTreeMap::new(),
        status_message: "Some nodes are not ready".to_string(),
    };

    assert!(ds.ready_count < ds.desired_count);
    assert!(ds.unavailable_count > 0);
    assert_ne!(ds.status_message, "Ready");
}

/// Tests DaemonSetInfo with OnDelete update strategy.
#[test]
fn test_daemonset_info_ondelete_strategy() {
    let ds = DaemonSetInfo {
        name: "critical-service".to_string(),
        namespace: "infrastructure".to_string(),
        desired_count: 4,
        ready_count: 4,
        unavailable_count: 0,
        image: Some("critical/service:stable".to_string()),
        age: None,
        created_at: None,
        selector: "tier=critical".to_string(),
        update_strategy: "OnDelete".to_string(),
        labels: BTreeMap::new(),
        status_message: "Ready".to_string(),
    };

    assert_eq!(ds.update_strategy, "OnDelete");
}

/// Tests DaemonSetInfo selector parsing.
#[test]
fn test_daemonset_info_selector_parsing() {
    let ds = DaemonSetInfo {
        name: "test-ds".to_string(),
        namespace: "default".to_string(),
        desired_count: 1,
        ready_count: 1,
        unavailable_count: 0,
        image: None,
        age: None,
        created_at: None,
        selector: "env=prod,tier=backend".to_string(),
        update_strategy: "RollingUpdate".to_string(),
        labels: BTreeMap::new(),
        status_message: "Ready".to_string(),
    };

    let selector_parts: Vec<&str> = ds.selector.split(',').collect();
    assert_eq!(selector_parts.len(), 2);
    assert!(selector_parts.contains(&"env=prod"));
    assert!(selector_parts.contains(&"tier=backend"));
}

/// Tests filtering DaemonSets by label via selector.
#[test]
fn test_daemonset_selector_label_matching() {
    let items = vec![
        DaemonSetInfo {
            name: "prod-ds".to_string(),
            namespace: "default".to_string(),
            desired_count: 3,
            ready_count: 3,
            unavailable_count: 0,
            image: Some("myapp:prod".to_string()),
            age: None,
            created_at: None,
            selector: "env=prod".to_string(),
            update_strategy: "RollingUpdate".to_string(),
            labels: {
                let mut m = BTreeMap::new();
                m.insert("env".to_string(), "prod".to_string());
                m
            },
            status_message: "Ready".to_string(),
        },
        DaemonSetInfo {
            name: "dev-ds".to_string(),
            namespace: "default".to_string(),
            desired_count: 1,
            ready_count: 1,
            unavailable_count: 0,
            image: Some("myapp:dev".to_string()),
            age: None,
            created_at: None,
            selector: "env=dev".to_string(),
            update_strategy: "RollingUpdate".to_string(),
            labels: {
                let mut m = BTreeMap::new();
                m.insert("env".to_string(), "dev".to_string());
                m
            },
            status_message: "Ready".to_string(),
        },
    ];

    let filtered = filtered_daemonsets(&items, "prod");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "prod-ds");
}

/// Tests namespace filtering for DaemonSets.
#[test]
fn test_daemonset_namespace_filtering() {
    let items = vec![
        DaemonSetInfo {
            name: "monitoring-ds".to_string(),
            namespace: "monitoring".to_string(),
            ..DaemonSetInfo::default()
        },
        DaemonSetInfo {
            name: "logging-ds".to_string(),
            namespace: "logging".to_string(),
            ..DaemonSetInfo::default()
        },
    ];

    let filtered = filtered_daemonsets(&items, "monitoring");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].namespace, "monitoring");
}

/// Tests filtering DaemonSets by image name.
#[test]
fn test_daemonset_image_filtering() {
    let items = vec![
        DaemonSetInfo {
            name: "app-exporter".to_string(),
            namespace: "default".to_string(),
            image: Some("prom/node-exporter:v1.6".to_string()),
            ..DaemonSetInfo::default()
        },
        DaemonSetInfo {
            name: "app-collector".to_string(),
            namespace: "default".to_string(),
            image: Some("fluent/fluent-bit:latest".to_string()),
            ..DaemonSetInfo::default()
        },
    ];

    let filtered = filtered_daemonsets(&items, "prom");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "app-exporter");
}

/// Tests DaemonSet readiness calculation.
#[test]
fn test_daemonset_readiness_calculation() {
    let fully_ready = DaemonSetInfo {
        name: "ready".to_string(),
        namespace: "default".to_string(),
        desired_count: 5,
        ready_count: 5,
        unavailable_count: 0,
        ..DaemonSetInfo::default()
    };

    let partially_ready = DaemonSetInfo {
        name: "partial".to_string(),
        namespace: "default".to_string(),
        desired_count: 5,
        ready_count: 3,
        unavailable_count: 2,
        ..DaemonSetInfo::default()
    };

    let not_ready = DaemonSetInfo {
        name: "not_ready".to_string(),
        namespace: "default".to_string(),
        desired_count: 5,
        ready_count: 0,
        unavailable_count: 5,
        ..DaemonSetInfo::default()
    };

    assert_eq!(fully_ready.ready_count, fully_ready.desired_count);
    assert!(partially_ready.ready_count < partially_ready.desired_count);
    assert_eq!(not_ready.ready_count, 0);
}
