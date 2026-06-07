//! `PhaseMetrics` — per-Phase observability (S24, ADR-0008 §0.7 loop).
//!
//! Each Task Assignment records four metrics:
//!   - `events_processed_total` (counter)
//!   - `processing_latency_p50/p99` (histogram with sensible buckets)
//!   - `cpu_seconds_total` (counter — fed by cgroup / process stats
//!     in production; tests can set it directly)
//!   - `backpressure_wait_seconds_total` (counter — time the Task
//!     spent waiting for input)
//!
//! The runtime records `events_processed_total`, `processing_latency`,
//! and `backpressure_wait_seconds_total` around handler calls. The
//! `cpu_seconds_total` is fed by an external sampler (S24 wires the
//! sampling path; the test path exposes a `record_cpu` method).
//!
//! ## < 1% CPU overhead (S24 acceptance)
//! When `metrics = None` is passed to `Runtime::run_with_metrics`,
//! the runtime's hot path contains one extra `Option::is_some` check
//! per handler call. The PhaseMetrics counters themselves use
//! `AtomicU64::fetch_add(Relaxed)` — a single relaxed atomic op per
//! observation. The histogram is fixed-size (5 buckets), no
//! allocation in the hot path.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Histogram bucket upper bounds in seconds. Per S24 acceptance:
/// "Histogram buckets are sensible (e.g., 1ms, 10ms, 100ms, 1s, 10s)".
///
/// An observation `d` falls into the first bucket whose upper bound
/// is `>= d.as_secs_f64()`. Observations larger than the last bucket
/// are clamped to the last bucket. The reported p50/p99 is the
/// upper bound of the bucket the quantile lands in (so a p50 of
/// "100ms" means "> 10ms, <= 100ms" — the bucket resolution is
/// the metric).
pub const HISTOGRAM_BUCKETS_SECS: &[f64] = &[0.001, 0.01, 0.1, 1.0, 10.0];

/// Fixed-size histogram over [`HISTOGRAM_BUCKETS_SECS`]. All
/// counters are atomic so the runtime can record from the handler
/// task without locking.
pub struct Histogram {
    buckets: [AtomicU64; 5],
    count: AtomicU64,
    sum_ns: AtomicU64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

impl Histogram {
    pub fn new() -> Self {
        Self {
            buckets: Default::default(),
            count: AtomicU64::new(0),
            sum_ns: AtomicU64::new(0),
        }
    }

    /// Record one observation. The bucket index is computed by
    /// linear scan over the (small, fixed) bucket list.
    pub fn record(&self, d: Duration) {
        let secs = d.as_secs_f64();
        let idx = HISTOGRAM_BUCKETS_SECS
            .iter()
            .position(|&b| secs <= b)
            .unwrap_or(HISTOGRAM_BUCKETS_SECS.len() - 1);
        self.buckets[idx].fetch_add(1, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum_ns.fetch_add(d.as_nanos() as u64, Ordering::Relaxed);
    }

    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    pub fn avg(&self) -> Option<Duration> {
        let c = self.count();
        if c == 0 {
            return None;
        }
        Some(Duration::from_nanos(self.sum_ns.load(Ordering::Relaxed) / c))
    }

    /// p50: the duration value at the 50th percentile. Returns
    /// `None` if no observations. Computed by cumulative count.
    pub fn p50(&self) -> Option<Duration> {
        self.quantile(0.5)
    }

    /// p99: the duration value at the 99th percentile.
    pub fn p99(&self) -> Option<Duration> {
        self.quantile(0.99)
    }

    fn quantile(&self, q: f64) -> Option<Duration> {
        let total = self.count();
        if total == 0 {
            return None;
        }
        let target = (total as f64 * q).ceil() as u64;
        let mut cumulative: u64 = 0;
        for (i, bucket) in self.buckets.iter().enumerate() {
            cumulative += bucket.load(Ordering::Relaxed);
            if cumulative >= target {
                return Some(Duration::from_secs_f64(HISTOGRAM_BUCKETS_SECS[i]));
            }
        }
        // Unreachable: target <= total <= cumulative after the
        // last bucket. Defensive fallback.
        Some(Duration::from_secs_f64(
            *HISTOGRAM_BUCKETS_SECS.last().unwrap(),
        ))
    }

    /// Per-bucket counts (read-only view). Used by tests.
    pub fn bucket_counts(&self) -> [u64; 5] {
        let mut out = [0u64; 5];
        for (i, b) in self.buckets.iter().enumerate() {
            out[i] = b.load(Ordering::Relaxed);
        }
        out
    }
}

/// Per-Task metrics. Owned by the `TaskWorker`; an `Arc<PhaseMetrics>`
/// is passed to the Runtime so it can record without taking a lock.
pub struct PhaseMetrics {
    /// Total events the handler has finished processing.
    pub events_processed_total: AtomicU64,
    /// Latency histogram (around each `handler.handle` call).
    pub processing_latency: Histogram,
    /// CPU seconds consumed (counter — production samples from
    /// cgroup; tests increment directly).
    pub cpu_seconds_total_ns: AtomicU64,
    /// Backpressure wait seconds — time the task spent waiting
    /// for the next input event.
    pub backpressure_wait_seconds_total_ns: AtomicU64,
}

impl Default for PhaseMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl PhaseMetrics {
    pub fn new() -> Self {
        Self {
            events_processed_total: AtomicU64::new(0),
            processing_latency: Histogram::new(),
            cpu_seconds_total_ns: AtomicU64::new(0),
            backpressure_wait_seconds_total_ns: AtomicU64::new(0),
        }
    }

    pub fn record_event_processed(&self) {
        self.events_processed_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_latency(&self, d: Duration) {
        self.processing_latency.record(d);
    }

    pub fn record_cpu(&self, d: Duration) {
        self.cpu_seconds_total_ns
            .fetch_add(d.as_nanos() as u64, Ordering::Relaxed);
    }

    pub fn record_backpressure_wait(&self, d: Duration) {
        self.backpressure_wait_seconds_total_ns
            .fetch_add(d.as_nanos() as u64, Ordering::Relaxed);
    }

    /// Snapshot the four metrics into a plain struct for printing
    /// (e.g., by `bee diagnostics <TaskId>`).
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            events_processed_total: self.events_processed_total.load(Ordering::Relaxed),
            latency_count: self.processing_latency.count(),
            latency_avg: self.processing_latency.avg(),
            latency_p50: self.processing_latency.p50(),
            latency_p99: self.processing_latency.p99(),
            cpu_seconds_total: Duration::from_nanos(
                self.cpu_seconds_total_ns.load(Ordering::Relaxed),
            ),
            backpressure_wait_seconds_total: Duration::from_nanos(
                self.backpressure_wait_seconds_total_ns.load(Ordering::Relaxed),
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub events_processed_total: u64,
    pub latency_count: u64,
    pub latency_avg: Option<Duration>,
    pub latency_p50: Option<Duration>,
    pub latency_p99: Option<Duration>,
    pub cpu_seconds_total: Duration,
    pub backpressure_wait_seconds_total: Duration,
}

impl std::fmt::Display for MetricsSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "events_processed_total: {}", self.events_processed_total)?;
        writeln!(f, "processing_latency_count: {}", self.latency_count)?;
        if let Some(avg) = self.latency_avg {
            writeln!(f, "processing_latency_avg: {:?}", avg)?;
        } else {
            writeln!(f, "processing_latency_avg: (none)")?;
        }
        if let Some(p50) = self.latency_p50 {
            writeln!(f, "processing_latency_p50: {:?}", p50)?;
        } else {
            writeln!(f, "processing_latency_p50: (none)")?;
        }
        if let Some(p99) = self.latency_p99 {
            writeln!(f, "processing_latency_p99: {:?}", p99)?;
        } else {
            writeln!(f, "processing_latency_p99: (none)")?;
        }
        writeln!(
            f,
            "cpu_seconds_total: {:?}",
            self.cpu_seconds_total
        )?;
        writeln!(
            f,
            "backpressure_wait_seconds_total: {:?}",
            self.backpressure_wait_seconds_total
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_empty_quantiles_are_none() {
        let h = Histogram::new();
        assert_eq!(h.count(), 0);
        assert_eq!(h.p50(), None);
        assert_eq!(h.p99(), None);
        assert_eq!(h.avg(), None);
        assert_eq!(h.bucket_counts(), [0, 0, 0, 0, 0]);
    }

    #[test]
    fn histogram_records_into_correct_buckets() {
        let h = Histogram::new();
        h.record(Duration::from_micros(500)); // <= 1ms → bucket 0
        h.record(Duration::from_micros(5_000)); // <= 10ms → bucket 1
        h.record(Duration::from_millis(50)); // <= 100ms → bucket 2
        h.record(Duration::from_millis(500)); // <= 1s → bucket 3
        h.record(Duration::from_secs(5)); // <= 10s → bucket 4
        h.record(Duration::from_secs(60)); // > 10s → bucket 4 (clamped)
        assert_eq!(h.bucket_counts(), [1, 1, 1, 1, 2]);
        assert_eq!(h.count(), 6);
    }

    #[test]
    fn histogram_p50_and_p99_match_observation_distribution() {
        // 10 observations at 1ms each, 10 observations at 100ms each.
        // p50 should be 1ms; p99 should be 100ms.
        let h = Histogram::new();
        for _ in 0..10 {
            h.record(Duration::from_millis(1));
        }
        for _ in 0..10 {
            h.record(Duration::from_millis(100));
        }
        assert_eq!(h.count(), 20);
        let p50 = h.p50().unwrap();
        let p99 = h.p99().unwrap();
        // 10th observation is the 1ms bucket; 11th is the 100ms bucket.
        // p50 (10th percentile of 20) → 1ms; p99 (20th) → 100ms.
        assert!(p50 <= Duration::from_millis(1));
        assert!(p99 >= Duration::from_millis(100));
    }

    #[test]
    fn histogram_avg_computes_mean() {
        let h = Histogram::new();
        h.record(Duration::from_millis(10));
        h.record(Duration::from_millis(20));
        h.record(Duration::from_millis(30));
        let avg = h.avg().unwrap();
        // Allow a small fudge for ns truncation.
        let diff = if avg > Duration::from_millis(20) {
            avg - Duration::from_millis(20)
        } else {
            Duration::from_millis(20) - avg
        };
        assert!(diff < Duration::from_micros(1), "avg = {avg:?}");
    }

    #[test]
    fn phase_metrics_counters_increment_independently() {
        let m = PhaseMetrics::new();
        m.record_event_processed();
        m.record_event_processed();
        m.record_event_processed();
        m.record_latency(Duration::from_millis(5));
        m.record_cpu(Duration::from_millis(50));
        m.record_backpressure_wait(Duration::from_millis(2));

        let snap = m.snapshot();
        assert_eq!(snap.events_processed_total, 3);
        assert_eq!(snap.latency_count, 1);
        assert_eq!(snap.cpu_seconds_total, Duration::from_millis(50));
        assert_eq!(
            snap.backpressure_wait_seconds_total,
            Duration::from_millis(2)
        );
    }

    #[test]
    fn metrics_snapshot_display_includes_all_four_fields() {
        // S24 acceptance: `bee diagnostics <TaskId>` prints all four
        // metrics. The Display impl on MetricsSnapshot is what the
        // CLI renders.
        let m = PhaseMetrics::new();
        m.record_event_processed();
        m.record_latency(Duration::from_micros(500));
        m.record_cpu(Duration::from_millis(10));
        m.record_backpressure_wait(Duration::from_millis(5));

        let snap = m.snapshot();
        let s = format!("{snap}");
        assert!(s.contains("events_processed_total: 1"));
        assert!(s.contains("processing_latency_count: 1"));
        assert!(s.contains("processing_latency_p50"));
        assert!(s.contains("processing_latency_p99"));
        assert!(s.contains("cpu_seconds_total"));
        assert!(s.contains("backpressure_wait_seconds_total"));
    }
}
