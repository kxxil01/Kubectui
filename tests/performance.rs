//! Optional performance checks for core pure functions.

mod common;

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Result;
use async_trait::async_trait;
use common::{make_node, make_pod, make_service};
use kubectui::{
    app::{AppState, AppView},
    k8s::dtos::{
        ClusterRoleBindingInfo, ClusterRoleInfo, CronJobInfo, DaemonSetInfo, DeploymentInfo,
        JobInfo, LimitRangeInfo, LimitSpec, NodeInfo, PodDisruptionBudgetInfo, PodInfo,
        ReplicaSetInfo, ReplicationControllerInfo, ResourceQuotaInfo, RoleBindingInfo, RoleInfo,
        ServiceAccountInfo, ServiceInfo, StatefulSetInfo, VulnerabilityReportInfo,
    },
    state::{
        ClusterDataSource, ClusterSnapshot, GlobalState, RefreshOptions, RefreshScope,
        alerts::{compute_alerts, compute_dashboard_insights, compute_workload_ready_percent},
    },
    ui::{
        self,
        views::filtering::{filtered_node_indices, filtered_service_indices},
    },
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
    let _ = filtered_node_indices(&nodes, "worker-99", None);
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
    let _ = filtered_service_indices(&svcs, "prod", None);
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

#[derive(Clone, Default)]
struct PerfCounters {
    total: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct PerfMockDataSource {
    counters: PerfCounters,
    delay_ms: u64,
}

impl PerfMockDataSource {
    fn new(delay_ms: u64) -> Self {
        Self {
            counters: PerfCounters::default(),
            delay_ms,
        }
    }

    fn bump(&self) {
        self.counters.total.fetch_add(1, Ordering::Relaxed);
    }

    async fn maybe_delay(&self) {
        if self.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        }
    }
}

#[async_trait]
impl ClusterDataSource for PerfMockDataSource {
    fn cluster_url(&self) -> &str {
        "https://perf.test"
    }

    fn cluster_context(&self) -> Option<&str> {
        Some("perf-context")
    }

    async fn fetch_nodes(&self) -> Result<Vec<NodeInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![make_node("node-a", true, "worker")])
    }

    async fn fetch_namespaces(&self) -> Result<Vec<String>> {
        Ok(vec!["default".to_string()])
    }

    async fn fetch_pods(&self, namespace: Option<&str>) -> Result<Vec<PodInfo>> {
        self.bump();
        self.maybe_delay().await;
        let namespace = namespace.unwrap_or("default");
        Ok(vec![make_pod("pod-a", namespace, "Running")])
    }

    async fn fetch_services(&self, namespace: Option<&str>) -> Result<Vec<ServiceInfo>> {
        self.bump();
        self.maybe_delay().await;
        let namespace = namespace.unwrap_or("default");
        Ok(vec![make_service("svc-a", namespace, "ClusterIP")])
    }

    async fn fetch_deployments(&self, namespace: Option<&str>) -> Result<Vec<DeploymentInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![DeploymentInfo {
            name: "deploy-a".to_string(),
            namespace: namespace.unwrap_or("default").to_string(),
            ready: "1/1".to_string(),
            desired_replicas: 1,
            ready_replicas: 1,
            ..DeploymentInfo::default()
        }])
    }

    async fn fetch_statefulsets(&self, namespace: Option<&str>) -> Result<Vec<StatefulSetInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![StatefulSetInfo {
            name: "db".to_string(),
            namespace: namespace.unwrap_or("default").to_string(),
            desired_replicas: 1,
            ready_replicas: 1,
            service_name: "db".to_string(),
            ..StatefulSetInfo::default()
        }])
    }

    async fn fetch_daemonsets(&self, namespace: Option<&str>) -> Result<Vec<DaemonSetInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![DaemonSetInfo {
            name: "agent".to_string(),
            namespace: namespace.unwrap_or("default").to_string(),
            desired_count: 1,
            ready_count: 1,
            unavailable_count: 0,
            ..DaemonSetInfo::default()
        }])
    }

    async fn fetch_replicasets(&self, _namespace: Option<&str>) -> Result<Vec<ReplicaSetInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_replication_controllers(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<ReplicationControllerInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_jobs(&self, namespace: Option<&str>) -> Result<Vec<JobInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![JobInfo {
            name: "job-a".to_string(),
            namespace: namespace.unwrap_or("default").to_string(),
            status: "Running".to_string(),
            completions: "0/1".to_string(),
            ..JobInfo::default()
        }])
    }

    async fn fetch_cronjobs(&self, namespace: Option<&str>) -> Result<Vec<CronJobInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![CronJobInfo {
            name: "cron-a".to_string(),
            namespace: namespace.unwrap_or("default").to_string(),
            schedule: "*/5 * * * *".to_string(),
            ..CronJobInfo::default()
        }])
    }

    async fn fetch_resource_quotas(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<ResourceQuotaInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![ResourceQuotaInfo::default()])
    }

    async fn fetch_limit_ranges(&self, _namespace: Option<&str>) -> Result<Vec<LimitRangeInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![LimitRangeInfo::default()])
    }

    async fn fetch_pod_disruption_budgets(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<PodDisruptionBudgetInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![PodDisruptionBudgetInfo::default()])
    }

    async fn fetch_service_accounts(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<ServiceAccountInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![ServiceAccountInfo::default()])
    }

    async fn fetch_roles(&self, _namespace: Option<&str>) -> Result<Vec<RoleInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![RoleInfo::default()])
    }

    async fn fetch_role_bindings(&self, _namespace: Option<&str>) -> Result<Vec<RoleBindingInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![RoleBindingInfo::default()])
    }

    async fn fetch_cluster_roles(&self) -> Result<Vec<ClusterRoleInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![ClusterRoleInfo::default()])
    }

    async fn fetch_cluster_role_bindings(&self) -> Result<Vec<ClusterRoleBindingInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![ClusterRoleBindingInfo::default()])
    }

    async fn fetch_vulnerability_reports(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<VulnerabilityReportInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_custom_resource_definitions(
        &self,
    ) -> Result<Vec<kubectui::k8s::dtos::CustomResourceDefinitionInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![kubectui::k8s::dtos::CustomResourceDefinitionInfo {
            name: "widgets.demo.io".to_string(),
            group: "demo.io".to_string(),
            version: "v1".to_string(),
            kind: "Widget".to_string(),
            plural: "widgets".to_string(),
            scope: "Namespaced".to_string(),
            instances: 1,
        }])
    }

    async fn fetch_cluster_version(&self) -> Result<kubectui::k8s::dtos::ClusterVersionInfo> {
        self.bump();
        self.maybe_delay().await;
        Ok(kubectui::k8s::dtos::ClusterVersionInfo {
            git_version: "v1.31.0".to_string(),
            platform: "linux/amd64".to_string(),
        })
    }

    async fn fetch_cluster_pod_count(&self) -> Result<usize> {
        self.bump();
        self.maybe_delay().await;
        Ok(1)
    }

    async fn fetch_endpoints(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<kubectui::k8s::dtos::EndpointInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_ingresses(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<kubectui::k8s::dtos::IngressInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_ingress_classes(&self) -> Result<Vec<kubectui::k8s::dtos::IngressClassInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_gateway_classes(&self) -> Result<Vec<kubectui::k8s::dtos::GatewayClassInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_gateways(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<kubectui::k8s::dtos::GatewayInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_http_routes(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<kubectui::k8s::dtos::HttpRouteInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_grpc_routes(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<kubectui::k8s::dtos::GrpcRouteInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_reference_grants(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<kubectui::k8s::dtos::ReferenceGrantInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_network_policies(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<kubectui::k8s::dtos::NetworkPolicyInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_config_maps(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<kubectui::k8s::dtos::ConfigMapInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_secrets(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<kubectui::k8s::dtos::SecretInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_hpas(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<kubectui::k8s::dtos::HpaInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_pvcs(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<kubectui::k8s::dtos::PvcInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_pvs(&self) -> Result<Vec<kubectui::k8s::dtos::PvInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_storage_classes(&self) -> Result<Vec<kubectui::k8s::dtos::StorageClassInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_namespace_list(&self) -> Result<Vec<kubectui::k8s::dtos::NamespaceInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(vec![kubectui::k8s::dtos::NamespaceInfo {
            name: "default".to_string(),
            status: "Active".to_string(),
            ..kubectui::k8s::dtos::NamespaceInfo::default()
        }])
    }

    async fn fetch_events(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<kubectui::k8s::dtos::K8sEventInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_priority_classes(&self) -> Result<Vec<kubectui::k8s::dtos::PriorityClassInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_helm_releases(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<kubectui::k8s::dtos::HelmReleaseInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_flux_resources(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<kubectui::k8s::dtos::FluxResourceInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_all_node_metrics(&self) -> Result<Vec<kubectui::k8s::dtos::NodeMetricsInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }

    async fn fetch_all_pod_metrics(
        &self,
        _namespace: Option<&str>,
    ) -> Result<Vec<kubectui::k8s::dtos::PodMetricsInfo>> {
        self.bump();
        self.maybe_delay().await;
        Ok(Vec::new())
    }
}

#[derive(Clone, Copy)]
struct RefreshScenario {
    name: &'static str,
    primary_scope: RefreshScope,
    background_scope: RefreshScope,
    expected_api_calls: usize,
}

fn median(values: &mut [u128]) -> u128 {
    values.sort_unstable();
    values[values.len() / 2]
}

async fn measure_refresh_path(
    source: &PerfMockDataSource,
    warm: bool,
    scenario: RefreshScenario,
) -> Result<(u128, u128, usize)> {
    let mut state = GlobalState::default();
    if warm {
        state
            .refresh_with_options(
                source,
                Some("default"),
                RefreshOptions {
                    scope: scenario.primary_scope.union(scenario.background_scope),
                    include_cluster_info: false,
                    skip_core: false,
                },
            )
            .await?;
    }

    let before_calls = source.counters.total.load(Ordering::Relaxed);
    let start = Instant::now();
    state
        .refresh_with_options(
            source,
            Some("default"),
            RefreshOptions {
                scope: scenario.primary_scope,
                include_cluster_info: false,
                skip_core: false,
            },
        )
        .await?;
    let primary_ms = start.elapsed().as_millis();

    if !scenario.background_scope.is_empty() {
        state
            .refresh_with_options(
                source,
                Some("default"),
                RefreshOptions {
                    scope: scenario.background_scope,
                    include_cluster_info: false,
                    skip_core: true,
                },
            )
            .await?;
    }

    let background_ms = start.elapsed().as_millis();
    let api_calls = source
        .counters
        .total
        .load(Ordering::Relaxed)
        .saturating_sub(before_calls);
    Ok((primary_ms, background_ms, api_calls))
}

/// Emits median startup/refresh readiness and API-call baselines for key views.
#[tokio::test]
#[ignore = "Optional profiling run"]
async fn profile_refresh_scope_baselines() {
    let scenarios = [
        RefreshScenario {
            name: "Dashboard",
            primary_scope: RefreshScope::CORE_OVERVIEW,
            background_scope: RefreshScope::METRICS,
            expected_api_calls: 14,
        },
        RefreshScenario {
            name: "Pods",
            primary_scope: RefreshScope::PODS,
            background_scope: RefreshScope::METRICS,
            expected_api_calls: 4,
        },
        RefreshScenario {
            name: "Nodes",
            primary_scope: RefreshScope::NODES,
            background_scope: RefreshScope::METRICS,
            expected_api_calls: 4,
        },
        RefreshScenario {
            name: "Services",
            primary_scope: RefreshScope::SERVICES,
            background_scope: RefreshScope::NETWORK,
            expected_api_calls: 5,
        },
        RefreshScenario {
            name: "ServiceAccounts",
            primary_scope: RefreshScope::SECURITY,
            background_scope: RefreshScope::NONE,
            expected_api_calls: 5,
        },
        RefreshScenario {
            name: "PVCs",
            primary_scope: RefreshScope::STORAGE,
            background_scope: RefreshScope::NONE,
            expected_api_calls: 3,
        },
        RefreshScenario {
            name: "Flux",
            primary_scope: RefreshScope::FLUX,
            background_scope: RefreshScope::NONE,
            expected_api_calls: 1,
        },
        RefreshScenario {
            name: "Issues",
            primary_scope: RefreshScope::CORE_OVERVIEW,
            background_scope: RefreshScope::LEGACY_SECONDARY.union(RefreshScope::FLUX),
            expected_api_calls: 33,
        },
    ];

    println!("refresh scope baselines:");
    for scenario in scenarios {
        let mut startup_primary = Vec::with_capacity(5);
        let mut startup_background = Vec::with_capacity(5);
        let mut startup_calls = Vec::with_capacity(5);
        let mut refresh_primary = Vec::with_capacity(5);
        let mut refresh_background = Vec::with_capacity(5);
        let mut refresh_calls = Vec::with_capacity(5);

        for _ in 0..5 {
            let startup_source = PerfMockDataSource::new(2);
            let (primary_ms, background_ms, api_calls) =
                measure_refresh_path(&startup_source, false, scenario)
                    .await
                    .unwrap_or_else(|err| {
                        panic!("{} startup measurement failed: {err}", scenario.name)
                    });
            startup_primary.push(primary_ms);
            startup_background.push(background_ms);
            startup_calls.push(api_calls as u128);

            let refresh_source = PerfMockDataSource::new(2);
            let (primary_ms, background_ms, api_calls) =
                measure_refresh_path(&refresh_source, true, scenario)
                    .await
                    .unwrap_or_else(|err| {
                        panic!("{} refresh measurement failed: {err}", scenario.name)
                    });
            refresh_primary.push(primary_ms);
            refresh_background.push(background_ms);
            refresh_calls.push(api_calls as u128);
        }

        let startup_primary_ms = median(&mut startup_primary);
        let startup_background_ms = median(&mut startup_background);
        let startup_api_calls = median(&mut startup_calls);
        let refresh_primary_ms = median(&mut refresh_primary);
        let refresh_background_ms = median(&mut refresh_background);
        let refresh_api_calls = median(&mut refresh_calls);

        assert_eq!(
            startup_api_calls as usize, scenario.expected_api_calls,
            "{} startup api_calls_per_refresh drifted",
            scenario.name
        );
        assert_eq!(
            refresh_api_calls as usize, scenario.expected_api_calls,
            "{} refresh api_calls_per_refresh drifted",
            scenario.name
        );
        assert!(
            startup_background_ms >= startup_primary_ms,
            "{} startup background readiness regressed below primary",
            scenario.name
        );
        assert!(
            refresh_background_ms >= refresh_primary_ms,
            "{} refresh background readiness regressed below primary",
            scenario.name
        );

        println!(
            "- {:<16} startup: primary={}ms background={}ms api_calls={} | refresh: primary={}ms background={}ms api_calls={}",
            scenario.name,
            startup_primary_ms,
            startup_background_ms,
            startup_api_calls,
            refresh_primary_ms,
            refresh_background_ms,
            refresh_api_calls
        );
    }
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
