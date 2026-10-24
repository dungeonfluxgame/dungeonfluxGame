use super::journey::*;
use super::{beat_script::*, model, pack, wire};
use df_model::checkpoint::*;
use df_protocol::common as rpc;
use df_types::{MemberId, OperationId};
const MEMBERS: [[u8; 16]; 2] = [[0x61; 16], [0x62; 16]];
const PACK: &[u8] = include_bytes!("../../../../content/harbor-journey/pack.json");
const TUNING: &[u8] = include_bytes!("fixtures/harbor-journey-tuning-baseline.json");
// Exact 62105b7a pack bytes: a41f5c06 and 613839eb deliberately changed the live
// presentation, but those edits cannot change the content pin of an archived checkpoint.
const LEGACY_PACK: &[u8] = include_bytes!("fixtures/legacy-beats/pack.json");
const SHOP_PACK: &[u8] = include_bytes!("../../../../content/merchant-shop/pack.json");
const SHOP_TUNING: &[u8] = include_bytes!("../../../../content/merchant-shop/tuning.json");
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
        let mut choices = build(name);
        // Authored scripts require the exact offered choice (62105b7a); these two
        // scene-projection tests began using the shop's authored script in 43f6fe0c.
        if !with_script(|script| script.is_canonical()) {
            choices.push((
                model::label(super::beat_script_runtime::CHOICE_GROUP).unwrap(),
                model::label("create-character").unwrap(),
            ));
        }
        current = stage_with_supplier(
            &current,
            &input(
                &current,
                MemberId::from_bytes(&bytes).unwrap(),
                op,
                "create-character",
                choices,
            ),
            &mut |_| panic!("creation cannot draw"),
        )
        .unwrap_or_else(|error| {
            panic!(
                "registered character outcome: {error:?}, step={:?}",
                super::refusal_detail::take()
            )
        });
    }
    current
}

fn opening_story() -> Checkpoint {
    let current = prepared_story();
    let choices = if with_script(|script| script.is_canonical()) {
        vec![]
    } else {
        vec![(
            model::label(super::beat_script_runtime::CHOICE_GROUP).unwrap(),
            model::label("begin-story").unwrap(),
        )]
    };
    stage_with_supplier(
        &current,
        &input(
            &current,
            MemberId::from_bytes(&MEMBERS[0]).unwrap(),
            5,
            "begin-story",
            choices,
        ),
        &mut |_| panic!("opening cannot draw"),
    )
    .expect("registered opening")
}

#[test]
fn canonical_source_scene_key_survives_wire_projection_when_room_and_opening_share_title_and_art() {
    let loaded = pack::load_from_bytes(SHOP_PACK, SHOP_TUNING).unwrap();
    pack::with_test_pack(loaded, || {
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let room = prepared_story();
        let opening = opening_story();
        let room_view = wire::journey_view(
            &room,
            df_persistence::local_demo_scope::LocalDemoRole::Player,
            member,
        )
        .unwrap();
        let opening_view = wire::journey_view(
            &opening,
            df_persistence::local_demo_scope::LocalDemoRole::Player,
            member,
        )
        .unwrap();
        let scene = |view: rpc::ViewMessage| match view.audience.unwrap() {
            rpc::view_message::Audience::Player(player) => player.scene.unwrap(),
            _ => panic!("player view"),
        };
        let room_scene = scene(room_view);
        let opening_scene = scene(opening_view);
        assert_eq!(room_scene.source_scene_key, "room");
        assert_eq!(opening_scene.source_scene_key, "opening");
        assert!(room_scene.source_scene_variant.is_empty());
        assert!(opening_scene.source_scene_variant.is_empty());
        assert_eq!(room_scene.title, opening_scene.title);
        assert_eq!(room_scene.scene_asset, opening_scene.scene_asset);
    });
}

#[test]
fn wire_projection_carries_the_exact_authored_scene_variant() {
    const ROOM_VARIANT: &str = "room-authored-revision";
    const OPENING_VARIANT: &str = "opening-authored-revision";
    let mut loaded = pack::load_from_bytes(SHOP_PACK, SHOP_TUNING).unwrap();
    let mut room_variant = loaded
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == pack::SceneKey::Room)
        .unwrap()
        .clone();
    room_variant.source_variant_id = Some(ROOM_VARIANT.to_owned());
    let mut opening_variant = loaded
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == pack::SceneKey::Opening)
        .unwrap()
        .clone();
    opening_variant.source_variant_id = Some(OPENING_VARIANT.to_owned());
    let script = loaded.pack.beat_script.as_mut().unwrap();
    script.scenes.insert(ROOM_VARIANT.to_owned(), room_variant);
    script
        .scenes
        .insert(OPENING_VARIANT.to_owned(), opening_variant);
    script
        .beats
        .iter_mut()
        .find(|beat| beat.id == "gather")
        .unwrap()
        .scene_variant = Some(ROOM_VARIANT.to_owned());
    script
        .beats
        .iter_mut()
        .find(|beat| beat.id == "freeplay")
        .unwrap()
        .scene_variant = Some(OPENING_VARIANT.to_owned());
    let destination_beat = script.find("gather").unwrap().clone();

    pack::with_test_pack(loaded, || {
        let destination = super::beat_script_runtime::destination_scene(&destination_beat).unwrap();
        assert_eq!(destination.source_scene_variant, ROOM_VARIANT);
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let room = prepared_story();
        let opening = opening_story();
        let room_view = wire::journey_view(
            &room,
            df_persistence::local_demo_scope::LocalDemoRole::Player,
            member,
        )
        .unwrap();
        let opening_view = wire::journey_view(
            &opening,
            df_persistence::local_demo_scope::LocalDemoRole::Player,
            member,
        )
        .unwrap();
        let scene = |view: rpc::ViewMessage| match view.audience.unwrap() {
            rpc::view_message::Audience::Player(player) => player.scene.unwrap(),
            _ => panic!("player view"),
        };
        assert_eq!(scene(room_view).source_scene_variant, ROOM_VARIANT);
        assert_eq!(scene(opening_view).source_scene_variant, OPENING_VARIANT);
    });
}

#[test]
fn canonical_skill_modifiers_and_passive_perception_follow_srd_5_2_1_pages_6_22_183_187_189_for_both_heroes()
 {
    // SRD 5.2.1 p. 6: add the relevant ability modifier and the Proficiency Bonus when proficient.
    // p. 22: "Passive Perception = 10 + Wisdom (Perception) check modifier." Search is p. 187,
    // Study p. 189, and Hide's Dexterity (Stealth) check is p. 183.
    let current = prepared_story();
    let skills = [
        ("perception", "wisdom", true),
        ("investigation", "intelligence", false),
        ("insight", "wisdom", false),
        ("survival", "wisdom", true),
        ("medicine", "wisdom", false),
        ("arcana", "intelligence", false),
        ("history", "intelligence", false),
        ("nature", "intelligence", false),
        ("religion", "intelligence", false),
        ("stealth", "dexterity", false),
    ];
    let proficiency_bonus = pack::with_creation(|creation| creation.proficiency_bonus);
    for member_bytes in MEMBERS {
        let member = MemberId::from_bytes(&member_bytes).unwrap();
        let actor = player_entity(member, &current).unwrap();
        for (skill, ability, proficient) in skills {
            let score = u8::try_from(value(current.state(), actor, ability).unwrap()).unwrap();
            let expected = (
                i8::try_from(df_rules::local_journey::ability_modifier(score)).unwrap(),
                if proficient { proficiency_bonus } else { 0 },
            );
            assert_eq!(
                super::beat_script_runtime::skill_check_modifiers(current.state(), actor, skill)
                    .unwrap(),
                expected,
                "{member_bytes:?} {skill}"
            );
        }
        let (wisdom_modifier, perception_proficiency) =
            super::beat_script_runtime::skill_check_modifiers(current.state(), actor, "perception")
                .unwrap();
        assert_eq!(
            super::beat_script_runtime::passive_perception(current.state(), actor).unwrap(),
            10 + i16::from(wisdom_modifier) + i16::from(perception_proficiency)
        );
    }
}

#[test]
fn canonical_phone_offers_look_around_with_scene_findings_srd_521_pages_187_189() {
    let mut content: pack::PackFile = serde_json::from_slice(PACK).unwrap();
    content
        .scenes
        .iter_mut()
        .find(|scene| scene.key == pack::SceneKey::Opening)
        .unwrap()
        .findings = vec![
        pack::Finding {
            id: "errand-truth".into(),
            sentence: "The warning concerns the sealed map bundle.".into(),
            skill: pack::FindingSkill::Investigation,
            dc: 15,
            private: false,
            consequence: None,
        },
        pack::Finding {
            id: "enemy-position".into(),
            sentence: "The pursuers wait beyond the boardwalk.".into(),
            skill: pack::FindingSkill::Perception,
            dc: 15,
            private: false,
            consequence: None,
        },
    ];
    let loaded = pack::load_from_bytes(&serde_json::to_vec(&content).unwrap(), TUNING).unwrap();
    pack::with_test_pack(loaded, || {
        let current = opening_story();
        assert_eq!(
            super::beat_script_runtime::action_label(
                &super::journey::offer_id(&current, rpc::GameplayActionKind::ExamineHarborSeal),
                rpc::GameplayActionKind::ExamineHarborSeal,
            )
            .unwrap(),
            "Look around"
        );
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let view = wire::journey_view(
            &current,
            df_persistence::local_demo_scope::LocalDemoRole::Player,
            member,
        )
        .unwrap();
        let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
            panic!("player view")
        };
        let offer = player
            .offers
            .into_iter()
            .find(|offer| offer.action_kind == rpc::GameplayActionKind::ExamineHarborSeal as i32)
            .expect("story scenes offer look around");
        assert_eq!(offer.label, "Look around");
        assert_eq!(offer.input_groups.len(), 1);
        assert_eq!(offer.input_groups[0].label, "What catches your eye?");
        assert_eq!(offer.input_groups[0].group_id, "attempt-thing");
        assert_ne!(offer.label, offer.input_groups[0].label);
        assert!(offer.input_groups[0].options.iter().any(|option| {
            option.option_id == "person-courier" && option.label == "Study Finn Alder's manner"
        }));
        // Unbound clues are not visible objects. Existing explicit finding submissions below
        // remain replay-compatible, but their secret ids are no longer exposed in the menu.
        assert!(!offer.input_groups[0].options.iter().any(|option| {
            matches!(option.option_id.as_str(), "errand-truth" | "enemy-position")
        }));
        let checked = stage_with_supplier(
            &current,
            &input(
                &current,
                member,
                0x63,
                "script-choice",
                vec![
                    (
                        model::label("finding").unwrap(),
                        model::label("errand-truth").unwrap(),
                    ),
                    (
                        model::label("attempt-skill").unwrap(),
                        model::label("investigation").unwrap(),
                    ),
                ],
            ),
            &mut |_| Ok(20),
        )
        .unwrap();
        let receipt = super::journey::accepted(checked.state().decisions.last().unwrap()).unwrap();
        let check = receipt.check.expect("Look around rolls on the server");
        assert_eq!(check.die, 20);
        assert_eq!(
            check.source_revision,
            "srd521-p6:opening:investigation:finding-errand-truth"
        );
        assert_eq!(receipt.phase, rpc::JourneyPhase::Opening as i32);
    });
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
            .unwrap_or_else(|error| {
                panic!(
                    "snapshot operation={operation}, entry={entry}, approach={approach}, defeat={defeat}, step={:?}: {error:?}",
                    super::refusal_detail::take()
                )
            });
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
                .unwrap_or_else(|error| {
                    panic!(
                        "snapshot {entry}: {error:?}, step={:?}",
                        super::refusal_detail::take()
                    )
                }),
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
    let current = history.last().unwrap();
    history.push(
        stage_with_supplier(
            current,
            &input(current, first, 50, "short-rest", vec![]),
            &mut |_| panic!("rest cannot draw before a die is chosen"),
        )
        .unwrap(),
    );
    // b44a9198 requires every eligible hero to choose whether to spend or keep a die.
    for (index, member) in [first, second].into_iter().enumerate() {
        let current = history.last().unwrap();
        if hit_point_die_offered(current.state(), player_entity(member, current).unwrap()).unwrap()
        {
            history.push(
                stage_with_supplier(
                    current,
                    &input(
                        current,
                        member,
                        51 + index as u8,
                        "short-rest",
                        vec![(
                            model::label("hit-point-die").unwrap(),
                            model::label("keep").unwrap(),
                        )],
                    ),
                    &mut |_| panic!("keeping a die cannot draw"),
                )
                .unwrap(),
            );
        }
    }
    assert!(short_rest_completed(history.last().unwrap().state()));
    if !defeat {
        let current = history.last().unwrap();
        history.push(
            stage_with_supplier(
                current,
                &input(
                    current,
                    first,
                    12,
                    "choose-harbor-scene",
                    vec![(
                        model::label("harbor-destination").unwrap(),
                        model::label("harbor-inn").unwrap(),
                    )],
                ),
                &mut |_| panic!("travel cannot draw"),
            )
            .unwrap(),
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
                        .unwrap_or_else(|error| {
                            panic!(
                                "player view failed for {approach}, defeat={defeat}, phase={:?}, error={error:?}, step={}",
                                phase(before).unwrap(),
                                crate::gameplay::refusal_detail::step_or_unknown()
                            )
                        }),
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
fn stored_game_without_hit_point_dice_loads_with_one_unspent_d10_for_short_rest_srd_p47() {
    let loaded = pack::load_from_bytes(LEGACY_PACK, TUNING).unwrap();
    pack::with_test_pack(loaded, || {
        let mut basis = model::basis().unwrap();
        for _ in 0..8 {
            basis.revision = basis.revision.next_sequence().unwrap();
        }
        let stored = df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
            include_bytes!("fixtures/legacy-beats/victory.bin"),
            basis,
            &model::pins().unwrap(),
            &model::recovery().unwrap(),
            codec(),
        )
        .unwrap();
        for character in &stored.state().characters {
            assert_eq!(
                value(stored.state(), character.entity, "hit-point-dice").unwrap(),
                1
            );
        }
    });
}

#[test]
fn real_pre_change_checkpoint_bytes_resume_with_exact_pins_and_reencode_unchanged() {
    let loaded = pack::load_from_bytes(LEGACY_PACK, TUNING).unwrap();
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
                // The stored heroes are untouched, so a short rest would
                // regain nothing and the machine refuses it; the resumed
                // game continues to the inn, and from the inn to its
                // resolution.
                8 => (
                    "choose-harbor-scene",
                    vec![(
                        model::label("harbor-destination").unwrap(),
                        model::label("harbor-inn").unwrap(),
                    )],
                ),
                _ => ("resolve-errand", vec![]),
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

#[test]
fn look_around_phone_offer_inspects_stock_with_price_and_resolves_its_real_target() {
    let mut loaded = pack::load_from_bytes(SHOP_PACK, SHOP_TUNING).unwrap();
    let model: super::adventure::scene_model::SceneModel = serde_json::from_slice(include_bytes!(
        "../../../../content/merchant-shop/scene-model.json"
    ))
    .unwrap();
    for scene in &mut loaded.pack.scenes {
        if let Some(setting) = scene.setting.as_mut() {
            setting.model = Some(model.clone());
        }
    }
    pack::with_test_pack(loaded, || {
        let current = opening_story();
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let view = wire::journey_view(
            &current,
            df_persistence::local_demo_scope::LocalDemoRole::Player,
            member,
        )
        .unwrap();
        let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
            panic!("player view")
        };
        let offer = player
            .offers
            .iter()
            .find(|offer| {
                offer.action_kind == rpc::GameplayActionKind::ExamineHarborSeal as i32
                    && offer
                        .input_groups
                        .iter()
                        .any(|group| group.group_id == "attempt-thing")
            })
            .expect("typed observation offer");
        let group = &offer.input_groups[0];
        assert_eq!(group.label, "What catches your eye?");
        assert_eq!(offer.label, "Look around");
        assert!(group.options.len() > 8);
        let item = group
            .options
            .iter()
            .find(|option| option.option_id == "offer-warm-cloak-wool")
            .unwrap();
        assert_eq!(item.label, "Inspect the warm wool cloak");
        assert!(item.description.contains("3 gp"));
        let request = rpc::SubmitActionRequest {
            session_id: Some(rpc::SessionId {
                value: Some(current.basis().session.as_bytes().to_vec()),
            }),
            run_id: Some(rpc::RunId {
                value: Some(current.basis().run.as_bytes().to_vec()),
            }),
            operation_id: Some(rpc::OperationId {
                value: Some(vec![0x76; 16]),
            }),
            observed_revision: Some(wire::revision(current.basis().revision)),
            offer_id: offer.offer_id.clone(),
            action_kind: offer.action_kind,
            action_choices: vec![rpc::JourneyChoice {
                group_id: group.group_id.clone(),
                option_id: item.option_id.clone(),
            }],
            ..Default::default()
        };
        let input = wire::journey_input(&request, member, &current).unwrap();
        let inspected = stage_with_supplier(&current, &input, &mut |_| Ok(20)).unwrap();
        let receipt =
            super::journey::accepted(inspected.state().decisions.last().unwrap()).unwrap();
        let check = receipt.check.unwrap();
        assert!(
            check
                .source_revision
                .ends_with(":investigation:target-offer-warm-cloak-wool"),
            "{}",
            check.source_revision
        );
        let scene = super::beat_script_runtime::scene(current.state()).unwrap();
        let observation = super::investigation::observation(&scene, &check).unwrap();
        assert!(observation.finding.unwrap().contains("3 gp"));
        assert_eq!(
            super::journey::phase(&inspected).unwrap(),
            rpc::JourneyPhase::Opening
        );
    });
}
