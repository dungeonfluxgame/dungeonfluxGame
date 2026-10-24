use super::*;

#[path = "../gameplay_browser/hero_creation_choices.rs"]
mod hero_creation_choices;

#[test]
fn revision_label_inner_cause_reaches_the_creation_refusal_without_player_text() {
    super::super::refusal_detail::begin_action();
    assert_eq!(
        model::label("private player text"),
        Err(RepositoryError::InvalidCandidate)
    );
    super::super::actor::record_refusal(
        rpc::GameplayActionKind::CreateCharacter as i32,
        "CapabilityUnavailable",
        "refused",
    );
    let lines = super::super::actor::audit_lines_containing(
        "step=model-revision-label outer=model-revision-label cause=RevisionLabel::InvalidCharacter { byte_index: 7 }",
    );
    assert!(!lines.is_empty());
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("private player text"))
    );
}

#[test]
fn full_species_form_accepts_seventeen_choices_and_refuses_eighteen() {
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../content/harbor-journey/pack.json"),
        include_bytes!("../../../../content/harbor-journey/tuning.json"),
    )
    .unwrap();
    pack::with_test_pack(loaded, || {
        let current = joined_room();
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let build = super::tests::build("Maren Coast");
        let mut choices = build
            .iter()
            .filter(|(key, _)| key.as_str() != "name")
            .map(|(key, value)| {
                (
                    key.as_str(),
                    if key.as_str() == "species" {
                        "human"
                    } else {
                        value.as_str()
                    },
                )
            })
            .collect::<Vec<_>>();
        choices.extend([
            ("species-size", "medium"),
            ("human-skill", "arcana"),
            ("versatile", "tough"),
            ("portrait", "portrait-hero-1"),
        ]);
        assert_eq!(choices.len(), 17);
        let mut request = phone_create(&current, 3, "Maren Coast", &choices);
        let input = wire::journey_input(&request, member, &current).unwrap();
        let next = model::HarborEngine
            .decide_admitted(&current, &input, OperationId::from_bytes(&[3; 16]).unwrap())
            .unwrap();
        assert_eq!(
            character_sheet(next.state(), player_entity(member, &next).unwrap())
                .unwrap()
                .maximum_hit_points,
            14
        );
        request
            .character
            .as_mut()
            .unwrap()
            .choices
            .push(rpc::JourneyChoice {
                group_id: "versatile".to_owned(),
                option_id: "tough".to_owned(),
            });
        assert_eq!(
            wire::journey_input(&request, member, &current)
                .unwrap_err()
                .code(),
            tonic::Code::InvalidArgument
        );
    });
}

#[test]
fn duplicate_name_offer_is_rejected_by_the_registered_creation_stage() {
    let mut current = initial().expect("empty room");
    current = stage_join(
        &current,
        &super::tests::input(
            &current,
            bootstrap_member().expect("bootstrap"),
            1,
            "join-room",
            vec![],
        ),
    )
    .expect("real joined member");
    let member = MemberId::from_bytes(&MEMBERS[0]).expect("joined member");
    let mut choices = super::tests::build("Brynn");
    choices.push((
        model::label("name").expect("name offer"),
        model::label(&hex(b"Neris")).expect("second encoded name"),
    ));
    let request = super::tests::input(&current, member, 2, "create-character", choices);
    let before = current.clone();

    assert!(
        stage_with_supplier(&current, &request, &mut |_| {
            panic!("character creation cannot draw")
        })
        .is_err()
    );
    assert_eq!(current, before);
    assert!(current.state().characters.is_empty());
}

#[test]
fn creation_rejects_foreign_choice_member_actor_and_basis_at_stage_boundary() {
    let mut current = initial().expect("empty room");
    current = stage_join(
        &current,
        &super::tests::input(
            &current,
            bootstrap_member().expect("bootstrap"),
            1,
            "join-room",
            vec![],
        ),
    )
    .expect("real joined member");
    let member = MemberId::from_bytes(&MEMBERS[0]).expect("joined member");
    let valid_choices = super::tests::build("Brynn");
    let before = current.clone();

    let mut foreign_choice = valid_choices.clone();
    let class = foreign_choice
        .iter_mut()
        .find(|(offer, _)| offer.as_str() == "class")
        .expect("class choice");
    class.1 = model::label("fighter-foreign").expect("foreign class");
    let wrong_choice = super::tests::input(&current, member, 2, "create-character", foreign_choice);
    assert!(
        stage_with_supplier(&current, &wrong_choice, &mut |_| {
            panic!("character creation cannot draw")
        })
        .is_err()
    );
    assert_eq!(current, before);

    let mut foreign_member = super::tests::input(
        &current,
        member,
        3,
        "create-character",
        valid_choices.clone(),
    );
    if let GameInput::Game(command) = &mut foreign_member {
        command.member = MemberId::from_bytes(&MEMBERS[1]).expect("foreign member");
    }
    assert!(
        stage_with_supplier(&current, &foreign_member, &mut |_| {
            panic!("character creation cannot draw")
        })
        .is_err()
    );
    assert_eq!(current, before);

    let mut wrong_actor = super::tests::input(
        &current,
        member,
        4,
        "create-character",
        valid_choices.clone(),
    );
    if let GameInput::Game(command) = &mut wrong_actor
        && let GameCommand::ProposeAction { actor, .. } = &mut command.command
    {
        *actor = room_entity().expect("room actor");
    }
    assert!(
        stage_with_supplier(&current, &wrong_actor, &mut |_| {
            panic!("character creation cannot draw")
        })
        .is_err()
    );
    assert_eq!(current, before);

    let mut stale_basis =
        super::tests::input(&current, member, 5, "create-character", valid_choices);
    if let GameInput::Game(command) = &mut stale_basis {
        command.basis.revision = command
            .basis
            .revision
            .next_sequence()
            .expect("next revision");
    }
    assert!(
        stage_with_supplier(&current, &stale_basis, &mut |_| {
            panic!("character creation cannot draw")
        })
        .is_err()
    );
    assert_eq!(current, before);
}

#[test]
fn accepted_creation_keeps_canonical_choices_and_source_backed_inventory() {
    let mut current = initial().expect("empty room");
    current = stage_join(
        &current,
        &super::tests::input(
            &current,
            bootstrap_member().expect("bootstrap"),
            1,
            "join-room",
            vec![],
        ),
    )
    .expect("real joined member");
    let member = MemberId::from_bytes(&MEMBERS[0]).expect("joined member");
    let request = super::tests::input(
        &current,
        member,
        2,
        "create-character",
        super::tests::build("Brynn"),
    );

    let created = stage_with_supplier(&current, &request, &mut |_| {
        panic!("character creation cannot draw")
    })
    .expect("source-qualified legal build");
    let entity = player_entity(member, &created).expect("member character");
    let character = created
        .state()
        .characters
        .iter()
        .find(|character| character.entity == entity)
        .expect("accepted character");
    assert_eq!(character.owner, member);
    assert_eq!(
        character.choices.len(),
        df_rules::local_journey::BUILD_GROUPS.len() + 1
    );
    assert_eq!(
        character
            .choices
            .iter()
            .filter(|choice| choice.offer.as_str() == "name")
            .count(),
        1
    );
    assert!(
        character.choices.iter().any(|choice| {
            choice.offer.as_str() == "name" && choice.selected.as_str() == "Brynn"
        })
    );
    assert_eq!(value(created.state(), entity, "hit-points"), Ok(13));
    assert_eq!(value(created.state(), entity, "armor-class"), Ok(17));
    assert_eq!(value(created.state(), entity, "strength"), Ok(17));
    assert_eq!(value(created.state(), entity, "constitution"), Ok(15));
    assert_eq!(value(created.state(), entity, "second-wind"), Ok(2));
    assert!(
        character
            .choices
            .iter()
            .all(|choice| choice.participant == member && choice.source == rule().expect("source"))
    );

    let inventory = created
        .state()
        .inventory
        .iter()
        .filter(|item| item.owner == entity)
        .collect::<Vec<_>>();
    assert_eq!(inventory.len(), 13);
    assert!(
        inventory
            .iter()
            .all(|item| item.source == rule().expect("source"))
    );
    assert!(inventory.iter().all(|item| {
        item.attunement_owner.is_none()
            && item.origin.package.as_str() == "harbor-investigation-demo-profile6-npc-1"
            && created.state().entities.iter().any(|world| {
                world.id == item.item
                    && world.definition == item.origin
                    && world.identity_revision.as_str() == "source-starting-equipment-1"
            })
    }));
    let quantity_for = |origin: &str| {
        inventory
            .iter()
            .find(|item| item.origin.entry.as_str() == origin)
            .map(|item| item.quantity)
    };
    assert_eq!(quantity_for("javelin"), Some(8));
    assert_eq!(quantity_for("spear"), Some(1));
    assert_eq!(quantity_for("shortbow"), Some(1));
    assert_eq!(quantity_for("arrow"), Some(20));
    assert_eq!(quantity_for("chain-mail"), Some(1));
    assert_eq!(quantity_for("greatsword"), Some(1));
    assert_eq!(quantity_for("flail"), Some(1));
    assert_eq!(quantity_for("dungeoneer-pack"), Some(1));
    assert_eq!(quantity_for("gaming-set"), Some(1));
    assert_eq!(quantity_for("healers-kit"), Some(1));
    assert_eq!(quantity_for("quiver"), Some(1));
    assert_eq!(quantity_for("travelers-clothes"), Some(1));
    assert_eq!(quantity_for("gold-piece"), Some(18));
}

// The live pack with the tactical section, as the server loads it.
fn live_pack<T>(run: impl FnOnce() -> T) -> T {
    let mut tuning: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../content/harbor-journey/tuning.json"
    ))
    .unwrap();
    tuning["tactical"] =
        serde_json::json!({"enemies":[{"srd_id":"bandit","count":3,"ammunition":20}]});
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../content/harbor-journey/pack.json"),
        &serde_json::to_vec(&tuning).unwrap(),
    )
    .unwrap();
    pack::with_test_pack(loaded, run)
}
fn phone_create(
    current: &Checkpoint,
    operation: u8,
    name: &str,
    choices: &[(&str, &str)],
) -> rpc::SubmitActionRequest {
    let basis = current.basis();
    rpc::SubmitActionRequest {
        session_id: Some(rpc::SessionId {
            value: Some(basis.session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(basis.run.as_bytes().to_vec()),
        }),
        operation_id: Some(rpc::OperationId {
            value: Some(vec![operation; 16]),
        }),
        observed_revision: Some(wire::revision(basis.revision)),
        offer_id: offer_id(current, rpc::GameplayActionKind::CreateCharacter),
        action_kind: rpc::GameplayActionKind::CreateCharacter as i32,
        character: Some(rpc::CharacterSelection {
            name: name.to_owned(),
            choices: choices
                .iter()
                .map(|(group, option)| rpc::JourneyChoice {
                    group_id: (*group).to_owned(),
                    option_id: (*option).to_owned(),
                })
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    }
}
const PHONE_SHARED: [(&str, &str); 10] = [
    ("species", "dwarf"),
    ("class", "fighter-1"),
    ("background", "soldier"),
    ("language-1", "elvish"),
    ("language-2", "orc"),
    ("skill-1", "acrobatics"),
    ("skill-2", "survival"),
    ("gaming-set", "playing-cards"),
    ("allied-ties", "join-order"),
    ("alignment", "chaotic-good"),
];
fn joined_room() -> Checkpoint {
    let mut current = initial().expect("empty room");
    for operation in [1, 2] {
        current = stage_join(
            &current,
            &super::tests::input(
                &current,
                bootstrap_member().expect("bootstrap"),
                operation,
                "join-room",
                vec![],
            ),
        )
        .expect("joined");
    }
    current
}
/// Creates one hero through the admitted entry the live server uses (`journey_input`, then
/// `HarborEngine::decide_admitted`) and checks that its receipt fits the live receipt bound.
fn seat_hero(
    current: &Checkpoint,
    seat: [u8; 16],
    operation: u8,
    name: &str,
    package: &str,
    portrait: &str,
) -> Checkpoint {
    let member = MemberId::from_bytes(&seat).expect("member");
    let mut choices = PHONE_SHARED.to_vec();
    choices.push(("build-package", package));
    choices.push(("portrait", portrait));
    let request = phone_create(current, operation, name, &choices);
    let input = wire::journey_input(&request, member, current).expect("the phone request converts");
    let operation_id = OperationId::from_bytes(&[operation; 16]).expect("operation");
    let mut engine = model::HarborEngine;
    let next = engine
        .decide_admitted(current, &input, operation_id)
        .unwrap_or_else(|error| panic!("{package} seat refused: {error:?}"));
    let decision = next.state().decisions.last().expect("creation decision");
    df_session::submission::DecisionReceipt::new(
        next.basis(),
        decision.clone(),
        df_persistence::local_demo_scope::RECEIPT_RETAINED_BYTES_MAX,
    )
    .expect("the creation receipt fits the live receipt bound");
    next
}
#[test]
fn creation_packages_stage_through_the_live_entry_for_each_seat_and_match_their_sheets() {
    live_pack(|| {
        for packages in [
            ["melee-stalwart", "archer"],
            ["archer", "melee-stalwart"],
            ["archer", "archer"],
        ] {
            let mut current = joined_room();
            for (index, package) in packages.into_iter().enumerate() {
                let portrait = if package == "archer" {
                    "portrait-hero-3"
                } else {
                    "portrait-hero-1"
                };
                let name = ["Brakka", "Sylvie"][index];
                current = seat_hero(
                    &current,
                    MEMBERS[index],
                    3 + index as u8,
                    name,
                    package,
                    portrait,
                );
                let entity =
                    player_entity(MemberId::from_bytes(&MEMBERS[index]).unwrap(), &current)
                        .expect("seated hero");
                let (armor, hit_points) = if package == "archer" {
                    (15, 13)
                } else {
                    (17, 13)
                };
                assert_eq!(value(current.state(), entity, "armor-class"), Ok(armor));
                assert_eq!(value(current.state(), entity, "hit-points"), Ok(hit_points));
            }
        }
    });
}

#[test]
fn archer_creation_as_second_player_accepts_the_complete_phone_selection() {
    live_pack(|| {
        let mut current = joined_room();
        current = seat_hero(
            &current,
            MEMBERS[0],
            3,
            "Brakka",
            "melee-stalwart",
            "portrait-hero-1",
        );
        let second = MemberId::from_bytes(&MEMBERS[1]).expect("second member");
        let choices = [
            ("species", "dwarf"),
            ("class", "fighter-1"),
            ("background", "soldier"),
            ("language-1", "elvish"),
            ("language-2", "orc"),
            ("skill-1", "acrobatics"),
            ("skill-2", "insight"),
            ("gaming-set", "playing-cards"),
            ("allied-ties", "join-order"),
            ("alignment", "neutral-good"),
            ("build-package", "archer"),
            ("portrait", "portrait-hero-3"),
        ];
        let request = phone_create(&current, 4, "Sigrun", &choices);
        let input = wire::journey_input(&request, second, &current)
            .expect("the complete archer phone request converts");
        let mut engine = model::HarborEngine;
        let next = engine
            .decide_admitted(
                &current,
                &input,
                OperationId::from_bytes(&[4; 16]).expect("operation"),
            )
            .expect("second player archer creation is accepted");
        let receipt = df_session::submission::DecisionReceipt::new(
            next.basis(),
            next.state().decisions.last().unwrap().clone(),
            usize::MAX,
        )
        .expect("second player archer receipt can be measured");
        assert!(receipt.retained_bytes().expect("receipt size") > 4096);
        assert!(
            receipt.retained_bytes().expect("receipt size")
                <= df_persistence::local_demo_scope::RECEIPT_RETAINED_BYTES_MAX
        );
        let entity = player_entity(second, &next).expect("second hero");
        assert_eq!(value(next.state(), entity, "armor-class"), Ok(15));
        assert!(
            next.state()
                .inventory
                .iter()
                .any(|item| item.owner == entity && item.origin.entry.as_str() == "longbow")
        );
    });
}
#[test]
fn legacy_portrait_metadata_does_not_constrain_the_mechanical_package() {
    live_pack(|| {
        let with = |package: &'static str, portrait: &'static str| {
            let mut choices = PHONE_SHARED.to_vec();
            choices.push(("build-package", package));
            choices.push(("portrait", portrait));
            pack::validate_character("Sylvie", &choices).is_ok()
        };
        assert!(with("archer", "portrait-hero-3"));
        assert!(with("archer", "portrait-hero-1"));
        assert!(with("melee-stalwart", "portrait-hero-1"));
        assert!(with("melee-stalwart", "portrait-hero-2"));
        assert!(with("melee-stalwart", "portrait-hero-3"));
    });
}
#[test]
fn each_offered_package_states_the_ability_scores_the_server_gives_its_hero() {
    live_pack(|| {
        let offer = wire::creation_offer().expect("creation offer");
        let group = |id: &str| {
            offer
                .groups
                .iter()
                .find(|group| group.group_id == id)
                .expect("offered group")
        };
        let mut seen = std::collections::BTreeSet::new();
        for package in &group("build-package").options {
            let mut choices = PHONE_SHARED.to_vec();
            choices.push(("build-package", package.option_id.as_str()));
            let [
                strength,
                dexterity,
                constitution,
                intelligence,
                wisdom,
                charisma,
            ] = pack::validate_character("Sylvie", &choices)
                .expect("offered package is accepted")
                .abilities;
            assert_eq!(
                package.description,
                format!(
                    "STR {strength} · DEX {dexterity} · CON {constitution} · INT {intelligence} · WIS {wisdom} · CHA {charisma}"
                ),
                "{}",
                package.option_id
            );
            // Two packages with the same equipment are told apart by these scores.
            assert!(
                seen.insert(package.description.clone()),
                "{}",
                package.option_id
            );
        }
    });
}
#[test]
fn every_package_completes_creation_without_a_stock_portrait_choice() {
    live_pack(|| {
        let offer = wire::creation_offer().unwrap();
        assert!(
            !offer
                .groups
                .iter()
                .any(|group| group.group_id == "portrait")
        );
        let packages = offer
            .groups
            .iter()
            .find(|group| group.group_id == "build-package")
            .unwrap();
        for package in &packages.options {
            let current = joined_room();
            let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let mut choices = PHONE_SHARED.to_vec();
            choices.push(("build-package", &package.option_id));
            let mut request = phone_create(&current, 3, "Sigrun", &choices);
            let mut identity = super::super::hero_identity::Identity::from_menus(&[])
                .unwrap()
                .to_rpc();
            identity.appearance.as_mut().unwrap().public_description =
                "An emerald-skinned elder with silver horns and a violet coat.".to_owned();
            request.character.as_mut().unwrap().identity = Some(identity);
            let input = creator_input(&current, member, &request);
            let next = model::HarborEngine
                .decide_admitted(&current, &input, OperationId::from_bytes(&[3; 16]).unwrap())
                .unwrap_or_else(|error| {
                    panic!(
                        "lock-in refused: {error:?}; {:?}",
                        df_session::submission::take_refusal_diagnostic()
                    )
                });
            let entity = player_entity(member, &next).unwrap();
            let character = next
                .state()
                .characters
                .iter()
                .find(|hero| hero.entity == entity)
                .unwrap();
            assert!(
                !character
                    .choices
                    .iter()
                    .any(|choice| choice.offer.as_str() == "portrait")
            );
            let saved = super::super::hero_library::snapshot(&next, member).unwrap();
            assert!(
                saved
                    .hero
                    .unwrap()
                    .identity
                    .unwrap()
                    .appearance
                    .unwrap()
                    .public_description
                    .contains("silver horns")
            );
        }
    });
}

#[test]
fn lock_in_creation_keeps_the_checked_attachment_and_no_portrait_failures_still_create() {
    live_pack(|| {
        for with_portrait in [true, false] {
            let current = joined_room();
            let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let mut choices = PHONE_SHARED.to_vec();
            choices.push(("build-package", "melee-stalwart"));
            let mut request = phone_create(&current, 3, "Thorin Ashbeard", &choices);
            let mut identity = super::super::hero_identity::Identity::from_menus(&[])
                .unwrap()
                .to_rpc();
            identity.appearance.as_mut().unwrap().public_description =
                "A broad dwarf with an ash-grey beard, bronze braids and a weathered mail coat."
                    .to_owned();
            request.character.as_mut().unwrap().identity = Some(identity.clone());
            let mut input = creator_input(&current, member, &request);
            if with_portrait {
                let GameInput::Game(command) = &mut input else {
                    panic!("game input")
                };
                let GameCommand::ProposeAction { choices, .. } = &mut command.command else {
                    panic!("creation")
                };
                choices.extend(
                    super::super::creator_portraits::Attachment {
                        key: "a".repeat(64),
                        digest: [0x12; 32],
                        bytes: 128,
                        revision: 7,
                    }
                    .choices()
                    .unwrap(),
                );
            }
            let next = model::HarborEngine
                .decide_admitted(&current, &input, OperationId::from_bytes(&[3; 16]).unwrap())
                .unwrap();
            let entity = player_entity(member, &next).unwrap();
            let sheet = character_sheet(next.state(), entity).unwrap();
            assert_eq!(sheet.name, "Thorin Ashbeard");
            assert_eq!(
                super::super::hero_library::snapshot(&next, member)
                    .unwrap()
                    .hero
                    .unwrap()
                    .identity,
                Some(identity)
            );
            if with_portrait {
                let expected = format!("/gameplay/creator-portrait/{}", "a".repeat(64));
                assert_eq!(sheet.portrait_uri, expected);
                assert_eq!(sheet.portrait_draft_revision, 7);
                assert_eq!(
                    super::super::hero_identity::approved_portrait_uri(next.state(), entity),
                    Some(expected)
                );
            } else {
                assert!(sheet.portrait_uri.is_empty());
                assert_eq!(sheet.portrait_draft_revision, 0);
            }
            for role in [
                df_persistence::local_demo_scope::LocalDemoRole::Player,
                df_persistence::local_demo_scope::LocalDemoRole::Display,
            ] {
                let view = wire::journey_view(&next, role, member).unwrap();
                let journey = match view.audience.unwrap() {
                    rpc::view_message::Audience::Player(player) => player.journey.unwrap(),
                    rpc::view_message::Audience::Display(display) => display.journey.unwrap(),
                };
                let party = journey
                    .party
                    .iter()
                    .find(|hero| hero.name == "Thorin Ashbeard")
                    .unwrap();
                assert_eq!(party.portrait, sheet.portrait_uri);
                if let Some(own) = journey.own_character {
                    assert_eq!(own.portrait_uri, sheet.portrait_uri);
                }
            }
            assert_eq!(next.state().characters.len(), 1);
        }
    });
}
#[test]
fn heroes_with_several_words_or_apostrophes_in_their_name_complete_creation() {
    live_pack(|| {
        for name in ["Vix Ashlantern", "D'Artagnan", "Ælfwyn of the Red-Hill"] {
            let current = joined_room();
            let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let mut choices = PHONE_SHARED.to_vec();
            choices.push(("build-package", "melee-vanguard"));
            let mut request = phone_create(&current, 3, name, &choices);
            request.character.as_mut().unwrap().identity = Some(
                super::super::hero_identity::Identity::from_menus(&[])
                    .unwrap()
                    .to_rpc(),
            );
            let input = creator_input(&current, member, &request);
            let next = model::HarborEngine
                .decide_admitted(&current, &input, OperationId::from_bytes(&[3; 16]).unwrap())
                .unwrap_or_else(|error| panic!("{name:?} refused: {error:?}"));
            let entity = player_entity(member, &next).unwrap();
            assert_eq!(
                super::super::journey::name(next.state(), entity).unwrap(),
                name
            );
        }
    });
}
#[test]
fn archer_creation_sheet_states_the_longbow_attack_and_its_ammunition() {
    live_pack(|| {
        let current = seat_hero(
            &joined_room(),
            MEMBERS[0],
            3,
            "Sylvie",
            "archer",
            "portrait-hero-3",
        );
        let decision = current.state().decisions.last().expect("creation decision");
        let action = accepted(decision).expect("accepted creation");
        let sheet = action.character.expect("creation sheet");
        let facts = sheet
            .facts
            .iter()
            .map(|fact| fact.value.as_str())
            .collect::<Vec<_>>();
        assert!(
            facts
                .iter()
                .any(|value| value.contains("+7 to hit · 1d8+3"))
        );
        let entity = player_entity(MemberId::from_bytes(&MEMBERS[0]).unwrap(), &current)
            .expect("seated hero");
        let arrows = current
            .state()
            .inventory
            .iter()
            .filter(|item| item.owner == entity && item.origin.entry.as_str() == "arrow")
            .map(|item| item.quantity)
            .sum::<u64>();
        assert_eq!(arrows, 20);
    });
}

#[test]
fn creator_catalog_unsupported_design_is_rejected_at_the_live_wire_boundary() {
    live_pack(|| {
        let current = joined_room();
        let before = current.clone();
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        for (species, class, background) in [
            ("aasimar", "fighter-1", "soldier"),
            ("elf", "rogue-1", "criminal"),
            ("dragonborn", "ranger-1", "guide"),
            ("dwarf", "fighter-1", "noble"),
        ] {
            let mut choices = PHONE_SHARED.to_vec();
            for (group, value) in &mut choices {
                *value = match *group {
                    "species" => species,
                    "class" => class,
                    "background" => background,
                    _ => *value,
                };
            }
            choices.push(("build-package", "melee-stalwart"));
            choices.push(("portrait", "portrait-hero-1"));
            let request = phone_create(&current, 31, "Maren", &choices);
            let error = wire::journey_input(&request, member, &current).unwrap_err();
            assert_eq!(error.code(), tonic::Code::FailedPrecondition);
            assert!(
                error
                    .message()
                    .contains("Gameplay rules for this combination are not ready")
            );
            assert_eq!(current, before);
            assert!(current.state().characters.is_empty());
        }
        let mut valid = PHONE_SHARED.to_vec();
        valid.push(("build-package", "melee-stalwart"));
        valid.push(("portrait", "portrait-hero-1"));
        assert!(
            wire::journey_input(
                &phone_create(&current, 32, "Maren", &valid),
                member,
                &current
            )
            .is_ok()
        );
    });
}

/// Match the session owner's borrowed-decision admission. A plain clone shrinks Strings and
/// Vecs, hiding excess retained capacity in the candidate that the owner actually checks.
fn creation_receipt_with_original_capacities(
    checkpoint: &Checkpoint,
) -> df_session::submission::DecisionReceipt {
    let original = checkpoint
        .state()
        .decisions
        .last()
        .expect("creation decision");
    let mut decision = original.clone();
    decision
        .facts
        .reserve_exact(original.facts.capacity() - decision.facts.len());
    decision
        .draws
        .reserve_exact(original.draws.capacity() - decision.draws.len());
    decision
        .effects
        .reserve_exact(original.effects.capacity() - decision.effects.len());
    let text = decision.semantic_output.as_mut().expect("creation result");
    text.reserve_exact(original.semantic_output.as_ref().unwrap().capacity() - text.len());
    assert_eq!(decision.facts.capacity(), original.facts.capacity());
    assert_eq!(decision.draws.capacity(), original.draws.capacity());
    assert_eq!(decision.effects.capacity(), original.effects.capacity());
    assert_eq!(
        decision.semantic_output.as_ref().unwrap().capacity(),
        original.semantic_output.as_ref().unwrap().capacity()
    );
    df_session::submission::DecisionReceipt::new(
        checkpoint.basis(),
        decision,
        df_persistence::local_demo_scope::RECEIPT_RETAINED_BYTES_MAX,
    )
    .expect("the actual retained creation receipt fits the live bound")
}

fn creator_identity_at_bounds(unicode: bool) -> rpc::HeroIdentity {
    let mut identity = super::super::hero_identity::Identity::from_menus(&[])
        .unwrap()
        .to_rpc();
    // Reach both public-look bounds: 2,000 characters and, with Unicode, 4,096 bytes.
    identity.appearance.as_mut().unwrap().public_description = if unicode {
        format!("{}\u{96ea}{}", "\u{1f409}".repeat(698), "a".repeat(1301))
    } else {
        "a".repeat(2000)
    };
    let character = if unicode { "\u{1f409}" } else { "a" };
    identity.origin = character.repeat(160);
    identity.occupation = character.repeat(160);
    identity.motivation = character.repeat(160);
    identity.backstory = character.repeat(480);
    identity.facts = (0..6)
        .map(|index| {
            let first = char::from_u32(if unicode {
                0x1f409 + index
            } else {
                0x61 + index
            })
            .unwrap();
            format!("{first}{}", character.repeat(159))
        })
        .collect();
    super::super::hero_identity::Identity::from_rpc(identity.clone())
        .expect("identity reaches the admitted bounds");
    identity
}

fn creator_input(
    current: &Checkpoint,
    member: MemberId,
    request: &rpc::SubmitActionRequest,
) -> GameInput {
    let mut input = wire::journey_input(request, member, current)
        .expect("the complete creator phone request converts");
    let GameInput::Game(command) = &mut input else {
        panic!("game input")
    };
    let GameCommand::ProposeAction { choices, .. } = &mut command.command else {
        panic!("creation action")
    };
    // Same trusted attachment hook as Actor::submit; clients cannot supply these choices.
    choices
        .extend(super::super::hero_library::identity_choices(request.character.as_ref()).unwrap());
    assert!(choices.len() <= 256, "creation record count");
    assert!(
        input.retained_bytes().unwrap()
            <= df_persistence::local_demo_scope::input_retained_bytes_max(&input)
    );
    input
}

#[test]
fn stalwart_then_every_package_with_maximal_creator_identity_fits_and_saves_losslessly() {
    live_pack(|| {
        let offer = wire::creation_offer().unwrap();
        let packages = offer
            .groups
            .iter()
            .find(|group| group.group_id == "build-package")
            .unwrap();
        for mode in ["default", "maximum-ascii", "maximum-unicode"] {
            let identity = match mode {
                "maximum-ascii" => creator_identity_at_bounds(false),
                "maximum-unicode" => creator_identity_at_bounds(true),
                _ => super::super::hero_identity::Identity::from_menus(&[])
                    .unwrap()
                    .to_rpc(),
            };
            // Include the maximum public scenery as well; it shares the creator record budget.
            let scenery = format!("{}{}", "\u{1f409}".repeat(136), "a".repeat(224));
            for second_package in &packages.options {
                let mut current = joined_room();
                for (index, package) in ["melee-stalwart", second_package.option_id.as_str()]
                    .into_iter()
                    .enumerate()
                {
                    let member = MemberId::from_bytes(&MEMBERS[index]).unwrap();
                    let mut choices = PHONE_SHARED.to_vec();
                    choices.push(("build-package", package));
                    let operation = 3 + index as u8;
                    let mut request =
                        phone_create(&current, operation, ["Brakka", "Sigrun"][index], &choices);
                    let selection = request.character.as_mut().unwrap();
                    selection.identity = Some(identity.clone());
                    selection.portrait_scenery = scenery.clone();
                    let input = creator_input(&current, member, &request);
                    let input_bytes = input.retained_bytes().unwrap();
                    current = model::HarborEngine
                        .decide_admitted(
                            &current,
                            &input,
                            OperationId::from_bytes(&[operation; 16]).unwrap(),
                        )
                        .unwrap_or_else(|error| {
                            panic!("{mode} {package} seat {index} refused: {error:?}")
                        });
                    let receipt = creation_receipt_with_original_capacities(&current);
                    let sheet = accepted(receipt.decision()).unwrap().character.unwrap();
                    assert!(
                        sheet.identity.is_none(),
                        "full identity stays in committed state"
                    );
                    assert_eq!(sheet.name, request.character.as_ref().unwrap().name);
                    println!(
                        "HS3 AFTER mode={mode} seat={} package={package} input={input_bytes} receipt={} semantic={}",
                        index + 1,
                        receipt.retained_bytes().unwrap(),
                        receipt.decision().semantic_output.as_ref().unwrap().len()
                    );
                }
                // The live store must save what the rules accepted: encode, decode and compare
                // the two-hero checkpoint under the server's own codec limits.
                model::recovery()
                    .unwrap()
                    .encode_candidate(
                        &current,
                        current.pins(),
                        crate::gameplay::live_codec_limits(),
                    )
                    .unwrap_or_else(|error| {
                        panic!("{mode} {second_package:?} checkpoint not storable: {error:?}")
                    });
                // SaveMySeatHero snapshots the committed seat, never the phone's identity.
                // Recover both after the second acceptance, and round-trip the actual library bytes.
                for (index, seat) in MEMBERS.iter().take(2).enumerate() {
                    let member = MemberId::from_bytes(seat).unwrap();
                    let saved = super::super::hero_library::snapshot(&current, member)
                        .unwrap()
                        .hero
                        .unwrap();
                    let recovered =
                        rpc::LibraryHeroIdentity::decode(saved.encode_to_vec().as_slice()).unwrap();
                    assert_eq!(recovered.name, ["Brakka", "Sigrun"][index]);
                    assert_eq!(recovered.identity, Some(identity.clone()));
                    assert_eq!(recovered.portrait_scenery, scenery);
                    assert_eq!(recovered.design.len(), 3);
                    assert!(recovered.design.iter().all(|choice| matches!(
                        choice.group_id.as_str(),
                        "species" | "class" | "background"
                    )));
                }
            }
        }
    });
}

#[test]
fn legacy_public_look_and_48_byte_creator_chunks_still_recover_the_full_identity() {
    live_pack(|| {
        let current = joined_room();
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let mut choices = PHONE_SHARED.to_vec();
        choices.extend([
            ("build-package", "melee-stalwart"),
            ("portrait", "portrait-hero-1"),
        ]);
        let mut request = phone_create(&current, 3, "Brakka", &choices);
        let mut identity = super::super::hero_identity::Identity::from_menus(&[])
            .unwrap()
            .to_rpc();
        identity.appearance.as_mut().unwrap().public_description =
            "Old look\n\u{96ea}\t\u{1f409}".repeat(10);
        identity.backstory = "The exact accepted story survives.".into();
        request.character.as_mut().unwrap().identity = Some(identity.clone());
        let mut input = creator_input(&current, member, &request);
        let GameInput::Game(command) = &mut input else {
            panic!("game input")
        };
        let GameCommand::ProposeAction { choices, .. } = &mut command.command else {
            panic!("creation action")
        };
        choices.retain(|(field, _)| !field.as_str().starts_with("identity-public-look:"));
        let public = identity
            .appearance
            .as_ref()
            .unwrap()
            .public_description
            .as_bytes();
        choices.push((
            model::label("identity-public-look:v1:count").unwrap(),
            model::label(&public.len().div_ceil(32).to_string()).unwrap(),
        ));
        for (index, chunk) in public.chunks(32).enumerate() {
            choices.push((
                model::label(&format!("identity-public-look:v1:{index:03}")).unwrap(),
                model::label(&hex(chunk)).unwrap(),
            ));
        }
        let mut record = Vec::new();
        for (field, value) in choices
            .iter()
            .filter(|(field, _)| field.as_str().starts_with("identity-portrait:creator-v1:"))
        {
            assert!(
                field
                    .as_str()
                    .ends_with(&format!("{:04}", record.len().div_ceil(96)))
            );
            record.extend(super::super::hero_library::decode_record_chunk(value.as_str()).unwrap());
        }
        choices.retain(|(field, _)| !field.as_str().starts_with("identity-portrait:creator-v1:"));
        for (index, chunk) in record.chunks(48).enumerate() {
            choices.push((
                model::label(&format!("identity-portrait:creator-v1:{index:04}")).unwrap(),
                model::label(&super::super::hero_library::encode_record_chunk(chunk)).unwrap(),
            ));
        }
        let next = model::HarborEngine
            .decide_admitted(&current, &input, OperationId::from_bytes(&[3; 16]).unwrap())
            .unwrap();
        creation_receipt_with_original_capacities(&next);
        assert_eq!(
            super::super::hero_library::snapshot(&next, member)
                .unwrap()
                .hero
                .unwrap()
                .identity,
            Some(identity)
        );
    });
}

#[test]
fn every_srd_521_species_creates_through_decide_admitted_and_projects_its_own_sheet() {
    live_pack(|| {
        for (species, extra, hp, speed, sight, gap) in [
            (
                "dragonborn",
                vec![("draconic-ancestry", "red")],
                12,
                "30 ft",
                "60 ft",
                "Draconic Flight",
            ),
            ("dwarf", vec![], 13, "30 ft", "120 ft", "Stonecunning"),
            (
                "elf",
                vec![
                    ("elven-lineage", "drow"),
                    ("keen-senses", "insight"),
                    ("species-spellcasting", "wisdom"),
                ],
                12,
                "30 ft",
                "120 ft",
                "Trance",
            ),
            (
                "gnome",
                vec![
                    ("gnomish-lineage", "rock-gnome"),
                    ("species-spellcasting", "intelligence"),
                ],
                12,
                "30 ft",
                "60 ft",
                "Gnomish Lineage",
            ),
            (
                "goliath",
                vec![("giant-ancestry", "fire")],
                12,
                "35 ft",
                "None",
                "Large Form",
            ),
            ("halfling", vec![], 12, "30 ft", "None", "Brave"),
            (
                "human",
                vec![
                    ("species-size", "small"),
                    ("human-skill", "arcana"),
                    ("versatile", "tough"),
                ],
                14,
                "30 ft",
                "None",
                "Resourceful",
            ),
            ("orc", vec![], 12, "30 ft", "120 ft", "Adrenaline Rush"),
            (
                "tiefling",
                vec![
                    ("fiendish-legacy", "chthonic"),
                    ("species-size", "small"),
                    ("species-spellcasting", "charisma"),
                ],
                12,
                "30 ft",
                "60 ft",
                "Thaumaturgy",
            ),
        ] {
            let current = joined_room();
            let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let mut choices = PHONE_SHARED.to_vec();
            choices
                .iter_mut()
                .find(|(key, _)| *key == "species")
                .unwrap()
                .1 = species;
            choices.extend(extra);
            choices.extend([
                ("build-package", "melee-stalwart"),
                ("portrait", "portrait-hero-1"),
            ]);
            assert!(choices.len() <= 16);
            let request = phone_create(&current, 3, "Maren Coast", &choices);
            let input = wire::journey_input(&request, member, &current).unwrap();
            let next = model::HarborEngine
                .decide_admitted(&current, &input, OperationId::from_bytes(&[3; 16]).unwrap())
                .unwrap_or_else(|error| panic!("{species}: {error:?}"));
            let sheet = accepted(next.state().decisions.last().unwrap())
                .unwrap()
                .character
                .unwrap();
            assert_eq!(sheet.maximum_hit_points, hp, "{species}");
            assert_eq!(sheet.name, "Maren Coast", "{species}");
            let who = player_entity(member, &next).unwrap();
            for (group, skill) in [("human-skill", "arcana"), ("keen-senses", "insight")] {
                if choices.iter().any(|(key, _)| *key == group) {
                    assert_eq!(
                        super::super::beat_script_runtime::skill_check_modifiers(
                            next.state(),
                            who,
                            skill
                        )
                        .unwrap()
                        .1,
                        2,
                        "{species}: source-selected skill proficiency"
                    );
                }
            }
            assert!(sheet.facts.iter().any(|fact|fact.label=="Species" && fact.value.eq_ignore_ascii_case(species)),"{species}");
            assert!(
                sheet
                    .facts
                    .iter()
                    .any(|fact| fact.label == "Speed" && fact.value == speed),
                "{species}"
            );
            assert!(
                sheet
                    .facts
                    .iter()
                    .any(|fact| fact.label == "Darkvision" && fact.value == sight),
                "{species}"
            );
            assert!(
                sheet
                    .facts
                    .iter()
                    .any(|fact| fact.label == "Not in play yet" && fact.value.contains(gap)),
                "{species}"
            );
            if species != "dwarf" {
                assert!(
                    !sheet
                        .facts
                        .iter()
                        .any(|fact| fact.value.contains("Dwarven Toughness"))
                );
            }
        }
    });
}
#[test]
fn all_nine_srd_species_are_supported_fighter_soldier_designs_and_aasimar_stays_design_only() {
    live_pack(|| {
        let offer = wire::creation_offer().unwrap();
        assert_eq!(offer.supported_designs.len(), 9);
        for species in [
            "dragonborn",
            "dwarf",
            "elf",
            "gnome",
            "goliath",
            "halfling",
            "human",
            "orc",
            "tiefling",
        ] {
            assert!(
                offer
                    .supported_designs
                    .iter()
                    .any(|design| design.species_id == species
                        && design.class_id == "fighter-1"
                        && design.background_id == "soldier")
            );
        }
        assert!(
            !offer
                .supported_designs
                .iter()
                .any(|design| design.species_id == "aasimar")
        );
    });
}
