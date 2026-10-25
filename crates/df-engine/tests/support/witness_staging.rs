use crate::fixture_model as fixture;
use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use df_engine::command_entry::{
    CommandEntryContext, CommandEntryLimits, CommandRejection, decide_registered_command,
};
use df_engine::witness_staging::*;
use df_knowledge::witness::{WitnessEligibility, WitnessLimits, WitnessRoute};
use df_model::checkpoint::*;
use df_model::commands::CommandLimits;
use df_rules::{DispatchRegistry, HandlerRegistration, RulesCommandInput};
use df_types::OperationId;
use std::cell::Cell;

pub fn content(entry: &str) -> ContentReference {
    ContentReference {
        entry: fixture::label(entry),
        ..fixture::content()
    }
}
pub fn fact(value: u8) -> FactId {
    FactId::from_bytes(&[value; 16]).unwrap()
}
pub fn record(value: u8) -> RecordId {
    RecordId::from_bytes(&[value; 16]).unwrap()
}
pub fn operation(value: u8) -> OperationId {
    OperationId::from_bytes(&[value; 16]).unwrap()
}

pub struct Fixture {
    pub registration: WitnessRegistration,
    pub pins: CheckpointPins,
    pub rules: Vec<RuleReference>,
    pub content: Vec<ContentReference>,
    pub resources: Vec<ResourceConstraint>,
    pub relationship_state: df_types::RevisionLabel,
}
impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}
impl Fixture {
    pub fn new() -> Self {
        Self {
            registration: WitnessRegistration {
                action: content("apply-native-witnesses"),
                source: fixture::rule(),
                witness_policy: content("authored-witness-policy"),
                decision_policy: fixture::label("compiled-witness-application"),
            },
            pins: fixture::pins(),
            rules: vec![fixture::rule()],
            content: vec![
                fixture::content(),
                content("apply-native-witnesses"),
                content("authored-witness-policy"),
                content("other-witness-policy"),
            ],
            resources: fixture::resource_constraints(),
            relationship_state: fixture::label("actual-contact-permitted"),
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
        let mut other = state.entities[0].clone();
        other.id = fixture::entity(6);
        state.entities.push(other);
        state.members.push(MembershipLink {
            member: fixture::member(5),
            character: Some(fixture::entity(6)),
        });
        state.facts = [
            AudienceScope::Shared,
            AudienceScope::Members(vec![fixture::member(3)]),
            AudienceScope::Host,
            AudienceScope::Members(vec![fixture::member(5)]),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, audience)| GameFact {
            id: fact(30 + index as u8),
            revision: fixture::basis().revision,
            operation: operation(30 + index as u8),
            ordinal: 0,
            cause: None,
            audience,
            value: FactValue::ContentEvent {
                definition: fixture::content(),
                subjects: vec![fixture::entity(4)],
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
                source_policy: fixture::label("accepted-world-source"),
                semantic_output: None,
            })
            .collect();
        state.continuity.witnesses = state
            .facts
            .iter()
            .enumerate()
            .map(|(index, fact)| WitnessRecord {
                id: record(40 + index as u8),
                observer: fixture::entity(4),
                fact: fact.id,
                perceived_at: state.logical_time,
                source: self.registration.witness_policy.clone(),
            })
            .collect();
        state.relationships.push(Relationship {
            subject: fixture::entity(4),
            object: fixture::entity(6),
            policy: self.registration.witness_policy.clone(),
            state: self.relationship_state.clone(),
            trust: RelationshipAxisState {
                value: self.relationship_state.clone(),
                provenance: RelationshipAxisProvenance::AuthoredBaseline {
                    source: self.registration.witness_policy.clone(),
                },
            },
            affection: RelationshipAxisState {
                value: self.relationship_state.clone(),
                provenance: RelationshipAxisProvenance::AuthoredBaseline {
                    source: self.registration.witness_policy.clone(),
                },
            },
            respect: RelationshipAxisState {
                value: self.relationship_state.clone(),
                provenance: RelationshipAxisProvenance::AuthoredBaseline {
                    source: self.registration.witness_policy.clone(),
                },
            },
            fear: RelationshipAxisState {
                value: self.relationship_state.clone(),
                provenance: RelationshipAxisProvenance::AuthoredBaseline {
                    source: self.registration.witness_policy.clone(),
                },
            },
            suspicion: RelationshipAxisState {
                value: self.relationship_state.clone(),
                provenance: RelationshipAxisProvenance::AuthoredBaseline {
                    source: self.registration.witness_policy.clone(),
                },
            },
            debt: RelationshipAxisState {
                value: self.relationship_state.clone(),
                provenance: RelationshipAxisProvenance::AuthoredBaseline {
                    source: self.registration.witness_policy.clone(),
                },
            },
            familiarity: RelationshipAxisState {
                value: self.relationship_state.clone(),
                provenance: RelationshipAxisProvenance::AuthoredBaseline {
                    source: self.registration.witness_policy.clone(),
                },
            },
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
    pub fn eligible(&self) -> [WitnessEligibility<'_>; 4] {
        [
            WitnessEligibility {
                witness: record(40),
                recipient: fixture::member(3),
                route: WitnessRoute::Sight,
            },
            WitnessEligibility {
                witness: record(41),
                recipient: fixture::member(3),
                route: WitnessRoute::Hearing,
            },
            WitnessEligibility {
                witness: record(40),
                recipient: fixture::member(5),
                route: WitnessRoute::Relationship {
                    permitted_state: &self.relationship_state,
                },
            },
            WitnessEligibility {
                witness: record(40),
                recipient: fixture::member(3),
                route: WitnessRoute::Sight,
            },
        ]
    }
    pub fn handler<'a>(
        &'a self,
        owner: &'a SourceOwner,
        basis: Basis,
        eligible: Option<&'a [WitnessEligibility<'a>]>,
    ) -> WitnessGrantHandler<'a, SourceOwner> {
        WitnessGrantHandler {
            owner,
            current_basis: basis,
            admitted_pins: &self.pins,
            inventory: self.inventory(),
            registration: &self.registration,
            eligible,
            limits: WitnessStagingLimits {
                maximum_checkpoint_bytes: 1024 * 1024,
                maximum_pass_bytes: 4 * 1024 * 1024,
                maximum_inventory_records: 32,
                checkpoint: fixture::limits(),
                witness: WitnessLimits {
                    maximum_candidates: 16,
                    maximum_scan_records: 1024,
                    maximum_record_comparisons: 1024,
                    maximum_grants: 16,
                    maximum_grant_bytes: 4096,
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
    Eligibility,
}

pub struct SourceOwner {
    expected: WitnessRegistration,
    pins: CheckpointPins,
    basis: Basis,
    witnesses: Vec<WitnessRecord>,
    members: Vec<MembershipLink>,
    pub refusal: Cell<Option<SourceRefusal>>,
    pub calls: Cell<usize>,
}
impl SourceOwner {
    pub fn new(fixture: &Fixture, current: &Checkpoint) -> Self {
        Self {
            expected: fixture.registration.clone(),
            pins: fixture.pins.clone(),
            basis: current.basis(),
            witnesses: current.state().continuity.witnesses.clone(),
            members: current.state().members.clone(),
            refusal: Cell::new(None),
            calls: Cell::new(0),
        }
    }
}
impl WitnessSourceOwner for SourceOwner {
    type Refusal = SourceRefusal;
    fn admit(
        &self,
        current: &Checkpoint,
        registration: &WitnessRegistration,
        command: &CommandInput,
        eligible: &[WitnessEligibility<'_>],
    ) -> Result<(), Self::Refusal> {
        self.calls.set(self.calls.get() + 1);
        if let Some(refusal) = self.refusal.get() {
            return Err(refusal);
        }
        if registration != &self.expected
            || current.pins() != &self.pins
            || current.basis() != self.basis
        {
            return Err(SourceRefusal::Binding);
        }
        let GameCommand::ProposeAction {
            actor,
            targets,
            choices,
            ..
        } = &command.command
        else {
            return Err(SourceRefusal::ActorControl);
        };
        if command.member != fixture::member(3)
            || *actor != fixture::entity(4)
            || !targets.is_empty()
            || !choices.is_empty()
            || !current
                .state()
                .characters
                .iter()
                .any(|character| character.entity == *actor && character.owner == command.member)
        {
            return Err(SourceRefusal::ActorControl);
        }
        // This finite synthetic native source admits these exact observations/contact only.
        // It does not claim to produce visibility or qualify a production authored policy.
        for candidate in eligible {
            let witness = current
                .state()
                .continuity
                .witnesses
                .iter()
                .find(|witness| witness.id == candidate.witness)
                .ok_or(SourceRefusal::Eligibility)?;
            if !self.witnesses.contains(witness)
                || witness.source != registration.witness_policy
                || !current.state().members.iter().any(|member| {
                    member.member == candidate.recipient
                        && self.members.contains(member)
                        && member.character
                            == Some(if candidate.recipient == fixture::member(3) {
                                fixture::entity(4)
                            } else {
                                fixture::entity(6)
                            })
                })
            {
                return Err(SourceRefusal::Eligibility);
            }
            let admitted = match candidate.route {
                WitnessRoute::Sight => {
                    candidate.recipient == fixture::member(3)
                        && candidate.witness == record(40)
                        && witness.observer == fixture::entity(4)
                }
                WitnessRoute::Hearing => {
                    candidate.recipient == fixture::member(3)
                        && [record(41), record(42), record(43)].contains(&candidate.witness)
                        && witness.observer == fixture::entity(4)
                }
                WitnessRoute::Relationship { permitted_state } => {
                    candidate.recipient == fixture::member(5)
                        && candidate.witness == record(40)
                        && *permitted_state == fixture::label("actual-contact-permitted")
                }
            };
            if !admitted {
                return Err(SourceRefusal::Eligibility);
            }
        }
        Ok(())
    }
}

pub type Rejection = CommandRejection<WitnessStagingError<SourceRefusal>>;
pub fn registered_handler(
    fixture: &Fixture,
    current: &Checkpoint,
    input: &GameInput,
    handler: &WitnessGrantHandler<'_, SourceOwner>,
) -> Result<Checkpoint, Rejection> {
    let selector = fixture::label("compiled-witness-handler");
    let entries = [CatalogEntry::new(
        &fixture.registration.source,
        b"synthetic witness source",
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
        handler,
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
        &fixture.registration.source,
    )
}
pub fn registered(
    fixture: &Fixture,
    source: &SourceOwner,
    current: &Checkpoint,
    input: &GameInput,
) -> Result<Checkpoint, Rejection> {
    let eligible = fixture.eligible();
    registered_handler(
        fixture,
        current,
        input,
        &fixture.handler(source, current.basis(), Some(&eligible)),
    )
}

pub fn expected_grants() -> Vec<KnowledgeGrant> {
    [(3, 30), (3, 31), (5, 30)]
        .into_iter()
        .map(|(member, id)| KnowledgeGrant {
            observer: fixture::member(member),
            fact: fact(id),
            source: fact(id),
        })
        .collect()
}
pub fn assert_preserved(original: &Checkpoint, next: &Checkpoint) {
    let mut expected = original.state().clone();
    expected.knowledge = expected_grants();
    expected
        .decisions
        .push(next.state().decisions.last().unwrap().clone());
    assert_eq!(next.state(), &expected);
    assert_eq!(next.pins(), original.pins());
}

#[cfg(not(target_arch = "wasm32"))]
pub mod durable {
    use super::*;
    use df_knowledge::perception::{ObserverScope, PerceptionLimits, perceive};
    use df_observe::OperationContext;
    use df_session::inbox::{ActorInput, AdmissionSequence, Reducer};
    use df_session::submission::*;
    use df_types::{MemberId, SessionId};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Eq, PartialEq)]
    pub struct Scope {
        pub input: GameInput,
        pub principal: MemberId,
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
    pub enum Failure {
        None,
        BeforeCommit,
        LostAck,
        UnknownNotCommitted,
        Publication,
        SourceWithdrawn,
        FailedTail,
    }
    pub struct Database {
        pub checkpoint: Checkpoint,
        pub ledger: Vec<(Scope, DecisionReceipt)>,
        pub failure: Failure,
        pub commits: usize,
        pub decisions: usize,
        pub publications: usize,
        pub wakes: usize,
        pub reloads: usize,
    }
    pub struct Repository {
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
            assert_preserved(&db.checkpoint, candidate);
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
            let mut db = self.database.borrow_mut();
            db.reloads += 1;
            Ok(db.checkpoint.clone())
        }
    }
    pub struct Engine {
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
            let failure = self.database.borrow().failure;
            if failure == Failure::SourceWithdrawn {
                self.source.refusal.set(Some(SourceRefusal::Withdrawn));
            }
            let mut eligible = self.fixture.eligible();
            if failure == Failure::FailedTail {
                eligible[1].witness = record(42);
            }
            let handler = self
                .fixture
                .handler(&self.source, current.basis(), Some(&eligible));
            registered_handler(&self.fixture, current, input, &handler)
                .map_err(|_| RepositoryError::InvalidCandidate)
        }
        fn validate_recovery(&mut self, current: &Checkpoint) -> Result<(), RepositoryError> {
            current
                .validate_resume(current.basis(), &self.fixture.pins)
                .map(|_| ())
                .map_err(|_| RepositoryError::InvalidCandidate)
        }
    }
    pub struct Publication {
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
            assert!(db.ledger.iter().any(|(prior, receipt)| prior == scope
                && receipt.basis() == checkpoint.basis()));
            assert_eq!(checkpoint.state().knowledge, expected_grants());
            let perceived = perceive(
                checkpoint,
                checkpoint.basis(),
                checkpoint.pins(),
                ObserverScope::Member(scope.principal),
                PerceptionLimits {
                    maximum_scan_records: 128,
                    maximum_member_comparisons: 128,
                    maximum_selected_facts: 128,
                },
            )
            .unwrap();
            assert_eq!(
                perceived
                    .facts()
                    .iter()
                    .map(|fact| fact.id)
                    .collect::<Vec<_>>(),
                vec![fact(30), fact(31)]
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
    pub type Owner = DurableOwner<Repository, Engine, Publication>;
    pub fn setup(failure: Failure) -> (Owner, Rc<RefCell<Database>>, Scope) {
        let fixture = Fixture::new();
        let current = fixture.current();
        let scope = Scope {
            input: fixture.input(current.basis()),
            principal: fixture::member(3),
        };
        let source = SourceOwner::new(&fixture, &current);
        let db = Rc::new(RefCell::new(Database {
            checkpoint: current.clone(),
            ledger: vec![],
            failure,
            commits: 0,
            decisions: 0,
            publications: 0,
            wakes: 0,
            reloads: 0,
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
    pub fn submit(owner: &mut Owner, scope: &Scope) -> SubmissionOutcome {
        let (input, wait) = OwnedInput::new(
            OperationContext {
                trace_parent: String::new(),
                build: "witness-fixture".to_owned(),
            },
            scope.clone(),
            scope.input.clone(),
        );
        owner.reduce(AdmissionSequence(1), input);
        wait.try_recv().unwrap()
    }
}
