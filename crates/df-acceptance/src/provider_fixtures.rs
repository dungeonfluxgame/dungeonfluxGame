use std::time::Duration;

use df_model::checkpoint::{Basis, ExecutionMode, JobId};
use df_observe::OperationContext;
use df_provider_api::{
    CheckedRequest, RequestBinding, RequestIdentity, RequestLimits, RequestOwnerState, RequestUsage,
};
use df_types::{
    OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision, Usage, UsageUnit,
};

const REQUEST_DEADLINE: Duration = Duration::from_secs(2);

/// The operation context every acceptance scenario passes to an instrumented call.
pub fn operation_context() -> OperationContext {
    OperationContext {
        trace_parent: String::new(),
        build: "acceptance".to_owned(),
    }
}

/// The binding of one admitted provider request for `operation` in `mode`.
pub fn request_binding(
    mode: ExecutionMode,
    operation: OperationId,
) -> RequestBinding<RevisionLabel> {
    RequestBinding {
        identity: RequestIdentity {
            basis: Basis {
                session: SessionId::from_bytes(&[1; 16]).expect("nonzero"),
                run: RunId::from_bytes(&[2; 16]).expect("nonzero"),
                revision: SessionRevision::new(RecoveryEpoch::new(3).expect("nonzero"), 4),
            },
            job: JobId::from_bytes(&[5; 16]).expect("nonzero"),
            operation,
            generation: 7,
        },
        semantic_basis: RevisionLabel::new(Some("source-rights-quote-current"))
            .expect("a valid label"),
        mode,
        deadline: REQUEST_DEADLINE,
    }
}

/// The owner's view of a request that is still current, not cancelled and not timed out.
pub fn request_owner_state(
    current: &RequestBinding<RevisionLabel>,
) -> RequestOwnerState<'_, RevisionLabel> {
    RequestOwnerState {
        current: Some(current),
        elapsed: Duration::ZERO,
        cancelled: false,
    }
}

/// A checked request for `current`, within its limits.
pub fn checked_request(current: &RequestBinding<RevisionLabel>) -> CheckedRequest<RevisionLabel> {
    let candidate = RequestBinding {
        identity: current.identity,
        semantic_basis: current.semantic_basis.clone(),
        mode: current.mode,
        deadline: current.deadline,
    };
    let limits = RequestLimits::new(
        4,
        3,
        Duration::from_nanos(5),
        Usage::new(2, UsageUnit::Token),
    )
    .expect("positive limits");
    CheckedRequest::new(
        candidate,
        b"data",
        RequestUsage::new(3, Duration::from_nanos(5), Usage::new(2, UsageUnit::Token)),
        limits,
        request_owner_state(current),
    )
    .expect("a request within its limits")
}
