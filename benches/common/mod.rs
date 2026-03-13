use chrono::Utc;
use kubectui::k8s::dtos::{
    ContainerMetrics, DeploymentInfo, NodeInfo, NodeMetricsInfo, PodInfo, PodMetricsInfo,
};
use kubectui::state::ClusterSnapshot;

pub fn make_test_snapshot(pod_count: usize) -> ClusterSnapshot {
    let namespaces = [
        "default",
        "kube-system",
        "monitoring",
        "production",
        "staging",
    ];
    let statuses = [
        "Running", "Running", "Running", "Running", "Pending", "Failed",
    ];
    let now = Utc::now();

    let pods: Vec<PodInfo> = (0..pod_count)
        .map(|i| {
            let has_requests = i % 2 == 0;
            PodInfo {
                name: format!("pod-{i}"),
                namespace: namespaces[i % namespaces.len()].to_string(),
                status: statuses[i % statuses.len()].to_string(),
                restarts: (i % 5) as i32,
                created_at: Some(now - chrono::Duration::seconds((i * 60) as i64)),
                cpu_request: if has_requests {
                    Some(format!("{}m", 100 + (i % 10) * 50))
                } else {
                    None
                },
                memory_request: if has_requests {
                    Some(format!("{}Mi", 64 + (i % 8) * 32))
                } else {
                    None
                },
                cpu_limit: if i % 3 == 0 {
                    Some(format!("{}m", 200 + (i % 10) * 100))
                } else {
                    None
                },
                memory_limit: if i % 3 == 0 {
                    Some(format!("{}Mi", 128 + (i % 8) * 64))
                } else {
                    None
                },
                ..Default::default()
            }
        })
        .collect();

    let node_count = (pod_count / 20).max(3);
    let nodes: Vec<NodeInfo> = (0..node_count)
        .map(|i| NodeInfo {
            name: format!("node-{i}"),
            ready: i % 10 != 0,
            kubelet_version: "v1.30.0".to_string(),
            os_image: "Ubuntu 22.04".to_string(),
            role: "worker".to_string(),
            cpu_allocatable: Some("4000m".to_string()),
            memory_allocatable: Some("8192Mi".to_string()),
            created_at: Some(now - chrono::Duration::hours(i as i64)),
            ..Default::default()
        })
        .collect();

    let node_metrics: Vec<NodeMetricsInfo> = (0..node_count)
        .map(|i| NodeMetricsInfo {
            name: format!("node-{i}"),
            cpu: format!("{}m", 1000 + (i % 6) * 500),
            memory: format!("{}Mi", 2048 + (i % 4) * 1024),
            ..Default::default()
        })
        .collect();

    let pod_metrics: Vec<PodMetricsInfo> = (0..pod_count)
        .filter(|i| statuses[i % statuses.len()] == "Running")
        .map(|i| PodMetricsInfo {
            name: format!("pod-{i}"),
            namespace: namespaces[i % namespaces.len()].to_string(),
            containers: vec![ContainerMetrics {
                name: "main".to_string(),
                cpu: format!("{}m", 10 + (i % 20) * 25),
                memory: format!("{}Mi", 16 + (i % 16) * 16),
            }],
            ..Default::default()
        })
        .collect();

    let deployments: Vec<DeploymentInfo> = (0..pod_count / 10)
        .map(|i| DeploymentInfo {
            name: format!("deploy-{i}"),
            namespace: namespaces[i % namespaces.len()].to_string(),
            ready_replicas: ((i % 3) + 1) as i32,
            desired_replicas: ((i % 3) + 1) as i32,
            created_at: Some(now - chrono::Duration::hours(i as i64)),
            ..Default::default()
        })
        .collect();

    ClusterSnapshot {
        snapshot_version: 1,
        pods,
        nodes,
        node_metrics,
        pod_metrics,
        deployments,
        namespaces_count: namespaces.len(),
        ..Default::default()
    }
}
