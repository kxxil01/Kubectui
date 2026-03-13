mod common;

use criterion::{Criterion, criterion_group, criterion_main};
use kubectui::state::alerts::{
    compute_alerts, compute_cluster_resource_summary, compute_dashboard_insights,
    compute_dashboard_stats, compute_namespace_utilization, compute_top_pod_consumers,
    compute_workload_ready_percent,
};

fn bench_dashboard_stats(c: &mut Criterion) {
    let mut group = c.benchmark_group("dashboard_stats");
    for count in [100, 500, 2000] {
        let snap = common::make_test_snapshot(count);
        group.bench_function(format!("{count}_pods"), |b| {
            b.iter(|| compute_dashboard_stats(&snap));
        });
    }
    group.finish();
}

fn bench_alerts(c: &mut Criterion) {
    let mut group = c.benchmark_group("dashboard_alerts");
    for count in [100, 500, 2000] {
        let snap = common::make_test_snapshot(count);
        group.bench_function(format!("{count}_pods"), |b| {
            b.iter(|| compute_alerts(&snap));
        });
    }
    group.finish();
}

fn bench_insights(c: &mut Criterion) {
    let mut group = c.benchmark_group("dashboard_insights");
    for count in [100, 500, 2000] {
        let snap = common::make_test_snapshot(count);
        group.bench_function(format!("{count}_pods"), |b| {
            b.iter(|| compute_dashboard_insights(&snap));
        });
    }
    group.finish();
}

fn bench_workload_ready(c: &mut Criterion) {
    let mut group = c.benchmark_group("workload_ready_percent");
    for count in [100, 500, 2000] {
        let snap = common::make_test_snapshot(count);
        group.bench_function(format!("{count}_pods"), |b| {
            b.iter(|| compute_workload_ready_percent(&snap));
        });
    }
    group.finish();
}

fn bench_cluster_resource_summary(c: &mut Criterion) {
    let mut group = c.benchmark_group("cluster_resource_summary");
    for count in [100, 500, 2000] {
        let snap = common::make_test_snapshot(count);
        group.bench_function(format!("{count}_pods"), |b| {
            b.iter(|| compute_cluster_resource_summary(&snap));
        });
    }
    group.finish();
}

fn bench_top_pod_consumers(c: &mut Criterion) {
    let mut group = c.benchmark_group("top_pod_consumers");
    for count in [100, 500, 2000] {
        let snap = common::make_test_snapshot(count);
        group.bench_function(format!("{count}_pods"), |b| {
            b.iter(|| compute_top_pod_consumers(&snap));
        });
    }
    group.finish();
}

fn bench_namespace_utilization(c: &mut Criterion) {
    let mut group = c.benchmark_group("namespace_utilization");
    for count in [100, 500, 2000] {
        let snap = common::make_test_snapshot(count);
        group.bench_function(format!("{count}_pods"), |b| {
            b.iter(|| compute_namespace_utilization(&snap));
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_dashboard_stats,
    bench_alerts,
    bench_insights,
    bench_workload_ready,
    bench_cluster_resource_summary,
    bench_top_pod_consumers,
    bench_namespace_utilization,
);
criterion_main!(benches);
