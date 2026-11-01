//! Correlates otherwise unobserved fatal stage errors without inspecting private error payloads.
use super::{ImageRuntimeTelemetry, RuntimeError};
use crate::execution::ExecutionError;
use crate::telemetry::{EventContext, EventKind, FailureKind, ImageEvent};

pub(super) fn correlate_failure(
    telemetry: &dyn ImageRuntimeTelemetry,
    error: &RuntimeError,
    context: EventContext,
) {
    let failure = match error {
        // These paths already observed their durable stage outcome or pending projection.
        RuntimeError::ProviderRefused
        | RuntimeError::ProviderUnknown
        | RuntimeError::SupplierQuotaExhausted
        | RuntimeError::SupplierRateLimited
        | RuntimeError::SupplierFailure(_)
        | RuntimeError::CostProjectionPending => return,
        RuntimeError::Journal(_) => FailureKind::Journal,
        RuntimeError::Execution(error) => match error {
            ExecutionError::CapacityFull | ExecutionError::Deferred { .. } => FailureKind::Capacity,
            ExecutionError::Disabled | ExecutionError::ConfigFenced { .. } => FailureKind::Disabled,
            ExecutionError::Expired => FailureKind::Expired,
            ExecutionError::Retired | ExecutionError::DemandLapsed => FailureKind::Cancelled,
            ExecutionError::Superseded
            | ExecutionError::LostClaim
            | ExecutionError::Stale { .. }
            | ExecutionError::OutputSealed => FailureKind::Conflict,
            _ => FailureKind::Journal,
        },
        RuntimeError::Publication(_) => FailureKind::Publication,
        RuntimeError::Cost(_) => FailureKind::UsageProjection,
        RuntimeError::CredentialUnavailable
        | RuntimeError::InvalidConfiguration
        | RuntimeError::InvalidRecord
        | RuntimeError::UnsupportedCapability => FailureKind::Validation,
        RuntimeError::SourceDenied => FailureKind::Authorization,
        RuntimeError::Disabled => FailureKind::Disabled,
        RuntimeError::Expired => FailureKind::Expired,
        RuntimeError::Cancelled => FailureKind::Cancelled,
        RuntimeError::QuotaBlocked { .. } => FailureKind::Capacity,
        RuntimeError::StorageUnavailable => FailureKind::Storage,
        RuntimeError::WorkerFailed | RuntimeError::TelemetryUnavailable => FailureKind::Worker,
    };
    telemetry.observe(ImageEvent::new(EventKind::Failed, context).failed(failure));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::{DispatchId, ExecutionStageId, FailureCause};
    use crate::telemetry::SharedImageObserver;
    use df_observe::{AnyValue, DiagnosticRecord, DiagnosticSink, TelemetryError};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Collected(Mutex<Vec<DiagnosticRecord>>);
    impl DiagnosticSink for Collected {
        fn emit(&self, record: DiagnosticRecord) -> Result<(), TelemetryError> {
            self.0.lock().unwrap().push(record);
            Ok(())
        }
    }

    #[test]
    fn fatal_storage_error_keeps_the_actual_stage_and_dispatch_without_an_error_body() {
        let sink = Arc::new(Collected::default());
        let observer = SharedImageObserver::new(sink.clone());
        let context = EventContext {
            job: Some(crate::ImageJobId::from_bytes(&[1; 16]).unwrap()),
            stage: Some(ExecutionStageId::from_bytes(&[2; 16]).unwrap()),
            attempt: Some(DispatchId::from_bytes(&[3; 16]).unwrap()),
            ..Default::default()
        };
        correlate_failure(&observer, &RuntimeError::StorageUnavailable, context);
        let records = sink.0.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].event_name, "image.failed");
        for (key, value) in [
            ("image.job_id", "01".repeat(16)),
            ("image.stage_id", "02".repeat(16)),
            ("image.attempt_id", "03".repeat(16)),
            ("image.failure", "storage".to_owned()),
        ] {
            assert!(
                records[0]
                    .input
                    .attributes
                    .contains(&(key.to_owned(), AnyValue::String(value.into())))
            );
        }
        assert!(
            records[0]
                .input
                .attributes
                .iter()
                .all(|(key, _)| key != "image.error_body")
        );
    }

    #[test]
    fn previously_observed_supplier_and_pending_outcomes_do_not_emit_another_failure() {
        let sink = Arc::new(Collected::default());
        let observer = SharedImageObserver::new(sink.clone());
        for error in [
            RuntimeError::ProviderRefused,
            RuntimeError::ProviderUnknown,
            RuntimeError::SupplierQuotaExhausted,
            RuntimeError::SupplierRateLimited,
            RuntimeError::SupplierFailure(FailureCause::TransportLost),
            RuntimeError::CostProjectionPending,
        ] {
            correlate_failure(&observer, &error, EventContext::default());
        }
        assert!(sink.0.lock().unwrap().is_empty());
        assert_eq!(observer.health().failures, 0);
    }
}
