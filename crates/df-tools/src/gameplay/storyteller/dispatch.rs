//! How the storyteller reaches a model: one request through the run's durable text service.
//!
//! The service owns the attempt record, the send marker and the backend; this module owns what the
//! storyteller keeps: the cost-book bookings, the token ledger, and the verdict on each answer. An
//! answer comes back as a candidate that the storyteller validates, then accepts or rejects here
//! (`conclude`). It never calls a provider or the Codex CLI directly, never retries a request that
//! may have been sent, and fails closed with a provider error when no text runtime is installed.
//!
//! The disk cache and the in-process flights stay in the parent module until the service owns them.

use std::{
    collections::VecDeque,
    sync::{
        Mutex, OnceLock, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use df_ai::{
    text::{
        TextBackend, TextOutcome, TextProfile, TextPurpose, TextReceipt, TextRequest,
        TextRequestError, TextUsage, UnavailableReason,
    },
    text_attempts::{DurableTextError, OperationKey, Qualification, Submission, Verdict},
};
use df_providers::{ProviderFailureClass, TextTokenUsage};
use serde_json::{Value, json};

use super::{
    Attempt, Expression, MAXIMUM_PROMPT_BYTES, OUTPUT_NAME, Online, StorytellerError,
    TextResourceStatus, booking_estimate, build_prompt_inner, hex, output_schema,
    output_token_limit, record_text_resource, string_fields,
};
use crate::gameplay::{
    codex_cli::CodexTextBackend,
    cost,
    resources::Unit,
    responses_text::ResponsesTextBackend,
    text_runtime::{self, RuntimeService, TextRuntime},
};

/// How many accepted attempts `Storyteller::forget` can still withdraw. The oldest are dropped
/// first; a caller rejecting an answer long after it was accepted has nothing left to invalidate.
const MAXIMUM_REMEMBERED_ACCEPTANCES: usize = 32;

/// Operation ids made by this process share one counter, so no two calls ever share a key.
static OPERATIONS: AtomicU64 = AtomicU64::new(0);
/// Separates this process's operation ids from those of an earlier run in a durable store.
static PROCESS_EPOCH: OnceLock<Option<String>> = OnceLock::new();

/// The backend one attempt runs on. Both are private adapters; the service is the only caller.
pub(super) enum Route {
    Responses(ResponsesTextBackend),
    Codex(CodexTextBackend),
}

impl TextBackend for Route {
    async fn complete(&self, request: &TextRequest) -> TextReceipt {
        match self {
            Self::Responses(backend) => backend.complete(request).await,
            Self::Codex(backend) => backend.complete(request).await,
        }
    }
}

/// A request in flight, and after it answers, the answer's candidate awaiting a verdict.
pub(super) struct Pending {
    service: RuntimeService<Route>,
    key: OperationKey,
    request: TextRequest,
}

/// What a call was charged before it was sent: the estimate on the token ledger and its entry on
/// the cost book.
struct Charge {
    reserved: u64,
    billed: cost::BilledCall,
}

/// What the service says about one call: whether it made nothing new, and what the supplier
/// reported.
struct Reply {
    replayed: bool,
    usage: Option<TextUsage>,
}

struct Accepted {
    cache_key: [u8; 32],
    pending: Box<Pending>,
}

/// The accepted attempts `forget` can still withdraw, newest last.
#[derive(Default)]
pub(super) struct Acceptances(Mutex<VecDeque<Accepted>>);

impl Acceptances {
    fn remember(&self, cache_key: &[u8; 32], pending: Box<Pending>) {
        let mut accepted = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if accepted.len() >= MAXIMUM_REMEMBERED_ACCEPTANCES {
            accepted.pop_front();
        }
        accepted.push_back(Accepted {
            cache_key: *cache_key,
            pending,
        });
    }

    fn take(&self, cache_key: &[u8; 32]) -> Vec<Pending> {
        let mut accepted = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let (taken, kept) = std::mem::take(&mut *accepted)
            .into_iter()
            .partition::<VecDeque<_>, _>(|entry| &entry.cache_key == cache_key);
        *accepted = kept;
        taken.into_iter().map(|entry| *entry.pending).collect()
    }
}

impl Online {
    /// The run's text runtime: the one present when the storyteller was built, else the first one
    /// installed. A storyteller keeps one runtime for its life, so a replaced global cannot split
    /// an answer from its verdict.
    fn runtime(&self) -> Option<&std::sync::Arc<TextRuntime>> {
        if let Some(runtime) = self.runtime.get() {
            return Some(runtime);
        }
        let current = text_runtime::current()?;
        Some(self.runtime.get_or_init(|| current))
    }

    /// One model attempt through the text service, booked on the token ledger and the cost book.
    /// Returns the answer's string fields with its candidate, or the typed failure; the caller
    /// owes the candidate a verdict through [`Self::conclude`].
    pub(super) async fn attempt(
        &self,
        request: &Expression<'_>,
        key: &[u8; 32],
        attempt: u64,
        prompt: &str,
        reserved: u64,
        started: Instant,
        complaint: Option<&str>,
    ) -> Result<Attempt, StorytellerError> {
        let remaining_time = request
            .profile
            .deadline
            .unwrap_or(self.deadline)
            .saturating_sub(started.elapsed());
        if remaining_time.is_zero() {
            return Err(StorytellerError::DeadlineExceeded);
        }
        let text_request = self.text_request(request, prompt, complaint, remaining_time)?;
        let route = self.route(request)?;
        let Some(runtime) = self.runtime() else {
            return Err(self.unavailable(request, key, "text_runtime_absent"));
        };
        let Some(operation) = operation_key(runtime, key, attempt) else {
            return Err(self.unavailable(request, key, "operation_key"));
        };
        let pending = Box::new(Pending {
            service: runtime.service(route),
            key: operation,
            request: text_request,
        });
        // The call is about to be sent: the estimate is charged from here. It stays charged if the
        // call outlives its deadline or its outcome is unknown, and is corrected when the supplier
        // reports what the call used (or gives back when the call provably never left).
        self.kit.get().charge(Unit::TextToken, reserved);
        let charge = Charge {
            reserved,
            billed: self.begin_call(request, prompt),
        };
        let submission = pending.service.submit(&pending.key, &pending.request).await;
        match submission {
            Err(error) => {
                self.give_back(charge);
                Err(self.durable_failure(request, key, error))
            }
            Ok(Submission::Candidate {
                answer,
                replayed,
                usage,
            })
            | Ok(Submission::Settled {
                outcome: TextOutcome::Answered(answer),
                replayed,
                usage,
            }) => {
                let reply = Reply { replayed, usage };
                self.answered(request, key, pending, answer, reply, charge)
                    .await
            }
            Ok(Submission::Settled {
                outcome,
                replayed,
                usage,
            }) => {
                let reply = Reply { replayed, usage };
                self.failed(request, key, outcome, reply, charge)
            }
            Ok(Submission::Unknown) => {
                drop(charge.billed);
                self.record_failure(request, key, "unknown");
                Err(StorytellerError::Provider(ProviderFailureClass::Unknown))
            }
            Ok(Submission::Rejected { .. } | Submission::InFlight | Submission::Cancelled) => {
                self.give_back(charge);
                Err(self.unavailable(request, key, "not_dispatched"))
            }
        }
    }

    /// Records the verdict on a candidate. A candidate the feature accepted comes back so the
    /// caller can [`Self::remember`] it; a verdict the service could not apply is recorded and
    /// never changes what the caller does with the text.
    pub(super) async fn conclude(
        &self,
        pending: Box<Pending>,
        verdict: Verdict,
        request: &Expression<'_>,
        key: &[u8; 32],
    ) -> Option<Box<Pending>> {
        match pending
            .service
            .qualify(&pending.key, &pending.request, verdict)
            .await
        {
            Ok(Qualification::Qualified) => Some(pending),
            Ok(Qualification::Rejected) => None,
            Ok(Qualification::NotACandidate) => {
                self.record_verdict_failure(request, key, "verdict_not_candidate");
                None
            }
            Ok(Qualification::Stale) => {
                self.record_verdict_failure(request, key, "verdict_stale");
                None
            }
            Err(error) => {
                self.record_verdict_failure(request, key, durable_label(error));
                None
            }
        }
    }

    /// Keeps an accepted attempt so a caller that later rejects its text can withdraw it.
    pub(super) fn remember(&self, key: &[u8; 32], accepted: Box<Pending>) {
        self.accepted.remember(key, accepted);
    }

    /// Withdraws every accepted attempt of the request keyed `key` from the service.
    pub(super) async fn invalidate_accepted(&self, request: &Expression<'_>, key: &[u8; 32]) {
        for pending in self.accepted.take(key) {
            if let Err(error) = pending
                .service
                .invalidate(&pending.key, &pending.request)
                .await
            {
                self.record_verdict_failure(request, key, durable_label(error));
            }
        }
    }

    fn text_request(
        &self,
        request: &Expression<'_>,
        prompt: &str,
        complaint: Option<&str>,
        remaining_time: std::time::Duration,
    ) -> Result<TextRequest, StorytellerError> {
        if prompt.len() > MAXIMUM_PROMPT_BYTES {
            return Err(StorytellerError::InvalidRequest("prompt"));
        }
        let schema = output_schema(request.slots).map_err(StorytellerError::InvalidRequest)?;
        // The Responses route sends the instructions and then the facts, so the prompt keeps its
        // own FACTS line and no facts follow it. Codex takes the facts as a separate field.
        let (profile, instructions, facts) = if self.codex {
            (
                "storyteller-codex",
                build_prompt_inner(request, complaint, false),
                request.facts.clone(),
            )
        } else {
            ("storyteller-responses", prompt.to_owned(), json!({}))
        };
        Ok(TextRequest {
            purpose: TextPurpose::Storyteller,
            profile: TextProfile::new(profile)
                .map_err(|_| StorytellerError::InvalidRequest("profile"))?,
            instructions,
            facts,
            output_name: OUTPUT_NAME.to_owned(),
            output_schema: schema.json_schema().clone(),
            deadline: remaining_time,
        })
    }

    fn route(&self, request: &Expression<'_>) -> Result<Route, StorytellerError> {
        if self.codex {
            return Ok(Route::Codex(CodexTextBackend));
        }
        let output_tokens = u32::try_from(output_token_limit(request))
            .map_err(|_| StorytellerError::InvalidRequest("tokens"))?;
        let mut backend = ResponsesTextBackend::new(&self.model, &self.credential, output_tokens)
            .map_err(|error| StorytellerError::Provider(error.failure_class()))?;
        if let Some(effort) = request.profile.reasoning {
            backend = backend.with_reasoning(effort);
        }
        #[cfg(test)]
        if let Some(address) = self.loopback {
            backend = backend.with_loopback(address);
        }
        Ok(Route::Responses(backend))
    }

    fn begin_call(&self, request: &Expression<'_>, prompt: &str) -> cost::BilledCall {
        if self.codex {
            self.kit.get().hub().ledger().begin_call(
                cost::codex_text_call(request.purpose),
                &[(cost::Category::Calls, 1)],
            )
        } else {
            self.kit.get().hub().ledger().begin_call(
                cost::text_call(&self.model, request.purpose),
                &booking_estimate(prompt, request),
            )
        }
    }

    async fn answered(
        &self,
        request: &Expression<'_>,
        key: &[u8; 32],
        pending: Box<Pending>,
        answer: Value,
        reply: Reply,
        charge: Charge,
    ) -> Result<Attempt, StorytellerError> {
        // A replayed answer was paid for when it was made: this call spent nothing.
        if reply.replayed {
            self.give_back(charge);
        } else {
            self.settle_paid(charge, reply.usage.as_ref());
            #[cfg(test)]
            if let Some(counters) = reply.usage.as_ref().and_then(token_usage)
                && !self.codex
            {
                self.usage_log
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(counters);
            }
        }
        let tokens = if reply.replayed {
            0
        } else {
            reply.usage.as_ref().map_or(0, |usage| usage.total_tokens)
        };
        let limit = u64::try_from(output_token_limit(request)).unwrap_or(u64::MAX);
        if self.codex
            && reply
                .usage
                .as_ref()
                .and_then(|usage| usage.output_tokens)
                .is_some_and(|written| written > limit)
        {
            self.conclude(pending, Verdict::Rejected, request, key)
                .await;
            return Err(StorytellerError::Provider(ProviderFailureClass::Contract));
        }
        match string_fields(&answer) {
            Some(values) => Ok(Attempt::Text(values, tokens, pending)),
            None => {
                self.conclude(pending, Verdict::Rejected, request, key)
                    .await;
                Ok(Attempt::BadShape)
            }
        }
    }

    fn failed(
        &self,
        request: &Expression<'_>,
        key: &[u8; 32],
        outcome: TextOutcome,
        reply: Reply,
        charge: Charge,
    ) -> Result<Attempt, StorytellerError> {
        if reply.replayed || (reply.usage.is_none() && never_sent(&outcome)) {
            self.give_back(charge);
        } else if reply.usage.is_some() {
            self.settle_paid(charge, reply.usage.as_ref());
        } else {
            drop(charge.billed);
        }
        match outcome {
            // The supplier answered but not in the requested shape; one retry is allowed. The
            // backend reports a malformed answer and a refusal the same way, so only paid answers
            // are retried.
            TextOutcome::Invalid if reply.usage.is_some() => Ok(Attempt::BadShape),
            TextOutcome::Deadline => Err(StorytellerError::DeadlineExceeded),
            other => {
                self.record_failure(request, key, outcome_label(&other));
                Err(StorytellerError::Provider(failure_class(&other)))
            }
        }
    }

    /// Replaces the estimate with what the supplier reported and books the call. A usage with a
    /// category the supplier left out is booked as an unknown outcome, never as zero.
    fn settle_paid(&self, charge: Charge, usage: Option<&TextUsage>) {
        if let Some(usage) = usage {
            self.reconcile(charge.reserved, u128::from(usage.total_tokens));
        }
        if self.codex {
            charge.billed.settle(cost::subscription_usage());
            return;
        }
        match usage.and_then(token_usage) {
            Some(counters) => charge.billed.settle(cost::text_usage(&counters)),
            None => drop(charge.billed),
        }
    }

    fn give_back(&self, charge: Charge) {
        self.release(charge.reserved);
        charge.billed.not_billed();
    }

    fn unavailable(
        &self,
        request: &Expression<'_>,
        key: &[u8; 32],
        label: &'static str,
    ) -> StorytellerError {
        self.record_failure(request, key, label);
        StorytellerError::Provider(ProviderFailureClass::Unavailable)
    }

    fn durable_failure(
        &self,
        request: &Expression<'_>,
        key: &[u8; 32],
        error: DurableTextError,
    ) -> StorytellerError {
        self.record_failure(request, key, durable_label(error));
        match error {
            DurableTextError::Refused(refusal) => StorytellerError::InvalidRequest(match refusal {
                TextRequestError::EmptyInstructions => "instructions",
                TextRequestError::InstructionsTooLarge => "prompt",
                TextRequestError::FactsTooLarge => "facts",
                TextRequestError::InvalidDeadline => "deadline",
                TextRequestError::InvalidSchema => "schema",
            }),
            DurableTextError::Conflict
            | DurableTextError::Store(_)
            | DurableTextError::Unauthorized
            | DurableTextError::UnknownOperation => {
                StorytellerError::Provider(ProviderFailureClass::Unavailable)
            }
        }
    }

    fn record_failure(&self, request: &Expression<'_>, key: &[u8; 32], label: &'static str) {
        record_text_resource(
            "gameplay.storyteller.provider_error",
            self,
            request,
            key,
            TextResourceStatus {
                stage: "provider_error",
                status: "provider_error",
                failure_category: Some(label),
                elapsed_ms: None,
                slot_counts: None,
            },
        );
    }

    fn record_verdict_failure(
        &self,
        request: &Expression<'_>,
        key: &[u8; 32],
        label: &'static str,
    ) {
        record_text_resource(
            "gameplay.storyteller.verdict_failed",
            self,
            request,
            key,
            TextResourceStatus {
                stage: "failed",
                status: "degraded",
                failure_category: Some(label),
                elapsed_ms: None,
                slot_counts: None,
            },
        );
    }
}

/// The key of one attempt: the request's cache key and attempt number, made unique by this
/// process and call. The service replays a settled key and refuses a changed request under it,
/// but the storyteller's own cache decides when a request is asked again, so every call is new.
fn operation_key(
    runtime: &TextRuntime,
    cache_key: &[u8; 32],
    attempt: u64,
) -> Option<OperationKey> {
    let epoch = PROCESS_EPOCH
        .get_or_init(|| {
            text_runtime::process_owner()
                .and_then(|owner| owner.as_bytes().last_chunk::<8>().map(|tail| hex(tail)))
        })
        .as_deref()?;
    let sequence = OPERATIONS.fetch_add(1, Ordering::Relaxed);
    OperationKey::new(
        runtime.scope(),
        &format!("{}-{attempt}-{epoch}-{sequence}", hex(cache_key)),
    )
    .ok()
}

/// The cost book's token categories need input, output and cached input; a supplier that left one
/// out makes the whole report unknown.
pub(super) fn token_usage(usage: &TextUsage) -> Option<TextTokenUsage> {
    Some(TextTokenUsage {
        input_tokens: u128::from(usage.input_tokens?),
        output_tokens: u128::from(usage.output_tokens?),
        total_tokens: u128::from(usage.total_tokens),
        cached_input_tokens: u128::from(usage.cached_input_tokens?),
        cache_write_tokens: usage.cache_write_tokens.map(u128::from),
        reasoning_output_tokens: u128::from(usage.reasoning_output_tokens?),
    })
}

/// Whether an outcome without usage proves the request never reached the supplier.
fn never_sent(outcome: &TextOutcome) -> bool {
    matches!(
        outcome,
        TextOutcome::Unavailable(
            UnavailableReason::Configuration
                | UnavailableReason::Authentication
                | UnavailableReason::Interrupted
        )
    )
}

fn failure_class(outcome: &TextOutcome) -> ProviderFailureClass {
    match outcome {
        TextOutcome::Unavailable(UnavailableReason::Configuration) => {
            ProviderFailureClass::InvalidRequest
        }
        TextOutcome::Unavailable(UnavailableReason::Authentication) => ProviderFailureClass::Denied,
        TextOutcome::Unavailable(UnavailableReason::UnsafeTools) => {
            ProviderFailureClass::Unavailable
        }
        TextOutcome::Unavailable(UnavailableReason::Interrupted) | TextOutcome::Stale => {
            ProviderFailureClass::Cancelled
        }
        TextOutcome::Deadline => ProviderFailureClass::Deadline,
        TextOutcome::Answered(_) | TextOutcome::Invalid => ProviderFailureClass::Contract,
        TextOutcome::Unknown => ProviderFailureClass::Unknown,
    }
}

/// A fixed label for the audit line of a failed call; never prompt, answer or supplier text.
pub(super) fn outcome_label(outcome: &TextOutcome) -> &'static str {
    match outcome {
        TextOutcome::Answered(_) => "answered",
        TextOutcome::Unavailable(UnavailableReason::Configuration) => "unavailable_configuration",
        TextOutcome::Unavailable(UnavailableReason::Authentication) => "unavailable_authentication",
        TextOutcome::Unavailable(UnavailableReason::UnsafeTools) => "unavailable_unsafe_tools",
        TextOutcome::Unavailable(UnavailableReason::Interrupted) => "unavailable_interrupted",
        TextOutcome::Deadline => "deadline",
        TextOutcome::Invalid => "invalid",
        TextOutcome::Stale => "stale",
        TextOutcome::Unknown => "unknown",
    }
}

fn durable_label(error: DurableTextError) -> &'static str {
    match error {
        DurableTextError::Refused(refusal) => refusal.label(),
        DurableTextError::Conflict => "operation_conflict",
        DurableTextError::Store(store) => store.label(),
        DurableTextError::Unauthorized => "unauthorized",
        DurableTextError::UnknownOperation => "unknown_operation",
    }
}
