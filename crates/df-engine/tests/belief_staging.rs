#[allow(dead_code)] // Shared fixture helpers are intentionally broader than this test.
#[path = "support/fixture_model.rs"]
mod fixture_model;

use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use df_engine::belief_staging::{
    AttributedBeliefUpdate, BeliefRegistration, BeliefSourceOwner, BeliefStagingError,
    BeliefStagingHandler, BeliefStagingLimits,
};
use df_engine::command_entry::{
    CommandEntryContext, CommandEntryLimits, CommandRejection, decide_registered_command,
};
use df_knowledge::beliefs::{BeliefBasis, BeliefValidationError};
use df_model::checkpoint::*;
use df_model::commands::CommandLimits;
use df_rules::{
    DispatchRegistry, HandlerRegistration, InvocationError, RulesCommandHandler, RulesCommandInput,
};
use df_types::OperationId;
use fixture_model as f;

type FixtureRegistration = BeliefRegistration;

fn content(entry: &str) -> ContentReference {
    ContentReference {
        entry: f::label(entry),
        ..f::content()
    }
}

fn fact(value: u8) -> FactId {
    FactId::from_bytes(&[value; 16]).unwrap()
}

fn record(value: u8) -> RecordId {
    RecordId::from_bytes(&[value; 16]).unwrap()
}

fn operation(value: u8) -> OperationId {
    OperationId::from_bytes(&[value; 16]).unwrap()
}

struct Fixture {
    registration: FixtureRegistration,
    pins: CheckpointPins,
    rules: Vec<RuleReference>,
    content: Vec<ContentReference>,
    resources: Vec<ResourceConstraint>,
}

impl Fixture {
    fn new() -> Self {
        let action = content("apply-native-belief");
        Self {
            registration: BeliefRegistration {
                action: action.clone(),
                source: f::rule(),
                claim_source: content("authored-belief-source"),
                decision_policy: f::label("compiled-belief-application"),
            },
            pins: f::pins(),
            rules: vec![f::rule()],
            content: vec![f::content(), action],
            resources: f::resource_constraints(),
        }
    }

    fn inventory(&self) -> ReferenceInventory<'_> {
        ReferenceInventory {
            rules: &self.rules,
            content: &self.content,
            resources: &self.resources,
            assets: &[],
        }
    }

    fn state(&self) -> GameState {
        let mut state = f::state();
        let mut other = state.entities[0].clone();
        other.id = f::entity(6);
        state.entities.push(other);
        state.members.push(MembershipLink {
            member: f::member(5),
            character: Some(f::entity(6)),
        });
        state.facts = [
            AudienceScope::Shared,
            AudienceScope::Members(vec![f::member(3)]),
            AudienceScope::Host,
            AudienceScope::Members(vec![f::member(5)]),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, audience)| GameFact {
            id: fact(30 + index as u8),
            revision: f::basis().revision,
            operation: operation(30 + index as u8),
            ordinal: 0,
            cause: None,
            audience,
            value: FactValue::ContentEvent {
                definition: f::content(),
                subjects: vec![f::entity(4)],
            },
        })
        .collect();
        state.decisions = state
            .facts
            .iter()
            .map(|fact| AcceptedDecision {
                operation: fact.operation,
                revision: fact.revision,
                facts: vec![fact.id],
                draws: vec![],
                effects: vec![],
                source_policy: f::label("accepted-world-source"),
                semantic_output: None,
            })
            .collect();
        state
    }

    fn checkpoint(&self, basis: Basis, state: GameState) -> Checkpoint {
        Checkpoint::new(
            CHECKPOINT_SCHEMA,
            basis,
            self.pins.clone(),
            state,
            self.inventory(),
            f::limits(),
        )
        .unwrap()
    }

    fn input(&self, basis: Basis) -> GameInput {
        GameInput::Game(CommandInput {
            basis,
            observed_revision: basis.revision,
            operation: operation(50),
            member: f::member(3),
            command: GameCommand::ProposeAction {
                actor: f::entity(4),
                action: self.registration.action.clone(),
                targets: vec![],
                choices: vec![],
            },
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Refusal {
    Withdrawn,
    Binding,
}

struct Owner {
    basis: Basis,
    registration: BeliefRegistration,
    permitted: AttributedBeliefUpdate,
    refusal: Option<Refusal>,
    attribution_allowed: bool,
}

impl BeliefBasis<AttributedBeliefUpdate> for Owner {
    fn current_basis(&self) -> &Basis {
        &self.basis
    }

    fn permits_subject(&self, subject: &EntityId) -> bool {
        *subject == self.permitted.claim.subject
    }

    fn permits_attribution(&self, actor: &EntityId, subject: &EntityId) -> bool {
        self.attribution_allowed
            && *actor == self.permitted.claim.holder
            && *subject == self.permitted.claim.subject
    }

    fn permits_evidence(&self, update: &AttributedBeliefUpdate) -> bool {
        update.claim.evidence == self.permitted.claim.evidence
    }
}

impl BeliefSourceOwner for Owner {
    type Refusal = Refusal;

    fn admit(
        &self,
        current: &Checkpoint,
        registration: &BeliefRegistration,
        command: &CommandInput,
        _update: &AttributedBeliefUpdate,
    ) -> Result<(), Self::Refusal> {
        if let Some(refusal) = self.refusal {
            return Err(refusal);
        }
        if current.basis() != self.basis
            || registration != &self.registration
            || command.member != f::member(3)
        {
            return Err(Refusal::Binding);
        }
        Ok(())
    }
}

struct Setup {
    fixture: Fixture,
    registration: BeliefRegistration,
    update: AttributedBeliefUpdate,
    current: Checkpoint,
}

impl Setup {
    fn new() -> Self {
        let mut fixture = Fixture::new();
        let claim_source = content("native-belief-source");
        fixture.content.push(claim_source.clone());
        let registration = BeliefRegistration {
            action: fixture.registration.action.clone(),
            source: fixture.registration.source.clone(),
            claim_source: claim_source.clone(),
            decision_policy: f::label("compiled-belief-application"),
        };
        let basis = f::basis();
        let update = AttributedBeliefUpdate {
            basis,
            claim: AttributedClaim {
                id: record(80),
                holder: f::entity(4),
                subject: f::entity(6),
                claim: "The sealed gate is open".to_owned(),
                evidence: vec![fact(30)],
                audience: AudienceScope::Members(vec![f::member(5)]),
                source: claim_source,
            },
        };
        let mut state = fixture.state();
        let sealed = content("sealed-gate-closed");
        fixture.content.push(sealed.clone());
        state.facts[0].value = FactValue::ContentEvent {
            definition: sealed,
            subjects: vec![f::entity(6)],
        };
        let current = fixture.checkpoint(f::basis(), state);
        Self {
            fixture,
            registration,
            update,
            current,
        }
    }

    fn owner(&self) -> Owner {
        Owner {
            basis: self.current.basis(),
            registration: self.registration.clone(),
            permitted: self.update.clone(),
            refusal: None,
            attribution_allowed: true,
        }
    }

    fn handler<'a>(
        &'a self,
        owner: &'a Owner,
        update: Option<&'a AttributedBeliefUpdate>,
    ) -> BeliefStagingHandler<'a, Owner> {
        BeliefStagingHandler {
            owner,
            current_basis: self.current.basis(),
            admitted_pins: &self.fixture.pins,
            inventory: self.fixture.inventory(),
            registration: &self.registration,
            update,
            limits: BeliefStagingLimits {
                maximum_checkpoint_bytes: 1024 * 1024,
                maximum_pass_bytes: 4 * 1024 * 1024,
                maximum_inventory_records: 32,
                checkpoint: f::limits(),
            },
        }
    }

    fn registered(
        &self,
        owner: &Owner,
        update: Option<&AttributedBeliefUpdate>,
        current: &Checkpoint,
        input: &GameInput,
    ) -> Result<Checkpoint, CommandRejection<BeliefStagingError<Refusal>>> {
        let handler = self.handler(owner, update);
        let selector = f::label("compiled-belief-handler");
        let entries = [CatalogEntry::new(
            &self.registration.source,
            b"synthetic belief source",
        )];
        let catalog = CatalogSnapshot::from_published(
            &self.fixture.pins.rules.catalog,
            &self.fixture.pins,
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
            &self.registration.source,
            &handler,
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
                inventory: self.fixture.inventory(),
                limits: CommandEntryLimits {
                    command: CommandLimits {
                        maximum_records: 100,
                        maximum_text_bytes: 256,
                        maximum_retained_bytes: 1024 * 1024,
                    },
                    maximum_staged_bytes: 1024 * 1024,
                },
            },
            &registry,
            &selector,
            &self.registration.source,
        )
    }

    fn input(&self) -> GameInput {
        self.fixture.input(self.current.basis())
    }
}

#[test]
fn registered_false_claim_is_attributed_private_and_never_rewrites_canonical_facts() {
    let setup = Setup::new();
    let owner = setup.owner();
    let original_facts = setup.current.state().facts.clone();
    let staged = setup
        .registered(&owner, Some(&setup.update), &setup.current, &setup.input())
        .unwrap();

    assert_eq!(staged.state().facts, original_facts);
    assert_eq!(setup.current.state().facts, original_facts);
    assert_eq!(
        setup.current.state().facts[0].value,
        FactValue::ContentEvent {
            definition: content("sealed-gate-closed"),
            subjects: vec![f::entity(6)],
        }
    );
    assert_eq!(setup.update.claim.claim, "The sealed gate is open");
    assert_eq!(
        staged.state().beliefs.as_slice(),
        std::slice::from_ref(&setup.update.claim)
    );
    assert_eq!(
        staged.state().beliefs[0].audience,
        AudienceScope::Members(vec![f::member(5)])
    );
    let accepted = staged.state().decisions.last().unwrap();
    assert!(accepted.facts.is_empty());
    assert!(accepted.draws.is_empty() && accepted.effects.is_empty());
    assert_eq!(accepted.operation, operation(50));
}

#[test]
fn stale_basis_unknown_subject_invalid_attribution_and_undisclosed_evidence_refuse() {
    let setup = Setup::new();
    let owner = setup.owner();
    let mut stale = setup.update.clone();
    stale.basis.revision = stale.basis.revision.next_sequence().unwrap();
    let handler = setup.handler(&owner, Some(&stale));
    assert_eq!(
        handler.stage(
            df_engine::command_entry::RulesCommandInput {
                command: &setup.input(),
                supplied_draws: &[],
            },
            &setup.current,
        ),
        Err(BeliefStagingError::Belief(
            BeliefValidationError::StaleBasis
        ))
    );

    let mut stale_owner = setup.owner();
    stale_owner.basis.revision = stale_owner.basis.revision.next_sequence().unwrap();
    stale_owner.permitted.basis = stale_owner.basis;
    let stale_pair_handler = setup.handler(&stale_owner, Some(&stale_owner.permitted));
    assert_eq!(
        stale_pair_handler.stage(
            df_engine::command_entry::RulesCommandInput {
                command: &setup.input(),
                supplied_draws: &[],
            },
            &setup.current,
        ),
        Err(BeliefStagingError::Belief(
            BeliefValidationError::StaleBasis
        ))
    );

    let mut stale_command = setup.input();
    let GameInput::Game(ref mut command) = stale_command else {
        unreachable!()
    };
    command.observed_revision = command.observed_revision.next_sequence().unwrap();
    let handler = setup.handler(&owner, Some(&setup.update));
    assert_eq!(
        handler.stage(
            df_engine::command_entry::RulesCommandInput {
                command: &stale_command,
                supplied_draws: &[],
            },
            &setup.current,
        ),
        Err(BeliefStagingError::StaleCommand)
    );

    let mut unknown = setup.update.clone();
    unknown.claim.subject = f::entity(99);
    assert_eq!(
        setup.registered(&owner, Some(&unknown), &setup.current, &setup.input()),
        Err(CommandRejection::Invocation(InvocationError::Handler(
            BeliefStagingError::Belief(BeliefValidationError::UnknownSubject)
        )))
    );

    let mut attribution_owner = setup.owner();
    attribution_owner.attribution_allowed = false;
    assert_eq!(
        setup.registered(
            &attribution_owner,
            Some(&setup.update),
            &setup.current,
            &setup.input()
        ),
        Err(CommandRejection::Invocation(InvocationError::Handler(
            BeliefStagingError::Belief(BeliefValidationError::InvalidAttribution)
        )))
    );

    let mut unsupported_evidence = setup.update.clone();
    unsupported_evidence.claim.evidence = vec![fact(99)];
    assert_eq!(
        setup.registered(
            &owner,
            Some(&unsupported_evidence),
            &setup.current,
            &setup.input()
        ),
        Err(CommandRejection::Invocation(InvocationError::Handler(
            BeliefStagingError::Belief(BeliefValidationError::UndisclosedEvidence)
        )))
    );

    let mut current_but_unpermitted_evidence = setup.update.clone();
    current_but_unpermitted_evidence.claim.evidence = vec![fact(31)];
    assert_eq!(
        setup.registered(
            &owner,
            Some(&current_but_unpermitted_evidence),
            &setup.current,
            &setup.input()
        ),
        Err(CommandRejection::Invocation(InvocationError::Handler(
            BeliefStagingError::Belief(BeliefValidationError::UndisclosedEvidence)
        )))
    );
    assert!(setup.current.state().beliefs.is_empty());
}

#[test]
fn withdrawal_duplicate_operation_duplicate_claim_and_missing_producer_refuse() {
    let setup = Setup::new();
    let owner = setup.owner();
    let mut withdrawn = setup.owner();
    withdrawn.refusal = Some(Refusal::Withdrawn);
    assert_eq!(
        setup.registered(
            &withdrawn,
            Some(&setup.update),
            &setup.current,
            &setup.input()
        ),
        Err(CommandRejection::Invocation(InvocationError::Handler(
            BeliefStagingError::Source(Refusal::Withdrawn)
        )))
    );

    let mut duplicate_op = setup.input();
    let GameInput::Game(ref mut command) = duplicate_op else {
        unreachable!()
    };
    command.operation = setup.current.state().decisions[0].operation;
    assert_eq!(
        setup.registered(&owner, Some(&setup.update), &setup.current, &duplicate_op),
        Err(CommandRejection::Invocation(
            InvocationError::AlreadyAccepted
        ))
    );

    let mut with_claim = setup.current.state().clone();
    with_claim.beliefs.push(setup.update.claim.clone());
    let duplicate_claim = setup.fixture.checkpoint(setup.current.basis(), with_claim);
    assert_eq!(
        setup.registered(
            &owner,
            Some(&setup.update),
            &duplicate_claim,
            &setup.input()
        ),
        Err(CommandRejection::Invocation(InvocationError::Handler(
            BeliefStagingError::DuplicateClaim
        )))
    );
    assert_eq!(
        setup.registered(&owner, None, &setup.current, &setup.input()),
        Err(CommandRejection::Invocation(InvocationError::Handler(
            BeliefStagingError::MissingProducer
        )))
    );
}

#[test]
fn capacity_and_source_registration_are_bounded_before_commit() {
    let setup = Setup::new();
    let owner = setup.owner();
    let mut registration = setup.registration.clone();
    registration.claim_source = content("unregistered-belief-source");
    let mut handler = setup.handler(&owner, Some(&setup.update));
    handler.registration = &registration;
    assert_eq!(
        handler.stage(
            df_engine::command_entry::RulesCommandInput {
                command: &setup.input(),
                supplied_draws: &[],
            },
            &setup.current,
        ),
        Err(BeliefStagingError::InvalidRegistration)
    );

    handler.limits.maximum_inventory_records = 0;
    assert_eq!(
        handler.stage(
            df_engine::command_entry::RulesCommandInput {
                command: &setup.input(),
                supplied_draws: &[],
            },
            &setup.current,
        ),
        Err(BeliefStagingError::Capacity)
    );

    let mut capacity_handler = setup.handler(&owner, Some(&setup.update));
    capacity_handler.limits.checkpoint.maximum_text_bytes = 4;
    assert_eq!(
        capacity_handler.stage(
            df_engine::command_entry::RulesCommandInput {
                command: &setup.input(),
                supplied_draws: &[],
            },
            &setup.current,
        ),
        Err(BeliefStagingError::Capacity)
    );

    let mut oversized_audience = setup.update.clone();
    oversized_audience.claim.audience = AudienceScope::Members(vec![f::member(5); 8]);
    let mut audience_handler = setup.handler(&owner, Some(&oversized_audience));
    audience_handler.limits.checkpoint.maximum_records = 4;
    assert_eq!(
        audience_handler.stage(
            df_engine::command_entry::RulesCommandInput {
                command: &setup.input(),
                supplied_draws: &[],
            },
            &setup.current,
        ),
        Err(BeliefStagingError::Capacity)
    );

    capacity_handler.limits.checkpoint.maximum_text_bytes = 256;
    capacity_handler.limits.checkpoint.maximum_records = 0;
    assert_eq!(
        capacity_handler.stage(
            df_engine::command_entry::RulesCommandInput {
                command: &setup.input(),
                supplied_draws: &[],
            },
            &setup.current,
        ),
        Err(BeliefStagingError::Capacity)
    );

    capacity_handler.limits.checkpoint.maximum_records = 100;
    capacity_handler.limits.maximum_checkpoint_bytes = 1;
    capacity_handler.limits.checkpoint.maximum_retained_bytes = 1;
    assert_eq!(
        capacity_handler.stage(
            df_engine::command_entry::RulesCommandInput {
                command: &setup.input(),
                supplied_draws: &[],
            },
            &setup.current,
        ),
        Err(BeliefStagingError::Capacity)
    );
    assert!(setup.current.state().beliefs.is_empty());
}

#[test]
fn actual_registered_belief_staging_preserves_source_bound_journal_and_commit_distinction() {
    use df_engine::belief_staging::select_journal;
    use df_knowledge::perception::{ObserverScope, PerceptionError};
    let mut setup = Setup::new();
    let mut supplied = setup.current.state().clone();
    let mut retained_claim = setup.update.claim.clone();
    retained_claim.id = record(81);
    supplied.beliefs.push(retained_claim.clone());
    supplied.continuity.journal.push(JournalEntry {
        id: record(82),
        source_facts: vec![fact(30)],
        attributed_claims: vec![retained_claim.id],
        audience: AudienceScope::Shared,
        text: "An attributed account of the sealed gate".to_owned(),
    });
    setup.current = setup.fixture.checkpoint(setup.current.basis(), supplied);
    let original = setup.current.clone();
    let owner = setup.owner();
    let staged = setup
        .registered(&owner, Some(&setup.update), &setup.current, &setup.input())
        .unwrap();
    assert_eq!(
        staged.basis().revision,
        original.basis().revision.next_sequence().unwrap()
    );
    assert_eq!(staged.state().facts, original.state().facts);
    assert_eq!(staged.state().resources, original.state().resources);
    assert_eq!(staged.state().draws, original.state().draws);
    assert_eq!(staged.state().intents, original.state().intents);
    assert_eq!(
        staged.state().continuity.journal,
        original.state().continuity.journal
    );
    assert_eq!(
        staged.state().beliefs,
        vec![retained_claim.clone(), setup.update.claim.clone()]
    );
    assert_eq!(
        select_journal(
            &staged,
            original.basis(),
            &setup.fixture.pins,
            ObserverScope::Member(f::member(5)),
            1000,
            1000,
            16
        )
        .unwrap_err(),
        PerceptionError::Checkpoint(CheckpointError::StaleBasis)
    );
    let selected = select_journal(
        &staged,
        staged.basis(),
        &setup.fixture.pins,
        ObserverScope::Member(f::member(5)),
        1000,
        1000,
        16,
    )
    .unwrap();
    assert_eq!(selected.len(), 1);
    assert!(std::ptr::eq(
        selected[0],
        &staged.state().continuity.journal[0]
    ));
    assert_eq!(selected[0].source_facts, vec![fact(30)]);
    assert_eq!(selected[0].attributed_claims, vec![retained_claim.id]);
    assert_eq!(retained_claim.claim, "The sealed gate is open");
    assert_eq!(
        staged.state().facts[0].value,
        FactValue::ContentEvent {
            definition: content("sealed-gate-closed"),
            subjects: vec![f::entity(6)],
        }
    );
    for observer in [ObserverScope::Shared, ObserverScope::Member(f::member(3))] {
        assert!(
            select_journal(
                &staged,
                staged.basis(),
                &setup.fixture.pins,
                observer,
                1000,
                1000,
                16
            )
            .unwrap()
            .is_empty()
        );
    }
    assert_eq!(
        select_journal(
            &staged,
            staged.basis(),
            &setup.fixture.pins,
            ObserverScope::Member(f::member(99)),
            1000,
            1000,
            16
        )
        .unwrap_err(),
        PerceptionError::ObserverUnavailable
    );
    // The original caller-owned checkpoint remains unchanged. The handler returns
    // a staged candidate; this pure read supplies no durable Session receipt.
    assert_eq!(setup.current, original);
    assert_eq!(
        select_journal(
            &original,
            original.basis(),
            &setup.fixture.pins,
            ObserverScope::Member(f::member(5)),
            1000,
            1000,
            16
        )
        .unwrap(),
        vec![&original.state().continuity.journal[0]]
    );
}

#[test]
fn actual_belief_consumer_refuses_unbound_journal_source_without_changing_stage() {
    use df_engine::belief_staging::select_journal;
    use df_knowledge::perception::{ObserverScope, PerceptionError};
    let mut setup = Setup::new();
    let mut supplied = setup.current.state().clone();
    supplied.decisions.remove(0);
    supplied.continuity.journal.push(JournalEntry {
        id: record(82),
        source_facts: vec![fact(30)],
        attributed_claims: vec![],
        audience: AudienceScope::Shared,
        text: "Seeded account without decision".to_owned(),
    });
    setup.current = setup.fixture.checkpoint(setup.current.basis(), supplied);
    let owner = setup.owner();
    let staged = setup
        .registered(&owner, Some(&setup.update), &setup.current, &setup.input())
        .unwrap();
    let original = staged.clone();
    assert!(
        select_journal(
            &staged,
            staged.basis(),
            &setup.fixture.pins,
            ObserverScope::Shared,
            1000,
            1000,
            16
        )
        .unwrap()
        .is_empty()
    );
    assert_eq!(
        select_journal(
            &staged,
            staged.basis(),
            &setup.fixture.pins,
            ObserverScope::Shared,
            0,
            1000,
            16
        )
        .unwrap_err(),
        PerceptionError::ScanCapacity
    );
    assert_eq!(staged, original);
    assert_eq!(staged.state().facts, setup.current.state().facts);
    assert_eq!(staged.state().beliefs, vec![setup.update.claim.clone()]);
}
