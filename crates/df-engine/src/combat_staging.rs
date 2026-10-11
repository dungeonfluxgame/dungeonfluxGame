//! Exact registered tactical inputs revalidated through the canonical engine command boundary.
use crate::command_entry::{CommandEntryContext, CommandRejection, decide_registered_command};
use crate::pending_resumption::{ResumeError, ResumeFence, ResumeLimits, stage_pending_response};
use df_combat::candidates::AdmittedCandidates;
use df_combat::registered_candidates::RegisteredActionCandidates;
use df_model::checkpoint::{
    Checkpoint, EntityId, GameCommand, GameInput, OfferedResponse, PendingInput, RuleReference,
};
use df_rules::preconditions::{PreconditionedCommandHandler, PreconditionedRejection};
use df_rules::{DispatchRegistry, InvocationError, RulesCommandHandler, RulesCommandInput};
use df_types::{MemberId, OperationId, RevisionLabel};

/// Current native admission, independent of tactical suggestions and client labels.
/// The receipt borrows exact canonical offers, not detached or reconstructed responses.
pub struct TacticalResponseAdmission<'a> {
    pub candidates: &'a AdmittedCandidates<'a, OfferedResponse>,
    pub member: MemberId,
    pub operation: OperationId,
    pub fence: ResumeFence,
    pub limits: ResumeLimits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TacticalResponseError<R> {
    Capacity,
    UnsupportedCommand,
    MemberMismatch,
    OperationMismatch,
    StaleBasis,
    StalePins,
    UnadmittedResponse,
    Resume(ResumeError<R>),
}

/// Stage an exact selected registered Choice/Reaction through the existing legal command path.
///
/// The caller composes Combat's registered enumeration and tactical selection, then supplies the
/// selected input and its current receipt. A preview checkpoint is never applied here. Exact
/// canonical response identity is rechecked; equal detached payloads are insufficient. The
/// current source registry, full source-owned dependencies and native generation/cancellation
/// fence are checked again by the pending engine entry before the handler runs.
///
/// Native member/operation admission must come from the authenticated session owner independently
/// of a suggestion. These fields and canonical attribution confer no autonomous NPC control.
/// General actions, rolls, host inputs and native completions remain outside this bounded entry.
/// Source rights, actor control and approved utility policy remain their existing owners' duties;
/// this adapter neither chooses a policy nor grants authority from a receipt.
///
/// The original native draw envelope is forwarded unchanged. A successful checkpoint is detached;
/// scoped operation lookup must precede this call, and durable acceptance/publication remain with
/// Session. Rejected/stale/unknown work cannot commit a preview or generate replacement outcomes.
pub fn stage_tactical_response<H: RulesCommandHandler>(
    input: RulesCommandInput<'_>,
    current: &Checkpoint,
    context: CommandEntryContext<'_>,
    admission: TacticalResponseAdmission<'_>,
    registry: &DispatchRegistry<'_, PreconditionedCommandHandler<'_, H>>,
    selector: &RevisionLabel,
) -> Result<Checkpoint, TacticalResponseError<H::Rejection>> {
    if admission.limits.maximum_checkpoint_bytes == 0
        || current
            .retained_bytes()
            .is_none_or(|bytes| bytes > admission.limits.maximum_checkpoint_bytes)
        || admission.candidates.offers().len() > context.limits.command.maximum_records
        || input
            .command
            .retained_bytes()
            .is_none_or(|bytes| bytes > context.limits.command.maximum_retained_bytes)
    {
        return Err(TacticalResponseError::Capacity);
    }
    let GameInput::Game(command) = input.command else {
        return Err(TacticalResponseError::UnsupportedCommand);
    };
    if command.member != admission.member {
        return Err(TacticalResponseError::MemberMismatch);
    }
    if command.operation != admission.operation {
        return Err(TacticalResponseError::OperationMismatch);
    }
    if admission.candidates.basis() != &current.basis()
        || context.current_basis != current.basis()
        || command.basis != current.basis()
        || command.observed_revision != current.basis().revision
    {
        return Err(TacticalResponseError::StaleBasis);
    }
    if admission.candidates.pins() != current.pins() || context.admitted_pins != current.pins() {
        return Err(TacticalResponseError::StalePins);
    }
    let (resolution, window, offer, option, reaction) = match &command.command {
        GameCommand::SelectChoice {
            resolution,
            window,
            offer,
            option,
        } => (resolution, window, offer, option, false),
        GameCommand::SelectReaction {
            resolution,
            window,
            offer,
            option,
        } => (resolution, window, offer, option, true),
        _ => return Err(TacticalResponseError::UnsupportedCommand),
    };
    let pending = current
        .state()
        .pending
        .iter()
        .find(|pending| pending.id == *resolution && pending.window.id == *window);
    let responses = match pending.map(|pending| &pending.next) {
        Some(PendingInput::Choice { remaining }) if !reaction => remaining,
        Some(PendingInput::Reaction { remaining }) if reaction => remaining,
        _ => return Err(TacticalResponseError::UnadmittedResponse),
    };
    let exact = admission.candidates.offers().iter().any(|candidate| {
        responses
            .iter()
            .any(|response| std::ptr::eq(*candidate, response))
            && candidate.participant == admission.member
            && &candidate.offer == offer
            && candidate.options.contains(option)
    });
    if !exact {
        return Err(TacticalResponseError::UnadmittedResponse);
    }
    stage_pending_response(
        input,
        current,
        context,
        admission.fence,
        admission.limits,
        registry,
        selector,
    )
    .map_err(TacticalResponseError::Resume)
}

/// Independent native admission of a selected source-bound action inventory.
/// Possession of the receipt and these values is not session authorization or NPC control.
pub struct TacticalActionAdmission<'a> {
    pub candidates: &'a RegisteredActionCandidates<'a>,
    pub actor: EntityId,
    pub member: MemberId,
    pub operation: OperationId,
    pub source: &'a RuleReference,
    pub fence: ResumeFence,
    pub limits: ResumeLimits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TacticalActionError<R> {
    Capacity,
    UnsupportedCommand,
    MemberMismatch,
    OperationMismatch,
    ActorMismatch,
    SourceMismatch,
    SelectorMismatch,
    StaleBasis,
    StalePins,
    UnadmittedAction,
    StaleGeneration,
    CancelledBeforeAdmission,
    Command(CommandRejection<PreconditionedRejection<R>>),
}

/// Revalidate and stage an exact registered ProposeAction using the existing engine command path.
///
/// The receipt retains enumeration source/selector/native identity as well as exact input borrows.
/// A detached equal input or another currently registered handler cannot replace that binding.
/// Current source inventory, full source-owned dependencies, model admission and shared candidate
/// accounting are checked again. The original native draw slice passes unchanged to Rules.
/// No preview checkpoint is applied and no outcome is generated or inferred by this adapter.
/// Source-producing legal enemy attacks, perception, actor control and utility policy remain
/// their owners' gates. Session must perform scoped operation lookup before entry and remains
/// the sole durable acceptance/publication owner; stale/Unknown work cannot use a preview.
pub fn stage_tactical_action<H: RulesCommandHandler>(
    input: RulesCommandInput<'_>,
    current: &Checkpoint,
    context: CommandEntryContext<'_>,
    admission: TacticalActionAdmission<'_>,
    registry: &DispatchRegistry<'_, PreconditionedCommandHandler<'_, H>>,
    selector: &RevisionLabel,
) -> Result<Checkpoint, TacticalActionError<H::Rejection>> {
    let candidates = admission.candidates;
    if admission.limits.maximum_checkpoint_bytes == 0
        || context.limits.maximum_staged_bytes == 0
        || current
            .retained_bytes()
            .is_none_or(|bytes| bytes > admission.limits.maximum_checkpoint_bytes)
        || candidates.admitted().offers().len() > context.limits.command.maximum_records
        || input
            .command
            .retained_bytes()
            .is_none_or(|bytes| bytes > context.limits.command.maximum_retained_bytes)
    {
        return Err(TacticalActionError::Capacity);
    }
    let GameInput::Game(command) = input.command else {
        return Err(TacticalActionError::UnsupportedCommand);
    };
    let GameCommand::ProposeAction { actor, .. } = &command.command else {
        return Err(TacticalActionError::UnsupportedCommand);
    };
    if command.member != admission.member || candidates.member() != admission.member {
        return Err(TacticalActionError::MemberMismatch);
    }
    if command.operation != admission.operation || candidates.operation() != admission.operation {
        return Err(TacticalActionError::OperationMismatch);
    }
    if *actor != admission.actor || candidates.actor() != admission.actor {
        return Err(TacticalActionError::ActorMismatch);
    }
    if candidates.source() != admission.source
        || admission.source.catalog != context.admitted_pins.rules.catalog
        || !context.inventory.rules.contains(admission.source)
    {
        return Err(TacticalActionError::SourceMismatch);
    }
    if candidates.selector() != selector {
        return Err(TacticalActionError::SelectorMismatch);
    }
    if candidates.admitted().basis() != &current.basis()
        || context.current_basis != current.basis()
        || command.basis != current.basis()
        || command.observed_revision != current.basis().revision
    {
        return Err(TacticalActionError::StaleBasis);
    }
    if candidates.admitted().pins() != current.pins() || context.admitted_pins != current.pins() {
        return Err(TacticalActionError::StalePins);
    }
    if !candidates
        .admitted()
        .offers()
        .iter()
        .any(|offer| std::ptr::eq(*offer, input.command))
    {
        return Err(TacticalActionError::UnadmittedAction);
    }
    if admission.fence.current_generation == 0
        || admission.fence.admitted_generation != admission.fence.current_generation
    {
        return Err(TacticalActionError::StaleGeneration);
    }
    if admission.fence.cancel_before_admission {
        return Err(TacticalActionError::CancelledBeforeAdmission);
    }
    let handler = registry
        .select(context.admitted_pins, selector, admission.source)
        .map_err(|error| {
            TacticalActionError::Command(CommandRejection::Invocation(InvocationError::Dispatch(
                error,
            )))
        })?;
    if handler.source() != admission.source {
        return Err(TacticalActionError::SourceMismatch);
    }
    decide_registered_command(
        input,
        current,
        context,
        registry,
        selector,
        admission.source,
    )
    .map_err(TacticalActionError::Command)
}
