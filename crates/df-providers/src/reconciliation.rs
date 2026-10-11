//! Reconciliation facts for the two selected supplier routes.
//!
//! Supplier request IDs are lookup handles, not application `OperationId`s.
//! Neither selected route documents duplicate-safe submission: fal documents
//! status/result lookup only after a request ID is captured, while the selected
//! ElevenLabs stream exposes response metadata without a status lookup. A
//! timeout, missing response, absent status, 404, cancellation, or incomplete
//! stream therefore cannot establish that submission was absent or safe to
//! repeat. This module classifies facts only; it never sends, retries, refunds,
//! or verifies billing. Source pages observed 2026-10-10:
//! - <https://elevenlabs.io/docs/api-reference/text-to-speech/stream>
//! - <https://elevenlabs.io/docs/api-reference/introduction/>
//! - <https://fal.ai/docs/documentation/model-apis/inference/queue>
//! - <https://fal.ai/models/fal-ai/flux/schnell/api>
//!   These pages document request/status/result fields; they do not establish
//!   duplicate-safe submission or negative absence/billing proof.

use std::fmt;

use df_provider_api::{
    BudgetMutation, CheckedRequest, ProviderAttemptObservation, ProviderBillingClass,
    ProviderFailureClass, ProviderOutcomeDecision, ProviderOutcomeError, ProviderResultClass,
    ProviderRetryPolicy, RequestOwnerState, classify_provider_outcome,
};
use df_types::OperationId;

use crate::{
    CandidateRouteId, ElevenLabsAudioObservation, FalQueueObservation, ProviderImageMetadata,
    ProviderRequestId, validate_fal_images,
};

/// Application identity and captured supplier identity for one stored attempt.
/// These are separate fields: a supplier request ID is never an OperationId.
#[derive(Clone, Eq, PartialEq)]
pub struct ProviderAttemptIdentity {
    operation: OperationId,
    route: CandidateRouteId,
    supplier_request_id: Option<ProviderRequestId>,
}

impl ProviderAttemptIdentity {
    pub fn new(
        operation: OperationId,
        route: CandidateRouteId,
        supplier_request_id: Option<ProviderRequestId>,
    ) -> Self {
        Self {
            operation,
            route,
            supplier_request_id,
        }
    }

    pub const fn operation(&self) -> OperationId {
        self.operation
    }

    pub const fn route(&self) -> CandidateRouteId {
        self.route
    }

    pub fn supplier_request_id(&self) -> Option<&ProviderRequestId> {
        self.supplier_request_id.as_ref()
    }
}

impl fmt::Debug for ProviderAttemptIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderAttemptIdentity")
            .field("operation", &self.operation)
            .field("route", &self.route)
            .field(
                "supplier_request_id",
                &self.supplier_request_id.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Documented status lookup capability, independent of submit idempotency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconciliationLookup {
    ResponseMetadataOnly,
    CapturedRequestId,
}

/// Current documented reconciliation capabilities for a selected route.
///
/// `submission_idempotency_documented` is false for both selected routes based
/// on the dated source decision. A positive lookup does not change that fact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconciliationCapability {
    lookup: ReconciliationLookup,
    submission_idempotency_documented: bool,
}

impl ReconciliationCapability {
    pub const fn lookup(self) -> ReconciliationLookup {
        self.lookup
    }

    pub const fn submission_idempotency_documented(self) -> bool {
        self.submission_idempotency_documented
    }
}

/// Read-only supplier evidence presented for the stored provider attempt.
pub enum ReconciliationInput<'a> {
    /// Complete schema-qualified native Responses observation for this exact
    /// operation and supplier response. Token counters are not billing proof.
    #[cfg(not(target_arch = "wasm32"))]
    OpenAiResponse(&'a crate::TextResponseObservation),
    /// Submit, status, or result response was lost, absent, partial, cancelled, or unusable.
    UnknownSupplierOutcome,
    /// A native attempt owner classified a terminal failure. HTTP status and
    /// timeout observations must use `Unknown` unless stronger facts are proven.
    ClassifiedFailure(ProviderFailureClass),
    /// Clean, validated ElevenLabs stream response. Its optional request ID is
    /// checked against the ID on the same attempt when the supplier returned it.
    ElevenLabsStream(&'a ElevenLabsAudioObservation),
    /// FAL status for the request ID stored on the exact attempt.
    FalStatus(&'a FalQueueObservation),
    /// FAL result facts after positive same-ID completion and image validation.
    FalResult {
        observation: &'a FalQueueObservation,
        images: &'a [ProviderImageMetadata],
    },
}

/// Describes lookup and submission idempotency separately for one route.
pub const fn reconciliation_capability(route: CandidateRouteId) -> ReconciliationCapability {
    let lookup = match route {
        CandidateRouteId::ElevenFlashV25Tts
        | CandidateRouteId::OpenAiResponsesText
        | CandidateRouteId::ElevenScribeV2Stt => ReconciliationLookup::ResponseMetadataOnly,
        CandidateRouteId::FalFluxSchnellDisposableImage => ReconciliationLookup::CapturedRequestId,
    };
    ReconciliationCapability {
        lookup,
        submission_idempotency_documented: false,
    }
}

/// Applies an actual native STT observation to its exact full original binding.
/// The canonical classifier owns outcome/liability policy; supplier meters never
/// establish verified billing. Source/rights/locale/profile substitutions refuse.
#[cfg(not(target_arch = "wasm32"))]
pub fn reconcile_stt_attempt<Receipt, Refusal, Failure>(
    request: &CheckedRequest<crate::ElevenSttBasis>,
    attempt: &ProviderAttemptIdentity,
    owner: RequestOwnerState<'_, crate::ElevenSttBasis>,
    observation: &crate::NativeSpeechObservation,
    billing: ProviderBillingClass,
    settlement: &BudgetMutation<Receipt, Refusal, Failure>,
    policy: ProviderRetryPolicy,
) -> Result<ProviderOutcomeDecision, ProviderOutcomeError> {
    let matched = observation.matches_stt(request);
    reconcile_native_speech(
        request,
        attempt,
        owner,
        (observation, matched),
        billing,
        settlement,
        policy,
    )
}

/// Encoded speech completion follows the same canonical policy and complete
/// immutable binding fence as transcription. It grants no decoder/playback rights.
#[cfg(not(target_arch = "wasm32"))]
pub fn reconcile_speech_attempt<Receipt, Refusal, Failure>(
    request: &CheckedRequest<crate::ElevenSpeechBasis>,
    attempt: &ProviderAttemptIdentity,
    owner: RequestOwnerState<'_, crate::ElevenSpeechBasis>,
    observation: &crate::NativeSpeechObservation,
    billing: ProviderBillingClass,
    settlement: &BudgetMutation<Receipt, Refusal, Failure>,
    policy: ProviderRetryPolicy,
) -> Result<ProviderOutcomeDecision, ProviderOutcomeError> {
    let matched = observation.matches_speech(request);
    reconcile_native_speech(
        request,
        attempt,
        owner,
        (observation, matched),
        billing,
        settlement,
        policy,
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn reconcile_native_speech<Semantic: Eq, Receipt, Refusal, Failure>(
    request: &CheckedRequest<Semantic>,
    attempt: &ProviderAttemptIdentity,
    owner: RequestOwnerState<'_, Semantic>,
    observed: (&crate::NativeSpeechObservation, bool),
    billing: ProviderBillingClass,
    settlement: &BudgetMutation<Receipt, Refusal, Failure>,
    policy: ProviderRetryPolicy,
) -> Result<ProviderOutcomeDecision, ProviderOutcomeError> {
    let (observation, matched) = observed;
    let complete = matched
        && attempt.route == observation.route
        && attempt.operation == observation.identity.operation
        && attempt.supplier_request_id.as_ref() == observation.metadata().request_id();
    classify_provider_outcome(
        request,
        owner,
        ProviderAttemptObservation {
            operation: attempt.operation,
            result: if complete {
                ProviderResultClass::Complete
            } else {
                ProviderResultClass::Incomplete
            },
            billing,
        },
        settlement,
        policy,
    )
}

/// Applies supplier result facts through the canonical provider outcome policy.
///
/// The stored attempt binds the caller's `OperationId`, selected route, and
/// captured supplier request ID. The canonical classifier receives that stored
/// operation, so a different operation is refused by the shared policy. FAL
/// completion requires validated same-ID result images; status alone is
/// insufficient. `billing` and `settlement` must come from the caller's verified
/// same-attempt authority; supplier status, inference time, response headers,
/// and invoice absence cannot establish them. This function performs no I/O or
/// mutation.
pub fn reconcile_provider_attempt<Semantic: Eq, Receipt, Refusal, Failure>(
    request: &CheckedRequest<Semantic>,
    attempt_identity: &ProviderAttemptIdentity,
    owner: RequestOwnerState<'_, Semantic>,
    supplier: ReconciliationInput<'_>,
    billing: ProviderBillingClass,
    settlement: &BudgetMutation<Receipt, Refusal, Failure>,
    policy: ProviderRetryPolicy,
) -> Result<ProviderOutcomeDecision, ProviderOutcomeError> {
    let result = match supplier {
        #[cfg(not(target_arch = "wasm32"))]
        ReconciliationInput::OpenAiResponse(response)
            if attempt_identity.route == CandidateRouteId::OpenAiResponsesText
                && attempt_identity.operation == response.identity.operation
                && response.identity == request.binding().identity
                && attempt_identity.supplier_request_id.as_ref()
                    == Some(&response.provider_request_id) =>
        {
            ProviderResultClass::Complete
        }
        #[cfg(not(target_arch = "wasm32"))]
        ReconciliationInput::OpenAiResponse(_) => ProviderResultClass::Incomplete,
        ReconciliationInput::UnknownSupplierOutcome => ProviderResultClass::Incomplete,
        ReconciliationInput::ClassifiedFailure(failure) => ProviderResultClass::Failed(failure),
        ReconciliationInput::ElevenLabsStream(audio)
            if attempt_identity.route == CandidateRouteId::ElevenFlashV25Tts
                && eleven_request_matches_attempt(attempt_identity, audio) =>
        {
            ProviderResultClass::Complete
        }
        ReconciliationInput::ElevenLabsStream(_) => ProviderResultClass::Incomplete,
        ReconciliationInput::FalStatus(observation)
            if attempt_identity.route == CandidateRouteId::FalFluxSchnellDisposableImage
                && fal_status_matches_attempt(attempt_identity, observation)
                && matches!(observation, FalQueueObservation::Failed { .. }) =>
        {
            ProviderResultClass::Failed(ProviderFailureClass::Unknown)
        }
        ReconciliationInput::FalStatus(_) => ProviderResultClass::Incomplete,
        ReconciliationInput::FalResult {
            observation,
            images,
        } if attempt_identity.route == CandidateRouteId::FalFluxSchnellDisposableImage
            && attempt_identity
                .supplier_request_id
                .as_ref()
                .is_some_and(|expected| {
                    validate_fal_images(observation, expected, images).is_ok()
                }) =>
        {
            ProviderResultClass::Complete
        }
        ReconciliationInput::FalResult { .. } => ProviderResultClass::Incomplete,
    };

    let attempt = ProviderAttemptObservation {
        operation: attempt_identity.operation,
        result,
        billing,
    };
    classify_provider_outcome(request, owner, attempt, settlement, policy)
}

fn fal_status_matches_attempt(
    attempt: &ProviderAttemptIdentity,
    observation: &FalQueueObservation,
) -> bool {
    let Some(expected) = attempt.supplier_request_id.as_ref() else {
        return false;
    };
    match observation {
        FalQueueObservation::Pending(observed)
        | FalQueueObservation::Completed {
            request_id: observed,
            ..
        }
        | FalQueueObservation::Failed {
            request_id: observed,
            ..
        }
        | FalQueueObservation::Unknown(observed) => expected == observed,
    }
}

fn eleven_request_matches_attempt(
    attempt: &ProviderAttemptIdentity,
    audio: &ElevenLabsAudioObservation,
) -> bool {
    match (audio.request_id(), attempt.supplier_request_id.as_ref()) {
        (Some(observed), Some(expected)) => observed == expected.as_str(),
        (None, None) => true,
        _ => false,
    }
}
