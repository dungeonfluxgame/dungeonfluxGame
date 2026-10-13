#![cfg(not(target_arch = "wasm32"))]

//! Independent source-contract probes. Scripted repository observations are NOT PostgreSQL proof.
use df_model::checkpoint::*;
use df_observe::OperationContext;
use df_session::inbox::{ActorInput, bounded_inbox};
use df_session::submission::*;
use df_types::{
    BuildIdentity, MemberId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId,
    SessionRevision,
};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

const RECEIPT_LIMIT: usize = 65536;

fn label(text: &str) -> RevisionLabel {
    RevisionLabel::new(Some(text)).unwrap()
}
fn operation(number: u8) -> OperationId {
    OperationId::from_bytes(&[number; 16]).unwrap()
}
fn basis() -> Basis {
    Basis {
        session: SessionId::from_bytes(&[71; 16]).unwrap(),
        run: RunId::from_bytes(&[72; 16]).unwrap(),
        revision: SessionRevision::new(RecoveryEpoch::new(7).unwrap(), 20),
    }
}
fn content() -> ContentReference {
    ContentReference {
        package: label("independent-package"),
        entry: label("empty-scenario"),
    }
}
fn pins() -> CheckpointPins {
    CheckpointPins {
        rules: RulesPins {
            mode: RulesMode::Standard2024,
            ruleset: label("independent-rules"),
            catalog: label("independent-catalog"),
            catalog_digest: ContentDigest([11; 32]),
            source_manifest: label("independent-sources"),
            source_manifest_digest: ContentDigest([12; 32]),
            handler: label("independent-handler"),
            handler_digest: ContentDigest([13; 32]),
        },
        content: ContentPins {
            content: label("independent-content"),
            content_digest: ContentDigest([14; 32]),
            package: label("independent-package"),
            package_digest: ContentDigest([15; 32]),
        },
        build: BuildIdentity::new(
            Some("e302cd6-independent-consumer"),
            Some("native"),
            Some("wasm-unperformed"),
            Some("fixture"),
            Some("independent-content"),
        )
        .unwrap(),
    }
}
fn empty_state() -> GameState {
    GameState {
        mode: ExecutionMode::Replay,
        logical_time: LogicalTime {
            ticks: 0,
            ticks_per_second: 1,
        },
        members: vec![],
        entities: vec![],
        characters: vec![],
        resources: vec![],
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
        continuity: ContinuityState {
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
        },
    }
}
fn checkpoint(at: Basis, state: GameState) -> Checkpoint {
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        at,
        pins(),
        state,
        ReferenceInventory {
            rules: &[],
            content: &[content()],
            resources: &[],
            assets: &[],
        },
        CheckpointLimits {
            maximum_records: 100,
            maximum_text_bytes: 512,
            maximum_total_text_bytes: 8192,
            maximum_retained_bytes: 1048576,
        },
    )
    .unwrap()
}
fn decision(id: OperationId, revision: SessionRevision) -> AcceptedDecision {
    AcceptedDecision {
        operation: id,
        revision,
        facts: vec![],
        draws: vec![],
        effects: vec![],
        source_policy: label("independent-policy"),
        semantic_output: Some("private fixture receipt".to_owned()),
    }
}
fn receipt_for(id: OperationId, at: Basis) -> DecisionReceipt {
    DecisionReceipt::new(at, decision(id, at.revision), RECEIPT_LIMIT).unwrap()
}
fn context() -> OperationContext {
    OperationContext {
        trace_parent: String::new(),
        build: "e302cd6-independent-consumer".to_owned(),
    }
}

// Full identity is a fixture label for existing authenticated scope, not an authority issuer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Key {
    principal: u8,
    namespace: u8,
    recovery_epoch: u64,
    operation: OperationId,
}
#[derive(Clone)]
struct Scope {
    key: Key,
    request_basis: Basis,
    lookup_only: bool,
}
impl ActorInput for Scope {
    fn retained_heap_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
// Fixed tenant/fingerprint bytes are explicit fixture data. Refreshed proof and
// request run/basis are deliberately absent from the retained operation identity.
#[derive(Eq, PartialEq)]
struct RetainedOperationKey {
    tenant: [u8; 16],
    session: SessionId,
    operation: Key,
    fingerprint_version: u16,
    fingerprint: [u8; 32],
}
impl ActorInput for RetainedOperationKey {
    fn retained_heap_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl OperationScope for Scope {
    type UncertaintyKey = RetainedOperationKey;

    fn capture_uncertainty_key(
        &self,
        maximum_retained_bytes: usize,
    ) -> Result<Self::UncertaintyKey, RepositoryError> {
        // All fields are inline; no allocation, hidden capacity, or lossy hash.
        if std::mem::size_of::<RetainedOperationKey>() > maximum_retained_bytes {
            return Err(RepositoryError::Capacity);
        }
        Ok(RetainedOperationKey {
            tenant: [74; 16],
            session: self.session(),
            operation: self.key,
            fingerprint_version: 1,
            fingerprint: [75; 32],
        })
    }

    fn session(&self) -> SessionId {
        self.request_basis.session
    }
    fn operation(&self) -> OperationId {
        self.key.operation
    }
    fn validate_input(&self, input: &GameInput) -> Result<(), RepositoryError> {
        match input {
            GameInput::Host(host)
                if host.basis == self.request_basis && host.operation == self.key.operation =>
            {
                Ok(())
            }
            _ => Err(RepositoryError::InputBinding),
        }
    }
    fn is_lookup_only(&self) -> bool {
        self.lookup_only
    }
}
fn scope(id: u8) -> Scope {
    Scope {
        key: Key {
            principal: 1,
            namespace: 1,
            recovery_epoch: 7,
            operation: operation(id),
        },
        request_basis: basis(),
        lookup_only: false,
    }
}
fn owned(scope: Scope) -> (OwnedInput<Scope>, Receiver<SubmissionOutcome>) {
    let input = GameInput::Host(HostInput {
        basis: scope.request_basis,
        operation: scope.key.operation,
        host: MemberId::from_bytes(&[73; 16]).unwrap(),
        command: HostCommand::RequestCheckpoint,
    });
    OwnedInput::new(context(), scope, input)
}

#[derive(Clone)]
enum Lookup {
    Absent,
    Pending,
    Expired,
    Conflict,
    Error,
    Refused,
    Receipt(DecisionReceipt),
}
#[derive(Clone, Copy)]
enum Commit {
    Acknowledged,
    Unknown,
    Conflict,
    MalformedReceipt,
}
struct Observations {
    lookups: HashMap<Key, Lookup>,
    current: Checkpoint,
    events: Vec<&'static str>,
    commit: Commit,
    fail_reload: bool,
    fail_delivery: bool,
    invalid_candidate: bool,
}
#[derive(Clone)]
struct Shared(Arc<Mutex<Observations>>);
impl SessionRepository for Shared {
    type Scope = Scope;
    fn lookup_operation(
        &mut self,
        scope: &Scope,
        _: &OperationContext,
    ) -> Result<OperationLookup, RepositoryError> {
        let mut observations = self.0.lock().unwrap();
        observations.events.push("lookup");
        match observations
            .lookups
            .get(&scope.key)
            .cloned()
            .unwrap_or(Lookup::Absent)
        {
            Lookup::Absent => Ok(OperationLookup::NotRecorded),
            Lookup::Pending => Ok(OperationLookup::InProgress),
            Lookup::Expired => Ok(OperationLookup::ExpiredOrIndeterminate),
            Lookup::Conflict => Ok(OperationLookup::Conflict),
            Lookup::Error => Err(RepositoryError::Unavailable),
            Lookup::Refused => Err(RepositoryError::InvalidCandidate),
            Lookup::Receipt(receipt) => Ok(OperationLookup::Committed(receipt)),
        }
    }
    fn commit_decision(
        &mut self,
        scope: &Scope,
        candidate: &Checkpoint,
        expected: Basis,
        _: &OperationContext,
    ) -> Result<CommitOutcome, RepositoryError> {
        let mut observations = self.0.lock().unwrap();
        observations.events.push("commit_enter");
        assert_eq!(expected, observations.current.basis());
        match observations.commit {
            Commit::Unknown => {
                observations.events.push("commit_unknown");
                Ok(CommitOutcome::Indeterminate)
            }
            Commit::Conflict => Err(RepositoryError::RevisionConflict),
            Commit::MalformedReceipt => Ok(CommitOutcome::Confirmed(receipt_for(
                operation(250),
                candidate.basis(),
            ))),
            Commit::Acknowledged => {
                let decision = candidate
                    .state()
                    .decisions
                    .iter()
                    .find(|item| item.operation == scope.key.operation)
                    .unwrap()
                    .clone();
                let receipt =
                    DecisionReceipt::new(candidate.basis(), decision, RECEIPT_LIMIT).unwrap();
                observations.current = candidate.clone();
                observations
                    .lookups
                    .insert(scope.key, Lookup::Receipt(receipt.clone()));
                observations.events.push("commit_ack");
                Ok(CommitOutcome::Confirmed(receipt))
            }
        }
    }
    fn load_current(
        &mut self,
        _: &Scope,
        _: &OperationContext,
    ) -> Result<Checkpoint, RepositoryError> {
        let mut observations = self.0.lock().unwrap();
        observations.events.push("reload");
        if observations.fail_reload {
            Err(RepositoryError::Unavailable)
        } else {
            Ok(observations.current.clone())
        }
    }
}
impl SessionEngine<Scope> for Shared {
    fn decide(
        &mut self,
        current: &Checkpoint,
        scope: &Scope,
        _: &GameInput,
    ) -> Result<Checkpoint, RepositoryError> {
        let mut observations = self.0.lock().unwrap();
        observations.events.push("engine");
        let mut next = current.basis();
        next.revision = next.revision.next_sequence().unwrap();
        if observations.invalid_candidate {
            next.run = RunId::from_bytes(&[99; 16]).unwrap();
        }
        let mut state = current.state().clone();
        state
            .decisions
            .push(decision(scope.key.operation, next.revision));
        Ok(checkpoint(next, state))
    }
    fn validate_recovery(&mut self, checkpoint: &Checkpoint) -> Result<(), RepositoryError> {
        self.0.lock().unwrap().events.push("validate_recovery");
        checkpoint
            .validate_resume(checkpoint.basis(), &pins())
            .map(|_| ())
            .map_err(|_| RepositoryError::InvalidCandidate)
    }
}
impl PublicationOwner<Scope> for Shared {
    fn publish_committed(
        &mut self,
        _: &Scope,
        checkpoint: &Checkpoint,
    ) -> Result<(), DeliveryError> {
        let mut observations = self.0.lock().unwrap();
        assert_eq!(checkpoint, &observations.current);
        observations.events.push("publish");
        if observations.fail_delivery {
            Err(DeliveryError::AccessRevoked)
        } else {
            Ok(())
        }
    }
    fn wake_committed_intents(&mut self, _: &Scope) -> Result<(), DeliveryError> {
        let mut observations = self.0.lock().unwrap();
        observations.events.push("wake");
        if observations.fail_delivery {
            Err(DeliveryError::Unavailable)
        } else {
            Ok(())
        }
    }
}
type Owner = DurableOwner<Shared, Shared, Shared>;
fn fixture() -> (Shared, Owner) {
    let current = checkpoint(basis(), empty_state());
    let shared = Shared(Arc::new(Mutex::new(Observations {
        lookups: HashMap::new(),
        current: current.clone(),
        events: vec![],
        commit: Commit::Acknowledged,
        fail_reload: false,
        fail_delivery: false,
        invalid_candidate: false,
    })));
    let owner = DurableOwner::new(
        shared.clone(),
        shared.clone(),
        shared.clone(),
        current,
        RECEIPT_LIMIT,
    )
    .unwrap();
    shared.0.lock().unwrap().events.clear();
    (shared, owner)
}
fn submit(owner: &mut Owner, scope: Scope) -> SubmissionOutcome {
    let (input, receiver) = owned(scope);
    let (inbox, actor) = bounded_inbox();
    assert!(inbox.try_submit(input).is_ok());
    assert_eq!(
        receiver.try_recv(),
        Err(TryRecvError::Empty),
        "admission must not yield receipt"
    );
    inbox.stop().unwrap();
    assert_eq!(actor.run(owner).unwrap().reduced_inputs, 1);
    receiver.try_recv().unwrap()
}
fn events(shared: &Shared) -> Vec<&'static str> {
    shared.0.lock().unwrap().events.clone()
}
fn set_lookup(shared: &Shared, scope: &Scope, lookup: Lookup) {
    shared.0.lock().unwrap().lookups.insert(scope.key, lookup);
}
fn confirmed(outcome: SubmissionOutcome) -> DecisionReceipt {
    match outcome {
        SubmissionOutcome::Confirmed(receipt) => receipt,
        other => panic!("expected confirmation, got {other:?}"),
    }
}

#[test]
fn admission_and_delivery_follow_acknowledged_repository_boundary() {
    let (shared, mut owner) = fixture();
    let receipt = confirmed(submit(&mut owner, scope(1)));
    assert_eq!(receipt.basis().revision.sequence(), 21);
    assert_eq!(
        events(&shared),
        vec![
            "lookup",
            "engine",
            "commit_enter",
            "commit_ack",
            "publish",
            "wake"
        ]
    );
    shared.0.lock().unwrap().events.clear();
    assert_eq!(confirmed(submit(&mut owner, scope(1))), receipt);
    assert_eq!(
        events(&shared),
        vec!["lookup"],
        "retry must neither reduce nor republish"
    );
    assert!(!format!("{receipt:?}").contains("private fixture receipt"));
}

#[test]
fn dropped_waiter_does_not_cancel_accepted_work() {
    let (shared, mut owner) = fixture();
    let (input, receiver) = owned(scope(2));
    let (inbox, actor) = bounded_inbox();
    assert!(inbox.try_submit(input).is_ok());
    drop(receiver);
    inbox.stop().unwrap();
    assert_eq!(actor.run(&mut owner).unwrap().reduced_inputs, 1);
    assert_eq!(
        events(&shared),
        vec![
            "lookup",
            "engine",
            "commit_enter",
            "commit_ack",
            "publish",
            "wake"
        ]
    );
    assert_eq!(owner.checkpoint().basis().revision.sequence(), 21);
}

#[test]
fn delivery_failures_preserve_the_confirmed_receipt() {
    let (shared, mut owner) = fixture();
    shared.0.lock().unwrap().fail_delivery = true;
    confirmed(submit(&mut owner, scope(3)));
    assert_eq!(owner.checkpoint().basis().revision.sequence(), 21);
    assert!(events(&shared).ends_with(&["commit_ack", "publish", "wake"]));
}

#[test]
fn commit_unknown_and_malformed_ack_never_publish_and_block_fresh_keys() {
    for behavior in [Commit::Unknown, Commit::MalformedReceipt] {
        let (shared, mut owner) = fixture();
        shared.0.lock().unwrap().commit = behavior;
        assert_eq!(
            submit(&mut owner, scope(4)),
            SubmissionOutcome::LookupRequired
        );
        assert_eq!(owner.checkpoint().basis(), basis());
        assert_eq!(
            owner.reload_current(&scope(4), &context()),
            Err(RepositoryError::UnresolvedCommit)
        );
        shared.0.lock().unwrap().events.clear();
        assert_eq!(
            submit(&mut owner, scope(5)),
            SubmissionOutcome::LookupRequired
        );
        assert_eq!(events(&shared), vec!["lookup"]);
    }
}

#[test]
fn lookup_uncertainty_and_lookup_only_absence_fence_new_operations() {
    for lookup in [Lookup::Pending, Lookup::Expired, Lookup::Error] {
        let (shared, mut owner) = fixture();
        set_lookup(&shared, &scope(6), lookup);
        assert!(!matches!(
            submit(&mut owner, scope(6)),
            SubmissionOutcome::Confirmed(_)
        ));
        assert_eq!(
            submit(&mut owner, scope(7)),
            SubmissionOutcome::LookupRequired
        );
        assert_eq!(events(&shared), vec!["lookup", "lookup"]);
    }
    let (shared, mut owner) = fixture();
    let mut lookup_only = scope(6);
    lookup_only.lookup_only = true;
    assert_eq!(
        submit(&mut owner, lookup_only),
        SubmissionOutcome::LookupRequired
    );
    assert_eq!(
        submit(&mut owner, scope(7)),
        SubmissionOutcome::LookupRequired
    );
    assert_eq!(events(&shared), vec!["lookup", "lookup"]);
}

#[test]
fn definitive_refusals_leave_the_owner_serving_the_next_operation() {
    // A lookup that refuses definitively (nothing was written, the cache was
    // current) must not fence the owner: the next operation is served.
    let (shared, mut owner) = fixture();
    set_lookup(&shared, &scope(6), Lookup::Refused);
    assert_eq!(
        submit(&mut owner, scope(6)),
        SubmissionOutcome::Refused(RepositoryError::InvalidCandidate)
    );
    assert!(owner.is_current(), "a definitive refusal must not fence");
    confirmed(submit(&mut owner, scope(7)));

    // The same after the engine refuses a candidate: refused once, then served.
    let (shared, mut owner) = fixture();
    shared.0.lock().unwrap().invalid_candidate = true;
    assert_eq!(
        submit(&mut owner, scope(6)),
        SubmissionOutcome::Refused(RepositoryError::InvalidCandidate)
    );
    shared.0.lock().unwrap().invalid_candidate = false;
    assert!(owner.is_current(), "a refused candidate must not fence");
    confirmed(submit(&mut owner, scope(7)));
}

#[test]
fn exact_scope_receipt_and_valid_reload_release_the_barrier() {
    let (shared, mut owner) = fixture();
    let unresolved = scope(8);
    set_lookup(&shared, &unresolved, Lookup::Pending);
    assert_eq!(
        submit(&mut owner, unresolved.clone()),
        SubmissionOutcome::LookupRequired
    );
    let retained = receipt_for(unresolved.operation(), basis());
    {
        let mut observations = shared.0.lock().unwrap();
        let mut state = observations.current.state().clone();
        state.decisions.push(retained.decision().clone());
        observations.current = checkpoint(basis(), state);
    }
    set_lookup(&shared, &unresolved, Lookup::Receipt(retained));
    confirmed(submit(&mut owner, unresolved));
    confirmed(submit(&mut owner, scope(9)));
    assert!(
        events(&shared)
            .windows(2)
            .any(|pair| pair == ["reload", "validate_recovery"])
    );
}

#[test]
fn failed_or_regressing_reload_cannot_release_uncertainty() {
    for regress in [false, true] {
        let (shared, mut owner) = fixture();
        let unresolved = scope(10);
        set_lookup(&shared, &unresolved, Lookup::Pending);
        assert_eq!(
            submit(&mut owner, unresolved.clone()),
            SubmissionOutcome::LookupRequired
        );
        let mut receipt_basis = basis();
        if regress {
            receipt_basis.revision = receipt_basis.revision.next_sequence().unwrap();
        } else {
            shared.0.lock().unwrap().fail_reload = true;
        }
        set_lookup(
            &shared,
            &unresolved,
            Lookup::Receipt(receipt_for(unresolved.operation(), receipt_basis)),
        );
        confirmed(submit(&mut owner, unresolved));
        shared.0.lock().unwrap().events.clear();
        assert_eq!(
            submit(&mut owner, scope(11)),
            SubmissionOutcome::LookupRequired
        );
        assert_eq!(events(&shared), vec!["lookup"]);
    }
}

#[test]
fn different_operation_id_receipt_does_not_resolve_original_unknown() {
    let (shared, mut owner) = fixture();
    set_lookup(&shared, &scope(12), Lookup::Pending);
    assert_eq!(
        submit(&mut owner, scope(12)),
        SubmissionOutcome::LookupRequired
    );
    set_lookup(
        &shared,
        &scope(13),
        Lookup::Receipt(receipt_for(operation(13), basis())),
    );
    confirmed(submit(&mut owner, scope(13)));
    assert_eq!(
        submit(&mut owner, scope(14)),
        SubmissionOutcome::LookupRequired
    );
    assert_eq!(events(&shared), vec!["lookup", "lookup", "lookup"]);
}

#[test]
fn revision_conflict_needs_explicit_reload_before_new_reduction() {
    let (shared, mut owner) = fixture();
    shared.0.lock().unwrap().commit = Commit::Conflict;
    assert_eq!(
        submit(&mut owner, scope(15)),
        SubmissionOutcome::Refused(RepositoryError::RevisionConflict)
    );
    assert_eq!(
        submit(&mut owner, scope(16)),
        SubmissionOutcome::LookupRequired
    );
    owner.reload_current(&scope(16), &context()).unwrap();
    shared.0.lock().unwrap().commit = Commit::Acknowledged;
    confirmed(submit(&mut owner, scope(16)));
}

#[test]
fn changed_run_candidate_and_fingerprint_conflict_refuse_before_commit() {
    let (shared, mut owner) = fixture();
    shared.0.lock().unwrap().invalid_candidate = true;
    assert_eq!(
        submit(&mut owner, scope(17)),
        SubmissionOutcome::Refused(RepositoryError::InvalidCandidate)
    );
    assert_eq!(events(&shared), vec!["lookup", "engine"]);
    shared.0.lock().unwrap().events.clear();
    set_lookup(&shared, &scope(18), Lookup::Conflict);
    assert_eq!(
        submit(&mut owner, scope(18)),
        SubmissionOutcome::OperationConflict
    );
    assert_eq!(events(&shared), vec!["lookup"]);
}

fn assert_scoped_collision_stays_fenced(dimension: &str) {
    let (shared, mut owner) = fixture();
    let unresolved = scope(20);
    let mut historical = unresolved.clone();
    match dimension {
        "principal" => historical.key.principal = 2,
        "namespace" => historical.key.namespace = 2,
        "recovery_epoch" => historical.key.recovery_epoch = 6,
        _ => panic!("unknown fixture dimension"),
    }
    historical.request_basis.run = RunId::from_bytes(&[70; 16]).unwrap();
    historical.request_basis.revision = SessionRevision::new(
        RecoveryEpoch::new(historical.key.recovery_epoch).unwrap(),
        19,
    );
    historical.lookup_only = true;
    assert_ne!(unresolved.key, historical.key);
    assert_eq!(unresolved.operation(), historical.operation());
    set_lookup(&shared, &unresolved, Lookup::Pending);
    set_lookup(
        &shared,
        &historical,
        Lookup::Receipt(receipt_for(
            historical.operation(),
            historical.request_basis,
        )),
    );
    assert_eq!(
        submit(&mut owner, unresolved.clone()),
        SubmissionOutcome::LookupRequired
    );
    confirmed(submit(&mut owner, historical));
    // The other scoped receipt is truthful, but it has resolved no fact about A.
    assert!(matches!(
        shared.0.lock().unwrap().lookups.get(&unresolved.key),
        Some(Lookup::Pending)
    ));
    shared.0.lock().unwrap().events.clear();
    let outcome = submit(&mut owner, scope(21));
    let observed_events = events(&shared);
    println!(
        "scope_collision dimension={dimension}; unresolved_A_still_pending=true; C={outcome:?}; events={observed_events:?}"
    );
    assert_eq!(
        outcome,
        SubmissionOutcome::LookupRequired,
        "a retained receipt from another {dimension} must not clear A's uncertainty"
    );
    assert_eq!(observed_events, vec!["lookup"]);
}

#[test]
fn same_operation_id_other_principal_must_not_clear_unknown() {
    assert_scoped_collision_stays_fenced("principal");
}
#[test]
fn same_operation_id_other_namespace_must_not_clear_unknown() {
    assert_scoped_collision_stays_fenced("namespace");
}
#[test]
fn same_operation_id_other_recovery_epoch_must_not_clear_unknown() {
    assert_scoped_collision_stays_fenced("recovery_epoch");
}
