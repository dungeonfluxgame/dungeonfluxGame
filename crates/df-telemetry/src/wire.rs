use df_observe::{
    CaptureKey, ProducerId, RecordId, Signal, SourceSequence, TelemetryError, TelemetryLimits,
};
use opentelemetry_proto::tonic::{
    collector::{logs::v1::ExportLogsServiceRequest, trace::v1::ExportTraceServiceRequest},
    common::v1::{KeyValue, any_value::Value},
    logs::v1::{ResourceLogs, ScopeLogs},
    trace::v1::{ResourceSpans, ScopeSpans},
};
use prost::Message;

#[derive(Clone, Debug)]
pub(crate) struct Record {
    pub key: CaptureKey,
    pub signal: Signal,
    pub timestamp: u64,
    pub observed: Option<u64>,
    pub severity: i32,
    pub trace: Vec<u8>,
    pub span: Vec<u8>,
    pub session: Option<String>,
    pub operation: Option<String>,
    pub build: Option<String>,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug)]
pub(crate) struct Batch {
    pub signal: Signal,
    pub bytes: Vec<u8>,
    pub records: Vec<Record>,
}
impl Batch {
    pub fn queue_cost(&self) -> Result<usize, TelemetryError> {
        // Charge retained capacities, including projections copied out of OTLP. The
        // existing structural allowance covers each record's fixed-size fields.
        queue_cost(
            std::iter::once(self.bytes.capacity()).chain(self.records.iter().flat_map(|record| {
                [
                    256,
                    record.bytes.capacity(),
                    record.trace.capacity(),
                    record.span.capacity(),
                    record.session.as_ref().map_or(0, String::capacity),
                    record.operation.as_ref().map_or(0, String::capacity),
                    record.build.as_ref().map_or(0, String::capacity),
                ]
            })),
        )
    }
    pub fn decode(
        signal: Signal,
        bytes: &[u8],
        limits: TelemetryLimits,
    ) -> Result<Self, TelemetryError> {
        if bytes.len() > limits.batch_bytes {
            return Err(TelemetryError::Oversized);
        }
        let mut budget = Budget {
            fields: 0,
            records: 0,
            limits,
        };
        scan(
            bytes,
            if signal == Signal::Logs {
                Shape::LogsExport
            } else {
                Shape::SpansExport
            },
            0,
            &mut budget,
        )?;
        let mut records = Vec::with_capacity(budget.records);
        match signal {
            Signal::Logs => {
                let request = ExportLogsServiceRequest::decode(bytes)
                    .map_err(|_| TelemetryError::Malformed)?;
                for resource in request.resource_logs {
                    let build = resource
                        .resource
                        .as_ref()
                        .and_then(|resource| text(&resource.attributes, "df.build"));
                    for scope in resource.scope_logs {
                        for log in scope.log_records {
                            let key = key(&log.attributes)?;
                            let single = ExportLogsServiceRequest {
                                resource_logs: vec![ResourceLogs {
                                    resource: resource.resource.clone(),
                                    schema_url: resource.schema_url.clone(),
                                    scope_logs: vec![ScopeLogs {
                                        scope: scope.scope.clone(),
                                        schema_url: scope.schema_url.clone(),
                                        log_records: vec![log.clone()],
                                    }],
                                }],
                            };
                            records.push(Record {
                                key,
                                signal,
                                timestamp: log.time_unix_nano,
                                observed: Some(log.observed_time_unix_nano),
                                severity: log.severity_number,
                                trace: log.trace_id,
                                span: log.span_id,
                                session: text(&log.attributes, "df.session"),
                                operation: text(&log.attributes, "df.operation"),
                                build: build.clone(),
                                bytes: single.encode_to_vec(),
                            });
                        }
                    }
                }
            }
            Signal::Spans => {
                let request = ExportTraceServiceRequest::decode(bytes)
                    .map_err(|_| TelemetryError::Malformed)?;
                for resource in request.resource_spans {
                    let build = resource
                        .resource
                        .as_ref()
                        .and_then(|resource| text(&resource.attributes, "df.build"));
                    for scope in resource.scope_spans {
                        for span in scope.spans {
                            let key = key(&span.attributes)?;
                            let single = ExportTraceServiceRequest {
                                resource_spans: vec![ResourceSpans {
                                    resource: resource.resource.clone(),
                                    schema_url: resource.schema_url.clone(),
                                    scope_spans: vec![ScopeSpans {
                                        scope: scope.scope.clone(),
                                        schema_url: scope.schema_url.clone(),
                                        spans: vec![span.clone()],
                                    }],
                                }],
                            };
                            records.push(Record {
                                key,
                                signal,
                                timestamp: span.start_time_unix_nano,
                                observed: None,
                                severity: 0,
                                trace: span.trace_id,
                                span: span.span_id,
                                session: text(&span.attributes, "df.session"),
                                operation: text(&span.attributes, "df.operation"),
                                build: build.clone(),
                                bytes: single.encode_to_vec(),
                            });
                        }
                    }
                }
            }
        }
        if records.is_empty() || records.len() > limits.batch_records {
            return Err(TelemetryError::Malformed);
        }
        for record in &records {
            if record.bytes.len() > limits.record_bytes {
                return Err(TelemetryError::Oversized);
            }
            if !(record.trace.is_empty() || record.trace.len() == 16)
                || !(record.span.is_empty() || record.span.len() == 8)
            {
                return Err(TelemetryError::Malformed);
            }
        }
        Ok(Self {
            signal,
            bytes: bytes.to_vec(),
            records,
        })
    }
}
fn queue_cost(mut capacities: impl Iterator<Item = usize>) -> Result<usize, TelemetryError> {
    capacities.try_fold(0usize, |cost, capacity| {
        cost.checked_add(capacity).ok_or(TelemetryError::Capacity)
    })
}
fn text(attributes: &[KeyValue], name: &str) -> Option<String> {
    attributes
        .iter()
        .find(|attribute| attribute.key == name)
        .and_then(
            |attribute| match attribute.value.as_ref()?.value.as_ref()? {
                Value::StringValue(value) => Some(value.clone()),
                _ => None,
            },
        )
}
fn key(attributes: &[KeyValue]) -> Result<CaptureKey, TelemetryError> {
    for name in ["df.producer_id", "df.record_id", "df.source_sequence"] {
        if attributes
            .iter()
            .filter(|attribute| attribute.key == name)
            .count()
            != 1
        {
            return Err(TelemetryError::InvalidIdentity);
        }
    }
    let producer = ProducerId::from_hex(
        &text(attributes, "df.producer_id").ok_or(TelemetryError::InvalidIdentity)?,
    )?;
    let record = RecordId::from_hex(
        &text(attributes, "df.record_id").ok_or(TelemetryError::InvalidIdentity)?,
    )?;
    let sequence = attributes
        .iter()
        .find(|attribute| attribute.key == "df.source_sequence")
        .and_then(
            |attribute| match attribute.value.as_ref()?.value.as_ref()? {
                Value::IntValue(value) => Some(*value),
                _ => None,
            },
        )
        .ok_or(TelemetryError::InvalidIdentity)?;
    Ok(CaptureKey {
        producer,
        record,
        sequence: SourceSequence::new(sequence)?,
    })
}
// Schema-aware, allocation-free protobuf preflight. Unknown fields remain bounded raw
// bytes; only known message fields recurse. Repeated empty messages and deep AnyValue
// trees are refused before prost can allocate their expanded representation.
#[derive(Clone, Copy)]
enum Shape {
    LogsExport,
    SpansExport,
    ResourceLogs,
    ResourceSpans,
    ScopeLogs,
    ScopeSpans,
    Log,
    Span,
    Resource,
    Scope,
    Attribute,
    Value,
    Array,
    Map,
    Event,
    Link,
    Status,
}
struct Budget {
    fields: usize,
    records: usize,
    limits: TelemetryLimits,
}
fn child(shape: Shape, field: u64) -> Option<Shape> {
    use Shape::*;
    match (shape, field) {
        (LogsExport, 1) => Some(ResourceLogs),
        (SpansExport, 1) => Some(ResourceSpans),
        (ResourceLogs | ResourceSpans, 1) => Some(Resource),
        (ResourceLogs, 2) => Some(ScopeLogs),
        (ResourceSpans, 2) => Some(ScopeSpans),
        (ScopeLogs | ScopeSpans, 1) => Some(Scope),
        (ScopeLogs, 2) => Some(Log),
        (ScopeSpans, 2) => Some(Span),
        (Resource, 1) | (Scope, 4) | (Log, 6) | (Span, 9) | (Event, 3) | (Link, 4) | (Map, 1) => {
            Some(Attribute)
        }
        (Log, 5) | (Attribute, 2) | (Array, 1) => Some(Value),
        (Value, 5) => Some(Array),
        (Value, 6) => Some(Map),
        (Span, 11) => Some(Event),
        (Span, 13) => Some(Link),
        (Span, 15) => Some(Status),
        _ => None,
    }
}
fn varint(bytes: &[u8], offset: &mut usize) -> Result<u64, TelemetryError> {
    let mut decoded_value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *bytes.get(*offset).ok_or(TelemetryError::Malformed)?;
        *offset += 1;
        if shift == 63 && byte > 1 {
            return Err(TelemetryError::Malformed);
        }
        decoded_value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(decoded_value);
        }
    }
    Err(TelemetryError::Malformed)
}
fn scan(
    bytes: &[u8],
    shape: Shape,
    depth: usize,
    budget: &mut Budget,
) -> Result<(), TelemetryError> {
    if depth > 12 {
        return Err(TelemetryError::Oversized);
    }
    if matches!(shape, Shape::Log | Shape::Span) {
        budget.records += 1;
        if budget.records > budget.limits.batch_records || bytes.len() > budget.limits.record_bytes
        {
            return Err(TelemetryError::Oversized);
        }
    }
    let mut offset = 0;
    let mut local_fields = 0;
    while offset < bytes.len() {
        local_fields += 1;
        if local_fields > 128 {
            return Err(TelemetryError::Oversized);
        }
        budget.fields += 1;
        if budget.fields > 4096 {
            return Err(TelemetryError::Oversized);
        }
        let tag = varint(bytes, &mut offset)?;
        let field = tag >> 3;
        if field == 0 {
            return Err(TelemetryError::Malformed);
        }
        let field_size_bytes = match tag & 7 {
            0 => {
                varint(bytes, &mut offset)?;
                0
            }
            1 => 8,
            5 => 4,
            2 => {
                let field_size_bytes = usize::try_from(varint(bytes, &mut offset)?)
                    .map_err(|_| TelemetryError::Oversized)?;
                let end = offset
                    .checked_add(field_size_bytes)
                    .ok_or(TelemetryError::Oversized)?;
                let nested = bytes.get(offset..end).ok_or(TelemetryError::Malformed)?;
                if let Some(child) = child(shape, field) {
                    scan(nested, child, depth + 1, budget)?;
                }
                field_size_bytes
            }
            _ => return Err(TelemetryError::Malformed),
        };
        offset = offset
            .checked_add(field_size_bytes)
            .ok_or(TelemetryError::Oversized)?;
        if offset > bytes.len() {
            return Err(TelemetryError::Malformed);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_queue_cost_refuses_arithmetic_overflow() {
        assert_eq!(queue_cost([usize::MAX - 1, 1].into_iter()), Ok(usize::MAX));
        assert_eq!(
            queue_cost([usize::MAX, 1].into_iter()),
            Err(TelemetryError::Capacity)
        );
    }
}
