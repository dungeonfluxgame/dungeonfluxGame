//! Bounded canonical responses and action inventories admitted through compiled Rules.
use crate::candidates::{
    AdmittedCandidates, CandidateContext, CandidateError, CandidateLimits, LegalOfferOwner,
    enumerate_candidates,
};
use df_model::checkpoint::{
    CheckpointError, CommandInput, EntityId, GameCommand, GameInput, OfferedResponse, PendingInput,
    ReferenceInventory, RuleReference,
};
use df_model::commands::{CommandError, validate_client_command};
use df_rules::current_responses::{
    ResponseError, ResponsePreparationError, prepare_current_response,
};
use df_rules::preconditions::{
    CurrentRuleContext, PreconditionedCommandHandler, PreconditionedRejection,
};
use df_rules::{DispatchRegistry, InvocationError, RulesCommandHandler, RulesCommandInput};
use df_types::{MemberId, OperationId, RevisionLabel};

/// Trusted native identities and current pins supplied independently of template labels.
/// The template selects Choice/Reaction kind and pending resolution/window. Its original
/// basis and observed revision pass through canonical command admission before staging.
pub struct RegisteredCandidateRequest<'a> {
    pub template: &'a CommandInput,
    pub current: CandidateContext<'a>,
    pub member: MemberId,
    pub operation: OperationId,
    pub selector: &'a RevisionLabel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegisteredCandidateLimits {
    pub candidates: CandidateLimits,
    pub maximum_checkpoint_bytes: usize,
    pub maximum_inventory_records: usize,
    pub maximum_rules_preparations: usize,
    pub maximum_staged_bytes: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub enum RegisteredCandidateError<R> {
    Capacity,
    Snapshot(CheckpointError),
    StaleBasis,
    StalePins,
    MemberMismatch,
    OperationMismatch,
    UnsupportedCommand,
    Window(CommandError),
    Enumeration(CandidateError<ResponsePreparationError<R>>),
}

/// Returns only exact current responses whose every option passes the same source handler
/// preparation used by Rules I03 on submission. No prepared checkpoint is
/// applied. A source revocation, missing handler, changed dependency or handler refusal rejects
/// the whole batch. Other participants' responses are omitted before rules preparation.
///
/// This is a concrete bounded pending-response connection, not a general NPC LegalActionSet.
/// Source qualification, actor control, perception, approved NPC tactics and session commit
/// remain owning gates; no action, roll, ruling, utility or game mechanic is invented here.
/// Deterministic Choice/Reaction preview supplies explicit empty draws. A handler that needs
/// actual outcomes returns its typed refusal for the whole batch; no outcomes are fabricated
/// or consumed and no partial offer set is presented as complete.
pub fn enumerate_registered_responses<'a, H: RulesCommandHandler>(
    context: CurrentRuleContext<'a>,
    registry: &DispatchRegistry<'_, PreconditionedCommandHandler<'_, H>>,
    request: RegisteredCandidateRequest<'a>,
    limits: RegisteredCandidateLimits,
) -> Result<AdmittedCandidates<'a, OfferedResponse>, RegisteredCandidateError<H::Rejection>> {
    if limits.maximum_checkpoint_bytes == 0
        || limits.maximum_inventory_records == 0
        || limits.maximum_staged_bytes == 0
        || context
            .checkpoint
            .retained_bytes()
            .is_none_or(|bytes| bytes > limits.maximum_checkpoint_bytes)
    {
        return Err(RegisteredCandidateError::Capacity);
    }
    let inventory_records = [
        context.inventory.rules.len(),
        context.inventory.content.len(),
        context.inventory.resources.len(),
        context.inventory.assets.len(),
    ]
    .into_iter()
    .try_fold(0usize, |total, count| total.checked_add(count))
    .ok_or(RegisteredCandidateError::Capacity)?;
    if inventory_records > limits.maximum_inventory_records {
        return Err(RegisteredCandidateError::Capacity);
    }
    if context.basis != *request.current.basis {
        return Err(RegisteredCandidateError::StaleBasis);
    }
    if context.pins != request.current.pins {
        return Err(RegisteredCandidateError::StalePins);
    }
    context
        .checkpoint
        .validate_resume(context.basis, context.pins)
        .map_err(RegisteredCandidateError::Snapshot)?;
    if request.template.member != request.member {
        return Err(RegisteredCandidateError::MemberMismatch);
    }
    if request.template.operation != request.operation {
        return Err(RegisteredCandidateError::OperationMismatch);
    }
    if !context
        .checkpoint
        .state()
        .members
        .iter()
        .any(|link| link.member == request.member)
    {
        return Err(RegisteredCandidateError::Window(
            CommandError::UnknownMember,
        ));
    }
    let (resolution, window) = match &request.template.command {
        GameCommand::SelectChoice {
            resolution, window, ..
        }
        | GameCommand::SelectReaction {
            resolution, window, ..
        } => (*resolution, *window),
        _ => return Err(RegisteredCandidateError::UnsupportedCommand),
    };
    let pending = context
        .checkpoint
        .state()
        .pending
        .iter()
        .find(|pending| pending.id == resolution && pending.window.id == window)
        .ok_or(RegisteredCandidateError::Window(CommandError::StaleWindow))?;
    if pending.window.source.catalog != context.pins.rules.catalog
        || !context.inventory.rules.contains(&pending.window.source)
    {
        return Err(RegisteredCandidateError::Window(
            CommandError::InvalidReference,
        ));
    }
    let responses = match (&request.template.command, &pending.next) {
        (GameCommand::SelectChoice { .. }, PendingInput::Choice { remaining })
        | (GameCommand::SelectReaction { .. }, PendingInput::Reaction { remaining }) => remaining,
        _ => {
            return Err(RegisteredCandidateError::Window(
                CommandError::WrongPendingKind,
            ));
        }
    };
    let preparations = responses
        .iter()
        .filter(|response| response.participant == request.member)
        .try_fold(0usize, |total, response| {
            total.checked_add(response.options.len())
        })
        .ok_or(RegisteredCandidateError::Capacity)?;
    if preparations > limits.maximum_rules_preparations {
        return Err(RegisteredCandidateError::Capacity);
    }
    let current = request.current;
    let owner = RegistryOwner {
        context,
        registry,
        request: &request,
        maximum_staged_bytes: limits.maximum_staged_bytes,
    };
    enumerate_candidates(
        &owner,
        request.template,
        current,
        current,
        responses,
        limits.candidates,
    )
    .map_err(RegisteredCandidateError::Enumeration)
}

struct RegistryOwner<'a, 'b, 'c, 'd, H> {
    context: CurrentRuleContext<'a>,
    registry: &'b DispatchRegistry<'c, PreconditionedCommandHandler<'d, H>>,
    request: &'b RegisteredCandidateRequest<'a>,
    maximum_staged_bytes: usize,
}

impl<H: RulesCommandHandler> LegalOfferOwner for RegistryOwner<'_, '_, '_, '_, H> {
    type Observation = CommandInput;
    type Offer = OfferedResponse;
    type Error = ResponsePreparationError<H::Rejection>;

    fn validate_observation(
        &self,
        _: &CommandInput,
        _: CandidateContext<'_>,
    ) -> Result<(), Self::Error> {
        // The public entry has already checked trusted identity, snapshot and current pins.
        Ok(())
    }

    fn is_current_legal(
        &self,
        template: &CommandInput,
        _: CandidateContext<'_>,
        response: &OfferedResponse,
    ) -> Result<bool, Self::Error> {
        if response.participant != self.request.member || response.options.is_empty() {
            return Ok(false);
        }
        for option in &response.options {
            let command = match &template.command {
                GameCommand::SelectChoice {
                    resolution, window, ..
                } => GameCommand::SelectChoice {
                    resolution: *resolution,
                    window: *window,
                    offer: response.offer.clone(),
                    option: option.clone(),
                },
                GameCommand::SelectReaction {
                    resolution, window, ..
                } => GameCommand::SelectReaction {
                    resolution: *resolution,
                    window: *window,
                    offer: response.offer.clone(),
                    option: option.clone(),
                },
                _ => {
                    return Err(ResponsePreparationError::Response(
                        ResponseError::UnsupportedCommand,
                    ));
                }
            };
            // Current response labels cannot rebase a request from a foreign scope or epoch.
            // Canonical admission still permits older observations in the current epoch.
            let candidate = GameInput::Game(CommandInput {
                basis: template.basis,
                observed_revision: template.observed_revision,
                operation: self.request.operation,
                member: self.request.member,
                command,
            });
            prepare_current_response(
                RulesCommandInput {
                    command: &candidate,
                    supplied_draws: &[],
                },
                borrowed_context(&self.context),
                self.registry,
                self.request.selector,
                self.maximum_staged_bytes,
            )?;
        }
        Ok(true)
    }

    fn same_offer(
        &self,
        left: &OfferedResponse,
        right: &OfferedResponse,
    ) -> Result<bool, Self::Error> {
        Ok(left.participant == right.participant && left.offer == right.offer)
    }

    fn offer_bytes(&self, response: &OfferedResponse) -> Result<usize, Self::Error> {
        let mut bytes = size_of::<OfferedResponse>()
            .checked_add(
                response
                    .options
                    .capacity()
                    .checked_mul(size_of::<RevisionLabel>())
                    .ok_or_else(capacity_error)?,
            )
            .ok_or_else(capacity_error)?;
        for label in [
            &response.offer,
            &response.source.catalog,
            &response.source.source,
            &response.source.entry,
            &response.source.clause,
        ]
        .into_iter()
        .chain(&response.options)
        {
            bytes = bytes
                .checked_add(label.retained_heap_bytes())
                .ok_or_else(capacity_error)?;
        }
        Ok(bytes)
    }
}

fn capacity_error<R>() -> ResponsePreparationError<R> {
    ResponsePreparationError::Response(ResponseError::Admission(CommandError::Capacity))
}

fn borrowed_context<'a>(context: &CurrentRuleContext<'a>) -> CurrentRuleContext<'a> {
    let inventory = &context.inventory;
    CurrentRuleContext {
        checkpoint: context.checkpoint,
        basis: context.basis,
        pins: context.pins,
        inventory: ReferenceInventory {
            rules: inventory.rules,
            content: inventory.content,
            resources: inventory.resources,
            assets: inventory.assets,
        },
        command_limits: context.command_limits,
    }
}

/// Native source-owner admission for one finite actor action inventory.
/// These values come from the session/source owners independently of offered commands.
/// They confer no actor control, perception rights or mechanical authority themselves.
pub struct RegisteredActionRequest<'a> {
    pub current: CandidateContext<'a>,
    pub actor: EntityId,
    pub member: MemberId,
    pub operation: OperationId,
    pub source: &'a RuleReference,
    pub selector: &'a RevisionLabel,
}

/// Ephemeral source-bound receipt over exact borrowed canonical action inputs.
/// No staged checkpoint, outcome or serialization is retained.
pub struct RegisteredActionCandidates<'a> {
    admitted: AdmittedCandidates<'a, GameInput>,
    actor: EntityId,
    member: MemberId,
    operation: OperationId,
    source: &'a RuleReference,
    selector: &'a RevisionLabel,
}

impl<'a> RegisteredActionCandidates<'a> {
    pub fn admitted(&self) -> &AdmittedCandidates<'a, GameInput> {
        &self.admitted
    }

    pub fn actor(&self) -> EntityId {
        self.actor
    }

    pub fn member(&self) -> MemberId {
        self.member
    }

    pub fn operation(&self) -> OperationId {
        self.operation
    }

    pub fn source(&self) -> &'a RuleReference {
        self.source
    }

    pub fn selector(&self) -> &'a RevisionLabel {
        self.selector
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum RegisteredActionError<R> {
    Capacity,
    Snapshot(CheckpointError),
    StaleBasis,
    StalePins,
    MemberMismatch,
    OperationMismatch,
    ActorMismatch,
    UnsupportedCommand,
    Admission(CommandError),
    SourceMismatch,
    Invocation(InvocationError<PreconditionedRejection<R>>),
    Enumeration(CandidateError<InvocationError<PreconditionedRejection<R>>>),
}

/// Admit a supplied, source-owned ProposeAction inventory through its exact compiled handler.
///
/// This is a single-source deterministic action admission boundary, not an action producer or
/// complete NPC LegalActionSet. Rules own every mechanical prerequisite and the reviewed
/// dependency list. Native owners must establish actor control and perception before supplying
/// the inventory. No targets, tactics, source mapping, catalog entries or mechanics are invented.
/// Every command is checked before any preparation; duplicate inputs reject the entire batch.
/// Each preparation receives empty actual draws and its detached checkpoint is discarded.
/// A draw-dependent or unsupported handler refuses the whole inventory, never a partial legal
/// set. Empty inventory is a successful no-action receipt. Session alone accepts and commits.
pub fn enumerate_registered_actions<'a, H: RulesCommandHandler>(
    context: CurrentRuleContext<'a>,
    registry: &DispatchRegistry<'_, PreconditionedCommandHandler<'_, H>>,
    request: RegisteredActionRequest<'a>,
    offers: &'a [GameInput],
    limits: RegisteredCandidateLimits,
) -> Result<RegisteredActionCandidates<'a>, RegisteredActionError<H::Rejection>> {
    if limits.maximum_checkpoint_bytes == 0
        || limits.maximum_inventory_records == 0
        || limits.maximum_staged_bytes == 0
        || context
            .checkpoint
            .retained_bytes()
            .is_none_or(|bytes| bytes > limits.maximum_checkpoint_bytes)
        || offers.len() > limits.maximum_rules_preparations
        || offers.len() > limits.candidates.max_offers
        || offers.len() > limits.candidates.max_candidates
    {
        return Err(RegisteredActionError::Capacity);
    }
    let inventory_records = [
        context.inventory.rules.len(),
        context.inventory.content.len(),
        context.inventory.resources.len(),
        context.inventory.assets.len(),
    ]
    .into_iter()
    .try_fold(0usize, |total, count| total.checked_add(count))
    .ok_or(RegisteredActionError::Capacity)?;
    if inventory_records > limits.maximum_inventory_records {
        return Err(RegisteredActionError::Capacity);
    }
    let mut retained_bytes = 0usize;
    for offer in offers {
        let bytes = offer
            .retained_bytes()
            .ok_or(RegisteredActionError::Capacity)?;
        if bytes > context.command_limits.maximum_retained_bytes {
            return Err(RegisteredActionError::Capacity);
        }
        retained_bytes = retained_bytes
            .checked_add(bytes)
            .ok_or(RegisteredActionError::Capacity)?;
    }
    if retained_bytes > limits.candidates.max_candidate_bytes {
        return Err(RegisteredActionError::Capacity);
    }
    if context.basis != *request.current.basis {
        return Err(RegisteredActionError::StaleBasis);
    }
    if context.pins != request.current.pins {
        return Err(RegisteredActionError::StalePins);
    }
    context
        .checkpoint
        .validate_resume(context.basis, context.pins)
        .map_err(RegisteredActionError::Snapshot)?;
    if !context
        .checkpoint
        .state()
        .members
        .iter()
        .any(|link| link.member == request.member)
    {
        return Err(RegisteredActionError::Admission(
            CommandError::UnknownMember,
        ));
    }
    if !context
        .checkpoint
        .state()
        .entities
        .iter()
        .any(|entity| entity.id == request.actor)
    {
        return Err(RegisteredActionError::Admission(
            CommandError::InvalidReference,
        ));
    }
    if request.source.catalog != context.pins.rules.catalog
        || !context.inventory.rules.contains(request.source)
    {
        return Err(RegisteredActionError::SourceMismatch);
    }
    for offer in offers {
        let GameInput::Game(command) = offer else {
            return Err(RegisteredActionError::UnsupportedCommand);
        };
        let GameCommand::ProposeAction { actor, .. } = &command.command else {
            return Err(RegisteredActionError::UnsupportedCommand);
        };
        if command.member != request.member {
            return Err(RegisteredActionError::MemberMismatch);
        }
        if command.operation != request.operation {
            return Err(RegisteredActionError::OperationMismatch);
        }
        if *actor != request.actor {
            return Err(RegisteredActionError::ActorMismatch);
        }
        if command.basis != context.basis || command.observed_revision != context.basis.revision {
            return Err(RegisteredActionError::StaleBasis);
        }
        validate_client_command(
            offer,
            context.checkpoint,
            borrowed_context(&context).inventory,
            context.command_limits,
        )
        .map_err(RegisteredActionError::Admission)?;
    }
    let handler = registry
        .select(context.pins, request.selector, request.source)
        .map_err(|error| RegisteredActionError::Invocation(InvocationError::Dispatch(error)))?;
    if handler.source() != request.source {
        return Err(RegisteredActionError::SourceMismatch);
    }
    let current = request.current;
    let owner = ActionRegistryOwner {
        context,
        registry,
        request: &request,
        maximum_staged_bytes: limits.maximum_staged_bytes,
    };
    let admitted = enumerate_candidates(&owner, &(), current, current, offers, limits.candidates)
        .map_err(RegisteredActionError::Enumeration)?;
    Ok(RegisteredActionCandidates {
        admitted,
        actor: request.actor,
        member: request.member,
        operation: request.operation,
        source: request.source,
        selector: request.selector,
    })
}

struct ActionRegistryOwner<'a, 'b, 'c, 'd, H> {
    context: CurrentRuleContext<'a>,
    registry: &'b DispatchRegistry<'c, PreconditionedCommandHandler<'d, H>>,
    request: &'b RegisteredActionRequest<'a>,
    maximum_staged_bytes: usize,
}

impl<H: RulesCommandHandler> LegalOfferOwner for ActionRegistryOwner<'_, '_, '_, '_, H> {
    type Observation = ();
    type Offer = GameInput;
    type Error = InvocationError<PreconditionedRejection<H::Rejection>>;

    fn validate_observation(&self, _: &(), _: CandidateContext<'_>) -> Result<(), Self::Error> {
        // The entry validated the entire supplied native identity and input batch.
        Ok(())
    }

    fn is_current_legal(
        &self,
        _: &(),
        _: CandidateContext<'_>,
        offer: &GameInput,
    ) -> Result<bool, Self::Error> {
        self.registry.stage(
            self.context.pins,
            self.request.selector,
            self.request.source,
            RulesCommandInput {
                command: offer,
                supplied_draws: &[],
            },
            self.context.checkpoint,
            self.maximum_staged_bytes,
        )?;
        Ok(true)
    }

    fn same_offer(&self, left: &GameInput, right: &GameInput) -> Result<bool, Self::Error> {
        Ok(left == right)
    }

    fn offer_bytes(&self, offer: &GameInput) -> Result<usize, Self::Error> {
        offer.retained_bytes().ok_or(InvocationError::Capacity)
    }
}
