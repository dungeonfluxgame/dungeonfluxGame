use super::*;
use df_model::checkpoint::*;
use df_types::{
    BuildIdentity, MemberId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId,
    SessionRevision,
};

fn label(value: &str) -> RevisionLabel {
    RevisionLabel::new(Some(value)).unwrap()
}
fn entity(value: u8) -> EntityId {
    EntityId::from_bytes(&[value; 16]).unwrap()
}
fn member(value: u8) -> MemberId {
    MemberId::from_bytes(&[value; 16]).unwrap()
}
fn revision(epoch: u64, sequence: u64) -> SessionRevision {
    SessionRevision::new(RecoveryEpoch::new(epoch).unwrap(), sequence)
}
fn basis() -> Basis {
    Basis {
        session: SessionId::from_bytes(&[1; 16]).unwrap(),
        run: RunId::from_bytes(&[2; 16]).unwrap(),
        revision: revision(2, 8),
    }
}
fn content() -> ContentReference {
    ContentReference {
        package: label("fixture-package-1"),
        entry: label("fixture-entry-1"),
    }
}
fn alternate_content() -> ContentReference {
    ContentReference {
        package: content().package,
        entry: label("fixture-entry-2"),
    }
}
fn rule() -> RuleReference {
    RuleReference {
        catalog: label("fixture-catalog-1"),
        source: label("fixture-source-1"),
        entry: label("fixture-entry-1"),
        clause: label("fixture-clause-1"),
    }
}
fn pins() -> CheckpointPins {
    CheckpointPins {
        rules: RulesPins {
            mode: RulesMode::Standard2024,
            ruleset: label("fixture-rules-1"),
            catalog: label("fixture-catalog-1"),
            catalog_digest: ContentDigest([1; 32]),
            source_manifest: label("fixture-sources-1"),
            source_manifest_digest: ContentDigest([2; 32]),
            handler: label("fixture-handler-1"),
            handler_digest: ContentDigest([3; 32]),
        },
        content: ContentPins {
            content: label("fixture-content-1"),
            content_digest: ContentDigest([4; 32]),
            package: label("fixture-package-1"),
            package_digest: ContentDigest([5; 32]),
        },
        build: BuildIdentity::new(
            Some("fixture-source-1"),
            Some("fixture-native-1"),
            Some("fixture-wasm-1"),
            Some("fixture-config-1"),
            Some("fixture-content-1"),
        )
        .unwrap(),
    }
}
fn resource_constraints() -> Vec<ResourceConstraint> {
    ["fixture-resource-1", "fixture-resource-2"]
        .into_iter()
        .map(|resource| ResourceConstraint {
            owner: entity(4),
            resource: label(resource),
            minimum: 0,
            maximum: 8,
            source: rule(),
        })
        .collect()
}
fn limits() -> CheckpointLimits {
    CheckpointLimits {
        maximum_records: 100,
        maximum_text_bytes: 256,
        maximum_total_text_bytes: 1024,
        maximum_retained_bytes: 1024 * 1024,
    }
}
fn state() -> GameState {
    GameState {
        mode: ExecutionMode::Replay,
        logical_time: LogicalTime {
            ticks: 120,
            ticks_per_second: 10,
        },
        members: vec![MembershipLink {
            member: member(3),
            character: Some(entity(4)),
        }],
        entities: vec![WorldEntity {
            id: entity(4),
            definition: content(),
            location: None,
            position: Some(Position { x: 0, y: 0, z: 0 }),
            identity_revision: label("fixture-entity-1"),
        }],
        characters: vec![CharacterState {
            entity: entity(4),
            build: content(),
            owner: member(3),
            choices: vec![],
        }],
        resources: vec![ResourceState {
            owner: entity(4),
            resource: label("fixture-resource-1"),
            value: 4,
            minimum: 0,
            maximum: 8,
            source: rule(),
        }],
        inventory: vec![],
        facts: vec![],
        draws: vec![],
        decisions: vec![],
        pending: vec![],
        intents: vec![],
        timers: vec![],
        active_effects: vec![],
        knowledge: vec![],
        beliefs: vec![],
        memories: vec![],
        schedules: vec![],
        threats: vec![],
        relationships: vec![],
        conversations: vec![],
        obligations: vec![],
        narrative: NarrativeState {
            definition: content(),
            active_beats: vec![],
            completed_beats: vec![],
            open_threads: vec![],
            accepted_facts: vec![],
            remaining_budget: 0,
        },
        encounters: vec![],
        activity: vec![],
        tempo: TempoState {
            policy: content(),
            presentation_ticks: 0,
            intensity: 0,
            inertia: 0,
            fatigue: vec![],
        },
        presentation: vec![],
        continuity: continuity(),
    }
}
fn checkpoint(state: GameState) -> Result<Checkpoint, CheckpointError> {
    let rules = vec![rule()];
    let content_entries = vec![content(), alternate_content()];
    let resource_constraints = resource_constraints();
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis(),
        pins(),
        state,
        ReferenceInventory {
            rules: &rules,
            content: &content_entries,
            resources: &resource_constraints,
            assets: &[],
        },
        limits(),
    )
}
fn fact(value: u8, ordinal: u32) -> GameFact {
    GameFact {
        id: FactId::from_bytes(&[value; 16]).unwrap(),
        revision: basis().revision,
        operation: OperationId::from_bytes(&[6; 16]).unwrap(),
        ordinal,
        cause: None,
        audience: AudienceScope::Shared,
        value: FactValue::ContentEvent {
            definition: content(),
            subjects: vec![entity(4)],
        },
    }
}

fn pending() -> PendingResolution {
    PendingResolution {
        id: ResolutionId::from_bytes(&[9; 16]).unwrap(),
        basis: basis(),
        continuation: label("fixture-continuation-position-1"),
        window: ResolutionWindow {
            id: WindowId::from_bytes(&[10; 16]).unwrap(),
            phase: TriggerPhase::BeforeDraw,
            causal_fact: FactId::from_bytes(&[7; 16]).unwrap(),
            source: rule(),
            timer: None,
        },
        next: PendingInput::Reaction {
            remaining: vec![OfferedResponse {
                participant: member(3),
                offer: label("fixture-offer-1"),
                options: vec![label("fixture-option-1")],
                source: rule(),
            }],
        },
        choices: vec![],
        draw_ordinals: vec![],
        spent: vec![ResourceSpend {
            owner: entity(4),
            resource: label("fixture-resource-1"),
            amount: 1,
            source: rule(),
        }],
        rulings: vec![],
    }
}
fn continuity() -> ContinuityState {
    ContinuityState {
        creation: vec![],
        simulation: vec![],
        catch_up: None,
        environment: vec![],
        travel: vec![],
        witnesses: vec![],
        rumors: vec![],
        journal: vec![],
        summaries: vec![],
        retrieval: vec![],
        retrieved: vec![],
        consolidation: vec![],
        npcs: vec![],
        hooks: vec![],
        arcs: vec![],
        remote: None,
        presence: vec![],
        audio: None,
        private_offers: vec![],
        knowledge_cues: vec![],
        moments: vec![],
        demands: vec![],
        asset_jobs: vec![],
        asset_dependencies: vec![],
        canonical_packs: vec![],
        shots: vec![],
        prefetch: None,
        scenes: vec![],
        item_origins: vec![],
        bookends: vec![],
        exports: vec![],
        critical_cues: vec![],
        content_candidates: vec![],
        content_admissions: vec![],
        recovery: RecoveryState {
            origin: None,
            retired_epochs: vec![],
            lost_ranges: vec![],
            suppression_generation: 0,
            redacted_records: vec![],
            unavailable_sources: vec![],
        },
    }
}

use crate::command_handler::InvocationError;
use crate::dispatch::{DispatchRegistry, HandlerRegistration};
use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use std::cell::Cell;

fn input() -> GameInput {
    GameInput::Game(CommandInput {
        basis: basis(),
        observed_revision: basis().revision,
        operation: OperationId::from_bytes(&[20; 16]).unwrap(),
        member: member(3),
        command: GameCommand::ProposeAction {
            actor: entity(4),
            action: content(),
            targets: vec![],
            choices: vec![],
        },
    })
}
fn response_input() -> GameInput {
    let pending = pending();
    let mut input = input();
    if let GameInput::Game(command) = &mut input {
        command.command = GameCommand::SelectReaction {
            resolution: pending.id,
            window: pending.window.id,
            offer: label("fixture-offer-1"),
            option: label("fixture-option-1"),
        };
    }
    input
}
fn resource_dependency() -> RuleDependency {
    RuleDependency::Resource {
        owner: entity(4),
        resource: label("fixture-resource-1"),
    }
}
fn full_dependencies() -> Vec<RuleDependency> {
    vec![
        resource_dependency(),
        RuleDependency::LogicalTime,
        RuleDependency::ActiveEffects,
        RuleDependency::PendingResolutions,
        RuleDependency::Timers,
    ]
}
fn bounds() -> PreconditionLimits {
    PreconditionLimits {
        maximum_dependencies: 8,
        maximum_comparisons: 1_000_000,
        maximum_checkpoint_bytes: 1024 * 1024,
    }
}
fn command_limits() -> CommandLimits {
    CommandLimits {
        maximum_records: 8,
        maximum_text_bytes: 64,
        maximum_retained_bytes: 8192,
    }
}
fn check(prepared: &Checkpoint, current: &Checkpoint) -> Result<(), PreconditionError> {
    check_input(prepared, current, &input(), &full_dependencies())
}
fn check_input(
    prepared: &Checkpoint,
    current: &Checkpoint,
    input: &GameInput,
    dependencies: &[RuleDependency],
) -> Result<(), PreconditionError> {
    check_boundary(prepared, current, input, dependencies, |_, _| {})
}
fn check_boundary(
    prepared: &Checkpoint,
    current: &Checkpoint,
    input: &GameInput,
    dependencies: &[RuleDependency],
    configure: impl FnOnce(&mut CurrentRuleContext<'_>, &mut PreconditionLimits),
) -> Result<(), PreconditionError> {
    let sources = [rule()];
    let content_entries = [content()];
    let constraints = resource_constraints();
    let preconditions = RulePreconditions {
        prepared,
        sources: &sources,
        dependencies,
    };
    let mut context = CurrentRuleContext {
        checkpoint: current,
        basis: current.basis(),
        pins: current.pins(),
        inventory: ReferenceInventory {
            rules: &sources,
            content: &content_entries,
            resources: &constraints,
            assets: &[],
        },
        command_limits: command_limits(),
    };
    let mut limits = bounds();
    configure(&mut context, &mut limits);
    validate_rule_preconditions(input, context, &preconditions, limits)
}
fn checkpoint_at(mut state: GameState, basis: Basis, pins: CheckpointPins) -> Checkpoint {
    for pending in &mut state.pending {
        pending.basis = basis;
    }
    let rules = [rule()];
    let content_entries = [content(), alternate_content()];
    let constraints = resource_constraints();
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis,
        pins,
        state,
        ReferenceInventory {
            rules: &rules,
            content: &content_entries,
            resources: &constraints,
            assets: &[],
        },
        limits(),
    )
    .unwrap()
}
fn with_pending() -> GameState {
    let mut state = state();
    state.facts.push(fact(7, 0));
    state.pending.push(pending());
    state
}
fn with_effect() -> GameState {
    let mut state = state();
    state.facts.push(fact(7, 0));
    state.active_effects.push(ActiveEffect {
        id: RecordId::from_bytes(&[8; 16]).unwrap(),
        source: rule(),
        origin: FactId::from_bytes(&[7; 16]).unwrap(),
        source_entity: Some(entity(4)),
        targets: vec![entity(4)],
        starts: state.logical_time,
        expires: Some(LogicalTime {
            ticks: 150,
            ticks_per_second: 10,
        }),
        concentration_owner: None,
        choices: vec![],
    });
    state
}
fn with_timer() -> GameState {
    let mut state = state();
    state.timers.push(OwnedTimer {
        id: TimerId::from_bytes(&[8; 16]).unwrap(),
        basis: basis(),
        generation: 1,
        due: LogicalTime {
            ticks: 150,
            ticks_per_second: 10,
        },
        source: rule(),
        status: DurableStatus::Pending,
    });
    state
}
fn with_second_resource() -> GameState {
    let mut state = state();
    state.resources.push(ResourceState {
        owner: entity(4),
        resource: label("fixture-resource-2"),
        value: 5,
        minimum: 0,
        maximum: 8,
        source: rule(),
    });
    state
}

#[test]
fn exact_dependencies_accept_without_changing_command_or_checkpoint() {
    let current = checkpoint(state()).unwrap();
    let before = current.clone();
    let command = input();
    let original = command.clone();
    assert_eq!(
        check_input(&current, &current, &command, &full_dependencies()),
        Ok(())
    );
    assert_eq!(
        check_input(&current, &current, &command, &full_dependencies()),
        Ok(())
    );
    assert_eq!(current, before);
    assert_eq!(command, original);
}
#[test]
fn changed_selected_resource_rejects_and_changed_unselected_resource_survives() {
    let prepared = checkpoint(with_second_resource()).unwrap();
    let mut state = with_second_resource();
    for resource in &mut state.resources {
        if resource.resource == label("fixture-resource-2") {
            resource.value = 3;
        }
    }
    let current = checkpoint(state.clone()).unwrap();
    assert_eq!(check(&prepared, &current), Ok(()));
    for resource in &mut state.resources {
        if resource.resource == label("fixture-resource-1") {
            resource.value = 3;
        }
    }
    let current = checkpoint(state).unwrap();
    let before = current.clone();
    assert_eq!(
        check(&prepared, &current),
        Err(PreconditionError::StaleResource)
    );
    assert_eq!(current, before);
    assert!(current.state().draws.is_empty());
}
#[test]
fn removed_resource_and_unknown_resource_dependency_fail_explicitly() {
    let prepared = checkpoint(state()).unwrap();
    let mut state = state();
    state.resources.clear();
    let current = checkpoint(state).unwrap();
    assert_eq!(
        check(&prepared, &current),
        Err(PreconditionError::StaleResource)
    );
    assert_eq!(
        check(&current, &current),
        Err(PreconditionError::InvalidReference)
    );
}
#[test]
fn selected_logical_time_rejects_and_unselected_time_survives() {
    let prepared = checkpoint(state()).unwrap();
    let mut state = state();
    state.logical_time.ticks += 1;
    let current = checkpoint(state).unwrap();
    assert_eq!(
        check(&prepared, &current),
        Err(PreconditionError::StaleTime)
    );
    assert_eq!(
        check_input(&prepared, &current, &input(), &[resource_dependency()]),
        Ok(())
    );
}
#[test]
fn selected_effect_presence_expiry_and_unselected_addition_are_distinct() {
    let empty = checkpoint(state()).unwrap();
    let active = checkpoint(with_effect()).unwrap();
    let selected = [RuleDependency::ActiveEffect(
        RecordId::from_bytes(&[8; 16]).unwrap(),
    )];
    assert_eq!(
        check_input(&empty, &active, &input(), &selected),
        Err(PreconditionError::StaleActiveEffects)
    );
    assert_eq!(
        check_input(&active, &empty, &input(), &selected),
        Err(PreconditionError::StaleActiveEffects)
    );
    let unselected = [RuleDependency::ActiveEffect(
        RecordId::from_bytes(&[99; 16]).unwrap(),
    )];
    assert_eq!(check_input(&empty, &active, &input(), &unselected), Ok(()));
    let mut state = with_effect();
    for effect in &mut state.active_effects {
        effect.expires = Some(LogicalTime {
            ticks: 160,
            ticks_per_second: 10,
        });
    }
    let changed = checkpoint(state).unwrap();
    assert_eq!(
        check_input(&active, &changed, &input(), &selected),
        Err(PreconditionError::StaleActiveEffects)
    );
    assert_eq!(
        check(&empty, &active),
        Err(PreconditionError::StaleActiveEffects)
    );
}
#[test]
fn selected_pending_presence_and_window_change_reject() {
    let empty = checkpoint(state()).unwrap();
    let prepared = checkpoint(with_pending()).unwrap();
    let selected = [RuleDependency::PendingResolution(pending().id)];
    assert_eq!(
        check_input(&empty, &prepared, &input(), &selected),
        Err(PreconditionError::StalePending)
    );
    assert_eq!(
        check_input(&prepared, &empty, &input(), &selected),
        Err(PreconditionError::StalePending)
    );
    let mut state = with_pending();
    for pending in &mut state.pending {
        pending.window.id = WindowId::from_bytes(&[11; 16]).unwrap();
    }
    let changed = checkpoint(state).unwrap();
    assert_eq!(
        check_input(&prepared, &changed, &input(), &selected),
        Err(PreconditionError::StalePending)
    );
    assert_eq!(
        check(&empty, &prepared),
        Err(PreconditionError::StalePending)
    );
}
#[test]
fn selected_timer_generation_status_or_due_change_rejects_unselected_addition_survives() {
    let prepared = checkpoint(with_timer()).unwrap();
    let selected = [RuleDependency::Timer(
        TimerId::from_bytes(&[8; 16]).unwrap(),
    )];
    for mutation in 0..3 {
        let mut state = with_timer();
        for timer in &mut state.timers {
            match mutation {
                0 => timer.generation = 2,
                1 => timer.status = DurableStatus::Cancelled,
                _ => timer.due.ticks += 1,
            }
        }
        let changed = checkpoint(state).unwrap();
        assert_eq!(
            check_input(&prepared, &changed, &input(), &selected),
            Err(PreconditionError::StaleTimers)
        );
    }
    let empty = checkpoint(state()).unwrap();
    let unselected = [RuleDependency::Timer(
        TimerId::from_bytes(&[99; 16]).unwrap(),
    )];
    assert_eq!(
        check_input(&empty, &prepared, &input(), &unselected),
        Ok(())
    );
    assert_eq!(
        check(&empty, &prepared),
        Err(PreconditionError::StaleTimers)
    );
}
#[test]
fn unrelated_sequence_and_presentation_change_preserve_same_pending_response() {
    let prepared = checkpoint(with_pending()).unwrap();
    let mut state = with_pending();
    state.tempo.presentation_ticks = 77;
    let current = checkpoint_at(
        state,
        Basis {
            revision: revision(2, 9),
            ..basis()
        },
        pins(),
    );
    assert_eq!(
        check_input(&prepared, &current, &response_input(), &full_dependencies()),
        Ok(())
    );
}
#[test]
fn replaced_source_content_or_build_pin_rejects() {
    let prepared = checkpoint(state()).unwrap();
    for mutation in 0..3 {
        let mut new_pins = pins();
        let expected = match mutation {
            0 => {
                new_pins.rules.handler_digest = ContentDigest([99; 32]);
                PreconditionError::SourceMismatch
            }
            1 => {
                new_pins.content.content_digest = ContentDigest([99; 32]);
                PreconditionError::ContentMismatch
            }
            _ => {
                new_pins.build = BuildIdentity::new(
                    Some("fixture-source-2"),
                    Some("fixture-native-1"),
                    Some("fixture-wasm-1"),
                    Some("fixture-config-1"),
                    Some("fixture-content-1"),
                )
                .unwrap();
                PreconditionError::BuildMismatch
            }
        };
        let current = checkpoint_at(state(), basis(), new_pins);
        assert_eq!(check(&prepared, &current), Err(expected));
    }
}
#[test]
fn retained_checkpoint_from_retired_epoch_rejects() {
    let prepared = checkpoint(state()).unwrap();
    let current = checkpoint_at(
        state(),
        Basis {
            revision: revision(3, 8),
            ..basis()
        },
        pins(),
    );
    assert_eq!(
        check(&prepared, &current),
        Err(PreconditionError::StalePreparedBasis)
    );
}
#[test]
fn canonical_command_admission_refuses_stale_window_and_internal_input() {
    let current = checkpoint(with_pending()).unwrap();
    let pending = pending();
    let mut command = input();
    if let GameInput::Game(input) = &mut command {
        input.command = GameCommand::SubmitRoll {
            resolution: pending.id,
            window: WindowId::from_bytes(&[22; 16]).unwrap(),
        };
    }
    assert_eq!(
        check_input(&current, &current, &command, &full_dependencies()),
        Err(PreconditionError::Command(CommandError::StaleWindow))
    );
    let timer = GameInput::Timer(TimerExpiry {
        basis: basis(),
        timer: TimerId::from_bytes(&[8; 16]).unwrap(),
        generation: 1,
        observed_time: current.state().logical_time,
    });
    assert_eq!(
        check_input(&current, &current, &timer, &full_dependencies()),
        Err(PreconditionError::Command(CommandError::InternalInput))
    );
}
#[test]
fn detached_current_checkpoint_must_match_trusted_serialized_owner_basis() {
    let current = checkpoint(state()).unwrap();
    assert_eq!(
        check_boundary(
            &current,
            &current,
            &input(),
            &full_dependencies(),
            |context, _| {
                context.basis.revision = revision(2, 9);
            }
        ),
        Err(PreconditionError::CurrentCheckpoint(
            CheckpointError::StaleBasis
        ))
    );
}
#[test]
fn revoked_source_or_resource_constraint_rejects_current_dependency() {
    let current = checkpoint(state()).unwrap();
    assert_eq!(
        check_boundary(
            &current,
            &current,
            &input(),
            &full_dependencies(),
            |context, _| {
                context.inventory.rules = &[];
            }
        ),
        Err(PreconditionError::InvalidReference)
    );
    assert_eq!(
        check_boundary(
            &current,
            &current,
            &input(),
            &full_dependencies(),
            |context, _| {
                context.inventory.resources = &[];
            }
        ),
        Err(PreconditionError::InvalidReference)
    );
}
#[test]
fn zero_exhausted_and_oversized_work_bounds_reject() {
    let current = checkpoint(state()).unwrap();
    for mutation in 0..5 {
        assert_eq!(
            check_boundary(
                &current,
                &current,
                &input(),
                &full_dependencies(),
                |_, limits| {
                    match mutation {
                        0 => limits.maximum_dependencies = 0,
                        1 => limits.maximum_comparisons = 0,
                        2 => limits.maximum_checkpoint_bytes = 0,
                        3 => limits.maximum_comparisons = 1,
                        _ => limits.maximum_checkpoint_bytes = 1,
                    }
                }
            ),
            Err(PreconditionError::Capacity)
        );
    }
}
#[test]
fn repeated_declared_dependency_is_refused() {
    let current = checkpoint(state()).unwrap();
    assert_eq!(
        check_input(
            &current,
            &current,
            &input(),
            &[resource_dependency(), resource_dependency()]
        ),
        Err(PreconditionError::DuplicateDependency)
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FixtureFailure {
    Unsupported,
}
struct FixtureHandler<'a> {
    pins: &'a CheckpointPins,
    calls: &'a Cell<usize>,
    decline: bool,
    bound_source: Option<&'a RuleReference>,
}
impl RulesCommandHandler for FixtureHandler<'_> {
    type Rejection = FixtureFailure;
    fn pins(&self) -> &CheckpointPins {
        self.pins
    }
    fn bound_source(&self) -> Option<&RuleReference> {
        self.bound_source
    }
    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, FixtureFailure> {
        self.calls.set(self.calls.get() + 1);
        if self.decline {
            return Err(FixtureFailure::Unsupported);
        }
        if !input.supplied_draws.is_empty() {
            return Err(FixtureFailure::Unsupported);
        }
        let operation = match input.command {
            GameInput::Game(command) => command.operation,
            GameInput::Host(command) => command.operation,
            _ => return Err(FixtureFailure::Unsupported),
        };
        let next_basis = Basis {
            revision: current.basis().revision.next_sequence().unwrap(),
            ..current.basis()
        };
        let mut state = current.state().clone();
        state.decisions.push(AcceptedDecision {
            operation,
            revision: next_basis.revision,
            facts: vec![],
            draws: vec![],
            effects: vec![],
            source_policy: label("fixture-policy"),
            semantic_output: None,
        });
        Ok(checkpoint_at(state, next_basis, self.pins.clone()))
    }
}
fn with_registered<R>(
    prepared: &Checkpoint,
    current: &Checkpoint,
    dependencies: &[RuleDependency],
    decline: bool,
    call: impl FnOnce(
        &DispatchRegistry<'_, PreconditionedCommandHandler<'_, FixtureHandler<'_>>>,
        CurrentRuleContext<'_>,
        &RevisionLabel,
        &RuleReference,
    ) -> R,
) -> (R, usize) {
    let sources = [rule()];
    let source = &sources[0];
    let entries = [CatalogEntry::new(source, b"synthetic-source")];
    let content_entries = [content()];
    let constraints = resource_constraints();
    let selector = label("fixture-selector");
    let calls = Cell::new(0);
    let handler = FixtureHandler {
        pins: current.pins(),
        calls: &calls,
        decline,
        bound_source: None,
    };
    let context = || CurrentRuleContext {
        checkpoint: current,
        basis: current.basis(),
        pins: current.pins(),
        inventory: ReferenceInventory {
            rules: &sources,
            content: &content_entries,
            resources: &constraints,
            assets: &[],
        },
        command_limits: command_limits(),
    };
    let guarded = PreconditionedCommandHandler::new(
        &handler,
        source,
        context(),
        RulePreconditions {
            prepared,
            sources: &sources,
            dependencies,
        },
        bounds(),
    );
    let registrations = [HandlerRegistration::new(&selector, source, &guarded)];
    let catalog = CatalogSnapshot::from_published(
        &current.pins().rules.catalog,
        current.pins(),
        b"synthetic-publication",
        &entries,
        CatalogLimits {
            max_complete_bytes: 64,
            max_entries: 2,
            max_item_bytes: 64,
            max_total_item_bytes: 128,
        },
    )
    .unwrap();
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 2).unwrap();
    let result = call(&registry, context(), &selector, source);
    (result, calls.get())
}
fn registered_stage(
    prepared: &Checkpoint,
    current: &Checkpoint,
    dependencies: &[RuleDependency],
    input: &GameInput,
    decline: bool,
) -> (
    Result<Checkpoint, InvocationError<PreconditionedRejection<FixtureFailure>>>,
    usize,
) {
    with_registered(
        prepared,
        current,
        dependencies,
        decline,
        |registry, context, selector, source| {
            registry.stage(
                context.pins,
                selector,
                source,
                RulesCommandInput {
                    command: input,
                    supplied_draws: &[],
                },
                current,
                1024 * 1024,
            )
        },
    )
}

#[test]
fn registered_stage_refuses_a_handler_bound_to_a_different_source() {
    let current = checkpoint(state()).unwrap();
    let before = current.clone();
    let registered_source = rule();
    let mut handler_source = registered_source.clone();
    handler_source.clause = label("other-clause");
    let sources = [registered_source.clone(), handler_source.clone()];
    let entries = [
        CatalogEntry::new(&registered_source, b"registered-source"),
        CatalogEntry::new(&handler_source, b"handler-source"),
    ];
    let content_entries = [content()];
    let constraints = resource_constraints();
    let selector = label("fixture-selector");
    let calls = Cell::new(0);
    let handler = FixtureHandler {
        pins: current.pins(),
        calls: &calls,
        decline: false,
        bound_source: None,
    };
    let guarded = PreconditionedCommandHandler::new(
        &handler,
        &handler_source,
        CurrentRuleContext {
            checkpoint: &current,
            basis: current.basis(),
            pins: current.pins(),
            inventory: ReferenceInventory {
                rules: &sources,
                content: &content_entries,
                resources: &constraints,
                assets: &[],
            },
            command_limits: command_limits(),
        },
        RulePreconditions {
            prepared: &current,
            sources: &sources,
            dependencies: &[],
        },
        bounds(),
    );
    let registrations = [HandlerRegistration::new(
        &selector,
        &registered_source,
        &guarded,
    )];
    let catalog = CatalogSnapshot::from_published(
        &current.pins().rules.catalog,
        current.pins(),
        b"synthetic-publication",
        &entries,
        CatalogLimits {
            max_complete_bytes: 128,
            max_entries: 2,
            max_item_bytes: 64,
            max_total_item_bytes: 128,
        },
    )
    .unwrap();
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 2).unwrap();
    let command = input();

    assert_eq!(
        registry.stage(
            current.pins(),
            &selector,
            &registered_source,
            RulesCommandInput {
                command: &command,
                supplied_draws: &[],
            },
            &current,
            1024 * 1024,
        ),
        Err(InvocationError::HandlerSourceMismatch)
    );
    assert_eq!(calls.get(), 0);
    assert_eq!(current, before);
}

#[test]
fn registered_wrapper_checks_inner_source_before_dependencies_and_invocation() {
    let current = checkpoint(state()).unwrap();
    let before = current.clone();
    let source = rule();
    let mut other_source = source.clone();
    other_source.clause = label("other-clause");
    let sources = [source.clone(), other_source.clone()];
    let entries = [
        CatalogEntry::new(&source, b"registered-source"),
        CatalogEntry::new(&other_source, b"other-source"),
    ];
    let contents = [content()];
    let resources = resource_constraints();
    let selector = label("fixture-selector");
    let command = input();

    for (inner_source, mismatched, invalid_dependency) in [
        (Some(&other_source), true, false),
        (Some(&other_source), true, true),
        (Some(&source), false, false),
        (None, false, false),
    ] {
        let calls = Cell::new(0);
        let handler = FixtureHandler {
            pins: current.pins(),
            calls: &calls,
            decline: false,
            bound_source: inner_source,
        };
        let absent_resource = [RuleDependency::Resource {
            owner: entity(4),
            resource: label("missing-resource"),
        }];
        let dependencies = if invalid_dependency {
            &absent_resource[..]
        } else {
            &[][..]
        };
        let guarded = PreconditionedCommandHandler::new(
            &handler,
            &source,
            CurrentRuleContext {
                checkpoint: &current,
                basis: current.basis(),
                pins: current.pins(),
                inventory: ReferenceInventory {
                    rules: &sources,
                    content: &contents,
                    resources: &resources,
                    assets: &[],
                },
                command_limits: command_limits(),
            },
            RulePreconditions {
                prepared: &current,
                sources: &sources,
                dependencies,
            },
            bounds(),
        );
        let registrations = [HandlerRegistration::new(&selector, &source, &guarded)];
        let catalog = CatalogSnapshot::from_published(
            &current.pins().rules.catalog,
            current.pins(),
            b"synthetic-publication",
            &entries,
            CatalogLimits {
                max_complete_bytes: 128,
                max_entries: 2,
                max_item_bytes: 64,
                max_total_item_bytes: 128,
            },
        )
        .unwrap();
        let registry = DispatchRegistry::from_catalog(catalog, &registrations, 2).unwrap();
        let result = registry.stage(
            current.pins(),
            &selector,
            &source,
            RulesCommandInput {
                command: &command,
                supplied_draws: &[],
            },
            &current,
            1024 * 1024,
        );

        if mismatched {
            assert_eq!(
                result,
                Err(InvocationError::Handler(
                    PreconditionedRejection::HandlerSourceMismatch
                ))
            );
            assert_eq!(calls.get(), 0);
        } else {
            let candidate = result.expect("matching or unbound handler stages");
            assert_eq!(
                candidate.state().decisions.len(),
                current.state().decisions.len() + 1
            );
            assert_eq!(calls.get(), 1);
        }
        assert_eq!(current, before);
        assert!(current.state().draws.is_empty());
    }
}

#[test]
fn registry_selected_handler_retains_dependency_list_and_skips_handler_on_selected_change() {
    let prepared = checkpoint(with_second_resource()).unwrap();
    let mut state = with_second_resource();
    for resource in &mut state.resources {
        if resource.resource == label("fixture-resource-1") {
            resource.value = 3;
        }
    }
    let current = checkpoint(state).unwrap();
    let before = current.clone();
    let (result, calls) = registered_stage(
        &prepared,
        &current,
        &[resource_dependency()],
        &input(),
        false,
    );
    assert_eq!(
        result,
        Err(InvocationError::Handler(
            PreconditionedRejection::Precondition(PreconditionError::StaleResource)
        ))
    );
    assert_eq!(calls, 0);
    assert_eq!(current, before);
}
#[test]
fn registry_selected_handler_accepts_unselected_change_and_preserves_canonical_candidate() {
    let prepared = checkpoint(with_second_resource()).unwrap();
    let mut state = with_second_resource();
    for resource in &mut state.resources {
        if resource.resource == label("fixture-resource-2") {
            resource.value = 3;
        }
    }
    let current = checkpoint(state).unwrap();
    let before = current.clone();
    let (result, calls) = registered_stage(
        &prepared,
        &current,
        &[resource_dependency()],
        &input(),
        false,
    );
    let candidate = result.unwrap();
    assert_eq!(
        candidate.basis().revision,
        current.basis().revision.next_sequence().unwrap()
    );
    assert_eq!(candidate.state().resources, current.state().resources);
    assert_eq!(candidate.state().decisions.len(), 1);
    assert_eq!(calls, 1);
    assert_eq!(current, before);
}
#[test]
fn registry_invocation_preserves_actual_handler_refusal() {
    let current = checkpoint(state()).unwrap();
    let (result, calls) =
        registered_stage(&current, &current, &[resource_dependency()], &input(), true);
    assert_eq!(
        result,
        Err(InvocationError::Handler(PreconditionedRejection::Handler(
            FixtureFailure::Unsupported
        )))
    );
    assert_eq!(calls, 1);
}
#[test]
fn registry_guard_rejects_declared_time_pending_and_timer_before_inner_handler() {
    let prepared = checkpoint(state()).unwrap();
    let mut state = state();
    state.logical_time.ticks += 1;
    let current = checkpoint(state).unwrap();
    let (result, calls) = registered_stage(
        &prepared,
        &current,
        &[RuleDependency::LogicalTime],
        &input(),
        false,
    );
    assert_eq!(
        result,
        Err(InvocationError::Handler(
            PreconditionedRejection::Precondition(PreconditionError::StaleTime)
        ))
    );
    assert_eq!(calls, 0);
    let current_pending = checkpoint(with_pending()).unwrap();
    let (result, calls) = registered_stage(
        &prepared,
        &current_pending,
        &[RuleDependency::PendingResolution(pending().id)],
        &input(),
        false,
    );
    assert_eq!(
        result,
        Err(InvocationError::Handler(
            PreconditionedRejection::Precondition(PreconditionError::StalePending)
        ))
    );
    assert_eq!(calls, 0);
    let timer = checkpoint(with_timer()).unwrap();
    let (result, calls) = registered_stage(
        &prepared,
        &timer,
        &[RuleDependency::Timer(
            TimerId::from_bytes(&[8; 16]).unwrap(),
        )],
        &input(),
        false,
    );
    assert_eq!(
        result,
        Err(InvocationError::Handler(
            PreconditionedRejection::Precondition(PreconditionError::StaleTimers)
        ))
    );
    assert_eq!(calls, 0);
}

#[test]
fn response_consumer_stages_unchanged_selected_pending_across_unselected_changes() {
    let prepared = checkpoint(with_pending()).unwrap();
    let mut state = with_pending();
    for resource in &mut state.resources {
        resource.value = 3;
    }
    state.logical_time.ticks += 1;
    state.tempo.presentation_ticks = 77;
    let current = checkpoint_at(
        state,
        Basis {
            revision: revision(2, 9),
            ..basis()
        },
        pins(),
    );
    let dependencies = [RuleDependency::PendingResolution(pending().id)];
    let (candidate, calls) = with_registered(
        &prepared,
        &current,
        &dependencies,
        false,
        |registry, context, selector, _| {
            crate::current_responses::prepare_current_response(
                RulesCommandInput {
                    command: &response_input(),
                    supplied_draws: &[],
                },
                context,
                registry,
                selector,
                1024 * 1024,
            )
        },
    );
    assert_eq!(calls, 1);
    assert_eq!(
        candidate.unwrap().basis().revision,
        current.basis().revision.next_sequence().unwrap()
    );
    assert_eq!(current.state().draws, prepared.state().draws);
    assert!(current.state().decisions.is_empty());
}

#[test]
fn response_consumer_checks_selected_time_pending_and_timer_dependencies_before_inner_handler() {
    let mut initial = with_pending();
    initial.timers = with_timer().timers;
    let prepared = checkpoint(initial.clone()).unwrap();
    for mutation in 0..3 {
        let mut state = initial.clone();
        let (dependency, reason) = match mutation {
            0 => {
                state.logical_time.ticks += 1;
                (RuleDependency::LogicalTime, PreconditionError::StaleTime)
            }
            1 => {
                for resolution in &mut state.pending {
                    resolution.continuation = label("fixture-next-continuation");
                }
                (
                    RuleDependency::PendingResolution(pending().id),
                    PreconditionError::StalePending,
                )
            }
            _ => {
                for timer in &mut state.timers {
                    timer.generation += 1;
                }
                (
                    RuleDependency::Timer(TimerId::from_bytes(&[8; 16]).unwrap()),
                    PreconditionError::StaleTimers,
                )
            }
        };
        let current = checkpoint(state).unwrap();
        let before = current.clone();
        let (result, calls) = with_registered(
            &prepared,
            &current,
            &[dependency],
            false,
            |registry, context, selector, _| {
                crate::current_responses::prepare_current_response(
                    RulesCommandInput {
                        command: &response_input(),
                        supplied_draws: &[],
                    },
                    context,
                    registry,
                    selector,
                    1024 * 1024,
                )
            },
        );
        assert_eq!(
            result,
            Err(
                crate::current_responses::ResponsePreparationError::Invocation(
                    InvocationError::Handler(PreconditionedRejection::Precondition(reason))
                )
            )
        );
        assert_eq!(calls, 0);
        assert_eq!(current, before);
    }
}

fn movement_state() -> GameState {
    let mut state = state();
    for id in [entity(6), entity(8)] {
        state.entities.push(WorldEntity {
            id,
            definition: content(),
            location: None,
            position: Some(Position { x: 10, y: 0, z: 0 }),
            identity_revision: label("fixture-target"),
        });
    }
    state.encounters.push(EncounterState {
        id: RecordId::from_bytes(&[40; 16]).unwrap(),
        definition: content(),
        participants: vec![entity(4), entity(6)],
        turn_order: vec![entity(4), entity(6)],
        active_turn: Some(entity(4)),
        objectives: vec![content()],
        combat_policy: content(),
        tactical: None,
    });
    state.continuity.environment.push(EnvironmentalState {
        location: entity(6),
        definition: content(),
        change_facts: vec![],
    });
    state
}
fn movement_dependencies() -> Vec<RuleDependency> {
    vec![
        RuleDependency::Entity(entity(4)),
        RuleDependency::Entity(entity(6)),
        RuleDependency::Encounter(RecordId::from_bytes(&[40; 16]).unwrap()),
        RuleDependency::Environment(entity(6)),
        resource_dependency(),
        RuleDependency::LogicalTime,
    ]
}
#[test]
fn selected_entity_position_and_presence_change_rejects_before_registered_handler() {
    let prepared = checkpoint(movement_state()).unwrap();
    let mut state = movement_state();
    for target in &mut state.entities {
        if target.id == entity(6) {
            target.position = Some(Position { x: 11, y: 0, z: 0 });
        }
    }
    let current = checkpoint(state).unwrap();
    let (result, calls) = registered_stage(
        &prepared,
        &current,
        &movement_dependencies(),
        &input(),
        false,
    );
    assert_eq!(
        result,
        Err(InvocationError::Handler(
            PreconditionedRejection::Precondition(PreconditionError::StaleEntity)
        ))
    );
    assert_eq!(calls, 0);
    let empty = checkpoint(self::state()).unwrap();
    assert_eq!(
        check_input(
            &empty,
            &prepared,
            &input(),
            &[RuleDependency::Entity(entity(6))]
        ),
        Err(PreconditionError::StaleEntity)
    );
    assert_eq!(
        check_input(
            &prepared,
            &empty,
            &input(),
            &[RuleDependency::Entity(entity(6))]
        ),
        Err(PreconditionError::StaleEntity)
    );
}
#[test]
fn selected_encounter_objectives_turn_or_policy_changes_rejects() {
    let prepared = checkpoint(movement_state()).unwrap();
    for mutation in 0..3 {
        let mut state = movement_state();
        for encounter in &mut state.encounters {
            match mutation {
                0 => encounter.objectives.clear(),
                1 => encounter.active_turn = Some(entity(6)),
                _ => encounter.combat_policy = alternate_content(),
            }
        }
        let current = checkpoint(state).unwrap();
        let (result, calls) = registered_stage(
            &prepared,
            &current,
            &movement_dependencies(),
            &input(),
            false,
        );
        assert_eq!(
            result,
            Err(InvocationError::Handler(
                PreconditionedRejection::Precondition(PreconditionError::StaleEncounter)
            ))
        );
        assert_eq!(calls, 0);
    }
    let empty = checkpoint(state()).unwrap();
    let dependency = [RuleDependency::Encounter(
        RecordId::from_bytes(&[40; 16]).unwrap(),
    )];
    assert_eq!(
        check_input(&empty, &prepared, &input(), &dependency),
        Err(PreconditionError::StaleEncounter)
    );
    assert_eq!(
        check_input(&prepared, &empty, &input(), &dependency),
        Err(PreconditionError::StaleEncounter)
    );
}
#[test]
fn selected_environment_checks_all_rows_at_location_including_presence_and_definition() {
    let prepared = checkpoint(movement_state()).unwrap();
    for mutation in 0..3 {
        let mut state = movement_state();
        match mutation {
            0 => {
                for environment in &mut state.continuity.environment {
                    environment.definition = alternate_content();
                }
            }
            1 => state.continuity.environment.push(EnvironmentalState {
                location: entity(6),
                definition: content(),
                change_facts: vec![],
            }),
            _ => state.continuity.environment.clear(),
        }
        let current = checkpoint(state).unwrap();
        let (result, calls) = registered_stage(
            &prepared,
            &current,
            &movement_dependencies(),
            &input(),
            false,
        );
        assert_eq!(
            result,
            Err(InvocationError::Handler(
                PreconditionedRejection::Precondition(PreconditionError::StaleEnvironment)
            ))
        );
        assert_eq!(calls, 0);
    }
    let mut state = movement_state();
    state.continuity.environment.clear();
    let empty = checkpoint(state).unwrap();
    assert_eq!(
        check_input(
            &empty,
            &prepared,
            &input(),
            &[RuleDependency::Environment(entity(6))]
        ),
        Err(PreconditionError::StaleEnvironment)
    );
}
#[test]
fn movement_readset_accepts_unselected_entity_and_other_location_environment_changes() {
    let prepared = checkpoint(movement_state()).unwrap();
    let mut state = movement_state();
    for target in &mut state.entities {
        if target.id == entity(8) {
            target.position = Some(Position { x: 99, y: 0, z: 0 });
        }
    }
    state.continuity.environment.push(EnvironmentalState {
        location: entity(8),
        definition: alternate_content(),
        change_facts: vec![],
    });
    let current = checkpoint(state).unwrap();
    let before = current.clone();
    let (result, calls) = registered_stage(
        &prepared,
        &current,
        &movement_dependencies(),
        &input(),
        false,
    );
    assert!(result.is_ok());
    assert_eq!(calls, 1);
    assert_eq!(current, before);
}
#[test]
fn selected_content_reference_revocation_refuses_same_entity_encounter_and_environment_records() {
    let current = checkpoint(movement_state()).unwrap();
    for dependency in [
        RuleDependency::Entity(entity(6)),
        RuleDependency::Encounter(RecordId::from_bytes(&[40; 16]).unwrap()),
        RuleDependency::Environment(entity(6)),
    ] {
        assert_eq!(
            check_boundary(&current, &current, &input(), &[dependency], |context, _| {
                context.inventory.content = &[];
            }),
            Err(PreconditionError::InvalidReference)
        );
    }
}

struct ForwardingHandler<'a> {
    pins: &'a CheckpointPins,
    command: &'a GameInput,
    draws: &'a [ActualDraw],
    calls: Cell<usize>,
    identical_command: Cell<bool>,
    identical_draw_slice: Cell<bool>,
}
impl RulesCommandHandler for ForwardingHandler<'_> {
    type Rejection = FixtureFailure;
    fn pins(&self) -> &CheckpointPins {
        self.pins
    }
    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        _: &Checkpoint,
    ) -> Result<Checkpoint, FixtureFailure> {
        self.calls.set(self.calls.get() + 1);
        self.identical_command
            .set(std::ptr::eq(self.command, input.command));
        self.identical_draw_slice.set(
            self.draws.len() == input.supplied_draws.len()
                && std::ptr::eq(self.draws.as_ptr(), input.supplied_draws.as_ptr())
                && self.draws == input.supplied_draws,
        );
        // Invocation observation has no mechanical outcome to stage. Preserve a typed gap;
        // the real handler, not this adapter, decides and records consumed supplied outcomes.
        Err(FixtureFailure::Unsupported)
    }
}
fn roll_state() -> GameState {
    let mut state = with_pending();
    for pending in &mut state.pending {
        pending.next = PendingInput::Roll {
            participant: member(3),
            sides: vec![20, 20],
            source: rule(),
        };
    }
    state
}
fn roll_input() -> GameInput {
    let mut input = input();
    if let GameInput::Game(command) = &mut input {
        command.command = GameCommand::SubmitRoll {
            resolution: pending().id,
            window: pending().window.id,
        };
    }
    input
}
fn trusted_draws() -> [ActualDraw; 2] {
    [(0, 17), (1, 2)].map(|(ordinal, value)| ActualDraw {
        operation: OperationId::from_bytes(&[20; 16]).unwrap(),
        ordinal,
        resolution: pending().id,
        window: pending().window.id,
        sides: 20,
        value,
        source: rule(),
    })
}

#[test]
fn registered_preconditioned_handler_forwards_identical_trusted_draw_envelope() {
    let current = checkpoint(roll_state()).unwrap();
    let original = current.clone();
    let command = roll_input();
    let original_command = command.clone();
    let draws = trusted_draws();
    let original_draws = draws.clone();
    let sources = [rule()];
    let contents = [content()];
    let resources = resource_constraints();
    let dependencies = [
        resource_dependency(),
        RuleDependency::PendingResolution(pending().id),
    ];
    let inner = ForwardingHandler {
        pins: current.pins(),
        command: &command,
        draws: &draws,
        calls: Cell::new(0),
        identical_command: Cell::new(false),
        identical_draw_slice: Cell::new(false),
    };
    let guarded = PreconditionedCommandHandler::new(
        &inner,
        &sources[0],
        CurrentRuleContext {
            checkpoint: &current,
            basis: current.basis(),
            pins: current.pins(),
            inventory: ReferenceInventory {
                rules: &sources,
                content: &contents,
                resources: &resources,
                assets: &[],
            },
            command_limits: command_limits(),
        },
        RulePreconditions {
            prepared: &current,
            sources: &sources,
            dependencies: &dependencies,
        },
        bounds(),
    );
    let selector = label("fixture-roll-handler");
    let registrations = [HandlerRegistration::new(&selector, &sources[0], &guarded)];
    let entries = [CatalogEntry::new(&sources[0], b"synthetic-roll-source")];
    let catalog = CatalogSnapshot::from_published(
        &current.pins().rules.catalog,
        current.pins(),
        b"synthetic-publication",
        &entries,
        CatalogLimits {
            max_complete_bytes: 64,
            max_entries: 2,
            max_item_bytes: 64,
            max_total_item_bytes: 128,
        },
    )
    .unwrap();
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 2).unwrap();
    let envelope = RulesCommandInput {
        command: &command,
        supplied_draws: &draws,
    };
    let result = registry.stage(
        current.pins(),
        &selector,
        &sources[0],
        envelope,
        &current,
        1024 * 1024,
    );
    assert_eq!(
        result,
        Err(InvocationError::Handler(PreconditionedRejection::Handler(
            FixtureFailure::Unsupported
        )))
    );
    assert_eq!(inner.calls.get(), 1);
    assert!(inner.identical_command.get());
    assert!(inner.identical_draw_slice.get());
    assert_eq!(draws, original_draws);
    assert_eq!(command, original_command);
    assert_eq!(current, original);
    assert!(current.state().draws.is_empty());
}

#[test]
fn stale_precondition_refuses_before_inner_handler_without_consuming_supplied_draws() {
    let prepared = checkpoint(roll_state()).unwrap();
    let mut state = roll_state();
    for resource in &mut state.resources {
        resource.value = 3;
    }
    let current = checkpoint(state).unwrap();
    let original = current.clone();
    let command = roll_input();
    let draws = trusted_draws();
    let original_draws = draws.clone();
    let sources = [rule()];
    let contents = [content()];
    let resources = resource_constraints();
    let dependencies = [resource_dependency()];
    let inner = ForwardingHandler {
        pins: current.pins(),
        command: &command,
        draws: &draws,
        calls: Cell::new(0),
        identical_command: Cell::new(false),
        identical_draw_slice: Cell::new(false),
    };
    let guarded = PreconditionedCommandHandler::new(
        &inner,
        &sources[0],
        CurrentRuleContext {
            checkpoint: &current,
            basis: current.basis(),
            pins: current.pins(),
            inventory: ReferenceInventory {
                rules: &sources,
                content: &contents,
                resources: &resources,
                assets: &[],
            },
            command_limits: command_limits(),
        },
        RulePreconditions {
            prepared: &prepared,
            sources: &sources,
            dependencies: &dependencies,
        },
        bounds(),
    );
    let result = guarded.stage(
        RulesCommandInput {
            command: &command,
            supplied_draws: &draws,
        },
        &current,
    );
    assert_eq!(
        result,
        Err(PreconditionedRejection::Precondition(
            PreconditionError::StaleResource
        ))
    );
    assert_eq!(inner.calls.get(), 0);
    assert_eq!(draws, original_draws);
    assert_eq!(current, original);
    assert!(current.state().draws.is_empty());
}
