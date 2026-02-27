use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    io::{self, Write},
    path::PathBuf,
    sync::{LazyLock, Mutex},
    time::Instant,
};

use crate::app::AppView;

use super::filter_cache::filter_cache_stats;

const MAX_SAMPLES_PER_VIEW: usize = 4096;

#[derive(Debug, Clone)]
struct SpanStat {
    count: u64,
    total_micros: u128,
    max_micros: u64,
}

impl SpanStat {
    fn record(&mut self, micros: u64) {
        self.count = self.count.saturating_add(1);
        self.total_micros = self.total_micros.saturating_add(micros as u128);
        self.max_micros = self.max_micros.max(micros);
    }
}

#[derive(Debug)]
struct ProfilerData {
    enabled: bool,
    output_dir: PathBuf,
    view_samples: HashMap<AppView, Vec<u64>>,
    spans: HashMap<&'static str, SpanStat>,
    folded: HashMap<String, u64>,
}

impl Default for ProfilerData {
    fn default() -> Self {
        Self {
            enabled: false,
            output_dir: PathBuf::from("target/profiles"),
            view_samples: HashMap::new(),
            spans: HashMap::new(),
            folded: HashMap::new(),
        }
    }
}

static PROFILER: LazyLock<Mutex<ProfilerData>> =
    LazyLock::new(|| Mutex::new(ProfilerData::default()));

thread_local! {
    static SPAN_STACK: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

pub fn init_from_env() {
    if let Ok(val) = std::env::var("KUBECTUI_PROFILE_RENDER")
        && (val == "1" || val.eq_ignore_ascii_case("true"))
    {
        set_enabled(true);
    }
    if let Ok(path) = std::env::var("KUBECTUI_PROFILE_OUTPUT")
        && !path.trim().is_empty()
    {
        set_output_dir(PathBuf::from(path));
    }
}

pub fn set_enabled(enabled: bool) {
    if let Ok(mut profiler) = PROFILER.lock() {
        profiler.enabled = enabled;
    }
}

pub fn set_output_dir(path: PathBuf) {
    if let Ok(mut profiler) = PROFILER.lock() {
        profiler.output_dir = path;
    }
}

pub fn is_enabled() -> bool {
    PROFILER.lock().map(|p| p.enabled).unwrap_or(false)
}

pub struct FrameScope {
    enabled: bool,
    view: AppView,
    started_at: Instant,
}

pub fn frame_scope(view: AppView) -> FrameScope {
    FrameScope {
        enabled: is_enabled(),
        view,
        started_at: Instant::now(),
    }
}

impl Drop for FrameScope {
    fn drop(&mut self) {
        if !self.enabled {
            return;
        }
        let micros = self.started_at.elapsed().as_micros() as u64;
        if let Ok(mut profiler) = PROFILER.lock() {
            let samples = profiler.view_samples.entry(self.view).or_default();
            if samples.len() >= MAX_SAMPLES_PER_VIEW {
                samples.remove(0);
            }
            samples.push(micros);
        }
    }
}

pub struct SpanScope {
    enabled: bool,
    name: &'static str,
    started_at: Instant,
}

pub fn span_scope(name: &'static str) -> SpanScope {
    let enabled = is_enabled();
    if enabled {
        SPAN_STACK.with(|stack| stack.borrow_mut().push(name));
    }
    SpanScope {
        enabled,
        name,
        started_at: Instant::now(),
    }
}

impl Drop for SpanScope {
    fn drop(&mut self) {
        if !self.enabled {
            return;
        }

        let micros = self.started_at.elapsed().as_micros() as u64;
        let mut collapsed = String::new();
        SPAN_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            collapsed = stack.join(";");
            let _ = stack.pop();
        });

        if let Ok(mut profiler) = PROFILER.lock() {
            profiler
                .spans
                .entry(self.name)
                .or_insert(SpanStat {
                    count: 0,
                    total_micros: 0,
                    max_micros: 0,
                })
                .record(micros);

            if !collapsed.is_empty() {
                let entry = profiler.folded.entry(collapsed).or_insert(0);
                *entry = entry.saturating_add(micros);
            }
        }
    }
}

pub fn write_report_if_enabled() -> io::Result<Option<(PathBuf, PathBuf)>> {
    let snapshot = {
        let profiler = match PROFILER.lock() {
            Ok(guard) => guard,
            Err(_) => return Ok(None),
        };
        if !profiler.enabled {
            return Ok(None);
        }
        (
            profiler.output_dir.clone(),
            profiler.view_samples.clone(),
            profiler.spans.clone(),
            profiler.folded.clone(),
        )
    };

    let (output_dir, view_samples, spans, folded) = snapshot;
    fs::create_dir_all(&output_dir)?;

    let summary_path = output_dir.join("render-frame-summary.txt");
    let folded_path = output_dir.join("render-flamegraph.folded");

    let mut summary_file = fs::File::create(&summary_path)?;
    summary_file.write_all(build_summary(&view_samples, &spans).as_bytes())?;

    let mut folded_entries = folded.into_iter().collect::<Vec<_>>();
    folded_entries.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut folded_file = fs::File::create(&folded_path)?;
    for (stack, micros) in folded_entries {
        writeln!(folded_file, "{stack} {micros}")?;
    }

    Ok(Some((summary_path, folded_path)))
}

fn build_summary(
    view_samples: &HashMap<AppView, Vec<u64>>,
    spans: &HashMap<&'static str, SpanStat>,
) -> String {
    let mut out = String::new();
    out.push_str("KubecTUI Render Profiling Summary\n");
    out.push_str("================================\n\n");
    out.push_str("Per-view frame-time breakdown (milliseconds)\n");
    out.push_str("-------------------------------------------\n");

    let mut rows = view_samples
        .iter()
        .filter(|(_, samples)| !samples.is_empty())
        .map(|(view, samples)| {
            let mut sorted = samples.clone();
            sorted.sort_unstable();
            let count = sorted.len();
            let total: u128 = sorted.iter().map(|v| *v as u128).sum();
            let avg = (total as f64 / count as f64) / 1000.0;
            let p50 = percentile(&sorted, 50.0) as f64 / 1000.0;
            let p95 = percentile(&sorted, 95.0) as f64 / 1000.0;
            let max = *sorted.last().unwrap_or(&0) as f64 / 1000.0;
            let total_ms = total as f64 / 1000.0;
            (*view, count, avg, p50, p95, max, total_ms)
        })
        .collect::<Vec<_>>();

    rows.sort_unstable_by(|a, b| b.6.total_cmp(&a.6));
    for (view, count, avg, p50, p95, max, total_ms) in rows {
        out.push_str(&format!(
            "- {:<28} frames={:<5} avg={:>7.3}ms p50={:>7.3}ms p95={:>7.3}ms max={:>7.3}ms total={:>8.3}ms\n",
            view.label(), count, avg, p50, p95, max, total_ms
        ));
    }

    out.push_str("\nTop render spans by total time (milliseconds)\n");
    out.push_str("--------------------------------------------\n");
    let mut span_rows = spans
        .iter()
        .map(|(name, stat)| {
            (
                *name,
                stat.count,
                stat.total_micros as f64 / 1000.0,
                stat.max_micros as f64 / 1000.0,
            )
        })
        .collect::<Vec<_>>();
    span_rows.sort_unstable_by(|a, b| b.2.total_cmp(&a.2));
    for (name, count, total_ms, max_ms) in span_rows.into_iter().take(20) {
        out.push_str(&format!(
            "- {:<28} count={:<6} total={:>8.3}ms max={:>7.3}ms\n",
            name, count, total_ms, max_ms
        ));
    }

    let cache = filter_cache_stats();
    out.push_str("\nFilter cache stats\n");
    out.push_str("------------------\n");
    out.push_str(&format!(
        "- entries={} hits={} misses={}\n",
        cache.entries, cache.hits, cache.misses
    ));

    out
}

fn percentile(samples: &[u64], p: f64) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let clamped = p.clamp(0.0, 100.0);
    let rank = ((samples.len() - 1) as f64 * (clamped / 100.0)).round() as usize;
    samples[rank]
}
