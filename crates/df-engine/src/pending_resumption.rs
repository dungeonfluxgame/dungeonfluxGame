//! Pending-response adapter over the canonical rules source resolver and registered engine entry.
//! Operation lookup and durable acceptance remain the serialized session owner's responsibility.
use crate::command_entry::{CommandEntryContext, CommandRejection, decide_registered_command};
use df_model::checkpoint::Checkpoint;
use df_rules::current_responses::{ResponseError, current_pending_source};
use df_rules::preconditions::{PreconditionedCommandHandler, PreconditionedRejection};
use df_rules::{DispatchRegistry, RulesCommandHandler, RulesCommandInput};
use df_types::RevisionLabel;

/// Native owner evidence for this attempt, never a client DTO or a persisted pending schema.
/// Session binds both values to the admitted pending lifetime and current owner generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResumeFence {
    pub admitted_generation: u64,
    pub current_generation: u64,
    pub cancel_before_admission: bool,
}

/// Explicit adapter traversal bound. Shared command/candidate/dependency bounds remain mandatory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResumeLimits {
    pub maximum_checkpoint_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResumeError<R> {
    StaleGeneration,
    CancelledBeforeAdmission,
    Capacity,
    Response(ResponseError),
    HandlerSourceMismatch,
    Command(CommandRejection<PreconditionedRejection<R>>),
}

/// Resolve one current pending response through the exact shared compiled handler boundary.
///
/// The session must perform scoped durable operation lookup first. Committed/Unknown/in-progress
/// outcomes never enter this function; receipt cancellation cannot undo committed pending work.
/// Native cancellation before admission discards only this attempt. An offered decline remains
/// an opaque source option for the real handler, with no inferred rollback or timeout default.
///
/// Source resolution comes from Rules I03; canonical command admission comes from Engine I01
/// and Model; relevant current resources/pending/time/source dependencies come from the exact
/// registered Rules I04 wrapper. This module cannot replace the wrapper's immutable dependency
/// list. Source review must establish that list's completeness. The source owner supplies the
/// selector-to-continuation mapping, never client labels or imported executable content.
///
/// The exact borrowed native draw slice is forwarded unchanged through the canonical envelope.
/// Command source and admission checks inspect only `input.command`; they never infer outcomes.
/// A returned complete canonical checkpoint is staged only. Rules handlers own real mechanics,
/// actual supplied draws and cost timing. Final combined validation and atomic commit/publication
/// remain required. No handler is created here; unsupported or missing handlers return typed
/// refusals. Persisted pending generation, cancel/ruling semantics and real source qualification
/// remain separately tracked contracts; a matching transient fence supplies no such authority.
pub fn stage_pending_response<H: RulesCommandHandler>(
    input: RulesCommandInput<'_>,
    current: &Checkpoint,
    context: CommandEntryContext<'_>,
    fence: ResumeFence,
    limits: ResumeLimits,
    registry: &DispatchRegistry<'_, PreconditionedCommandHandler<'_, H>>,
    selector: &RevisionLabel,
) -> Result<Checkpoint, ResumeError<H::Rejection>> {
    if limits.maximum_checkpoint_bytes == 0
        || context.limits.maximum_staged_bytes == 0
        || current
            .retained_bytes()
            .is_none_or(|bytes| bytes > limits.maximum_checkpoint_bytes)
        || input
            .command
            .retained_bytes()
            .is_none_or(|bytes| bytes > context.limits.command.maximum_retained_bytes)
    {
        return Err(ResumeError::Capacity);
    }
    if fence.current_generation == 0 || fence.admitted_generation != fence.current_generation {
        return Err(ResumeError::StaleGeneration);
    }
    if fence.cancel_before_admission {
        return Err(ResumeError::CancelledBeforeAdmission);
    }
    let pending_source =
        current_pending_source(input.command, current).map_err(ResumeError::Response)?;
    let handler = registry
        .select(context.admitted_pins, selector, pending_source)
        .map_err(|error| {
            ResumeError::Command(CommandRejection::Invocation(
                df_rules::InvocationError::Dispatch(error),
            ))
        })?;
    if handler.source() != pending_source {
        return Err(ResumeError::HandlerSourceMismatch);
    }
    decide_registered_command(input, current, context, registry, selector, pending_source)
        .map_err(ResumeError::Command)
}
