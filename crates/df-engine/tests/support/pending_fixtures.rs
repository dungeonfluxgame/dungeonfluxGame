use df_model::checkpoint::*;
use df_model::commands::CommandLimits;
use df_types::{
    BuildIdentity, MemberId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId,
    SessionRevision,
};

pub(super) fn label(label_text: &str) -> RevisionLabel {
    RevisionLabel::new(Some(label_text)).unwrap()
}

pub(super) fn entity(repeated_byte: u8) -> EntityId {
    EntityId::from_bytes(&[repeated_byte; 16]).unwrap()
}

pub(super) fn member(repeated_byte: u8) -> MemberId {
    MemberId::from_bytes(&[repeated_byte; 16]).unwrap()
}

pub(super) fn revision(epoch: u64, sequence: u64) -> SessionRevision {
    SessionRevision::new(RecoveryEpoch::new(epoch).unwrap(), sequence)
}

pub(super) fn basis() -> Basis {
    Basis {
        session: SessionId::from_bytes(&[1; 16]).unwrap(),
        run: RunId::from_bytes(&[2; 16]).unwrap(),
        revision: revision(2, 8),
    }
}

pub(super) fn content() -> ContentReference {
    ContentReference {
        package: label("fixture-package-1"),
        entry: label("fixture-entry-1"),
    }
}

pub(super) fn rule() -> RuleReference {
    RuleReference {
        catalog: label("fixture-catalog-1"),
        source: label("fixture-source-1"),
        entry: label("fixture-entry-1"),
        clause: label("fixture-clause-1"),
    }
}

pub(super) fn pins() -> CheckpointPins {
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

pub(super) fn resource_constraints() -> Vec<ResourceConstraint> {
    vec![ResourceConstraint {
        owner: entity(4),
        resource: label("fixture-resource-1"),
        minimum: 0,
        maximum: 8,
        source: rule(),
    }]
}

pub(super) fn limits() -> CheckpointLimits {
    CheckpointLimits {
        maximum_records: 100,
        maximum_text_bytes: 256,
        maximum_total_text_bytes: 1024,
        maximum_retained_bytes: 1024 * 1024,
    }
}

pub(super) fn state() -> GameState {
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

pub(super) fn checkpoint(state: GameState) -> Result<Checkpoint, CheckpointError> {
    let rules = vec![rule()];
    let content_entries = vec![content()];
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

pub(super) fn fact(repeated_byte: u8, ordinal: u32) -> GameFact {
    GameFact {
        id: FactId::from_bytes(&[repeated_byte; 16]).unwrap(),
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

pub(super) fn pending() -> PendingResolution {
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

pub(super) fn continuity() -> ContinuityState {
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

pub(super) fn command_limits() -> CommandLimits {
    CommandLimits {
        maximum_records: 8,
        maximum_text_bytes: 32,
        maximum_retained_bytes: 8192,
    }
}

pub(super) fn game(command: GameCommand) -> GameInput {
    GameInput::Game(CommandInput {
        basis: basis(),
        observed_revision: basis().revision,
        operation: OperationId::from_bytes(&[20; 16]).unwrap(),
        member: member(3),
        command,
    })
}

pub(super) fn host(command: HostCommand) -> GameInput {
    GameInput::Host(HostInput {
        basis: basis(),
        operation: OperationId::from_bytes(&[20; 16]).unwrap(),
        host: member(3),
        command,
    })
}

pub(super) fn response(kind: u8) -> GameInput {
    let pending = pending();
    let resolution = pending.id;
    let window = pending.window.id;
    let offer = label("fixture-offer-1");
    let option = label("fixture-option-1");
    match kind {
        0 => game(GameCommand::SelectChoice {
            resolution,
            window,
            offer,
            option,
        }),
        1 => game(GameCommand::SelectReaction {
            resolution,
            window,
            offer,
            option,
        }),
        2 => game(GameCommand::SubmitRoll { resolution, window }),
        3 => host(HostCommand::ResolveRuling {
            resolution,
            window,
            offer,
            option,
        }),
        _ => unreachable!(),
    }
}

pub(super) fn waiting(kind: u8) -> Checkpoint {
    let mut supplied = state();
    if kind == 2 {
        supplied.mode = ExecutionMode::Live;
    }
    supplied.facts.push(fact(7, 0));
    let mut pending = pending();
    let offered = vec![OfferedResponse {
        participant: member(3),
        offer: label("fixture-offer-1"),
        options: vec![label("fixture-option-1")],
        source: rule(),
    }];
    pending.next = match kind {
        0 => PendingInput::Choice { remaining: offered },
        1 => PendingInput::Reaction { remaining: offered },
        2 => PendingInput::Roll {
            participant: member(3),
            sides: vec![20],
            source: rule(),
        },
        3 => PendingInput::Ruling {
            permitted: offered,
            source: rule(),
        },
        _ => unreachable!(),
    };
    supplied.pending.push(pending);
    checkpoint(supplied).unwrap()
}

use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use df_engine::command_entry::{CommandEntryContext, CommandEntryLimits};
use df_engine::pending_resumption::{
    ResumeError, ResumeFence, ResumeLimits, stage_pending_response,
};
use df_rules::preconditions::{
    CurrentRuleContext, PreconditionLimits, PreconditionedCommandHandler, RuleDependency,
    RulePreconditions,
};
use df_rules::{DispatchRegistry, HandlerRegistration, RulesCommandHandler, RulesCommandInput};
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FixtureRejection {
    Unsupported,
}

// Returns a caller-supplied immutable fixture checkpoint; it implements no continuation mechanic.
pub(super) struct SuppliedHandler {
    pub candidate: Checkpoint,
    pub pins: CheckpointPins,
    pub calls: Rc<Cell<usize>>,
    pub refuse: bool,
    pub observed_draw_pointer: Rc<Cell<*const ActualDraw>>,
}
impl RulesCommandHandler for SuppliedHandler {
    type Rejection = FixtureRejection;
    fn pins(&self) -> &CheckpointPins {
        &self.pins
    }
    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        _: &Checkpoint,
    ) -> Result<Checkpoint, FixtureRejection> {
        self.calls.set(self.calls.get() + 1);
        self.observed_draw_pointer
            .set(input.supplied_draws.as_ptr());
        if self.refuse {
            Err(FixtureRejection::Unsupported)
        } else {
            Ok(self.candidate.clone())
        }
    }
}

pub(super) fn supplied_candidate(current: &Checkpoint, input: RulesCommandInput<'_>) -> Checkpoint {
    let operation = match input.command {
        GameInput::Game(command) => command.operation,
        GameInput::Host(command) => command.operation,
        _ => panic!("fixture command only"),
    };
    let mut basis = current.basis();
    basis.revision = basis.revision.next_sequence().unwrap();
    let mut state = current.state().clone();
    // Test-only accounting of explicitly supplied outcomes, with no draw generation or mechanics.
    // Preserve the retained prefix and append exactly the external native fixture slice.
    let mut draw_facts = Vec::new();
    for draw in input.supplied_draws {
        state.draws.push(draw.clone());
        let id = FactId::from_bytes(&[40 + u8::try_from(draw.ordinal).unwrap(); 16]).unwrap();
        let ordinal = u32::try_from(
            state
                .facts
                .iter()
                .filter(|fact| fact.operation == operation)
                .count(),
        )
        .unwrap();
        state.facts.push(GameFact {
            id,
            revision: basis.revision,
            operation,
            ordinal,
            cause: None,
            audience: AudienceScope::Shared,
            value: FactValue::DrawAccepted {
                operation,
                ordinal: draw.ordinal,
            },
        });
        draw_facts.push(id);
        let pending = state
            .pending
            .iter_mut()
            .find(|pending| pending.id == draw.resolution)
            .unwrap();
        pending.draw_ordinals.push((operation, draw.ordinal));
    }
    let accepted_ordinals = state
        .draws
        .iter()
        .filter(|draw| draw.operation == operation)
        .map(|draw| draw.ordinal)
        .collect();
    for pending in &mut state.pending {
        pending.basis = basis;
    }
    state.decisions.push(AcceptedDecision {
        operation,
        revision: basis.revision,
        facts: draw_facts,
        draws: accepted_ordinals,
        effects: vec![],
        source_policy: label("fixture-supplied-suspension"),
        semantic_output: None,
    });
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis,
        current.pins().clone(),
        state,
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &resource_constraints(),
            assets: &[],
        },
        limits(),
    )
    .unwrap()
}

pub(super) fn fence() -> ResumeFence {
    ResumeFence {
        admitted_generation: 5,
        current_generation: 5,
        cancel_before_admission: false,
    }
}

pub(super) fn invoke(
    input: RulesCommandInput<'_>,
    current: &Checkpoint,
    prepared: &Checkpoint,
    handler: &SuppliedHandler,
    owner_fence: ResumeFence,
    maximum_bytes: usize,
) -> Result<Checkpoint, ResumeError<FixtureRejection>> {
    let rules = [rule()];
    let contents = [content()];
    let resources = resource_constraints();
    let dependencies = [
        RuleDependency::Resource {
            owner: entity(4),
            resource: label("fixture-resource-1"),
        },
        RuleDependency::PendingResolutions,
    ];
    let source = rule();
    let selector = label("fixture-continuation-position-1");
    let references = || ReferenceInventory {
        rules: &rules,
        content: &contents,
        resources: &resources,
        assets: &[],
    };
    let wrapper = PreconditionedCommandHandler::new(
        handler,
        &source,
        CurrentRuleContext {
            checkpoint: current,
            basis: current.basis(),
            pins: current.pins(),
            inventory: references(),
            command_limits: command_limits(),
        },
        RulePreconditions {
            prepared,
            sources: &rules,
            dependencies: &dependencies,
        },
        PreconditionLimits {
            maximum_dependencies: 8,
            maximum_comparisons: 1024 * 1024,
            maximum_checkpoint_bytes: maximum_bytes,
        },
    );
    let entries = [CatalogEntry::new(&source, b"synthetic-clause")];
    let catalog = CatalogSnapshot::from_published(
        &current.pins().rules.catalog,
        current.pins(),
        b"synthetic-publication",
        &entries,
        CatalogLimits {
            max_complete_bytes: 128,
            max_entries: 4,
            max_item_bytes: 64,
            max_total_item_bytes: 128,
        },
    )
    .unwrap();
    let registrations = [HandlerRegistration::new(&selector, &source, &wrapper)];
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 1).unwrap();
    stage_pending_response(
        input,
        current,
        CommandEntryContext {
            current_basis: current.basis(),
            admitted_pins: current.pins(),
            inventory: references(),
            limits: CommandEntryLimits {
                command: command_limits(),
                maximum_staged_bytes: maximum_bytes,
            },
        },
        owner_fence,
        ResumeLimits {
            maximum_checkpoint_bytes: maximum_bytes,
        },
        &registry,
        &selector,
    )
}

/// Explicit synthetic native outcome, supplied by each roll fixture before dispatch.
/// Neither production I03 nor this support's invoke/handler generates a draw on request.
pub(super) fn native_roll_draw(ordinal: u32) -> ActualDraw {
    ActualDraw {
        operation: OperationId::from_bytes(&[20; 16]).unwrap(),
        ordinal,
        resolution: ResolutionId::from_bytes(&[9; 16]).unwrap(),
        window: WindowId::from_bytes(&[10; 16]).unwrap(),
        sides: 20,
        value: 11,
        source: rule(),
    }
}
