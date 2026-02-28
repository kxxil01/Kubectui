//! Optional performance checks for core pure functions.

mod common;

use std::time::Instant;

use common::{make_node, make_pod, make_service};
use kubectui::{
    app::{AppState, AppView},
    k8s::dtos::{
        ClusterRoleBindingInfo, ClusterRoleInfo, CronJobInfo, DaemonSetInfo, DeploymentInfo,
        JobInfo, LimitRangeInfo, LimitSpec, NodeInfo, PodDisruptionBudgetInfo, PodInfo,
        ReplicaSetInfo, ReplicationControllerInfo, ResourceQuotaInfo, RoleBindingInfo, RoleInfo,
        ServiceAccountInfo, ServiceInfo, StatefulSetInfo,
    },
    state::{
        ClusterSnapshot,
        alerts::{compute_alerts, compute_dashboard_insights, compute_workload_ready_percent},
        filters::{NodeRoleFilter, NodeStatusFilter, filter_nodes, filter_services},
    },
    ui,
};
use ratatui::{Terminal, backend::TestBackend};

/// Verifies filtering 10k nodes stays under 100ms.
#[test]
#[ignore = "Optional performance run"]
fn benchmark_filter_10k_nodes_under_100ms() {
    let nodes = (0..10_000)
        .map(|i| make_node(&format!("worker-{i}"), i % 2 == 0, "worker"))
        .collect::<Vec<_>>();

    let start = Instant::now();
    let _ = filter_nodes(
        &nodes,
        "worker-99",
        Some(NodeStatusFilter::Ready),
        Some(NodeRoleFilter::Worker),
    );
    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 100, "{}ms", elapsed.as_millis());
}

/// Verifies filtering 1k services stays under 50ms.
#[test]
#[ignore = "Optional performance run"]
fn benchmark_filter_1k_services_under_50ms() {
    let svcs = (0..1_000)
        .map(|i| {
            make_service(
                &format!("svc-{i}"),
                if i % 2 == 0 { "prod" } else { "dev" },
                "ClusterIP",
            )
        })
        .collect::<Vec<_>>();

    let start = Instant::now();
    let _ = filter_services(&svcs, "svc-9", Some("prod"), Some("ClusterIP"));
    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 50, "{}ms", elapsed.as_millis());
}

/// Verifies computing alerts from 1k pods stays under 50ms.
#[test]
#[ignore = "Optional performance run"]
fn benchmark_compute_alerts_1k_pods_under_50ms() {
    let mut snapshot = ClusterSnapshot::default();
    for i in 0..1_000 {
        snapshot.pods.push(make_pod(
            &format!("pod-{i}"),
            "default",
            if i % 3 == 0 { "Failed" } else { "Running" },
        ));
    }

    let start = Instant::now();
    let _ = compute_alerts(&snapshot);
    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 50, "{}ms", elapsed.as_millis());
}

/// Verifies dashboard insights for 1k nodes stay under 40ms.
#[test]
#[ignore = "Optional performance run"]
fn benchmark_compute_dashboard_insights_1k_nodes_under_40ms() {
    let mut snapshot = ClusterSnapshot::default();
    for i in 0..1_000 {
        snapshot.nodes.push(NodeInfo {
            name: format!("node-{i:04}"),
            ready: i % 10 != 0,
            cpu_allocatable: Some("2000m".to_string()),
            memory_allocatable: Some("4096Mi".to_string()),
            ..NodeInfo::default()
        });
        snapshot
            .node_metrics
            .push(kubectui::k8s::dtos::NodeMetricsInfo {
                name: format!("node-{i:04}"),
                cpu: format!("{}m", 600 + (i % 1000)),
                memory: format!("{}Mi", 1024 + (i % 2048)),
                ..kubectui::k8s::dtos::NodeMetricsInfo::default()
            });
    }

    let start = Instant::now();
    let _ = compute_dashboard_insights(&snapshot);
    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 40, "{}ms", elapsed.as_millis());
}

/// Verifies workload readiness computation remains lightweight.
#[test]
#[ignore = "Optional performance run"]
fn benchmark_compute_workload_ready_percent_2k_workloads_under_10ms() {
    let mut snapshot = ClusterSnapshot::default();
    for i in 0..2_000 {
        snapshot.deployments.push(DeploymentInfo {
            name: format!("deploy-{i:04}"),
            namespace: "default".to_string(),
            desired_replicas: 3,
            ready_replicas: if i % 5 == 0 { 2 } else { 3 },
            ..DeploymentInfo::default()
        });
    }

    let start = Instant::now();
    let _ = compute_workload_ready_percent(&snapshot);
    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 10, "{}ms", elapsed.as_millis());
}

/// Verifies tab switching on AppState is very fast.
#[test]
#[ignore = "Optional performance run"]
fn benchmark_tab_switch_under_10ms() {
    let mut app = AppState::default();

    let start = Instant::now();
    for _ in 0..1_000 {
        app.handle_key_event(crossterm::event::KeyEvent::from(
            crossterm::event::KeyCode::Tab,
        ));
    }
    let elapsed = start.elapsed();

    assert!(elapsed.as_millis() < 10, "{}ms", elapsed.as_millis());
}

/// Verifies search keystroke routing is under 5ms for 1k chars.
#[test]
#[ignore = "Optional performance run"]
fn benchmark_search_keystroke_under_5ms() {
    let mut app = AppState::default();
    app.handle_key_event(crossterm::event::KeyEvent::from(
        crossterm::event::KeyCode::Char('/'),
    ));

    let start = Instant::now();
    for _ in 0..1_000 {
        app.handle_key_event(crossterm::event::KeyEvent::from(
            crossterm::event::KeyCode::Char('a'),
        ));
    }
    let elapsed = start.elapsed();

    assert!(elapsed.as_millis() < 5, "{}ms", elapsed.as_millis());
}

/// Generates folded stacks and per-view frame-time summary for render path.
#[test]
#[ignore = "Optional profiling run"]
fn profile_render_path_and_emit_reports() {
    let mut snapshot = ClusterSnapshot {
        snapshot_version: 1,
        ..ClusterSnapshot::default()
    };

    for i in 0..1200 {
        let ns = if i % 2 == 0 { "prod" } else { "dev" };
        snapshot.nodes.push(NodeInfo {
            name: format!("node-{i:04}"),
            role: if i % 3 == 0 { "master" } else { "worker" }.to_string(),
            ready: i % 5 != 0,
            ..NodeInfo::default()
        });
        snapshot.pods.push(PodInfo {
            name: format!("pod-{i:04}"),
            namespace: ns.to_string(),
            status: if i % 9 == 0 { "Failed" } else { "Running" }.to_string(),
            ..PodInfo::default()
        });
        snapshot.services.push(ServiceInfo {
            name: format!("svc-{i:04}"),
            namespace: ns.to_string(),
            type_: if i % 4 == 0 { "NodePort" } else { "ClusterIP" }.to_string(),
            service_type: if i % 4 == 0 { "NodePort" } else { "ClusterIP" }.to_string(),
            ports: vec!["80/TCP".to_string(), "443/TCP".to_string()],
            ..ServiceInfo::default()
        });
        snapshot.deployments.push(DeploymentInfo {
            name: format!("deploy-{i:04}"),
            namespace: ns.to_string(),
            ready: if i % 7 == 0 { "0/3" } else { "3/3" }.to_string(),
            ..DeploymentInfo::default()
        });
        snapshot.statefulsets.push(StatefulSetInfo {
            name: format!("stateful-{i:04}"),
            namespace: ns.to_string(),
            desired_replicas: 3,
            ready_replicas: if i % 6 == 0 { 2 } else { 3 },
            service_name: "db".to_string(),
            image: Some("postgres:16".to_string()),
            ..StatefulSetInfo::default()
        });
        snapshot.daemonsets.push(DaemonSetInfo {
            name: format!("daemon-{i:04}"),
            namespace: ns.to_string(),
            desired_count: 10,
            ready_count: if i % 10 == 0 { 7 } else { 10 },
            unavailable_count: if i % 10 == 0 { 3 } else { 0 },
            ..DaemonSetInfo::default()
        });
        snapshot.jobs.push(JobInfo {
            name: format!("job-{i:04}"),
            namespace: ns.to_string(),
            status: if i % 8 == 0 { "Failed" } else { "Running" }.to_string(),
            ..JobInfo::default()
        });
        snapshot.cronjobs.push(CronJobInfo {
            name: format!("cron-{i:04}"),
            namespace: ns.to_string(),
            schedule: "*/5 * * * *".to_string(),
            ..CronJobInfo::default()
        });
        snapshot.replicasets.push(ReplicaSetInfo {
            name: format!("rs-{i:04}"),
            namespace: ns.to_string(),
            desired: 3,
            ready: if i % 11 == 0 { 2 } else { 3 },
            available: if i % 11 == 0 { 2 } else { 3 },
            ..ReplicaSetInfo::default()
        });
        snapshot
            .replication_controllers
            .push(ReplicationControllerInfo {
                name: format!("rc-{i:04}"),
                namespace: ns.to_string(),
                desired: 3,
                ready: if i % 11 == 0 { 2 } else { 3 },
                available: if i % 11 == 0 { 2 } else { 3 },
                ..ReplicationControllerInfo::default()
            });
    }

    for i in 0..500 {
        let ns = if i % 2 == 0 { "prod" } else { "dev" };
        snapshot.resource_quotas.push(ResourceQuotaInfo {
            name: format!("quota-{i:04}"),
            namespace: ns.to_string(),
            ..ResourceQuotaInfo::default()
        });
        snapshot.limit_ranges.push(LimitRangeInfo {
            name: format!("limits-{i:04}"),
            namespace: ns.to_string(),
            limits: vec![LimitSpec {
                type_: "Container".to_string(),
                ..LimitSpec::default()
            }],
            ..LimitRangeInfo::default()
        });
        snapshot
            .pod_disruption_budgets
            .push(PodDisruptionBudgetInfo {
                name: format!("pdb-{i:04}"),
                namespace: ns.to_string(),
                disruptions_allowed: if i % 9 == 0 { 0 } else { 2 },
                ..PodDisruptionBudgetInfo::default()
            });
        snapshot.service_accounts.push(ServiceAccountInfo {
            name: format!("sa-{i:04}"),
            namespace: ns.to_string(),
            ..ServiceAccountInfo::default()
        });
        snapshot.roles.push(RoleInfo {
            name: format!("role-{i:04}"),
            namespace: ns.to_string(),
            ..RoleInfo::default()
        });
        snapshot.role_bindings.push(RoleBindingInfo {
            name: format!("rb-{i:04}"),
            namespace: ns.to_string(),
            role_ref_kind: "Role".to_string(),
            role_ref_name: format!("role-{i:04}"),
            ..RoleBindingInfo::default()
        });
        snapshot.cluster_roles.push(ClusterRoleInfo {
            name: format!("cr-{i:04}"),
            ..ClusterRoleInfo::default()
        });
        snapshot.cluster_role_bindings.push(ClusterRoleBindingInfo {
            name: format!("crb-{i:04}"),
            role_ref_kind: "ClusterRole".to_string(),
            role_ref_name: format!("cr-{i:04}"),
            ..ClusterRoleBindingInfo::default()
        });
    }

    ui::profiling::set_enabled(true);
    ui::profiling::set_output_dir(std::path::PathBuf::from("target/profiles/tests"));

    let backend = TestBackend::new(180, 58);
    let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
    let mut app = AppState::default();

    for view in AppView::tabs() {
        app.view = *view;
        app.search_query = "prod".to_string();
        for _ in 0..20 {
            terminal
                .draw(|frame| ui::render(frame, &app, &snapshot))
                .expect("render should succeed");
        }
        app.search_query.clear();
        for _ in 0..20 {
            terminal
                .draw(|frame| ui::render(frame, &app, &snapshot))
                .expect("render should succeed");
        }
    }

    let paths = ui::profiling::write_report_if_enabled()
        .expect("profile report write should not fail")
        .expect("profiling should be enabled");
    assert!(paths.0.exists());
    assert!(paths.1.exists());
}
