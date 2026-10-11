//! Source-owned world proposals through the registered rules/session boundary.
//! Environmental causes are committed; time and movement facts come from source handlers.

use df_model::checkpoint::{
    AcceptedDecision, Basis, Checkpoint, CheckpointError, CheckpointLimits, CheckpointPins,
    CommandInput, ContentReference, FactId, GameCommand, GameInput, ReferenceInventory,
    RuleReference,
};
use df_rules::{RulesCommandHandler, RulesCommandInput};
use df_types::RevisionLabel;
use df_world::{
    AdmittedEnvironmentalChange, EnvironmentalDeltaError, EnvironmentalDeltaLimits,
    stage_environmental_delta,
};

/// Compiled native source mapping; possession of these references is not authorization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnvironmentalRegistration {
    pub action: ContentReference,
    pub environment: ContentReference,
    pub source: RuleReference,
    pub policy: RevisionLabel,
}

/// Independently authorize the complete current registration, source rights and actor control.
/// Implementations are pure and bounded. They inspect every proposed replacement and its
/// retained accepted causes, including the original decision policy and permitted audience.
/// Joined membership, an inventory reference, or a client action never grants source authority.
/// Geometry-byte/spatial legality, when required by the source, remains this owner's prerequisite.
pub trait EnvironmentalSourceOwner {
    type Refusal;

    fn admit(
        &self,
        current: &Checkpoint,
        registration: &EnvironmentalRegistration,
        command: &CommandInput,
        changes: &[AdmittedEnvironmentalChange<'_>],
    ) -> Result<(), Self::Refusal>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorldStagingLimits {
    pub maximum_checkpoint_bytes: usize,
    pub maximum_pass_bytes: usize,
    pub maximum_inventory_records: usize,
    pub checkpoint: CheckpointLimits,
    pub world: EnvironmentalDeltaLimits,
}

#[derive(Debug, Eq, PartialEq)]
pub enum WorldStagingError<R> {
    Capacity,
    Snapshot(CheckpointError),
    UnsupportedInput,
    StaleCommand,
    PendingResolution,
    AlreadyAccepted,
    MissingProducer,
    InvalidRegistration,
    WrongCommand,
    Source(R),
    World(EnvironmentalDeltaError),
    RevisionExhausted,
    InvalidCandidate(CheckpointError),
}

/// Trusted native request captured independently of client input. Register this handler behind
/// `PreconditionedCommandHandler` with the source-reviewed prerequisites, then invoke
/// `decide_registered_command` inside `SessionEngine`. Only the session owner commits/publishes.
/// `None` is a missing producer; `Some([])` is an explicitly source-admitted no-change request.
pub struct EnvironmentalChangeHandler<'a, Owner> {
    pub owner: &'a Owner,
    pub current_basis: Basis,
    pub admitted_pins: &'a CheckpointPins,
    pub inventory: ReferenceInventory<'a>,
    pub registration: &'a EnvironmentalRegistration,
    pub changes: Option<&'a [AdmittedEnvironmentalChange<'a>]>,
    pub limits: WorldStagingLimits,
}

impl<Owner: EnvironmentalSourceOwner> RulesCommandHandler
    for EnvironmentalChangeHandler<'_, Owner>
{
    type Rejection = WorldStagingError<Owner::Refusal>;

    fn pins(&self) -> &CheckpointPins {
        self.admitted_pins
    }

    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, Self::Rejection> {
        self.check_capacity(current, input.command)?;
        current
            .validate_resume(self.current_basis, self.admitted_pins)
            .map_err(WorldStagingError::Snapshot)?;
        let GameInput::Game(command) = input.command else {
            return Err(WorldStagingError::UnsupportedInput);
        };
        if !input.supplied_draws.is_empty() {
            return Err(WorldStagingError::UnsupportedInput);
        }
        if command.basis != self.current_basis
            || command.observed_revision != self.current_basis.revision
        {
            return Err(WorldStagingError::StaleCommand);
        }
        if !current.state().pending.is_empty() {
            return Err(WorldStagingError::PendingResolution);
        }
        if current
            .state()
            .decisions
            .iter()
            .any(|decision| decision.operation == command.operation)
        {
            return Err(WorldStagingError::AlreadyAccepted);
        }
        let changes = self.changes.ok_or(WorldStagingError::MissingProducer)?;
        let registration = self.registration;
        if registration.source.catalog != current.pins().rules.catalog
            || !self.inventory.rules.contains(&registration.source)
            || [&registration.action, &registration.environment]
                .into_iter()
                .any(|reference| {
                    reference.package != current.pins().content.package
                        || !self.inventory.content.contains(reference)
                })
            || changes
                .iter()
                .any(|change| change.replacement.definition != registration.environment)
        {
            return Err(WorldStagingError::InvalidRegistration);
        }
        let GameCommand::ProposeAction {
            actor,
            action,
            targets,
            choices,
        } = &command.command
        else {
            return Err(WorldStagingError::WrongCommand);
        };
        if action != &registration.action
            || !choices.is_empty()
            || targets.len() != changes.len()
            || targets
                .iter()
                .zip(changes)
                .any(|(target, change)| *target != change.expected.location)
            || !current
                .state()
                .members
                .iter()
                .any(|link| link.member == command.member && link.character == Some(*actor))
            || !current
                .state()
                .entities
                .iter()
                .any(|entity| entity.id == *actor)
        {
            return Err(WorldStagingError::WrongCommand);
        }
        // Source authority is checked independently before any detached proposal/candidate.
        self.owner
            .admit(current, registration, command, changes)
            .map_err(WorldStagingError::Source)?;
        let proposal = stage_environmental_delta(
            current,
            self.admitted_pins,
            self.current_basis,
            Some(changes),
            self.limits.world,
        )
        .map_err(WorldStagingError::World)?;
        let mut next = self.current_basis;
        next.revision = next
            .revision
            .next_sequence()
            .map_err(|_| WorldStagingError::RevisionExhausted)?;
        let mut state = current.state().clone();
        for replacement in proposal.replacements {
            let record = state
                .continuity
                .environment
                .iter_mut()
                .find(|record| record.location == replacement.location)
                .ok_or(WorldStagingError::World(
                    EnvironmentalDeltaError::UnknownEnvironment,
                ))?;
            *record = replacement;
        }
        state
            .decisions
            .try_reserve_exact(1)
            .map_err(|_| WorldStagingError::Capacity)?;
        // The new operation owns application of the proposal, not the original cause facts.
        // Keeping its fact list empty preserves the immutable declaration of each prior cause.
        state.decisions.push(AcceptedDecision {
            operation: command.operation,
            revision: next.revision,
            facts: vec![],
            draws: vec![],
            effects: vec![],
            source_policy: registration.policy.clone(),
            semantic_output: None,
        });
        Checkpoint::new(
            current.schema(),
            next,
            current.pins().clone(),
            state,
            ReferenceInventory {
                rules: self.inventory.rules,
                content: self.inventory.content,
                resources: self.inventory.resources,
                assets: self.inventory.assets,
            },
            self.limits.checkpoint,
        )
        .map_err(WorldStagingError::InvalidCandidate)
    }
}

impl<Owner: EnvironmentalSourceOwner> EnvironmentalChangeHandler<'_, Owner> {
    fn check_capacity(
        &self,
        current: &Checkpoint,
        input: &GameInput,
    ) -> Result<(), WorldStagingError<Owner::Refusal>> {
        let limits = self.limits;
        let changes = self.changes.unwrap_or(&[]);
        if limits.maximum_checkpoint_bytes == 0
            || limits.maximum_inventory_records == 0
            || limits.checkpoint.maximum_retained_bytes == 0
            || limits.checkpoint.maximum_retained_bytes > limits.maximum_checkpoint_bytes
            || limits.world.input_records == 0
            || limits.world.input_records > 4096
            || limits.world.changes == 0
            || limits.world.changes > limits.world.input_records
            || limits.world.output_bytes == 0
            || changes.len() > limits.world.changes
        {
            return Err(WorldStagingError::Capacity);
        }
        let mut request_bytes = std::mem::size_of::<EnvironmentalRegistration>();
        let mut add = |bytes: usize| -> Result<(), WorldStagingError<Owner::Refusal>> {
            request_bytes = request_bytes
                .checked_add(bytes)
                .ok_or(WorldStagingError::Capacity)?;
            if request_bytes > limits.maximum_pass_bytes {
                return Err(WorldStagingError::Capacity);
            }
            Ok(())
        };
        for label in [
            &self.registration.action.package,
            &self.registration.action.entry,
            &self.registration.environment.package,
            &self.registration.environment.entry,
            &self.registration.source.catalog,
            &self.registration.source.source,
            &self.registration.source.entry,
            &self.registration.source.clause,
            &self.registration.policy,
        ] {
            add(label.retained_heap_bytes())?;
        }
        for change in changes {
            add(std::mem::size_of::<AdmittedEnvironmentalChange<'_>>())?;
            for environment in [change.expected, change.replacement] {
                if environment.change_facts.len() > limits.world.input_records {
                    return Err(WorldStagingError::Capacity);
                }
                add(std::mem::size_of_val(environment))?;
                add(environment.definition.package.retained_heap_bytes())?;
                add(environment.definition.entry.retained_heap_bytes())?;
                add(environment
                    .change_facts
                    .len()
                    .checked_mul(std::mem::size_of::<FactId>())
                    .ok_or(WorldStagingError::Capacity)?)?;
            }
            if change.geometry.source_facts.len() > limits.world.input_records {
                return Err(WorldStagingError::Capacity);
            }
            add(std::mem::size_of_val(change.geometry))?;
            for label in [
                &change.geometry.revision,
                &change.geometry.geometry.key,
                &change.geometry.canonical_pack,
            ] {
                add(label.retained_heap_bytes())?;
            }
            add(change
                .geometry
                .source_facts
                .len()
                .checked_mul(std::mem::size_of::<FactId>())
                .ok_or(WorldStagingError::Capacity)?)?;
        }
        let records = [
            self.inventory.rules.len(),
            self.inventory.content.len(),
            self.inventory.resources.len(),
            self.inventory.assets.len(),
        ]
        .into_iter()
        .try_fold(0usize, |count, next| count.checked_add(next))
        .ok_or(WorldStagingError::Capacity)?;
        let current_bytes = current
            .retained_bytes()
            .ok_or(WorldStagingError::Capacity)?;
        let total = limits
            .maximum_checkpoint_bytes
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(current_bytes))
            .and_then(|bytes| bytes.checked_add(limits.world.output_bytes))
            .and_then(|bytes| bytes.checked_add(request_bytes))
            .and_then(|bytes| bytes.checked_add(input.retained_bytes()?))
            .ok_or(WorldStagingError::Capacity)?;
        if records > limits.maximum_inventory_records
            || current_bytes > limits.maximum_checkpoint_bytes
            || total > limits.maximum_pass_bytes
        {
            return Err(WorldStagingError::Capacity);
        }
        Ok(())
    }
}

/// One exact source producer followed by the existing logical schedule composition.
/// The session supplies its current admitted pause state; this does not persist a pause policy.
/// Register the decorator with the producer's exact clause and current source prerequisites.
/// Only a time-only schedule producer is supported: its entity state and ordered movement facts
/// must match the immutable original schedule selection. Durable admission/publication remain
/// session-owned; this decorator creates no source facts or decisions.
pub struct AcceptedTimeScheduleHandler<'a, Handler> {
    pub producer: &'a Handler,
    pub current_basis: Basis,
    pub inventory: ReferenceInventory<'a>,
    pub paused: bool,
    pub deadline_remaining: std::time::Duration,
    pub policy: &'a ContentReference,
    pub destinations: &'a [df_world::AdmittedScheduleDestination<'a>],
    pub limits: crate::director_staging::ScheduleDirectorLimits,
}

/// A refusal returns no candidate and leaves the complete original checkpoint unchanged.
#[derive(Debug, Eq, PartialEq)]
pub enum TimeScheduleError<R> {
    Capacity,
    Snapshot(CheckpointError),
    UnsupportedInput,
    StaleCommand,
    Paused,
    PendingResolution,
    Producer(df_rules::InvocationError<R>),
    InvalidTimeFact,
    MissingMovementFact,
    SourceWorldConflict,
    World(crate::director_staging::DirectorError),
    WorldPending,
    InvalidCandidate(CheckpointError),
}

impl<Handler: RulesCommandHandler> RulesCommandHandler
    for AcceptedTimeScheduleHandler<'_, Handler>
{
    type Rejection = TimeScheduleError<Handler::Rejection>;

    fn pins(&self) -> &CheckpointPins {
        self.producer.pins()
    }

    fn bound_source(&self) -> Option<&RuleReference> {
        self.producer.bound_source()
    }

    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, Self::Rejection> {
        use crate::director_staging::{
            DirectorCandidates, DirectorStaging, EnvironmentalRequest, ScheduleDirectorRequest,
            compose_schedule_candidates,
        };
        use df_model::checkpoint::FactValue;
        use df_world::DueSelectionRequest;

        let bounds = self.limits.directors;
        let request_bytes = self
            .destinations
            .len()
            .checked_mul(std::mem::size_of::<df_world::AdmittedScheduleDestination<'_>>())
            .and_then(|bytes| bytes.checked_add(self.policy.package.retained_heap_bytes()))
            .and_then(|bytes| bytes.checked_add(self.policy.entry.retained_heap_bytes()))
            .ok_or(TimeScheduleError::Capacity)?;
        let inventory_records = [
            self.inventory.rules.len(),
            self.inventory.content.len(),
            self.inventory.resources.len(),
            self.inventory.assets.len(),
        ]
        .into_iter()
        .try_fold(0usize, |total, count| total.checked_add(count))
        .ok_or(TimeScheduleError::Capacity)?;
        let budget = bounds
            .maximum_checkpoint_bytes
            .checked_mul(5)
            .and_then(|bytes| bytes.checked_add(self.limits.schedule.output_bytes))
            .and_then(|bytes| bytes.checked_add(request_bytes))
            .ok_or(TimeScheduleError::Capacity)?;
        if inventory_records > self.limits.checkpoint.maximum_records
            || bounds.maximum_checkpoint_bytes == 0
            || self.limits.checkpoint.maximum_retained_bytes == 0
            || self.limits.checkpoint.maximum_retained_bytes > bounds.maximum_checkpoint_bytes
            || self.destinations.len() > self.limits.schedule.destinations
            || self.destinations.len() > 4096
            || budget > bounds.maximum_pass_bytes
            || current
                .retained_bytes()
                .is_none_or(|bytes| bytes > bounds.maximum_checkpoint_bytes)
        {
            return Err(TimeScheduleError::Capacity);
        }
        current
            .validate_resume(self.current_basis, self.pins())
            .map_err(TimeScheduleError::Snapshot)?;
        let (basis, observed) = match input.command {
            GameInput::Game(command) => (command.basis, command.observed_revision),
            GameInput::Host(command) => (command.basis, command.basis.revision),
            _ => return Err(TimeScheduleError::UnsupportedInput),
        };
        if basis != self.current_basis || observed != self.current_basis.revision {
            return Err(TimeScheduleError::StaleCommand);
        }
        if self.paused {
            return Err(TimeScheduleError::Paused);
        }
        if !current.state().pending.is_empty() {
            return Err(TimeScheduleError::PendingResolution);
        }
        // The canonical guard binds the sole new operation, its ordered facts and borrowed draws.
        // Registration/source rights and actor control remain the approved source owner's work.
        let candidate = df_rules::stage_handler(
            self.producer,
            self.pins(),
            input,
            current,
            bounds.maximum_checkpoint_bytes,
        )
        .map_err(TimeScheduleError::Producer)?;
        if !candidate.state().pending.is_empty() {
            return Err(TimeScheduleError::PendingResolution);
        }
        let appended = candidate
            .state()
            .facts
            .get(current.state().facts.len()..)
            .ok_or(TimeScheduleError::InvalidTimeFact)?;
        let mut times = appended.iter().filter_map(|fact| match fact.value {
            FactValue::TimeAdvanced { before, after } => Some((before, after)),
            _ => None,
        });
        if times.next() != Some((current.state().logical_time, candidate.state().logical_time))
            || times.next().is_some()
        {
            return Err(TimeScheduleError::InvalidTimeFact);
        }
        // The producer owns source consequences. Only this adapter owns the derived cursor.
        if candidate.state().schedules != current.state().schedules
            || candidate.state().continuity.catch_up != current.state().continuity.catch_up
            || candidate.state().continuity.environment != current.state().continuity.environment
        {
            return Err(TimeScheduleError::SourceWorldConflict);
        }
        let composed = compose_schedule_candidates(
            current,
            self.pins(),
            ScheduleDirectorRequest {
                accepted_time: Some(DueSelectionRequest {
                    expected_basis: self.current_basis,
                    target_time: candidate.state().logical_time,
                    paused: false,
                    deadline_remaining: self.deadline_remaining,
                    policy: self.policy,
                }),
                destinations: self.destinations,
                environmental: EnvironmentalRequest::NotApplicable,
            },
            DirectorCandidates {
                interaction: None,
                narrative: None,
            },
            ReferenceInventory {
                rules: self.inventory.rules,
                content: self.inventory.content,
                resources: self.inventory.resources,
                assets: self.inventory.assets,
            },
            self.limits,
        )
        .map_err(TimeScheduleError::World)?;
        let DirectorStaging::Staged(staged) = composed else {
            return Err(TimeScheduleError::WorldPending);
        };
        if candidate.state().entities != staged.candidate().state().entities {
            return Err(TimeScheduleError::SourceWorldConflict);
        }
        let mut movement_facts = appended.iter().filter_map(|fact| match &fact.value {
            FactValue::EntityMoved {
                entity,
                destination,
                position,
            } => Some((*entity, *destination, *position)),
            _ => None,
        });
        for event in &staged.world().events {
            let destination = self
                .destinations
                .iter()
                .find(|destination| destination.event == event.id)
                .ok_or(TimeScheduleError::MissingMovementFact)?;
            let position = current
                .state()
                .entities
                .iter()
                .find(|entity| entity.id == destination.entity)
                .and_then(|entity| entity.position)
                .ok_or(TimeScheduleError::MissingMovementFact)?;
            if movement_facts.next()
                != Some((destination.entity, destination.destination, position))
            {
                return Err(TimeScheduleError::MissingMovementFact);
            }
        }
        if movement_facts.next().is_some() {
            return Err(TimeScheduleError::MissingMovementFact);
        }
        // All source records remain exact. No time/movement fact or accepted decision is invented.
        let mut state = candidate.state().clone();
        state.continuity.catch_up = staged.candidate().state().continuity.catch_up.clone();
        Checkpoint::new(
            candidate.schema(),
            candidate.basis(),
            candidate.pins().clone(),
            state,
            ReferenceInventory {
                rules: self.inventory.rules,
                content: self.inventory.content,
                resources: self.inventory.resources,
                assets: self.inventory.assets,
            },
            self.limits.checkpoint,
        )
        .map_err(TimeScheduleError::InvalidCandidate)
    }
}
