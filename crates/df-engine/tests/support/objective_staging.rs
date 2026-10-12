use super::*;

pub fn content(entry: &str) -> ContentReference {
    ContentReference {
        entry: fixture::label(entry),
        ..fixture::content()
    }
}

pub fn operation(value: u8) -> OperationId {
    OperationId::from_bytes(&[value; 16]).unwrap()
}

pub fn encounter(value: u8) -> RecordId {
    RecordId::from_bytes(&[value; 16]).unwrap()
}

pub fn command_limits() -> CommandLimits {
    CommandLimits {
        maximum_records: 100,
        maximum_text_bytes: 256,
        maximum_retained_bytes: 1024 * 1024,
    }
}

pub struct Fixture {
    pub registration: ObjectiveRegistration,
    pub pins: CheckpointPins,
    pub rules: Vec<RuleReference>,
    pub content: Vec<ContentReference>,
    pub resources: Vec<ResourceConstraint>,
    pub expected: [ContentReference; 2],
    pub replacements: [ContentReference; 2],
    pub policy: ContentReference,
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

impl Fixture {
    pub fn new() -> Self {
        let expected = [
            content("authored-first-objective"),
            content("authored-second-objective"),
        ];
        let replacements = [
            content("authored-first-next"),
            content("authored-second-next"),
        ];
        let policy = content("authored-objective-policy");
        let registration = ObjectiveRegistration {
            action: content("apply-authored-objectives"),
            source: fixture::rule(),
            policy: fixture::label("compiled-objective-application"),
        };
        let mut content = vec![
            fixture::content(),
            registration.action.clone(),
            policy.clone(),
        ];
        content.extend(expected.iter().cloned());
        content.extend(replacements.iter().cloned());
        Self {
            registration,
            pins: fixture::pins(),
            rules: vec![fixture::rule()],
            content,
            resources: fixture::resource_constraints(),
            expected,
            replacements,
            policy,
        }
    }

    pub fn inventory(&self) -> ReferenceInventory<'_> {
        ReferenceInventory {
            rules: &self.rules,
            content: &self.content,
            resources: &self.resources,
            assets: &[],
        }
    }

    pub fn state(&self) -> GameState {
        let mut state = fixture::state();
        state.encounters = [12, 13]
            .into_iter()
            .map(|id| EncounterState {
                id: encounter(id),
                definition: fixture::content(),
                participants: vec![fixture::entity(4)],
                turn_order: vec![fixture::entity(4)],
                active_turn: Some(fixture::entity(4)),
                objectives: self.expected.to_vec(),
                combat_policy: fixture::content(),
                tactical: None,
            })
            .collect();
        state.facts = [30, 31]
            .into_iter()
            .enumerate()
            .map(|(ordinal, id)| GameFact {
                id: FactId::from_bytes(&[id; 16]).unwrap(),
                revision: fixture::basis().revision,
                operation: operation(30),
                ordinal: ordinal as u32,
                cause: None,
                audience: AudienceScope::Members(vec![fixture::member(3)]),
                value: FactValue::ContentEvent {
                    definition: fixture::content(),
                    subjects: vec![fixture::entity(4)],
                },
            })
            .collect();
        state.decisions.push(AcceptedDecision {
            operation: operation(30),
            revision: fixture::basis().revision,
            facts: state.facts.iter().map(|fact| fact.id).collect(),
            draws: vec![],
            effects: vec![],
            source_policy: fixture::label("accepted-objective-cause-policy"),
            semantic_output: None,
        });
        state.beliefs.push(AttributedClaim {
            id: encounter(20),
            holder: fixture::entity(4),
            subject: fixture::entity(4),
            claim: "private unrelated belief".to_owned(),
            evidence: vec![state.facts[0].id],
            audience: AudienceScope::Host,
            source: fixture::content(),
        });
        state
    }

    pub fn checkpoint(&self, basis: Basis, state: GameState) -> Checkpoint {
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

    pub fn current(&self) -> Checkpoint {
        self.checkpoint(fixture::basis(), self.state())
    }

    pub fn input(&self, basis: Basis) -> GameInput {
        GameInput::Game(CommandInput {
            basis,
            observed_revision: basis.revision,
            operation: operation(50),
            member: fixture::member(3),
            command: GameCommand::ProposeAction {
                actor: fixture::entity(4),
                action: self.registration.action.clone(),
                targets: vec![],
                choices: vec![],
            },
        })
    }

    pub fn transitions<'a>(
        &'a self,
        current: &'a Checkpoint,
    ) -> Vec<AuthoredObjectiveTransition<'a>> {
        self.expected
            .iter()
            .zip(&self.replacements)
            .zip(&current.state().facts)
            .enumerate()
            .map(|(objective_index, ((expected, replacement), cause))| {
                AuthoredObjectiveTransition {
                    encounter: encounter(12),
                    objective_index,
                    expected,
                    replacement,
                    policy: &self.policy,
                    source: &self.registration.source,
                    actor: fixture::entity(4),
                    cause,
                }
            })
            .collect()
    }

    pub fn handler<'a>(
        &'a self,
        owner: &'a SourceOwner,
        basis: Basis,
        transitions: Option<&'a [AuthoredObjectiveTransition<'a>]>,
    ) -> ObjectiveTransitionHandler<'a, SourceOwner> {
        ObjectiveTransitionHandler {
            owner,
            current_basis: basis,
            admitted_pins: &self.pins,
            inventory: self.inventory(),
            registration: &self.registration,
            transitions,
            limits: ObjectiveStagingLimits {
                maximum_checkpoint_bytes: 1024 * 1024,
                maximum_pass_bytes: 8 * 1024 * 1024,
                checkpoint: fixture::limits(),
                objectives: ObjectiveProposalLimits {
                    maximum_changes: 4,
                    maximum_records: 100,
                    maximum_comparisons: 10_000,
                    maximum_input_bytes: 1024 * 1024,
                    maximum_output_bytes: 1024 * 1024,
                },
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceRefusal {
    Withdrawn,
    Binding,
    ActorControl,
}

/// Finite deterministic authored fixture, not a substitute rulebook catalog or real grant.
pub struct SourceOwner {
    expected_registration: ObjectiveRegistration,
    pins: CheckpointPins,
    expected: [ContentReference; 2],
    replacements: [ContentReference; 2],
    policy: ContentReference,
    causes: Vec<GameFact>,
    decisions: Vec<AcceptedDecision>,
    pub refusal: Cell<Option<SourceRefusal>>,
    pub policy_refusal: Cell<Option<ObjectivePolicyError>>,
    pub refused_index: Cell<Option<usize>>,
    pub calls: Cell<usize>,
    pub applications: Cell<usize>,
}

impl SourceOwner {
    pub fn new(fixture: &Fixture) -> Self {
        Self {
            expected_registration: fixture.registration.clone(),
            pins: fixture.pins.clone(),
            expected: fixture.expected.clone(),
            replacements: fixture.replacements.clone(),
            policy: fixture.policy.clone(),
            causes: fixture.state().facts,
            decisions: fixture.state().decisions,
            refusal: Cell::new(None),
            policy_refusal: Cell::new(None),
            refused_index: Cell::new(None),
            calls: Cell::new(0),
            applications: Cell::new(0),
        }
    }
}

impl ObjectivePolicyOwner for SourceOwner {
    fn admit_transition(
        &self,
        current: &Checkpoint,
        transition: &AuthoredObjectiveTransition<'_>,
    ) -> Result<(), ObjectivePolicyError> {
        self.calls.set(self.calls.get() + 1);
        if let Some(refusal) = self.policy_refusal.get() {
            return Err(refusal);
        }
        if self.refused_index.get() == Some(transition.objective_index) {
            return Err(ObjectivePolicyError::RightsDenied);
        }
        let index = transition.objective_index;
        if current.pins() != &self.pins
            || current.basis() != fixture::basis()
            || transition.encounter != encounter(12)
            || self.expected.get(index) != Some(transition.expected)
            || self.replacements.get(index) != Some(transition.replacement)
            || transition.policy != &self.policy
            || transition.source != &self.expected_registration.source
            || self.causes.get(index) != Some(transition.cause)
            || current.state().decisions != self.decisions
        {
            return Err(ObjectivePolicyError::PolicyMismatch);
        }
        if transition.actor != fixture::entity(4) {
            return Err(ObjectivePolicyError::ActorDenied);
        }
        Ok(())
    }
}

impl ObjectiveSourceOwner for SourceOwner {
    type Refusal = SourceRefusal;

    fn admit_application(
        &self,
        current: &Checkpoint,
        registration: &ObjectiveRegistration,
        command: &CommandInput,
        _: &[AuthoredObjectiveTransition<'_>],
    ) -> Result<(), Self::Refusal> {
        self.applications.set(self.applications.get() + 1);
        if let Some(refusal) = self.refusal.get() {
            return Err(refusal);
        }
        if registration != &self.expected_registration || current.pins() != &self.pins {
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
        Ok(())
    }
}

pub type Rejection =
    CommandRejection<PreconditionedRejection<ObjectiveStagingError<SourceRefusal>>>;

pub fn registered_handler(
    fixture: &Fixture,
    prepared: &Checkpoint,
    current: &Checkpoint,
    input: &GameInput,
    handler: &ObjectiveTransitionHandler<'_, SourceOwner>,
) -> Result<Checkpoint, Rejection> {
    let dependencies = [
        RuleDependency::Entity(fixture::entity(4)),
        RuleDependency::Encounter(encounter(12)),
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
            maximum_dependencies: 4,
            maximum_comparisons: 10_000_000,
            maximum_checkpoint_bytes: 1024 * 1024,
        },
    );
    let selector = fixture::label("compiled-objective-application");
    let entries = [CatalogEntry::new(
        &fixture.registration.source,
        b"exact synthetic authored objective source",
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
            supplied_draws: &[],
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

pub fn registered(
    fixture: &Fixture,
    source: &SourceOwner,
    current: &Checkpoint,
    input: &GameInput,
) -> Result<Checkpoint, Rejection> {
    let changes = fixture.transitions(current);
    let handler = fixture.handler(source, current.basis(), Some(&changes));
    registered_handler(fixture, current, current, input, &handler)
}

pub fn assert_preserved(fixture: &Fixture, original: &Checkpoint, next: &Checkpoint) {
    let mut expected = original.state().clone();
    expected.encounters[0].objectives = fixture.replacements.to_vec();
    expected.decisions.push(AcceptedDecision {
        operation: operation(50),
        revision: original.basis().revision.next_sequence().unwrap(),
        facts: vec![],
        draws: vec![],
        effects: vec![],
        source_policy: fixture.registration.policy.clone(),
        semantic_output: None,
    });
    assert_eq!(next.state(), &expected);
    let mut basis = original.basis();
    basis.revision = basis.revision.next_sequence().unwrap();
    assert_eq!(next.basis(), basis);
    assert_eq!(next.pins(), original.pins());
    assert_eq!(original.state().facts, next.state().facts);
    assert_eq!(original.state().decisions[0], next.state().decisions[0]);
}
