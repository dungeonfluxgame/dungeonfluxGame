//! A content route must be structurally complete, then pass current narrative admission.
use df_content::graph::{
    GraphAlternative, GraphBeat, GraphError, GraphLimits, GraphView, validate_graph,
};

type SceneGraphError = GraphError<(String, String), FactId, (String, String)>;
use df_model::checkpoint::*;
use df_narrative::{
    AdmittedBeatAlternative, BeatCause, BeatSelectionError, BeatSelectionLimits,
    CheckpointBeatRequest, stage_checkpoint_beat_selection,
};
use df_types::{BuildIdentity, OperationId, RevisionLabel};

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

fn graph_id(reference: &ContentReference) -> (String, String) {
    (
        reference.package.as_str().to_owned(),
        reference.entry.as_str().to_owned(),
    )
}

fn graph_limits() -> GraphLimits {
    GraphLimits {
        max_beats: 8,
        max_edges: 8,
        max_facts: 4,
        max_alternatives: 4,
        max_work: 4096,
    }
}

struct Fixture {
    room: ContentReference,
    opening: ContentReference,
    seal: ContentReference,
    escort: ContentReference,
    ask: ContentReference,
    escort_choice: ContentReference,
    thread: ContentReference,
    event: ContentReference,
    foreign_event: ContentReference,
    references: Vec<ContentReference>,
    rules: Vec<RuleReference>,
    resources: Vec<ResourceConstraint>,
    checkpoint: Checkpoint,
    source: FactId,
    policy: RevisionLabel,
}

impl Fixture {
    fn new() -> Self {
        let room = content("room");
        let opening = content("opening");
        let seal = content("courier-answer-seal");
        let escort = content("courier-answer-escort");
        let ask = content("ask-courier");
        let escort_choice = content("escort-courier");
        let thread = content("packet-thread");
        let event = content("begin-story");
        let foreign_event = content("foreign-event");
        let references = [
            model::content(),
            room.clone(),
            opening.clone(),
            seal.clone(),
            escort.clone(),
            ask.clone(),
            escort_choice.clone(),
            thread.clone(),
            event.clone(),
            foreign_event.clone(),
        ]
        .to_vec();
        let source = fact_id(20);
        let operation = OperationId::from_bytes(&[21; 16]).unwrap();
        let mut state = model::state();
        state.narrative.active_beats = vec![opening.clone()];
        state.narrative.open_threads = vec![thread.clone()];
        state.narrative.accepted_facts = vec![source];
        state.facts.push(GameFact {
            id: source,
            revision: model::basis().revision,
            operation,
            ordinal: 0,
            cause: None,
            audience: AudienceScope::Shared,
            value: FactValue::ContentEvent {
                definition: event.clone(),
                subjects: vec![],
            },
        });
        state.decisions.push(AcceptedDecision {
            operation,
            revision: model::basis().revision,
            facts: vec![source],
            draws: vec![],
            effects: vec![],
            source_policy: model::label("admitted-story-policy"),
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
            room,
            opening,
            seal,
            escort,
            ask,
            escort_choice,
            thread,
            event,
            foreign_event,
            references,
            rules,
            resources,
            checkpoint,
            source,
            policy: model::label("authored-story-policy"),
        }
    }

    fn inventory<'a>(&'a self, content: &'a [ContentReference]) -> ReferenceInventory<'a> {
        ReferenceInventory {
            rules: &self.rules,
            content,
            resources: &self.resources,
            assets: &[],
        }
    }

    fn graph(&self) -> Result<(), Box<SceneGraphError>> {
        let room = graph_id(&self.room);
        let opening = graph_id(&self.opening);
        let seal = graph_id(&self.seal);
        let escort = graph_id(&self.escort);
        let ask = graph_id(&self.ask);
        let escort_choice = graph_id(&self.escort_choice);
        let room_to_opening = [opening.clone()];
        let opening_to_exits = [seal.clone(), escort.clone()];
        let no_next = [];
        let beats = [
            GraphBeat {
                id: &room,
                next: &room_to_opening,
                terminal: false,
            },
            GraphBeat {
                id: &opening,
                next: &opening_to_exits,
                terminal: false,
            },
            GraphBeat {
                id: &seal,
                next: &no_next,
                terminal: true,
            },
            GraphBeat {
                id: &escort,
                next: &no_next,
                terminal: true,
            },
        ];
        let alternatives = [
            GraphAlternative {
                id: &ask,
                owner: &room,
                entry: &opening,
                exit: &seal,
                disclosure_fact: Some(&self.source),
            },
            GraphAlternative {
                id: &escort_choice,
                owner: &room,
                entry: &opening,
                exit: &escort,
                disclosure_fact: Some(&self.source),
            },
        ];
        let facts = [self.source];
        validate_graph(
            &GraphView {
                start: &room,
                beats: &beats,
                facts: &facts,
                alternatives: &alternatives,
            },
            graph_limits(),
        )
        .map_err(Box::new)
    }

    // This test helper forwards the complete request shape accepted by the director.
    #[allow(clippy::too_many_arguments)]
    fn select(
        &self,
        current: &Checkpoint,
        selection: &ContentReference,
        cause: FactId,
        expected_policy: &RevisionLabel,
        admitted_pins: &CheckpointPins,
        expected_basis: Basis,
        content: &[ContentReference],
        limits: BeatSelectionLimits,
    ) -> Result<Checkpoint, BeatSelectionError> {
        let causes = [BeatCause {
            fact: cause,
            event: &self.event,
            consumed_by_narrative: true,
        }];
        let required_threads = [self.thread.clone()];
        let alternatives = [
            AdmittedBeatAlternative {
                selection: &self.ask,
                from: &self.opening,
                to: &self.seal,
                causes: &causes,
                required_threads: &required_threads,
                opened_thread: None,
            },
            AdmittedBeatAlternative {
                selection: &self.escort_choice,
                from: &self.opening,
                to: &self.escort,
                causes: &causes,
                required_threads: &required_threads,
                opened_thread: None,
            },
        ];
        stage_checkpoint_beat_selection(
            current,
            CheckpointBeatRequest {
                expected_basis,
                admitted_pins,
                policy: &self.policy,
                expected_policy,
                recipient: model::member(3),
                selection,
                alternatives: &alternatives,
                inventory: self.inventory(content),
                checkpoint_limits: model::limits(),
            },
            limits,
        )
    }

    fn limits() -> BeatSelectionLimits {
        BeatSelectionLimits {
            records: 64,
            alternatives: 2,
            work: 4096,
        }
    }
}

#[test]
fn authored_alternatives_have_current_canonical_prerequisites_and_distinct_terminal_exits() {
    let fixture = Fixture::new();
    fixture.graph().unwrap();
    for (selection, destination) in [
        (&fixture.ask, &fixture.seal),
        (&fixture.escort_choice, &fixture.escort),
    ] {
        let before = fixture.checkpoint.clone();
        let staged = fixture
            .select(
                &fixture.checkpoint,
                selection,
                fixture.source,
                &fixture.policy,
                fixture.checkpoint.pins(),
                fixture.checkpoint.basis(),
                &fixture.references,
                Fixture::limits(),
            )
            .unwrap();
        let mut expected_state = before.state().clone();
        expected_state.narrative.active_beats = vec![destination.clone()];
        expected_state.narrative.completed_beats = vec![fixture.opening.clone()];
        assert_eq!(staged.state(), &expected_state);
        assert_eq!(staged.state().facts, before.state().facts);
        assert_eq!(staged.state().decisions, before.state().decisions);
        assert_eq!(staged.state().draws, before.state().draws);
        assert_eq!(
            staged.state().narrative.accepted_facts,
            before.state().narrative.accepted_facts
        );
        assert_eq!(
            staged.state().narrative.open_threads,
            before.state().narrative.open_threads
        );
        assert_eq!(staged.basis(), before.basis());
        assert_eq!(staged.pins(), before.pins());
        assert_eq!(fixture.checkpoint, before);
    }
}

#[test]
fn graph_refuses_missing_references_nonterminal_exits_and_unbounded_work() {
    let fixture = Fixture::new();
    let room = graph_id(&fixture.room);
    let opening = graph_id(&fixture.opening);
    let seal = graph_id(&fixture.seal);
    let escort = graph_id(&fixture.escort);
    let ask = graph_id(&fixture.ask);
    let escort_choice = graph_id(&fixture.escort_choice);
    let thread = graph_id(&fixture.thread);
    let room_to_opening = [opening.clone()];
    let opening_to_exits = [seal.clone(), escort.clone()];
    let no_next = [];
    let beats = [
        GraphBeat {
            id: &room,
            next: &room_to_opening,
            terminal: false,
        },
        GraphBeat {
            id: &opening,
            next: &opening_to_exits,
            terminal: false,
        },
        GraphBeat {
            id: &seal,
            next: &no_next,
            terminal: true,
        },
        GraphBeat {
            id: &escort,
            next: &no_next,
            terminal: false,
        },
    ];
    let alternatives = [
        GraphAlternative {
            id: &ask,
            owner: &room,
            entry: &opening,
            exit: &seal,
            disclosure_fact: Some(&fixture.source),
        },
        GraphAlternative {
            id: &escort_choice,
            owner: &room,
            entry: &opening,
            exit: &escort,
            disclosure_fact: Some(&fixture.source),
        },
    ];
    let facts = [fixture.source];
    let graph = GraphView {
        start: &room,
        beats: &beats,
        facts: &facts,
        alternatives: &alternatives,
    };
    assert!(matches!(
        validate_graph(&graph, graph_limits()),
        Err(GraphError::AlternativeExitNotTerminal { .. })
    ));
    assert!(matches!(
        validate_graph(
            &graph,
            GraphLimits {
                max_work: 0,
                ..graph_limits()
            }
        ),
        Err(GraphError::LimitExceeded(_))
    ));

    let dangling = [
        GraphBeat {
            id: &room,
            next: &room_to_opening,
            terminal: false,
        },
        GraphBeat {
            id: &opening,
            next: &opening_to_exits,
            terminal: false,
        },
        GraphBeat {
            id: &seal,
            next: &no_next,
            terminal: true,
        },
    ];
    let dangling_graph = GraphView {
        start: &room,
        beats: &dangling,
        facts: &facts,
        alternatives: &alternatives,
    };
    assert!(matches!(
        validate_graph(&dangling_graph, graph_limits()),
        Err(GraphError::MissingEdge { .. })
    ));

    let missing_fact = [];
    let missing_fact_graph = GraphView {
        start: &room,
        beats: &beats,
        facts: &missing_fact,
        alternatives: &alternatives,
    };
    assert!(matches!(
        validate_graph(&missing_fact_graph, graph_limits()),
        Err(GraphError::MissingDisclosureFact { .. })
    ));

    let missing_owner = [GraphAlternative {
        id: &ask,
        owner: &thread,
        entry: &opening,
        exit: &seal,
        disclosure_fact: Some(&fixture.source),
    }];
    let missing_owner_graph = GraphView {
        start: &room,
        beats: &beats,
        facts: &facts,
        alternatives: &missing_owner,
    };
    assert!(matches!(
        validate_graph(&missing_owner_graph, graph_limits()),
        Err(GraphError::MissingAlternativeOwner { .. })
    ));
    let missing_entry = [GraphAlternative {
        id: &ask,
        owner: &room,
        entry: &thread,
        exit: &seal,
        disclosure_fact: Some(&fixture.source),
    }];
    let missing_entry_graph = GraphView {
        start: &room,
        beats: &beats,
        facts: &facts,
        alternatives: &missing_entry,
    };
    assert!(matches!(
        validate_graph(&missing_entry_graph, graph_limits()),
        Err(GraphError::MissingAlternativeEntry { .. })
    ));
    let missing_exit = [GraphAlternative {
        id: &ask,
        owner: &room,
        entry: &opening,
        exit: &thread,
        disclosure_fact: Some(&fixture.source),
    }];
    let missing_exit_graph = GraphView {
        start: &room,
        beats: &beats,
        facts: &facts,
        alternatives: &missing_exit,
    };
    assert!(matches!(
        validate_graph(&missing_exit_graph, graph_limits()),
        Err(GraphError::MissingAlternativeExit { .. })
    ));

    let unreachable_exits = [seal.clone()];
    let disconnected = [
        GraphBeat {
            id: &room,
            next: &room_to_opening,
            terminal: false,
        },
        GraphBeat {
            id: &opening,
            next: &unreachable_exits,
            terminal: false,
        },
        GraphBeat {
            id: &seal,
            next: &no_next,
            terminal: true,
        },
        GraphBeat {
            id: &escort,
            next: &no_next,
            terminal: true,
        },
    ];
    let route_unreachable = GraphView {
        start: &room,
        beats: &disconnected,
        facts: &facts,
        alternatives: &alternatives,
    };
    assert!(matches!(
        validate_graph(&route_unreachable, graph_limits()),
        Err(GraphError::UnreachableAlternativeExit { .. })
    ));

    let cycle_edges = [opening.clone()];
    let cyclic = [
        GraphBeat {
            id: &room,
            next: &room_to_opening,
            terminal: false,
        },
        GraphBeat {
            id: &opening,
            next: &cycle_edges,
            terminal: false,
        },
        GraphBeat {
            id: &seal,
            next: &no_next,
            terminal: true,
        },
        GraphBeat {
            id: &escort,
            next: &no_next,
            terminal: true,
        },
    ];
    let cyclic_graph = GraphView {
        start: &room,
        beats: &cyclic,
        facts: &facts,
        alternatives: &alternatives[..0],
    };
    assert!(matches!(
        validate_graph(&cyclic_graph, graph_limits()),
        Err(GraphError::Cycle { .. })
    ));

    let no_terminal = [
        GraphBeat {
            id: &room,
            next: &room_to_opening,
            terminal: false,
        },
        GraphBeat {
            id: &opening,
            next: &opening_to_exits,
            terminal: false,
        },
        GraphBeat {
            id: &seal,
            next: &no_next,
            terminal: false,
        },
        GraphBeat {
            id: &escort,
            next: &no_next,
            terminal: false,
        },
    ];
    let no_terminal_graph = GraphView {
        start: &room,
        beats: &no_terminal,
        facts: &facts,
        alternatives: &alternatives[..0],
    };
    assert!(matches!(
        validate_graph(&no_terminal_graph, graph_limits()),
        Err(GraphError::NoReachableExit)
    ));

    let duplicate = [
        GraphBeat {
            id: &room,
            next: &room_to_opening,
            terminal: false,
        },
        GraphBeat {
            id: &opening,
            next: &opening_to_exits,
            terminal: false,
        },
        GraphBeat {
            id: &seal,
            next: &no_next,
            terminal: true,
        },
        GraphBeat {
            id: &seal,
            next: &no_next,
            terminal: true,
        },
    ];
    let duplicate_graph = GraphView {
        start: &room,
        beats: &duplicate,
        facts: &facts,
        alternatives: &alternatives[..0],
    };
    assert!(matches!(
        validate_graph(&duplicate_graph, graph_limits()),
        Err(GraphError::DuplicateBeat(_))
    ));
    let duplicate_facts = [fixture.source, fixture.source];
    assert!(matches!(
        validate_graph(
            &GraphView {
                start: &room,
                beats: &beats,
                facts: &duplicate_facts,
                alternatives: &alternatives[..0],
            },
            graph_limits()
        ),
        Err(GraphError::DuplicateFact(_))
    ));
    let duplicate_alternatives = [
        GraphAlternative {
            id: &ask,
            owner: &room,
            entry: &opening,
            exit: &seal,
            disclosure_fact: None,
        },
        GraphAlternative {
            id: &ask,
            owner: &room,
            entry: &opening,
            exit: &seal,
            disclosure_fact: None,
        },
    ];
    assert!(matches!(
        validate_graph(
            &GraphView {
                start: &room,
                beats: &beats,
                facts: &facts,
                alternatives: &duplicate_alternatives,
            },
            graph_limits()
        ),
        Err(GraphError::DuplicateAlternative(_))
    ));
    assert!(matches!(
        validate_graph(
            &GraphView {
                start: &room,
                beats: &beats,
                facts: &facts,
                alternatives: &alternatives
            },
            GraphLimits {
                max_beats: 3,
                ..graph_limits()
            }
        ),
        Err(GraphError::LimitExceeded(_))
    ));
}

#[test]
fn runtime_refuses_missing_stale_or_recipient_denied_causes_without_changing_checkpoint() {
    let fixture = Fixture::new();
    fixture.graph().unwrap();
    for case in 0..18 {
        let mut state = fixture.checkpoint.state().clone();
        let mut basis = fixture.checkpoint.basis();
        let mut pins = fixture.checkpoint.pins().clone();
        let mut policy = fixture.policy.clone();
        let mut selection = fixture.ask.clone();
        let mut cause = fixture.source;
        let content_refs = fixture.references.clone();
        let mut limits = Fixture::limits();
        let expected = match case {
            0 => {
                state.facts.clear();
                state.narrative.accepted_facts.clear();
                state.decisions.clear();
                BeatSelectionError::Source
            }
            1 => {
                state.facts[0].audience = AudienceScope::Members(vec![model::member(4)]);
                state.members.push(MembershipLink {
                    member: model::member(4),
                    character: None,
                });
                BeatSelectionError::Recipient
            }
            2 => {
                policy = model::label("stale-story-policy");
                BeatSelectionError::StalePolicy
            }
            3 => {
                basis.revision = model::revision(2, 9);
                BeatSelectionError::Binding(CheckpointError::StaleBasis)
            }
            4 => {
                selection = content("unknown-route");
                BeatSelectionError::Selection
            }
            5 => {
                cause = fact_id(99);
                BeatSelectionError::Source
            }
            6 => {
                pins.content.package_digest = ContentDigest([9; 32]);
                BeatSelectionError::Binding(CheckpointError::ContentMismatch)
            }
            7 => {
                state.facts[0].value = FactValue::ContentEvent {
                    definition: fixture.foreign_event.clone(),
                    subjects: vec![],
                };
                BeatSelectionError::Source
            }
            8 => {
                state.decisions.clear();
                BeatSelectionError::Source
            }
            9 => {
                state.narrative.open_threads.clear();
                BeatSelectionError::Prerequisite
            }
            10 => {
                state.narrative.active_beats = vec![fixture.room.clone()];
                BeatSelectionError::Prerequisite
            }
            11 => BeatSelectionError::UnadmittedContent,
            12 => BeatSelectionError::Capacity,
            13 => {
                pins.rules.catalog_digest = ContentDigest([9; 32]);
                BeatSelectionError::Binding(CheckpointError::RulesMismatch)
            }
            14 => {
                pins.build = BuildIdentity::new(
                    Some("stale-source"),
                    Some("fixture-native-1"),
                    Some("fixture-wasm-1"),
                    Some("fixture-config-1"),
                    Some("fixture-content-1"),
                )
                .unwrap();
                BeatSelectionError::Binding(CheckpointError::BuildMismatch)
            }
            15 => BeatSelectionError::Capacity,
            16 => BeatSelectionError::Capacity,
            17 => BeatSelectionError::Capacity,
            _ => unreachable!(),
        };
        if case == 12 {
            limits.work = 0;
        }
        if case == 15 {
            limits.records = 9;
        }
        if case == 16 {
            limits.alternatives = 1;
        }
        if case == 17 {
            limits.work = 1;
        }
        let inventory = if case == 11 {
            content_refs
                .iter()
                .filter(|reference| **reference != fixture.ask)
                .cloned()
                .collect::<Vec<_>>()
        } else {
            content_refs
        };
        let current = Checkpoint::new(
            CHECKPOINT_SCHEMA,
            fixture.checkpoint.basis(),
            fixture.checkpoint.pins().clone(),
            state,
            fixture.inventory(&fixture.references),
            model::limits(),
        )
        .unwrap();
        let before = current.clone();
        let result = fixture.select(
            &current, &selection, cause, &policy, &pins, basis, &inventory, limits,
        );
        assert_eq!(result, Err(expected), "runtime refusal case {case}");
        assert_eq!(current, before, "runtime refusal case {case}");
    }
}
