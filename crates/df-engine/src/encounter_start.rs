//! Pure, source-bound initiative for the admitted local journey party.
use df_encounter::participants::ValidatedParticipants;
use df_model::checkpoint::{
    Basis, Checkpoint, CheckpointError, EntityId, GameInput, GameState, ResolutionId,
    RuleReference, WindowId,
};
use df_model::commands::{CommandError, validate_client_command};
use df_rules::local_journey::{JourneyRuleError, initiative_order};

use crate::command_entry::{CommandEntryContext, RulesCommandInput};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EncounterStartError {
    Checkpoint(CheckpointError),
    Command(CommandError),
    InvalidParticipants,
    InvalidSource,
    InvalidDraws,
    InvalidDexterity,
    Rules(JourneyRuleError),
}

/// Order the exact admitted local party using already-observed d20 draws.
///
/// The caller admits the working state and participants through their canonical
/// owner and supplies a trusted source inventory. This neither authenticates a
/// sender nor commits an encounter. It validates the current basis, pins, source,
/// draw identities and source-backed Dexterity resources without mutation or I/O.
/// Equal totals retain the caller's authored participant order; this is the local
/// journey decision, not a general 2024 tie-policy implementation.
pub fn local_journey_turn_order(
    input: RulesCommandInput<'_>,
    current: &Checkpoint,
    context: CommandEntryContext<'_>,
    admitted: &ValidatedParticipants<'_, EntityId, Basis>,
    staged: &GameState,
    source: &RuleReference,
) -> Result<Vec<EntityId>, EncounterStartError> {
    current
        .validate_resume(context.current_basis, context.admitted_pins)
        .map_err(EncounterStartError::Checkpoint)?;
    validate_client_command(
        input.command,
        current,
        context.inventory,
        context.limits.command,
    )
    .map_err(EncounterStartError::Command)?;
    let GameInput::Game(command) = input.command else {
        return Err(EncounterStartError::InvalidParticipants);
    };
    if *admitted.current_basis() != context.current_basis || command.basis != context.current_basis
    {
        return Err(EncounterStartError::InvalidParticipants);
    }
    let actors = admitted.actors();
    if !(2..=df_rules::local_journey::MAXIMUM_PARTY + 1).contains(&actors.len()) {
        return Err(EncounterStartError::InvalidParticipants);
    }
    if source.catalog != context.admitted_pins.rules.catalog
        || !context.inventory.rules.contains(source)
    {
        return Err(EncounterStartError::InvalidSource);
    }
    let draws = input.supplied_draws;
    if draws.len() != actors.len() {
        return Err(EncounterStartError::InvalidDraws);
    }
    let resolution = ResolutionId::from_bytes(command.operation.as_bytes())
        .map_err(|_| EncounterStartError::InvalidDraws)?;
    let window = WindowId::from_bytes(command.operation.as_bytes())
        .map_err(|_| EncounterStartError::InvalidDraws)?;
    // The admitted inventory bounds the local resource scan, including new Bandit resources.
    if staged.resources.len() > context.inventory.resources.len() {
        return Err(EncounterStartError::InvalidDexterity);
    }
    let mut initiative_inputs = vec![(0, 0); actors.len()];
    for (ordinal, ((roll_and_dexterity, actor), draw)) in initiative_inputs
        .iter_mut()
        .zip(actors)
        .zip(draws)
        .enumerate()
    {
        if draw.operation != command.operation
            || draw.ordinal != ordinal as u32
            || draw.resolution != resolution
            || draw.window != window
            || draw.source != *source
            || draw.sides != 20
            || !(1..=20).contains(&draw.value)
        {
            return Err(EncounterStartError::InvalidDraws);
        }
        let mut resources = staged.resources.iter().filter(|resource| {
            resource.owner == *actor && resource.resource.as_str() == "dexterity"
        });
        let resource = resources
            .next()
            .ok_or(EncounterStartError::InvalidDexterity)?;
        if resources.next().is_some() {
            return Err(EncounterStartError::InvalidDexterity);
        }
        let mut constraints = context.inventory.resources.iter().filter(|constraint| {
            constraint.owner == *actor && constraint.resource == resource.resource
        });
        let constraint = constraints
            .next()
            .ok_or(EncounterStartError::InvalidDexterity)?;
        if constraints.next().is_some()
            || constraint.source != *source
            || resource.source != constraint.source
            || constraint.minimum != 1
            || constraint.maximum != 20
            || resource.minimum != constraint.minimum
            || resource.maximum != constraint.maximum
            || !(constraint.minimum..=constraint.maximum).contains(&resource.value)
        {
            return Err(EncounterStartError::InvalidDexterity);
        }
        let dexterity =
            u8::try_from(resource.value).map_err(|_| EncounterStartError::InvalidDexterity)?;
        *roll_and_dexterity = (draw.value, dexterity);
    }
    // Keep initiative mechanics and the existing authored-order tie decision in df-rules.
    fn order<const N: usize>(
        initiative_inputs: &[(u32, u8)],
    ) -> Result<Vec<usize>, EncounterStartError> {
        let initiative_inputs = initiative_inputs
            .try_into()
            .map_err(|_| EncounterStartError::InvalidParticipants)?;
        initiative_order::<N>(initiative_inputs)
            .map(|indices| indices.to_vec())
            .map_err(EncounterStartError::Rules)
    }
    let indices = match initiative_inputs.len() {
        2 => order::<2>(&initiative_inputs),
        3 => order::<3>(&initiative_inputs),
        4 => order::<4>(&initiative_inputs),
        5 => order::<5>(&initiative_inputs),
        6 => order::<6>(&initiative_inputs),
        7 => order::<7>(&initiative_inputs),
        _ => return Err(EncounterStartError::InvalidParticipants),
    }?;
    Ok(indices.into_iter().map(|index| actors[index]).collect())
}

use df_encounter::challenge::{
    ChallengeBudget, ChallengeMode, EncounterDirector, EncounterError, EncounterPlan,
    EncounterSourceOwner,
};
use df_rules::{InvocationError, RulesCommandHandler, stage_handler};

/// Current source authority for the exact noncombat application, not a new mechanics owner.
///
/// Admission checks actor control, the command/template mapping and source-use rights before
/// mechanics run. Candidate admission checks the source-defined encounter, objectives, escape
/// and consequences, including explicit pending outcomes. Both callbacks are pure and bounded
/// by the admitted challenge and checkpoint limits. A mode or reference label grants nothing.
pub trait NoncombatSourceOwner: EncounterSourceOwner {
    fn admit_command_application(
        &self,
        current: &Checkpoint,
        plan: &EncounterPlan,
        input: RulesCommandInput<'_>,
    ) -> Result<(), Self::Error>;

    fn admit_candidate_application(
        &self,
        current: &Checkpoint,
        plan: &EncounterPlan,
        input: RulesCommandInput<'_>,
        candidate: &Checkpoint,
    ) -> Result<(), Self::Error>;
}

#[derive(Debug, Eq, PartialEq)]
pub enum NoncombatStagingError<OwnerError, MechanicsError> {
    UnsupportedMode,
    SourceMismatch,
    Encounter(EncounterError<OwnerError>),
    Command(CommandError),
    Source(OwnerError),
    Mechanics(InvocationError<MechanicsError>),
}

/// Register this wrapper on the exact source clause inside the existing session engine.
///
/// The director's plan is revalidated against current source/participant authority. Only the
/// supplied compiled rules handler determines costs, draws, pending windows and outcomes.
/// Canonical staging validates its complete candidate before source application admission.
/// Refusal returns no candidate; this wrapper performs no commit, publication, I/O or draw.
/// Missing production source catalogs/handlers remain explicit gaps, without a generic DC.
pub struct NoncombatEncounterHandler<'a, Owner, Handler> {
    pub owner: &'a Owner,
    pub plan: &'a EncounterPlan,
    pub mechanics: &'a Handler,
    pub context: CommandEntryContext<'a>,
    pub budget: ChallengeBudget,
}

impl<Owner: NoncombatSourceOwner, Handler: RulesCommandHandler> RulesCommandHandler
    for NoncombatEncounterHandler<'_, Owner, Handler>
{
    type Rejection = NoncombatStagingError<Owner::Error, Handler::Rejection>;

    fn pins(&self) -> &df_model::checkpoint::CheckpointPins {
        self.context.admitted_pins
    }

    fn bound_source(&self) -> Option<&RuleReference> {
        Some(&self.plan.definition.source)
    }

    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, Self::Rejection> {
        if self.plan.mode == ChallengeMode::Combat {
            return Err(NoncombatStagingError::UnsupportedMode);
        }
        EncounterDirector::validate(
            self.owner,
            current,
            self.plan,
            self.context.current_basis,
            self.context.admitted_pins,
            self.context.inventory,
            self.budget,
        )
        .map_err(NoncombatStagingError::Encounter)?;
        validate_client_command(
            input.command,
            current,
            self.context.inventory,
            self.context.limits.command,
        )
        .map_err(NoncombatStagingError::Command)?;
        if self.plan.definition.source.catalog != self.context.admitted_pins.rules.catalog
            || self.mechanics.bound_source() != Some(&self.plan.definition.source)
        {
            return Err(NoncombatStagingError::SourceMismatch);
        }
        self.owner
            .admit_command_application(current, self.plan, input)
            .map_err(NoncombatStagingError::Source)?;
        let candidate = stage_handler(
            self.mechanics,
            self.context.admitted_pins,
            input,
            current,
            self.context.limits.maximum_staged_bytes,
        )
        .map_err(NoncombatStagingError::Mechanics)?;
        self.owner
            .admit_candidate_application(current, self.plan, input, &candidate)
            .map_err(NoncombatStagingError::Source)?;
        Ok(candidate)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "../tests/support/fixture_model.rs"]
mod noncombat_fixture;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod noncombat_tests {
    use super::noncombat_fixture as fixture;
    use super::*;
    use crate::command_entry::{CommandEntryLimits, decide_registered_transition};
    use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
    use df_encounter::challenge::{
        AuthoredChallengeReference, ChallengeReferenceKind, EncounterRequest, SourceAdmission,
    };
    use df_encounter::participants::{ParticipantOwner, ParticipantStatus};
    use df_model::checkpoint::*;
    use df_model::commands::CommandLimits;
    use df_model::transition::TransitionResult;
    use df_rules::{DispatchError, DispatchRegistry, HandlerRegistration};
    use df_session::inbox::ActorInput;
    use df_session::submission::{OperationScope, RepositoryError, SessionEngine};
    use df_types::{OperationId, SessionId};
    use std::cell::Cell;

    // Explicit compiled fixtures exercise admission/staging, not standard source coverage.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Refusal {
        Withdrawn,
        WrongCommand,
        CandidateMismatch,
        UnsupportedMechanics,
    }

    struct Source {
        mode: ChallengeMode,
        status: Cell<SourceAdmission>,
        participant: Cell<ParticipantStatus>,
        deny_command: Cell<bool>,
        admitted_candidate: Checkpoint,
        command_calls: Cell<usize>,
        candidate_calls: Cell<usize>,
    }

    impl ParticipantOwner for Source {
        type Actor = EntityId;
        type Basis = Basis;
        type Error = Refusal;
        fn validate_basis(&self, proposed: &Basis, current: &Basis) -> Result<(), Refusal> {
            if proposed != current {
                return Err(Refusal::Withdrawn);
            }
            Ok(())
        }
        fn participant_status(
            &self,
            _: &Basis,
            actor: &EntityId,
        ) -> Result<ParticipantStatus, Refusal> {
            Ok(if *actor == fixture::entity(4) {
                self.participant.get()
            } else {
                ParticipantStatus::Unknown
            })
        }
    }
    impl EncounterSourceOwner for Source {
        fn admit_reference(
            &self,
            _: &Checkpoint,
            mode: ChallengeMode,
            _: ChallengeReferenceKind,
            reference: &AuthoredChallengeReference,
        ) -> Result<SourceAdmission, Refusal> {
            Ok(if mode != self.mode || *reference != authored() {
                SourceAdmission::Unsupported
            } else {
                self.status.get()
            })
        }
    }
    impl NoncombatSourceOwner for Source {
        fn admit_command_application(
            &self,
            _: &Checkpoint,
            _: &EncounterPlan,
            input: RulesCommandInput<'_>,
        ) -> Result<(), Refusal> {
            self.command_calls.set(self.command_calls.get() + 1);
            if self.deny_command.get() || *input.command != fixture::action() {
                return Err(Refusal::WrongCommand);
            }
            Ok(())
        }
        fn admit_candidate_application(
            &self,
            _: &Checkpoint,
            plan: &EncounterPlan,
            _: RulesCommandInput<'_>,
            candidate: &Checkpoint,
        ) -> Result<(), Refusal> {
            self.candidate_calls.set(self.candidate_calls.get() + 1);
            if *candidate != self.admitted_candidate
                || plan.definition != authored()
                || plan.objectives.len() != 1
                || plan.objectives[0].authored != authored()
                || plan.consequences.len() != 1
                || plan.consequences[0].authored != authored()
            {
                return Err(Refusal::CandidateMismatch);
            }
            Ok(())
        }
    }

    struct Mechanics {
        candidate: Checkpoint,
        source: Option<RuleReference>,
        calls: Cell<usize>,
        input_pointer: Cell<*const GameInput>,
        draws_pointer: Cell<*const ActualDraw>,
        refusal: bool,
    }
    impl RulesCommandHandler for Mechanics {
        type Rejection = Refusal;
        fn pins(&self) -> &CheckpointPins {
            self.candidate.pins()
        }
        fn bound_source(&self) -> Option<&RuleReference> {
            self.source.as_ref()
        }
        fn stage(
            &self,
            input: RulesCommandInput<'_>,
            _: &Checkpoint,
        ) -> Result<Checkpoint, Refusal> {
            self.calls.set(self.calls.get() + 1);
            self.input_pointer.set(input.command as *const GameInput);
            self.draws_pointer.set(input.supplied_draws.as_ptr());
            if self.refusal {
                return Err(Refusal::UnsupportedMechanics);
            }
            Ok(self.candidate.clone())
        }
    }

    struct Fixture {
        current: Checkpoint,
        pins: CheckpointPins,
        references: Vec<ContentReference>,
        rules: Vec<RuleReference>,
        resources: Vec<ResourceConstraint>,
    }
    impl Fixture {
        fn new() -> Self {
            Self {
                current: fixture::checkpoint(fixture::state()).unwrap(),
                pins: fixture::pins(),
                references: vec![fixture::content()],
                rules: vec![fixture::rule()],
                resources: fixture::resource_constraints(),
            }
        }
        fn inventory(&self) -> ReferenceInventory<'_> {
            ReferenceInventory {
                rules: &self.rules,
                content: &self.references,
                resources: &self.resources,
                assets: &[],
            }
        }
        fn context(&self) -> CommandEntryContext<'_> {
            CommandEntryContext {
                current_basis: self.current.basis(),
                admitted_pins: &self.pins,
                inventory: self.inventory(),
                limits: CommandEntryLimits {
                    command: CommandLimits {
                        maximum_records: 32,
                        maximum_text_bytes: 256,
                        maximum_retained_bytes: 64 * 1024,
                    },
                    maximum_staged_bytes: 1024 * 1024,
                },
            }
        }
    }
    fn authored() -> AuthoredChallengeReference {
        AuthoredChallengeReference {
            definition: fixture::content(),
            source: fixture::rule(),
        }
    }
    fn budget() -> ChallengeBudget {
        ChallengeBudget {
            max_participants: 4,
            max_objectives: 4,
            max_escape_references: 1,
            max_consequence_references: 4,
            max_comparisons: 128,
            max_input_bytes: 64 * 1024,
            max_output_bytes: 64 * 1024,
        }
    }
    fn source(mode: ChallengeMode, candidate: &Checkpoint) -> Source {
        Source {
            mode,
            status: Cell::new(SourceAdmission::Admitted),
            participant: Cell::new(ParticipantStatus::Available),
            deny_command: Cell::new(false),
            admitted_candidate: candidate.clone(),
            command_calls: Cell::new(0),
            candidate_calls: Cell::new(0),
        }
    }
    fn mechanics(candidate: &Checkpoint) -> Mechanics {
        Mechanics {
            candidate: candidate.clone(),
            source: Some(fixture::rule()),
            calls: Cell::new(0),
            input_pointer: Cell::new(std::ptr::null()),
            draws_pointer: Cell::new(std::ptr::null()),
            refusal: false,
        }
    }
    fn plan(f: &Fixture, owner: &Source) -> EncounterPlan {
        let definition = authored();
        let participants = [fixture::entity(4)];
        let objectives = [authored()];
        let consequences = [authored()];
        EncounterDirector::propose(
            owner,
            EncounterRequest {
                current: &f.current,
                expected_basis: f.current.basis(),
                admitted_pins: &f.pins,
                inventory: f.inventory(),
                mode: owner.mode,
                definition: &definition,
                participants: &participants,
                objectives: &objectives,
                escape_condition: Some(&definition),
                consequences: &consequences,
                budget: budget(),
            },
        )
        .unwrap()
    }
    fn candidate() -> Checkpoint {
        let mut state = fixture::state();
        state.resources[0].value = 3;
        state.encounters.push(EncounterState {
            id: RecordId::from_bytes(&[71; 16]).unwrap(),
            definition: fixture::content(),
            participants: vec![fixture::entity(4)],
            turn_order: vec![],
            active_turn: None,
            objectives: vec![fixture::content()],
            combat_policy: fixture::content(),
            tactical: None,
        });
        fixture::accepted_from(state)
    }
    fn registered(
        f: &Fixture,
        owner: &Source,
        plan: &EncounterPlan,
        mechanics: &Mechanics,
        input: RulesCommandInput<'_>,
    ) -> TransitionResult<
        crate::command_entry::CommandRejection<NoncombatStagingError<Refusal, Refusal>>,
    > {
        let handler = NoncombatEncounterHandler {
            owner,
            plan,
            mechanics,
            context: f.context(),
            budget: budget(),
        };
        let selector = fixture::label("noncombat-fixture-handler");
        let source = fixture::rule();
        let entries = [CatalogEntry::new(&source, b"synthetic compiled fixture")];
        let catalog = CatalogSnapshot::from_published(
            &f.pins.rules.catalog,
            &f.pins,
            b"synthetic complete source",
            &entries,
            CatalogLimits {
                max_complete_bytes: 128,
                max_entries: 4,
                max_item_bytes: 128,
                max_total_item_bytes: 512,
            },
        )
        .unwrap();
        let registrations = [HandlerRegistration::new(&selector, &source, &handler)];
        let registry = DispatchRegistry::from_catalog(catalog, &registrations, 4).unwrap();
        decide_registered_transition(
            input,
            &f.current,
            f.context(),
            &registry,
            &selector,
            &source,
        )
    }

    #[test]
    fn all_noncombat_modes_stage_exact_source_mechanics_through_registered_boundary() {
        for mode in [
            ChallengeMode::Social,
            ChallengeMode::Chase,
            ChallengeMode::Hazard,
            ChallengeMode::Puzzle,
            ChallengeMode::Survival,
        ] {
            let f = Fixture::new();
            let expected = candidate();
            let owner = source(mode, &expected);
            let plan = plan(&f, &owner);
            let mechanics = mechanics(&expected);
            let command = fixture::action();
            let draws = [];
            let actual = registered(
                &f,
                &owner,
                &plan,
                &mechanics,
                RulesCommandInput {
                    command: &command,
                    supplied_draws: &draws,
                },
            );
            assert!(matches!(actual, TransitionResult::Accepted(ref c) if c == &expected));
            assert_eq!(mechanics.calls.get(), 1);
            assert_eq!(mechanics.input_pointer.get(), &command as *const GameInput);
            assert_eq!(mechanics.draws_pointer.get(), draws.as_ptr());
            assert_eq!(owner.command_calls.get(), 1);
            assert_eq!(owner.candidate_calls.get(), 1);
            assert_eq!(f.current.state(), &fixture::state());
        }
    }

    #[test]
    fn withdrawn_sources_and_unavailable_participants_never_invoke_mechanics() {
        let f = Fixture::new();
        let expected = candidate();
        let owner = source(ChallengeMode::Social, &expected);
        let plan = plan(&f, &owner);
        let mechanics = mechanics(&expected);
        let command = fixture::action();
        for status in [
            SourceAdmission::Denied,
            SourceAdmission::Unavailable,
            SourceAdmission::Unsupported,
        ] {
            owner.status.set(status);
            assert!(matches!(registered(&f,&owner,&plan,&mechanics,
                RulesCommandInput { command: &command, supplied_draws: &[] }),
                TransitionResult::Rejected(crate::command_entry::CommandRejection::Invocation(
                    InvocationError::Handler(NoncombatStagingError::Encounter(
                        EncounterError::SourceGap(gap))))) if gap.reason == status));
        }
        owner.status.set(SourceAdmission::Admitted);
        for status in [
            ParticipantStatus::Unknown,
            ParticipantStatus::Unavailable,
            ParticipantStatus::Undisclosed,
        ] {
            owner.participant.set(status);
            assert!(matches!(
                registered(
                    &f,
                    &owner,
                    &plan,
                    &mechanics,
                    RulesCommandInput {
                        command: &command,
                        supplied_draws: &[]
                    }
                ),
                TransitionResult::Rejected(_)
            ));
        }
        assert_eq!(mechanics.calls.get(), 0);
        assert_eq!(owner.command_calls.get(), 0);
        assert_eq!(owner.candidate_calls.get(), 0);
        assert_eq!(f.current.state(), &fixture::state());
    }

    #[test]
    fn stale_plan_pins_mode_and_missing_bound_source_refuse_before_mechanics() {
        let f = Fixture::new();
        let expected = candidate();
        let owner = source(ChallengeMode::Social, &expected);
        let original = plan(&f, &owner);
        let mechanics = mechanics(&expected);
        let command = fixture::action();
        let mut changed = original.clone();
        changed.basis.revision = changed.basis.revision.next_sequence().unwrap();
        assert!(matches!(
            registered(
                &f,
                &owner,
                &changed,
                &mechanics,
                RulesCommandInput {
                    command: &command,
                    supplied_draws: &[]
                }
            ),
            TransitionResult::Rejected(_)
        ));
        changed = original.clone();
        changed.pins.content.content = fixture::label("foreign-content");
        assert!(matches!(
            registered(
                &f,
                &owner,
                &changed,
                &mechanics,
                RulesCommandInput {
                    command: &command,
                    supplied_draws: &[]
                }
            ),
            TransitionResult::Rejected(_)
        ));
        changed = original.clone();
        changed.mode = ChallengeMode::Combat;
        assert!(matches!(
            registered(
                &f,
                &owner,
                &changed,
                &mechanics,
                RulesCommandInput {
                    command: &command,
                    supplied_draws: &[]
                }
            ),
            TransitionResult::Rejected(crate::command_entry::CommandRejection::Invocation(
                InvocationError::Handler(NoncombatStagingError::UnsupportedMode)
            ))
        ));
        let mut unbound = mechanics;
        unbound.source = None;
        assert!(matches!(
            registered(
                &f,
                &owner,
                &original,
                &unbound,
                RulesCommandInput {
                    command: &command,
                    supplied_draws: &[]
                }
            ),
            TransitionResult::Rejected(crate::command_entry::CommandRejection::Invocation(
                InvocationError::Handler(NoncombatStagingError::SourceMismatch)
            ))
        ));
        assert_eq!(unbound.calls.get(), 0);
    }

    #[test]
    fn command_and_candidate_admission_refusals_are_atomic() {
        let f = Fixture::new();
        let expected = candidate();
        let owner = source(ChallengeMode::Hazard, &expected);
        let original = plan(&f, &owner);
        let mechanics = mechanics(&expected);
        let command = fixture::action();
        owner.deny_command.set(true);
        assert!(matches!(
            registered(
                &f,
                &owner,
                &original,
                &mechanics,
                RulesCommandInput {
                    command: &command,
                    supplied_draws: &[]
                }
            ),
            TransitionResult::Rejected(crate::command_entry::CommandRejection::Invocation(
                InvocationError::Handler(NoncombatStagingError::Source(Refusal::WrongCommand))
            ))
        ));
        assert_eq!(mechanics.calls.get(), 0);
        owner.deny_command.set(false);
        let mut changed = original.clone();
        changed.consequences.clear();
        assert!(matches!(
            registered(
                &f,
                &owner,
                &changed,
                &mechanics,
                RulesCommandInput {
                    command: &command,
                    supplied_draws: &[]
                }
            ),
            TransitionResult::Rejected(crate::command_entry::CommandRejection::Invocation(
                InvocationError::Handler(NoncombatStagingError::Source(Refusal::CandidateMismatch))
            ))
        ));
        assert_eq!(mechanics.calls.get(), 1);
        assert_eq!(owner.candidate_calls.get(), 1);
        assert_eq!(f.current.state(), &fixture::state());
        changed = original.clone();
        changed.objectives.clear();
        assert!(matches!(
            registered(
                &f,
                &owner,
                &changed,
                &mechanics,
                RulesCommandInput {
                    command: &command,
                    supplied_draws: &[]
                }
            ),
            TransitionResult::Rejected(_)
        ));
        assert_eq!(f.current.state(), &fixture::state());
    }

    #[test]
    fn canonical_candidate_failures_and_capacity_preserve_original_checkpoint() {
        let f = Fixture::new();
        let expected = candidate();
        let owner = source(ChallengeMode::Puzzle, &expected);
        let plan = plan(&f, &owner);
        let mut mechanics = mechanics(&fixture::checkpoint(fixture::state()).unwrap());
        let command = fixture::action();
        assert!(matches!(
            registered(
                &f,
                &owner,
                &plan,
                &mechanics,
                RulesCommandInput {
                    command: &command,
                    supplied_draws: &[]
                }
            ),
            TransitionResult::Rejected(crate::command_entry::CommandRejection::Invocation(
                InvocationError::Handler(NoncombatStagingError::Mechanics(
                    InvocationError::CandidateBasisMismatch
                ))
            ))
        ));
        assert_eq!(owner.candidate_calls.get(), 0);
        mechanics.candidate = expected;
        let mut handler = NoncombatEncounterHandler {
            owner: &owner,
            plan: &plan,
            mechanics: &mechanics,
            context: f.context(),
            budget: budget(),
        };
        handler.context.limits.maximum_staged_bytes = 0;
        assert!(matches!(
            handler.stage(
                RulesCommandInput {
                    command: &command,
                    supplied_draws: &[]
                },
                &f.current
            ),
            Err(NoncombatStagingError::Mechanics(InvocationError::Capacity))
        ));
        handler.budget.max_comparisons = 0;
        assert!(matches!(
            handler.stage(
                RulesCommandInput {
                    command: &command,
                    supplied_draws: &[]
                },
                &f.current
            ),
            Err(NoncombatStagingError::Encounter(EncounterError::Capacity))
        ));
        assert_eq!(mechanics.calls.get(), 1);
        assert_eq!(f.current.state(), &fixture::state());
    }

    #[test]
    fn mechanics_rejection_and_draw_accounting_failures_are_not_success() {
        let f = Fixture::new();
        let expected = candidate();
        let owner = source(ChallengeMode::Chase, &expected);
        let plan = plan(&f, &owner);
        let mut mechanics = mechanics(&expected);
        let command = fixture::action();
        mechanics.refusal = true;
        assert!(matches!(
            registered(
                &f,
                &owner,
                &plan,
                &mechanics,
                RulesCommandInput {
                    command: &command,
                    supplied_draws: &[]
                }
            ),
            TransitionResult::Rejected(crate::command_entry::CommandRejection::Invocation(
                InvocationError::Handler(NoncombatStagingError::Mechanics(
                    InvocationError::Handler(Refusal::UnsupportedMechanics)
                ))
            ))
        ));
        mechanics.refusal = false;
        let draws = [ActualDraw {
            operation: fixture::operation(),
            ordinal: 0,
            resolution: ResolutionId::from_bytes(&[81; 16]).unwrap(),
            window: WindowId::from_bytes(&[82; 16]).unwrap(),
            source: fixture::rule(),
            sides: 20,
            value: 12,
        }];
        assert!(matches!(
            registered(
                &f,
                &owner,
                &plan,
                &mechanics,
                RulesCommandInput {
                    command: &command,
                    supplied_draws: &draws
                }
            ),
            TransitionResult::Rejected(crate::command_entry::CommandRejection::Invocation(
                InvocationError::Handler(NoncombatStagingError::Mechanics(
                    InvocationError::CandidateDrawMismatch
                ))
            ))
        ));
        assert_eq!(mechanics.draws_pointer.get(), draws.as_ptr());
        assert_eq!(owner.candidate_calls.get(), 0);
        assert_eq!(f.current.state(), &fixture::state());
    }

    fn pending_candidate() -> Checkpoint {
        let accepted = fixture::accepted();
        let mut state = accepted.state().clone();
        let fact = FactId::from_bytes(&[81; 16]).unwrap();
        let basis = accepted.basis();
        state.facts.push(GameFact {
            id: fact,
            revision: basis.revision,
            operation: fixture::operation(),
            ordinal: 0,
            cause: None,
            audience: AudienceScope::Shared,
            value: FactValue::ContentEvent {
                definition: fixture::content(),
                subjects: vec![],
            },
        });
        state.decisions[0].facts.push(fact);
        state.pending.push(PendingResolution {
            id: ResolutionId::from_bytes(&[82; 16]).unwrap(),
            basis,
            continuation: fixture::label("source-continuation"),
            window: ResolutionWindow {
                id: WindowId::from_bytes(&[83; 16]).unwrap(),
                phase: TriggerPhase::BeforeConsequence,
                causal_fact: fact,
                source: fixture::rule(),
                timer: None,
            },
            next: PendingInput::Choice {
                remaining: vec![OfferedResponse {
                    participant: fixture::member(3),
                    offer: fixture::label("source-offer"),
                    options: vec![fixture::label("source-option")],
                    source: fixture::rule(),
                }],
            },
            choices: vec![],
            draw_ordinals: vec![],
            spent: vec![],
            rulings: vec![],
        });
        Checkpoint::new(
            CHECKPOINT_SCHEMA,
            basis,
            fixture::pins(),
            state,
            ReferenceInventory {
                rules: &[fixture::rule()],
                content: &[fixture::content()],
                resources: &fixture::resource_constraints(),
                assets: &[],
            },
            fixture::limits(),
        )
        .unwrap()
    }

    #[derive(Clone, Eq, PartialEq)]
    struct Scope(GameInput);
    impl ActorInput for Scope {
        fn retained_heap_bytes(&self) -> Option<usize> {
            self.0.retained_heap_bytes()
        }
    }
    impl OperationScope for Scope {
        type UncertaintyKey = Self;
        fn capture_uncertainty_key(&self, limit: usize) -> Result<Self, RepositoryError> {
            if std::mem::size_of::<Self>()
                + self
                    .retained_heap_bytes()
                    .ok_or(RepositoryError::Capacity)?
                > limit
            {
                return Err(RepositoryError::Capacity);
            }
            Ok(self.clone())
        }
        fn session(&self) -> SessionId {
            fixture::basis().session
        }
        fn operation(&self) -> OperationId {
            fixture::operation()
        }
        fn validate_input(&self, input: &GameInput) -> Result<(), RepositoryError> {
            if *input == self.0 {
                Ok(())
            } else {
                Err(RepositoryError::InputBinding)
            }
        }
        fn is_lookup_only(&self) -> bool {
            false
        }
    }
    struct Engine {
        f: Fixture,
        owner: Source,
        plan: EncounterPlan,
        mechanics: Mechanics,
    }
    impl SessionEngine<Scope> for Engine {
        fn decide(
            &mut self,
            current: &Checkpoint,
            scope: &Scope,
            input: &GameInput,
        ) -> Result<Checkpoint, RepositoryError> {
            scope.validate_input(input)?;
            if *current != self.f.current {
                return Err(RepositoryError::RevisionConflict);
            }
            match registered(
                &self.f,
                &self.owner,
                &self.plan,
                &self.mechanics,
                RulesCommandInput {
                    command: input,
                    supplied_draws: &[],
                },
            ) {
                TransitionResult::Accepted(candidate) => Ok(candidate),
                TransitionResult::Pending { candidate, .. } => Ok(candidate),
                TransitionResult::Rejected(_) => Err(RepositoryError::InvalidCandidate),
            }
        }
        fn validate_recovery(&mut self, current: &Checkpoint) -> Result<(), RepositoryError> {
            current
                .validate_resume(current.basis(), &self.f.pins)
                .map(|_| ())
                .map_err(|_| RepositoryError::InvalidCandidate)
        }
    }

    #[test]
    fn actual_session_engine_preserves_source_pending_and_rejects_repeated_operation() {
        let f = Fixture::new();
        let expected = pending_candidate();
        let owner = source(ChallengeMode::Social, &expected);
        let plan = plan(&f, &owner);
        let mechanics = mechanics(&expected);
        let current = f.current.clone();
        let mut engine = Engine {
            f,
            owner,
            plan,
            mechanics,
        };
        let input = fixture::action();
        let scope = Scope(input.clone());
        assert!(matches!(engine.decide_transition(&current,&scope,&input),
            TransitionResult::Pending { ref candidate, .. } if candidate==&expected));
        assert_eq!(engine.mechanics.calls.get(), 1);
        // The session's durable operation lookup owns retry; direct restaging is refused.
        let handler = NoncombatEncounterHandler {
            owner: &engine.owner,
            plan: &engine.plan,
            mechanics: &engine.mechanics,
            context: engine.f.context(),
            budget: budget(),
        };
        assert!(matches!(
            stage_handler(
                &handler,
                &engine.f.pins,
                RulesCommandInput {
                    command: &input,
                    supplied_draws: &[]
                },
                &expected,
                1024 * 1024
            ),
            Err(InvocationError::AlreadyAccepted)
        ));
        assert_eq!(engine.mechanics.calls.get(), 1);
        assert_eq!(current.state(), &fixture::state());
    }

    #[test]
    fn missing_registered_handler_remains_explicit_gap() {
        let f = Fixture::new();
        let expected = candidate();
        let owner = source(ChallengeMode::Survival, &expected);
        let plan = plan(&f, &owner);
        let mechanics = mechanics(&expected);
        let handler = NoncombatEncounterHandler {
            owner: &owner,
            plan: &plan,
            mechanics: &mechanics,
            context: f.context(),
            budget: budget(),
        };
        let source = fixture::rule();
        let selector = fixture::label("missing");
        let entries = [CatalogEntry::new(&source, b"synthetic")];
        let catalog = CatalogSnapshot::from_published(
            &f.pins.rules.catalog,
            &f.pins,
            b"synthetic",
            &entries,
            CatalogLimits {
                max_complete_bytes: 64,
                max_entries: 1,
                max_item_bytes: 64,
                max_total_item_bytes: 64,
            },
        )
        .unwrap();
        let registrations: [HandlerRegistration<'_, _>; 0] = [];
        let registry =
            DispatchRegistry::<NoncombatEncounterHandler<'_, Source, Mechanics>>::from_catalog(
                catalog,
                &registrations,
                1,
            )
            .unwrap();
        let command = fixture::action();
        assert!(matches!(
            decide_registered_transition(
                RulesCommandInput {
                    command: &command,
                    supplied_draws: &[]
                },
                &f.current,
                f.context(),
                &registry,
                &selector,
                &source
            ),
            TransitionResult::Rejected(crate::command_entry::CommandRejection::Invocation(
                InvocationError::Dispatch(DispatchError::UnknownHandler)
            ))
        ));
        assert_eq!(handler.mechanics.calls.get(), 0);
        assert_eq!(owner.command_calls.get(), 0);
    }
}
