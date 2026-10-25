#[cfg(not(target_arch = "wasm32"))]
mod native {
    use df_observe::{
        AnyValue, LogInput, NativeProducer, PendingReceipt, ProducerId, RecordId, Severity,
        SourceSequence, TelemetryError, TelemetryLimits,
    };
    use df_telemetry::{BarrierStage, DiagnosticReader, FailureBarrier, QueryFilter, Store};
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
            "emit" | "load" => {
                let count_or_rate = args
                    .get(3)
                    .ok_or(TelemetryError::Malformed)?
                    .parse::<u64>()
                    .map_err(|_| TelemetryError::Malformed)?;
                let fourth_argument = args
                    .get(4)
                    .ok_or(TelemetryError::Malformed)?
                    .parse::<i64>()
                    .map_err(|_| TelemetryError::Malformed)?;
                let (count, start, rate) = if command == "load" {
                    if !matches!(count_or_rate, 100 | 1000) || !(1..=10).contains(&fourth_argument)
                    {
                        return Err(TelemetryError::InvalidLimits);
                    }
                    (
                        count_or_rate
                            .checked_mul(fourth_argument as u64)
                            .ok_or(TelemetryError::InvalidLimits)?,
                        1,
                        Some(count_or_rate),
                    )
                } else {
                    (count_or_rate, fourth_argument, None)
                };
                if count == 0 || count > 32768 {
                    return Err(TelemetryError::InvalidLimits);
                }
                let control = barrier(args.get(5).map(String::as_str))?;
                if control.is_some() && count != 1 {
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
                    SourceSequence::new(start)?,
                    &build,
                    store.ingress(),
                    TelemetryLimits::default(),
                )?;
                let time = Instant::now();
                let mut pending = VecDeque::<(Instant, PendingReceipt)>::new();
                let mut emit_micros = Vec::new();
                let mut receipt_micros = Vec::new();
                let mut spooled = 0;
                let mut committed = 0;
                let mut maximum_late_micros = 0u128;
                let mut peak_items = 0;
                let mut peak_bytes = 0;
                let mut peak_backlog = 0;
                let mut rejected = 0;
                for index in 0..count {
                    if let Some(rate) = rate {
                        let target = time + Duration::from_nanos(index * 1000000000 / rate);
                        if let Some(delay) = target.checked_duration_since(Instant::now()) {
                            std::thread::sleep(delay);
                        }
                    }
                    if let Some(rate) = rate {
                        let target = time + Duration::from_nanos(index * 1000000000 / rate);
                        maximum_late_micros = maximum_late_micros
                            .max(Instant::now().saturating_duration_since(target).as_micros());
                    }
                    let number = (start as u128)
                        .checked_add(u128::from(index))
                        .ok_or(TelemetryError::SequenceExhausted)?;
                    let key = producer.reserve(RecordId::new(number.to_be_bytes())?)?;
                    let timestamp = SystemTime::UNIX_EPOCH
                        + Duration::from_nanos(1700000000000000000u64.saturating_add(index));
                    let emit_start = Instant::now();
                    match producer.emit_log(
                        key,
                        LogInput {
                            timestamp,
                            observed_timestamp: timestamp + Duration::from_nanos(7),
                            severity: Severity::Info,
                            body: AnyValue::String("s".repeat(1024).into()),
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
                        },
                    ) {
                        Ok(captured) => {
                            pending.push_back((emit_start, captured.pending));
                        }
                        Err(TelemetryError::Capacity) => {
                            rejected += 1;
                        }
                        Err(error) => return Err(error),
                    }
                    emit_micros.push(emit_start.elapsed().as_micros());
                    if rate.is_some() {
                        while let Some((sent, ticket)) = pending.front() {
                            let Some(receipt) = ticket.try_receive()? else {
                                break;
                            };
                            receipt_micros.push(sent.elapsed().as_micros());
                            match receipt.durability {
                                df_observe::Durability::Spooled => spooled += 1,
                                df_observe::Durability::Committed => committed += 1,
                            }
                            pending.pop_front();
                        }
                        if pending.len() > 1024 {
                            return Err(TelemetryError::Capacity);
                        }
                    } else if pending.len() == 128 {
                        for (sent, ticket) in pending.drain(..) {
                            let receipt = ticket.wait(Duration::from_secs(30))?;
                            receipt_micros.push(sent.elapsed().as_micros());
                            match receipt.durability {
                                df_observe::Durability::Spooled => spooled += 1,
                                df_observe::Durability::Committed => committed += 1,
                            }
                        }
                    }
                    store.set_time(time.elapsed())?;
                    let health = store.health();
                    peak_items = peak_items.max(health.pending_items);
                    peak_bytes = peak_bytes.max(health.pending_bytes);
                    peak_backlog = peak_backlog.max(health.backlog_files);
                }
                if let Some(rate) = rate {
                    let target = time + Duration::from_nanos(count * 1000000000 / rate);
                    if let Some(delay) = target.checked_duration_since(Instant::now()) {
                        std::thread::sleep(delay);
                    }
                }
                let offered_window_seconds = time.elapsed().as_secs_f64();
                let drain_start = Instant::now();
                if let Some((reached, release)) = control {
                    let stage = reached
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|_| TelemetryError::Deadline)?;
                    println!("{{\"barrier\":\"{stage:?}\",\"source\":\"{SOURCE}\"}}");
                    io::stdout().flush().map_err(|_| TelemetryError::Io)?;
                    // The external fixture owner kills at this observed state. Without that
                    // owner, this intentionally controlled failure releases within 25 seconds.
                    std::thread::park_timeout(Duration::from_secs(25));
                    release.send(()).map_err(|_| TelemetryError::Closed)?;
                }
                for (sent, ticket) in pending {
                    let receipt = ticket.wait(Duration::from_secs(30))?;
                    receipt_micros.push(sent.elapsed().as_micros());
                    match receipt.durability {
                        df_observe::Durability::Spooled => spooled += 1,
                        df_observe::Durability::Committed => committed += 1,
                    }
                }
                producer.shutdown(Duration::from_secs(1))?;
                store.flush(Duration::from_secs(30))?;
                let health = store.health();
                println!(
                    "{{\"source\":\"{SOURCE}\",\"requested\":{count},\"rejected\":{rejected},\"spooled_receipts\":{spooled},\"committed_receipts\":{committed},\"watermark\":{},\"committed_records\":{},\"checkpoint\":{},\"seconds\":{},\"offered_window_seconds\":{offered_window_seconds},\"drain_seconds\":{},\"emit_p95_micros\":{},\"durable_receipt_p95_micros\":{},\"maximum_schedule_late_micros\":{maximum_late_micros},\"peak_pending_items\":{peak_items},\"peak_pending_bytes\":{peak_bytes},\"peak_backlog_files\":{peak_backlog},\"unconfirmed_gaps\":{}}}",
                    health.committed_watermark,
                    health.committed,
                    health.checkpoint,
                    time.elapsed().as_secs_f64(),
                    drain_start.elapsed().as_secs_f64(),
                    percentile(&mut emit_micros),
                    percentile(&mut receipt_micros),
                    health.unconfirmed_gaps
                );
                store.shutdown(Duration::from_secs(30))?;
            }
            "replay" => {
                let mut store = Store::open(root, TelemetryLimits::default(), None)?;
                store.flush(Duration::from_secs(30))?;
                let health = store.health();
                println!(
                    "{{\"watermark\":{},\"checkpoint\":{},\"replayed\":{},\"duplicates\":{},\"backlog\":{}}}",
                    health.committed_watermark,
                    health.checkpoint,
                    health.committed,
                    health.duplicates,
                    health.backlog_files
                );
                store.shutdown(Duration::from_secs(30))?;
            }
            "query" => {
                let mut reader = DiagnosticReader::open(root)?;
                let limit = args
                    .get(3)
                    .map(|value| value.parse::<usize>())
                    .transpose()
                    .map_err(|_| TelemetryError::Malformed)?
                    .unwrap_or(100);
                let cursor = args
                    .get(4)
                    .map(|value| value.parse::<u64>())
                    .transpose()
                    .map_err(|_| TelemetryError::Malformed)?
                    .unwrap_or(0);
                let mut filter = QueryFilter::default();
                for argument in args.iter().skip(5) {
                    let (name, value) =
                        argument.split_once('=').ok_or(TelemetryError::Malformed)?;
                    match name {
                        "producer" => filter.producer = Some(ProducerId::from_hex(value)?),
                        "session" => filter.session = Some(value.to_owned()),
                        "operation" => filter.operation = Some(value.to_owned()),
                        "build" => filter.build = Some(value.to_owned()),
                        "severity" => {
                            filter.minimum_severity =
                                Some(value.parse().map_err(|_| TelemetryError::Malformed)?)
                        }
                        "trace" => filter.trace = Some(ProducerId::from_hex(value)?.bytes()),
                        "from" => {
                            filter.from_unix_nanos =
                                Some(value.parse().map_err(|_| TelemetryError::Malformed)?)
                        }
                        "through" => {
                            filter.through_unix_nanos =
                                Some(value.parse().map_err(|_| TelemetryError::Malformed)?)
                        }
                        _ => return Err(TelemetryError::Malformed),
                    }
                }
                let page = reader.query(&filter, cursor, limit)?;
                println!(
                    "{{\"watermark\":{},\"spool_checkpoint\":{},\"next\":{},\"count\":{},\"write_denied\":{},\"unconfirmed_capture_gaps\":{},\"emergency_bytes\":{}}}",
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
                        "{{\"source_watermark\":\"{}\",\"accepted_records\":{},\"first_observed_sequence\":{},\"highest_sequence\":{},\"missing_between_observed\":{},\"producer_lifetime_complete\":false,\"retention\":\"finite fixture quota; all accepted spool frames retained; no automatic deletion\"}}",
                        source.producer.hex(),
                        source.accepted_records,
                        source.first_observed_sequence,
                        source.highest_sequence,
                        source.missing_through_highest
                    );
                }
                for record in page.records {
                    println!(
                        "{{\"position\":{},\"producer\":\"{}\",\"record\":\"{}\",\"sequence\":{},\"signal\":\"{:?}\",\"source_time\":{},\"observed_time\":{},\"otlp_hex\":\"{}\"}}",
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
            }
            _ => return Err(TelemetryError::Malformed),
        }
        Ok(())
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
            "df-telemetry: {error}; usage: emit ROOT COUNT FIRST_SEQUENCE [preaccept|spooled|committed] | load ROOT RATE SECONDS | replay ROOT | query ROOT [LIMIT] [CURSOR] [name=value...]"
        );
        std::process::exit(1);
    }
}
#[cfg(target_arch = "wasm32")]
compile_error!(
    "df-telemetry is a native-only synthetic executable; no browser storage is implemented"
);
