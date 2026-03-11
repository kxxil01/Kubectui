mod common;

use criterion::{Criterion, criterion_group, criterion_main};
use kubectui::app::filtered_pod_indices;

fn bench_filtered_pod_indices(c: &mut Criterion) {
    let mut group = c.benchmark_group("filtered_pod_indices");
    for count in [100, 500, 2000] {
        let snap = common::make_test_snapshot(count);

        group.bench_function(format!("{count}_no_filter"), |b| {
            b.iter(|| filtered_pod_indices(&snap.pods, "", None));
        });

        group.bench_function(format!("{count}_filter_hit"), |b| {
            b.iter(|| filtered_pod_indices(&snap.pods, "pod-1", None));
        });

        group.bench_function(format!("{count}_filter_miss"), |b| {
            b.iter(|| filtered_pod_indices(&snap.pods, "zzzzz", None));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_filtered_pod_indices);
criterion_main!(benches);
