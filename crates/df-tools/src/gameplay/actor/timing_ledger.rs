//! Owner-side timing histograms: how long each owner call waited and reduced, and how each hero line's
//! stages took. Fixed buckets and fixed-size counters keep the ledger bounded: the call names are the
//! owner's closed `Call` variants and the stage names are the closed `LineStage` labels.
//!
//! Writers: the owner's reduce (every call) and a finished hero line (every line that is not truncated).
//! Readers take a copy under the lock and never walk a log, so a read costs the same however long the game
//! has run.
//!
//! # Export contract (`GET /dev/latency`, D20 metrics)
//!
//! The body is `{"contract": "df-gameplay-timing/1", "metrics": [...]}`, plus `"bucket_upper_us"` when
//! the read asks for buckets. Every metric has `type`, `name`, `labels` and its values:
//!
//! * `type: "histogram"`: `count`, `sum_us`, `max_us` (exact), `p50_us_estimate` and `p95_us_estimate`
//!   (the upper bound of the bucket holding the nearest-rank sample). With `?buckets=1` also `buckets`:
//!   cumulative counts in increasing `le` order, ending with `le: "+Inf"`, whose count equals `count`.
//! * `type: "counter"`: `value`, a monotonic total. `df_gameplay_call_queue_unmeasured_total` counts calls
//!   whose queue wait was not reported; `df_gameplay_line_truncated_total` counts lines not summarized.
//!
//! Metric names: `df_gameplay_call_queue_us` and `df_gameplay_call_reduce_us` (label `call`),
//! `df_gameplay_line_total_us` and `df_gameplay_line_unattributed_us` (no labels), and
//! `df_gameplay_line_stage_us` (label `stage`, one entry per closed stage, zero when unseen).

use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Mutex, PoisonError},
};

use super::line_timing::{LineRecord, LineStage, StageRecord};

/// Upper bounds of the fixed buckets, in microseconds. A value above the last bound lands in the
/// overflow bucket, which the export labels `+Inf`.
pub(in crate::gameplay) const BUCKET_UPPER_US: [u64; 16] = [
    100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000, 500_000,
    1_000_000, 2_500_000, 5_000_000, 10_000_000,
];
const BUCKETS: usize = BUCKET_UPPER_US.len() + 1;
pub(in crate::gameplay) const CONTRACT: &str = "df-gameplay-timing/1";

/// A fixed-size histogram: bucket counts, the sample count, the exact sum and the exact maximum.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::gameplay) struct Histogram {
    counts: [u64; BUCKETS],
    count: u64,
    sum_us: u64,
    max_us: u64,
}

impl Histogram {
    pub(in crate::gameplay) fn record(&mut self, value_us: u64) {
        let bucket = BUCKET_UPPER_US
            .iter()
            .position(|upper| value_us <= *upper)
            .unwrap_or(BUCKET_UPPER_US.len());
        if let Some(count) = self.counts.get_mut(bucket) {
            *count += 1;
        }
        self.count += 1;
        self.sum_us = self.sum_us.saturating_add(value_us);
        self.max_us = self.max_us.max(value_us);
    }

    pub(in crate::gameplay) fn count(&self) -> u64 {
        self.count
    }

    pub(in crate::gameplay) fn sum_us(&self) -> u64 {
        self.sum_us
    }

    pub(in crate::gameplay) fn max_us(&self) -> u64 {
        self.max_us
    }

    /// The nearest-rank percentile, estimated as the upper bound of the bucket that holds it. The
    /// overflow bucket reports the exact maximum. An empty histogram reports zero.
    pub(in crate::gameplay) fn percentile_estimate_us(&self, percent: u64) -> u64 {
        if self.count == 0 {
            return 0;
        }
        let rank = (self.count * percent).div_ceil(100).max(1);
        let mut seen = 0;
        for (bucket, count) in self.counts.iter().enumerate() {
            seen += count;
            if seen >= rank {
                return BUCKET_UPPER_US.get(bucket).copied().unwrap_or(self.max_us);
            }
        }
        self.max_us
    }

    /// The histogram as an export metric, with cumulative buckets only when asked.
    fn metric(&self, name: &str, labels: Value, buckets: bool) -> Value {
        let mut metric = json!({
            "type": "histogram",
            "name": name,
            "labels": labels,
            "count": self.count(),
            "sum_us": self.sum_us(),
            "max_us": self.max_us(),
            "p50_us_estimate": self.percentile_estimate_us(50),
            "p95_us_estimate": self.percentile_estimate_us(95),
        });
        if buckets {
            let mut cumulative = 0;
            let rows: Vec<Value> = self
                .counts
                .iter()
                .enumerate()
                .map(|(bucket, count)| {
                    cumulative += count;
                    json!({"le": bucket_label(bucket), "count": cumulative})
                })
                .collect();
            metric["buckets"] = Value::Array(rows);
        }
        metric
    }
}

/// The `le` label of a bucket: its upper bound in microseconds, or `+Inf` for the overflow bucket.
fn bucket_label(bucket: usize) -> Value {
    match BUCKET_UPPER_US.get(bucket) {
        Some(upper) => json!(upper),
        None => json!("+Inf"),
    }
}

/// One owner call name's histograms. `queue_us` counts only the calls whose queue wait was reported;
/// `queue_unmeasured` counts the rest.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::gameplay) struct CallHistograms {
    pub(in crate::gameplay) queue_us: Histogram,
    pub(in crate::gameplay) queue_unmeasured: u64,
    pub(in crate::gameplay) reduce_us: Histogram,
}

/// Hero-line histograms: the total, the time no stage covers, the exact covered sum, each stage, and
/// how many lines were left out because they were truncated.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::gameplay) struct LineHistograms {
    pub(in crate::gameplay) total_us: Histogram,
    pub(in crate::gameplay) unattributed_us: Histogram,
    pub(in crate::gameplay) covered_us: u64,
    pub(in crate::gameplay) stages_us: BTreeMap<&'static str, Histogram>,
    pub(in crate::gameplay) truncated: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::gameplay) struct TimingLedger {
    pub(in crate::gameplay) calls: BTreeMap<&'static str, CallHistograms>,
    pub(in crate::gameplay) lines: LineHistograms,
}

/// The process-wide ledger. Only the owner's reduce and a finished line write it; reads copy it.
static LEDGER: Mutex<TimingLedger> = Mutex::new(TimingLedger {
    calls: BTreeMap::new(),
    lines: LineHistograms {
        total_us: Histogram::EMPTY,
        unattributed_us: Histogram::EMPTY,
        covered_us: 0,
        stages_us: BTreeMap::new(),
        truncated: 0,
    },
});

impl Histogram {
    const EMPTY: Self = Self {
        counts: [0; BUCKETS],
        count: 0,
        sum_us: 0,
        max_us: 0,
    };
}

fn with_ledger<T>(update: impl FnOnce(&mut TimingLedger) -> T) -> T {
    let mut ledger = LEDGER.lock().unwrap_or_else(PoisonError::into_inner);
    update(&mut ledger)
}

/// The time the stages cover: the length of the union of their intervals, so overlapping stages are
/// counted once.
fn covered_us(stages: &[StageRecord]) -> u64 {
    let mut intervals: Vec<(u64, u64)> = stages
        .iter()
        .map(|stage| {
            (
                stage.start_us,
                stage.start_us.saturating_add(stage.duration_us),
            )
        })
        .collect();
    intervals.sort_unstable();
    let mut covered = 0;
    let mut current: Option<(u64, u64)> = None;
    for (start, end) in intervals {
        current = match current {
            Some((open, close)) if start <= close => Some((open, close.max(end))),
            Some((open, close)) => {
                covered += close - open;
                Some((start, end))
            }
            None => Some((start, end)),
        };
    }
    if let Some((open, close)) = current {
        covered += close - open;
    }
    covered
}

impl TimingLedger {
    /// Counts one owner call: its queue wait when it was reported, and its reduce time.
    pub(in crate::gameplay) fn summarize_call(
        &mut self,
        name: &'static str,
        queue_us: Option<u64>,
        reduce_us: u64,
    ) {
        let call = self.calls.entry(name).or_default();
        match queue_us {
            Some(queue_us) => call.queue_us.record(queue_us),
            None => call.queue_unmeasured += 1,
        }
        call.reduce_us.record(reduce_us);
    }

    /// Adds one finished hero line: its total, the time its stages cover (the union of their intervals),
    /// the time no stage covers, and each stage. A truncated line is counted and left out.
    pub(in crate::gameplay) fn summarize_line(&mut self, record: &LineRecord) -> bool {
        if record.truncated {
            self.lines.truncated += 1;
            return false;
        }
        let covered = covered_us(&record.stages).min(record.total_us);
        self.lines.total_us.record(record.total_us);
        self.lines.unattributed_us.record(record.total_us - covered);
        self.lines.covered_us += covered;
        for stage in &record.stages {
            self.lines
                .stages_us
                .entry(stage.stage.label())
                .or_default()
                .record(stage.duration_us);
        }
        true
    }

    /// The export contract (see the module doc). Buckets are included only when `buckets` is set.
    pub(in crate::gameplay) fn export(&self, buckets: bool) -> Value {
        let mut metrics = vec![
            self.lines
                .total_us
                .metric("df_gameplay_line_total_us", json!({}), buckets),
            self.lines.unattributed_us.metric(
                "df_gameplay_line_unattributed_us",
                json!({}),
                buckets,
            ),
        ];
        for stage in LineStage::ALL {
            let histogram = self
                .lines
                .stages_us
                .get(stage.label())
                .copied()
                .unwrap_or_default();
            metrics.push(histogram.metric(
                "df_gameplay_line_stage_us",
                json!({"stage": stage.label()}),
                buckets,
            ));
        }
        metrics.push(json!({
            "type": "counter",
            "name": "df_gameplay_line_truncated_total",
            "labels": {},
            "value": self.lines.truncated,
        }));
        for (call, histograms) in &self.calls {
            metrics.push(histograms.queue_us.metric(
                "df_gameplay_call_queue_us",
                json!({"call": call}),
                buckets,
            ));
            metrics.push(histograms.reduce_us.metric(
                "df_gameplay_call_reduce_us",
                json!({"call": call}),
                buckets,
            ));
            metrics.push(json!({
                "type": "counter",
                "name": "df_gameplay_call_queue_unmeasured_total",
                "labels": {"call": call},
                "value": histograms.queue_unmeasured,
            }));
        }
        let mut body = json!({"contract": CONTRACT, "metrics": metrics});
        if buckets {
            body["bucket_upper_us"] = json!(BUCKET_UPPER_US);
        }
        body
    }
}

/// Counts one owner call in the process ledger.
pub(in crate::gameplay) fn record_call(name: &'static str, queue_us: Option<u64>, reduce_us: u64) {
    with_ledger(|ledger| ledger.summarize_call(name, queue_us, reduce_us));
}

/// Adds one finished hero line to the process ledger. Returns whether it was counted.
pub(in crate::gameplay) fn record_finished_line(record: &LineRecord) -> bool {
    with_ledger(|ledger| ledger.summarize_line(record))
}

/// A copy of the process ledger, taken under the lock and read outside it.
pub(in crate::gameplay) fn snapshot() -> TimingLedger {
    with_ledger(|ledger| ledger.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(stage: LineStage, start_us: u64, duration_us: u64) -> StageRecord {
        StageRecord {
            stage,
            start_us,
            duration_us,
        }
    }

    fn line(total_us: u64, stages: Vec<StageRecord>) -> LineRecord {
        LineRecord {
            total_us,
            stages,
            truncated: false,
        }
    }

    #[test]
    fn a_value_lands_in_the_first_bucket_whose_bound_holds_it() {
        let mut histogram = Histogram::default();
        histogram.record(100);
        histogram.record(101);
        histogram.record(20_000_000);
        assert_eq!(histogram.count(), 3);
        assert_eq!(histogram.sum_us(), 100 + 101 + 20_000_000);
        assert_eq!(histogram.max_us(), 20_000_000);
        let metric = histogram.metric("m", json!({}), true);
        let buckets = metric["buckets"].as_array().unwrap();
        assert_eq!(buckets[0]["count"], 1);
        assert_eq!(buckets[1]["count"], 2);
    }

    #[test]
    fn the_export_is_monotonic_and_ends_at_the_explicit_inf_bucket() {
        let mut histogram = Histogram::default();
        for value in [50, 300, 300, 7_000, 90_000_000, 20_000_000] {
            histogram.record(value);
        }
        let metric = histogram.metric("m", json!({}), true);
        let buckets = metric["buckets"].as_array().unwrap();
        let counts: Vec<u64> = buckets
            .iter()
            .map(|bucket| bucket["count"].as_u64().unwrap())
            .collect();
        assert!(
            counts.windows(2).all(|pair| pair[0] <= pair[1]),
            "{counts:?}"
        );
        assert_eq!(*counts.last().unwrap(), histogram.count());
        assert_eq!(buckets.last().unwrap()["le"], "+Inf");
        assert_eq!(buckets[0]["le"], 100);
    }

    #[test]
    fn percentiles_are_bucket_estimates_and_an_empty_histogram_is_zero() {
        let mut histogram = Histogram::default();
        assert_eq!(histogram.percentile_estimate_us(95), 0);
        for _ in 0..19 {
            histogram.record(90);
        }
        histogram.record(40_000);
        assert_eq!(histogram.percentile_estimate_us(50), 100);
        assert_eq!(histogram.percentile_estimate_us(95), 100);
        assert_eq!(histogram.percentile_estimate_us(100), 50_000);
    }

    #[test]
    fn covered_time_is_the_union_of_the_stage_intervals() {
        // Classify [0, 60) and Addressee [40, 80) overlap on [40, 60): their union is [0, 80), 80 us.
        // Reason [200, 250) sits apart and adds 50 us.
        let stages = [
            stage(LineStage::Classify, 0, 60),
            stage(LineStage::Addressee, 40, 40),
            stage(LineStage::Reason, 200, 50),
        ];
        assert_eq!(covered_us(&stages), 80 + 50);
    }

    #[test]
    fn a_truncated_line_is_counted_as_truncated_and_never_summarized() {
        let mut ledger = TimingLedger::default();
        let truncated = LineRecord {
            truncated: true,
            ..line(1_000, vec![stage(LineStage::Classify, 0, 500)])
        };
        assert!(!ledger.summarize_line(&truncated));
        assert_eq!(ledger.lines.truncated, 1);
        assert_eq!(ledger.lines.total_us.count(), 0);
        assert_eq!(ledger.lines.covered_us, 0);
    }

    #[test]
    fn overlapping_stages_do_not_inflate_the_covered_time() {
        let mut ledger = TimingLedger::default();
        ledger.summarize_line(&line(
            1_000,
            vec![
                stage(LineStage::Classify, 0, 600),
                stage(LineStage::Reason, 300, 600),
            ],
        ));
        assert_eq!(ledger.lines.covered_us, 900);
        assert_eq!(ledger.lines.unattributed_us.sum_us(), 100);
    }

    #[test]
    fn the_ledger_counts_calls_with_and_without_a_reported_queue() {
        let mut ledger = TimingLedger::default();
        ledger.summarize_call("View", Some(250), 4_000);
        ledger.summarize_call("View", None, 6_000);
        let view = ledger.calls["View"];
        assert_eq!(view.queue_us.count(), 1);
        assert_eq!(view.queue_unmeasured, 1);
        assert_eq!(view.reduce_us.count(), 2);
        assert_eq!(view.reduce_us.sum_us(), 10_000);
        assert_eq!(view.reduce_us.max_us(), 6_000);
    }

    #[test]
    fn the_export_shape_is_compact_by_default_and_buckets_are_opt_in() {
        let mut ledger = TimingLedger::default();
        ledger.summarize_call("View", None, 4_000);
        let compact = ledger.export(false);
        assert_eq!(compact["contract"], CONTRACT);
        assert!(compact.get("bucket_upper_us").is_none());
        let metrics = compact["metrics"].as_array().unwrap();
        let reduce = metrics
            .iter()
            .find(|metric| metric["name"] == "df_gameplay_call_reduce_us")
            .unwrap();
        assert_eq!(reduce["type"], "histogram");
        assert_eq!(reduce["labels"]["call"], "View");
        assert_eq!(reduce["max_us"], 4_000);
        assert!(reduce.get("p50_us_estimate").is_some() && reduce.get("p95_us_estimate").is_some());
        assert!(reduce.get("buckets").is_none());
        let unmeasured = metrics
            .iter()
            .find(|metric| metric["name"] == "df_gameplay_call_queue_unmeasured_total")
            .unwrap();
        assert_eq!(unmeasured["type"], "counter");
        assert_eq!(unmeasured["value"], 1);
        let stages: Vec<&Value> = metrics
            .iter()
            .filter(|metric| metric["name"] == "df_gameplay_line_stage_us")
            .collect();
        assert_eq!(
            stages.len(),
            LineStage::ALL.len(),
            "every closed stage appears"
        );

        let full = ledger.export(true);
        assert!(full.get("bucket_upper_us").is_some());
        let view = full["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|metric| metric["name"] == "df_gameplay_call_reduce_us")
            .unwrap();
        assert!(view["buckets"].is_array());
    }
}
