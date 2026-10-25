use crate::CaptureKey;
use df_types::{BuildIdentity, BuildRevision, OperationId, SessionId};
use opentelemetry::trace::SpanContext;
use opentelemetry_proto::tonic::{
    collector::logs::v1::ExportLogsServiceRequest,
    common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value::Value},
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs, SeverityNumber},
    resource::v1::Resource,
};
use prost::Message;
use std::{rc::Rc, time::Duration};

const MAXIMUM_RECORDS: usize = 128;
const MAXIMUM_BYTES: usize = 256 * 1024;
const MAXIMUM_RECORD_BYTES: usize = 8192;
const MAXIMUM_UPLOAD_RECORDS: usize = 32;
const MAXIMUM_UPLOAD_BYTES: usize = 64 * 1024;
const UPLOAD_DEADLINE: Duration = Duration::from_secs(10);

/// Browser capture selects only approved typed fields before standard OTLP encoding.
/// Names/statuses come from static instrumentation, never runtime error or player text.
/// Capture identities and revision labels are correlation, not authorization or uniqueness proof.
pub struct BrowserLog {
    pub timestamp_unix_nanos: u64,
    pub observed_unix_nanos: u64,
    pub severity: SeverityNumber,
    pub event_name: &'static str,
    pub status: &'static str,
    pub measured_bytes: Option<i64>,
    pub session: Option<SessionId>,
    pub operation: Option<OperationId>,
    pub trace: Option<SpanContext>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BufferError {
    Empty,
    Oversized,
    Capacity,
    IdentityExhausted,
    StaleLease,
    Deadline,
    NotExpired,
    ClockOverflow,
    InvalidMeasurement,
}

/// Classified upload termination; none asserts that server-owned work was cancelled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadFailure {
    Transport,
    Cancelled,
    PartialOrRejected,
}

/// Payload-free diagnostic state. A saturated refusal count means at least this many.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferSnapshot {
    pub retained_records: usize,
    pub retained_bytes: usize,
    pub refused_records: usize,
    pub upload_active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LeaseIdentity {
    generation: u64,
    attempt: u64,
    first_record: u64,
    records: usize,
    bytes: usize,
    deadline: Duration,
}

/// Opaque token for one upload. It retains owner identity, never payload.
/// Callers own and dispose callbacks/tokens; there is no public cloning or reconstruction.
#[derive(Debug)]
pub struct UploadLease {
    owner: Rc<u8>,
    identity: LeaseIdentity,
}
impl UploadLease {
    pub fn records(&self) -> usize {
        self.identity.records
    }
    pub fn bytes(&self) -> usize {
        self.identity.bytes
    }
    pub fn deadline(&self) -> Duration {
        self.identity.deadline
    }
}
struct RetainedRecord {
    ordinal: u64,
    bytes: Box<[u8]>,
}

/// Best-effort page/binding memory, bounded to 128 records/256 KiB encoded payload.
/// One prefix remains included in that bound until a matched terminal durable receipt.
/// This owner does not implement upload RPCs, timers, browser crash storage, or retries.
pub struct BrowserBuffer {
    owner: Rc<u8>,
    generation: u64,
    slots: [Option<RetainedRecord>; MAXIMUM_RECORDS],
    head: usize,
    count: usize,
    bytes: usize,
    refused: usize,
    next_record: u64,
    next_attempt: u64,
    lease: Option<LeaseIdentity>,
}
impl BrowserBuffer {
    pub fn new(generation: u64) -> Self {
        Self {
            owner: Rc::new(0),
            generation,
            slots: std::array::from_fn(|_| None),
            head: 0,
            count: 0,
            bytes: 0,
            refused: 0,
            next_record: 1,
            next_attempt: 1,
            lease: None,
        }
    }
    pub fn snapshot(&self) -> BufferSnapshot {
        BufferSnapshot {
            retained_records: self.count,
            retained_bytes: self.bytes,
            refused_records: self.refused,
            upload_active: self.lease.is_some(),
        }
    }
    fn refuse(&mut self, error: BufferError) -> BufferError {
        self.refused = self.refused.saturating_add(1);
        error
    }
    /// Capture one no-body OTLP log. Caller supplies a unique canonical capture key.
    /// Refusal retains neither the rejected input nor its encoded bytes; older records stay owned.
    pub fn retain_log(
        &mut self,
        build: &BuildIdentity,
        key: CaptureKey,
        record: BrowserLog,
    ) -> Result<(), BufferError> {
        if record.event_name.is_empty() {
            return Err(self.refuse(BufferError::Empty));
        }
        if record.event_name.len() > 256 || record.status.len() > 256 {
            return Err(self.refuse(BufferError::Oversized));
        }
        if record.measured_bytes.is_some_and(|bytes| bytes < 0) {
            return Err(self.refuse(BufferError::InvalidMeasurement));
        }
        if self.count == MAXIMUM_RECORDS {
            return Err(self.refuse(BufferError::Capacity));
        }
        let request = encode_log(build, key, record);
        self.retain_encoded(&request)
    }
    fn retain_encoded(&mut self, request: &impl Message) -> Result<(), BufferError> {
        let length = request.encoded_len();
        if length == 0 {
            return Err(self.refuse(BufferError::Empty));
        }
        if length > MAXIMUM_RECORD_BYTES {
            return Err(self.refuse(BufferError::Oversized));
        }
        if self.count == MAXIMUM_RECORDS
            || self
                .bytes
                .checked_add(length)
                .is_none_or(|sum| sum > MAXIMUM_BYTES)
        {
            return Err(self.refuse(BufferError::Capacity));
        }
        if self.next_record == u64::MAX {
            return Err(self.refuse(BufferError::IdentityExhausted));
        }
        let index = (self.head + self.count) % MAXIMUM_RECORDS;
        self.slots[index] = Some(RetainedRecord {
            ordinal: self.next_record,
            bytes: request.encode_to_vec().into_boxed_slice(),
        });
        self.next_record += 1;
        self.count += 1;
        self.bytes += length;
        Ok(())
    }
    /// Explicit monotonic clock input; an upload expires exactly at its deadline.
    pub fn begin_upload(&mut self, now: Duration) -> Result<UploadLease, BufferError> {
        if self.lease.is_some() {
            return Err(BufferError::Capacity);
        }
        if self.next_attempt == u64::MAX {
            return Err(BufferError::IdentityExhausted);
        }
        let first_record = self.slots[self.head]
            .as_ref()
            .ok_or(BufferError::Empty)?
            .ordinal;
        let deadline = now
            .checked_add(UPLOAD_DEADLINE)
            .ok_or(BufferError::ClockOverflow)?;
        let mut bytes = 0;
        let mut records = 0;
        for offset in 0..self.count {
            let record = self.slots[(self.head + offset) % MAXIMUM_RECORDS]
                .as_ref()
                .ok_or(BufferError::Empty)?;
            if records == MAXIMUM_UPLOAD_RECORDS
                || bytes + record.bytes.len() > MAXIMUM_UPLOAD_BYTES
            {
                break;
            }
            bytes += record.bytes.len();
            records += 1;
        }
        let identity = LeaseIdentity {
            generation: self.generation,
            attempt: self.next_attempt,
            first_record,
            records,
            bytes,
            deadline,
        };
        self.next_attempt += 1;
        self.lease = Some(identity);
        Ok(UploadLease {
            owner: Rc::clone(&self.owner),
            identity,
        })
    }
    fn check_lease(&self, lease: &UploadLease) -> Result<(), BufferError> {
        if !Rc::ptr_eq(&self.owner, &lease.owner)
            || lease.identity.generation != self.generation
            || self.lease != Some(lease.identity)
            || self.count < lease.identity.records
            || (0..lease.identity.records)
                .any(|offset| self.slots[(self.head + offset) % MAXIMUM_RECORDS].is_none())
            || self.slots[self.head].as_ref().map(|record| record.ordinal)
                != Some(lease.identity.first_record)
        {
            return Err(BufferError::StaleLease);
        }
        Ok(())
    }
    /// Borrow unchanged single-record OTLP requests; no payload copy or generic upload envelope.
    pub fn upload_records<'a>(
        &'a self,
        lease: &UploadLease,
    ) -> Result<impl Iterator<Item = &'a [u8]>, BufferError> {
        self.check_lease(lease)?;
        Ok((0..lease.identity.records).filter_map(|offset| {
            self.slots[(self.head + offset) % MAXIMUM_RECORDS]
                .as_ref()
                .map(|record| record.bytes.as_ref())
        }))
    }
    /// Local adapter seam: call only after matching full-prefix terminal Spooled/Committed receipt.
    /// Pending/send/half-close/partial receipts must not call this. No RPC receipt is invented here.
    pub fn acknowledge_durable(
        &mut self,
        lease: &UploadLease,
        now: Duration,
    ) -> Result<(), BufferError> {
        self.check_lease(lease)?;
        if now >= lease.identity.deadline {
            return Err(BufferError::Deadline);
        }
        for _ in 0..lease.identity.records {
            if let Some(record) = self.slots[self.head].take() {
                self.bytes -= record.bytes.len();
                self.count -= 1;
                self.head = (self.head + 1) % MAXIMUM_RECORDS;
            }
        }
        self.lease = None;
        Ok(())
    }
    /// End only this attempt; leave all original bytes/identities for the lifecycle owner's policy.
    pub fn fail_upload(
        &mut self,
        lease: &UploadLease,
        reason: UploadFailure,
    ) -> Result<UploadFailure, BufferError> {
        self.check_lease(lease)?;
        self.lease = None;
        Ok(reason)
    }
    pub fn expire_upload(&mut self, now: Duration) -> Result<(), BufferError> {
        let lease = self.lease.ok_or(BufferError::Empty)?;
        if now < lease.deadline {
            return Err(BufferError::NotExpired);
        }
        self.lease = None;
        Ok(())
    }
    /// Close/rebind explicitly loses best-effort remainder; returned count exposes that gap.
    /// Counters never reset, including when a caller mistakenly reuses a generation.
    pub fn invalidate(&mut self, generation: u64) -> usize {
        let lost = self.count;
        self.slots = std::array::from_fn(|_| None);
        self.head = 0;
        self.count = 0;
        self.bytes = 0;
        self.lease = None;
        self.generation = generation;
        lost
    }
}
fn attribute(name: &'static str, value: Value) -> KeyValue {
    KeyValue {
        key: name.to_owned(),
        value: Some(AnyValue { value: Some(value) }),
    }
}
fn identity_hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn encode_log(
    build: &BuildIdentity,
    key: CaptureKey,
    input: BrowserLog,
) -> ExportLogsServiceRequest {
    let mut attributes = vec![
        attribute("df.producer_id", Value::StringValue(key.producer.hex())),
        attribute("df.record_id", Value::StringValue(key.record.hex())),
        attribute("df.source_sequence", Value::IntValue(key.sequence.get())),
        attribute(
            "fixture.build",
            Value::StringValue(build.revision(BuildRevision::Source).as_str().to_owned()),
        ),
        attribute("rpc.status", Value::StringValue(input.status.to_owned())),
        attribute(
            "rpc.bytes.measured",
            Value::BoolValue(input.measured_bytes.is_some()),
        ),
    ];
    if let Some(bytes) = input.measured_bytes {
        attributes.push(attribute("rpc.bytes", Value::IntValue(bytes)));
    }
    if let Some(session) = input.session {
        attributes.push(attribute(
            "df.session",
            Value::StringValue(identity_hex(session.as_bytes())),
        ));
    }
    if let Some(operation) = input.operation {
        attributes.push(attribute(
            "df.operation",
            Value::StringValue(identity_hex(operation.as_bytes())),
        ));
    }
    // As in D01, malformed/invalid correlation becomes unparented, never authority.
    let trace = input.trace.filter(SpanContext::is_valid);
    let record = LogRecord {
        time_unix_nano: input.timestamp_unix_nanos,
        observed_time_unix_nano: input.observed_unix_nanos,
        severity_number: input.severity as i32,
        severity_text: input.severity.as_str_name().to_owned(),
        body: None,
        attributes,
        trace_id: trace
            .as_ref()
            .map(|trace| trace.trace_id().to_bytes().to_vec())
            .unwrap_or_default(),
        span_id: trace
            .as_ref()
            .map(|trace| trace.span_id().to_bytes().to_vec())
            .unwrap_or_default(),
        flags: trace
            .as_ref()
            .map(|trace| u32::from(trace.trace_flags().to_u8()))
            .unwrap_or_default(),
        event_name: input.event_name.to_owned(),
        ..Default::default()
    };
    let resource = Resource {
        attributes: std::iter::once(attribute(
            "service.name",
            Value::StringValue("df-observe.browser".to_owned()),
        ))
        .chain(
            [
                ("df.build", BuildRevision::Source),
                ("df.native", BuildRevision::Native),
                ("df.wasm", BuildRevision::Wasm),
                ("df.configuration", BuildRevision::Configuration),
                ("df.content", BuildRevision::Content),
            ]
            .into_iter()
            .map(|(name, revision)| {
                attribute(
                    name,
                    Value::StringValue(build.revision(revision).as_str().to_owned()),
                )
            }),
        )
        .collect(),
        ..Default::default()
    };
    ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            resource: Some(resource),
            scope_logs: vec![ScopeLogs {
                scope: Some(InstrumentationScope {
                    name: "df-observe.browser".to_owned(),
                    version: "0.1.0".to_owned(),
                    ..Default::default()
                }),
                log_records: vec![record],
                schema_url: "https://dungeonflux.local/telemetry/scope-v1".to_owned(),
            }],
            schema_url: "https://dungeonflux.local/telemetry/native-v1".to_owned(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded_record(bytes: usize) -> ExportLogsServiceRequest {
        let build = BuildIdentity::new(
            Some("source"),
            Some("native"),
            Some("wasm"),
            Some("config"),
            Some("content"),
        )
        .unwrap();
        let key = CaptureKey {
            producer: crate::ProducerId::new([1; 16]).unwrap(),
            record: crate::RecordId::new([2; 16]).unwrap(),
            sequence: crate::SourceSequence::new(1).unwrap(),
        };
        // The internal encoded admission seam tests payload bounds beyond the public static cap.
        // Bytes still use the actual standard OTLP encoder, build and capture identity fields.
        encode_log(
            &build,
            key,
            BrowserLog {
                timestamp_unix_nanos: 1,
                observed_unix_nanos: 2,
                severity: SeverityNumber::Info,
                event_name: Box::leak("x".repeat(bytes).into_boxed_str()),
                status: "complete",
                measured_bytes: None,
                session: None,
                operation: None,
                trace: None,
            },
        )
    }
    #[test]
    fn encoded_payload_and_prefix_byte_bounds_refuse_without_eviction() {
        // Exercise the private encoding boundary independently of the narrower safe log catalog.
        // This does not claim that public static-name inputs can reach these larger lengths.
        let request = encoded_record(6000);
        let length = request.encoded_len();
        assert!(length <= MAXIMUM_RECORD_BYTES);
        let mut buffer = BrowserBuffer::new(1);
        while buffer.bytes + length <= MAXIMUM_BYTES {
            buffer.retain_encoded(&request).unwrap();
        }
        let before = buffer.snapshot();
        assert!(before.retained_records < MAXIMUM_RECORDS);
        assert_eq!(buffer.retain_encoded(&request), Err(BufferError::Capacity));
        assert_eq!(buffer.snapshot().retained_bytes, before.retained_bytes);
        assert_eq!(buffer.snapshot().retained_records, before.retained_records);
        assert_eq!(buffer.snapshot().refused_records, 1);
        let lease = buffer.begin_upload(Duration::ZERO).unwrap();
        assert_eq!(lease.records(), MAXIMUM_UPLOAD_BYTES / length);
        assert_eq!(lease.bytes(), lease.records() * length);
        assert!(lease.bytes() <= MAXIMUM_UPLOAD_BYTES);
        assert_eq!(
            buffer.retain_encoded(&encoded_record(8192)),
            Err(BufferError::Oversized)
        );
    }
    #[test]
    fn identity_exhaustion_refuses_and_survives_rebind() {
        let request = encoded_record(1);
        let mut buffer = BrowserBuffer::new(1);
        buffer.next_record = u64::MAX;
        assert_eq!(
            buffer.retain_encoded(&request),
            Err(BufferError::IdentityExhausted)
        );
        assert_eq!(buffer.snapshot().retained_records, 0);
        buffer.next_record = 1;
        buffer.retain_encoded(&request).unwrap();
        buffer.next_attempt = u64::MAX;
        assert!(matches!(
            buffer.begin_upload(Duration::ZERO),
            Err(BufferError::IdentityExhausted)
        ));
        buffer.invalidate(1);
        buffer.retain_encoded(&request).unwrap();
        assert!(matches!(
            buffer.begin_upload(Duration::ZERO),
            Err(BufferError::IdentityExhausted)
        ));
        buffer.refused = usize::MAX;
        assert_eq!(
            buffer.retain_encoded(&encoded_record(9000)),
            Err(BufferError::Oversized)
        );
        assert_eq!(buffer.snapshot().refused_records, usize::MAX);
    }
}
