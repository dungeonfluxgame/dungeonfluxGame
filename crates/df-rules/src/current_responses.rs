//! Canonical current-response agreement through registered source and dependency guards.
use crate::command_handler::{InvocationError, RulesCommandHandler, RulesCommandInput};
use crate::dispatch::DispatchRegistry;
use crate::preconditions::{
    CurrentRuleContext, PreconditionedCommandHandler, PreconditionedRejection,
};
use df_model::checkpoint::{
    Basis, Checkpoint, CheckpointError, CheckpointPins, CommandInput, GameCommand, GameInput,
    HostCommand, OfferedResponse, PendingInput, PendingResolution, ReferenceInventory,
    ResolutionId, RuleReference, WindowId,
};
use df_model::commands::{CommandError, CommandLimits, validate_client_command};
use df_types::RevisionLabel;

/// Why a current Choice/Reaction query or submission was refused before any handler ran.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseError {
    Snapshot(CheckpointError),
    Admission(CommandError),
    UnsupportedCommand,
}

/// The refusal for a query whose records, options or allocation pass the caller's limits.
const CAPACITY_REFUSAL: ResponseError = ResponseError::Admission(CommandError::Capacity);

/// Borrows only this participant's current Choice or Reaction responses.
///
/// The command supplies the response kind, resolution and window; its offer/option labels are
/// ignored for enumeration. Every returned option passes the same structural admission function
/// used for submission. Bounds cover response records, option count and the returned allocation.
/// This is not LegalActionSet or source-handler legality. Authentication, actor control, source
/// handler preparation/resolution, retry receipts and audience projection remain owner gates.
/// The owner must pass its admitted pins and serialize the subsequent decision against this basis.
pub fn current_responses<'a>(
    request: &CommandInput,
    checkpoint: &'a Checkpoint,
    current_basis: Basis,
    admitted_pins: &CheckpointPins,
    inventory: ReferenceInventory<'_>,
    limits: CommandLimits,
) -> Result<Vec<&'a OfferedResponse>, ResponseError> {
    current_snapshot(checkpoint, current_basis, admitted_pins)?;
    let (resolution, window) = match &request.command {
        GameCommand::SelectChoice {
            resolution, window, ..
        }
        | GameCommand::SelectReaction {
            resolution, window, ..
        } => (*resolution, *window),
        _ => return Err(ResponseError::UnsupportedCommand),
    };
    let pending = pending_resolution(checkpoint, resolution, window)?;
    let offered = offered_responses(&request.command, pending)?;
    let participant_responses = || {
        offered
            .iter()
            .filter(|record| record.participant == request.member)
    };
    let mut records = 0usize;
    let mut options = 0usize;
    for response in participant_responses() {
        records = records.checked_add(1).ok_or(CAPACITY_REFUSAL)?;
        options = options
            .checked_add(response.options.len())
            .ok_or(CAPACITY_REFUSAL)?;
    }
    let retained_bytes = records
        .checked_mul(size_of::<&OfferedResponse>())
        .and_then(|records_bytes| records_bytes.checked_add(size_of::<Vec<&OfferedResponse>>()))
        .ok_or(CAPACITY_REFUSAL)?;
    if limits.maximum_records == 0
        || limits.maximum_text_bytes == 0
        || records > limits.maximum_records
        || options > limits.maximum_records
        || retained_bytes > limits.maximum_retained_bytes
    {
        return Err(CAPACITY_REFUSAL);
    }
    if records == 0 {
        return Err(ResponseError::Admission(CommandError::UnofferedResponse));
    }
    let mut responses = Vec::with_capacity(records);
    for response in participant_responses() {
        for option in &response.options {
            let candidate = GameInput::Game(CommandInput {
                command: with_offered_option(&request.command, &response.offer, option)?,
                ..*request
            });
            validate_current_submission(
                &candidate,
                checkpoint,
                current_basis,
                admitted_pins,
                inventory,
                limits,
            )?;
        }
        responses.push(response);
    }
    Ok(responses)
}

/// Structurally admits a current Choice/Reaction command without producing state, facts or draws.
///
/// The trusted current basis/pins identify the detached rules snapshot, independently of the
/// client observation. An older same-epoch client sequence remains admissible when its exact
/// current window/offer survives, as canonical model admission requires. Mechanical legality
/// must use prepare_current_response and its selected source handler before engine commit.
pub fn validate_current_submission(
    request: &GameInput,
    checkpoint: &Checkpoint,
    current_basis: Basis,
    admitted_pins: &CheckpointPins,
    inventory: ReferenceInventory<'_>,
    limits: CommandLimits,
) -> Result<(), ResponseError> {
    current_snapshot(checkpoint, current_basis, admitted_pins)?;
    if !matches!(
        request,
        GameInput::Game(CommandInput {
            command: GameCommand::SelectChoice { .. } | GameCommand::SelectReaction { .. },
            ..
        })
    ) {
        return Err(ResponseError::UnsupportedCommand);
    }
    validate_client_command(request, checkpoint, inventory, limits)
        .map_err(ResponseError::Admission)
}

fn current_snapshot(
    checkpoint: &Checkpoint,
    current_basis: Basis,
    admitted_pins: &CheckpointPins,
) -> Result<(), ResponseError> {
    checkpoint
        .validate_resume(current_basis, admitted_pins)
        .map(|_| ())
        .map_err(ResponseError::Snapshot)
}

/// The pending resolution a command names by resolution and window, or a stale-window refusal.
fn pending_resolution(
    checkpoint: &Checkpoint,
    resolution: ResolutionId,
    window: WindowId,
) -> Result<&PendingResolution, ResponseError> {
    checkpoint
        .state()
        .pending
        .iter()
        .find(|record| record.id == resolution && record.window.id == window)
        .ok_or(ResponseError::Admission(CommandError::StaleWindow))
}

/// The responses still offered for a Choice or Reaction command whose kind matches the pending
/// input; any other pairing is the wrong pending kind.
fn offered_responses<'a>(
    command: &GameCommand,
    pending: &'a PendingResolution,
) -> Result<&'a [OfferedResponse], ResponseError> {
    match (command, &pending.next) {
        (GameCommand::SelectChoice { .. }, PendingInput::Choice { remaining })
        | (GameCommand::SelectReaction { .. }, PendingInput::Reaction { remaining }) => {
            Ok(remaining)
        }
        _ => Err(ResponseError::Admission(CommandError::WrongPendingKind)),
    }
}

/// The same Choice or Reaction command, for the same resolution and window, naming one offer and
/// option instead of the caller's labels.
fn with_offered_option(
    command: &GameCommand,
    offer: &RevisionLabel,
    option: &RevisionLabel,
) -> Result<GameCommand, ResponseError> {
    match command {
        GameCommand::SelectChoice {
            resolution, window, ..
        } => Ok(GameCommand::SelectChoice {
            resolution: *resolution,
            window: *window,
            offer: offer.clone(),
            option: option.clone(),
        }),
        GameCommand::SelectReaction {
            resolution, window, ..
        } => Ok(GameCommand::SelectReaction {
            resolution: *resolution,
            window: *window,
            offer: offer.clone(),
            option: option.clone(),
        }),
        _ => Err(ResponseError::UnsupportedCommand),
    }
}

/// Classified refusal from the exact current registry/dependency/handler pipeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResponsePreparationError<R> {
    Response(ResponseError),
    HandlerSourceMismatch,
    Invocation(InvocationError<PreconditionedRejection<R>>),
}

/// Stages a current canonical choice, reaction, roll or host ruling through its source handler.
/// The selected wrapper binds immutable handler dependencies; callers cannot replace them after
/// source selection. The returned Checkpoint is a candidate only. Session commit authority,
/// operation receipt lookup and authentication remain outside this pure boundary. Trusted
/// supplied actual draws pass unchanged to the selected handler and its shared accounting guard;
/// this boundary never generates outcomes or supplies a default draw sequence.
pub fn prepare_current_response<H: RulesCommandHandler>(
    input: RulesCommandInput<'_>,
    context: CurrentRuleContext<'_>,
    registry: &DispatchRegistry<'_, PreconditionedCommandHandler<'_, H>>,
    selector: &RevisionLabel,
    maximum_staged_bytes: usize,
) -> Result<Checkpoint, ResponsePreparationError<H::Rejection>> {
    current_snapshot(context.checkpoint, context.basis, context.pins)
        .map_err(ResponsePreparationError::Response)?;
    validate_client_command(
        input.command,
        context.checkpoint,
        context.inventory,
        context.command_limits,
    )
    .map_err(|error| ResponsePreparationError::Response(ResponseError::Admission(error)))?;
    let source = current_pending_source(input.command, context.checkpoint)
        .map_err(ResponsePreparationError::Response)?;
    let handler = registry
        .select(context.pins, selector, source)
        .map_err(|error| ResponsePreparationError::Invocation(InvocationError::Dispatch(error)))?;
    if handler.source() != source {
        return Err(ResponsePreparationError::HandlerSourceMismatch);
    }
    registry
        .stage(
            context.pins,
            selector,
            source,
            input,
            context.checkpoint,
            maximum_staged_bytes,
        )
        .map_err(ResponsePreparationError::Invocation)
}

/// Produces canonical current commands by staging each exact current pending response through
/// the same path used on submit. The template selects kind/window/member/operation; stale client
/// labels cannot mint alternatives. Returned commands carry the current trusted observation.
/// Any source/dependency/handler gap refuses the entire query, rather than publishing a partial
/// set as complete. No LegalActionSet, rulebook catalog, source qualification or mechanics is
/// created here; the registered deterministic handler must supply the actual source behavior.
/// Only Choice/Reaction responses are enumerated and staging receives explicit empty actual draws.
/// Where even a legal choice/reaction requires outcomes, this resolution path cannot establish
/// preview legality: its typed refusal leaves the separate source-owned legality preview pending.
/// It must not pre-roll, fabricate outcomes or silently remove a source-legal alternative.
pub fn offered_current_commands<H: RulesCommandHandler>(
    template: &CommandInput,
    context: CurrentRuleContext<'_>,
    registry: &DispatchRegistry<'_, PreconditionedCommandHandler<'_, H>>,
    selector: &RevisionLabel,
    maximum_staged_bytes: usize,
) -> Result<Vec<GameInput>, ResponsePreparationError<H::Rejection>> {
    let capacity_refusal = || ResponsePreparationError::Response(CAPACITY_REFUSAL);
    let offered = current_responses(
        template,
        context.checkpoint,
        context.basis,
        context.pins,
        context.inventory,
        context.command_limits,
    )
    .map_err(ResponsePreparationError::Response)?;
    let count = offered
        .iter()
        .try_fold(0usize, |total, response| {
            total.checked_add(response.options.len())
        })
        .ok_or_else(capacity_refusal)?;
    let mut retained_bytes = count
        .checked_mul(size_of::<GameInput>())
        .and_then(|commands_bytes| commands_bytes.checked_add(size_of::<Vec<GameInput>>()))
        .ok_or_else(capacity_refusal)?;
    if retained_bytes > context.command_limits.maximum_retained_bytes {
        return Err(capacity_refusal());
    }
    let mut commands = Vec::with_capacity(count);
    for response in offered {
        for option in &response.options {
            let command = with_offered_option(&template.command, &response.offer, option)
                .map_err(ResponsePreparationError::Response)?;
            let input = GameInput::Game(CommandInput {
                basis: context.basis,
                observed_revision: context.basis.revision,
                operation: template.operation,
                member: template.member,
                command,
            });
            retained_bytes = input
                .retained_heap_bytes()
                .and_then(|input_bytes| retained_bytes.checked_add(input_bytes))
                .ok_or_else(capacity_refusal)?;
            if retained_bytes > context.command_limits.maximum_retained_bytes {
                return Err(capacity_refusal());
            }
            prepare_current_response(
                RulesCommandInput {
                    command: &input,
                    supplied_draws: &[],
                },
                context,
                registry,
                selector,
                maximum_staged_bytes,
            )?;
            commands.push(input);
        }
    }
    Ok(commands)
}

fn matched_response<'a>(
    input: &GameInput,
    checkpoint: &'a Checkpoint,
) -> Result<&'a OfferedResponse, ResponseError> {
    let GameInput::Game(request) = input else {
        return Err(ResponseError::UnsupportedCommand);
    };
    let (resolution, window, offer, option) = match &request.command {
        GameCommand::SelectChoice {
            resolution,
            window,
            offer,
            option,
        }
        | GameCommand::SelectReaction {
            resolution,
            window,
            offer,
            option,
        } => (*resolution, *window, offer, option),
        _ => return Err(ResponseError::UnsupportedCommand),
    };
    let pending = pending_resolution(checkpoint, resolution, window)?;
    offered_responses(&request.command, pending)?
        .iter()
        .find(|response| {
            response.participant == request.member
                && response.offer == *offer
                && response.options.contains(option)
        })
        .ok_or(ResponseError::Admission(CommandError::UnofferedResponse))
}

/// Resolves the current canonical pending source for registry dispatch; never authorizes input.
/// Admission must precede invocation; returned provenance alone does not prove source legality.
pub fn current_pending_source<'a>(
    input: &GameInput,
    checkpoint: &'a Checkpoint,
) -> Result<&'a RuleReference, ResponseError> {
    match input {
        GameInput::Game(CommandInput {
            command: GameCommand::SelectChoice { .. } | GameCommand::SelectReaction { .. },
            ..
        }) => matched_response(input, checkpoint).map(|response| &response.source),
        GameInput::Game(CommandInput {
            command: GameCommand::SubmitRoll { resolution, window },
            ..
        }) => match &pending_resolution(checkpoint, *resolution, *window)?.next {
            PendingInput::Roll { source, .. } => Ok(source),
            _ => Err(ResponseError::Admission(CommandError::WrongPendingKind)),
        },
        GameInput::Host(request) => {
            let HostCommand::ResolveRuling {
                resolution,
                window,
                offer,
                option,
            } = &request.command
            else {
                return Err(ResponseError::UnsupportedCommand);
            };
            let pending = pending_resolution(checkpoint, *resolution, *window)?;
            let PendingInput::Ruling { permitted, .. } = &pending.next else {
                return Err(ResponseError::Admission(CommandError::WrongPendingKind));
            };
            permitted
                .iter()
                .find(|response| {
                    response.participant == request.host
                        && response.offer == *offer
                        && response.options.contains(option)
                })
                .map(|response| &response.source)
                .ok_or(ResponseError::Admission(CommandError::UnofferedResponse))
        }
        _ => Err(ResponseError::UnsupportedCommand),
    }
}

#[cfg(test)]
mod cross_consumer_tests;
#[cfg(test)]
mod current_responses_tests;
#[cfg(test)]
mod fixtures;
