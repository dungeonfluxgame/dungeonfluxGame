//! Synthetic native fixture CLI: emit or load synthetic logs, replay a spool, query a root.
//! It prints one JSON object per line on stdout and never touches a non-fixture root.
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use df_observe::{
        AnyValue, Durability, IngestReceipt, LogInput, NativeProducer, PendingReceipt, ProducerId,
        RecordId, Severity, SourceSequence, TelemetryError, TelemetryLimits,
    };
    use df_telemetry::{
        BarrierStage, DiagnosticReader, FailureBarrier, MAXIMUM_PAGE_ROWS, MAXIMUM_ROOT_RECORDS,
        QueryFilter, Store,
    };
    use df_types::BuildIdentity;
    use std::{
        collections::VecDeque,
        io::{self, Write},
        path::Path,
        sync::mpsc,
        time::{Duration, Instant, SystemTime},
    };
    const SOURCE: &str = match option_env!("DF_FIXTURE_BUILD") {
        Some(source) => source,
        None => "unknown-build",
    };
    /// `emit` waits for receipts in groups of this many, keeping the receipt queue bounded.
    const EMIT_RECEIPT_GROUP: usize = 128;
    /// `load` fails once this many receipts are outstanding: ingestion is not keeping up.
    const MAXIMUM_LOAD_PENDING_RECEIPTS: usize = 1024;
    /// Wait for one durable receipt, a flush, or a store shutdown.
    const DURABLE_WAIT: Duration = Duration::from_secs(30);
    /// Wait for the worker to reach a requested crash barrier.
    const BARRIER_REACHED_WAIT: Duration = Duration::from_secs(5);
    /// Without an external fixture owner killing the process, a held barrier releases after this.
    const BARRIER_HOLD: Duration = Duration::from_secs(25);
    /// Bytes of synthetic body text per emitted record.
    const SYNTHETIC_BODY_BYTES: usize = 1024;
    /// Source timestamp of the first synthetic record.
    const SYNTHETIC_EPOCH_NANOS: u64 = 1_700_000_000_000_000_000;
    const NANOS_PER_SECOND: u64 = 1_000_000_000;

    struct BarrierControl {
        failure: FailureBarrier,
        reached: mpsc::Receiver<BarrierStage>,
        release: mpsc::Sender<()>,
    }
    fn barrier(stage: Option<&str>) -> Result<Option<BarrierControl>, TelemetryError> {
        let stage = match stage {
            None => return Ok(None),
            Some("preaccept") => BarrierStage::BeforeSpool,
            Some("spooled") => BarrierStage::Spooled,
            Some("committed") => BarrierStage::CommittedBeforeCheckpoint,
            _ => return Err(TelemetryError::Malformed),
        };
        let (reached, receiver) = mpsc::sync_channel(1);
        let (release, released) = mpsc::channel();
        Ok(Some(BarrierControl {
            failure: FailureBarrier {
                stage,
                reached,
                release: released,
            },
            reached: receiver,
            release,
        }))
    }
    pub fn run() -> Result<(), TelemetryError> {
        let args: Vec<String> = std::env::args().collect();
        let command = args
            .get(1)
            .map(String::as_str)
            .ok_or(TelemetryError::Malformed)?;
        let root = Path::new(args.get(2).ok_or(TelemetryError::Malformed)?);
        match command {
            "emit" | "load" => run_emit(root, command == "load", &args),
            "replay" => run_replay(root),
            "query" => run_query(root, &args),
            _ => Err(TelemetryError::Malformed),
        }
    }

    /// What one `emit` or `load` run asks for.
    struct EmitPlan {
        count: u64,
        first_sequence: i64,
        /// Records per second for `load`; `emit` sends as fast as admission allows.
        rate_per_second: Option<u64>,
    }
    fn parse_argument<T: std::str::FromStr>(
        args: &[String],
        index: usize,
    ) -> Result<T, TelemetryError> {
        args.get(index)
            .ok_or(TelemetryError::Malformed)?
            .parse::<T>()
            .map_err(|_| TelemetryError::Malformed)
    }
    /// `emit ROOT COUNT FIRST_SEQUENCE` or `load ROOT RATE SECONDS`.
    fn emit_plan(is_load: bool, args: &[String]) -> Result<EmitPlan, TelemetryError> {
        let count_or_rate = parse_argument::<u64>(args, 3)?;
        let fourth_argument = parse_argument::<i64>(args, 4)?;
        let plan = if is_load {
            if !matches!(count_or_rate, 100 | 1000) || !(1..=10).contains(&fourth_argument) {
                return Err(TelemetryError::InvalidLimits);
            }
            let seconds =
                u64::try_from(fourth_argument).map_err(|_| TelemetryError::InvalidLimits)?;
            EmitPlan {
                count: count_or_rate
                    .checked_mul(seconds)
                    .ok_or(TelemetryError::InvalidLimits)?,
                first_sequence: 1,
                rate_per_second: Some(count_or_rate),
            }
        } else {
            EmitPlan {
                count: count_or_rate,
                first_sequence: fourth_argument,
                rate_per_second: None,
            }
        };
        // Each emitted record takes one spool frame and one identity of the root's lifetime.
        let within_root =
            usize::try_from(plan.count).is_ok_and(|count| count <= MAXIMUM_ROOT_RECORDS);
        if plan.count == 0 || !within_root {
            return Err(TelemetryError::InvalidLimits);
        }
        Ok(plan)
    }

    /// Receipt and pacing measurements of one `emit` or `load` run.
    #[derive(Default)]
    struct EmitTally {
        emit_micros: Vec<u128>,
        receipt_micros: Vec<u128>,
        spooled: u64,
        committed: u64,
        rejected: u64,
        maximum_late_micros: u128,
        peak_items: usize,
        peak_bytes: usize,
        peak_backlog: usize,
    }
    impl EmitTally {
        fn record_receipt(&mut self, sent: Instant, receipt: IngestReceipt) {
            self.receipt_micros.push(sent.elapsed().as_micros());
            match receipt.durability {
                Durability::Spooled => self.spooled += 1,
                Durability::Committed => self.committed += 1,
            }
        }
    }

    fn synthetic_log(index: u64) -> LogInput {
        let timestamp = SystemTime::UNIX_EPOCH
            + Duration::from_nanos(SYNTHETIC_EPOCH_NANOS.saturating_add(index));
        LogInput {
            timestamp,
            observed_timestamp: timestamp + Duration::from_nanos(7),
            severity: Severity::Info,
            body: AnyValue::String("s".repeat(SYNTHETIC_BODY_BYTES).into()),
            attributes: vec![
                (
                    "df.session".into(),
                    AnyValue::String("synthetic-session".into()),
                ),
                (
                    "df.operation".into(),
                    AnyValue::String("synthetic-operation".into()),
                ),
            ],
            trace: None,
        }
    }

    fn run_emit(root: &Path, is_load: bool, args: &[String]) -> Result<(), TelemetryError> {
        let plan = emit_plan(is_load, args)?;
        let control = barrier(args.get(5).map(String::as_str))?;
        if control.is_some() && plan.count != 1 {
            return Err(TelemetryError::InvalidLimits);
        }
        let (failure, control) = match control {
            Some(control) => (
                Some(control.failure),
                Some((control.reached, control.release)),
            ),
            None => (None, None),
        };
        let mut store = Store::open(root, TelemetryLimits::default(), failure)?;
        let build = BuildIdentity::new(
            Some(SOURCE),
            Some(SOURCE),
            Some("native-only-unqualified"),
            Some("native-fixture-v1"),
            Some("synthetic-no-private-data"),
        )
        .map_err(|_| TelemetryError::InvalidIdentity)?;
        let mut producer = NativeProducer::new(
            ProducerId::new([1; 16])?,
            SourceSequence::new(plan.first_sequence)?,
            &build,
            store.ingress(),
            TelemetryLimits::default(),
        )?;
        let started = Instant::now();
        let scheduled_at =
            |index: u64, rate: u64| started + Duration::from_nanos(index * NANOS_PER_SECOND / rate);
        let mut pending = VecDeque::<(Instant, PendingReceipt)>::new();
        let mut tally = EmitTally::default();
        for index in 0..plan.count {
            if let Some(rate) = plan.rate_per_second {
                let target = scheduled_at(index, rate);
                if let Some(delay) = target.checked_duration_since(Instant::now()) {
                    std::thread::sleep(delay);
                }
                tally.maximum_late_micros = tally
                    .maximum_late_micros
                    .max(Instant::now().saturating_duration_since(target).as_micros());
            }
            let number = (plan.first_sequence as u128)
                .checked_add(u128::from(index))
                .ok_or(TelemetryError::SequenceExhausted)?;
            let key = producer.reserve(RecordId::new(number.to_be_bytes())?)?;
            let emit_start = Instant::now();
            match producer.emit_log(key, synthetic_log(index)) {
                Ok(captured) => pending.push_back((emit_start, captured.pending)),
                Err(TelemetryError::Capacity) => tally.rejected += 1,
                Err(error) => return Err(error),
            }
            tally.emit_micros.push(emit_start.elapsed().as_micros());
            if plan.rate_per_second.is_some() {
                while let Some((sent, ticket)) = pending.front() {
                    let Some(receipt) = ticket.try_receive()? else {
                        break;
                    };
                    tally.record_receipt(*sent, receipt);
                    pending.pop_front();
                }
                if pending.len() > MAXIMUM_LOAD_PENDING_RECEIPTS {
                    return Err(TelemetryError::Capacity);
                }
            } else if pending.len() == EMIT_RECEIPT_GROUP {
                for (sent, ticket) in pending.drain(..) {
                    tally.record_receipt(sent, ticket.wait(DURABLE_WAIT)?);
                }
            }
            store.set_time(started.elapsed())?;
            let health = store.health();
            tally.peak_items = tally.peak_items.max(health.pending_items);
            tally.peak_bytes = tally.peak_bytes.max(health.pending_bytes);
            tally.peak_backlog = tally.peak_backlog.max(health.backlog_files);
        }
        if let Some(rate) = plan.rate_per_second
            && let Some(delay) =
                scheduled_at(plan.count, rate).checked_duration_since(Instant::now())
        {
            std::thread::sleep(delay);
        }
        let offered_window_seconds = started.elapsed().as_secs_f64();
        let drain_start = Instant::now();
        if let Some((reached, release)) = control {
            let stage = reached
                .recv_timeout(BARRIER_REACHED_WAIT)
                .map_err(|_| TelemetryError::Deadline)?;
            println!("{{\"barrier\":\"{stage:?}\",\"source\":\"{SOURCE}\"}}");
            io::stdout().flush().map_err(|_| TelemetryError::Io)?;
            // The external fixture owner kills at this observed state. Without that owner,
            // this intentionally controlled failure releases after the hold.
            std::thread::park_timeout(BARRIER_HOLD);
            release.send(()).map_err(|_| TelemetryError::Closed)?;
        }
        for (sent, ticket) in pending {
            tally.record_receipt(sent, ticket.wait(DURABLE_WAIT)?);
        }
        producer.shutdown(Duration::from_secs(1))?;
        store.flush(DURABLE_WAIT)?;
        let health = store.health();
        println!(
            concat!(
                "{{\"source\":\"{}\",\"requested\":{},\"rejected\":{},",
                "\"spooled_receipts\":{},\"committed_receipts\":{},\"watermark\":{},",
                "\"committed_records\":{},\"checkpoint\":{},\"seconds\":{},",
                "\"offered_window_seconds\":{},\"drain_seconds\":{},\"emit_p95_micros\":{},",
                "\"durable_receipt_p95_micros\":{},\"maximum_schedule_late_micros\":{},",
                "\"peak_pending_items\":{},\"peak_pending_bytes\":{},",
                "\"peak_backlog_files\":{},\"unconfirmed_gaps\":{}}}",
            ),
            SOURCE,
            plan.count,
            tally.rejected,
            tally.spooled,
            tally.committed,
            health.committed_watermark,
            health.committed,
            health.checkpoint,
            started.elapsed().as_secs_f64(),
            offered_window_seconds,
            drain_start.elapsed().as_secs_f64(),
            percentile(&mut tally.emit_micros),
            percentile(&mut tally.receipt_micros),
            tally.maximum_late_micros,
            tally.peak_items,
            tally.peak_bytes,
            tally.peak_backlog,
            health.unconfirmed_gaps
        );
        store.shutdown(DURABLE_WAIT)
    }

    fn run_replay(root: &Path) -> Result<(), TelemetryError> {
        let mut store = Store::open(root, TelemetryLimits::default(), None)?;
        store.flush(DURABLE_WAIT)?;
        let health = store.health();
        println!(
            concat!(
                "{{\"watermark\":{},\"checkpoint\":{},\"replayed\":{},\"duplicates\":{},",
                "\"backlog\":{}}}",
            ),
            health.committed_watermark,
            health.checkpoint,
            health.committed,
            health.duplicates,
            health.backlog_files
        );
        store.shutdown(DURABLE_WAIT)
    }

    /// `query ROOT [LIMIT] [CURSOR] [name=value...]`.
    fn run_query(root: &Path, args: &[String]) -> Result<(), TelemetryError> {
        let mut reader = DiagnosticReader::open(root)?;
        let limit = args
            .get(3)
            .map(|value| value.parse::<usize>())
            .transpose()
            .map_err(|_| TelemetryError::Malformed)?
            .unwrap_or(MAXIMUM_PAGE_ROWS);
        let cursor = args
            .get(4)
            .map(|value| value.parse::<u64>())
            .transpose()
            .map_err(|_| TelemetryError::Malformed)?
            .unwrap_or(0);
        let filter = query_filter(args.iter().skip(5))?;
        let page = reader.query(&filter, cursor, limit)?;
        println!(
            concat!(
                "{{\"watermark\":{},\"spool_checkpoint\":{},\"next\":{},\"count\":{},",
                "\"write_denied\":{},\"unconfirmed_capture_gaps\":{},\"emergency_bytes\":{}}}",
            ),
            page.committed_watermark,
            page.spool_checkpoint,
            page.next_cursor,
            page.records.len(),
            reader.verify_write_denied()?,
            page.unconfirmed_capture_gaps,
            page.emergency_bytes.len()
        );
        for source in &page.source_watermarks {
            println!(
                concat!(
                    "{{\"source_watermark\":\"{}\",\"accepted_records\":{},",
                    "\"first_observed_sequence\":{},\"highest_sequence\":{},",
                    "\"missing_between_observed\":{},\"producer_lifetime_complete\":false,",
                    "\"retention\":\"finite fixture quota; all accepted spool frames retained; ",
                    "no automatic deletion\"}}",
                ),
                source.producer.hex(),
                source.accepted_records,
                source.first_observed_sequence,
                source.highest_sequence,
                source.missing_through_highest
            );
        }
        for record in page.records {
            println!(
                concat!(
                    "{{\"position\":{},\"producer\":\"{}\",\"record\":\"{}\",\"sequence\":{},",
                    "\"signal\":\"{:?}\",\"source_time\":{},\"observed_time\":{},",
                    "\"otlp_hex\":\"{}\"}}",
                ),
                record.position,
                record.producer.hex(),
                record.record.hex(),
                record.source_sequence,
                record.signal,
                record.source_time_unix_nanos,
                record
                    .observed_time_unix_nanos
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "null".into()),
                hex(&record.otlp)
            );
        }
        Ok(())
    }
    fn query_filter<'a>(
        arguments: impl Iterator<Item = &'a String>,
    ) -> Result<QueryFilter, TelemetryError> {
        let parse_number = |value: &str| value.parse().map_err(|_| TelemetryError::Malformed);
        let mut filter = QueryFilter::default();
        for argument in arguments {
            let (name, value) = argument.split_once('=').ok_or(TelemetryError::Malformed)?;
            match name {
                "producer" => filter.producer = Some(ProducerId::from_hex(value)?),
                "session" => filter.session = Some(value.to_owned()),
                "operation" => filter.operation = Some(value.to_owned()),
                "build" => filter.build = Some(value.to_owned()),
                "severity" => {
                    filter.minimum_severity =
                        Some(value.parse().map_err(|_| TelemetryError::Malformed)?)
                }
                // A trace id is 16 bytes of lower-case hex, the same spelling as a producer id.
                "trace" => filter.trace = Some(ProducerId::from_hex(value)?.bytes()),
                "from" => filter.from_unix_nanos = Some(parse_number(value)?),
                "through" => filter.through_unix_nanos = Some(parse_number(value)?),
                _ => return Err(TelemetryError::Malformed),
            }
        }
        Ok(filter)
    }
    fn percentile(values: &mut [u128]) -> u128 {
        values.sort_unstable();
        values
            .get(values.len().saturating_sub(1) * 95 / 100)
            .copied()
            .unwrap_or(0)
    }
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(error) = native::run() {
        eprintln!(
            concat!(
                "df-telemetry: {}; usage: emit ROOT COUNT FIRST_SEQUENCE ",
                "[preaccept|spooled|committed] | load ROOT RATE SECONDS | replay ROOT | ",
                "query ROOT [LIMIT] [CURSOR] [name=value...]",
            ),
            error
        );
        std::process::exit(1);
    }
}
#[cfg(target_arch = "wasm32")]
compile_error!(
    "df-telemetry is a native-only synthetic executable; no browser storage is implemented"
);
