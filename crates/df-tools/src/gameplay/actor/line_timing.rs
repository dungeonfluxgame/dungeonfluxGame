//! One correlated timing record per hero line: a line id, then named stages with start offsets and
//! durations, emitted once through the shared audit path when the line completes.
//!
//! A line captures the audit correlation in force where it begins (the dispatch), and its record is
//! emitted inside that correlation even when it finishes elsewhere. A line's stages are recorded from
//! wherever its work runs, through [`within`] and [`stage`]; a stage outside any line does nothing, so
//! the judgements that also serve other callers need no knowledge of the line they belong to.
//!
//! A line that needs more stages than the bound is emitted with `overflow=<n>` and is not counted in
//! the timing histograms: its stage list is incomplete, so its totals would be wrong.

use df_observe::AuditContext;
use std::{
    future::Future,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use super::{super::journey, timing_ledger};

/// Bounds one line's record. A line that needs more spans keeps the first ones and counts the rest.
const MAXIMUM_SPANS: usize = 64;
static NEXT_LINE: AtomicU64 = AtomicU64::new(1);

tokio::task_local! {
    static CURRENT: LineTiming;
}

/// The named stages of one hero line. Labels are the log vocabulary; a stage is never a free string.
/// Phase B adds the reply stages together with the hero-line code that records them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum LineStage {
    Classify,
    Addressee,
    Reason,
}

impl LineStage {
    /// Every stage, in the order the log vocabulary lists them.
    pub(in crate::gameplay) const ALL: [Self; 3] = [Self::Classify, Self::Addressee, Self::Reason];

    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::Classify => "classify",
            Self::Addressee => "addressee",
            Self::Reason => "reason",
        }
    }

    #[cfg(test)]
    fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|stage| stage.label() == label)
    }
}

/// One stage's place in the line: its start offset from the line's start and its duration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct Span {
    pub(in crate::gameplay) stage: LineStage,
    pub(in crate::gameplay) start: Duration,
    pub(in crate::gameplay) duration: Duration,
}

/// One stage as the record states it, in microseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct StageRecord {
    pub(in crate::gameplay) stage: LineStage,
    pub(in crate::gameplay) start_us: u64,
    pub(in crate::gameplay) duration_us: u64,
}

/// One finished hero line: its total and its stages. `truncated` is set when the line lost spans to
/// the bound; such a record is emitted but never counted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct LineRecord {
    pub(in crate::gameplay) total_us: u64,
    pub(in crate::gameplay) stages: Vec<StageRecord>,
    pub(in crate::gameplay) truncated: bool,
}

impl LineRecord {
    /// The record of a line from its own spans, so the histograms never depend on the emitted text.
    fn from_spans(total: Duration, spans: &[Span], overflow: usize) -> Self {
        Self {
            total_us: micros(total),
            stages: spans
                .iter()
                .map(|span| StageRecord {
                    stage: span.stage,
                    start_us: micros(span.start),
                    duration_us: micros(span.duration),
                })
                .collect(),
            truncated: overflow > 0,
        }
    }
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

struct Record {
    id: u64,
    started: Instant,
    context: AuditContext,
    /// The request input id, hex, when the line serves one request: the correlation the audit context lacks at dispatch.
    input: Option<String>,
    spans: Vec<Span>,
    overflow: usize,
    finished: bool,
}

/// The shared record of one hero line. Cloning shares the record. [`LineTiming::finish`] emits it at
/// most once; later calls return `None`.
#[derive(Clone)]
pub(in crate::gameplay) struct LineTiming(Arc<Mutex<Record>>);

impl LineTiming {
    /// A new line, started now, under the correlation in force on this thread.
    pub(in crate::gameplay) fn begin() -> Self {
        Self::begin_at(Instant::now())
    }

    fn begin_at(started: Instant) -> Self {
        let id = NEXT_LINE.fetch_add(1, Ordering::Relaxed);
        Self(Arc::new(Mutex::new(Record {
            id,
            started,
            context: df_observe::current_audit_context(),
            input: None,
            spans: Vec::new(),
            overflow: 0,
            finished: false,
        })))
    }

    /// The line's id, as the emitted text names it.
    #[cfg(test)]
    fn id(&self) -> u64 {
        self.with(|record| record.id)
    }

    /// The stages recorded so far, in the order they were recorded.
    #[cfg(test)]
    pub(in crate::gameplay) fn spans(&self) -> Vec<Span> {
        self.with(|record| record.spans.clone())
    }

    /// Records one stage that ran from `from` to `to`.
    pub(in crate::gameplay) fn record_between(&self, stage: LineStage, from: Instant, to: Instant) {
        self.with(|record| {
            let start = from.saturating_duration_since(record.started);
            let duration = to.saturating_duration_since(from);
            push(
                record,
                Span {
                    stage,
                    start,
                    duration,
                },
            );
        });
    }

    /// Records one stage that started at `started` and ended now.
    pub(in crate::gameplay) fn record_since(&self, stage: LineStage, started: Instant) {
        self.record_between(stage, started, Instant::now());
    }

    /// Emits the line once, now, and returns its text. Later calls return `None`.
    pub(in crate::gameplay) fn finish(&self) -> Option<String> {
        self.finish_at(Instant::now())
    }

    /// Emits the line once, as if it finished at `finished`, and returns its text. The line carries
    /// the correlation captured when it began, and the emission runs inside that correlation.
    pub(in crate::gameplay) fn finish_at(&self, finished: Instant) -> Option<String> {
        let (text, context, record) = {
            let mut record = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            if record.finished {
                return None;
            }
            record.finished = true;
            let total = finished.saturating_duration_since(record.started);
            let text = render(
                record.id,
                total,
                &record.spans,
                record.overflow,
                record.input.as_deref(),
            );
            let line = LineRecord::from_spans(total, &record.spans, record.overflow);
            (text, record.context.clone(), line)
        };
        let _correlation = df_observe::audit_scope(context);
        super::record_audit_line("gameplay.line_timing", text.clone());
        timing_ledger::record_finished_line(&record);
        Some(text)
    }

    fn with<T>(&self, read: impl FnOnce(&mut Record) -> T) -> T {
        let mut record = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        read(&mut record)
    }
}

/// Keeps the first spans up to the bound; later ones are counted so the line says it was cut short.
fn push(record: &mut Record, span: Span) {
    if record.spans.len() < MAXIMUM_SPANS {
        record.spans.push(span);
    } else {
        record.overflow += 1;
    }
}

/// Runs `future` as part of `line`, so every [`stage`] inside it lands in that line.
pub(in crate::gameplay) async fn within<F: Future>(line: LineTiming, future: F) -> F::Output {
    CURRENT.scope(line, future).await
}

/// Runs one request's work as one timed hero line. The line begins here, carries the request's input id
/// (hex) so its record can be matched to the request, runs `work` inside itself, and is emitted once when
/// `work` finishes, whether it returns `Ok` or `Err`. A dropped future emits nothing.
pub(in crate::gameplay) async fn submit_timed_line<T>(
    input_id: Option<&[u8]>,
    work: impl Future<Output = T>,
) -> T {
    let line = LineTiming::begin();
    if let Some(bytes) = input_id {
        line.with(|record| record.input = Some(journey::hex(bytes)));
    }
    let output = within(line.clone(), work).await;
    line.finish();
    output
}

/// Times `future` as `stage` of the current line. Outside any line the future simply runs.
pub(in crate::gameplay) async fn stage<F: Future>(stage: LineStage, future: F) -> F::Output {
    let Ok(line) = CURRENT.try_with(LineTiming::clone) else {
        return future.await;
    };
    let started = Instant::now();
    let output = future.await;
    line.record_since(stage, started);
    output
}

/// The line text: the id, the whole-millisecond total, the total in microseconds, the stages in the
/// order they ran, and how many spans the bound cut.
fn render(
    id: u64,
    total: Duration,
    spans: &[Span],
    overflow: usize,
    input: Option<&str>,
) -> String {
    let stages = spans
        .iter()
        .map(|span| {
            format!(
                "{}@{}+{}",
                span.stage.label(),
                span.start.as_micros(),
                span.duration.as_micros()
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "LINE_TIMING line={id} ms={} total_us={} stages={stages} overflow={overflow} input={}",
        total.as_millis(),
        total.as_micros(),
        input.unwrap_or("none"),
    )
}

/// Reads an emitted `LINE_TIMING` line back. Tests use it to check that the text states the record.
/// Any other line, an unknown stage label or a malformed number yields `None`.
#[cfg(test)]
fn parse(text: &str) -> Option<LineRecord> {
    let mut fields = text.split_ascii_whitespace();
    if fields.next()? != "LINE_TIMING" {
        return None;
    }
    let mut total_us = None;
    let mut stages = None;
    let mut truncated = false;
    for field in fields {
        if let Some(value) = field.strip_prefix("total_us=") {
            total_us = value.parse().ok();
        } else if let Some(value) = field.strip_prefix("stages=") {
            stages = Some(value);
        } else if let Some(value) = field.strip_prefix("overflow=") {
            truncated = value.parse::<usize>().ok()? > 0;
        }
    }
    Some(LineRecord {
        total_us: total_us?,
        stages: parse_stages(stages.unwrap_or_default())?,
        truncated,
    })
}

/// `label@start+duration` items, comma-separated; an empty list is a line with no stages.
#[cfg(test)]
fn parse_stages(text: &str) -> Option<Vec<StageRecord>> {
    text.split(',')
        .filter(|item| !item.is_empty())
        .map(|item| {
            let (label, timing) = item.split_once('@')?;
            let (start, duration) = timing.split_once('+')?;
            Some(StageRecord {
                stage: LineStage::from_label(label)?,
                start_us: start.parse().ok()?,
                duration_us: duration.parse().ok()?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn us(value: u64) -> Duration {
        Duration::from_micros(value)
    }

    /// The emitted lines that name this line's id, from the test tap of the shared audit path.
    fn emitted(id: u64) -> Vec<String> {
        crate::gameplay::actor::audit_lines_containing(&format!("LINE_TIMING line={id} "))
    }

    /// One real-clock line: begin, three timed stages, finish, then the emitted text parsed back and
    /// summarized. Each stage starts where the last one ended, so only the clock-read gaps separate the
    /// stage cover from the total.
    fn real_clock_line() -> (u64, u64) {
        let line = LineTiming::begin();
        let mut mark = Instant::now();
        for (stage, pause_ms) in [
            (LineStage::Classify, 30),
            (LineStage::Addressee, 20),
            (LineStage::Reason, 40),
        ] {
            std::thread::sleep(Duration::from_millis(pause_ms));
            line.record_since(stage, mark);
            mark = Instant::now();
        }
        let text = line.finish().expect("the line finishes once");
        let record = parse(&text).expect("the emitted text parses");
        let mut ledger = timing_ledger::TimingLedger::default();
        ledger.summarize_line(&record);
        (ledger.lines.covered_us, ledger.lines.total_us.sum_us())
    }

    #[test]
    fn real_clock_stages_cover_the_line_total_within_five_percent() {
        // Up to three attempts: a preempted clock read can break one run on a loaded machine.
        let mut attempts = Vec::new();
        for _ in 0..3 {
            let (covered, total) = real_clock_line();
            if covered.abs_diff(total) * 20 <= total {
                return;
            }
            attempts.push((covered, total));
        }
        panic!("no attempt was within five percent: (covered, total) {attempts:?}");
    }

    #[test]
    fn controlled_clock_stages_cover_the_line_total_within_five_percent() {
        // The same pipeline on a controlled clock: the expected total is exact.
        let t0 = Instant::now();
        let line = LineTiming::begin_at(t0);
        line.record_between(LineStage::Classify, t0, t0 + us(400_000));
        line.record_between(LineStage::Addressee, t0 + us(400_000), t0 + us(700_000));
        line.record_between(LineStage::Reason, t0 + us(700_000), t0 + us(980_000));
        let text = line
            .finish_at(t0 + us(1_000_000))
            .expect("the line finishes once");
        let record = parse(&text).expect("the emitted text parses");
        let mut ledger = timing_ledger::TimingLedger::default();
        ledger.summarize_line(&record);

        assert_eq!(ledger.lines.total_us.sum_us(), 1_000_000);
        assert_eq!(ledger.lines.covered_us, 980_000);
        // The 20 ms no stage covers is the unattributed time, 2% of the total.
        assert_eq!(ledger.lines.unattributed_us.sum_us(), 20_000);
    }

    /// The emitted lines that carry this request input id, from the test tap of the shared audit path.
    fn emitted_for_input(bytes: &[u8]) -> Vec<String> {
        let needle = format!("input={}", journey::hex(bytes));
        crate::gameplay::actor::audit_lines_containing(&needle)
            .into_iter()
            .filter(|line| line.starts_with("LINE_TIMING "))
            .collect()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_timed_line_emits_exactly_one_line_whether_the_work_succeeds_or_fails() {
        // A plain future, not a dialogue: the helper's contract is the one emitted line either way.
        let succeeding_input = [0x5a; 16];
        let failing_input = [0x6b; 16];
        let succeeded: Result<u8, u8> =
            submit_timed_line(Some(&succeeding_input), async { Ok(1) }).await;
        let failed: Result<u8, u8> =
            submit_timed_line(Some(&failing_input), async { Err(2) }).await;

        assert_eq!((succeeded, failed), (Ok(1), Err(2)));
        let ok_lines = emitted_for_input(&succeeding_input);
        let err_lines = emitted_for_input(&failing_input);
        assert_eq!(ok_lines.len(), 1, "{ok_lines:?}");
        assert_eq!(err_lines.len(), 1, "{err_lines:?}");
        assert!(err_lines[0].contains(&format!("input={}", journey::hex(&failing_input))));
    }

    #[test]
    fn a_line_finishes_once_and_is_emitted_once() {
        let t0 = Instant::now();
        let line = LineTiming::begin_at(t0);
        let id = line.id();
        assert!(line.finish_at(t0 + us(10)).is_some());
        assert!(line.finish_at(t0 + us(20)).is_none());
        assert!(line.finish().is_none());
        assert_eq!(emitted(id).len(), 1, "{:?}", emitted(id));
    }

    #[test]
    fn the_emitted_line_carries_the_correlation_captured_at_the_start() {
        let t0 = Instant::now();
        let line = {
            let _dispatch = df_observe::audit_scope(df_observe::AuditContext {
                operation: Some(4_242),
                ..df_observe::AuditContext::default()
            });
            LineTiming::begin_at(t0)
        };
        let id = line.id();
        // The finish runs later, outside that dispatch's correlation.
        line.finish_at(t0 + us(10)).expect("the line finishes once");
        let emitted = emitted(id);
        assert!(
            emitted.iter().any(|text| text.contains("op=4242")),
            "{emitted:?}"
        );
    }

    #[test]
    fn the_rendered_line_names_stages_in_order_with_micro_offsets_and_a_millisecond_total() {
        let spans = [
            Span {
                stage: LineStage::Classify,
                start: us(10),
                duration: us(2_500),
            },
            Span {
                stage: LineStage::Reason,
                start: us(2_510),
                duration: us(40),
            },
        ];
        let line = render(42, us(2_600), &spans, 0, None);

        assert_eq!(
            line,
            "LINE_TIMING line=42 ms=2 total_us=2600 stages=classify@10+2500,reason@2510+40 \
             overflow=0 input=none"
        );
        let expected = LineRecord::from_spans(us(2_600), &spans, 0);
        assert_eq!(parse(&line), Some(expected));
    }

    #[test]
    fn a_record_built_from_spans_states_the_same_stages_as_its_text() {
        let spans = [Span {
            stage: LineStage::Addressee,
            start: us(5),
            duration: us(70),
        }];
        let record = LineRecord::from_spans(us(100), &spans, 2);
        assert!(record.truncated);
        assert_eq!(
            record.stages,
            vec![StageRecord {
                stage: LineStage::Addressee,
                start_us: 5,
                duration_us: 70,
            }]
        );
    }

    #[test]
    fn other_lines_unknown_stages_and_malformed_numbers_do_not_parse() {
        assert_eq!(parse("CALL timing call=View"), None);
        assert_eq!(parse("LINE_TIMING line=1 total_us=x stages="), None);
        assert_eq!(parse("LINE_TIMING line=1 total_us=5 stages=warp@0+1"), None);
        assert_eq!(
            parse("LINE_TIMING line=1 total_us=5 stages=classify@0+oops"),
            None
        );
        assert_eq!(
            parse("LINE_TIMING line=1 total_us=5 stages= overflow=0"),
            Some(LineRecord {
                total_us: 5,
                stages: Vec::new(),
                truncated: false,
            })
        );
    }

    #[test]
    fn every_stage_label_is_in_the_closed_list_once() {
        let labels: Vec<&str> = LineStage::ALL.iter().map(|stage| stage.label()).collect();
        assert_eq!(labels, ["classify", "addressee", "reason"]);
        assert_eq!(LineStage::from_label("reason"), Some(LineStage::Reason));
    }

    #[test]
    fn spans_past_the_bound_are_counted_not_stored() {
        let t0 = Instant::now();
        let line = LineTiming::begin_at(t0);
        for _ in 0..(MAXIMUM_SPANS + 3) {
            line.record_between(LineStage::Reason, t0, t0);
        }
        let (stored, overflow) = line.with(|record| (record.spans.len(), record.overflow));
        assert_eq!((stored, overflow), (MAXIMUM_SPANS, 3));
    }

    #[test]
    fn line_ids_are_distinct_per_line() {
        assert_ne!(LineTiming::begin().id(), LineTiming::begin().id());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_stage_inside_a_line_is_recorded_and_outside_one_is_not() {
        let line = LineTiming::begin();
        let observed = line.clone();
        let value = within(line, async { stage(LineStage::Reason, async { 7 }).await }).await;
        assert_eq!(value, 7);
        assert_eq!(observed.spans().len(), 1);

        // Outside a line the stage runs and records nothing.
        assert_eq!(stage(LineStage::Reason, async { 8 }).await, 8);
    }
}
