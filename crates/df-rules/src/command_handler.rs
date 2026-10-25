//! One compiled handler contract and source-pinned invocation boundary.
use crate::dispatch::{DispatchError, DispatchRegistry};
use crate::explanation::RuleExplanation;
use df_model::checkpoint::{
    ActualDraw, Checkpoint, CheckpointError, CheckpointPins, FactValue, GameCommand, GameInput,
    PendingInput, RuleReference,
};
use df_types::{OperationId, RevisionError, RevisionLabel};
use std::collections::{BTreeMap, BTreeSet};

/// Canonical command and actual outcomes supplied by the trusted session owner.
/// Clients request a roll; they cannot supply these outcomes. Borrowing this input neither
/// consumes durable dice nor grants source, membership, or commit authority.
#[derive(Clone, Copy)]
pub struct RulesCommandInput<'a> {
    pub command: &'a GameInput,
    pub supplied_draws: &'a [ActualDraw],
}

/// Compiled source-qualified mechanics supplied by the rules owner.
///
/// Staging borrows canonical input/state and returns a detached canonical checkpoint. It must
/// use supplied time/dice/policy only, perform no I/O and grant no commit authority. Unsupported
/// mechanics return the implementation's typed rejection. Pins alone never qualify a source.
pub trait RulesCommandHandler {
    type Rejection;

    fn pins(&self) -> &CheckpointPins;
    /// A clause fixed by the compiled wrapper, when the handler has one. The registry refuses
    /// staging if its registration names a different clause.
    fn bound_source(&self) -> Option<&RuleReference> {
        None
    }
    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        checkpoint: &Checkpoint,
    ) -> Result<Checkpoint, Self::Rejection>;
}

/// A missing source binding and a rejected mechanic remain distinct typed refusals.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvocationError<Rejection> {
    Dispatch(DispatchError),
    UnsupportedInput,
    InputBasisMismatch,
    AlreadyAccepted,
    CurrentPinsMismatch,
    CurrentCheckpoint(CheckpointError),
    HandlerSourceMismatch,
    HandlerRulesMismatch,
    HandlerContentMismatch,
    HandlerBuildMismatch,
    Revision(RevisionError),
    Handler(Rejection),
    CandidateBasisMismatch,
    CandidateRulesMismatch,
    CandidateContentMismatch,
    CandidateBuildMismatch,
    CandidateCheckpoint(CheckpointError),
    MissingAcceptedDecision,
    InvalidDrawInput,
    DrawOperationMismatch,
    DrawSourceMismatch,
    DrawOrderMismatch,
    DrawAlreadyConsumed,
    RollInputMismatch,
    CandidateDrawMismatch,
    CandidateDrawAccountingMismatch,
    CandidateDecisionHistoryMismatch,
    CandidateIntentBindingMismatch,
    CandidateFactAccountingMismatch,
    Capacity,
}

/// Invokes one compiled handler and verifies its canonical output binding without applying it.
/// Structural command admission, source selection and dependency legality belong to their
/// respective callers. Reuse this same boundary after registry selection for offer and submit.
/// One command appends exactly its requested decision after the unchanged accepted history.
/// Retained intents preserve their work bindings; only their status may change.
/// Appended facts exactly match that decision in order, operation and revision.
pub fn stage_handler<Handler: RulesCommandHandler>(
    handler: &Handler,
    expected_pins: &CheckpointPins,
    input: RulesCommandInput<'_>,
    current: &Checkpoint,
    maximum_candidate_bytes: usize,
) -> Result<Checkpoint, InvocationError<Handler::Rejection>> {
    if current.pins() != expected_pins {
        return Err(InvocationError::CurrentPinsMismatch);
    }
    current
        .validate_resume(current.basis(), expected_pins)
        .map_err(InvocationError::CurrentCheckpoint)?;
    let (input_basis, operation) = match input.command {
        GameInput::Game(command) => (command.basis, command.operation),
        GameInput::Host(command) => (command.basis, command.operation),
        _ => return Err(InvocationError::UnsupportedInput),
    };
    let current_basis = current.basis();
    if input_basis.session != current_basis.session
        || input_basis.run != current_basis.run
        || input_basis.revision.epoch() != current_basis.revision.epoch()
        || input_basis.revision > current_basis.revision
    {
        return Err(InvocationError::InputBasisMismatch);
    }
    if current
        .state()
        .decisions
        .iter()
        .any(|decision| decision.operation == operation)
    {
        return Err(InvocationError::AlreadyAccepted);
    }
    if maximum_candidate_bytes == 0 {
        return Err(InvocationError::Capacity);
    }
    validate_draw_input(input, current, operation, maximum_candidate_bytes)?;
    let handler_pins = handler.pins();
    if handler_pins.rules != expected_pins.rules {
        return Err(InvocationError::HandlerRulesMismatch);
    }
    if handler_pins.content != expected_pins.content {
        return Err(InvocationError::HandlerContentMismatch);
    }
    if handler_pins.build != expected_pins.build {
        return Err(InvocationError::HandlerBuildMismatch);
    }
    let mut next_basis = current.basis();
    next_basis.revision = next_basis
        .revision
        .next_sequence()
        .map_err(InvocationError::Revision)?;
    let candidate = handler
        .stage(input, current)
        .map_err(InvocationError::Handler)?;
    if candidate.basis() != next_basis {
        return Err(InvocationError::CandidateBasisMismatch);
    }
    if candidate.pins().rules != expected_pins.rules {
        return Err(InvocationError::CandidateRulesMismatch);
    }
    if candidate.pins().content != expected_pins.content {
        return Err(InvocationError::CandidateContentMismatch);
    }
    if candidate.pins().build != expected_pins.build {
        return Err(InvocationError::CandidateBuildMismatch);
    }
    candidate
        .validate_resume(next_basis, expected_pins)
        .map_err(InvocationError::CandidateCheckpoint)?;
    if !candidate
        .state()
        .decisions
        .iter()
        .any(|decision| decision.operation == operation && decision.revision == next_basis.revision)
    {
        return Err(InvocationError::MissingAcceptedDecision);
    }
    if candidate
        .retained_bytes()
        .filter(|retained_bytes| *retained_bytes <= maximum_candidate_bytes)
        .is_none()
    {
        return Err(InvocationError::Capacity);
    }
    validate_draw_accounting(input.supplied_draws, current, &candidate, operation)?;
    // Canonical checkpoints retain past operations, but one staged command appends only its
    // requested decision. Existing matching-decision and prefix guards bind this sole append.
    if candidate
        .state()
        .decisions
        .len()
        .checked_sub(current.state().decisions.len())
        != Some(1)
    {
        return Err(InvocationError::CandidateDecisionHistoryMismatch);
    }
    validate_retained_intents(current, &candidate)?;
    validate_appended_facts(current, &candidate)?;
    Ok(candidate)
}

fn validate_appended_facts<Rejection>(
    current: &Checkpoint,
    candidate: &Checkpoint,
) -> Result<(), InvocationError<Rejection>> {
    let decision = candidate
        .state()
        .decisions
        .last()
        .ok_or(InvocationError::MissingAcceptedDecision)?;
    // Prior guards preserve the historical prefix and bind the sole appended decision.
    // Compare its complete ordered fact projection, regardless of the canonical value type.
    let appended = candidate
        .state()
        .facts
        .get(current.state().facts.len()..)
        .ok_or(InvocationError::CandidateFactAccountingMismatch)?;
    if appended.len() != decision.facts.len()
        || !appended.iter().zip(&decision.facts).all(|(fact, id)| {
            fact.id == *id
                && fact.operation == decision.operation
                && fact.revision == decision.revision
        })
    {
        return Err(InvocationError::CandidateFactAccountingMismatch);
    }
    Ok(())
}

fn validate_retained_intents<Rejection>(
    current: &Checkpoint,
    candidate: &Checkpoint,
) -> Result<(), InvocationError<Rejection>> {
    let prior = &current.state().intents;
    let staged = &candidate.state().intents;
    if prior.is_empty() {
        return Ok(());
    }
    if prior.len() > staged.len() {
        return Err(InvocationError::CandidateIntentBindingMismatch);
    }
    // Candidate capacity has already bounded the index and the prior count. Borrow bindings
    // rather than cloning definitions; retained intent order carries no identity semantics.
    let by_id: BTreeMap<_, _> = staged.iter().map(|intent| (intent.id, intent)).collect();
    for intent in prior {
        let Some(retained) = by_id.get(&intent.id) else {
            return Err(InvocationError::CandidateIntentBindingMismatch);
        };
        if retained.basis != intent.basis
            || retained.operation != intent.operation
            || retained.slot != intent.slot
            || retained.kind != intent.kind
            || retained.job != intent.job
            || retained.timer != intent.timer
            || retained.generation != intent.generation
            || retained.definition != intent.definition
        {
            return Err(InvocationError::CandidateIntentBindingMismatch);
        }
    }
    Ok(())
}

fn validate_draw_input<Rejection>(
    input: RulesCommandInput<'_>,
    current: &Checkpoint,
    operation: OperationId,
    maximum_retained_bytes: usize,
) -> Result<(), InvocationError<Rejection>> {
    // Bound the count before traversing supplied records or comparing retained history.
    let mut retained_bytes = input
        .supplied_draws
        .len()
        .checked_add(current.state().draws.len())
        .and_then(|count| count.checked_mul(std::mem::size_of::<ActualDraw>()))
        .filter(|retained_bytes| *retained_bytes <= maximum_retained_bytes)
        .ok_or(InvocationError::Capacity)?;
    for draw in input.supplied_draws {
        for label in [
            &draw.source.catalog,
            &draw.source.source,
            &draw.source.entry,
            &draw.source.clause,
        ] {
            retained_bytes = retained_bytes
                .checked_add(label.retained_heap_bytes())
                .filter(|retained_bytes| *retained_bytes <= maximum_retained_bytes)
                .ok_or(InvocationError::Capacity)?;
        }
    }
    let mut next_ordinal = 0;
    for draw in current
        .state()
        .draws
        .iter()
        .filter(|draw| draw.operation == operation)
    {
        next_ordinal = draw
            .ordinal
            .checked_add(1)
            .ok_or(InvocationError::DrawOrderMismatch)?;
    }
    for draw in input.supplied_draws {
        if draw.operation != operation {
            return Err(InvocationError::DrawOperationMismatch);
        }
        if draw.source.catalog != current.pins().rules.catalog {
            return Err(InvocationError::DrawSourceMismatch);
        }
        if draw.sides == 0 || draw.value == 0 || draw.value > draw.sides {
            return Err(InvocationError::InvalidDrawInput);
        }
        if draw.ordinal < next_ordinal {
            return Err(InvocationError::DrawAlreadyConsumed);
        }
        if draw.ordinal != next_ordinal {
            return Err(InvocationError::DrawOrderMismatch);
        }
        next_ordinal = next_ordinal
            .checked_add(1)
            .ok_or(InvocationError::DrawOrderMismatch)?;
    }
    if let GameInput::Game(command) = input.command
        && let GameCommand::SubmitRoll { resolution, window } = &command.command
    {
        if current
            .state()
            .pending
            .len()
            .checked_mul(std::mem::size_of::<df_model::checkpoint::PendingResolution>())
            .filter(|pending_bytes| *pending_bytes <= maximum_retained_bytes)
            .is_none()
        {
            return Err(InvocationError::Capacity);
        }
        let pending = current
            .state()
            .pending
            .iter()
            .find(|pending| pending.id == *resolution && pending.window.id == *window)
            .ok_or(InvocationError::RollInputMismatch)?;
        let PendingInput::Roll {
            participant,
            sides,
            source,
        } = &pending.next
        else {
            return Err(InvocationError::RollInputMismatch);
        };
        if *participant != command.member || sides.len() != input.supplied_draws.len() {
            return Err(InvocationError::RollInputMismatch);
        }
        for (draw, sides) in input.supplied_draws.iter().zip(sides) {
            if draw.resolution != *resolution
                || draw.window != *window
                || draw.source != *source
                || draw.sides != *sides
            {
                return Err(InvocationError::RollInputMismatch);
            }
        }
    }
    Ok(())
}

fn validate_draw_accounting<Rejection>(
    supplied: &[ActualDraw],
    current: &Checkpoint,
    candidate: &Checkpoint,
    operation: OperationId,
) -> Result<(), InvocationError<Rejection>> {
    // A mechanics handler cannot compact accepted history or erase retry provenance.
    if !candidate
        .state()
        .decisions
        .starts_with(&current.state().decisions)
    {
        return Err(InvocationError::CandidateDecisionHistoryMismatch);
    }
    let mut candidate_draws = candidate.state().draws.iter();
    for prior in &current.state().draws {
        if candidate_draws.next() != Some(prior) {
            return Err(InvocationError::CandidateDrawMismatch);
        }
    }
    if !candidate_draws.eq(supplied.iter()) {
        return Err(InvocationError::CandidateDrawMismatch);
    }
    let decision = candidate
        .state()
        .decisions
        .iter()
        .find(|decision| decision.operation == operation)
        .ok_or(InvocationError::MissingAcceptedDecision)?;
    let retained_ordinals = current
        .state()
        .draws
        .iter()
        .filter(|draw| draw.operation == operation)
        .map(|draw| draw.ordinal);
    if !decision
        .draws
        .iter()
        .copied()
        .eq(retained_ordinals.chain(supplied.iter().map(|draw| draw.ordinal)))
    {
        return Err(InvocationError::CandidateDrawAccountingMismatch);
    }
    // Canonical facts retain chronological order. The bounded set checks their decision
    // ownership without scanning the full decision inventory separately for every draw.
    let decision_facts: BTreeSet<_> = decision.facts.iter().copied().collect();
    if !candidate.state().facts.starts_with(&current.state().facts) {
        return Err(InvocationError::CandidateDrawAccountingMismatch);
    }
    let mut expected = supplied.iter();
    for fact in candidate
        .state()
        .facts
        .iter()
        .skip(current.state().facts.len())
    {
        if let FactValue::DrawAccepted {
            operation: draw_operation,
            ordinal,
        } = fact.value
        {
            let draw = expected
                .next()
                .ok_or(InvocationError::CandidateDrawAccountingMismatch)?;
            if draw_operation != operation
                || draw.ordinal != ordinal
                || fact.operation != operation
                || fact.revision != decision.revision
                || !decision_facts.contains(&fact.id)
            {
                return Err(InvocationError::CandidateDrawAccountingMismatch);
            }
        }
    }
    if expected.next().is_some() {
        return Err(InvocationError::CandidateDrawAccountingMismatch);
    }
    Ok(())
}

impl<'a, Handler: RulesCommandHandler> DispatchRegistry<'a, Handler> {
    /// Selects the exact pinned compiled handler, then runs the shared staging guard.
    /// Selection failure never invokes any registered handler. The source owner supplies the
    /// command-to-selector/clause mapping; imported catalog bytes cannot supply executable code.
    pub fn stage(
        &self,
        expected_pins: &CheckpointPins,
        selector: &RevisionLabel,
        source: &RuleReference,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
        maximum_candidate_bytes: usize,
    ) -> Result<Checkpoint, InvocationError<Handler::Rejection>> {
        self.stage_with_provenance(
            expected_pins,
            selector,
            source,
            input,
            current,
            maximum_candidate_bytes,
        )
        .map(|(candidate, _)| candidate)
    }

    /// Stages through the canonical guard and returns the actual selected source explanation.
    /// The checkpoint remains an uncommitted candidate. Refused mechanics or invalid candidate
    /// bindings return the existing typed error without a successful decision explanation.
    /// Build labels/source bytes remain supplied facts, not executed-check or rights evidence.
    pub fn stage_with_provenance(
        &self,
        expected_pins: &CheckpointPins,
        selector: &RevisionLabel,
        source: &RuleReference,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
        maximum_candidate_bytes: usize,
    ) -> Result<(Checkpoint, RuleExplanation<'a>), InvocationError<Handler::Rejection>> {
        let (handler, explanation) = self
            .select_with_provenance(expected_pins, selector, source)
            .map_err(InvocationError::Dispatch)?;
        if handler.bound_source().is_some_and(|bound| bound != source) {
            return Err(InvocationError::HandlerSourceMismatch);
        }
        let candidate = stage_handler(
            handler,
            expected_pins,
            input,
            current,
            maximum_candidate_bytes,
        )?;
        Ok((candidate, explanation))
    }
}
