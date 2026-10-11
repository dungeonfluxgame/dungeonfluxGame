//! Admitted semantic pending responses enter the existing registered engine command path.
use crate::command_entry::CommandEntryContext;
use crate::pending_resumption::{ResumeError, ResumeFence, ResumeLimits, stage_pending_response};
use df_intent::candidate::{CandidateError, CandidateOwner, validate_semantic_candidate};
use df_intent::dialogue::{DialogueError, DialogueLimits, classify_dialogue, confirm_final_input};
use df_model::checkpoint::{Checkpoint, GameInput, ReferenceInventory};
use df_model::intent::{DiscourseContext, InputFinality, IntentDisposition, RawInputRef};
use df_rules::preconditions::PreconditionedCommandHandler;
use df_rules::{DispatchRegistry, RulesCommandHandler, RulesCommandInput};
use df_types::RevisionLabel;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticResponseError<R> {
    Candidate(CandidateError),
    Resume(ResumeError<R>),
}

/// Native attempt admission, independent of classifier output and client observations.
/// These existing owner/fence/bound contracts are not a persisted intent or plan schema.
pub struct SemanticResponseAdmission<'a> {
    pub owner: CandidateOwner<'a>,
    pub fence: ResumeFence,
    pub limits: ResumeLimits,
}

/// Admits a typed model suggestion, then stages it through the real pending command entry.
///
/// The native session supplies the authenticated member, admitted operation and current owner
/// basis/pins independently of the suggestion. Only currently offered choice/reaction responses
/// and roll requests are supported. Speech, general actions, host commands and native completions
/// cannot become commands through this entry; actor control or ambiguous input confirmation is
/// never inferred. This operation does not classify text or implement compound action plans.
///
/// Admission delegates to Intent's canonical validator; pending source selection, generation/
/// cancellation checks and exact registered handler dispatch delegate to Engine's existing entry.
/// The registry retains its source-reviewed precondition dependencies. The supplied native draw
/// slice reaches that handler unchanged; this entry cannot invent outcomes or consume durable dice.
/// All refusals return no checkpoint and leave the current snapshot unchanged.
///
/// The session must lookup the exact scoped operation before calling this function. Already
/// committed, conflicting, in-progress or uncertain operations must retain their session outcome.
/// A successful checkpoint is detached: owner fencing, atomic commit and publication remain with
/// the durable session. No source mapping, authentication, source rights or provider qualification
/// is granted by passing these borrowed contexts.
pub fn stage_semantic_response<H: RulesCommandHandler>(
    input: RulesCommandInput<'_>,
    current: &Checkpoint,
    context: CommandEntryContext<'_>,
    admission: SemanticResponseAdmission<'_>,
    registry: &DispatchRegistry<'_, PreconditionedCommandHandler<'_, H>>,
    selector: &RevisionLabel,
) -> Result<Checkpoint, SemanticResponseError<H::Rejection>> {
    validate_semantic_candidate(
        input.command,
        current,
        admission.owner,
        ReferenceInventory {
            rules: context.inventory.rules,
            content: context.inventory.content,
            resources: context.inventory.resources,
            assets: context.inventory.assets,
        },
        context.limits.command,
    )
    .map_err(SemanticResponseError::Candidate)?;
    stage_pending_response(
        input,
        current,
        context,
        admission.fence,
        admission.limits,
        registry,
        selector,
    )
    .map_err(SemanticResponseError::Resume)
}

/// A native dialogue result never treats an inert input as an accepted next checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DialogueResponseOutcome {
    NoMechanicalProposal(IntentDisposition),
    Staged(Box<Checkpoint>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DialogueResponseError<R> {
    Dialogue(DialogueError),
    ConfirmationMismatch,
    InputBindingMismatch,
    ConfirmationOperationReuse,
    Semantic(SemanticResponseError<R>),
}

/// Borrowed native capture and separately admitted confirmation; neither is a capability.
/// The native caller owns the retained ticket, exact typed selection and authentication.
pub struct DialogueResponseAdmission<'a> {
    pub captured: &'a RawInputRef,
    pub confirmed: Option<&'a RawInputRef>,
    pub text: &'a str,
    pub discourse: DiscourseContext,
    pub limits: DialogueLimits,
}

/// Keep dialogue inert until a distinct, current, explicit final typed response is admitted.
///
/// Classification never promotes wording, a model disposition or tentative speech to mechanics.
/// Non-action discourse stays inert even if a caller attaches a confirmation. Ambiguous final
/// input requires the native owner's separately confirmed exact capture and a distinct command
/// operation. The caller must first authenticate and lookup that operation, retain and check its
/// ticket and exact current typed offer. These borrowed values do not prove those permissions.
///
/// Raw text is borrowed solely for bounded classification and is never retained in the result.
/// Only a separately confirmed final response reaches the existing source-registered semantic
/// path; its supplied draw slice, generation/cancellation and staging limits remain unchanged.
/// The native session still owns durable commit, effects and publication.
pub fn stage_dialogue_response<H: RulesCommandHandler>(
    input: RulesCommandInput<'_>,
    current: &Checkpoint,
    context: CommandEntryContext<'_>,
    dialogue: DialogueResponseAdmission<'_>,
    admission: SemanticResponseAdmission<'_>,
    registry: &DispatchRegistry<'_, PreconditionedCommandHandler<'_, H>>,
    selector: &RevisionLabel,
) -> Result<DialogueResponseOutcome, DialogueResponseError<H::Rejection>> {
    let disposition = classify_dialogue(
        dialogue.text,
        dialogue.discourse,
        dialogue.captured,
        current,
        admission.owner.member,
        dialogue.limits,
    )
    .map_err(DialogueResponseError::Dialogue)?;
    if dialogue.captured.finality == InputFinality::Partial
        || disposition != IntentDisposition::Clarify
    {
        return Ok(DialogueResponseOutcome::NoMechanicalProposal(disposition));
    }
    let Some(confirmed) = dialogue.confirmed else {
        return Ok(DialogueResponseOutcome::NoMechanicalProposal(disposition));
    };
    if confirmed != dialogue.captured {
        return Err(DialogueResponseError::ConfirmationMismatch);
    }
    confirm_final_input(confirmed, current, admission.owner.member)
        .map_err(DialogueResponseError::Dialogue)?;
    if confirmed.member != admission.owner.member
        || confirmed.basis != admission.owner.basis
        || &confirmed.pins != admission.owner.pins
    {
        return Err(DialogueResponseError::InputBindingMismatch);
    }
    if confirmed.input == admission.owner.operation {
        return Err(DialogueResponseError::ConfirmationOperationReuse);
    }
    if let GameInput::Game(command) = input.command
        && (command.basis != confirmed.basis
            || command.observed_revision != confirmed.basis.revision)
    {
        return Err(DialogueResponseError::InputBindingMismatch);
    }
    stage_semantic_response(input, current, context, admission, registry, selector)
        .map(|candidate| DialogueResponseOutcome::Staged(Box::new(candidate)))
        .map_err(DialogueResponseError::Semantic)
}
