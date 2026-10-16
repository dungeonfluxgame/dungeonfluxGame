use super::*;

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
fn appearance_must_match_the_combat_package_on_the_server() {
    live_pack(|| {
        let with = |package: &'static str, portrait: &'static str| {
            let mut choices = PHONE_SHARED.to_vec();
            choices.push(("build-package", package));
            choices.push(("portrait", portrait));
            pack::validate_character("Sylvie", &choices).is_ok()
        };
        assert!(with("archer", "portrait-hero-3"));
        assert!(!with("archer", "portrait-hero-1"));
        assert!(with("melee-stalwart", "portrait-hero-1"));
        assert!(with("melee-stalwart", "portrait-hero-2"));
        assert!(!with("melee-stalwart", "portrait-hero-3"));
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
            let appearance = group("portrait")
                .options
                .iter()
                .find(|look| look.requires_options.contains(&package.option_id))
                .expect("every package has a look");
            let mut choices = PHONE_SHARED.to_vec();
            choices.push(("build-package", package.option_id.as_str()));
            choices.push(("portrait", appearance.option_id.as_str()));
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
fn the_offer_lists_each_appearance_only_with_the_packages_the_server_accepts_it_with() {
    live_pack(|| {
        let offer = wire::creation_offer().expect("creation offer");
        let appearances = offer
            .groups
            .iter()
            .find(|group| group.group_id == "portrait")
            .expect("appearance group");
        let packages = offer
            .groups
            .iter()
            .find(|group| group.group_id == "build-package")
            .expect("package group");
        assert!(!appearances.options.is_empty() && !packages.options.is_empty());
        for appearance in &appearances.options {
            assert_eq!(appearance.requires_group, "build-package");
            for package in &packages.options {
                let mut choices = PHONE_SHARED.to_vec();
                choices.push(("build-package", package.option_id.as_str()));
                choices.push(("portrait", appearance.option_id.as_str()));
                assert_eq!(
                    appearance.requires_options.contains(&package.option_id),
                    pack::validate_character("Sylvie", &choices).is_ok(),
                    "{} with {}",
                    appearance.option_id,
                    package.option_id
                );
            }
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
