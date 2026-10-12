use super::journey::*;
use super::{beat_script::*, model, pack, wire};
use df_model::checkpoint::*;
use df_protocol::common as rpc;
use df_types::{MemberId, OperationId};
const MEMBERS: [[u8; 16]; 2] = [[0x61; 16], [0x62; 16]];
const PACK: &[u8] = include_bytes!("../../../../content/harbor-journey/pack.json");
const TUNING: &[u8] = include_bytes!("fixtures/harbor-journey-tuning-baseline.json");
pub(super) fn input(
    current: &Checkpoint,
    member: MemberId,
    operation: u8,
    entry: &str,
    choices: Vec<(df_types::RevisionLabel, df_types::RevisionLabel)>,
) -> GameInput {
    GameInput::Game(CommandInput {
        basis: current.basis(),
        operation: OperationId::from_bytes(&[operation; 16]).expect("operation"),
        member,
        observed_revision: current.basis().revision,
        command: GameCommand::ProposeAction {
            actor: if member == bootstrap_member().expect("bootstrap") {
                room_entity().expect("room")
            } else {
                player_entity(member, current).expect("joined")
            },
            action: model::content(entry).expect("admitted content"),
            targets: vec![],
            choices,
        },
    })
}
pub(super) fn build(name: &str) -> Vec<(df_types::RevisionLabel, df_types::RevisionLabel)> {
    [
        ("name", hex(name.as_bytes())),
        ("species", "dwarf".to_owned()),
        ("class", "fighter-1".to_owned()),
        ("background", "soldier".to_owned()),
        ("array", "stalwart".to_owned()),
        ("alignment", "lawful-good".to_owned()),
        ("language-1", "dwarvish".to_owned()),
        ("language-2", "elvish".to_owned()),
        ("skill-1", "perception".to_owned()),
        ("skill-2", "survival".to_owned()),
        ("gaming-set", "dice".to_owned()),
        ("equipment", "fighter-a-soldier-a".to_owned()),
        (
            "style-masteries",
            "defense-greatsword-flail-javelin".to_owned(),
        ),
        ("allied-ties", "join-order".to_owned()),
    ]
    .into_iter()
    .map(|(key, value)| {
        (
            model::label(key).expect("key"),
            model::label(&value).expect("choice"),
        )
    })
    .collect()
}
pub(super) fn prepared_story() -> Checkpoint {
    let mut current = initial().expect("initial room");
    for op in [1, 2] {
        current = stage_join(
            &current,
            &input(
                &current,
                bootstrap_member().unwrap(),
                op,
                "join-room",
                vec![],
            ),
        )
        .expect("joined member");
    }
    for (op, bytes, name) in [(3, MEMBERS[0], "Brynn"), (4, MEMBERS[1], "Vale")] {
        current = stage_with_supplier(
            &current,
            &input(
                &current,
                MemberId::from_bytes(&bytes).unwrap(),
                op,
                "create-character",
                build(name),
            ),
            &mut |_| panic!("creation cannot draw"),
        )
        .expect("registered character outcome");
    }
    current
}

fn opening_story() -> Checkpoint {
    let current = prepared_story();
    stage_with_supplier(
        &current,
        &input(
            &current,
            MemberId::from_bytes(&MEMBERS[0]).unwrap(),
            5,
            "begin-story",
            vec![],
        ),
        &mut |_| panic!("opening cannot draw"),
    )
    .expect("registered opening")
}

fn snapshots(approach: &str, defeat: bool) -> Vec<Checkpoint> {
    let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
    let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
    let mut history = vec![prepared_story()];
    {
        let mut perform = |member,
                           operation,
                           entry: &str,
                           choices,
                           dice: &mut dyn FnMut(
            u32,
        ) -> Result<
            u32,
            df_session::submission::RepositoryError,
        >| {
            let current = history.last().unwrap();
            let next = stage_with_supplier(
                current,
                &input(current, member, operation, entry, choices),
                dice,
            )
            .unwrap();
            history.push(next);
        };
        perform(first, 5, "begin-story", vec![], &mut |_| {
            panic!("story never draws")
        });
        perform(first, 6, approach, vec![], &mut |_| {
            panic!("conversation never draws")
        });
        let mut n = 0;
        perform(first, 7, "defend-courier", vec![], &mut |sides| {
            n += 1;
            Ok(if n <= 3 {
                if defeat {
                    if n == 3 { 20 } else { 1 }
                } else if n == 1 {
                    20
                } else {
                    1
                }
            } else if sides == 20 {
                20
            } else {
                sides
            })
        });
    }
    let current = history.last().unwrap();
    let active = current.state().encounters[0].active_turn.unwrap();
    let survivor = current
        .state()
        .members
        .iter()
        .find(|link| link.character == Some(active))
        .unwrap()
        .member;
    {
        let mut perform = |member,
                           operation,
                           entry: &str,
                           choices,
                           dice: &mut dyn FnMut(
            u32,
        ) -> Result<
            u32,
            df_session::submission::RepositoryError,
        >| {
            let current = history.last().unwrap();
            history.push(
                stage_with_supplier(
                    current,
                    &input(current, member, operation, entry, choices),
                    dice,
                )
                .unwrap(),
            );
        };
        perform(survivor, 8, "second-wind", vec![], &mut |sides| Ok(sides));
        perform(survivor, 9, "end-turn", vec![], &mut |sides| Ok(sides));
        if !defeat {
            perform(
                second,
                10,
                "greatsword-attack",
                vec![
                    (
                        model::label("savage-attacker").unwrap(),
                        model::label("no").unwrap(),
                    ),
                    (model::label("graze").unwrap(), model::label("no").unwrap()),
                ],
                &mut |sides| Ok(sides),
            );
        }
    }
    if defeat {
        let mut operation = 20;
        while phase(history.last().unwrap()).unwrap() != rpc::JourneyPhase::Complete {
            let current = history.last().unwrap();
            let actor = current.state().encounters[0].active_turn.unwrap();
            let member = current
                .state()
                .members
                .iter()
                .find(|link| link.character == Some(actor))
                .unwrap()
                .member;
            history.push(
                stage_with_supplier(
                    current,
                    &input(current, member, operation, "end-turn", vec![]),
                    &mut |sides| Ok(sides),
                )
                .unwrap(),
            );
            operation += 1;
            assert!(operation < 40);
        }
    }
    let mut perform = |member, operation, entry: &str, choices| {
        let current = history.last().unwrap();
        history.push(
            stage_with_supplier(
                current,
                &input(current, member, operation, entry, choices),
                &mut |_| panic!("rest/travel dice"),
            )
            .unwrap(),
        );
    };
    perform(first, 50, "short-rest", vec![]);
    if !defeat {
        perform(
            first,
            12,
            "choose-harbor-scene",
            vec![(
                model::label("harbor-destination").unwrap(),
                model::label("harbor-inn").unwrap(),
            )],
        );
    }
    history
}
#[test]
fn canonical_script_reproduces_states_receipts_offers_and_scenes_for_harbor_and_three_seeds() {
    let tables = super::adventure::Tables::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/adventure-tables")
            .as_path(),
    )
    .unwrap();
    let mut packs = vec![pack::load_from_bytes(PACK, TUNING).unwrap()];
    for seed in [0x3c6ef378dd81de97, 0xdaa66d325ccc5aac, 17] {
        let mut generated = super::adventure::generate(
            &tables,
            seed,
            super::adventure::GenerateOptions::default(),
            &Default::default(),
        )
        .unwrap();
        // Compare the E01 sequencing change against its archived mechanics.
        generated.tuning.tactical = None;
        packs.push(
            pack::load_from_bytes(
                &serde_json::to_vec(&generated.pack).unwrap(),
                &serde_json::to_vec(&generated.tuning).unwrap(),
            )
            .unwrap(),
        );
    }
    for loaded in packs {
        canonical().validate(&loaded.pack).unwrap();
        for (approach, defeat) in [
            ("escort-courier", false),
            ("ask-courier", false),
            ("escort-courier", true),
            ("ask-courier", true),
        ] {
            let legacy = pack::with_test_pack(loaded.clone(), || snapshots(approach, defeat));
            let mut scripted = loaded.pack.clone();
            let mut script = canonical().clone();
            // Labels are presentation: the generated chapter words its ending steps itself.
            if let Some(own) = &loaded.pack.beat_script {
                script.labels = own.labels.clone();
            }
            scripted.beat_script = Some(script);
            let scripted = pack::load_from_bytes(
                &serde_json::to_vec(&scripted).unwrap(),
                &serde_json::to_vec(&loaded.tuning).unwrap(),
            )
            .unwrap();
            let actual = pack::with_test_pack(scripted.clone(), || snapshots(approach, defeat));
            for (before, after) in legacy.iter().zip(&actual) {
                assert_eq!(before.state(), after.state());
                let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
                let a = pack::with_test_pack(loaded.clone(), || {
                    (
                        offered(before, member).unwrap(),
                        wire::journey_view(
                            before,
                            df_persistence::local_demo_scope::LocalDemoRole::Player,
                            member,
                        )
                        .unwrap(),
                    )
                });
                let b = pack::with_test_pack(scripted.clone(), || {
                    (
                        offered(after, member).unwrap(),
                        wire::journey_view(
                            after,
                            df_persistence::local_demo_scope::LocalDemoRole::Player,
                            member,
                        )
                        .unwrap(),
                    )
                });
                assert_eq!(a, b);
            }
        }
    }
}
fn codec() -> df_persistence::NativeCodecLimits {
    df_persistence::NativeCodecLimits {
        maximum_document_bytes: 1024 * 1024,
        maximum_allocated_bytes: 2 * 1024 * 1024,
        maximum_collection_items: 4096,
        maximum_text_bytes: 4096,
    }
}
#[test]
fn real_pre_change_checkpoint_bytes_resume_with_exact_pins_and_reencode_unchanged() {
    let loaded = pack::load_from_bytes(
        include_bytes!("fixtures/harbor-journey-pack-integration-baseline.json"),
        TUNING,
    )
    .unwrap();
    pack::with_test_pack(loaded, || {
        for (bytes, sequence, expected) in [
            (
                include_bytes!("fixtures/legacy-beats/opening.bin").as_slice(),
                5,
                rpc::JourneyPhase::Opening,
            ),
            (
                include_bytes!("fixtures/legacy-beats/combat.bin").as_slice(),
                7,
                rpc::JourneyPhase::Combat,
            ),
            (
                include_bytes!("fixtures/legacy-beats/victory.bin").as_slice(),
                8,
                rpc::JourneyPhase::Complete,
            ),
            (
                include_bytes!("fixtures/legacy-beats/arrival.bin").as_slice(),
                9,
                rpc::JourneyPhase::Complete,
            ),
        ] {
            let mut basis = model::basis().unwrap();
            for _ in 0..sequence {
                basis.revision = basis.revision.next_sequence().unwrap();
            }
            let current = df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
                bytes,
                basis,
                &model::pins().unwrap(),
                &model::recovery().unwrap(),
                codec(),
            )
            .unwrap();
            assert_eq!(phase(&current).unwrap(), expected);
            validate_resume(&current).unwrap();
            assert_eq!(
                df_persistence::local_demo_scope::encode_owned_demo_checkpoint(&current, codec())
                    .unwrap(),
                bytes
            );
            let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            assert!(offered(&current, member).is_ok());
            let (entry, choices) = match sequence {
                5 => ("escort-courier", vec![]),
                7 => (
                    "greatsword-attack",
                    vec![
                        (
                            model::label("savage-attacker").unwrap(),
                            model::label("no").unwrap(),
                        ),
                        (model::label("graze").unwrap(), model::label("no").unwrap()),
                    ],
                ),
                _ => ("short-rest", vec![]),
            };
            let continued = stage_with_supplier(
                &current,
                &input(&current, member, 40, entry, choices),
                &mut |sides| Ok(sides),
            )
            .unwrap();
            assert_eq!(continued.pins(), current.pins());
            assert_eq!(
                continued.basis().revision,
                current.basis().revision.next_sequence().unwrap()
            );
            let encoded =
                df_persistence::local_demo_scope::encode_owned_demo_checkpoint(&continued, codec())
                    .unwrap();
            assert_eq!(
                df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
                    &encoded,
                    continued.basis(),
                    continued.pins(),
                    &model::recovery().unwrap(),
                    codec()
                )
                .unwrap(),
                continued
            );
            assert!(
                df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
                    &bytes[..bytes.len() - 1],
                    basis,
                    &model::pins().unwrap(),
                    &model::recovery().unwrap(),
                    codec()
                )
                .is_err()
            );
            let mut wrong = model::pins().unwrap();
            wrong.content.content_digest = ContentDigest([9; 32]);
            assert!(
                df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
                    bytes,
                    basis,
                    &wrong,
                    &model::recovery().unwrap(),
                    codec()
                )
                .is_err()
            );
        }
    });
}
#[test]
fn checkpoint_from_different_script_is_refused_with_typed_revision_error() {
    let legacy = pack::load_from_bytes(PACK, TUNING).unwrap();
    let current = pack::with_test_pack(legacy.clone(), opening_story);
    let mut altered = legacy.pack;
    altered.beat_script = Some(canonical().clone());
    let loaded = pack::load_from_bytes(&serde_json::to_vec(&altered).unwrap(), TUNING).unwrap();
    pack::with_test_pack(loaded, || {
        assert_eq!(
            validate_resume(&current),
            Err(ScriptResumeError::RevisionMismatch)
        )
    });
}
#[test]
fn scripts_refuse_bad_graphs_labels_handlers_scenes_and_rules_wiring_by_beat() {
    for (case, reason) in [
        (0, ScriptErrorReason::MissingTarget),
        (1, ScriptErrorReason::NoExit),
        (2, ScriptErrorReason::Unreachable),
        (3, ScriptErrorReason::FightWiring),
        (4, ScriptErrorReason::RestWiring),
        (5, ScriptErrorReason::MissingLabel),
    ] {
        let mut pack: pack::PackFile = serde_json::from_slice(PACK).unwrap();
        let mut script = canonical().clone();
        match case {
            0 => script.beats[0].transitions[1].target = "missing".into(),
            1 => {
                script.beats[1].offers.clear();
                script.beats[1].transitions.clear();
            }
            2 => {
                script.beats[1]
                    .offers
                    .retain(|o| o.effect != BeatEffect::EscortCourier);
                script.beats[1].transitions.retain(|t| {
                    t.trigger
                        != BeatTrigger::Offer {
                            offer: "escort-courier".into(),
                        }
                });
            }
            3 => script.beats[4]
                .transitions
                .retain(|t| t.trigger != BeatTrigger::FightLost),
            4 => script.beats[5].transitions[0].target = "opening".into(),
            _ => script.beats[0].offers[0].label_slot = "missing".into(),
        }
        pack.beat_script = Some(script);
        let error = pack::load_from_bytes(&serde_json::to_vec(&pack).unwrap(), TUNING).unwrap_err();
        assert!(
            matches!(error,pack::PackError::Script(ScriptError {reason:actual,..}) if actual==reason),
            "case {case}: {error:?}"
        );
    }
    for (field, value, reason) in [
        ("kind", "teleport", ScriptErrorReason::UnknownHandler),
        ("scene", "unknown", ScriptErrorReason::UnknownScene),
    ] {
        let mut value_json: serde_json::Value = serde_json::from_slice(PACK).unwrap();
        value_json["beat_script"] = serde_json::to_value(canonical()).unwrap();
        value_json["beat_script"]["beats"][1][field] = value.into();
        let error =
            pack::load_from_bytes(&serde_json::to_vec(&value_json).unwrap(), TUNING).unwrap_err();
        assert_eq!(
            error,
            pack::PackError::Script(ScriptError {
                beat: "opening".into(),
                reason
            })
        );
    }
}

#[test]
fn impossible_outcome_only_fights_rest_without_action_duplicate_effects_and_wrong_handlers_refuse_at_load()
 {
    for case in 0..13 {
        let mut content: pack::PackFile = serde_json::from_slice(PACK).unwrap();
        let mut script = canonical().clone();
        match case {
            0 => {
                let fight = &mut script.beats[4];
                fight.offers.clear();
                fight
                    .transitions
                    .retain(|t| !matches!(t.trigger, BeatTrigger::Offer { .. }));
            }
            1 => {
                script.beats[5]
                    .offers
                    .retain(|o| o.effect != BeatEffect::ShortRest);
            }
            2 => {
                let mut offer = script.beats[0].offers[0].clone();
                offer.id = "duplicate-create".into();
                script.beats[0].offers.push(offer);
                script.beats[0].transitions.push(BeatTransition {
                    trigger: BeatTrigger::Offer {
                        offer: "duplicate-create".into(),
                    },
                    target: "room".into(),
                });
            }
            3 => {
                script.beats[2].kind = BeatKind::Gather;
            }
            4 => {
                let mut beat = script.beats[2].clone();
                beat.id = "overlapping".into();
                script.beats.push(beat);
            }
            5 => script.beats[0].state.victory = Some(true),
            6 => script.beats[2].state.rested = Some(true),
            7 => script.beats[8].offers.clear(),
            8 => script.beats[9].state.rested = Some(false),
            9 => script.beats[9].state.victory = Some(false),
            10 => script.terminal.clear(),
            // The inn concludes the chapter only by leading on to its ending steps.
            11 => {
                script.beats[9]
                    .offers
                    .retain(|o| o.effect != BeatEffect::Resolve);
                script.beats[9].transitions.retain(|t| {
                    t.trigger
                        != BeatTrigger::Offer {
                            offer: "resolve-errand".into(),
                        }
                });
            }
            _ => script.beats[6].state.rested = Some(false),
        }
        content.beat_script = Some(script);
        assert!(
            matches!(
                pack::load_from_bytes(&serde_json::to_vec(&content).unwrap(), TUNING),
                Err(pack::PackError::Script(_))
            ),
            "case {case}"
        );
    }
}
