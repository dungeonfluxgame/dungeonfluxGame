//! Executable Design contract for the registered prepared-only courier consumer.
use super::*;
use df_ai::lookup::{LookupError, lookup_prepared, lookup_replay};
use df_types::{BuildIdentity, LocaleTag};

fn authored(current: &Checkpoint) -> AuthoredRecording<'_> {
    let effect = intent(current);
    AuthoredRecording {
        current,
        intent: effect,
        key: model::content(DEFINITION).unwrap(),
        basis: recording_basis(current, effect).unwrap(),
        response: response(),
    }
}

fn complete_events(
    effect: &DurableIntent,
    text: &str,
) -> [Result<RecordEvent<JobId, Basis>, RecordInputError>; 2] {
    [
        Ok(RecordEvent::Chunk(text.as_bytes().to_vec())),
        Ok(RecordEvent::Complete {
            identity: RecordIdentity {
                key: effect.job.unwrap(),
                basis: effect.basis,
                operation: effect.operation,
            },
            byte_length: text.len() as u64,
            sha256: Sha256::digest(text.as_bytes()).into(),
        }),
    ]
}

fn withdraw_source(current: &Checkpoint, case: u8) -> Checkpoint {
    let mut state = current.state().clone();
    match case {
        0 => {
            state
                .facts
                .iter_mut()
                .find(|fact| {
                    fact.operation == intent(current).operation
                        && matches!(fact.value, FactValue::ContentEvent { .. })
                })
                .unwrap()
                .audience = AudienceScope::Shared;
        }
        1 => {
            state
                .facts
                .iter_mut()
                .find(|fact| {
                    fact.operation == intent(current).operation
                        && matches!(fact.value, FactValue::ContentEvent { .. })
                })
                .unwrap()
                .audience = AudienceScope::Host
        }
        2 => {
            state
                .facts
                .iter_mut()
                .find(|fact| {
                    fact.operation == intent(current).operation
                        && matches!(fact.value, FactValue::ContentEvent { .. })
                })
                .unwrap()
                .value = FactValue::ContentEvent {
                definition: model::content("escort-courier").unwrap(),
                subjects: vec![],
            }
        }
        3 => {
            state
                .decisions
                .iter_mut()
                .find(|decision| decision.operation == intent(current).operation)
                .unwrap()
                .source_policy = model::label("unadmitted-source-policy").unwrap()
        }
        _ => unreachable!(),
    }
    model::checkpoint(current.basis(), state).unwrap()
}

#[test]
fn exact_prepared_entry_is_borrowed_and_complete_before_candidate_admission() {
    let (current, first, second) = pending();
    let original = current.clone();
    let recording = authored(&current);
    let text = lookup_prepared(&recording, &recording.key, &recording.basis).unwrap();
    assert_eq!(*text, RESPONSE);
    assert_eq!(recording.basis.source_locale.as_str(), "en");
    let completion = admit_record(
        &current,
        intent(&current),
        complete_events(intent(&current), text),
    )
    .unwrap();
    assert_eq!(completion, execute(&current, intent(&current)).unwrap());
    assert_eq!(current, original);
    assert!(player_clue(&current, first).is_empty());
    assert!(player_clue(&current, second).is_empty());
    let completed = stage_completion(
        &current,
        &completion,
        completion_operation(intent(&current)).unwrap(),
    )
    .unwrap();
    assert_eq!(player_clue(&completed, first), RESPONSE);
    assert!(player_clue(&completed, second).is_empty());
}

#[test]
fn complete_source_basis_and_key_mismatches_are_stale_without_fallback() {
    let (current, _, _) = pending();
    let recording = authored(&current);
    for case in 0..15 {
        let mut basis = recording.basis.clone();
        let changed = model::label("different-admitted-source").unwrap();
        match case {
            0 => basis.pins.rules.mode = RulesMode::DisclosedCustom,
            1 => basis.pins.rules.ruleset = changed,
            2 => basis.pins.rules.catalog = changed,
            3 => basis.pins.rules.catalog_digest.0[0] ^= 1,
            4 => basis.pins.rules.source_manifest = changed,
            5 => basis.pins.rules.source_manifest_digest.0[0] ^= 1,
            6 => basis.pins.rules.handler = changed,
            7 => basis.pins.rules.handler_digest.0[0] ^= 1,
            8 => basis.pins.content.content = changed,
            9 => basis.pins.content.content_digest.0[0] ^= 1,
            10 => basis.pins.content.package = changed,
            11 => basis.pins.content.package_digest.0[0] ^= 1,
            12 => {
                basis.pins.build = BuildIdentity::new(
                    Some("other-source"),
                    Some("other-native"),
                    Some("other-wasm"),
                    Some("other-config"),
                    Some("other-content"),
                )
                .unwrap()
            }
            13 => basis.policy = model::content("harbor").unwrap(),
            14 => basis.model = changed,
            _ => unreachable!(),
        }
        assert_eq!(
            lookup_prepared(&recording, &recording.key, &basis),
            Err(LookupError::Stale),
            "case {case}"
        );
        assert_eq!(
            lookup_replay(&recording, &recording.key, &basis),
            Err(LookupError::Missing)
        );
    }
    assert_eq!(
        lookup_prepared(
            &recording,
            &model::content("harbor").unwrap(),
            &recording.basis
        ),
        Err(LookupError::Stale)
    );
    assert_eq!(
        *lookup_prepared(&recording, &recording.key, &recording.basis).unwrap(),
        RESPONSE
    );
}

#[test]
fn locale_case_normalization_preserves_exact_authored_source_identity() {
    let (current, _, _) = pending();
    let recording = authored(&current);
    let mut basis = recording.basis.clone();
    basis.source_locale = LocaleTag::parse("EN").unwrap();
    assert_eq!(
        *lookup_prepared(&recording, &recording.key, &basis).unwrap(),
        RESPONSE
    );
    for locale in ["en-US", "en-GB", "fr", "iw", "he"] {
        basis.source_locale = LocaleTag::parse(locale).unwrap();
        assert_eq!(
            lookup_prepared(&recording, &recording.key, &basis),
            Err(LookupError::Stale),
            "{locale}"
        );
    }
    assert_eq!(recording.basis.source_locale.as_str(), "en");
}

#[test]
fn recording_replay_missing_and_unapproved_modes_never_select_prepared_fallback() {
    let (current, _, _) = pending();
    let recording = authored(&current);
    assert_eq!(
        lookup_replay(&recording, &recording.key, &recording.basis),
        Err(LookupError::Missing)
    );
    assert!(lookup_prepared(&recording, &recording.key, &recording.basis).is_ok());
    for mode in [ExecutionMode::Replay, ExecutionMode::Live] {
        let mut state = current.state().clone();
        state.mode = mode;
        let changed = model::checkpoint(current.basis(), state).unwrap();
        let original = changed.clone();
        assert_eq!(
            execute(&changed, intent(&changed)),
            Err(CourierError::Unavailable)
        );
        assert_eq!(changed, original);
    }
}

#[test]
fn complete_prepared_bytes_cannot_publish_after_current_access_or_source_withdrawal() {
    let (current, _, _) = pending();
    let recording = authored(&current);
    let text = lookup_prepared(&recording, &recording.key, &recording.basis).unwrap();
    let completion = admit_record(
        &current,
        intent(&current),
        complete_events(intent(&current), text),
    )
    .unwrap();
    for case in 0..4 {
        let changed = withdraw_source(&current, case);
        let original = changed.clone();
        assert!(
            admit_record(
                &changed,
                intent(&changed),
                complete_events(intent(&changed), text)
            )
            .is_err(),
            "case {case}"
        );
        assert_eq!(
            stage_completion(
                &changed,
                &completion,
                completion_operation(intent(&changed)).unwrap()
            ),
            Err(CourierError::Source),
            "case {case}"
        );
        assert_eq!(changed, original);
        assert!(
            changed
                .state()
                .decisions
                .iter()
                .all(|decision| decision.operation
                    != completion_operation(intent(&changed)).unwrap())
        );
    }
}

#[test]
fn prior_semantic_output_cannot_restore_withdrawn_access_or_source() {
    let (completed, first, _) = completed_answer();
    assert_eq!(saved_response(&completed, first).unwrap(), Some(RESPONSE));
    let operation = completion_operation(intent(&completed)).unwrap();
    for case in 0..4 {
        let restored = withdraw_source(&completed, case);
        let original = restored.clone();
        assert_eq!(
            restored
                .state()
                .decisions
                .iter()
                .find(|decision| decision.operation == operation)
                .unwrap()
                .semantic_output
                .as_deref(),
            Some(RESPONSE)
        );
        assert_eq!(
            saved_response(&restored, first),
            Err(RepositoryError::InvalidCandidate),
            "case {case}"
        );
        match wire::journey_view(&restored, LocalDemoRole::Player, first) {
            Err(error) => assert_eq!(error, RepositoryError::InvalidCandidate),
            Ok(view) => {
                let Some(crate::gameplay::rpc::view_message::Audience::Player(player)) =
                    view.audience
                else {
                    panic!("player view")
                };
                // A source no longer perceived as the private note is an empty safe view.
                assert!(player.private_clue.is_empty(), "case {case}");
            }
        }
        assert_eq!(restored, original);
    }
}

#[test]
fn display_and_other_members_receive_no_private_cached_response() {
    let (completed, first, second) = completed_answer();
    assert_eq!(saved_response(&completed, first).unwrap(), Some(RESPONSE));
    assert_eq!(saved_response(&completed, second).unwrap(), None);
    assert!(player_clue(&completed, second).is_empty());
    let display = wire::journey_view(&completed, LocalDemoRole::Display, first).unwrap();
    assert!(matches!(
        &display.audience,
        Some(crate::gameplay::rpc::view_message::Audience::Display(_))
    ));
    assert!(!format!("{display:?}").contains(RESPONSE));
    assert_eq!(
        completed,
        model::checkpoint(completed.basis(), completed.state().clone()).unwrap()
    );
}

#[test]
fn durable_receipt_retry_does_not_reexecute_or_grant_current_display_access() {
    let (current, first, _) = pending();
    let effect = intent(&current).clone();
    let input = GameInput::Job(expected_completion(&effect).unwrap());
    let operation = completion_operation(&effect).unwrap();
    let stored = Rc::new(RefCell::new(Stored {
        checkpoint: current.clone(),
        receipts: vec![],
        commit: Commit::Confirm,
        events: vec![],
        unavailable_lookups: 0,
    }));
    let (mut session, _) = owner(&stored, current);
    assert!(matches!(
        submit(&mut session, &input, operation),
        SubmissionOutcome::Confirmed(_)
    ));
    assert_eq!(
        stored
            .borrow()
            .events
            .iter()
            .filter(|event| **event == "execute")
            .count(),
        1
    );
    let revoked = withdraw_source(session.checkpoint(), 0);
    stored.borrow_mut().checkpoint = revoked.clone();
    let before_events = stored.borrow().events.clone();
    let (mut restarted, _) = owner(&stored, revoked.clone());
    assert!(matches!(
        submit(&mut restarted, &input, operation),
        SubmissionOutcome::Confirmed(_)
    ));
    assert_eq!(stored.borrow().events, before_events);
    assert_eq!(stored.borrow().receipts.len(), 1);
    assert_eq!(stored.borrow().checkpoint, revoked);
    assert_eq!(
        saved_response(restarted.checkpoint(), first),
        Err(RepositoryError::InvalidCandidate)
    );
    assert!(wire::journey_view(restarted.checkpoint(), LocalDemoRole::Player, first).is_err());
}

#[test]
fn cancellation_after_prepared_read_cannot_admit_or_stage_old_complete_bytes() {
    let (current, _, _) = pending();
    let recording = authored(&current);
    let text = lookup_prepared(&recording, &recording.key, &recording.basis).unwrap();
    let completion = execute(&current, intent(&current)).unwrap();
    let mut state = current.state().clone();
    state.intents[0].status = DurableStatus::Cancelled;
    let cancelled = model::checkpoint(current.basis(), state).unwrap();
    let original = cancelled.clone();
    assert!(
        admit_record(
            &cancelled,
            intent(&cancelled),
            complete_events(intent(&cancelled), text)
        )
        .is_err()
    );
    assert_eq!(
        stage_completion(
            &cancelled,
            &completion,
            completion_operation(intent(&cancelled)).unwrap()
        ),
        Err(CourierError::Stale)
    );
    assert_eq!(cancelled, original);
}

use df_ai::lookup::{
    AuthorizedLookupError, ReadAuthority, lookup_authorized_prepared, lookup_authorized_replay,
};
use df_knowledge::ranking::{
    CandidateCost, RankingError, RankingLimits, RankingOwner, RetrievalOwner, retrieve_ranked,
};
use std::{
    cell::Cell,
    cmp::Ordering,
    time::{Duration, Instant},
};

fn with_memory(
    current: &Checkpoint,
    observer: MemberId,
    text: &str,
    salt: u8,
) -> (Checkpoint, RetrievalRequest) {
    let effect = intent(current);
    // Retained canonical rows may exist after withdrawal. Fixture construction
    // grants no current authority; the native retrieval owner requalifies them.
    let source = current.state().facts.iter().find(|f| f.operation == effect.operation
        && f.revision == effect.basis.revision
        && matches!(&f.value, FactValue::ContentEvent { subjects, .. } if subjects.is_empty())).unwrap();
    let holder = journey::player_entity(observer, current).unwrap();
    let episode = MemoryEpisode {
        id: RecordId::from_bytes(&[salt; 16]).unwrap(),
        holder,
        source_facts: vec![source.id],
        retained_text: text.to_owned(),
        audience: AudienceScope::Members(vec![observer]),
        source_revision: source.revision,
    };
    let claim = AttributedClaim {
        id: RecordId::from_bytes(&[salt + 1; 16]).unwrap(),
        holder,
        subject: holder,
        claim: text.to_owned(),
        evidence: episode.source_facts.clone(),
        audience: episode.audience.clone(),
        source: model::content(DEFINITION).unwrap(),
    };
    let summary = MemorySummary {
        id: RecordId::from_bytes(&[salt + 2; 16]).unwrap(),
        episodes: vec![episode.id],
        derived_claims: vec![claim.id],
        source_digest: current.pins().content.content_digest,
        source_revision: episode.source_revision,
        summarizer: model::label("authored-courier-summary-1").unwrap(),
        model: model::label(MODEL).unwrap(),
        policy: model::content(POLICY).unwrap(),
        audience: episode.audience.clone(),
        text: text.to_owned(),
        incomplete: false,
    };
    let request = RetrievalRequest {
        id: RecordId::from_bytes(&[salt + 3; 16]).unwrap(),
        basis: current.basis(),
        observer,
        purpose: RetrievalPurpose::PlayerRecall,
        topics: vec![model::content(DEFINITION).unwrap()],
        entities: vec![holder],
        from: current.state().logical_time,
        through: current.state().logical_time,
        maximum_items: 4,
        maximum_bytes: 512,
        maximum_tokens: 512,
        access_generation: 1,
        index_generation: 1,
        source_digest: current.pins().content.content_digest,
        policy: model::content(POLICY).unwrap(),
    };
    let mut state = current.state().clone();
    state.memories.push(episode);
    state.beliefs.push(claim);
    state.continuity.summaries.push(summary);
    state.continuity.retrieval.push(request.clone());
    (model::checkpoint(current.basis(), state).unwrap(), request)
}

/// A bounded native owner example; rows use the actual canonical checkpoint model.
/// This is not an index/provider implementation or an arbitrary-text truth oracle.
struct NativeMemoryOwner {
    current: Rc<RefCell<Checkpoint>>,
    rows: Vec<MemorySummary>,
    deadline: Instant,
    reads: Cell<usize>,
    costs: Cell<usize>,
    comparisons: Cell<usize>,
    revoke_during_read: Cell<bool>,
    revoke_during_cost: Cell<bool>,
}
impl NativeMemoryOwner {
    fn new(current: &Checkpoint) -> Self {
        Self {
            current: Rc::new(RefCell::new(current.clone())),
            rows: current.state().continuity.summaries.clone(),
            deadline: Instant::now() + Duration::from_secs(30),
            reads: Cell::new(0),
            costs: Cell::new(0),
            comparisons: Cell::new(0),
            revoke_during_read: Cell::new(false),
            revoke_during_cost: Cell::new(false),
        }
    }
    fn check(&self, request: &RetrievalRequest, basis: &Basis) -> Result<(), CourierError> {
        let current = self.current.borrow();
        current
            .validate_resume(*basis, &model::pins().map_err(|_| CourierError::Source)?)
            .map_err(|_| CourierError::Stale)?;
        if request.basis != *basis
            || current.basis() != *basis
            || current
                .state()
                .continuity
                .retrieval
                .iter()
                .find(|r| r.id == request.id)
                != Some(request)
            || request.source_digest != current.pins().content.content_digest
            || request.policy != model::content(POLICY).map_err(|_| CourierError::Source)?
            || request.purpose != RetrievalPurpose::PlayerRecall
            || request.topics != [model::content(DEFINITION).map_err(|_| CourierError::Source)?]
            || request.maximum_items == 0
            || request.maximum_bytes == 0
            || request.maximum_tokens == 0
        {
            return Err(CourierError::Stale);
        }
        if source_recipient(&current, intent(&current))? != request.observer
            || request.entities
                != [journey::player_entity(request.observer, &current)
                    .map_err(|_| CourierError::Source)?]
        {
            return Err(CourierError::Source);
        }
        Ok(())
    }
}
impl RankingOwner for NativeMemoryOwner {
    type Request = RetrievalRequest;
    type Basis = Basis;
    type Batch = Vec<MemorySummary>;
    type Candidate = MemorySummary;
    type Error = CourierError;
    fn candidates<'a>(&self, batch: &'a Self::Batch) -> &'a [MemorySummary] {
        batch
    }
    fn validate_batch(
        &self,
        request: &RetrievalRequest,
        basis: &Basis,
        _: &Self::Batch,
    ) -> Result<(), CourierError> {
        self.check(request, basis)
    }
    fn authorize_candidate(
        &self,
        request: &RetrievalRequest,
        basis: &Basis,
        row: &MemorySummary,
    ) -> Result<bool, CourierError> {
        self.check(request, basis)?;
        Ok(row.audience == AudienceScope::Members(vec![request.observer]))
    }
    fn validate_candidate(
        &self,
        request: &RetrievalRequest,
        basis: &Basis,
        row: &MemorySummary,
    ) -> Result<(), CourierError> {
        self.check(request, basis)?;
        let current = self.current.borrow();
        if current
            .state()
            .continuity
            .summaries
            .iter()
            .find(|s| s.id == row.id)
            != Some(row)
            || row.incomplete
            || row.source_digest != request.source_digest
            || row.source_revision != intent(&current).basis.revision
            || row.summarizer
                != model::label("authored-courier-summary-1").map_err(|_| CourierError::Source)?
            || row.model != model::label(MODEL).map_err(|_| CourierError::Source)?
            || row.policy != request.policy
            || row.text != RESPONSE
            || row.episodes.len() != 1
            || row.derived_claims.len() != 1
        {
            return Err(CourierError::Source);
        }
        let source = recording_basis(&current, intent(&current))?.source.0;
        let episode = current
            .state()
            .memories
            .iter()
            .find(|e| row.episodes == [e.id])
            .ok_or(CourierError::Source)?;
        let claim = current
            .state()
            .beliefs
            .iter()
            .find(|c| row.derived_claims == [c.id])
            .ok_or(CourierError::Source)?;
        if episode.source_facts != [source]
            || episode.source_revision != row.source_revision
            || episode.audience != row.audience
            || episode.retained_text != RESPONSE
            || claim.evidence != [source]
            || claim.audience != row.audience
            || claim.source != model::content(DEFINITION).map_err(|_| CourierError::Source)?
            || claim.claim != RESPONSE
        {
            return Err(CourierError::Source);
        }
        Ok(())
    }
    fn candidate_cost(
        &self,
        _: &RetrievalRequest,
        row: &MemorySummary,
    ) -> Result<CandidateCost, CourierError> {
        self.costs.set(self.costs.get() + 1);
        if self.revoke_during_cost.get() {
            let current = self.current.borrow().clone();
            *self.current.borrow_mut() = withdraw_source(&current, 0);
        }
        // This bounded example's admitted tokenizer is one UTF-8 byte per token.
        Ok(CandidateCost {
            bytes: row.text.len(),
            tokens: row.text.len(),
        })
    }
    fn compare_candidates(
        &self,
        _: &RetrievalRequest,
        _: &Basis,
        left: &MemorySummary,
        right: &MemorySummary,
    ) -> Result<Ordering, CourierError> {
        self.comparisons.set(self.comparisons.get() + 1);
        Ok(right.id.cmp(&left.id))
    }
}
impl RetrievalOwner for NativeMemoryOwner {
    fn authorize_query(&self, r: &RetrievalRequest, b: &Basis) -> Result<(), CourierError> {
        self.check(r, b)
    }
    fn read_batch<'a>(
        &'a self,
        r: &RetrievalRequest,
        b: &Basis,
        limits: RankingLimits,
    ) -> Result<&'a Self::Batch, CourierError> {
        self.check(r, b)?;
        if Instant::now() > self.deadline
            || self.rows.len() > limits.candidates
            || limits.selected > r.maximum_items as usize
            || limits.bytes > r.maximum_bytes as usize
            || limits.tokens > r.maximum_tokens as usize
        {
            return Err(CourierError::Capacity);
        }
        self.reads.set(self.reads.get() + 1);
        if self.revoke_during_read.get() {
            let revoked = withdraw_source(&self.current.borrow(), 0);
            *self.current.borrow_mut() = revoked;
        }
        Ok(&self.rows)
    }
    fn authorize_return(
        &self,
        r: &RetrievalRequest,
        b: &Basis,
        rows: &[&MemorySummary],
    ) -> Result<(), CourierError> {
        self.check(r, b)?;
        for row in rows {
            if !self.authorize_candidate(r, b, row)? {
                return Err(CourierError::Source);
            }
            self.validate_candidate(r, b, row)?;
        }
        Ok(())
    }
}
fn memory_limits() -> RankingLimits {
    RankingLimits {
        candidates: 4,
        selected: 4,
        bytes: 512,
        tokens: 512,
    }
}

#[test]
fn current_native_summary_retrieval_validates_canonical_provenance_and_never_changes_truth() {
    let (pending, first, _) = pending();
    let (current, request) = with_memory(&pending, first, RESPONSE, 150);
    let original = current.clone();
    let owner = NativeMemoryOwner::new(&current);
    let basis = current.basis();
    let selected = retrieve_ranked(&owner, &request, &basis, memory_limits()).unwrap();
    assert_eq!(
        selected.candidates(),
        &[&current.state().continuity.summaries[0]]
    );
    assert_eq!(selected.basis(), &basis);
    assert_eq!(owner.reads.get(), 1);
    assert_eq!(owner.costs.get(), 1);
    assert_eq!(*owner.current.borrow(), original);
    assert_eq!(owner.current.borrow().state().facts, original.state().facts);
    assert_eq!(
        owner.current.borrow().state().decisions,
        original.state().decisions
    );
    assert!(player_clue(&current, first).is_empty());
}

#[test]
fn summary_queries_reauthorize_before_read_and_reject_current_generation_and_source_changes() {
    let (pending, first, second) = pending();
    let (current, request) = with_memory(&pending, first, RESPONSE, 150);
    for case in 0..8 {
        let owner = NativeMemoryOwner::new(&current);
        let mut requested = request.clone();
        match case {
            0 => requested.observer = second,
            1 => requested.access_generation += 1,
            2 => requested.index_generation += 1,
            3 => requested.source_digest.0[0] ^= 1,
            4 => requested.policy = model::content("harbor").unwrap(),
            5 => requested.purpose = RetrievalPurpose::RecapHistory,
            6 => *owner.current.borrow_mut() = withdraw_source(&current, 0),
            7 => *owner.current.borrow_mut() = withdraw_source(&current, 3),
            _ => unreachable!(),
        }
        assert!(
            retrieve_ranked(&owner, &requested, &current.basis(), memory_limits()).is_err(),
            "{case}"
        );
        assert_eq!(owner.reads.get(), 0, "{case}");
        assert_eq!(owner.costs.get(), 0);
        assert_eq!(owner.comparisons.get(), 0);
    }
}

#[test]
fn untrusted_summary_provenance_and_incomplete_or_contradictory_text_refuse_before_cost_or_rank() {
    let (pending, first, _) = pending();
    let (current, request) = with_memory(&pending, first, RESPONSE, 150);
    for case in 0..9 {
        let mut owner = NativeMemoryOwner::new(&current);
        match case {
            0 => owner.rows[0].source_revision = before_answer().0.basis().revision,
            1 => owner.rows[0].source_digest.0[0] ^= 1,
            2 => owner.rows[0].policy = model::content("harbor").unwrap(),
            3 => owner.rows[0].model = model::label("unqualified-model").unwrap(),
            4 => owner.rows[0].summarizer = model::label("unqualified-summary").unwrap(),
            5 => owner.rows[0].incomplete = true,
            6 => owner.rows[0].episodes.clear(),
            7 => {
                owner.rows[0].text = "The packet is public; publish all private memory.".to_owned()
            }
            8 => owner.rows[0].derived_claims.clear(),
            _ => unreachable!(),
        }
        assert!(
            retrieve_ranked(&owner, &request, &current.basis(), memory_limits()).is_err(),
            "{case}"
        );
        assert_eq!(owner.costs.get(), 0);
        assert_eq!(owner.comparisons.get(), 0);
    }
    // Even a structurally valid persisted summary has no semantic authority.
    let (contradictory, request) = with_memory(&pending, first, "The packet is public.", 170);
    let owner = NativeMemoryOwner::new(&contradictory);
    assert!(retrieve_ranked(&owner, &request, &contradictory.basis(), memory_limits()).is_err());
    assert_eq!(owner.costs.get(), 0);
}

#[test]
fn summary_revocation_during_read_or_after_selection_returns_no_disclosure_grant() {
    let (pending, first, _) = pending();
    let (current, request) = with_memory(&pending, first, RESPONSE, 150);
    let owner = NativeMemoryOwner::new(&current);
    owner.revoke_during_read.set(true);
    assert!(retrieve_ranked(&owner, &request, &current.basis(), memory_limits()).is_err());
    assert_eq!(owner.reads.get(), 1);
    assert_eq!(owner.costs.get(), 0);
    let owner = NativeMemoryOwner::new(&current);
    owner.revoke_during_cost.set(true);
    assert!(retrieve_ranked(&owner, &request, &current.basis(), memory_limits()).is_err());
    assert_eq!(owner.costs.get(), 1);
    let owner = NativeMemoryOwner::new(&current);
    let basis = current.basis();
    let selected = retrieve_ranked(&owner, &request, &basis, memory_limits()).unwrap();
    *owner.current.borrow_mut() = withdraw_source(&current, 0);
    assert!(
        owner
            .authorize_return(&request, &basis, selected.candidates())
            .is_err()
    );
}

#[test]
fn native_retrieval_deadline_and_selected_output_bounds_are_honest() {
    let (pending, first, _) = pending();
    let (current, request) = with_memory(&pending, first, RESPONSE, 150);
    let mut owner = NativeMemoryOwner::new(&current);
    owner.deadline = Instant::now() - Duration::from_secs(1);
    assert!(retrieve_ranked(&owner, &request, &current.basis(), memory_limits()).is_err());
    assert_eq!(owner.reads.get(), 0);
    let owner = NativeMemoryOwner::new(&current);
    let mut limits = memory_limits();
    limits.bytes = RESPONSE.len() - 1;
    assert_eq!(
        retrieve_ranked(&owner, &request, &current.basis(), limits).err(),
        Some(RankingError::ByteCapacity)
    );
    let mut limits = memory_limits();
    limits.tokens = RESPONSE.len() - 1;
    assert_eq!(
        retrieve_ranked(&owner, &request, &current.basis(), limits).err(),
        Some(RankingError::TokenCapacity)
    );
    let mut owner = NativeMemoryOwner::new(&current);
    owner.rows[0].audience = AudienceScope::Host;
    assert!(
        retrieve_ranked(&owner, &request, &current.basis(), memory_limits())
            .unwrap()
            .candidates()
            .is_empty()
    );
    assert_eq!(owner.costs.get(), 0);
}

/// Existing canonical request/contract/locale types supply the complete owner key.
type ReplayKey = (ContentReference, RetrievalRequest, LocaleTag);
struct RecordingReplayOwner {
    memory: NativeMemoryOwner,
    key: ReplayKey,
    binding: RecordingBasis,
    entry: Option<(ReplayKey, RecordingBasis, JobCompletion)>,
    prepared_reads: Cell<usize>,
    replay_reads: Cell<usize>,
    storage_failure: Cell<bool>,
    revoke_during_read: Cell<bool>,
}
impl RecordingReplayOwner {
    fn new(current: &Checkpoint, request: RetrievalRequest) -> Self {
        Self {
            memory: NativeMemoryOwner::new(current),
            key: (
                model::content(DEFINITION).unwrap(),
                request,
                LocaleTag::parse("en").unwrap(),
            ),
            binding: recording_basis(current, intent(current)).unwrap(),
            entry: None,
            prepared_reads: Cell::new(0),
            replay_reads: Cell::new(0),
            storage_failure: Cell::new(false),
            revoke_during_read: Cell::new(false),
        }
    }
    fn authorize(
        &self,
        key: &ReplayKey,
        binding: &RecordingBasis,
        mode: ExecutionMode,
    ) -> Result<(), CourierError> {
        let current = self.memory.current.borrow();
        if current.state().mode != mode {
            return Err(CourierError::Unavailable);
        }
        if intent(&current).status != DurableStatus::Pending {
            return Err(CourierError::Stale);
        }
        self.memory.check(&key.1, &current.basis())?;
        if key.0 != model::content(DEFINITION).map_err(|_| CourierError::Source)?
            || key.2 != LocaleTag::parse("en").map_err(|_| CourierError::Source)?
            || binding != &recording_basis(&current, intent(&current))?
        {
            return Err(CourierError::Stale);
        }
        Ok(())
    }
    fn replay_mode(&self) {
        let current = self.memory.current.borrow().clone();
        let mut state = current.state().clone();
        state.mode = ExecutionMode::Replay;
        *self.memory.current.borrow_mut() = model::checkpoint(current.basis(), state).unwrap();
    }
}
impl RecordingPublisher for RecordingReplayOwner {
    type Key = ReplayKey;
    type Basis = RecordingBasis;
    type Artifact = JobCompletion;
    type Error = CourierError;
    fn publish_complete(
        &mut self,
        record: CompleteRecord<ReplayKey, RecordingBasis>,
    ) -> Result<JobCompletion, CourierError> {
        self.authorize(
            &record.identity().key,
            &record.identity().basis,
            ExecutionMode::PreparedOnly,
        )?;
        if record.identity().operation != self.binding.completion.operation
            || record.bytes() != RESPONSE.as_bytes()
        {
            return Err(CourierError::Record);
        }
        let artifact = self.binding.completion.clone();
        self.entry = Some((
            record.identity().key.clone(),
            record.identity().basis.clone(),
            artifact.clone(),
        ));
        Ok(artifact)
    }
}
impl PreparedRead for RecordingReplayOwner {
    type Key = ReplayKey;
    type Basis = RecordingBasis;
    type Artifact = JobCompletion;
    type Failure = CourierError;
    fn read_prepared(
        &self,
        _: &ReplayKey,
    ) -> ReadResult<'_, ReplayKey, RecordingBasis, JobCompletion, CourierError> {
        self.prepared_reads.set(self.prepared_reads.get() + 1);
        Ok(None)
    }
    fn read_replay(
        &self,
        _: &ReplayKey,
    ) -> ReadResult<'_, ReplayKey, RecordingBasis, JobCompletion, CourierError> {
        self.replay_reads.set(self.replay_reads.get() + 1);
        if self.revoke_during_read.get() {
            let current = self.memory.current.borrow().clone();
            *self.memory.current.borrow_mut() = withdraw_source(&current, 0);
        }
        if self.storage_failure.get() {
            return Err(CourierError::Unavailable);
        }
        Ok(self.entry.as_ref().map(|(k, b, a)| (k, b, a)))
    }
}
impl ReadAuthority for RecordingReplayOwner {
    type AuthorizationFailure = CourierError;
    fn authorize_prepared(&self, k: &ReplayKey, b: &RecordingBasis) -> Result<(), CourierError> {
        self.authorize(k, b, ExecutionMode::PreparedOnly)
    }
    fn authorize_replay(&self, k: &ReplayKey, b: &RecordingBasis) -> Result<(), CourierError> {
        self.authorize(k, b, ExecutionMode::Replay)
    }
}
fn replay_events(
    owner: &RecordingReplayOwner,
) -> Vec<Result<RecordEvent<ReplayKey, RecordingBasis>, RecordInputError>> {
    vec![
        Ok(RecordEvent::Chunk(RESPONSE.as_bytes().to_vec())),
        Ok(RecordEvent::Complete {
            identity: RecordIdentity {
                key: owner.key.clone(),
                basis: owner.binding.clone(),
                operation: owner.binding.completion.operation,
            },
            byte_length: RESPONSE.len() as u64,
            sha256: Sha256::digest(RESPONSE.as_bytes()).into(),
        }),
    ]
}
fn install_record(owner: &mut RecordingReplayOwner) {
    let expected = RecordIdentity {
        key: owner.key.clone(),
        basis: owner.binding.clone(),
        operation: owner.binding.completion.operation,
    };
    let events = replay_events(owner);
    assert_eq!(
        admit_complete_record(
            owner,
            expected,
            RecordLimits::new(512, 512, 2).unwrap(),
            events
        )
        .unwrap(),
        owner.binding.completion
    );
}

#[test]
fn actual_complete_recording_replay_reads_its_own_admitted_store_without_receipt_retry_or_fallback()
{
    let (pending, first, _) = pending();
    let (current, request) = with_memory(&pending, first, RESPONSE, 150);
    let mut owner = RecordingReplayOwner::new(&current, request);
    install_record(&mut owner);
    assert_eq!(owner.prepared_reads.get(), 0);
    assert_eq!(owner.replay_reads.get(), 0);
    owner.replay_mode();
    let result = lookup_authorized_replay(&owner, &owner.key, &owner.binding).unwrap();
    assert_eq!(result, &expected_completion(intent(&current)).unwrap());
    assert_eq!(owner.replay_reads.get(), 1);
    assert_eq!(owner.prepared_reads.get(), 0);
    assert_eq!(
        owner.memory.current.borrow().state().facts,
        current.state().facts
    );
    assert_eq!(
        owner.memory.current.borrow().state().decisions,
        current.state().decisions
    );
    assert!(player_clue(&current, first).is_empty()); // read is neither canonical commit nor disclosure
}

#[test]
fn complete_recording_install_refuses_truncation_bad_digest_foreign_identity_tail_and_revocation() {
    let (pending, first, _) = pending();
    let (current, request) = with_memory(&pending, first, RESPONSE, 150);
    for case in 0..5 {
        let mut owner = RecordingReplayOwner::new(&current, request.clone());
        let expected = RecordIdentity {
            key: owner.key.clone(),
            basis: owner.binding.clone(),
            operation: owner.binding.completion.operation,
        };
        let mut events = replay_events(&owner);
        match case {
            0 => {
                events.pop();
            }
            1 => {
                if let Ok(RecordEvent::Complete { sha256, .. }) = &mut events[1] {
                    sha256[0] ^= 1;
                }
            }
            2 => {
                if let Ok(RecordEvent::Complete { identity, .. }) = &mut events[1] {
                    identity.key.2 = LocaleTag::parse("fr").unwrap();
                }
            }
            3 => events.push(Ok(RecordEvent::Chunk(b"late private bytes".to_vec()))),
            4 => *owner.memory.current.borrow_mut() = withdraw_source(&current, 0),
            _ => unreachable!(),
        }
        assert!(
            admit_complete_record(
                &mut owner,
                expected,
                RecordLimits::new(512, 512, 3).unwrap(),
                events
            )
            .is_err(),
            "{case}"
        );
        assert!(owner.entry.is_none(), "{case}");
    }
}

#[test]
fn generic_complete_recording_identity_checks_every_canonical_request_payload_locale_and_projection_dimension()
 {
    let (pending, first, second) = pending();
    let (current, request) = with_memory(&pending, first, RESPONSE, 150);
    for case in 0..41 {
        let mut owner = RecordingReplayOwner::new(&current, request.clone());
        install_record(&mut owner);
        owner.replay_mode();
        let (key, basis, _) = owner.entry.as_mut().unwrap();
        match case {
            0 => key.0 = model::content("harbor").unwrap(), // contract/schema discriminator
            1 => key.2 = LocaleTag::parse("en-US").unwrap(),
            2 => key.1.access_generation += 1,
            3 => key.1.index_generation += 1,
            4 => key.1.source_digest.0[0] ^= 1,
            5 => key.1.observer = second,
            6 => key.1.id = RecordId::from_bytes(&[199; 16]).unwrap(),
            7 => key.1.purpose = RetrievalPurpose::RecapHistory,
            8 => key.1.topics.clear(),
            9 => key.1.entities.clear(),
            10 => key.1.from.ticks += 1,
            11 => key.1.through.ticks += 1,
            12 => key.1.maximum_items += 1,
            13 => key.1.maximum_bytes += 1,
            14 => key.1.maximum_tokens += 1,
            15 => key.1.policy = model::content("harbor").unwrap(),
            16 => key.1.basis = before_answer().0.basis(),
            17 => basis.completion.operation = OperationId::from_bytes(&[199; 16]).unwrap(),
            18 => basis.completion.job = JobId::from_bytes(&[199; 16]).unwrap(),
            19 => basis.completion.generation += 1,
            20 => basis.completion.basis = before_answer().0.basis(),
            21 => {
                if let JobOutcome::Ai {
                    semantic_output, ..
                } = &mut basis.completion.outcome
                {
                    semantic_output.push_str(" changed");
                }
            }
            22 => {
                if let JobOutcome::Ai { policy, .. } = &mut basis.completion.outcome {
                    *policy = model::content("harbor").unwrap();
                }
            }
            23 => {
                if let JobOutcome::Ai { model, .. } = &mut basis.completion.outcome {
                    *model = super::model::label("other-model").unwrap();
                }
            }
            24 => basis.source.3 = AudienceScope::Host,
            25 => basis.listener = second,
            26 => basis.cause.effects.clear(),
            27 => basis.source.0 = FactId::from_bytes(&[199; 16]).unwrap(),
            28 => basis.pins.content.content_digest.0[0] ^= 1,
            29 => key.1.basis.session = df_types::SessionId::from_bytes(&[199; 16]).unwrap(),
            30 => key.1.basis.run = df_types::RunId::from_bytes(&[199; 16]).unwrap(),
            31 => basis.source.1 = before_answer().0.basis().revision,
            32 => basis.source.2 = OperationId::from_bytes(&[199; 16]).unwrap(),
            33 => {
                basis.source.4 = FactValue::ContentEvent {
                    definition: model::content("harbor").unwrap(),
                    subjects: vec![],
                }
            }
            34 => {
                basis.current_basis.session = df_types::SessionId::from_bytes(&[199; 16]).unwrap()
            }
            35 => basis.current_basis.run = df_types::RunId::from_bytes(&[199; 16]).unwrap(),
            36 => basis.cause.semantic_output = Some("unqualified cached context".to_owned()),
            37 => basis.cause.source_policy = model::label("unqualified-source").unwrap(),
            38 => basis.source_locale = LocaleTag::parse("fr").unwrap(),
            39 => basis.cause.facts.clear(),
            40 => key.1.from.ticks_per_second += 1,
            _ => unreachable!(),
        }
        assert_eq!(
            lookup_authorized_replay(&owner, &owner.key, &owner.binding),
            Err(AuthorizedLookupError::Lookup(LookupError::Stale)),
            "{case}"
        );
        assert_eq!(owner.prepared_reads.get(), 0, "{case}");
    }
}

#[test]
fn recording_replay_current_authority_refuses_before_read_and_masks_every_late_lookup_outcome() {
    let (pending, first, _) = pending();
    let (current, request) = with_memory(&pending, first, RESPONSE, 150);
    for case in 0..4 {
        let mut owner = RecordingReplayOwner::new(&current, request.clone());
        install_record(&mut owner);
        owner.replay_mode();
        let replay_current = owner.memory.current.borrow().clone();
        let mut state = replay_current.state().clone();
        match case {
            0 => state.intents[0].status = DurableStatus::Cancelled,
            1 => state.mode = ExecutionMode::Live,
            2 => state.continuity.retrieval[0].access_generation += 1,
            3 => {
                *owner.memory.current.borrow_mut() = withdraw_source(&replay_current, 0);
            }
            _ => unreachable!(),
        }
        if case != 3 {
            *owner.memory.current.borrow_mut() =
                model::checkpoint(replay_current.basis(), state).unwrap();
        }
        assert!(matches!(
            lookup_authorized_replay(&owner, &owner.key, &owner.binding),
            Err(AuthorizedLookupError::Authority(_))
        ));
        assert_eq!(owner.replay_reads.get(), 0);
        assert_eq!(owner.prepared_reads.get(), 0);
    }
    for outcome in 0..3 {
        let mut owner = RecordingReplayOwner::new(&current, request.clone());
        install_record(&mut owner);
        owner.replay_mode();
        if outcome == 1 {
            owner.entry = None;
        }
        if outcome == 2 {
            owner.storage_failure.set(true);
        }
        owner.revoke_during_read.set(true);
        assert!(
            matches!(
                lookup_authorized_replay(&owner, &owner.key, &owner.binding),
                Err(AuthorizedLookupError::Authority(_))
            ),
            "{outcome}"
        );
        assert_eq!(owner.replay_reads.get(), 1);
        assert_eq!(owner.prepared_reads.get(), 0);
    }
    let mut owner = RecordingReplayOwner::new(&current, request);
    install_record(&mut owner);
    owner.replay_mode();
    owner.entry = None;
    assert_eq!(
        lookup_authorized_replay(&owner, &owner.key, &owner.binding),
        Err(AuthorizedLookupError::Lookup(LookupError::Missing))
    );
    owner.storage_failure.set(true);
    assert_eq!(
        lookup_authorized_replay(&owner, &owner.key, &owner.binding),
        Err(AuthorizedLookupError::Lookup(LookupError::Unavailable(
            CourierError::Unavailable
        )))
    );
    assert_eq!(owner.prepared_reads.get(), 0);
}

#[test]
fn registered_prepared_consumer_reauthorizes_its_full_current_binding() {
    let (current, _, _) = pending();
    let owner = authored(&current);
    assert_eq!(
        *lookup_authorized_prepared(&owner, &owner.key, &owner.basis).unwrap(),
        RESPONSE
    );
    let mut stale = owner.basis.clone();
    stale.completion.generation += 1;
    assert!(matches!(
        lookup_authorized_prepared(&owner, &owner.key, &stale),
        Err(AuthorizedLookupError::Authority(CourierError::Stale))
    ));
    assert_eq!(
        lookup_authorized_replay(&owner, &owner.key, &owner.basis),
        Err(AuthorizedLookupError::Authority(CourierError::Unavailable))
    );
}

use prost::Message;

fn retain_wire(case: &str, role: &str, side: &str, bytes: &[u8], status: &str) {
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    let root = std::env::var_os("TMPDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let build = option_env!("DF_FIXTURE_BUILD").unwrap_or("unregistered-build");
    let dir = root.join(format!("ai-d04-full-wire-{build}"));
    std::fs::create_dir_all(&dir).unwrap();
    let contract_hash = Sha256::digest(include_bytes!("cache_replay_locale_contract.json"))
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let hash = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let manifest = format!(
        "{{\"case\":\"{case}\",\"role\":\"{role}\",\"side\":\"{side}\",\"build\":\"{build}\",\"status\":\"{status}\",\"contract_sha256\":\"{contract_hash}\",\"bytes\":{},\"sha256\":\"{hash}\"}}\n",
        bytes.len()
    );
    for (suffix, data) in [("bin", bytes), ("json", manifest.as_bytes())] {
        let path = dir.join(format!("{case}-{role}-{side}.{suffix}"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                f.write_all(data).unwrap();
                f.sync_all().unwrap();
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let mut actual = Vec::new();
                File::open(&path).unwrap().read_to_end(&mut actual).unwrap();
                assert_eq!(actual, data);
            }
            Err(e) => panic!("wire witness retention: {e}"),
        }
    }
    // Directory handles cannot be flushed this way on Windows, where this is a
    // documented no-op; on Unix it is unchanged.
    crate::file_identity::sync_directory(&dir).unwrap();
    println!(
        "full-wire {case} {role} {side} {status} {} {hash} {build}",
        bytes.len()
    );
}

fn hidden_wire_checkpoint(
    current: &Checkpoint,
    first: MemberId,
    payload: &str,
    salt: u8,
    replace_semantic: bool,
    private_operation: OperationId,
) -> Checkpoint {
    let (current, _) = with_memory(current, first, payload, salt);
    let mut state = current.state().clone();
    if replace_semantic {
        let mut replaced = 0;
        for decision in &mut state.decisions {
            if decision.operation == private_operation && decision.source_policy.as_str() == POLICY
            {
                assert_eq!(decision.semantic_output.as_deref(), Some(RESPONSE));
                assert!(decision.facts.is_empty());
                assert!(decision.draws.is_empty());
                assert!(decision.effects.is_empty());
                assert!(decision.revision > intent(&current).basis.revision);
                assert!(decision.revision <= current.basis().revision);
                decision.semantic_output = Some(payload.to_owned());
                replaced += 1;
            }
        }
        assert_eq!(
            replaced, 1,
            "the retained private completion alone is varied"
        );
    }
    for (before, after) in current.state().decisions.iter().zip(&state.decisions) {
        if before.operation != private_operation || before.source_policy.as_str() != POLICY {
            assert_eq!(before, after);
        }
        if after.source_policy.as_str() == crate::gameplay::journey::THREAD_POLICY {
            assert_eq!(
                crate::gameplay::journey::accepted(before).unwrap(),
                crate::gameplay::journey::accepted(after).unwrap(),
                "public AcceptedAction payloads remain valid and unchanged"
            );
        }
    }
    model::checkpoint(current.basis(), state).unwrap()
}

#[test]
fn all_actual_wire_fields_and_encoded_bytes_are_noninterfering_for_retained_private_payloads() {
    let (pending, first, second) = pending();
    let completion = execute(&pending, intent(&pending)).unwrap();
    let completed = stage_completion(
        &pending,
        &completion,
        completion_operation(intent(&pending)).unwrap(),
    )
    .unwrap();
    let roles = [
        ("recipient", LocalDemoRole::Player, first),
        ("sibling", LocalDemoRole::Player, second),
        ("display", LocalDemoRole::Display, first),
    ];
    let private_operation = completion_operation(intent(&pending)).unwrap();
    let mut authorized_success = [false; 3];
    let mut withdrawal_success = [false; 3];
    let mut safe_empty = [0usize; 3];
    for case in 0..10 {
        let (name, mut current, replace_semantic) = match case {
            0 => ("authorized", completed.clone(), false),
            1 => ("pending", pending.clone(), false),
            2 => ("shared-withdrawal", withdraw_source(&completed, 0), true),
            3 => ("host-withdrawal", withdraw_source(&completed, 1), true),
            4 => ("source-withdrawal", withdraw_source(&completed, 2), true),
            5 => ("policy-withdrawal", withdraw_source(&completed, 3), true),
            6 => ("cancelled", completed.clone(), true),
            7 => ("replay-mode", completed.clone(), true),
            8 => ("live-mode", completed.clone(), true),
            9 => ("stale-generation", completed.clone(), true),
            _ => unreachable!(),
        };
        if case >= 6 {
            let mut state = current.state().clone();
            match case {
                6 => state.intents[0].status = DurableStatus::Cancelled,
                7 => state.mode = ExecutionMode::Replay,
                8 => state.mode = ExecutionMode::Live,
                9 => state.intents[0].generation += 1,
                _ => unreachable!(),
            }
            current = model::checkpoint(current.basis(), state).unwrap();
        }
        let left = hidden_wire_checkpoint(
            &current,
            first,
            "PRIVATE-LEFT: hidden name, destination and memory metadata",
            150,
            replace_semantic,
            private_operation,
        );
        let right = hidden_wire_checkpoint(
            &current,
            first,
            "PRIVATE-RIGHT: different hidden name, destination and memory metadata",
            170,
            replace_semantic,
            private_operation,
        );
        for (slot, (role, role_kind, principal)) in roles.iter().enumerate() {
            let a = wire::journey_view(&left, *role_kind, *principal);
            let b = wire::journey_view(&right, *role_kind, *principal);
            match (a, b) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(a, b, "{name}/{role}: every speech/text/metadata field");
                    let encoded_a = a.encode_to_vec();
                    let encoded_b = b.encode_to_vec();
                    assert_eq!(encoded_a, encoded_b, "{name}/{role}: entire encoded wire");
                    assert!(
                        !encoded_a.is_empty(),
                        "{name}/{role}: actual encoded success"
                    );
                    if case == 0 {
                        authorized_success[slot] = true;
                    }
                    if case == 4 {
                        withdrawal_success[slot] = true;
                    }
                    assert_eq!(
                        crate::gameplay::rpc::ViewMessage::decode(encoded_a.as_slice()).unwrap(),
                        a
                    );
                    for private in [
                        "PRIVATE-LEFT",
                        "PRIVATE-RIGHT",
                        "hidden name",
                        "memory metadata",
                    ] {
                        assert!(
                            !encoded_a
                                .windows(private.len())
                                .any(|w| w == private.as_bytes()),
                            "{name}/{role}/{private}"
                        );
                    }
                    if let Some(crate::gameplay::rpc::view_message::Audience::Player(player)) =
                        &a.audience
                    {
                        if case == 0 && slot == 0 {
                            assert_eq!(player.private_clue, RESPONSE);
                        } else {
                            assert!(player.private_clue.is_empty(), "{name}/{role}");
                            safe_empty[slot] += 1;
                        }
                    } else {
                        safe_empty[slot] += 1;
                    }
                    retain_wire(name, role, "left", &encoded_a, "success");
                    retain_wire(name, role, "right", &encoded_b, "success");
                }
                (Err(a), Err(b)) => {
                    assert!(
                        case != 0 && case != 4,
                        "{name}/{role}: required successful projection refused: {a:?} / {b:?}"
                    );
                    assert_eq!(
                        format!("{a:?}"),
                        format!("{b:?}"),
                        "{name}/{role}: typed refusal"
                    );
                    for marker in ["PRIVATE-LEFT", "PRIVATE-RIGHT"] {
                        assert!(!format!("{a:?}").contains(marker));
                    }
                    retain_wire(name, role, "left", &[], "typed-refusal");
                    retain_wire(name, role, "right", &[], "typed-refusal");
                }
                _ => panic!("{name}/{role}: hidden retained payload changed outcome"),
            }
        }
    }
    assert!(authorized_success.into_iter().all(|success| success));
    assert!(
        withdrawal_success.into_iter().all(|success| success),
        "source definition withdrawal must produce full successful safe-empty wire for every role"
    );
    assert!(
        safe_empty.into_iter().all(|count| count > 0),
        "each role has actual full safe-empty witnesses"
    );
}

#[test]
fn source_withdrawal_refuses_current_prepared_summary_and_replay_after_positive_controls() {
    let (pending, first, second) = pending();
    let completed = stage_completion(
        &pending,
        &execute(&pending, intent(&pending)).unwrap(),
        completion_operation(intent(&pending)).unwrap(),
    )
    .unwrap();
    for salt in [150, 170] {
        let (current, request) = with_memory(&pending, first, RESPONSE, salt);
        let prepared = authored(&current);
        let selected =
            lookup_authorized_prepared(&prepared, &prepared.key, &prepared.basis).unwrap();
        assert_eq!(*selected, RESPONSE);
        let memory = NativeMemoryOwner::new(&current);
        assert_eq!(
            retrieve_ranked(&memory, &request, &current.basis(), memory_limits())
                .unwrap()
                .candidates(),
            &[&current.state().continuity.summaries[0]]
        );
        assert_eq!(memory.reads.get(), 1);
        assert_eq!(memory.costs.get(), 1);
        let mut replay = RecordingReplayOwner::new(&current, request.clone());
        install_record(&mut replay);
        replay.replay_mode();
        assert_eq!(
            lookup_authorized_replay(&replay, &replay.key, &replay.binding).unwrap(),
            &expected_completion(intent(&current)).unwrap()
        );
        assert_eq!(replay.replay_reads.get(), 1);
        let withdrawn = withdraw_source(&current, 2);
        let mut prepared = prepared;
        prepared.current = &withdrawn;
        prepared.intent = intent(&withdrawn);
        assert!(matches!(
            lookup_authorized_prepared(&prepared, &prepared.key, &prepared.basis),
            Err(AuthorizedLookupError::Authority(_))
        ));
        let refused_memory = NativeMemoryOwner::new(&current);
        *refused_memory.current.borrow_mut() = withdrawn.clone();
        assert!(
            retrieve_ranked(&refused_memory, &request, &current.basis(), memory_limits()).is_err()
        );
        assert_eq!(refused_memory.reads.get(), 0);
        assert_eq!(refused_memory.costs.get(), 0);
        assert_eq!(refused_memory.comparisons.get(), 0);
        let replay_current = replay.memory.current.borrow().clone();
        let replay_withdrawn = withdraw_source(&replay_current, 2);
        *replay.memory.current.borrow_mut() = replay_withdrawn.clone();
        assert!(matches!(
            lookup_authorized_replay(&replay, &replay.key, &replay.binding),
            Err(AuthorizedLookupError::Authority(_))
        ));
        assert_eq!(
            replay.replay_reads.get(),
            1,
            "withdrawal adds no replay storage read"
        );
        assert_eq!(replay.prepared_reads.get(), 0);
        assert_eq!(*refused_memory.current.borrow(), withdrawn);
        assert_eq!(*replay.memory.current.borrow(), replay_withdrawn);
        let wire_withdrawn = withdraw_source(&completed, 2);
        for (role, principal) in [
            (LocalDemoRole::Player, first),
            (LocalDemoRole::Player, second),
            (LocalDemoRole::Display, first),
        ] {
            let view = wire::journey_view(&wire_withdrawn, role, principal).unwrap();
            if let Some(crate::gameplay::rpc::view_message::Audience::Player(player)) =
                &view.audience
            {
                assert!(player.private_clue.is_empty());
            }
            assert!(!view.encode_to_vec().is_empty());
        }
    }
}
