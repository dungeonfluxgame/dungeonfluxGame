//! Listener-safe expression request production and structured output qualification.
//! This module neither sends work nor grants source, spend, reveal or publication authority.
use std::fmt;

use df_interaction::conversation::{
    ConversationContext, ConversationError, ConversationObservation, ConversationPlan,
    project_conversation,
};
use df_interaction::speech::{
    DeclaredUncertainty, ExpressionBasis, ExpressionClaim, ExpressionContext, ExpressionMetadata,
    SlotKind, SpeechError, SpeechIntent, SpeechObservation,
};
use df_model::checkpoint::{ContentReference, EntityId, RecordId};
use df_provider_api::{
    CheckedRequest, RequestBinding, RequestError, RequestLimits, RequestOwnerState, RequestUsage,
};

#[derive(Debug, Eq, PartialEq)]
pub enum ExpressionRequestError {
    Speech(SpeechError),
    Empty,
    Binding,
    Capacity,
    Allocation,
    Request(RequestError),
}

#[derive(Debug, Eq, PartialEq)]
pub enum ConversationExpressionError {
    Conversation(ConversationError),
    Request(ExpressionRequestError),
}

/// A checked request whose conversation permission remains bound for egress and output.
pub struct PreparedConversationExpression<'a> {
    conversation: ConversationContext<'a>,
    request: CheckedRequest<ExpressionBasis>,
}

impl fmt::Debug for PreparedConversationExpression<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedConversationExpression")
            .finish_non_exhaustive()
    }
}

impl<'a> PreparedConversationExpression<'a> {
    pub fn request(&self) -> &CheckedRequest<ExpressionBasis> {
        &self.request
    }

    pub fn metadata(&self) -> ExpressionMetadata<'_> {
        self.conversation.metadata()
    }

    /// Recheck conversation permission before egress and again before output use.
    pub fn validate_current(
        &self,
        current: &ConversationObservation<'_>,
        owner: RequestOwnerState<'_, ExpressionBasis>,
    ) -> Result<(), ConversationExpressionError> {
        self.conversation
            .validate_current(current)
            .map_err(ConversationExpressionError::Conversation)?;
        self.request.validate_current(owner).map_err(|error| {
            ConversationExpressionError::Request(ExpressionRequestError::Request(error))
        })
    }
}

/// Holds only the actual checked request and the source of its public expression bytes.
pub struct PreparedExpression<'a> {
    context: ExpressionContext<'a>,
    request: CheckedRequest<ExpressionBasis>,
}

impl fmt::Debug for PreparedExpression<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedExpression").finish_non_exhaustive()
    }
}

impl<'a> PreparedExpression<'a> {
    pub fn request(&self) -> &CheckedRequest<ExpressionBasis> {
        &self.request
    }
    pub fn metadata(&self) -> ExpressionMetadata<'_> {
        self.context.metadata()
    }
    pub fn context(&self) -> &ExpressionContext<'a> {
        &self.context
    }

    /// Native egress must use this observation and must not serialize the private semantic binding.
    pub fn validate_current(
        &self,
        current: &SpeechObservation<'_>,
        owner: RequestOwnerState<'_, ExpressionBasis>,
    ) -> Result<(), ExpressionRequestError> {
        self.context
            .validate_current(current)
            .map_err(ExpressionRequestError::Speech)?;
        self.request
            .validate_current(owner)
            .map_err(ExpressionRequestError::Request)
    }
}

fn append_bounded_bytes(
    bytes: &mut Vec<u8>,
    value: &[u8],
    maximum_bytes: usize,
) -> Result<(), ExpressionRequestError> {
    let length = bytes
        .len()
        .checked_add(value.len())
        .ok_or(ExpressionRequestError::Capacity)?;
    if length > maximum_bytes {
        return Err(ExpressionRequestError::Capacity);
    }
    bytes
        .try_reserve(value.len())
        .map_err(|_| ExpressionRequestError::Allocation)?;
    bytes.extend_from_slice(value);
    Ok(())
}

fn append_length_prefixed_bytes(
    bytes: &mut Vec<u8>,
    value: &[u8],
    maximum_bytes: usize,
) -> Result<(), ExpressionRequestError> {
    let length = u64::try_from(value.len()).map_err(|_| ExpressionRequestError::Capacity)?;
    append_bounded_bytes(bytes, &length.to_be_bytes(), maximum_bytes)?;
    append_bounded_bytes(bytes, value, maximum_bytes)
}

fn intent_byte(intent: SpeechIntent) -> u8 {
    match intent {
        SpeechIntent::Claim => 0,
        SpeechIntent::ApprovedDeceit => 1,
        SpeechIntent::FalseBelief => 2,
    }
}
fn uncertainty_byte(value: DeclaredUncertainty) -> u8 {
    match value {
        DeclaredUncertainty::NoneDeclared => 0,
        DeclaredUncertainty::Explicit => 1,
    }
}
fn slot_byte(value: SlotKind) -> u8 {
    match value {
        SlotKind::Outcome => 0,
        SlotKind::Name => 1,
        SlotKind::Number => 2,
    }
}

/// The versioned expression-input format is owned here, not a second Model/RPC serializer.
/// Every prompt/retrieval/style/asset field comes from the permitted canonical claim selection.
fn encode_context(
    context: &ExpressionContext<'_>,
    maximum_bytes: usize,
) -> Result<Vec<u8>, ExpressionRequestError> {
    let mut bytes = Vec::new();
    append_bounded_bytes(&mut bytes, b"df-expression-input/1\n", maximum_bytes)?;
    append_length_prefixed_bytes(
        &mut bytes,
        context.metadata().locale.as_bytes(),
        maximum_bytes,
    )?;
    let count =
        u64::try_from(context.claims().len()).map_err(|_| ExpressionRequestError::Capacity)?;
    append_bounded_bytes(&mut bytes, &count.to_be_bytes(), maximum_bytes)?;
    for claim in context.claims() {
        append_length_prefixed_bytes(&mut bytes, claim.id.as_bytes(), maximum_bytes)?;
        append_length_prefixed_bytes(&mut bytes, claim.holder.as_bytes(), maximum_bytes)?;
        append_length_prefixed_bytes(&mut bytes, claim.subject.as_bytes(), maximum_bytes)?;
        append_bounded_bytes(
            &mut bytes,
            &[
                intent_byte(claim.intent),
                uncertainty_byte(claim.uncertainty),
            ],
            maximum_bytes,
        )?;
        append_length_prefixed_bytes(&mut bytes, claim.text.as_bytes(), maximum_bytes)?;
        // Retrieval keys are the same already-visible evidence, never a private query context.
        let evidence =
            u64::try_from(claim.evidence.len()).map_err(|_| ExpressionRequestError::Capacity)?;
        append_bounded_bytes(&mut bytes, &evidence.to_be_bytes(), maximum_bytes)?;
        for id in claim.evidence {
            append_length_prefixed_bytes(&mut bytes, id.as_bytes(), maximum_bytes)?;
        }
        // Source style references and subject asset cues have the same listener scope as the claim.
        append_length_prefixed_bytes(
            &mut bytes,
            claim.source.package.as_str().as_bytes(),
            maximum_bytes,
        )?;
        append_length_prefixed_bytes(
            &mut bytes,
            claim.source.entry.as_str().as_bytes(),
            maximum_bytes,
        )?;
        let slots =
            u64::try_from(claim.slots.len()).map_err(|_| ExpressionRequestError::Capacity)?;
        append_bounded_bytes(&mut bytes, &slots.to_be_bytes(), maximum_bytes)?;
        for slot in &claim.slots {
            append_bounded_bytes(&mut bytes, &[slot_byte(slot.kind)], maximum_bytes)?;
            append_length_prefixed_bytes(&mut bytes, slot.value.as_bytes(), maximum_bytes)?;
        }
    }
    Ok(bytes)
}

/// No arbitrary prompt/payload parameter exists at this modality boundary.
pub fn prepare_expression_request<'a>(
    context: ExpressionContext<'a>,
    current: &SpeechObservation<'_>,
    binding: RequestBinding<ExpressionBasis>,
    usage: RequestUsage,
    limits: RequestLimits,
    owner: RequestOwnerState<'_, ExpressionBasis>,
) -> Result<PreparedExpression<'a>, ExpressionRequestError> {
    let request = checked_expression_request(&context, current, binding, usage, limits, owner)?;
    Ok(PreparedExpression { context, request })
}

fn checked_expression_request(
    context: &ExpressionContext<'_>,
    current: &SpeechObservation<'_>,
    binding: RequestBinding<ExpressionBasis>,
    usage: RequestUsage,
    limits: RequestLimits,
    owner: RequestOwnerState<'_, ExpressionBasis>,
) -> Result<CheckedRequest<ExpressionBasis>, ExpressionRequestError> {
    context
        .validate_current(current)
        .map_err(ExpressionRequestError::Speech)?;
    if context.claims().is_empty() {
        return Err(ExpressionRequestError::Empty);
    }
    if binding.identity.basis != context.basis()
        || &binding.semantic_basis != context.semantic_basis()
    {
        return Err(ExpressionRequestError::Binding);
    }
    let payload = encode_context(context, limits.max_bytes())?;
    let request = CheckedRequest::new(binding, &payload, usage, limits, owner)
        .map_err(ExpressionRequestError::Request)?;
    Ok(request)
}

/// Consume the actual current conversation projection; no raw prompt enters this boundary.
pub fn prepare_conversation_expression_request<'a>(
    plan: &'a ConversationPlan,
    current: &ConversationObservation<'a>,
    binding: RequestBinding<ExpressionBasis>,
    usage: RequestUsage,
    limits: RequestLimits,
    owner: RequestOwnerState<'_, ExpressionBasis>,
) -> Result<PreparedConversationExpression<'a>, ConversationExpressionError> {
    let conversation =
        project_conversation(plan, current).map_err(ConversationExpressionError::Conversation)?;
    let request = conversation
        .with_expression(|expression| {
            checked_expression_request(expression, &current.speech, binding, usage, limits, owner)
        })
        .map_err(ConversationExpressionError::Request)?;
    Ok(PreparedConversationExpression {
        conversation,
        request,
    })
}

#[derive(Clone, Eq, PartialEq)]
pub struct GroundedValue {
    pub kind: SlotKind,
    pub value: String,
}

#[derive(Clone, Eq, PartialEq)]
pub struct GroundedClause {
    pub claim_id: RecordId,
    pub holder: EntityId,
    pub subject: EntityId,
    pub intent: SpeechIntent,
    pub uncertainty: DeclaredUncertainty,
    pub text: String,
    pub slots: Vec<GroundedValue>,
}

impl fmt::Debug for GroundedClause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GroundedClause").finish_non_exhaustive()
    }
}

/// Unvalidated open flavor is preserved as uncertainty, never approved by a "noncanonical" flag.
pub struct FlavorClause {
    pub noncanonical: bool,
    pub allowed_style: ContentReference,
    pub local_text: String,
}

pub enum ExpressionClause {
    Grounded(GroundedClause),
    Flavor(FlavorClause),
}

/// Closed structured candidates have no tool, URL-action or instruction operation variants.
pub struct SpeechEnvelope {
    pub clauses: Vec<ExpressionClause>,
}

impl fmt::Debug for SpeechEnvelope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpeechEnvelope").finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct QualificationLimits {
    pub maximum_clauses: usize,
    pub maximum_output_bytes: usize,
    pub maximum_slots: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub enum QualificationError {
    Current(ExpressionRequestError),
    Capacity,
    ClauseCount,
    Rejected,
    Uncertain,
    Allocation,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ConversationQualificationError {
    Current(ConversationExpressionError),
    Expression(QualificationError),
}

/// Source-exact grounded output only. This does not commit a reveal or publish captions/TTS.
pub struct QualifiedExpression {
    clauses: Vec<GroundedClause>,
}
impl fmt::Debug for QualifiedExpression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QualifiedExpression")
            .finish_non_exhaustive()
    }
}
impl QualifiedExpression {
    pub fn clauses(&self) -> &[GroundedClause] {
        &self.clauses
    }
}

fn consume(remaining: &mut usize, amount: usize) -> Result<(), QualificationError> {
    *remaining = remaining
        .checked_sub(amount)
        .ok_or(QualificationError::Capacity)?;
    Ok(())
}

/// Exact slot rendering protects canonical names/numbers/outcomes/negation/attribution.
/// Arbitrary natural-language flavor and provider wire decoding remain separate unqualified ports.
pub fn qualify_expression(
    prepared: &PreparedExpression<'_>,
    current: &SpeechObservation<'_>,
    owner: RequestOwnerState<'_, ExpressionBasis>,
    envelope: SpeechEnvelope,
    limits: QualificationLimits,
) -> Result<QualifiedExpression, QualificationError> {
    prepared
        .validate_current(current, owner)
        .map_err(QualificationError::Current)?;
    qualify_claims(prepared.context.claims(), envelope, limits)
}

fn qualify_claims(
    claims: &[ExpressionClaim<'_>],
    envelope: SpeechEnvelope,
    limits: QualificationLimits,
) -> Result<QualifiedExpression, QualificationError> {
    if envelope.clauses.len() > limits.maximum_clauses {
        return Err(QualificationError::Capacity);
    }
    if envelope.clauses.len() != claims.len() {
        return Err(QualificationError::ClauseCount);
    }
    let mut bytes = limits.maximum_output_bytes;
    let mut slots = limits.maximum_slots;
    let mut qualified = Vec::new();
    qualified
        .try_reserve_exact(envelope.clauses.len())
        .map_err(|_| QualificationError::Allocation)?;
    for (candidate, permitted) in envelope.clauses.into_iter().zip(claims) {
        let clause = match candidate {
            ExpressionClause::Grounded(clause) => clause,
            ExpressionClause::Flavor(flavor) => {
                consume(&mut bytes, flavor.local_text.len())?;
                return Err(if flavor.noncanonical {
                    QualificationError::Uncertain
                } else {
                    QualificationError::Rejected
                });
            }
        };
        consume(&mut bytes, clause.text.len())?;
        consume(&mut slots, clause.slots.len())?;
        if clause.claim_id != permitted.id
            || clause.holder != permitted.holder
            || clause.subject != permitted.subject
            || clause.intent != permitted.intent
            || clause.uncertainty != permitted.uncertainty
            || clause.text != permitted.text
            || clause.slots.len() != permitted.slots.len()
        {
            return Err(QualificationError::Rejected);
        }
        for (slot, expected) in clause.slots.iter().zip(&permitted.slots) {
            consume(&mut bytes, slot.value.len())?;
            if slot.kind != expected.kind || slot.value != expected.value {
                return Err(QualificationError::Rejected);
            }
        }
        qualified.push(clause);
    }
    Ok(QualifiedExpression { clauses: qualified })
}

/// Refuse the whole response if conversation permission changed since request preparation.
pub fn qualify_conversation_expression(
    prepared: &PreparedConversationExpression<'_>,
    current: &ConversationObservation<'_>,
    owner: RequestOwnerState<'_, ExpressionBasis>,
    envelope: SpeechEnvelope,
    limits: QualificationLimits,
) -> Result<QualifiedExpression, ConversationQualificationError> {
    prepared
        .validate_current(current, owner)
        .map_err(ConversationQualificationError::Current)?;
    prepared
        .conversation
        .with_expression(|expression| qualify_claims(expression.claims(), envelope, limits))
        .map_err(ConversationQualificationError::Expression)
}
