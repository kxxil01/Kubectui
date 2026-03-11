use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use kubectui::ui::format_age;

fn bench_format_age(c: &mut Criterion) {
    let mut group = c.benchmark_group("format_age");

    group.bench_function("none", |b| {
        b.iter(|| format_age(None));
    });

    group.bench_function("seconds", |b| {
        b.iter(|| format_age(Some(Duration::from_secs(45))));
    });

    group.bench_function("hours", |b| {
        b.iter(|| format_age(Some(Duration::from_secs(3600 * 5 + 120))));
    });

    group.bench_function("days", |b| {
        b.iter(|| format_age(Some(Duration::from_secs(86400 * 7 + 3600))));
    });

    group.finish();
}

criterion_group!(benches, bench_format_age);
criterion_main!(benches);
