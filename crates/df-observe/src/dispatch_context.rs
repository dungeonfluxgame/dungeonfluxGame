//! Spans for owned asynchronous work, causally linked to the request that dispatched it.
use crate::{OperationContext, OperationSpan, parent};
use opentelemetry::{
    global,
    trace::{Link, Span, TraceContextExt, Tracer},
};

/// Begin owned asynchronous work with a causal link to its dispatch snapshot.
/// Capture with `OperationSpan::capture_context` before transferring work to its owner.
/// Request completion or cancellation does not invalidate the snapshot or cancel the child.
pub fn begin_dispatched(context: &OperationContext, method: &'static str) -> OperationSpan {
    begin_with_tracer(
        &global::tracer(crate::OPERATION_TRACER_SCOPE),
        context,
        method,
    )
}

/// Begin dispatched work with an owned diagnostic snapshot, including when no SDK is installed.
/// The snapshot is provenance only and must be captured by the dispatch owner.
pub fn begin_dispatched_with_audit(
    context: &OperationContext,
    audit: &crate::AuditContext,
    method: &'static str,
) -> OperationSpan {
    let mut span = begin_dispatched(context, method);
    span.audit = audit.clone();
    span
}

fn begin_with_tracer(
    tracer: &global::BoxedTracer,
    context: &OperationContext,
    method: &'static str,
) -> OperationSpan {
    let parent = parent(context);
    let mut builder = tracer.span_builder(method);
    let source = parent.span().span_context().clone();
    if source.is_valid() {
        builder = builder.with_links(vec![Link::new(source, Vec::new(), 0)]);
    }
    OperationSpan::new(tracer.build_with_context(builder, &parent), context, method)
}

impl OperationSpan {
    /// Own the safe audit correlation at dispatch, independently of SDK trace sampling.
    pub fn capture_audit_context(&self) -> crate::AuditContext {
        self.audit.clone()
    }

    /// Capture diagnostic correlation and admitted build before dispatch. The returned
    /// owned context contains this span's SDK identity, never raw incoming metadata.
    /// It carries no authorization, job lifetime, deadline, or cancellation ownership;
    /// those remain with the native dispatch owner independently of the request wait.
    pub fn capture_context(&self) -> OperationContext {
        let source = self.span.span_context();
        let trace_parent = if source.is_valid() {
            format!(
                "00-{}-{}-{:02x}",
                source.trace_id(),
                source.span_id(),
                source.trace_flags().to_u8()
            )
        } else {
            String::new()
        };
        OperationContext {
            trace_parent,
            build: self.build.clone(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl crate::FixtureTelemetry {
    /// Begin dispatched work using this fixture's independent bounded exporter.
    pub fn begin_dispatched(
        &self,
        context: &OperationContext,
        method: &'static str,
    ) -> OperationSpan {
        begin_with_tracer(&self.tracer(), context, method)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod dispatch_context_tests;
