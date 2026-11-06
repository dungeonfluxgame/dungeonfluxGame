//! Fixture-only bounded in-memory span export; never durable telemetry storage.
use crate::limits::MAXIMUM_FIXTURE_SPANS;
use opentelemetry::global;
use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::{
    error::{OTelSdkError, OTelSdkResult},
    trace::{SdkTracerProvider, SpanData, SpanExporter},
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
#[derive(Clone, Debug, Default)]
struct BoundedExporter {
    spans: Arc<Mutex<VecDeque<SpanData>>>,
    dropped: Arc<AtomicUsize>,
}
impl SpanExporter for BoundedExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let mut spans = self.spans.lock().map_err(|_| {
            OTelSdkError::InternalFailure("fixture export lock poisoned".to_owned())
        })?;
        for span in batch {
            if spans.len() == MAXIMUM_FIXTURE_SPANS {
                spans.pop_front();
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            spans.push_back(span);
        }
        Ok(())
    }
}
/// Fixture-only bounded in-memory export. Not durable telemetry storage.
pub struct FixtureTelemetry {
    provider: SdkTracerProvider,
    exporter: BoundedExporter,
}
impl Default for FixtureTelemetry {
    fn default() -> Self {
        let exporter = BoundedExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        Self { provider, exporter }
    }
}
impl FixtureTelemetry {
    /// Begin a span with this fixture's bounded, in-memory exporter.
    pub fn begin(
        &self,
        context: &crate::OperationContext,
        method: &'static str,
    ) -> crate::OperationSpan {
        crate::begin_with_tracer(&self.tracer(), context, method)
    }
    /// A tracer that exports only to this fixture's bounded exporter.
    pub(crate) fn tracer(&self) -> global::BoxedTracer {
        global::BoxedTracer::new(Box::new(
            self.provider.tracer(crate::OPERATION_TRACER_SCOPE),
        ))
    }
    /// Record a lifecycle event in this fixture's bounded, in-memory exporter.
    pub fn record(
        &self,
        context: &crate::OperationContext,
        method: &'static str,
        status: &str,
        bytes: usize,
    ) {
        self.begin(context, method).finish(status, bytes);
    }
    /// Return safe emitted span count; exporter failures remain visible.
    pub fn count(&self) -> Result<usize, String> {
        self.exporter
            .spans
            .lock()
            .map(|spans| spans.len())
            .map_err(|error| error.to_string())
    }
    /// Old diagnostic spans are discarded only at this declared fixture bound.
    pub fn dropped(&self) -> usize {
        self.exporter.dropped.load(Ordering::Relaxed)
    }
    /// Read-only safe OTEL records retain parent/trace IDs and typed attributes.
    pub fn snapshot(&self) -> Result<String, String> {
        let spans = self
            .exporter
            .spans
            .lock()
            .map_err(|error| error.to_string())?;
        Ok(spans
            .iter()
            .map(|span| {
                format!(
                    "{} trace={} span={} parent={} duration_us={:?} attributes={:?} events={:?}\n",
                    span.name,
                    span.span_context.trace_id(),
                    span.span_context.span_id(),
                    span.parent_span_id,
                    span.end_time
                        .duration_since(span.start_time)
                        .map(|duration| duration.as_micros()),
                    span.attributes,
                    span.events
                        .iter()
                        .map(|event| (&event.name, &event.attributes))
                        .collect::<Vec<_>>()
                )
            })
            .collect())
    }
    /// Flush the actual SDK before preview shutdown.
    pub fn shutdown(&self) -> Result<(), String> {
        self.provider.shutdown().map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod fixture_telemetry_tests;
