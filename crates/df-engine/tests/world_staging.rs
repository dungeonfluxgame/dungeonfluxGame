#[path = "support/fixture_model.rs"]
pub mod fixture_model;

use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use df_engine::command_entry::{
    CommandEntryContext, CommandEntryLimits, CommandRejection, decide_registered_command,
};
use df_engine::world_staging::*;
use df_model::checkpoint::*;
use df_model::commands::CommandLimits;
use df_rules::preconditions::*;
use df_rules::{
    DispatchRegistry, HandlerRegistration, InvocationError, RulesCommandHandler, RulesCommandInput,
};
use df_types::OperationId;
use df_world::{AdmittedEnvironmentalChange, EnvironmentalDeltaError, EnvironmentalDeltaLimits};
use fixture_model as fixture;
use std::cell::Cell;

fn content(entry: &str) -> ContentReference {
    ContentReference {
        entry: fixture::label(entry),
        ..fixture::content()
    }
}
fn fact(value: u8) -> FactId {
    FactId::from_bytes(&[value; 16]).unwrap()
}
fn operation(value: u8) -> OperationId {
    OperationId::from_bytes(&[value; 16]).unwrap()
}
fn command_limits() -> CommandLimits {
    CommandLimits {
        maximum_records: 100,
        maximum_text_bytes: 256,
        maximum_retained_bytes: 1024 * 1024,
    }
}

struct Fixture {
    registration: EnvironmentalRegistration,
    pins: CheckpointPins,
    rules: Vec<RuleReference>,
    content: Vec<ContentReference>,
    resources: Vec<ResourceConstraint>,
    assets: Vec<AssetReference>,
}
impl Fixture {
    fn new() -> Self {
        Self {
            registration: EnvironmentalRegistration {
                action: content("apply-authored-storm"),
                environment: content("authored-storm"),
                source: fixture::rule(),
                policy: fixture::label("compiled-storm-application"),
            },
            pins: fixture::pins(),
            rules: vec![fixture::rule()],
            content: vec![
                fixture::content(),
                content("apply-authored-storm"),
                content("authored-storm"),
            ],
            resources: fixture::resource_constraints(),
            assets: vec![AssetReference {
                key: fixture::label("exact-tactical-geometry"),
                digest: ContentDigest([6; 32]),
                byte_length: 128,
                kind: AssetKind::TacticalGeometry,
            }],
        }
    }
    fn inventory(&self) -> ReferenceInventory<'_> {
        ReferenceInventory {
            rules: &self.rules,
            content: &self.content,
            resources: &self.resources,
            assets: &self.assets,
        }
    }
    fn state(&self) -> GameState {
        let mut state = fixture::state();
        for id in [5, 6] {
            let mut location = state.entities[0].clone();
            location.id = fixture::entity(id);
            state.entities.push(location);
        }
        state.facts = vec![
            GameFact {
                id: fact(30),
                revision: fixture::revision(2, 7),
                operation: operation(30),
                ordinal: 0,
                cause: None,
                audience: AudienceScope::Members(vec![fixture::member(3)]),
                value: FactValue::ContentEvent {
                    definition: fixture::content(),
                    subjects: vec![fixture::entity(5), fixture::entity(6)],
                },
            },
            GameFact {
                id: fact(31),
                revision: fixture::basis().revision,
                operation: operation(31),
                ordinal: 0,
                cause: Some(fact(30)),
                audience: AudienceScope::Members(vec![fixture::member(3)]),
                value: FactValue::ContentEvent {
                    definition: self.registration.environment.clone(),
                    subjects: vec![fixture::entity(5), fixture::entity(6)],
                },
            },
        ];
        state.decisions = state
            .facts
            .iter()
            .map(|cause| AcceptedDecision {
                operation: cause.operation,
                revision: cause.revision,
                facts: vec![cause.id],
                draws: vec![],
                effects: vec![],
                source_policy: fixture::label("accepted-storm-source"),
                semantic_output: None,
            })
            .collect();
        state.continuity.canonical_packs.push(CanonicalPack {
            revision: fixture::label("geometry-pack"),
            digest: ContentDigest([7; 32]),
            bible: VisualBible {
                revision: fixture::label("geometry-bible"),
                definition: fixture::content(),
                palette: vec![],
                style: String::new(),
                references: vec![],
            },
            identities: vec![],
        });
        for id in [5, 6] {
            state.continuity.environment.push(EnvironmentalState {
                location: fixture::entity(id),
                definition: fixture::content(),
                change_facts: vec![fact(30)],
            });
            state.continuity.scenes.push(SceneIdentityRevision {
                scene: fixture::entity(id),
                revision: fixture::label("current-scene"),
                source_facts: vec![fact(30)],
                geometry: self.assets[0].clone(),
                canonical_pack: fixture::label("geometry-pack"),
            });
        }
        state
    }
    fn checkpoint(&self, basis: Basis, state: GameState) -> Checkpoint {
        Checkpoint::new(
            CHECKPOINT_SCHEMA,
            basis,
            self.pins.clone(),
            state,
            self.inventory(),
            fixture::limits(),
        )
        .unwrap()
    }
    fn current(&self) -> Checkpoint {
        self.checkpoint(fixture::basis(), self.state())
    }
    fn replacements(&self) -> Vec<EnvironmentalState> {
        [5, 6]
            .into_iter()
            .map(|id| EnvironmentalState {
                location: fixture::entity(id),
                definition: self.registration.environment.clone(),
                change_facts: vec![fact(30), fact(31)],
            })
            .collect()
    }
    fn input(&self, basis: Basis) -> GameInput {
        GameInput::Game(CommandInput {
            basis,
            observed_revision: basis.revision,
            operation: operation(50),
            member: fixture::member(3),
            command: GameCommand::ProposeAction {
                actor: fixture::entity(4),
                action: self.registration.action.clone(),
                targets: vec![fixture::entity(5), fixture::entity(6)],
                choices: vec![],
            },
        })
    }
    fn handler<'a>(
        &'a self,
        owner: &'a SourceOwner,
        basis: Basis,
        changes: Option<&'a [AdmittedEnvironmentalChange<'a>]>,
    ) -> EnvironmentalChangeHandler<'a, SourceOwner> {
        EnvironmentalChangeHandler {
            owner,
            current_basis: basis,
            admitted_pins: &self.pins,
            inventory: self.inventory(),
            registration: &self.registration,
            changes,
            limits: WorldStagingLimits {
                maximum_checkpoint_bytes: 1024 * 1024,
                maximum_pass_bytes: 4 * 1024 * 1024,
                maximum_inventory_records: 32,
                checkpoint: fixture::limits(),
                world: EnvironmentalDeltaLimits {
                    input_records: 256,
                    changes: 4,
                    output_bytes: 16 * 1024,
                },
            },
        }
    }
}
fn changes<'a>(
    current: &'a Checkpoint,
    replacements: &'a [EnvironmentalState],
) -> Vec<AdmittedEnvironmentalChange<'a>> {
    current
        .state()
        .continuity
        .environment
        .iter()
        .zip(replacements)
        .zip(&current.state().continuity.scenes)
        .map(
            |((expected, replacement), geometry)| AdmittedEnvironmentalChange {
                expected,
                replacement,
                geometry,
            },
        )
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SourceRefusal {
    Withdrawn,
    Binding,
    ActorControl,
    Cause,
}
struct SourceOwner {
    expected: EnvironmentalRegistration,
    pins: CheckpointPins,
    refusal: Cell<Option<SourceRefusal>>,
    calls: Cell<usize>,
}
impl SourceOwner {
    fn new(fixture: &Fixture) -> Self {
        Self {
            expected: fixture.registration.clone(),
            pins: fixture.pins.clone(),
            refusal: Cell::new(None),
            calls: Cell::new(0),
        }
    }
}
impl EnvironmentalSourceOwner for SourceOwner {
    type Refusal = SourceRefusal;
    fn admit(
        &self,
        current: &Checkpoint,
        registration: &EnvironmentalRegistration,
        command: &CommandInput,
        changes: &[AdmittedEnvironmentalChange<'_>],
    ) -> Result<(), Self::Refusal> {
        self.calls.set(self.calls.get() + 1);
        if let Some(refusal) = self.refusal.get() {
            return Err(refusal);
        }
        if registration != &self.expected || current.pins() != &self.pins {
            return Err(SourceRefusal::Binding);
        }
        let GameCommand::ProposeAction { actor, .. } = &command.command else {
            return Err(SourceRefusal::ActorControl);
        };
        if command.member != fixture::member(3)
            || *actor != fixture::entity(4)
            || !current
                .state()
                .characters
                .iter()
                .any(|character| character.entity == *actor && character.owner == command.member)
        {
            return Err(SourceRefusal::ActorControl);
        }
        // Finite test source: only this authored storm, these two locations and private cause.
        // No geometry-byte/spatial or production source qualification is claimed by this fixture.
        for change in changes {
            if ![fixture::entity(5), fixture::entity(6)].contains(&change.expected.location) {
                return Err(SourceRefusal::Binding);
            }
            for id in change
                .replacement
                .change_facts
                .iter()
                .skip(change.expected.change_facts.len())
            {
                let cause = current
                    .state()
                    .facts
                    .iter()
                    .find(|cause| cause.id == *id)
                    .ok_or(SourceRefusal::Cause)?;
                if cause.audience != AudienceScope::Members(vec![fixture::member(3)])
                    || !current.state().decisions.iter().any(|decision| {
                        decision.operation == cause.operation
                            && decision.revision == cause.revision
                            && decision.facts.contains(id)
                            && decision.source_policy == fixture::label("accepted-storm-source")
                    })
                {
                    return Err(SourceRefusal::Cause);
                }
            }
        }
        Ok(())
    }
}

type Rejection = CommandRejection<PreconditionedRejection<WorldStagingError<SourceRefusal>>>;
fn registered_handler(
    fixture: &Fixture,
    prepared: &Checkpoint,
    current: &Checkpoint,
    input: &GameInput,
    handler: &EnvironmentalChangeHandler<'_, SourceOwner>,
) -> Result<Checkpoint, Rejection> {
    registered_with_draws(fixture, prepared, current, input, handler, &[])
}
fn registered_with_draws(
    fixture: &Fixture,
    prepared: &Checkpoint,
    current: &Checkpoint,
    input: &GameInput,
    handler: &EnvironmentalChangeHandler<'_, SourceOwner>,
    supplied_draws: &[ActualDraw],
) -> Result<Checkpoint, Rejection> {
    let dependencies = [
        RuleDependency::Entity(fixture::entity(4)),
        RuleDependency::Entity(fixture::entity(5)),
        RuleDependency::Entity(fixture::entity(6)),
        RuleDependency::Environment(fixture::entity(5)),
        RuleDependency::Environment(fixture::entity(6)),
        RuleDependency::LogicalTime,
        RuleDependency::PendingResolutions,
    ];
    let sources = [fixture.registration.source.clone()];
    let guarded = PreconditionedCommandHandler::new(
        handler,
        &fixture.registration.source,
        CurrentRuleContext {
            checkpoint: current,
            basis: handler.current_basis,
            pins: handler.admitted_pins,
            inventory: fixture.inventory(),
            command_limits: command_limits(),
        },
        RulePreconditions {
            prepared,
            sources: &sources,
            dependencies: &dependencies,
        },
        PreconditionLimits {
            maximum_dependencies: 8,
            maximum_comparisons: 10_000_000,
            maximum_checkpoint_bytes: 1024 * 1024,
        },
    );
    let selector = fixture::label("compiled-environment-application");
    let entries = [CatalogEntry::new(
        &fixture.registration.source,
        b"exact synthetic environmental source",
    )];
    let catalog = CatalogSnapshot::from_published(
        &fixture.pins.rules.catalog,
        &fixture.pins,
        b"complete synthetic catalog",
        &entries,
        CatalogLimits {
            max_complete_bytes: 64,
            max_entries: 1,
            max_item_bytes: 64,
            max_total_item_bytes: 64,
        },
    )
    .unwrap();
    let registrations = [HandlerRegistration::new(
        &selector,
        &fixture.registration.source,
        &guarded,
    )];
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 1).unwrap();
    decide_registered_command(
        RulesCommandInput {
            command: input,
            supplied_draws,
        },
        current,
        CommandEntryContext {
            current_basis: handler.current_basis,
            admitted_pins: handler.admitted_pins,
            inventory: fixture.inventory(),
            limits: CommandEntryLimits {
                command: command_limits(),
                maximum_staged_bytes: 1024 * 1024,
            },
        },
        &registry,
        &selector,
        &fixture.registration.source,
    )
}
fn registered(
    fixture: &Fixture,
    source: &SourceOwner,
    current: &Checkpoint,
    input: &GameInput,
) -> Result<Checkpoint, Rejection> {
    let replacements = fixture.replacements();
    let changes = changes(current, &replacements);
    let handler = fixture.handler(source, current.basis(), Some(&changes));
    registered_handler(fixture, current, current, input, &handler)
}

#[test]
fn registered_world_replacement_preserves_exact_committed_private_cause_and_sibling_state() {
    let fixture = Fixture::new();
    let current = fixture.current();
    let original = current.clone();
    let source = SourceOwner::new(&fixture);
    let next = registered(&fixture, &source, &current, &fixture.input(current.basis())).unwrap();
    let mut expected = current.state().clone();
    expected.continuity.environment = fixture.replacements();
    let decision = next.state().decisions.last().unwrap();
    assert_eq!(decision.operation, operation(50));
    assert_eq!(decision.source_policy, fixture.registration.policy);
    assert!(decision.facts.is_empty() && decision.draws.is_empty() && decision.effects.is_empty());
    expected.decisions.push(decision.clone());
    assert_eq!(next.state(), &expected);
    assert_eq!(current, original);
    assert_eq!(source.calls.get(), 1);
    assert_eq!(
        next.basis().revision,
        current.basis().revision.next_sequence().unwrap()
    );
    assert_eq!(next.pins(), current.pins());
}

#[test]
fn source_withdrawal_actor_control_and_registration_are_independent_of_membership() {
    let fixture = Fixture::new();
    let current = fixture.current();
    let source = SourceOwner::new(&fixture);
    for refusal in [SourceRefusal::Withdrawn, SourceRefusal::ActorControl] {
        source.refusal.set(Some(refusal));
        assert_eq!(
            registered(&fixture, &source, &current, &fixture.input(current.basis())),
            Err(CommandRejection::Invocation(InvocationError::Handler(
                PreconditionedRejection::Handler(WorldStagingError::Source(refusal))
            )))
        );
        assert_eq!(current, fixture.current());
    }
    source.refusal.set(None);
    let mut altered = fixture.registration.clone();
    altered.policy = fixture::label("unadmitted-policy");
    let replacements = fixture.replacements();
    let changes = changes(&current, &replacements);
    let mut handler = fixture.handler(&source, current.basis(), Some(&changes));
    handler.registration = &altered;
    assert_eq!(
        handler.stage(
            RulesCommandInput {
                command: &fixture.input(current.basis()),
                supplied_draws: &[]
            },
            &current
        ),
        Err(WorldStagingError::Source(SourceRefusal::Binding))
    );
}

#[test]
fn stale_precondition_refuses_before_source_owner_and_no_partial_environment_escapes() {
    let fixture = Fixture::new();
    let prepared = fixture.current();
    let source = SourceOwner::new(&fixture);
    for case in ["environment", "time", "entity"] {
        let mut state = fixture.state();
        match case {
            "environment" => state.continuity.environment[1].change_facts.clear(),
            "time" => state.logical_time.ticks += 1,
            _ => state.entities[0].identity_revision = fixture::label("different-entity"),
        }
        let current = fixture.checkpoint(prepared.basis(), state);
        let replacements = fixture.replacements();
        let changes = changes(&current, &replacements);
        let handler = fixture.handler(&source, current.basis(), Some(&changes));
        let reason = match case {
            "environment" => PreconditionError::StaleEnvironment,
            "time" => PreconditionError::StaleTime,
            _ => PreconditionError::StaleEntity,
        };
        assert_eq!(
            registered_handler(
                &fixture,
                &prepared,
                &current,
                &fixture.input(current.basis()),
                &handler
            ),
            Err(CommandRejection::Invocation(InvocationError::Handler(
                PreconditionedRejection::Precondition(reason)
            )))
        );
        assert_eq!(source.calls.get(), 0);
        assert_eq!(current.state().decisions.len(), 2);
    }
}

#[test]
fn last_change_geometry_prefix_and_source_cause_fail_atomically() {
    let fixture = Fixture::new();
    let current = fixture.current();
    let source = SourceOwner::new(&fixture);
    for case in ["geometry", "prefix", "cause", "definition"] {
        let mut replacements = fixture.replacements();
        let mut geometry = current.state().continuity.scenes[1].clone();
        match case {
            "geometry" => geometry.revision = fixture::label("stale-scene"),
            "prefix" => {
                replacements[1].change_facts.remove(0);
            }
            "cause" => replacements[1].change_facts = vec![fact(30)],
            _ => replacements[1].definition = fixture::content(),
        };
        let mut changes = changes(&current, &replacements);
        changes[1].geometry = &geometry;
        let handler = fixture.handler(&source, current.basis(), Some(&changes));
        let result = handler.stage(
            RulesCommandInput {
                command: &fixture.input(current.basis()),
                supplied_draws: &[],
            },
            &current,
        );
        assert!(result.is_err(), "{case}");
        assert_eq!(current, fixture.current());
        if case == "geometry" {
            assert_eq!(
                result,
                Err(WorldStagingError::World(
                    EnvironmentalDeltaError::StaleGeometry
                ))
            );
        }
        if case == "cause" {
            assert_eq!(
                result,
                Err(WorldStagingError::World(
                    EnvironmentalDeltaError::MissingCause
                ))
            );
        }
    }
}

#[test]
fn missing_producer_stale_basis_and_full_source_pins_refuse_before_source_admission() {
    let fixture = Fixture::new();
    let current = fixture.current();
    let source = SourceOwner::new(&fixture);
    let input = fixture.input(current.basis());
    let mut handler = fixture.handler(&source, current.basis(), None);
    assert_eq!(
        handler.stage(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &current
        ),
        Err(WorldStagingError::MissingProducer)
    );
    handler.current_basis.revision = fixture::revision(2, 7);
    assert!(matches!(
        handler.stage(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &current
        ),
        Err(WorldStagingError::Snapshot(_))
    ));
    handler.current_basis = current.basis();
    let mut pins = fixture.pins.clone();
    pins.content.package_digest = ContentDigest([99; 32]);
    handler.admitted_pins = &pins;
    assert!(matches!(
        handler.stage(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &current
        ),
        Err(WorldStagingError::Snapshot(_))
    ));
    assert_eq!(source.calls.get(), 0);
    assert_eq!(current, fixture.current());
}

#[test]
fn bounded_request_and_owned_world_output_fail_without_candidate() {
    let fixture = Fixture::new();
    let current = fixture.current();
    let source = SourceOwner::new(&fixture);
    let replacements = fixture.replacements();
    let changes = changes(&current, &replacements);
    let input = fixture.input(current.basis());
    let mut handler = fixture.handler(&source, current.basis(), Some(&changes));
    handler.limits.maximum_pass_bytes = 1;
    assert_eq!(
        handler.stage(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &current
        ),
        Err(WorldStagingError::Capacity)
    );
    assert_eq!(source.calls.get(), 0);
    handler.limits.maximum_pass_bytes = 4 * 1024 * 1024;
    handler.limits.world.output_bytes = 1;
    assert_eq!(
        handler.stage(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &current
        ),
        Err(WorldStagingError::World(
            EnvironmentalDeltaError::OutputCapacity
        ))
    );
    assert_eq!(current, fixture.current());
}

#[test]
fn original_source_policy_and_private_cause_audience_are_revalidated() {
    let fixture = Fixture::new();
    let source = SourceOwner::new(&fixture);
    for case in ["policy", "audience"] {
        let mut state = fixture.state();
        if case == "policy" {
            state.decisions[1].source_policy = fixture::label("other-source-policy");
        } else {
            state.facts[1].audience = AudienceScope::Shared;
        }
        let current = fixture.checkpoint(fixture::basis(), state);
        assert_eq!(
            registered(&fixture, &source, &current, &fixture.input(current.basis())),
            Err(CommandRejection::Invocation(InvocationError::Handler(
                PreconditionedRejection::Handler(WorldStagingError::Source(SourceRefusal::Cause))
            )))
        );
        assert_eq!(current.state().decisions.len(), 2);
    }
}

#[test]
fn historical_accepted_cause_cannot_be_reapplied_at_a_later_current_revision() {
    let fixture = Fixture::new();
    let source = SourceOwner::new(&fixture);
    let mut basis = fixture::basis();
    basis.revision = basis.revision.next_sequence().unwrap();
    let current = fixture.checkpoint(basis, fixture.state());
    assert_eq!(
        registered(&fixture, &source, &current, &fixture.input(current.basis())),
        Err(CommandRejection::Invocation(InvocationError::Handler(
            PreconditionedRejection::Handler(WorldStagingError::World(
                EnvironmentalDeltaError::StaleFact
            ))
        )))
    );
    assert_eq!(current.state(), &fixture.state());
}

#[test]
fn direct_handler_rejects_client_actor_targets_and_observed_revision_without_source_calls() {
    let fixture = Fixture::new();
    let current = fixture.current();
    let source = SourceOwner::new(&fixture);
    let replacements = fixture.replacements();
    let changes = changes(&current, &replacements);
    let handler = fixture.handler(&source, current.basis(), Some(&changes));
    for case in ["actor", "targets", "observed"] {
        let mut input = fixture.input(current.basis());
        let GameInput::Game(command) = &mut input else {
            unreachable!()
        };
        if case == "observed" {
            command.observed_revision = fixture::revision(2, 7);
        } else {
            let GameCommand::ProposeAction { actor, targets, .. } = &mut command.command else {
                unreachable!()
            };
            if case == "actor" {
                *actor = fixture::entity(5);
            } else {
                targets.reverse();
            }
        }
        let reason = if case == "observed" {
            WorldStagingError::StaleCommand
        } else {
            WorldStagingError::WrongCommand
        };
        assert_eq!(
            handler.stage(
                RulesCommandInput {
                    command: &input,
                    supplied_draws: &[]
                },
                &current
            ),
            Err(reason)
        );
    }
    assert_eq!(source.calls.get(), 0);
    assert_eq!(current, fixture.current());
}

#[test]
fn explicitly_admitted_empty_batch_records_no_change_without_inventing_a_cause() {
    let fixture = Fixture::new();
    let current = fixture.current();
    let source = SourceOwner::new(&fixture);
    let handler = fixture.handler(&source, current.basis(), Some(&[]));
    let mut input = fixture.input(current.basis());
    let GameInput::Game(command) = &mut input else {
        unreachable!()
    };
    let GameCommand::ProposeAction { targets, .. } = &mut command.command else {
        unreachable!()
    };
    targets.clear();
    let next = registered_handler(&fixture, &current, &current, &input, &handler).unwrap();
    let mut expected = current.state().clone();
    expected
        .decisions
        .push(next.state().decisions.last().unwrap().clone());
    assert_eq!(next.state(), &expected);
    assert_eq!(source.calls.get(), 1);
    assert_eq!(current, fixture.current());
}

fn pending(basis: Basis) -> PendingResolution {
    PendingResolution {
        id: ResolutionId::from_bytes(&[9; 16]).unwrap(),
        basis,
        continuation: fixture::label("existing-rules-continuation"),
        window: ResolutionWindow {
            id: WindowId::from_bytes(&[10; 16]).unwrap(),
            phase: TriggerPhase::BeforeDraw,
            causal_fact: fact(31),
            source: fixture::rule(),
            timer: None,
        },
        next: PendingInput::Choice {
            remaining: vec![OfferedResponse {
                participant: fixture::member(3),
                offer: fixture::label("existing-rules-offer"),
                options: vec![fixture::label("existing-rules-choice")],
                source: fixture::rule(),
            }],
        },
        choices: vec![],
        draw_ordinals: vec![],
        spent: vec![],
        rulings: vec![],
    }
}

#[test]
fn supplied_actual_draw_and_existing_pending_resolution_cannot_enter_world_staging() {
    let fixture = Fixture::new();
    let current = fixture.current();
    let source = SourceOwner::new(&fixture);
    let replacements = fixture.replacements();
    let admitted = changes(&current, &replacements);
    let handler = fixture.handler(&source, current.basis(), Some(&admitted));
    let draw = ActualDraw {
        operation: operation(50),
        ordinal: 0,
        resolution: ResolutionId::from_bytes(&[9; 16]).unwrap(),
        window: WindowId::from_bytes(&[10; 16]).unwrap(),
        sides: 20,
        value: 1,
        source: fixture::rule(),
    };
    assert_eq!(
        registered_with_draws(
            &fixture,
            &current,
            &current,
            &fixture.input(current.basis()),
            &handler,
            &[draw]
        ),
        Err(CommandRejection::Invocation(InvocationError::Handler(
            PreconditionedRejection::Handler(WorldStagingError::UnsupportedInput)
        )))
    );
    assert_eq!(source.calls.get(), 0);
    assert_eq!(current, fixture.current());
    let mut state = fixture.state();
    state.pending.push(pending(current.basis()));
    let blocked = fixture.checkpoint(current.basis(), state);
    let replacements = fixture.replacements();
    let changes = changes(&blocked, &replacements);
    let handler = fixture.handler(&source, blocked.basis(), Some(&changes));
    assert_eq!(
        handler.stage(
            RulesCommandInput {
                command: &fixture.input(blocked.basis()),
                supplied_draws: &[]
            },
            &blocked
        ),
        Err(WorldStagingError::PendingResolution)
    );
    assert!(
        registered_handler(
            &fixture,
            &blocked,
            &blocked,
            &fixture.input(blocked.basis()),
            &handler
        )
        .is_err()
    );
    assert_eq!(source.calls.get(), 0);
    assert_eq!(blocked.state().pending.len(), 1);
    assert_eq!(blocked.state().decisions.len(), 2);
}

#[cfg(not(target_arch = "wasm32"))]
mod durable_session {
    use super::*;
    use df_observe::OperationContext;
    use df_session::inbox::{ActorInput, AdmissionSequence, Reducer};
    use df_session::submission::*;
    use df_types::{MemberId, SessionId};
    use std::cell::RefCell;
    use std::rc::Rc;

    // Reuses obligation_fulfillment's controlled ledger fixture pattern, now with the actual
    // registered environmental pipeline in SessionEngine. This is not physical PG/fence proof.
    #[derive(Clone, Eq, PartialEq)]
    struct Scope {
        input: GameInput,
        principal: MemberId,
    }
    impl ActorInput for Scope {
        fn retained_heap_bytes(&self) -> Option<usize> {
            self.input.retained_heap_bytes()
        }
    }
    impl OperationScope for Scope {
        type UncertaintyKey = Scope;
        fn capture_uncertainty_key(
            &self,
            maximum_retained_bytes: usize,
        ) -> Result<Scope, RepositoryError> {
            if self
                .input
                .retained_bytes()
                .and_then(|retained_bytes| retained_bytes.checked_add(std::mem::size_of::<Scope>()))
                .is_none_or(|retained_bytes| retained_bytes > maximum_retained_bytes)
            {
                return Err(RepositoryError::Capacity);
            }
            Ok(self.clone())
        }
        fn session(&self) -> SessionId {
            let GameInput::Game(command) = &self.input else {
                unreachable!()
            };
            command.basis.session
        }
        fn operation(&self) -> OperationId {
            let GameInput::Game(command) = &self.input else {
                unreachable!()
            };
            command.operation
        }
        fn validate_input(&self, input: &GameInput) -> Result<(), RepositoryError> {
            if &self.input != input {
                return Err(RepositoryError::InputBinding);
            }
            let GameInput::Game(command) = input else {
                return Err(RepositoryError::InputBinding);
            };
            if command.member != self.principal || self.principal != fixture::member(3) {
                return Err(RepositoryError::Unauthorized);
            }
            Ok(())
        }
        fn is_lookup_only(&self) -> bool {
            false
        }
    }
    #[derive(Clone, Copy, Eq, PartialEq)]
    enum Failure {
        None,
        BeforeCommit,
        LostAck,
        UnknownNotCommitted,
        Publication,
        SourceWithdrawn,
        RulesPending,
    }
    struct Database {
        checkpoint: Checkpoint,
        ledger: Vec<(Scope, DecisionReceipt)>,
        failure: Failure,
        commits: usize,
        decisions: usize,
        publications: usize,
        wakes: usize,
    }
    struct Repository {
        database: Rc<RefCell<Database>>,
    }
    impl SessionRepository for Repository {
        type Scope = Scope;
        fn lookup_operation(
            &mut self,
            scope: &Scope,
            _: &OperationContext,
        ) -> Result<OperationLookup, RepositoryError> {
            scope.validate_input(&scope.input)?;
            let db = self.database.borrow();
            if let Some((prior, receipt)) = db
                .ledger
                .iter()
                .find(|(prior, _)| prior.operation() == scope.operation())
            {
                return Ok(if prior == scope {
                    OperationLookup::Committed(receipt.clone())
                } else {
                    OperationLookup::Conflict
                });
            }
            Ok(OperationLookup::NotRecorded)
        }
        fn commit_decision(
            &mut self,
            scope: &Scope,
            candidate: &Checkpoint,
            expected: Basis,
            _: &OperationContext,
        ) -> Result<CommitOutcome, RepositoryError> {
            scope.validate_input(&scope.input)?;
            let mut db = self.database.borrow_mut();
            db.commits += 1;
            if expected != db.checkpoint.basis() {
                return Err(RepositoryError::RevisionConflict);
            }
            if db.failure == Failure::BeforeCommit {
                return Err(RepositoryError::Unavailable);
            }
            if db.failure == Failure::UnknownNotCommitted {
                return Ok(CommitOutcome::Indeterminate);
            }
            let decision = candidate
                .state()
                .decisions
                .iter()
                .find(|decision| decision.operation == scope.operation())
                .unwrap()
                .clone();
            let receipt = DecisionReceipt::new(candidate.basis(), decision, 64 * 1024)?;
            db.checkpoint = candidate.clone();
            db.ledger.push((scope.clone(), receipt.clone()));
            Ok(if db.failure == Failure::LostAck {
                CommitOutcome::Indeterminate
            } else {
                CommitOutcome::Confirmed(receipt)
            })
        }
        fn load_current(
            &mut self,
            scope: &Scope,
            _: &OperationContext,
        ) -> Result<Checkpoint, RepositoryError> {
            scope.validate_input(&scope.input)?;
            Ok(self.database.borrow().checkpoint.clone())
        }
    }
    struct Engine {
        fixture: Fixture,
        source: SourceOwner,
        database: Rc<RefCell<Database>>,
    }
    impl SessionEngine<Scope> for Engine {
        fn decide(
            &mut self,
            current: &Checkpoint,
            scope: &Scope,
            input: &GameInput,
        ) -> Result<Checkpoint, RepositoryError> {
            scope.validate_input(input)?;
            self.database.borrow_mut().decisions += 1;
            if self.database.borrow().failure == Failure::SourceWithdrawn {
                self.source.refusal.set(Some(SourceRefusal::Withdrawn));
            }
            registered(&self.fixture, &self.source, current, input)
                .map_err(|_| RepositoryError::InvalidCandidate)
        }
        fn validate_recovery(&mut self, current: &Checkpoint) -> Result<(), RepositoryError> {
            current
                .validate_resume(current.basis(), &self.fixture.pins)
                .map(|_| ())
                .map_err(|_| RepositoryError::InvalidCandidate)
        }
    }
    struct Publication {
        database: Rc<RefCell<Database>>,
    }
    impl PublicationOwner<Scope> for Publication {
        fn publish_committed(
            &mut self,
            scope: &Scope,
            checkpoint: &Checkpoint,
        ) -> Result<(), DeliveryError> {
            let mut db = self.database.borrow_mut();
            assert_eq!(checkpoint, &db.checkpoint);
            assert!(db.ledger.iter().any(|(prior,receipt)| prior==scope && receipt.basis()==checkpoint.basis()));
            assert_eq!(
                checkpoint.state().continuity.environment[0].definition,
                content("authored-storm")
            );
            db.publications += 1;
            if db.failure == Failure::Publication {
                return Err(DeliveryError::Unavailable);
            }
            Ok(())
        }
        fn wake_committed_intents(&mut self, scope: &Scope) -> Result<(), DeliveryError> {
            let mut db = self.database.borrow_mut();
            assert!(db.ledger.iter().any(|(prior, _)| prior == scope));
            assert!(db.publications > 0);
            assert!(db.checkpoint.state().intents.is_empty());
            db.wakes += 1;
            Ok(())
        }
    }
    type Owner = DurableOwner<Repository, Engine, Publication>;
    fn setup(failure: Failure) -> (Owner, Rc<RefCell<Database>>, Scope) {
        let fixture = Fixture::new();
        let mut state = fixture.state();
        if failure == Failure::RulesPending {
            state.pending.push(pending(fixture::basis()));
        }
        let current = fixture.checkpoint(fixture::basis(), state);
        let scope = Scope {
            input: fixture.input(current.basis()),
            principal: fixture::member(3),
        };
        let source = SourceOwner::new(&fixture);
        let db = Rc::new(RefCell::new(Database {
            checkpoint: current.clone(),
            ledger: vec![],
            failure,
            commits: 0,
            decisions: 0,
            publications: 0,
            wakes: 0,
        }));
        let owner = DurableOwner::new(
            Repository {
                database: Rc::clone(&db),
            },
            Engine {
                fixture,
                source,
                database: Rc::clone(&db),
            },
            Publication {
                database: Rc::clone(&db),
            },
            current,
            64 * 1024,
        )
        .unwrap();
        (owner, db, scope)
    }
    fn submit(owner: &mut Owner, scope: &Scope) -> SubmissionOutcome {
        let (input, wait) = OwnedInput::new(
            OperationContext {
                trace_parent: String::new(),
                build: "environment-fixture".to_owned(),
            },
            scope.clone(),
            scope.input.clone(),
        );
        owner.reduce(AdmissionSequence(1), input);
        wait.try_recv().unwrap()
    }
    fn assert_preserved(original: &Checkpoint, next: &Checkpoint) {
        let mut expected = original.state().clone();
        expected.continuity.environment = Fixture::new().replacements();
        expected
            .decisions
            .push(next.state().decisions.last().unwrap().clone());
        assert_eq!(next.state(), &expected);
        assert_eq!(next.pins(), original.pins());
    }
    #[test]
    fn committed_world_batch_publishes_then_wakes_once_and_exact_retry_returns_original_receipt() {
        let (mut owner, db, scope) = setup(Failure::None);
        let original = owner.checkpoint().clone();
        let receipt = submit(&mut owner, &scope);
        assert!(matches!(receipt, SubmissionOutcome::Confirmed(_)));
        assert_eq!(submit(&mut owner, &scope), receipt);
        let db = db.borrow();
        assert_eq!(owner.checkpoint(), &db.checkpoint);
        assert_preserved(&original, &db.checkpoint);
        assert_eq!(
            (
                db.decisions,
                db.commits,
                db.publications,
                db.wakes,
                db.ledger.len()
            ),
            (1, 1, 1, 1, 1)
        );
    }
    #[test]
    fn rollback_retains_entire_original_and_safe_retry_stages_one_complete_batch() {
        let (mut owner, db, scope) = setup(Failure::BeforeCommit);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Refused(RepositoryError::Unavailable)
        );
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!(db.borrow().checkpoint, original);
        assert!(db.borrow().ledger.is_empty());
        assert_eq!((db.borrow().publications, db.borrow().wakes), (0, 0));
        db.borrow_mut().failure = Failure::None;
        assert!(matches!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Confirmed(_)
        ));
        assert_preserved(&original, &db.borrow().checkpoint);
        assert_eq!(
            (
                db.borrow().decisions,
                db.borrow().commits,
                db.borrow().ledger.len()
            ),
            (2, 2, 1)
        );
    }
    #[test]
    fn unknown_absent_commit_blocks_reexecution_and_unrelated_operations() {
        let (mut owner, db, scope) = setup(Failure::UnknownNotCommitted);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::LookupRequired
        );
        db.borrow_mut().failure = Failure::None;
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::LookupRequired
        );
        let mut other = scope.clone();
        let GameInput::Game(command) = &mut other.input else {
            unreachable!()
        };
        command.operation = operation(51);
        assert_eq!(
            submit(&mut owner, &other),
            SubmissionOutcome::LookupRequired
        );
        assert!(owner.has_uncertain_operation());
        let db = db.borrow();
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!(db.checkpoint, original);
        assert_eq!(
            (
                db.decisions,
                db.commits,
                db.publications,
                db.wakes,
                db.ledger.len()
            ),
            (1, 1, 0, 0, 0)
        );
    }
    #[test]
    fn lost_ack_recovers_exact_committed_batch_without_restage_or_premature_publication() {
        let (mut owner, db, scope) = setup(Failure::LostAck);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::LookupRequired
        );
        assert_eq!(owner.checkpoint(), &original);
        assert_preserved(&original, &db.borrow().checkpoint);
        assert_eq!((db.borrow().publications, db.borrow().wakes), (0, 0));
        let result = submit(&mut owner, &scope);
        assert!(matches!(result, SubmissionOutcome::Confirmed(_)));
        assert!(owner.is_current());
        assert_eq!(submit(&mut owner, &scope), result);
        let db = db.borrow();
        assert_eq!(owner.checkpoint(), &db.checkpoint);
        assert_eq!(
            (
                db.decisions,
                db.commits,
                db.publications,
                db.wakes,
                db.ledger.len()
            ),
            (1, 1, 0, 0, 1)
        );
    }
    #[test]
    fn failed_publication_does_not_undo_commit_or_authorize_another_world_pass() {
        let (mut owner, db, scope) = setup(Failure::Publication);
        let original = owner.checkpoint().clone();
        let result = submit(&mut owner, &scope);
        assert!(matches!(result, SubmissionOutcome::Confirmed(_)));
        assert_eq!(submit(&mut owner, &scope), result);
        assert_preserved(&original, &db.borrow().checkpoint);
        assert_eq!(owner.checkpoint(), &db.borrow().checkpoint);
        assert_eq!(
            (
                db.borrow().decisions,
                db.borrow().commits,
                db.borrow().publications,
                db.borrow().ledger.len()
            ),
            (1, 1, 1, 1)
        );
    }
    #[test]
    fn trusted_principal_and_operation_conflict_refuse_without_staging_or_commit() {
        let (mut owner, db, scope) = setup(Failure::None);
        let original = owner.checkpoint().clone();
        let mut forged = scope.clone();
        forged.principal = fixture::member(9);
        assert_eq!(
            submit(&mut owner, &forged),
            SubmissionOutcome::Refused(RepositoryError::Unauthorized)
        );
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!((db.borrow().decisions, db.borrow().commits), (0, 0));
        assert!(matches!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Confirmed(_)
        ));
        let mut conflict = scope.clone();
        let GameInput::Game(command) = &mut conflict.input else {
            unreachable!()
        };
        command.command = GameCommand::Speak {
            speaker: fixture::entity(4),
            text: "different exact operation input".to_owned(),
            conversation: None,
        };
        assert_eq!(
            submit(&mut owner, &conflict),
            SubmissionOutcome::OperationConflict
        );
        assert_eq!(
            (
                db.borrow().decisions,
                db.borrow().commits,
                db.borrow().ledger.len()
            ),
            (1, 1, 1)
        );
    }
    #[test]
    fn session_source_refusal_never_reaches_repository_commit_or_publication() {
        let (mut owner, db, scope) = setup(Failure::SourceWithdrawn);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Refused(RepositoryError::InvalidCandidate)
        );
        let db = db.borrow();
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!(db.checkpoint, original);
        assert_eq!(
            (
                db.decisions,
                db.commits,
                db.publications,
                db.wakes,
                db.ledger.len()
            ),
            (1, 0, 0, 0, 0)
        );
    }
    #[test]
    fn session_pending_rules_refusal_preserves_complete_continuation_without_commit_or_wake() {
        let (mut owner, db, scope) = setup(Failure::RulesPending);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Refused(RepositoryError::InvalidCandidate)
        );
        let db = db.borrow();
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!(db.checkpoint, original);
        assert_eq!(
            (
                db.decisions,
                db.commits,
                db.publications,
                db.wakes,
                db.ledger.len()
            ),
            (1, 0, 0, 0, 0)
        );
    }
}

mod accepted_time_schedule {
    use super::*;
    use df_engine::director_staging::{DirectorLimits, ScheduleDirectorLimits};
    use df_world::{AdmittedScheduleDestination, DueSelectionLimits, ScheduleAdvancementLimits};
    use std::time::Duration;

    #[derive(Clone, Copy)]
    enum Fault {
        None,
        Withdrawn,
        MissingTime,
        WrongTime,
        RepeatedTime,
        MissingMovement,
        WrongMovement,
        ExtraMovement,
        ChangedSchedule,
        Pending,
    }

    // Deliberately finite compiled source fixture. It does not qualify a production time source,
    // persisted host pause, travel/rest mechanics, source rights or native durable persistence.
    struct TimeProducer<'a> {
        fixture: &'a Fixture,
        target: LogicalTime,
        destination_ids: &'a [u8],
        fault: Fault,
        calls: Cell<usize>,
    }
    impl<'a> TimeProducer<'a> {
        fn new(fixture: &'a Fixture, destination_ids: &'a [u8]) -> Self {
            Self {
                fixture,
                target: LogicalTime {
                    ticks: 125,
                    ticks_per_second: 10,
                },
                destination_ids,
                fault: Fault::None,
                calls: Cell::new(0),
            }
        }
    }
    impl RulesCommandHandler for TimeProducer<'_> {
        type Rejection = SourceRefusal;
        fn pins(&self) -> &CheckpointPins {
            &self.fixture.pins
        }
        fn bound_source(&self) -> Option<&RuleReference> {
            Some(&self.fixture.registration.source)
        }
        fn stage(
            &self,
            input: RulesCommandInput<'_>,
            current: &Checkpoint,
        ) -> Result<Checkpoint, Self::Rejection> {
            self.calls.set(self.calls.get() + 1);
            if matches!(self.fault, Fault::Withdrawn) {
                return Err(SourceRefusal::Withdrawn);
            }
            let GameInput::Game(command) = input.command else {
                return Err(SourceRefusal::ActorControl);
            };
            let GameInput::Game(expected) = self.fixture.input(current.basis()) else {
                unreachable!()
            };
            if command.member != expected.member || command.command != expected.command {
                return Err(SourceRefusal::ActorControl);
            }
            let mut basis = current.basis();
            basis.revision = basis.revision.next_sequence().unwrap();
            let mut state = current.state().clone();
            state.logical_time = self.target;
            let start = state.facts.len();
            let base_fact_id = 200 + (command.operation.as_bytes()[0] % 10) * 5;
            let mut values = Vec::new();
            if !matches!(self.fault, Fault::MissingTime) {
                let mut before = current.state().logical_time;
                if matches!(self.fault, Fault::WrongTime) {
                    before.ticks -= 1;
                }
                values.push(FactValue::TimeAdvanced {
                    before,
                    after: self.target,
                });
                if matches!(self.fault, Fault::RepeatedTime) {
                    values.push(FactValue::TimeAdvanced {
                        before,
                        after: self.target,
                    });
                }
            }
            state.draws.extend_from_slice(input.supplied_draws);
            for draw in input.supplied_draws {
                values.push(FactValue::DrawAccepted {
                    operation: draw.operation,
                    ordinal: draw.ordinal,
                });
            }
            for destination in self.destination_ids {
                state
                    .entities
                    .iter_mut()
                    .find(|entity| entity.id == fixture::entity(4))
                    .unwrap()
                    .location = Some(fixture::entity(*destination));
                if !matches!(self.fault, Fault::MissingMovement) {
                    values.push(FactValue::EntityMoved {
                        entity: fixture::entity(4),
                        destination: fixture::entity(
                            if matches!(self.fault, Fault::WrongMovement) {
                                5
                            } else {
                                *destination
                            },
                        ),
                        position: Position { x: 0, y: 0, z: 0 },
                    });
                }
            }
            if matches!(self.fault, Fault::ExtraMovement) {
                values.push(FactValue::EntityMoved {
                    entity: fixture::entity(5),
                    destination: fixture::entity(6),
                    position: Position { x: 0, y: 0, z: 0 },
                });
            }
            for (ordinal, value) in values.into_iter().enumerate() {
                state.facts.push(GameFact {
                    id: fact(base_fact_id + ordinal as u8),
                    revision: basis.revision,
                    operation: command.operation,
                    ordinal: ordinal as u32,
                    cause: None,
                    audience: AudienceScope::Members(vec![command.member]),
                    value,
                });
            }
            state.decisions.push(AcceptedDecision {
                operation: command.operation,
                revision: basis.revision,
                facts: state.facts[start..].iter().map(|fact| fact.id).collect(),
                draws: input
                    .supplied_draws
                    .iter()
                    .map(|draw| draw.ordinal)
                    .collect(),
                effects: vec![],
                source_policy: fixture::label("synthetic-accepted-time-source"),
                semantic_output: None,
            });
            if matches!(self.fault, Fault::ChangedSchedule) {
                state.schedules.pop();
            }
            if matches!(self.fault, Fault::Pending) {
                let mut pending = pending(basis);
                pending.window.causal_fact = state.facts[start].id;
                state.pending.push(pending);
            }
            state.facts.shrink_to_fit();
            state.decisions.shrink_to_fit();
            Checkpoint::new(
                current.schema(),
                basis,
                self.pins().clone(),
                state,
                self.fixture.inventory(),
                fixture::limits(),
            )
            .map_err(|_| SourceRefusal::Binding)
        }
    }

    fn current(fixture: &Fixture) -> Checkpoint {
        let mut state = fixture.state();
        for (id, ticks) in [(80, 121), (81, 122), (82, 130)] {
            state.schedules.push(ScheduledEvent {
                id: RecordId::from_bytes(&[id; 16]).unwrap(),
                definition: fixture::content(),
                entity: fixture::entity(4),
                due: LogicalTime {
                    ticks,
                    ticks_per_second: 10,
                },
            });
        }
        fixture.checkpoint(fixture::basis(), state)
    }
    fn destinations(current: &Checkpoint) -> Vec<AdmittedScheduleDestination<'_>> {
        current
            .state()
            .schedules
            .iter()
            .zip([(None, 5), (Some(5), 6), (Some(6), 5)])
            .map(|(event, (before, after))| AdmittedScheduleDestination {
                event: event.id,
                definition: &event.definition,
                entity: event.entity,
                expected_location: before.map(fixture::entity),
                destination: fixture::entity(after),
            })
            .collect()
    }
    fn limits() -> ScheduleDirectorLimits {
        let selection = DueSelectionLimits {
            queue_events: 32,
            selected_events: 2,
            output_bytes: 32 * 1024,
        };
        ScheduleDirectorLimits {
            directors: DirectorLimits {
                maximum_checkpoint_bytes: 1024 * 1024,
                maximum_pass_bytes: 8 * 1024 * 1024,
                maximum_relationships: 32,
                world: selection,
            },
            schedule: ScheduleAdvancementLimits {
                selection,
                entities: 32,
                destinations: 32,
                movements: 32,
                output_bytes: 64 * 1024,
            },
            checkpoint: fixture::limits(),
        }
    }
    type Error = CommandRejection<TimeScheduleError<SourceRefusal>>;
    fn registered(
        fixture: &Fixture,
        current: &Checkpoint,
        input: &GameInput,
        producer: &TimeProducer<'_>,
        destinations: &[AdmittedScheduleDestination<'_>],
        limits: ScheduleDirectorLimits,
        paused: bool,
    ) -> Result<Checkpoint, Error> {
        registered_with_input(
            fixture,
            current,
            RulesCommandInput {
                command: input,
                supplied_draws: &[],
            },
            producer,
            destinations,
            limits,
            paused,
        )
    }
    fn registered_with_input(
        fixture: &Fixture,
        current: &Checkpoint,
        input: RulesCommandInput<'_>,
        producer: &TimeProducer<'_>,
        destinations: &[AdmittedScheduleDestination<'_>],
        limits: ScheduleDirectorLimits,
        paused: bool,
    ) -> Result<Checkpoint, Error> {
        let handler = AcceptedTimeScheduleHandler {
            producer,
            current_basis: current.basis(),
            inventory: fixture.inventory(),
            paused,
            deadline_remaining: Duration::from_secs(1),
            policy: &fixture.content[0],
            destinations,
            limits,
        };
        let selector = fixture::label("synthetic-accepted-time-source");
        let entries = [CatalogEntry::new(
            &fixture.registration.source,
            b"synthetic time source",
        )];
        let catalog = CatalogSnapshot::from_published(
            &fixture.pins.rules.catalog,
            &fixture.pins,
            b"synthetic complete time catalog",
            &entries,
            CatalogLimits {
                max_complete_bytes: 64,
                max_entries: 1,
                max_item_bytes: 64,
                max_total_item_bytes: 64,
            },
        )
        .unwrap();
        let registrations = [HandlerRegistration::new(
            &selector,
            &fixture.registration.source,
            &handler,
        )];
        let registry = DispatchRegistry::from_catalog(catalog, &registrations, 1).unwrap();
        decide_registered_command(
            input,
            current,
            CommandEntryContext {
                current_basis: current.basis(),
                admitted_pins: &fixture.pins,
                inventory: fixture.inventory(),
                limits: CommandEntryLimits {
                    command: command_limits(),
                    maximum_staged_bytes: 1024 * 1024,
                },
            },
            &registry,
            &selector,
            &fixture.registration.source,
        )
    }
    fn refused(error: TimeScheduleError<SourceRefusal>) -> Error {
        CommandRejection::Invocation(InvocationError::Handler(error))
    }

    #[test]
    fn registered_time_and_ordered_movement_facts_compose_one_exact_source_candidate() {
        let fixture = Fixture::new();
        let current = current(&fixture);
        let original = current.clone();
        let producer = TimeProducer::new(&fixture, &[5, 6]);
        let next = registered(
            &fixture,
            &current,
            &fixture.input(current.basis()),
            &producer,
            &destinations(&current),
            limits(),
            false,
        )
        .unwrap();
        let mut expected = original.state().clone();
        expected.logical_time = producer.target;
        expected.entities[0].location = Some(fixture::entity(6));
        expected
            .facts
            .extend_from_slice(&next.state().facts[original.state().facts.len()..]);
        expected
            .decisions
            .push(next.state().decisions.last().unwrap().clone());
        expected.continuity.catch_up = next.state().continuity.catch_up.clone();
        assert_eq!(next.state(), &expected);
        assert_eq!(current, original);
        assert_eq!(producer.calls.get(), 1);
        assert_eq!(next.state().decisions.last().unwrap().facts.len(), 3);
        let cursor = next.state().continuity.catch_up.as_ref().unwrap();
        assert_eq!(cursor.last_processed, Some(current.state().schedules[1].id));
        assert_eq!(cursor.time, producer.target);
        assert!(cursor.pending_events.is_empty());
        assert_eq!(next.pins(), current.pins());
    }

    #[test]
    fn paused_time_calls_no_producer_and_preserves_complete_world_and_pending_state() {
        let fixture = Fixture::new();
        let mut producer = TimeProducer::new(&fixture, &[5, 6]);
        producer.target.ticks = 999_999;
        for has_pending in [false, true] {
            let mut state = current(&fixture).state().clone();
            if has_pending {
                state.pending.push(pending(fixture::basis()));
            }
            let current = fixture.checkpoint(fixture::basis(), state);
            let original = current.clone();
            assert_eq!(
                registered(
                    &fixture,
                    &current,
                    &fixture.input(current.basis()),
                    &producer,
                    &destinations(&current),
                    limits(),
                    true,
                ),
                Err(refused(TimeScheduleError::Paused))
            );
            assert_eq!(current, original);
            assert_eq!(producer.calls.get(), 0);
        }
    }

    #[test]
    fn bounded_resume_keeps_backlog_and_never_repeats_processed_event_or_accepted_operation() {
        let fixture = Fixture::new();
        let current = current(&fixture);
        let first_producer = TimeProducer::new(&fixture, &[5]);
        let mut bounded = limits();
        bounded.directors.world.selected_events = 1;
        bounded.schedule.selection = bounded.directors.world;
        let first = registered(
            &fixture,
            &current,
            &fixture.input(current.basis()),
            &first_producer,
            &destinations(&current),
            bounded,
            false,
        )
        .unwrap();
        let cursor = first.state().continuity.catch_up.as_ref().unwrap();
        assert_eq!(cursor.last_processed, Some(current.state().schedules[0].id));
        assert_eq!(cursor.pending_events, vec![current.state().schedules[1].id]);
        let resumed_producer = TimeProducer::new(&fixture, &[6]);
        let mut input = fixture.input(first.basis());
        let GameInput::Game(command) = &mut input else {
            unreachable!()
        };
        command.operation = operation(51);
        let resumed = registered(
            &fixture,
            &first,
            &input,
            &resumed_producer,
            &destinations(&first),
            bounded,
            false,
        )
        .unwrap();
        assert_eq!(resumed.state().logical_time, first.state().logical_time);
        assert_eq!(
            resumed.state().entities[0].location,
            Some(fixture::entity(6))
        );
        assert!(
            resumed
                .state()
                .continuity
                .catch_up
                .as_ref()
                .unwrap()
                .pending_events
                .is_empty()
        );
        assert_eq!(resumed.state().facts.len() - first.state().facts.len(), 2);
        let duplicate = fixture.input(first.basis());
        assert_eq!(
            registered(
                &fixture,
                &first,
                &duplicate,
                &first_producer,
                &destinations(&first),
                bounded,
                false,
            ),
            Err(CommandRejection::Invocation(
                InvocationError::AlreadyAccepted
            ))
        );
        assert_eq!(first_producer.calls.get(), 1);
    }

    #[test]
    fn source_withdrawal_and_stale_command_leave_no_time_or_movement_candidate() {
        let fixture = Fixture::new();
        let current = current(&fixture);
        let mut producer = TimeProducer::new(&fixture, &[5, 6]);
        producer.fault = Fault::Withdrawn;
        assert_eq!(
            registered(
                &fixture,
                &current,
                &fixture.input(current.basis()),
                &producer,
                &destinations(&current),
                limits(),
                false,
            ),
            Err(refused(TimeScheduleError::Producer(
                InvocationError::Handler(SourceRefusal::Withdrawn),
            )))
        );
        let mut old = fixture.input(current.basis());
        let GameInput::Game(command) = &mut old else {
            unreachable!()
        };
        command.observed_revision = fixture::revision(2, 7);
        assert!(
            registered(
                &fixture,
                &current,
                &old,
                &producer,
                &destinations(&current),
                limits(),
                false,
            )
            .is_err()
        );
        assert_eq!(producer.calls.get(), 1);
        assert_eq!(current, self::current(&fixture));
    }

    #[test]
    fn missing_wrong_and_repeated_time_facts_cannot_authorize_schedule_advancement() {
        let fixture = Fixture::new();
        let current = current(&fixture);
        for fault in [Fault::MissingTime, Fault::WrongTime, Fault::RepeatedTime] {
            let mut producer = TimeProducer::new(&fixture, &[5, 6]);
            producer.fault = fault;
            assert_eq!(
                registered(
                    &fixture,
                    &current,
                    &fixture.input(current.basis()),
                    &producer,
                    &destinations(&current),
                    limits(),
                    false,
                ),
                Err(refused(TimeScheduleError::InvalidTimeFact))
            );
            assert_eq!(current, self::current(&fixture));
        }
    }

    #[test]
    fn every_selected_movement_requires_its_exact_ordered_source_fact() {
        let fixture = Fixture::new();
        let current = current(&fixture);
        for fault in [
            Fault::MissingMovement,
            Fault::WrongMovement,
            Fault::ExtraMovement,
        ] {
            let mut producer = TimeProducer::new(&fixture, &[5, 6]);
            producer.fault = fault;
            assert_eq!(
                registered(
                    &fixture,
                    &current,
                    &fixture.input(current.basis()),
                    &producer,
                    &destinations(&current),
                    limits(),
                    false,
                ),
                Err(refused(TimeScheduleError::MissingMovementFact))
            );
            assert_eq!(current, self::current(&fixture));
        }
    }

    #[test]
    fn pending_rules_unmapped_due_event_and_source_schedule_change_refuse_atomically() {
        let fixture = Fixture::new();
        let current = current(&fixture);
        let producer = TimeProducer::new(&fixture, &[5, 6]);
        assert_eq!(
            registered(
                &fixture,
                &current,
                &fixture.input(current.basis()),
                &producer,
                &[],
                limits(),
                false,
            ),
            Err(refused(TimeScheduleError::WorldPending))
        );
        for (fault, expected) in [
            (
                Fault::ChangedSchedule,
                TimeScheduleError::SourceWorldConflict,
            ),
            (Fault::Pending, TimeScheduleError::PendingResolution),
        ] {
            let mut producer = TimeProducer::new(&fixture, &[5, 6]);
            producer.fault = fault;
            assert_eq!(
                registered(
                    &fixture,
                    &current,
                    &fixture.input(current.basis()),
                    &producer,
                    &destinations(&current),
                    limits(),
                    false,
                ),
                Err(refused(expected))
            );
        }
        let mut state = current.state().clone();
        state.pending.push(pending(current.basis()));
        let blocked = fixture.checkpoint(current.basis(), state);
        let untouched = TimeProducer::new(&fixture, &[5, 6]);
        assert_eq!(
            registered(
                &fixture,
                &blocked,
                &fixture.input(blocked.basis()),
                &untouched,
                &destinations(&blocked),
                limits(),
                false,
            ),
            Err(refused(TimeScheduleError::PendingResolution))
        );
        assert_eq!(untouched.calls.get(), 0);
        assert_eq!(current, self::current(&fixture));
    }

    #[test]
    fn pass_budget_due_output_and_final_checkpoint_refusals_never_publish_partial_world() {
        let fixture = Fixture::new();
        let current = current(&fixture);
        let producer = TimeProducer::new(&fixture, &[5, 6]);
        let mut bounded = limits();
        bounded.directors.maximum_pass_bytes = 1;
        assert_eq!(
            registered(
                &fixture,
                &current,
                &fixture.input(current.basis()),
                &producer,
                &destinations(&current),
                bounded,
                false,
            ),
            Err(refused(TimeScheduleError::Capacity))
        );
        assert_eq!(producer.calls.get(), 0);
        let input = fixture.input(current.basis());
        let raw = producer
            .stage(
                RulesCommandInput {
                    command: &input,
                    supplied_draws: &[],
                },
                &current,
            )
            .unwrap();
        producer.calls.set(0);
        let mut bounded = limits();
        bounded.checkpoint.maximum_retained_bytes = raw.retained_bytes().unwrap();
        assert!(matches!(
            registered(
                &fixture,
                &current,
                &input,
                &producer,
                &destinations(&current),
                bounded,
                false,
            ),
            Err(CommandRejection::Invocation(InvocationError::Handler(
                TimeScheduleError::InvalidCandidate(CheckpointError::Capacity)
            )))
        ));
        assert_eq!(producer.calls.get(), 1);
        let mut bounded = limits();
        bounded.directors.world.output_bytes = 1;
        bounded.schedule.selection = bounded.directors.world;
        assert!(matches!(
            registered(
                &fixture,
                &current,
                &input,
                &producer,
                &destinations(&current),
                bounded,
                false,
            ),
            Err(CommandRejection::Invocation(InvocationError::Handler(
                TimeScheduleError::World(_)
            )))
        ));
        assert_eq!(current, self::current(&fixture));
    }

    #[test]
    fn unchanged_source_inputs_replay_exact_candidate_without_a_clock_or_new_draw() {
        let fixture = Fixture::new();
        let current = current(&fixture);
        let producer = TimeProducer::new(&fixture, &[5, 6]);
        let input = fixture.input(current.basis());
        let first = registered(
            &fixture,
            &current,
            &input,
            &producer,
            &destinations(&current),
            limits(),
            false,
        )
        .unwrap();
        let second = registered(
            &fixture,
            &current,
            &input,
            &producer,
            &destinations(&current),
            limits(),
            false,
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(current, self::current(&fixture));
        assert_eq!(first.state().draws, current.state().draws);
        assert_eq!(first.state().resources, current.state().resources);
        assert_eq!(first.state().threats, current.state().threats);
    }
    #[test]
    fn actual_supplied_draws_remain_owned_by_the_same_source_decision() {
        let fixture = Fixture::new();
        let current = current(&fixture);
        let producer = TimeProducer::new(&fixture, &[5, 6]);
        let input = fixture.input(current.basis());
        let draws = [ActualDraw {
            operation: operation(50),
            ordinal: 0,
            resolution: ResolutionId::from_bytes(&[9; 16]).unwrap(),
            window: WindowId::from_bytes(&[10; 16]).unwrap(),
            sides: 20,
            value: 7,
            source: fixture::rule(),
        }];
        let next = registered_with_input(
            &fixture,
            &current,
            RulesCommandInput {
                command: &input,
                supplied_draws: &draws,
            },
            &producer,
            &destinations(&current),
            limits(),
            false,
        )
        .unwrap();
        assert_eq!(next.state().draws, draws);
        assert_eq!(next.state().decisions.last().unwrap().draws, vec![0]);
        assert!(next.state().facts.iter().any(|fact| matches!(
            fact.value, FactValue::DrawAccepted { operation: id, ordinal: 0 }
                if id == operation(50)
        )));
        assert_eq!(producer.calls.get(), 1);
        assert_eq!(current, self::current(&fixture));
    }
}
