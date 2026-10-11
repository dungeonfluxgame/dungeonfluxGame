#![cfg(not(target_arch = "wasm32"))]

//! Executable D01 contract using the real actor and durable owner with an explicitly
//! scripted in-memory repository. This is not hosted authorization or PostgreSQL proof.
use df_engine::host_pause::{
    HostPauseContext, HostPauseError, HostPausePolicy, committed_pause,
    guard_pause_time_transition, stage_host_pause,
};
use df_model::checkpoint::*;
use df_model::commands::{CommandError, CommandLimits};
use df_observe::OperationContext;
use df_session::inbox::{ActorInput, bounded_inbox};
use df_session::submission::*;
use df_types::{
    BuildIdentity, MemberId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId,
    SessionRevision,
};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RECEIPT_LIMIT: usize = 65536;
fn label(text: &str) -> RevisionLabel {
    RevisionLabel::new(Some(text)).unwrap()
}
fn operation(number: u8) -> OperationId {
    OperationId::from_bytes(&[number; 16]).unwrap()
}
fn host() -> MemberId {
    MemberId::from_bytes(&[73; 16]).unwrap()
}
fn entity() -> EntityId {
    EntityId::from_bytes(&[74; 16]).unwrap()
}
fn basis() -> Basis {
    Basis {
        session: SessionId::from_bytes(&[71; 16]).unwrap(),
        run: RunId::from_bytes(&[72; 16]).unwrap(),
        revision: SessionRevision::new(RecoveryEpoch::new(7).unwrap(), 20),
    }
}
fn content(entry: &str) -> ContentReference {
    ContentReference {
        package: label("pause-contract-package"),
        entry: label(entry),
    }
}
fn rule() -> RuleReference {
    RuleReference {
        catalog: label("pause-contract-catalog"),
        source: label("pause-contract-source"),
        entry: label("held-reaction"),
        clause: label("held-reaction"),
    }
}
fn pins() -> CheckpointPins {
    CheckpointPins {
        rules: RulesPins {
            mode: RulesMode::Standard2024,
            ruleset: label("pause-contract-rules"),
            catalog: label("pause-contract-catalog"),
            catalog_digest: ContentDigest([11; 32]),
            source_manifest: label("pause-contract-sources"),
            source_manifest_digest: ContentDigest([12; 32]),
            handler: label("pause-contract-handler"),
            handler_digest: ContentDigest([13; 32]),
        },
        content: ContentPins {
            content: label("pause-contract-content"),
            content_digest: ContentDigest([14; 32]),
            package: label("pause-contract-package"),
            package_digest: ContentDigest([15; 32]),
        },
        build: BuildIdentity::new(
            Some("F21-D01-canonical-contract"),
            Some("native"),
            Some("wasm-unperformed"),
            Some("fixture"),
            Some("pause-contract-content"),
        )
        .unwrap(),
    }
}
fn limits() -> CheckpointLimits {
    CheckpointLimits {
        maximum_records: 128,
        maximum_text_bytes: 512,
        maximum_total_text_bytes: 8192,
        maximum_retained_bytes: 1048576,
    }
}
struct Selection {
    pins: CheckpointPins,
    content: [ContentReference; 4],
    rules: [RuleReference; 1],
    resources: [ResourceConstraint; 1],
    source_policy: RevisionLabel,
    initially_paused: bool,
}
impl Selection {
    fn new(initially_paused: bool) -> Self {
        Self {
            pins: pins(),
            content: [
                content("scenario"),
                content("host-pause-policy"),
                content("host-paused"),
                content("host-resumed"),
            ],
            rules: [rule()],
            resources: [ResourceConstraint {
                owner: entity(),
                resource: label("fixture-resource"),
                minimum: 0,
                maximum: 8,
                source: rule(),
            }],
            source_policy: label("source-selected-host-pause-v1"),
            initially_paused,
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
    fn context(&self, current: &Checkpoint) -> HostPauseContext<'_> {
        HostPauseContext {
            current_basis: current.basis(),
            admitted_pins: &self.pins,
            inventory: self.inventory(),
            checkpoint_limits: limits(),
            command_limits: CommandLimits {
                maximum_records: 128,
                maximum_text_bytes: 512,
                maximum_retained_bytes: 8192,
            },
            policy: HostPausePolicy {
                command: &self.content[1],
                paused_event: &self.content[2],
                resumed_event: &self.content[3],
                source_policy: &self.source_policy,
                initially_paused: self.initially_paused,
            },
        }
    }
    fn checkpoint(&self, at: Basis, state: GameState) -> Checkpoint {
        Checkpoint::new(
            CHECKPOINT_SCHEMA,
            at,
            self.pins.clone(),
            state,
            self.inventory(),
            limits(),
        )
        .unwrap()
    }
}
fn context() -> OperationContext {
    OperationContext {
        trace_parent: String::new(),
        build: "F21-D01-canonical-contract".to_owned(),
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
            definition: content("scenario"),
            active_beats: vec![],
            completed_beats: vec![],
            open_threads: vec![],
            accepted_facts: vec![],
            remaining_budget: 0,
        },
        encounters: vec![],
        activity: vec![],
        tempo: TempoState {
            policy: content("scenario"),
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

fn current(selection: &Selection) -> Checkpoint {
    let mut state = empty_state();
    state.logical_time = LogicalTime {
        ticks: 120,
        ticks_per_second: 10,
    };
    state.members.push(MembershipLink {
        member: host(),
        character: Some(entity()),
    });
    state.entities.push(WorldEntity {
        id: entity(),
        definition: content("scenario"),
        location: None,
        position: None,
        identity_revision: label("fixture-entity"),
    });
    state.resources.push(ResourceState {
        owner: entity(),
        resource: label("fixture-resource"),
        value: 4,
        minimum: 0,
        maximum: 8,
        source: rule(),
    });
    let seed = FactId::from_bytes(&[70; 16]).unwrap();
    let timer = TimerId::from_bytes(&[60; 16]).unwrap();
    let effect = EffectId::from_bytes(&[61; 16]).unwrap();
    state.facts.push(GameFact {
        id: seed,
        revision: basis().revision,
        operation: operation(65),
        ordinal: 0,
        cause: None,
        audience: AudienceScope::Shared,
        value: FactValue::ContentEvent {
            definition: content("scenario"),
            subjects: vec![entity()],
        },
    });
    state.timers.push(OwnedTimer {
        id: timer,
        basis: basis(),
        generation: 9,
        due: LogicalTime {
            ticks: 127,
            ticks_per_second: 10,
        },
        source: rule(),
        status: DurableStatus::Pending,
    });
    state.intents.push(DurableIntent {
        id: effect,
        basis: basis(),
        operation: operation(65),
        slot: 0,
        kind: EffectKind::ArmTimer,
        job: None,
        timer: Some(timer),
        generation: 9,
        status: DurableStatus::Pending,
        definition: content("scenario"),
    });
    state.decisions.push(AcceptedDecision {
        operation: operation(65),
        revision: basis().revision,
        facts: vec![seed],
        draws: vec![],
        effects: vec![effect],
        source_policy: label("fixture-seed-policy"),
        semantic_output: None,
    });
    state.pending.push(PendingResolution {
        id: ResolutionId::from_bytes(&[62; 16]).unwrap(),
        basis: basis(),
        continuation: label("keep-source-reaction"),
        window: ResolutionWindow {
            id: WindowId::from_bytes(&[63; 16]).unwrap(),
            causal_fact: seed,
            source: rule(),
            phase: TriggerPhase::BeforeDraw,
            timer: Some(timer),
        },
        next: PendingInput::Roll {
            participant: host(),
            sides: vec![20],
            source: rule(),
        },
        choices: vec![],
        draw_ordinals: vec![],
        spent: vec![],
        rulings: vec![],
    });
    state.schedules.push(ScheduledEvent {
        id: RecordId::from_bytes(&[64; 16]).unwrap(),
        entity: entity(),
        due: LogicalTime {
            ticks: 127,
            ticks_per_second: 10,
        },
        definition: content("scenario"),
    });
    selection.checkpoint(basis(), state)
}
fn input(current: &Checkpoint, number: u8, paused: bool) -> GameInput {
    GameInput::Host(HostInput {
        basis: current.basis(),
        operation: operation(number),
        host: host(),
        command: HostCommand::SetPause {
            paused,
            policy: content("host-pause-policy"),
        },
    })
}
fn stage(selection: &Selection, current: &Checkpoint, number: u8, paused: bool) -> Checkpoint {
    stage_host_pause(
        current,
        &input(current, number, paused),
        FactId::from_bytes(operation(number).as_bytes()).unwrap(),
        &selection.context(current),
    )
    .unwrap()
}
fn assert_carried(before: &Checkpoint, after: &Checkpoint) {
    let mut carried = after.state().clone();
    carried.facts.truncate(before.state().facts.len());
    carried.decisions.truncate(before.state().decisions.len());
    for pending in &mut carried.pending {
        assert_eq!(pending.basis, after.basis());
        pending.basis = before.basis();
    }
    assert_eq!(&carried, before.state());
    assert_eq!(after.pins(), before.pins());
    assert_eq!(after.basis().session, before.basis().session);
    assert_eq!(after.basis().run, before.basis().run);
}
#[derive(Clone)]
struct Scope {
    input: GameInput,
}
impl ActorInput for Scope {
    fn retained_heap_bytes(&self) -> Option<usize> {
        self.input.retained_heap_bytes()
    }
}
impl OperationScope for Scope {
    type UncertaintyKey = GameInput;
    fn capture_uncertainty_key(&self, maximum: usize) -> Result<GameInput, RepositoryError> {
        let bytes = std::mem::size_of::<GameInput>()
            .checked_add(
                self.input
                    .retained_heap_bytes()
                    .ok_or(RepositoryError::Capacity)?,
            )
            .ok_or(RepositoryError::Capacity)?;
        if bytes > maximum {
            return Err(RepositoryError::Capacity);
        }
        Ok(self.input.clone())
    }
    fn session(&self) -> SessionId {
        let GameInput::Host(host) = &self.input else {
            panic!("fixture host input");
        };
        host.basis.session
    }
    fn operation(&self) -> OperationId {
        let GameInput::Host(host) = &self.input else {
            panic!("fixture host input");
        };
        host.operation
    }
    fn validate_input(&self, input: &GameInput) -> Result<(), RepositoryError> {
        if input == &self.input {
            Ok(())
        } else {
            Err(RepositoryError::InputBinding)
        }
    }
    fn is_lookup_only(&self) -> bool {
        false
    }
}
#[derive(Clone, Copy)]
enum CommitBehavior {
    Acknowledge,
    FailBeforeCommit,
    LoseAcknowledgement,
}
struct Observations {
    current: Checkpoint,
    current_host: Option<MemberId>,
    revoke_at_commit: bool,
    behavior: CommitBehavior,
    receipts: Vec<(GameInput, DecisionReceipt)>,
    events: Vec<&'static str>,
}
#[derive(Clone)]
struct Shared(Arc<Mutex<Observations>>);
fn check_host(observed: &Observations, scope: &Scope) -> Result<(), RepositoryError> {
    let GameInput::Host(host) = &scope.input else {
        return Err(RepositoryError::InputBinding);
    };
    if observed.current_host != Some(host.host) {
        return Err(RepositoryError::Unauthorized);
    }
    if host.basis.session != observed.current.basis().session {
        return Err(RepositoryError::InputBinding);
    }
    Ok(())
}
impl SessionRepository for Shared {
    type Scope = Scope;
    fn lookup_operation(
        &mut self,
        scope: &Scope,
        _: &OperationContext,
    ) -> Result<OperationLookup, RepositoryError> {
        let mut observed = self.0.lock().unwrap();
        observed.events.push("lookup");
        check_host(&observed, scope)?;
        Ok(observed
            .receipts
            .iter()
            .find(|(input, _)| input == &scope.input)
            .map(|(_, receipt)| OperationLookup::Committed(receipt.clone()))
            .unwrap_or(OperationLookup::NotRecorded))
    }
    fn commit_decision(
        &mut self,
        scope: &Scope,
        candidate: &Checkpoint,
        expected: Basis,
        _: &OperationContext,
    ) -> Result<CommitOutcome, RepositoryError> {
        let mut observed = self.0.lock().unwrap();
        observed.events.push("commit_enter");
        if observed.revoke_at_commit {
            observed.current_host = None;
        }
        check_host(&observed, scope)?;
        assert_eq!(observed.current.basis(), expected);
        if matches!(observed.behavior, CommitBehavior::FailBeforeCommit) {
            return Err(RepositoryError::Unavailable);
        }
        let decision = candidate
            .state()
            .decisions
            .iter()
            .find(|decision| decision.operation == scope.operation())
            .unwrap()
            .clone();
        let receipt = DecisionReceipt::new(candidate.basis(), decision, RECEIPT_LIMIT).unwrap();
        observed.current = candidate.clone();
        observed
            .receipts
            .push((scope.input.clone(), receipt.clone()));
        observed.events.push("commit_ack");
        if matches!(observed.behavior, CommitBehavior::LoseAcknowledgement) {
            return Ok(CommitOutcome::Indeterminate);
        }
        Ok(CommitOutcome::Confirmed(receipt))
    }
    fn load_current(
        &mut self,
        scope: &Scope,
        _: &OperationContext,
    ) -> Result<Checkpoint, RepositoryError> {
        let mut observed = self.0.lock().unwrap();
        observed.events.push("reload");
        check_host(&observed, scope)?;
        Ok(observed.current.clone())
    }
}
struct Engine {
    source: Selection,
    observed: Shared,
}
impl SessionEngine<Scope> for Engine {
    fn decide(
        &mut self,
        current: &Checkpoint,
        scope: &Scope,
        input: &GameInput,
    ) -> Result<Checkpoint, RepositoryError> {
        scope.validate_input(input)?;
        self.observed.0.lock().unwrap().events.push("engine");
        let candidate = stage_host_pause(
            current,
            input,
            FactId::from_bytes(scope.operation().as_bytes()).unwrap(),
            &self.source.context(current),
        )
        .map_err(|_| RepositoryError::InvalidCandidate)?;
        guard_pause_time_transition(current, &candidate, &self.source.context(current))
            .map_err(|_| RepositoryError::InvalidCandidate)?;
        Ok(candidate)
    }
    fn validate_recovery(&mut self, current: &Checkpoint) -> Result<(), RepositoryError> {
        committed_pause(current, &self.source.context(current))
            .map(|_| ())
            .map_err(|_| RepositoryError::InvalidCandidate)
    }
}
impl PublicationOwner<Scope> for Shared {
    fn publish_committed(
        &mut self,
        scope: &Scope,
        checkpoint: &Checkpoint,
    ) -> Result<(), DeliveryError> {
        let mut observed = self.0.lock().unwrap();
        check_host(&observed, scope).map_err(|_| DeliveryError::AccessRevoked)?;
        assert_eq!(checkpoint, &observed.current);
        observed.events.push("publish");
        Ok(())
    }
    fn wake_committed_intents(&mut self, _: &Scope) -> Result<(), DeliveryError> {
        self.0.lock().unwrap().events.push("wake");
        Ok(())
    }
}
type Owner = DurableOwner<Shared, Engine, Shared>;
fn owner_for(shared: &Shared, initially_paused: bool) -> Owner {
    let current = shared.0.lock().unwrap().current.clone();
    DurableOwner::new(
        shared.clone(),
        Engine {
            source: Selection::new(initially_paused),
            observed: shared.clone(),
        },
        shared.clone(),
        current,
        RECEIPT_LIMIT,
    )
    .unwrap()
}
fn fixture(initially_paused: bool) -> (Shared, Owner) {
    let source = Selection::new(initially_paused);
    let shared = Shared(Arc::new(Mutex::new(Observations {
        current: current(&source),
        current_host: Some(host()),
        revoke_at_commit: false,
        behavior: CommitBehavior::Acknowledge,
        receipts: vec![],
        events: vec![],
    })));
    let owner = owner_for(&shared, initially_paused);
    (shared, owner)
}
fn owned(input: GameInput) -> (OwnedInput<Scope>, Receiver<SubmissionOutcome>) {
    OwnedInput::new(
        context(),
        Scope {
            input: input.clone(),
        },
        input,
    )
}
fn submit(owner: &mut Owner, input: GameInput) -> SubmissionOutcome {
    let (item, receipt) = owned(input);
    let (inbox, actor) = bounded_inbox();
    assert!(inbox.try_submit(item).is_ok());
    assert_eq!(receipt.try_recv(), Err(TryRecvError::Empty));
    inbox.stop().unwrap();
    assert_eq!(actor.run(owner).unwrap().reduced_inputs, 1);
    receipt.try_recv().unwrap()
}
fn confirm(result: SubmissionOutcome) -> DecisionReceipt {
    match result {
        SubmissionOutcome::Confirmed(receipt) => receipt,
        other => panic!("expected confirmed pause, got {other:?}"),
    }
}
fn events(shared: &Shared) -> Vec<&'static str> {
    shared.0.lock().unwrap().events.clone()
}
fn time_candidate(source: &Selection, current: &Checkpoint, time: LogicalTime) -> Checkpoint {
    let mut next = current.basis();
    next.revision = next.revision.next_sequence().unwrap();
    let mut state = current.state().clone();
    state.logical_time = time;
    for pending in &mut state.pending {
        pending.basis = next;
    }
    source.checkpoint(next, state)
}

#[test]
fn acknowledged_pause_reaches_real_actor_and_preserves_pending_owned_records() {
    let source = Selection::new(false);
    let (shared, mut owner) = fixture(false);
    let before = owner.checkpoint().clone();
    let command = input(&before, 1, true);
    let receipt = confirm(submit(&mut owner, command));
    assert_eq!(
        receipt.basis().revision,
        before.basis().revision.next_sequence().unwrap()
    );
    assert!(committed_pause(owner.checkpoint(), &source.context(owner.checkpoint())).unwrap());
    assert_carried(&before, owner.checkpoint());
    assert_eq!(
        events(&shared),
        [
            "lookup",
            "engine",
            "commit_enter",
            "commit_ack",
            "publish",
            "wake"
        ]
    );
}

#[test]
fn explicit_same_value_and_exact_lost_receipt_retry_never_toggle_or_duplicate() {
    let source = Selection::new(false);
    let (shared, mut owner) = fixture(false);
    let command = input(owner.checkpoint(), 2, true);
    let (item, dropped_receipt) = owned(command.clone());
    let (inbox, actor) = bounded_inbox();
    assert!(inbox.try_submit(item).is_ok());
    drop(dropped_receipt);
    inbox.stop().unwrap();
    actor.run(&mut owner).unwrap();
    let paused = owner.checkpoint().clone();
    shared.0.lock().unwrap().events.clear();
    confirm(submit(&mut owner, command));
    assert_eq!(owner.checkpoint(), &paused);
    assert_eq!(events(&shared), ["lookup"]);
    let command = input(owner.checkpoint(), 3, true);
    confirm(submit(&mut owner, command));
    assert!(committed_pause(owner.checkpoint(), &source.context(owner.checkpoint())).unwrap());
    assert_carried(&paused, owner.checkpoint());
}

#[test]
fn resume_and_recovery_use_canonical_pause_at_exact_unchanged_time() {
    let source = Selection::new(false);
    let (shared, mut owner) = fixture(false);
    let start = owner.checkpoint().clone();
    let command = input(owner.checkpoint(), 4, true);
    confirm(submit(&mut owner, command));
    drop(owner);
    let mut recovered = owner_for(&shared, false);
    assert!(
        committed_pause(
            recovered.checkpoint(),
            &source.context(recovered.checkpoint())
        )
        .unwrap()
    );
    let paused = recovered.checkpoint().clone();
    let command = input(recovered.checkpoint(), 5, false);
    confirm(submit(&mut recovered, command));
    assert!(
        !committed_pause(
            recovered.checkpoint(),
            &source.context(recovered.checkpoint())
        )
        .unwrap()
    );
    assert_eq!(
        recovered.checkpoint().state().logical_time,
        start.state().logical_time
    );
    assert_carried(&paused, recovered.checkpoint());
}

#[test]
fn positive_process_budget_and_actual_owner_wake_do_not_advance_paused_time() {
    let source = Selection::new(false);
    let (_, mut owner) = fixture(false);
    let command = input(owner.checkpoint(), 6, true);
    confirm(submit(&mut owner, command));
    let before = owner.checkpoint().clone();
    let (inbox, actor) = bounded_inbox::<OwnedInput<Scope>>();
    let deadline = Instant::now();
    let elapsed_budget = Duration::from_secs(86400);
    let mut wakes = 0;
    let outcome = actor
        .run_with_owner_wake(
            &mut owner,
            |_| Some(deadline),
            |owner| {
                wakes += 1;
                assert!(elapsed_budget > Duration::ZERO);
                let mut proposed = owner.checkpoint().state().logical_time;
                proposed.ticks += 7;
                let candidate = time_candidate(&source, owner.checkpoint(), proposed);
                assert_eq!(
                    guard_pause_time_transition(
                        owner.checkpoint(),
                        &candidate,
                        &source.context(owner.checkpoint())
                    ),
                    Err(HostPauseError::PausedTimeAdvance)
                );
                inbox.stop().unwrap();
            },
        )
        .unwrap();
    assert_eq!(wakes, 1);
    assert_eq!(outcome.reduced_inputs, 0);
    assert_eq!(owner.checkpoint(), &before);
    assert_eq!(
        owner.checkpoint().state().timers[0].status,
        DurableStatus::Pending
    );
    assert!(
        owner.checkpoint().state().logical_time.ticks
            < owner.checkpoint().state().timers[0].due.ticks
    );
}

#[test]
fn explicit_initial_pause_is_required_selection_without_a_derived_default() {
    for initial in [false, true] {
        let source = Selection::new(initial);
        let current = current(&source);
        assert_eq!(
            committed_pause(&current, &source.context(&current)).unwrap(),
            initial
        );
        let mut proposed = current.state().logical_time;
        proposed.ticks += 1;
        let candidate = time_candidate(&source, &current, proposed);
        let result = guard_pause_time_transition(&current, &candidate, &source.context(&current));
        assert_eq!(
            result,
            if initial {
                Err(HostPauseError::PausedTimeAdvance)
            } else {
                Ok(())
            }
        );
    }
}

#[test]
fn known_commit_failure_and_lost_ack_preserve_cache_and_never_publish_success() {
    for behavior in [
        CommitBehavior::FailBeforeCommit,
        CommitBehavior::LoseAcknowledgement,
    ] {
        let source = Selection::new(false);
        let (shared, mut owner) = fixture(false);
        let before = owner.checkpoint().clone();
        shared.0.lock().unwrap().behavior = behavior;
        let command = input(&before, 7, true);
        let result = submit(&mut owner, command.clone());
        assert_eq!(
            result,
            match behavior {
                CommitBehavior::FailBeforeCommit =>
                    SubmissionOutcome::Refused(RepositoryError::Unavailable),
                CommitBehavior::LoseAcknowledgement => SubmissionOutcome::LookupRequired,
                CommitBehavior::Acknowledge => unreachable!(),
            }
        );
        assert_eq!(owner.checkpoint(), &before);
        assert!(!events(&shared).contains(&"publish"));
        if matches!(behavior, CommitBehavior::LoseAcknowledgement) {
            shared.0.lock().unwrap().events.clear();
            confirm(submit(&mut owner, command));
            assert!(
                committed_pause(owner.checkpoint(), &source.context(owner.checkpoint())).unwrap()
            );
            assert_eq!(events(&shared), ["lookup", "reload"]);
            assert_eq!(shared.0.lock().unwrap().receipts.len(), 1);
        }
    }
}

#[test]
fn nonhost_revoked_and_revocation_between_stage_and_commit_fail_closed() {
    for case in 0..3 {
        let (shared, mut owner) = fixture(false);
        let before = owner.checkpoint().clone();
        let mut command = input(&before, 8, true);
        match case {
            0 => {
                if let GameInput::Host(host) = &mut command {
                    host.host = MemberId::from_bytes(&[99; 16]).unwrap();
                }
            }
            1 => shared.0.lock().unwrap().current_host = None,
            2 => shared.0.lock().unwrap().revoke_at_commit = true,
            _ => unreachable!(),
        }
        assert_eq!(
            submit(&mut owner, command),
            SubmissionOutcome::Refused(RepositoryError::Unauthorized)
        );
        assert_eq!(owner.checkpoint(), &before);
        assert!(shared.0.lock().unwrap().receipts.is_empty());
        assert!(!events(&shared).contains(&"publish"));
        if case == 2 {
            assert!(events(&shared).contains(&"engine"));
        }
    }
}

#[test]
fn stale_run_epoch_basis_and_foreign_session_refuse_without_commit() {
    for case in 0..4 {
        let (shared, mut owner) = fixture(false);
        let before = owner.checkpoint().clone();
        let mut command = input(&before, 9, true);
        if let GameInput::Host(host) = &mut command {
            match case {
                0 => host.basis.session = SessionId::from_bytes(&[99; 16]).unwrap(),
                1 => host.basis.run = RunId::from_bytes(&[99; 16]).unwrap(),
                2 => host.basis.revision = SessionRevision::new(RecoveryEpoch::new(6).unwrap(), 20),
                3 => host.basis.revision = host.basis.revision.next_sequence().unwrap(),
                _ => unreachable!(),
            }
        }
        assert!(matches!(
            submit(&mut owner, command),
            SubmissionOutcome::Refused(_)
        ));
        assert_eq!(owner.checkpoint(), &before);
        assert!(!events(&shared).contains(&"commit_enter"));
    }
}

#[test]
fn unknown_policy_duplicate_operation_and_limits_refuse_atomically() {
    let source = Selection::new(false);
    let current = current(&source);
    let before = current.clone();
    let mut command = input(&current, 10, true);
    if let GameInput::Host(host) = &mut command {
        host.command = HostCommand::SetPause {
            paused: true,
            policy: content("unadmitted"),
        };
    }
    let fact = FactId::from_bytes(&[10; 16]).unwrap();
    assert_eq!(
        stage_host_pause(&current, &command, fact, &source.context(&current)),
        Err(HostPauseError::InvalidPolicy)
    );
    let command = input(&current, 10, true);
    let mut context = source.context(&current);
    context.command_limits.maximum_retained_bytes = 1;
    assert_eq!(
        stage_host_pause(&current, &command, fact, &context),
        Err(HostPauseError::Command(CommandError::Capacity))
    );
    context = source.context(&current);
    context.checkpoint_limits.maximum_records = 1;
    assert_eq!(
        stage_host_pause(&current, &command, fact, &context),
        Err(HostPauseError::Checkpoint(CheckpointError::Capacity))
    );
    let paused = stage(&source, &current, 10, true);
    assert_eq!(
        stage_host_pause(
            &paused,
            &input(&paused, 10, false),
            FactId::from_bytes(&[11; 16]).unwrap(),
            &source.context(&paused)
        ),
        Err(HostPauseError::DuplicateOperation)
    );
    assert_eq!(current, before);
}

#[test]
fn unsupported_commands_and_unknown_members_cannot_change_pause() {
    let source = Selection::new(false);
    let current = current(&source);
    let mut command = input(&current, 11, true);
    if let GameInput::Host(host) = &mut command {
        host.command = HostCommand::RequestCheckpoint;
    }
    let fact = FactId::from_bytes(&[11; 16]).unwrap();
    assert_eq!(
        stage_host_pause(&current, &command, fact, &source.context(&current)),
        Err(HostPauseError::UnsupportedCommand)
    );
    let mut command = input(&current, 11, true);
    if let GameInput::Host(host) = &mut command {
        host.host = MemberId::from_bytes(&[99; 16]).unwrap();
    }
    assert_eq!(
        stage_host_pause(&current, &command, fact, &source.context(&current)),
        Err(HostPauseError::Command(CommandError::UnknownMember))
    );
}

#[test]
fn matching_unbound_or_malformed_pause_records_refuse_instead_of_fabricating_state() {
    let source = Selection::new(false);
    let current = current(&source);
    let paused = stage(&source, &current, 12, true);
    for case in 0..5 {
        let mut state = paused.state().clone();
        match case {
            0 => {
                state.decisions.pop();
            }
            1 => state.decisions.last_mut().unwrap().source_policy = label("different-policy"),
            2 => state.facts.last_mut().unwrap().audience = AudienceScope::Host,
            3 => {
                if let FactValue::ContentEvent { subjects, .. } =
                    &mut state.facts.last_mut().unwrap().value
                {
                    subjects.push(entity());
                }
            }
            4 => {
                state.decisions.last_mut().unwrap().semantic_output =
                    Some("not-pause-data".to_owned())
            }
            _ => unreachable!(),
        }
        let malformed = source.checkpoint(paused.basis(), state);
        assert_eq!(
            committed_pause(&malformed, &source.context(&malformed)),
            Err(HostPauseError::MalformedPauseRecord)
        );
    }
}

#[test]
fn source_withdrawal_alias_and_changed_pins_refuse_without_staging() {
    let source = Selection::new(false);
    let current = current(&source);
    let mut context = source.context(&current);
    context.policy.resumed_event = context.policy.paused_event;
    assert_eq!(
        committed_pause(&current, &context),
        Err(HostPauseError::InvalidPolicy)
    );
    let mut context = source.context(&current);
    let withdrawn = [content("scenario"), content("host-pause-policy")];
    context.inventory.content = &withdrawn;
    assert_eq!(
        committed_pause(&current, &context),
        Err(HostPauseError::InvalidPolicy)
    );
    let mut changed = source.pins.clone();
    changed.content.content_digest.0[0] ^= 1;
    context = source.context(&current);
    context.admitted_pins = &changed;
    assert_eq!(
        committed_pause(&current, &context),
        Err(HostPauseError::Checkpoint(CheckpointError::ContentMismatch))
    );
}

#[test]
fn candidate_timebase_regression_and_foreign_lineage_refuse_before_commit() {
    let source = Selection::new(false);
    let current = current(&source);
    for time in [
        LogicalTime {
            ticks: 119,
            ticks_per_second: 10,
        },
        LogicalTime {
            ticks: 120,
            ticks_per_second: 11,
        },
    ] {
        let candidate = time_candidate(&source, &current, time);
        assert_eq!(
            guard_pause_time_transition(&current, &candidate, &source.context(&current)),
            Err(HostPauseError::InvalidTime)
        );
    }
    let mut candidate = current.basis();
    candidate.revision = candidate.revision.next_sequence().unwrap();
    candidate.run = RunId::from_bytes(&[99; 16]).unwrap();
    let mut state = empty_state();
    state.logical_time = current.state().logical_time;
    let foreign = source.checkpoint(candidate, state);
    assert_eq!(
        guard_pause_time_transition(&current, &foreign, &source.context(&current)),
        Err(HostPauseError::Checkpoint(CheckpointError::WrongRun))
    );
}
