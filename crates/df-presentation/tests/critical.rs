#[path = "../../df-session/tests/support/fixture_model.rs"]
pub mod fixture_model;

use df_model::checkpoint::*;
use df_presentation::critical::*;

fn record_id(value: u8) -> RecordId {
    RecordId::from_bytes(&[value; 16]).unwrap()
}
fn resolution() -> ResolutionId {
    ResolutionId::from_bytes(&[51; 16]).unwrap()
}
fn window() -> WindowId {
    WindowId::from_bytes(&[52; 16]).unwrap()
}

struct Consumer {
    checkpoint: Checkpoint,
    rules: Vec<RuleReference>,
    content: Vec<ContentReference>,
    resources: Vec<ResourceConstraint>,
    assets: Vec<AssetReference>,
}

impl Consumer {
    fn new(edit: impl FnOnce(&mut GameState)) -> Self {
        let rules = vec![fixture_model::rule()];
        let content = vec![fixture_model::content()];
        let resources = fixture_model::resource_constraints();
        let assets = vec![AssetReference {
            key: fixture_model::label("critical-prepared-image-1"),
            digest: ContentDigest([18; 32]),
            byte_length: 4,
            kind: AssetKind::Image,
        }];
        let mut state = fixture_model::state();
        state.mode = ExecutionMode::Live;
        state.facts.push(GameFact {
            id: FactId::from_bytes(&[31; 16]).unwrap(),
            revision: fixture_model::basis().revision,
            operation: fixture_model::operation(),
            ordinal: 0,
            cause: None,
            audience: AudienceScope::Shared,
            value: FactValue::ContentEvent {
                definition: fixture_model::content(),
                subjects: vec![fixture_model::entity(4)],
            },
        });
        state.decisions.push(AcceptedDecision {
            operation: fixture_model::operation(),
            revision: fixture_model::basis().revision,
            facts: vec![state.facts[0].id],
            draws: vec![],
            effects: vec![],
            source_policy: fixture_model::label("critical-resolved-policy-1"),
            semantic_output: Some("source-owner admitted committed outcome".to_owned()),
        });
        state.continuity.critical_cues.push(CriticalCueEligibility {
            id: record_id(32),
            fact: state.facts[0].id,
            resolution: resolution(),
            audience: AudienceScope::Shared,
            ready_assets: assets.clone(),
            policy: fixture_model::content(),
            maximum_duration_ticks: 20,
        });
        edit(&mut state);
        let checkpoint = Checkpoint::new(
            CHECKPOINT_SCHEMA,
            fixture_model::basis(),
            fixture_model::pins(),
            state,
            ReferenceInventory {
                rules: &rules,
                content: &content,
                resources: &resources,
                assets: &assets,
            },
            Self::checkpoint_limits(),
        )
        .unwrap();
        Self {
            checkpoint,
            rules,
            content,
            resources,
            assets,
        }
    }

    fn checkpoint_limits() -> CheckpointLimits {
        let mut limits = fixture_model::limits();
        limits.maximum_records = 512;
        limits.maximum_total_text_bytes = 16 * 1024;
        limits
    }

    fn inventory(&self) -> ReferenceInventory<'_> {
        ReferenceInventory {
            rules: &self.rules,
            content: &self.content,
            resources: &self.resources,
            assets: &self.assets,
        }
    }

    fn context(&self) -> CriticalContext<'_> {
        CriticalContext {
            basis: self.checkpoint.basis(),
            pins: self.checkpoint.pins(),
            cue: &self.checkpoint.state().continuity.critical_cues[0],
        }
    }

    fn request(&self) -> CriticalCueRequest<'_> {
        CriticalCueRequest {
            context: self.context(),
            now: LogicalTime {
                ticks: 120,
                ticks_per_second: 10,
            },
            duration_ticks: 10,
            limits: CriticalCueLimits {
                maximum_items: 128,
                maximum_asset_bytes: 64,
                maximum_duration_ticks: 30,
            },
        }
    }

    fn outcome(&self) -> ResolvedOutcomeAdmission<'_> {
        // Test-only trusted source owner explicitly admits these full canonical records.
        // Their labels, checkpoint membership and draw values never classify success.
        ResolvedOutcomeAdmission {
            context: self.context(),
            fact: &self.checkpoint.state().facts[0],
            decision: &self.checkpoint.state().decisions[0],
            resolution: resolution(),
            source: &self.rules[0],
            outcome: ResolvedOutcome::Succeeded,
        }
    }

    fn permitted(&self) -> PermittedCriticalCue<'_> {
        // This fixture owner represents current filtered inventory, not application rights.
        PermittedCriticalCue {
            context: self.context(),
            fact: &self.checkpoint.state().facts[0],
            content: &self.content,
            assets: &self.assets,
            cadence: CueCadence::Allowed,
            expires: LogicalTime {
                ticks: 140,
                ticks_per_second: 10,
            },
        }
    }

    fn select(
        &self,
        request: CriticalCueRequest<'_>,
        outcome: Option<ResolvedOutcomeAdmission<'_>>,
        permitted: Option<PermittedCriticalCue<'_>>,
    ) -> Result<(), CriticalCueError> {
        select_committed_celebration(
            &self.checkpoint,
            request,
            self.inventory(),
            Self::checkpoint_limits(),
            outcome,
            permitted,
        )
        .map(|_| ())
    }
}

#[test]
fn exact_source_admitted_committed_outcome_precedes_eligible_cosmetic_proposal() {
    let consumer = Consumer::new(|_| {});
    let before = consumer.checkpoint.clone();
    let selected = select_committed_celebration(
        &consumer.checkpoint,
        consumer.request(),
        consumer.inventory(),
        Consumer::checkpoint_limits(),
        Some(consumer.outcome()),
        Some(consumer.permitted()),
    )
    .unwrap();
    assert_eq!(selected, consumer.context().cue);
    assert!(std::ptr::eq(selected, consumer.context().cue));
    assert_eq!(consumer.checkpoint, before);
    assert!(!consumer.checkpoint.state().resources.is_empty());
    assert_eq!(
        consumer.checkpoint.state().logical_time,
        before.state().logical_time
    );
}

#[test]
fn pending_choice_uncommitted_content_or_die20_never_implies_celebration() {
    let consumer = Consumer::new(|state| {
        state.draws.push(ActualDraw {
            operation: fixture_model::operation(),
            ordinal: 0,
            resolution: resolution(),
            window: window(),
            sides: 20,
            value: 20,
            source: fixture_model::rule(),
        });
        state.decisions[0].draws.push(0);
    });
    let before = consumer.checkpoint.clone();
    assert_eq!(
        consumer.select(consumer.request(), None, Some(consumer.permitted())),
        Err(CriticalCueError::MissingOutcomeAdmission)
    );
    let mut failed = consumer.outcome();
    failed.outcome = ResolvedOutcome::Failed;
    assert_eq!(
        consumer.select(consumer.request(), Some(failed), Some(consumer.permitted())),
        Err(CriticalCueError::OutcomeNotSuccessful)
    );
    let mut invented = consumer.outcome().fact.clone();
    invented.value = FactValue::ContentEvent {
        definition: fixture_model::content(),
        subjects: vec![],
    };
    let mut admission = consumer.outcome();
    admission.fact = &invented;
    assert_eq!(
        consumer.select(
            consumer.request(),
            Some(admission),
            Some(consumer.permitted())
        ),
        Err(CriticalCueError::InvalidOutcomeAdmission)
    );
    assert_eq!(consumer.checkpoint, before);

    let pending = Consumer::new(|state| {
        let choice = AcceptedChoice {
            participant: fixture_model::member(3),
            offer: fixture_model::label("reaction-offer-1"),
            selected: fixture_model::label("continue-1"),
            source: fixture_model::rule(),
        };
        state.facts[0].value = FactValue::ChoiceAccepted {
            resolution: resolution(),
            window: window(),
            choice: choice.clone(),
        };
        state.pending.push(PendingResolution {
            id: resolution(),
            basis: fixture_model::basis(),
            continuation: fixture_model::label("resolve-after-reaction-1"),
            window: ResolutionWindow {
                id: window(),
                phase: TriggerPhase::BeforeConsequence,
                causal_fact: state.facts[0].id,
                source: fixture_model::rule(),
                timer: None,
            },
            next: PendingInput::Reaction {
                remaining: vec![OfferedResponse {
                    participant: fixture_model::member(3),
                    offer: fixture_model::label("reaction-offer-1"),
                    options: vec![fixture_model::label("continue-1")],
                    source: fixture_model::rule(),
                }],
            },
            choices: vec![choice],
            draw_ordinals: vec![],
            spent: vec![],
            rulings: vec![],
        });
    });
    let before = pending.checkpoint.clone();
    assert_eq!(
        pending.select(
            pending.request(),
            Some(pending.outcome()),
            Some(pending.permitted())
        ),
        Err(CriticalCueError::PendingResolution)
    );
    assert_eq!(pending.checkpoint, before);
    let draw_only = Consumer::new(|state| {
        state.draws.push(ActualDraw {
            operation: fixture_model::operation(),
            ordinal: 0,
            resolution: resolution(),
            window: window(),
            sides: 20,
            value: 20,
            source: fixture_model::rule(),
        });
        state.decisions[0].draws.push(0);
        state.facts[0].value = FactValue::DrawAccepted {
            operation: fixture_model::operation(),
            ordinal: 0,
        };
    });
    assert_eq!(
        draw_only.select(
            draw_only.request(),
            Some(draw_only.outcome()),
            Some(draw_only.permitted())
        ),
        Err(CriticalCueError::InvalidOutcomeAdmission)
    );
}

#[test]
fn wrong_resolution_source_basis_or_decision_membership_refuses() {
    let unaccepted = Consumer::new(|state| state.decisions[0].facts.clear());
    assert_eq!(
        unaccepted.select(
            unaccepted.request(),
            Some(unaccepted.outcome()),
            Some(unaccepted.permitted())
        ),
        Err(CriticalCueError::InvalidOutcomeAdmission)
    );

    let wrong_intrinsic_resolution = Consumer::new(|state| {
        state.facts[0].value = FactValue::ChoiceAccepted {
            resolution: ResolutionId::from_bytes(&[93; 16]).unwrap(),
            window: window(),
            choice: AcceptedChoice {
                participant: fixture_model::member(3),
                offer: fixture_model::label("offer-1"),
                selected: fixture_model::label("choice-1"),
                source: fixture_model::rule(),
            },
        };
    });
    assert_eq!(
        wrong_intrinsic_resolution.select(
            wrong_intrinsic_resolution.request(),
            Some(wrong_intrinsic_resolution.outcome()),
            Some(wrong_intrinsic_resolution.permitted()),
        ),
        Err(CriticalCueError::InvalidOutcomeAdmission)
    );

    let consumer = Consumer::new(|_| {});
    let mut outcome = consumer.outcome();
    outcome.resolution = ResolutionId::from_bytes(&[99; 16]).unwrap();
    assert_eq!(
        consumer.select(
            consumer.request(),
            Some(outcome),
            Some(consumer.permitted())
        ),
        Err(CriticalCueError::InvalidOutcomeAdmission)
    );
    let mut source = fixture_model::rule();
    source.clause = fixture_model::label("unadmitted-source");
    outcome = consumer.outcome();
    outcome.source = &source;
    assert_eq!(
        consumer.select(
            consumer.request(),
            Some(outcome),
            Some(consumer.permitted())
        ),
        Err(CriticalCueError::InvalidOutcomeAdmission)
    );
    let mut decision = consumer.outcome().decision.clone();
    decision.facts.clear();
    outcome = consumer.outcome();
    outcome.decision = &decision;
    assert_eq!(
        consumer.select(
            consumer.request(),
            Some(outcome),
            Some(consumer.permitted())
        ),
        Err(CriticalCueError::InvalidOutcomeAdmission)
    );
    let mut request = consumer.request();
    request.context.basis.revision = request.context.basis.revision.next_sequence().unwrap();
    assert_eq!(
        consumer.select(
            request,
            Some(consumer.outcome()),
            Some(consumer.permitted())
        ),
        Err(CriticalCueError::Checkpoint(CheckpointError::StaleBasis))
    );
    let mut pins = consumer.checkpoint.pins().clone();
    pins.rules.handler_digest = ContentDigest([92; 32]);
    request = consumer.request();
    request.context.pins = &pins;
    assert_eq!(
        consumer.select(
            request,
            Some(consumer.outcome()),
            Some(consumer.permitted())
        ),
        Err(CriticalCueError::Checkpoint(CheckpointError::RulesMismatch))
    );
}

#[test]
fn private_outcome_or_unpermitted_assets_cannot_affect_public_cue() {
    let consumer = Consumer::new(|state| {
        state.facts[0].audience = AudienceScope::Members(vec![fixture_model::member(3)]);
    });
    let before = consumer.checkpoint.clone();
    assert_eq!(
        consumer.select(
            consumer.request(),
            Some(consumer.outcome()),
            Some(consumer.permitted())
        ),
        Err(CriticalCueError::AudienceDenied)
    );
    assert_eq!(consumer.checkpoint, before);
    let public = Consumer::new(|_| {});
    let mut permitted = public.permitted();
    permitted.assets = &[];
    assert_eq!(
        public.select(public.request(), Some(public.outcome()), Some(permitted)),
        Err(CriticalCueError::AssetsUnavailable)
    );
    let mut changed = public.assets.clone();
    changed[0].digest = ContentDigest([99; 32]);
    permitted = public.permitted();
    permitted.assets = &changed;
    assert_eq!(
        public.select(public.request(), Some(public.outcome()), Some(permitted)),
        Err(CriticalCueError::AssetsUnavailable)
    );
}

#[test]
fn replay_stale_or_missing_outcome_admission_returns_no_fresh_impact() {
    let replay = Consumer::new(|state| state.mode = ExecutionMode::Replay);
    let before = replay.checkpoint.clone();
    assert_eq!(
        replay.select(
            replay.request(),
            Some(replay.outcome()),
            Some(replay.permitted())
        ),
        Err(CriticalCueError::ReplaySuppressed)
    );
    assert_eq!(replay.checkpoint, before);
    let current = Consumer::new(|_| {});
    let mut outcome = current.outcome();
    outcome.context.basis.run = df_types::RunId::from_bytes(&[77; 16]).unwrap();
    assert_eq!(
        current.select(current.request(), Some(outcome), Some(current.permitted())),
        Err(CriticalCueError::InvalidOutcomeAdmission)
    );
    assert_eq!(
        current.select(current.request(), None, Some(current.permitted())),
        Err(CriticalCueError::MissingOutcomeAdmission)
    );
    let mut permitted = current.permitted();
    permitted.context.basis.revision = fixture_model::revision(1, 8);
    assert_eq!(
        current.select(current.request(), Some(current.outcome()), Some(permitted)),
        Err(CriticalCueError::StalePermission)
    );
}

#[test]
fn capacity_duration_duplicates_and_missing_policy_refuse_without_state_or_input_changes() {
    let current = Consumer::new(|_| {});
    let before = current.checkpoint.clone();
    let mut request = current.request();
    request.limits.maximum_items = 1;
    assert_eq!(
        current.select(request, Some(current.outcome()), Some(current.permitted())),
        Err(CriticalCueError::Capacity)
    );
    request = current.request();
    request.duration_ticks = 21;
    assert_eq!(
        current.select(request, Some(current.outcome()), Some(current.permitted())),
        Err(CriticalCueError::InvalidTime)
    );
    request = current.request();
    request.limits.maximum_asset_bytes = 3;
    assert_eq!(
        current.select(request, Some(current.outcome()), Some(current.permitted())),
        Err(CriticalCueError::Capacity)
    );
    let mut permitted = current.permitted();
    permitted.content = &[];
    assert_eq!(
        current.select(current.request(), Some(current.outcome()), Some(permitted)),
        Err(CriticalCueError::AudienceDenied)
    );
    permitted = current.permitted();
    permitted.cadence = CueCadence::Suppressed;
    assert_eq!(
        current.select(current.request(), Some(current.outcome()), Some(permitted)),
        Err(CriticalCueError::Suppressed)
    );
    permitted = current.permitted();
    permitted.expires.ticks = 125;
    assert_eq!(
        current.select(current.request(), Some(current.outcome()), Some(permitted)),
        Err(CriticalCueError::Expired)
    );
    assert_eq!(
        current.select(current.request(), Some(current.outcome()), None),
        Err(CriticalCueError::MissingPermission)
    );
    assert_eq!(current.checkpoint, before);
    let duplicate = Consumer::new(|state| {
        let asset = state.continuity.critical_cues[0].ready_assets[0].clone();
        state.continuity.critical_cues[0].ready_assets.push(asset);
    });
    assert_eq!(
        duplicate.select(
            duplicate.request(),
            Some(duplicate.outcome()),
            Some(duplicate.permitted())
        ),
        Err(CriticalCueError::AssetsUnavailable)
    );
}
