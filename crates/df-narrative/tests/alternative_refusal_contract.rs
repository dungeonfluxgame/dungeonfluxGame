//! Authored alternatives and actual accepted refusal retain player agency.
//! The content owner, not this selector, admits the two authored alternatives.
use df_model::checkpoint::*;
use df_narrative::*;
use df_types::{OperationId, RevisionLabel};

#[expect(
    dead_code,
    reason = "shared canonical checkpoint fixture has other consumers"
)]
#[path = "../../df-engine/tests/support/fixture_model.rs"]
mod model;

fn content(entry: &str) -> ContentReference {
    ContentReference {
        package: model::pins().content.package,
        entry: model::label(entry),
    }
}

fn fact_id(value: u8) -> FactId {
    FactId::from_bytes(&[value; 16]).unwrap()
}

struct Fixture {
    references: Vec<ContentReference>,
    rules: Vec<RuleReference>,
    resources: Vec<ResourceConstraint>,
    checkpoint: Checkpoint,
    source: FactId,
    policy: RevisionLabel,
}

impl Fixture {
    fn new() -> Self {
        let references = [
            "fixture-entry-1",
            "opening",
            "courier-answer-seal",
            "courier-answer-escort",
            "ask-courier",
            "escort-courier",
            "packet-thread",
            "begin-story",
            "refusal-recorded",
            "follow-refusal-trail",
            "wait-for-courier",
            "refusal-trail",
            "refusal-wait",
        ]
        .map(content)
        .to_vec();
        let source = fact_id(20);
        let operation = OperationId::from_bytes(&[21; 16]).unwrap();
        let mut state = model::state();
        state.narrative.active_beats = vec![references[1].clone()];
        state.narrative.open_threads = vec![references[6].clone()];
        state.narrative.accepted_facts = vec![source];
        state.facts.push(GameFact {
            id: source,
            revision: model::basis().revision,
            operation,
            ordinal: 0,
            cause: None,
            audience: AudienceScope::Shared,
            value: FactValue::ContentEvent {
                definition: references[7].clone(),
                subjects: vec![],
            },
        });
        state.decisions.push(AcceptedDecision {
            operation,
            revision: model::basis().revision,
            facts: vec![source],
            draws: vec![],
            effects: vec![],
            source_policy: model::label("admitted-begin-story"),
            semantic_output: None,
        });
        let rules = vec![model::rule()];
        let resources = model::resource_constraints();
        let checkpoint = Checkpoint::new(
            CHECKPOINT_SCHEMA,
            model::basis(),
            model::pins(),
            state,
            ReferenceInventory {
                rules: &rules,
                content: &references,
                resources: &resources,
                assets: &[],
            },
            model::limits(),
        )
        .unwrap();
        Self {
            references,
            rules,
            resources,
            checkpoint,
            source,
            policy: model::label("authored-courier-alternatives"),
        }
    }

    fn recorded_refusal(&self) -> Checkpoint {
        let mut state = self.checkpoint.state().clone();
        let operation = OperationId::from_bytes(&[23; 16]).unwrap();
        let source = fact_id(22);
        state.facts.push(GameFact {
            id: source,
            revision: self.checkpoint.basis().revision,
            operation,
            ordinal: 0,
            cause: Some(self.source),
            audience: AudienceScope::Shared,
            value: FactValue::ContentEvent {
                definition: self.references[8].clone(),
                subjects: vec![],
            },
        });
        state.decisions.push(AcceptedDecision {
            operation,
            revision: self.checkpoint.basis().revision,
            facts: vec![source],
            draws: vec![],
            effects: vec![],
            source_policy: model::label("admitted-courier-refusal"),
            semantic_output: None,
        });
        self.checkpoint_with(state)
    }

    fn select_refusal(
        &self,
        current: &Checkpoint,
        selection: &ContentReference,
        limits: BeatSelectionLimits,
    ) -> Result<Checkpoint, BeatSelectionError> {
        let causes = [
            BeatCause {
                fact: self.source,
                event: &self.references[7],
                consumed_by_narrative: true,
            },
            BeatCause {
                fact: fact_id(22),
                event: &self.references[8],
                consumed_by_narrative: false,
            },
        ];
        let threads = [self.references[6].clone()];
        let alternatives = [
            AdmittedBeatAlternative {
                selection: &self.references[9],
                from: &self.references[1],
                to: &self.references[11],
                causes: &causes,
                required_threads: &threads,
                opened_thread: None,
            },
            AdmittedBeatAlternative {
                selection: &self.references[10],
                from: &self.references[1],
                to: &self.references[12],
                causes: &causes,
                required_threads: &threads,
                opened_thread: None,
            },
        ];
        stage_checkpoint_beat_selection(
            current,
            CheckpointBeatRequest {
                expected_basis: current.basis(),
                admitted_pins: current.pins(),
                policy: &self.policy,
                expected_policy: &self.policy,
                recipient: model::member(3),
                selection,
                alternatives: &alternatives,
                inventory: self.inventory(),
                checkpoint_limits: model::limits(),
            },
            limits,
        )
    }

    fn inventory(&self) -> ReferenceInventory<'_> {
        ReferenceInventory {
            rules: &self.rules,
            content: &self.references,
            resources: &self.resources,
            assets: &[],
        }
    }

    fn checkpoint_with(&self, state: GameState) -> Checkpoint {
        Checkpoint::new(
            CHECKPOINT_SCHEMA,
            self.checkpoint.basis(),
            self.checkpoint.pins().clone(),
            state,
            self.inventory(),
            model::limits(),
        )
        .unwrap()
    }

    fn beat_limits() -> BeatSelectionLimits {
        BeatSelectionLimits {
            records: 100,
            alternatives: 2,
            work: 4096,
        }
    }

    fn progress_limits() -> ProgressLimits {
        ProgressLimits {
            records: 100,
            consequences: 2,
            work: 4096,
        }
    }

    fn select(
        &self,
        current: &Checkpoint,
        selection: &ContentReference,
        cause: FactId,
        expected_basis: Basis,
        expected_policy: &RevisionLabel,
        duplicate: bool,
    ) -> Result<Checkpoint, BeatSelectionError> {
        let causes = [BeatCause {
            fact: cause,
            event: &self.references[7],
            consumed_by_narrative: true,
        }];
        let threads = [&self.references[6]].map(Clone::clone);
        let ask = AdmittedBeatAlternative {
            selection: &self.references[4],
            from: &self.references[1],
            to: &self.references[2],
            causes: &causes,
            required_threads: &threads,
            opened_thread: None,
        };
        let escort = AdmittedBeatAlternative {
            selection: &self.references[5],
            from: &self.references[1],
            to: &self.references[3],
            causes: &causes,
            required_threads: &threads,
            opened_thread: None,
        };
        let repeated_ask = AdmittedBeatAlternative {
            selection: &self.references[4],
            from: &self.references[1],
            to: &self.references[2],
            causes: &causes,
            required_threads: &threads,
            opened_thread: None,
        };
        let alternatives = if duplicate {
            [ask, repeated_ask]
        } else {
            [ask, escort]
        };
        stage_checkpoint_beat_selection(
            current,
            CheckpointBeatRequest {
                expected_basis,
                admitted_pins: self.checkpoint.pins(),
                policy: &self.policy,
                expected_policy,
                recipient: model::member(3),
                selection,
                alternatives: &alternatives,
                inventory: self.inventory(),
                checkpoint_limits: model::limits(),
            },
            Self::beat_limits(),
        )
    }

    fn progress(
        &self,
        current: &Checkpoint,
        selections: &[ThreadConsequenceSelection<'_>],
    ) -> Result<CheckpointThreadProgressProposal, CheckpointProgressError> {
        stage_checkpoint_thread_progress(
            current,
            ThreadCheckpointRequest {
                expected_basis: current.basis(),
                admitted_pins: current.pins(),
                policy: &self.policy,
                expected_policy: &self.policy,
                inventory: self.inventory(),
                checkpoint_limits: model::limits(),
                selections,
            },
            Self::progress_limits(),
        )
    }
}

#[test]
fn unselected_and_refused_route_leave_player_choice_and_canonical_state_untouched() {
    let fixture = Fixture::new();
    let current = &fixture.checkpoint;
    let original = current.clone();
    let refusal = fixture.select(
        current,
        &fixture.references[8],
        fixture.source,
        current.basis(),
        &fixture.policy,
        false,
    );
    assert_eq!(refusal, Err(BeatSelectionError::Selection));
    assert_eq!(current, &original);
    assert!(current.state().narrative.completed_beats.is_empty());
    assert_eq!(
        current.state().narrative.active_beats,
        [fixture.references[1].clone()]
    );
    assert!(current.state().characters[0].choices.is_empty());
    assert_eq!(current.state().resources[0].value, 4);
    assert_eq!(current.state().facts.len(), 1);
    assert_eq!(current.state().decisions.len(), 1);
}

#[test]
fn explicitly_selected_alternative_stages_only_authored_route_and_preserves_cause() {
    let fixture = Fixture::new();
    let current = &fixture.checkpoint;
    for (selection, destination) in [(4, 2), (5, 3)] {
        let staged = fixture
            .select(
                current,
                &fixture.references[selection],
                fixture.source,
                current.basis(),
                &fixture.policy,
                false,
            )
            .unwrap();
        let narrative = &staged.state().narrative;
        assert_eq!(
            narrative.active_beats,
            [fixture.references[destination].clone()]
        );
        assert_eq!(narrative.completed_beats, [fixture.references[1].clone()]);
        assert_eq!(narrative.accepted_facts, [fixture.source]);
        assert_eq!(
            narrative.open_threads,
            current.state().narrative.open_threads
        );
        let mut protected = staged.state().clone();
        protected.narrative = current.state().narrative.clone();
        assert_eq!(protected, *current.state());
        assert_eq!(staged.basis(), current.basis());
        assert_eq!(staged.pins(), current.pins());
        assert_eq!(
            current.state().narrative.active_beats,
            [fixture.references[1].clone()]
        );
    }
}

#[test]
fn repeated_stale_foreign_and_ambiguous_route_requests_refuse_atomically() {
    let fixture = Fixture::new();
    let current = &fixture.checkpoint;
    let ask = &fixture.references[4];
    let selected = fixture
        .select(
            current,
            ask,
            fixture.source,
            current.basis(),
            &fixture.policy,
            false,
        )
        .unwrap();
    assert_eq!(
        fixture.select(
            &selected,
            ask,
            fixture.source,
            selected.basis(),
            &fixture.policy,
            false
        ),
        Err(BeatSelectionError::Prerequisite)
    );
    assert_eq!(
        fixture.select(
            current,
            ask,
            fixture.source,
            current.basis(),
            &fixture.policy,
            true
        ),
        Err(BeatSelectionError::Selection)
    );
    assert_eq!(
        fixture.select(
            current,
            ask,
            fact_id(99),
            current.basis(),
            &fixture.policy,
            false
        ),
        Err(BeatSelectionError::Source)
    );
    assert_eq!(
        fixture.select(
            current,
            ask,
            fixture.source,
            Basis {
                revision: current.basis().revision.next_sequence().unwrap(),
                ..current.basis()
            },
            &fixture.policy,
            false,
        ),
        Err(BeatSelectionError::Binding(CheckpointError::StaleBasis))
    );
    assert_eq!(
        fixture.select(
            current,
            ask,
            fixture.source,
            current.basis(),
            &model::label("foreign-policy"),
            false
        ),
        Err(BeatSelectionError::StalePolicy)
    );
    let mut unsupported = current.state().clone();
    unsupported.decisions.clear();
    let unsupported = fixture.checkpoint_with(unsupported);
    assert_eq!(
        fixture.select(
            &unsupported,
            ask,
            fixture.source,
            unsupported.basis(),
            &fixture.policy,
            false
        ),
        Err(BeatSelectionError::Source)
    );
    assert_eq!(current, &fixture.checkpoint);
    assert_eq!(selected.state().facts, current.state().facts);
}

#[test]
fn missing_understanding_thread_and_source_audience_refuse_without_fabricating_access() {
    let fixture = Fixture::new();
    let current = &fixture.checkpoint;
    let ask = &fixture.references[4];
    for case in 0..4 {
        let mut state = current.state().clone();
        let expected = match case {
            0 => {
                state.narrative.accepted_facts.clear();
                BeatSelectionError::Source
            }
            1 => {
                state.narrative.open_threads.clear();
                BeatSelectionError::Prerequisite
            }
            2 => {
                state.facts[0].audience = AudienceScope::Host;
                BeatSelectionError::Recipient
            }
            3 => {
                state.facts[0].value = FactValue::ContentEvent {
                    definition: fixture.references[8].clone(),
                    subjects: vec![],
                };
                BeatSelectionError::Source
            }
            _ => unreachable!(),
        };
        let changed = fixture.checkpoint_with(state);
        let original = changed.clone();
        assert_eq!(
            fixture.select(
                &changed,
                ask,
                fixture.source,
                changed.basis(),
                &fixture.policy,
                false,
            ),
            Err(expected),
            "case {case}"
        );
        assert_eq!(changed, original);
        assert!(changed.state().characters[0].choices.is_empty());
        assert_eq!(changed.state().resources[0].value, 4);
    }
}

#[test]
fn refusal_consequence_retains_thread_until_an_admitted_resolution() {
    let fixture = Fixture::new();
    let refused = fixture.recorded_refusal();
    let current = &refused;
    let selection = [ThreadConsequenceSelection {
        thread: &fixture.references[6],
        event_definition: &fixture.references[8],
        source: fact_id(22),
        disposition: ThreadDisposition::Continue,
    }];
    // The actual accepted refusal may be consumed once while the player's thread remains open.
    let unconsumed = current.clone();
    let continued = fixture.progress(&unconsumed, &selection).unwrap();
    assert_eq!(
        continued.checkpoint.state().narrative.open_threads,
        [fixture.references[6].clone()]
    );
    assert_eq!(
        continued.checkpoint.state().narrative.active_beats,
        unconsumed.state().narrative.active_beats
    );
    assert_eq!(continued.evidence[0].source, fact_id(22));
    assert_eq!(
        continued.evidence[0].disposition,
        ThreadDisposition::Continue
    );
    assert_eq!(
        fixture.progress(&continued.checkpoint, &selection),
        Err(CheckpointProgressError::Progress(
            ProgressError::RepeatedConsequence
        ))
    );
    let bad = [ThreadConsequenceSelection {
        thread: &fixture.references[6],
        event_definition: &fixture.references[8],
        source: fact_id(98),
        disposition: ThreadDisposition::Continue,
    }];
    assert_eq!(
        fixture.progress(&unconsumed, &bad),
        Err(CheckpointProgressError::Progress(
            ProgressError::MissingFact
        ))
    );
    assert_eq!(
        unconsumed.state().narrative.open_threads,
        [fixture.references[6].clone()]
    );
}

#[test]
fn actual_refusal_enables_two_authored_fallbacks_without_forcing_either_or_granting_success() {
    let fixture = Fixture::new();
    let refused = fixture.recorded_refusal();
    let unchanged = refused.clone();
    for (selection, destination) in [(9, 11), (10, 12)] {
        assert_eq!(
            fixture.select_refusal(
                &fixture.checkpoint,
                &fixture.references[selection],
                Fixture::beat_limits(),
            ),
            Err(BeatSelectionError::Source),
        );
        let candidate = fixture
            .select_refusal(
                &refused,
                &fixture.references[selection],
                Fixture::beat_limits(),
            )
            .unwrap();
        assert_eq!(
            candidate.state().narrative.active_beats,
            [fixture.references[destination].clone()]
        );
        let mut protected = candidate.state().clone();
        protected.narrative = refused.state().narrative.clone();
        assert_eq!(protected, *refused.state());
        assert_eq!(
            candidate.state().narrative.open_threads,
            refused.state().narrative.open_threads
        );
        assert_eq!(
            candidate.state().narrative.accepted_facts,
            refused.state().narrative.accepted_facts
        );
        assert_eq!(candidate.state().resources[0].value, 4);
        assert!(candidate.state().knowledge.is_empty());
    }
    assert_eq!(refused, unchanged);
    assert_eq!(
        refused.state().narrative.active_beats,
        [fixture.references[1].clone()]
    );
}

#[test]
fn refusal_fallback_rejects_revoked_unaccepted_and_mismatched_evidence_atomically() {
    let fixture = Fixture::new();
    let refused = fixture.recorded_refusal();
    for case in 0..3 {
        let mut state = refused.state().clone();
        let expected = match case {
            0 => {
                state.facts[1].audience = AudienceScope::Host;
                BeatSelectionError::Recipient
            }
            1 => {
                state.decisions.pop();
                BeatSelectionError::Source
            }
            _ => {
                state.facts[1].value = FactValue::ContentEvent {
                    definition: fixture.references[7].clone(),
                    subjects: vec![],
                };
                BeatSelectionError::Source
            }
        };
        let current = fixture.checkpoint_with(state);
        let unchanged = current.clone();
        assert_eq!(
            fixture.select_refusal(&current, &fixture.references[9], Fixture::beat_limits()),
            Err(expected)
        );
        assert_eq!(current, unchanged);
    }
}

#[test]
fn refusal_fallback_count_and_work_limits_return_no_candidate() {
    let fixture = Fixture::new();
    let current = fixture.recorded_refusal();
    let unchanged = current.clone();
    for limits in [
        BeatSelectionLimits {
            alternatives: 1,
            ..Fixture::beat_limits()
        },
        BeatSelectionLimits {
            records: 1,
            ..Fixture::beat_limits()
        },
        BeatSelectionLimits {
            work: 0,
            ..Fixture::beat_limits()
        },
    ] {
        assert_eq!(
            fixture.select_refusal(&current, &fixture.references[9], limits),
            Err(BeatSelectionError::Capacity)
        );
    }
    assert_eq!(current, unchanged);
}

#[test]
fn authored_alternative_cannot_reopen_or_complete_an_already_completed_beat() {
    let fixture = Fixture::new();
    for completed in [1, 2] {
        let mut state = fixture.checkpoint.state().clone();
        state
            .narrative
            .completed_beats
            .push(fixture.references[completed].clone());
        let current = fixture.checkpoint_with(state);
        let unchanged = current.clone();
        assert_eq!(
            fixture.select(
                &current,
                &fixture.references[4],
                fixture.source,
                current.basis(),
                &fixture.policy,
                false
            ),
            Err(BeatSelectionError::Prerequisite),
        );
        assert_eq!(current, unchanged);
    }
}
