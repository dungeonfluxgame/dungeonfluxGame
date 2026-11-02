//! The OpenAI Responses route as a df-ai text backend.
//!
//! A composition adapter over df-providers' `NativeTextProvider`: it sends one request and maps
//! what comes back. It never retries, caches, books cost or reads configuration; df-ai owns those,
//! and its authority port, not the local identity labels used here, is the owner fence.

use std::time::Duration;

use df_ai::text::{
    TextBackend, TextOutcome, TextReceipt, TextRequest, TextUsage, UnavailableReason,
};
use df_model::checkpoint::{Basis, ContentDigest, ExecutionMode, JobId};
use df_provider_api::{
    ModalityRequestError, RequestBasis, RequestBinding, RequestError, RequestIdentity,
    RequestLimits, RequestModality, RequestOwnerState, RequestUsage, TextEventKind,
    TextRequest as ProviderRequest, TextSemantics,
};
use df_providers::{
    CandidateRouteId, NativeHttpRefusal, NativeTextProvider, ProviderFailureClass, ProviderLimits,
    ReasoningEffort, TextOutputSchema, TextProviderConfig, TextResponseError, TextSchemaLimits,
    TextTokenUsage, registered_text_provider,
};
use df_types::{
    LocaleTag, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision, Usage,
    UsageUnit,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[cfg(test)]
use std::net::SocketAddr;

/// Bounds the parse of the supplier's whole response, which echoes the schema and carries usage
/// details, so they sit well above the schema's own size.
const SCHEMA_LIMITS: TextSchemaLimits = TextSchemaLimits {
    maximum_schema_bytes: 65_536,
    maximum_depth: 32,
    maximum_nodes: 4096,
};
const MAXIMUM_RESPONSE_BYTES: usize = 256 * 1024;
/// Room on the wire for the model name, metadata and framing around the prompt and schema.
const WIRE_OVERHEAD_BYTES: usize = 4096;
const OUTPUT_FORMAT_REVISION: &str = "responses-text-v1";
const RIGHTS_REVISION: &str = "text-backend-local";

/// Why a request could not be turned into a provider call before anything was sent.
enum PrepareError {
    Schema,
    Identity,
    Limits,
    Provider(TextResponseError),
}

pub(super) struct ResponsesTextBackend {
    model: String,
    credential: String,
    maximum_output_tokens: u32,
    reasoning: Option<ReasoningEffort>,
    #[cfg(test)]
    loopback: Option<SocketAddr>,
}

impl ResponsesTextBackend {
    /// Validates the model, output bound and credential once, so a bad configuration fails at
    /// composition instead of on the first request.
    pub(super) fn new(
        model: &str,
        credential: &str,
        maximum_output_tokens: u32,
    ) -> Result<Self, TextResponseError> {
        let placeholder = TextOutputSchema::new(
            "validation",
            br#"{"type":"object","properties":{},"required":[],"additionalProperties":false}"#,
            SCHEMA_LIMITS,
        )
        .map_err(|_| TextResponseError::InvalidConfiguration)?;
        let config = TextProviderConfig::new(model, placeholder, maximum_output_tokens)?;
        registered_text_provider(
            CandidateRouteId::OpenAiResponsesText,
            config,
            credential,
            ProviderLimits::new(
                Duration::from_secs(1),
                WIRE_OVERHEAD_BYTES,
                MAXIMUM_RESPONSE_BYTES,
            )
            .build(),
        )?;
        Ok(Self {
            model: model.to_owned(),
            credential: credential.to_owned(),
            maximum_output_tokens,
            reasoning: None,
            #[cfg(test)]
            loopback: None,
        })
    }

    #[must_use]
    pub(super) fn with_reasoning(mut self, reasoning: ReasoningEffort) -> Self {
        self.reasoning = Some(reasoning);
        self
    }

    #[cfg(test)]
    #[must_use]
    pub(super) fn with_loopback(mut self, address: SocketAddr) -> Self {
        self.loopback = Some(address);
        self
    }

    fn prepare(&self, request: &TextRequest) -> Result<Prepared, PrepareError> {
        let schema_bytes =
            serde_json::to_vec(&request.output_schema).map_err(|_| PrepareError::Schema)?;
        let schema = TextOutputSchema::new(&request.output_name, &schema_bytes, SCHEMA_LIMITS)
            .map_err(|_| PrepareError::Schema)?;
        let mut config =
            TextProviderConfig::new(&self.model, schema.clone(), self.maximum_output_tokens)
                .map_err(PrepareError::Provider)?;
        if let Some(effort) = self.reasoning {
            config = config.with_reasoning_effort(effort);
        }
        let prompt = prompt_text(&request.instructions, &request.facts);
        let body_limit = prompt
            .len()
            .saturating_add(schema_bytes.len().saturating_mul(2))
            .saturating_add(WIRE_OVERHEAD_BYTES);
        let limits =
            ProviderLimits::new(request.deadline, body_limit, MAXIMUM_RESPONSE_BYTES).build();
        #[allow(
            unused_mut,
            reason = "only the loopback test fixture rebinds the provider"
        )]
        let mut provider = registered_text_provider(
            CandidateRouteId::OpenAiResponsesText,
            config,
            &self.credential,
            limits,
        )
        .map_err(PrepareError::Provider)?;
        #[cfg(test)]
        if let Some(address) = self.loopback {
            provider = provider
                .for_loopback_fixture(address)
                .map_err(PrepareError::Provider)?;
        }
        let key = request_key(request, &schema_bytes);
        let binding = || {
            binding_for(
                request.purpose.label(),
                &key,
                schema.clone(),
                request.deadline,
            )
        };
        let current = binding().ok_or(PrepareError::Identity)?;
        // A byte is at least a token, so prompt and schema bytes plus the output bound cover the
        // supplier's total; the doubled ceiling only refuses a count that cannot be real.
        let declared = u128::try_from(prompt.len())
            .ok()
            .and_then(|bytes| bytes.checked_add(u128::try_from(schema_bytes.len()).ok()?))
            .and_then(|bytes| bytes.checked_add(u128::from(self.maximum_output_tokens)))
            .ok_or(PrepareError::Limits)?;
        let ceiling = declared.saturating_mul(2);
        let provider_request = ProviderRequest::new(
            binding().ok_or(PrepareError::Identity)?,
            prompt.as_bytes(),
            RequestUsage::new(
                declared,
                Duration::ZERO,
                Usage::new(declared, UsageUnit::Token),
            ),
            RequestLimits::new(
                prompt.len(),
                ceiling,
                request.deadline,
                Usage::new(ceiling, UsageUnit::Token),
            )
            .map_err(|_| PrepareError::Limits)?,
            owner_state(&current),
        )
        .map_err(|error| match error {
            ModalityRequestError::Request(error) => {
                PrepareError::Provider(TextResponseError::Owner(error))
            }
            ModalityRequestError::WrongModality => PrepareError::Identity,
        })?;
        Ok(Prepared {
            provider,
            request: provider_request,
            current,
        })
    }

    async fn receipt(&self, request: &TextRequest) -> TextReceipt {
        if request.deadline.is_zero() {
            return TextOutcome::Deadline.into();
        }
        let prepared = match self.prepare(request) {
            Ok(prepared) => prepared,
            Err(error) => return prepare_receipt(error),
        };
        let response = match prepared
            .provider
            .submit(
                &prepared.request,
                owner_state(&prepared.current),
                std::future::pending(),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => return error_receipt(&error),
        };
        let usage = usage_from(response.supplier_usage());
        let [candidate, _completed] = match response.complete(owner_state(&prepared.current)) {
            Ok(events) => events,
            Err(error) => return error_receipt(&error),
        };
        let outcome = match candidate.kind {
            TextEventKind::Candidate(output) => TextOutcome::Answered(output.json().clone()),
            _ => TextOutcome::Invalid,
        };
        TextReceipt { outcome, usage }
    }
}

impl TextBackend for ResponsesTextBackend {
    async fn complete(&self, request: &TextRequest) -> TextReceipt {
        self.receipt(request).await
    }
}

type Binding = RequestBinding<RequestBasis<TextSemantics<String, TextOutputSchema>>>;

struct Prepared {
    provider: NativeTextProvider,
    request: ProviderRequest<String, TextOutputSchema>,
    current: Binding,
}

fn owner_state(
    current: &Binding,
) -> RequestOwnerState<'_, RequestBasis<TextSemantics<String, TextOutputSchema>>> {
    RequestOwnerState {
        current: Some(current),
        elapsed: Duration::ZERO,
        cancelled: false,
    }
}

/// The instructions, a blank line, then the compact JSON of the facts.
fn prompt_text(instructions: &str, facts: &Value) -> String {
    format!("{instructions}\n\n{facts}")
}

/// Identifies the request by what it asks, so the same ask always carries the same labels.
fn request_key(request: &TextRequest, schema_bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in [
        request.output_name.as_bytes(),
        request.instructions.as_bytes(),
        request.facts.to_string().as_bytes(),
        schema_bytes,
    ] {
        // The length prefix keeps ("ab", "c") and ("a", "bc") from hashing alike.
        hasher.update(u64::try_from(part.len()).unwrap_or(u64::MAX).to_le_bytes());
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn binding_for(
    purpose: &str,
    key: &[u8; 32],
    schema: TextOutputSchema,
    deadline: Duration,
) -> Option<Binding> {
    // These labels name a side channel only. df-ai's authority port owns the real owner fence,
    // so the session and run are fixed local values, not game identities.
    let operation: [u8; 16] = key[..16].try_into().ok()?;
    let job: [u8; 16] = key[16..].try_into().ok()?;
    Some(RequestBinding {
        identity: RequestIdentity {
            basis: Basis {
                session: SessionId::from_bytes(&[0x53; 16]).ok()?,
                run: RunId::from_bytes(&[0x52; 16]).ok()?,
                revision: SessionRevision::new(RecoveryEpoch::new(1).ok()?, 1),
            },
            job: JobId::from_bytes(&job).ok()?,
            operation: OperationId::from_bytes(&operation).ok()?,
            generation: 1,
        },
        semantic_basis: RequestBasis {
            modality: RequestModality::Text,
            source: ContentDigest(*key),
            rights_revision: RevisionLabel::new(Some(RIGHTS_REVISION)).ok()?,
            output_format_revision: RevisionLabel::new(Some(OUTPUT_FORMAT_REVISION)).ok()?,
            semantic: TextSemantics {
                locale: LocaleTag::parse("en-US").ok()?,
                context: purpose.to_owned(),
                output_schema: schema,
            },
        },
        mode: ExecutionMode::Live,
        deadline,
    })
}

/// Converts supplier counters without clamping. A total that does not fit makes the whole usage
/// unknown, because a saturated total would be booked as a real number. An optional category
/// that does not fit becomes unknown on its own.
fn usage_from(usage: TextTokenUsage) -> Option<TextUsage> {
    let total_tokens = u64::try_from(usage.total_tokens).ok()?;
    Some(TextUsage {
        total_tokens,
        input_tokens: u64::try_from(usage.input_tokens).ok(),
        output_tokens: u64::try_from(usage.output_tokens).ok(),
        cached_input_tokens: u64::try_from(usage.cached_input_tokens).ok(),
        cache_write_tokens: usage
            .cache_write_tokens
            .and_then(|tokens| u64::try_from(tokens).ok()),
        reasoning_output_tokens: u64::try_from(usage.reasoning_output_tokens).ok(),
    })
}

fn error_receipt(error: &TextResponseError) -> TextReceipt {
    TextReceipt {
        outcome: failure_outcome(error),
        usage: error.supplier_usage().and_then(usage_from),
    }
}

fn prepare_receipt(error: PrepareError) -> TextReceipt {
    match error {
        PrepareError::Schema | PrepareError::Identity | PrepareError::Limits => {
            TextOutcome::Unavailable(UnavailableReason::Configuration).into()
        }
        PrepareError::Provider(error) => error_receipt(&error),
    }
}

/// Whether the request provably never left, so nothing can have reached the supplier. This is
/// the storyteller's rule, narrowed by one fact: an owner error that carries supplier usage came
/// from consuming a response, so the call was sent.
fn request_never_sent(error: &TextResponseError) -> bool {
    error.supplier_usage().is_none()
        && matches!(
            error.reason(),
            TextResponseError::UnregisteredRoute
                | TextResponseError::InvalidConfiguration
                | TextResponseError::InvalidCredential
                | TextResponseError::InvalidInput
                | TextResponseError::ModeDoesNotDispatch
                | TextResponseError::SchemaConfigurationMismatch
                | TextResponseError::Owner(_)
                | TextResponseError::Transport(NativeHttpRefusal::ProviderDisabled)
        )
}

/// Maps a provider failure to a df-ai outcome. Usage travels separately and is kept whenever the
/// supplier reported it.
///
/// | Failure                                                                   | Outcome                     |
/// | ------------------------------------------------------------------------- | --------------------------- |
/// | unregistered route, configuration, mode, schema mismatch, invalid input   | Unavailable(Configuration)  |
/// | transport refused before sending: disabled, limits, endpoint, too large   | Unavailable(Configuration)  |
/// | supplier `InvalidRequest` (it refused the request as built)               | Unavailable(Configuration)  |
/// | invalid credential, transport credential, supplier `Denied`               | Unavailable(Authentication) |
/// | owner, transport or supplier deadline                                     | Deadline                    |
/// | any cancellation, provably never sent                                     | Unavailable(Interrupted)    |
/// | any cancellation, possibly sent                                           | Unknown                     |
/// | other owner errors, `Contract`, `UsageExceeded`, supplier `Contract`      | Invalid                     |
/// | other transport errors (dropped, redirect, content type, size)            | Unknown                     |
/// | supplier `Unknown`, `Capacity`, `Unavailable`                             | Unknown                     |
///
/// `Capacity` and `Unavailable` are Unknown because the error cannot say whether the supplier had
/// begun work; only [`request_never_sent`] proves a request never left.
fn failure_outcome(error: &TextResponseError) -> TextOutcome {
    use NativeHttpRefusal as Http;
    use ProviderFailureClass as Supplier;
    let cancelled = || {
        if request_never_sent(error) {
            TextOutcome::Unavailable(UnavailableReason::Interrupted)
        } else {
            TextOutcome::Unknown
        }
    };
    match error.reason() {
        TextResponseError::UnregisteredRoute
        | TextResponseError::InvalidConfiguration
        | TextResponseError::InvalidInput
        | TextResponseError::ModeDoesNotDispatch
        | TextResponseError::SchemaConfigurationMismatch
        | TextResponseError::Transport(
            Http::ProviderDisabled
            | Http::InvalidLimits
            | Http::InvalidConfiguration
            | Http::InvalidEndpoint
            | Http::RequestTooLarge,
        )
        | TextResponseError::Supplier(Supplier::InvalidRequest) => {
            TextOutcome::Unavailable(UnavailableReason::Configuration)
        }
        TextResponseError::InvalidCredential
        | TextResponseError::Transport(Http::InvalidCredential)
        | TextResponseError::Supplier(Supplier::Denied) => {
            TextOutcome::Unavailable(UnavailableReason::Authentication)
        }
        TextResponseError::Owner(RequestError::DeadlineExceeded)
        | TextResponseError::Transport(Http::Deadline)
        | TextResponseError::Supplier(Supplier::Deadline) => TextOutcome::Deadline,
        TextResponseError::Owner(RequestError::Cancelled)
        | TextResponseError::Transport(Http::Cancelled)
        | TextResponseError::Supplier(Supplier::Cancelled) => cancelled(),
        TextResponseError::Owner(_)
        | TextResponseError::Contract { .. }
        | TextResponseError::UsageExceeded(_)
        | TextResponseError::Supplier(Supplier::Contract) => TextOutcome::Invalid,
        TextResponseError::Transport(_)
        | TextResponseError::Supplier(
            Supplier::Unknown | Supplier::Capacity | Supplier::Unavailable,
        )
        | TextResponseError::Observed { .. } => TextOutcome::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use df_ai::text::{TextProfile, TextPurpose};
    use df_providers::TextResponseContract;
    use serde_json::json;

    use super::*;
    use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply};

    fn usage(total: u128) -> TextTokenUsage {
        TextTokenUsage {
            input_tokens: 100,
            output_tokens: 40,
            total_tokens: total,
            cached_input_tokens: 7,
            cache_write_tokens: Some(3),
            reasoning_output_tokens: 11,
        }
    }

    fn with_usage(failure: TextResponseError) -> TextResponseError {
        TextResponseError::Observed {
            failure: Box::new(failure),
            provider_request_id: None,
            transport_request_id: None,
            usage: Some(Box::new(usage(140))),
        }
    }

    fn outcome(error: TextResponseError) -> TextOutcome {
        failure_outcome(&error)
    }

    fn request() -> TextRequest {
        TextRequest {
            purpose: TextPurpose::Storyteller,
            profile: TextProfile::new("responses-test").unwrap(),
            instructions: "Greet the traveller.".to_owned(),
            facts: json!({"npc": "Courier"}),
            output_name: "greeting".to_owned(),
            output_schema: json!({
                "type": "object",
                "properties": {"reply": {"type": "string"}},
                "required": ["reply"],
                "additionalProperties": false
            }),
            deadline: Duration::from_secs(5),
        }
    }

    #[test]
    fn a_full_usage_report_converts_every_category() {
        assert_eq!(
            usage_from(usage(140)),
            Some(TextUsage {
                total_tokens: 140,
                input_tokens: Some(100),
                output_tokens: Some(40),
                cached_input_tokens: Some(7),
                cache_write_tokens: Some(3),
                reasoning_output_tokens: Some(11),
            })
        );
    }

    #[test]
    fn a_total_beyond_u64_makes_the_whole_usage_unknown_not_saturated() {
        let overflow = u128::from(u64::MAX) + 1;
        assert_eq!(usage_from(usage(overflow)), None);
    }

    #[test]
    fn a_category_beyond_u64_becomes_unknown_while_the_rest_stay() {
        let overflow = u128::from(u64::MAX) + 1;
        let converted = usage_from(TextTokenUsage {
            reasoning_output_tokens: overflow,
            cache_write_tokens: Some(overflow),
            ..usage(140)
        })
        .unwrap();
        assert_eq!(converted.reasoning_output_tokens, None);
        assert_eq!(converted.cache_write_tokens, None);
        assert_eq!(converted.total_tokens, 140);
        assert_eq!(converted.input_tokens, Some(100));
    }

    #[test]
    fn an_absent_cache_write_report_stays_unknown() {
        let converted = usage_from(TextTokenUsage {
            cache_write_tokens: None,
            ..usage(140)
        })
        .unwrap();
        assert_eq!(converted.cache_write_tokens, None);
    }

    #[test]
    fn configuration_failures_are_unavailable_configuration() {
        let configuration = TextOutcome::Unavailable(UnavailableReason::Configuration);
        for error in [
            TextResponseError::UnregisteredRoute,
            TextResponseError::InvalidConfiguration,
            TextResponseError::InvalidInput,
            TextResponseError::ModeDoesNotDispatch,
            TextResponseError::SchemaConfigurationMismatch,
            TextResponseError::Transport(NativeHttpRefusal::ProviderDisabled),
            TextResponseError::Transport(NativeHttpRefusal::InvalidLimits),
            TextResponseError::Transport(NativeHttpRefusal::InvalidConfiguration),
            TextResponseError::Transport(NativeHttpRefusal::InvalidEndpoint),
            TextResponseError::Transport(NativeHttpRefusal::RequestTooLarge),
            TextResponseError::Supplier(ProviderFailureClass::InvalidRequest),
        ] {
            assert_eq!(outcome(error), configuration);
        }
    }

    #[test]
    fn credential_and_denial_failures_are_unavailable_authentication() {
        let authentication = TextOutcome::Unavailable(UnavailableReason::Authentication);
        for error in [
            TextResponseError::InvalidCredential,
            TextResponseError::Transport(NativeHttpRefusal::InvalidCredential),
            TextResponseError::Supplier(ProviderFailureClass::Denied),
        ] {
            assert_eq!(outcome(error), authentication);
        }
    }

    #[test]
    fn deadlines_from_owner_transport_and_supplier_are_deadline() {
        for error in [
            TextResponseError::Owner(RequestError::DeadlineExceeded),
            TextResponseError::Transport(NativeHttpRefusal::Deadline),
            TextResponseError::Supplier(ProviderFailureClass::Deadline),
        ] {
            assert_eq!(outcome(error), TextOutcome::Deadline);
        }
    }

    #[test]
    fn contract_usage_and_owner_validation_failures_are_invalid() {
        for error in [
            TextResponseError::Contract {
                reason: TextResponseContract::Schema,
                usage: None,
            },
            TextResponseError::Contract {
                reason: TextResponseContract::Refusal,
                usage: Some(Box::new(usage(140))),
            },
            TextResponseError::UsageExceeded(Box::new(usage(140))),
            TextResponseError::Owner(RequestError::SemanticBasisMismatch),
            TextResponseError::Supplier(ProviderFailureClass::Contract),
        ] {
            assert_eq!(outcome(error), TextOutcome::Invalid);
        }
    }

    #[test]
    fn a_cancellation_before_anything_was_sent_is_interrupted() {
        assert_eq!(
            outcome(TextResponseError::Owner(RequestError::Cancelled)),
            TextOutcome::Unavailable(UnavailableReason::Interrupted)
        );
    }

    #[test]
    fn a_cancellation_that_may_have_been_sent_is_unknown() {
        for error in [
            TextResponseError::Transport(NativeHttpRefusal::Cancelled),
            TextResponseError::Supplier(ProviderFailureClass::Cancelled),
            with_usage(TextResponseError::Owner(RequestError::Cancelled)),
        ] {
            assert_eq!(outcome(error), TextOutcome::Unknown);
        }
    }

    #[test]
    fn a_failure_after_the_request_may_have_arrived_is_unknown() {
        for error in [
            TextResponseError::Transport(NativeHttpRefusal::Unknown),
            TextResponseError::Transport(NativeHttpRefusal::ContentType),
            TextResponseError::Transport(NativeHttpRefusal::Redirect),
            TextResponseError::Transport(NativeHttpRefusal::ResponseTooLarge),
            TextResponseError::Transport(NativeHttpRefusal::HeadersTooLarge),
            TextResponseError::Transport(NativeHttpRefusal::TooManyChunks),
            TextResponseError::Supplier(ProviderFailureClass::Unknown),
            TextResponseError::Supplier(ProviderFailureClass::Capacity),
            TextResponseError::Supplier(ProviderFailureClass::Unavailable),
        ] {
            assert_eq!(outcome(error), TextOutcome::Unknown);
        }
    }

    #[test]
    fn an_observed_wrapper_is_classified_by_the_failure_inside_it() {
        let wrapped = TextResponseError::Observed {
            failure: Box::new(TextResponseError::Supplier(ProviderFailureClass::Denied)),
            provider_request_id: None,
            transport_request_id: None,
            usage: None,
        };
        assert_eq!(
            outcome(wrapped),
            TextOutcome::Unavailable(UnavailableReason::Authentication)
        );
    }

    #[test]
    fn supplier_usage_survives_every_failure_class_that_carries_it() {
        let expected = usage_from(usage(140));
        for error in [
            with_usage(TextResponseError::Owner(RequestError::DeadlineExceeded)),
            with_usage(TextResponseError::Supplier(ProviderFailureClass::Unknown)),
            TextResponseError::Contract {
                reason: TextResponseContract::Status,
                usage: Some(Box::new(usage(140))),
            },
            TextResponseError::UsageExceeded(Box::new(usage(140))),
        ] {
            assert_eq!(error_receipt(&error).usage, expected);
        }
        assert_eq!(
            error_receipt(&TextResponseError::Transport(NativeHttpRefusal::Unknown)).usage,
            None
        );
    }

    #[test]
    fn a_failure_with_an_oversized_total_keeps_its_outcome_and_drops_usage() {
        let overflow = u128::from(u64::MAX) + 1;
        let receipt = error_receipt(&TextResponseError::UsageExceeded(Box::new(usage(overflow))));
        assert_eq!(receipt.outcome, TextOutcome::Invalid);
        assert_eq!(receipt.usage, None);
    }

    #[test]
    fn construction_rejects_a_bad_model_credential_or_output_bound() {
        assert!(ResponsesTextBackend::new("", "credential", 100).is_err());
        assert!(ResponsesTextBackend::new("gpt test", "credential", 100).is_err());
        assert!(ResponsesTextBackend::new("gpt-test", "", 100).is_err());
        assert!(ResponsesTextBackend::new("gpt-test", "has space", 100).is_err());
        assert!(ResponsesTextBackend::new("gpt-test", "credential", 0).is_err());
        assert!(ResponsesTextBackend::new("gpt-test", "credential", 100).is_ok());
    }

    #[test]
    fn the_prompt_is_the_instructions_a_blank_line_and_compact_facts() {
        assert_eq!(
            prompt_text("Greet.", &json!({"a": 1, "b": [true]})),
            "Greet.\n\n{\"a\":1,\"b\":[true]}"
        );
    }

    #[test]
    fn the_request_key_changes_with_every_input_and_not_with_a_shifted_boundary() {
        let base = request();
        let schema = serde_json::to_vec(&base.output_schema).unwrap();
        let key = request_key(&base, &schema);
        assert_eq!(key, request_key(&base, &schema));
        let mut renamed = request();
        renamed.output_name.push('x');
        let mut reworded = request();
        reworded.instructions.push('x');
        let mut refacted = request();
        refacted.facts = json!({"npc": "Guard"});
        for changed in [renamed, reworded, refacted] {
            assert_ne!(key, request_key(&changed, &schema));
        }
        assert_ne!(key, request_key(&base, b"{}"));
        let mut left = request();
        left.output_name = "ab".to_owned();
        left.instructions = "c".to_owned();
        let mut right = request();
        right.output_name = "a".to_owned();
        right.instructions = "bc".to_owned();
        assert_ne!(request_key(&left, &schema), request_key(&right, &schema));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_loopback_answer_is_returned_with_the_supplier_usage() {
        let endpoint = FakeEndpoint::start(vec![Reply::Output(json!({"reply": "Well met."}))]);
        let backend = ResponsesTextBackend::new("gpt-test-model", "credential-fixture", 400)
            .unwrap()
            .with_loopback(endpoint.address);

        let receipt = backend.complete(&request()).await;

        assert_eq!(
            receipt.outcome,
            TextOutcome::Answered(json!({"reply": "Well met."}))
        );
        assert_eq!(
            receipt.usage,
            Some(TextUsage {
                total_tokens: 140,
                input_tokens: Some(100),
                output_tokens: Some(40),
                cached_input_tokens: Some(0),
                cache_write_tokens: None,
                reasoning_output_tokens: Some(0),
            })
        );
        assert_eq!(endpoint.connections(), 1);
        let sent = &endpoint.bodies()[0];
        assert_eq!(sent["model"], "gpt-test-model");
        assert_eq!(
            sent["input"],
            "Greet the traveller.\n\n{\"npc\":\"Courier\"}"
        );
    }
}
