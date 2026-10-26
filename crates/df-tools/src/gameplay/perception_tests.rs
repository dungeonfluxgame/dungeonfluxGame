use super::*;
use crate::gameplay::{courier_ai, journey, model};
use df_model::checkpoint::*;
use df_persistence::local_demo_scope::{DISPLAY, LocalDemoRole, PLAYER};
use df_types::{MemberId, OperationId, RevisionLabel};
use prost::Message;

fn command(
    current: &Checkpoint,
    member: MemberId,
    operation: u8,
    entry: &str,
    choices: Vec<(RevisionLabel, RevisionLabel)>,
) -> GameInput {
    GameInput::Game(CommandInput {
        basis: current.basis(),
        operation: OperationId::from_bytes(&[operation; 16]).unwrap(),
        member,
        observed_revision: current.basis().revision,
        command: GameCommand::ProposeAction {
            actor: if member.as_bytes() == &PLAYER {
                journey::room_entity().unwrap()
            } else {
                journey::player_entity(member, current).unwrap()
            },
            action: model::content(entry).unwrap(),
            targets: vec![],
            choices,
        },
    })
}
fn build(name: &str) -> Vec<(RevisionLabel, RevisionLabel)> {
    [
        ("name", journey::hex(name.as_bytes())),
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
    .map(|(key, value)| (model::label(key).unwrap(), model::label(&value).unwrap()))
    .collect()
}
fn before_answer() -> (Checkpoint, MemberId, MemberId) {
    let mut current = journey::initial().unwrap();
    let bootstrap = MemberId::from_bytes(&PLAYER).unwrap();
    for operation in [1, 2] {
        current = journey::stage_join(
            &current,
            &command(&current, bootstrap, operation, "join-room", vec![]),
        )
        .unwrap();
    }
    let members = current
        .state()
        .members
        .iter()
        .filter(|link| link.member != bootstrap)
        .map(|link| link.member)
        .collect::<Vec<_>>();
    let (first, second) = (members[0], members[1]);
    current = journey::stage(
        &current,
        &command(&current, first, 3, "create-character", build("Brynn")),
    )
    .unwrap();
    current = journey::stage(
        &current,
        &command(&current, second, 4, "create-character", build("Vale")),
    )
    .unwrap();
    current = journey::stage(
        &current,
        &command(&current, first, 5, "begin-story", vec![]),
    )
    .unwrap();
    (current, first, second)
}
fn pending() -> (Checkpoint, MemberId, MemberId) {
    let (current, first, second) = before_answer();
    (
        journey::stage(
            &current,
            &command(&current, first, 6, "ask-courier", vec![]),
        )
        .unwrap(),
        first,
        second,
    )
}

fn completed() -> (Checkpoint, MemberId, MemberId) {
    let (current, first, second) = pending();
    let intent = current.state().intents.first().unwrap();
    let completion = courier_ai::execute(&current, intent).unwrap();
    let next = courier_ai::stage_completion(
        &current,
        &completion,
        courier_ai::completion_operation(intent).unwrap(),
    )
    .unwrap();
    (next, first, second)
}

fn player(current: &Checkpoint, member: MemberId) -> rpc::PlayerGameplayView {
    let projected = journey_view(current, LocalDemoRole::Player, member).unwrap();
    let Some(rpc::view_message::Audience::Player(view)) = projected.audience else {
        panic!("player projection required")
    };
    view
}

fn display(current: &Checkpoint) -> rpc::ViewMessage {
    journey_view(
        current,
        LocalDemoRole::Display,
        MemberId::from_bytes(&DISPLAY).unwrap(),
    )
    .unwrap()
}

fn note_mut(state: &mut GameState) -> &mut GameFact {
    state.facts.iter_mut().find(|fact| matches!(
        &fact.value,
        FactValue::ContentEvent { definition, .. } if definition.entry.as_str() == "private-courier-note"
    )).unwrap()
}

#[test]
fn witnessed_shared_courier_reaction_reaches_both_players_and_display_without_private_cue() {
    let (opening, first, second) = before_answer();
    let escorted = journey::stage(
        &opening,
        &command(&opening, first, 6, "escort-courier", vec![]),
    )
    .unwrap();
    let defended = journey::stage(
        &escorted,
        &command(&escorted, first, 7, "defend-courier", vec![]),
    )
    .unwrap();
    let witnessed = "The courier now looks to Brynn for support on the escort.";
    for view in [
        journey_view(&defended, LocalDemoRole::Player, first).unwrap(),
        journey_view(&defended, LocalDemoRole::Player, second).unwrap(),
        display(&defended),
    ] {
        let narration = match view.audience.as_ref().unwrap() {
            rpc::view_message::Audience::Player(player) => &player.narration,
            rpc::view_message::Audience::Display(display) => &display.narration,
        };
        assert!(narration.contains(witnessed));
        let encoded = view.encode_to_vec();
        assert!(
            encoded
                .windows(witnessed.len())
                .any(|bytes| bytes == witnessed.as_bytes())
        );
        assert!(
            !encoded
                .windows(courier_ai::RESPONSE.len())
                .any(|bytes| bytes == courier_ai::RESPONSE.as_bytes())
        );
    }

    let (private, recipient, other) = completed();
    let private_defended = journey::stage(
        &private,
        &command(&private, recipient, 7, "defend-courier", vec![]),
    )
    .unwrap();
    assert_eq!(
        player(&private_defended, recipient).private_clue,
        courier_ai::RESPONSE
    );
    for view in [
        journey_view(&private_defended, LocalDemoRole::Player, other).unwrap(),
        display(&private_defended),
    ] {
        let encoded = view.encode_to_vec();
        assert!(
            !encoded
                .windows(witnessed.len())
                .any(|bytes| bytes == witnessed.as_bytes())
        );
        assert!(
            !encoded
                .windows(courier_ai::RESPONSE.len())
                .any(|bytes| bytes == courier_ai::RESPONSE.as_bytes())
        );
    }
}

#[test]
fn witnessed_courier_reaction_requires_current_shared_cause_and_npc_evidence() {
    let (opening, first, second) = before_answer();
    let escorted = journey::stage(
        &opening,
        &command(&opening, first, 6, "escort-courier", vec![]),
    )
    .unwrap();
    let defended = journey::stage(
        &escorted,
        &command(&escorted, first, 7, "defend-courier", vec![]),
    )
    .unwrap();
    let mut without_witness = defended.state().clone();
    without_witness.continuity.witnesses.clear();
    let mut without_knowledge = defended.state().clone();
    without_knowledge.continuity.npcs[0].known_facts.clear();
    let mut without_relationship = defended.state().clone();
    without_relationship
        .relationships
        .iter_mut()
        .find(|relationship| relationship.state.as_str() == "escort-supported")
        .unwrap()
        .state = model::label("unfamiliar").unwrap();
    let mut private_contact = defended.state().clone();
    private_contact
        .facts
        .iter_mut()
        .find(|fact| {
            matches!(&fact.value, FactValue::ContentEvent { definition, .. }
            if definition.entry.as_str() == "courier-escort-contact")
        })
        .unwrap()
        .audience = AudienceScope::Members(vec![first]);
    let mut wrong_cause = defended.state().clone();
    let escort_terminal = wrong_cause
        .facts
        .iter()
        .find(|fact| {
            matches!(&fact.value, FactValue::ContentEvent { definition, .. }
            if definition.entry.as_str() == "escort-courier")
        })
        .unwrap()
        .id;
    wrong_cause
        .facts
        .iter_mut()
        .find(|fact| {
            matches!(&fact.value, FactValue::ContentEvent { definition, .. }
            if definition.entry.as_str() == "courier-escort-reaction")
        })
        .unwrap()
        .cause = Some(escort_terminal);
    for state in [
        without_witness,
        without_knowledge,
        without_relationship,
        private_contact,
        wrong_cause,
    ] {
        let changed = model::checkpoint(defended.basis(), state).unwrap();
        for (role, principal) in [
            (LocalDemoRole::Player, first),
            (LocalDemoRole::Player, second),
            (
                LocalDemoRole::Display,
                MemberId::from_bytes(&DISPLAY).unwrap(),
            ),
        ] {
            assert_eq!(
                journey_view(&changed, role, principal),
                Err(RepositoryError::InvalidCandidate)
            );
        }
    }
}

#[test]
fn altered_only_retained_witness_identity_refuses_all_roles_and_valid_replay_is_unchanged() {
    let (opening, first, second) = before_answer();
    let escorted = journey::stage(
        &opening,
        &command(&opening, first, 6, "escort-courier", vec![]),
    )
    .unwrap();
    let defended = journey::stage(
        &escorted,
        &command(&escorted, first, 7, "defend-courier", vec![]),
    )
    .unwrap();
    let original = defended.clone();
    let restored = model::checkpoint(defended.basis(), defended.state().clone()).unwrap();
    assert_eq!(restored, defended);
    let contact = defended
        .state()
        .facts
        .iter()
        .find(|fact| {
            matches!(&fact.value, FactValue::ContentEvent { definition, .. }
                if definition.entry.as_str() == "courier-escort-contact")
        })
        .unwrap();
    let mut changed_state = defended.state().clone();
    let witness_index = changed_state
        .continuity
        .witnesses
        .iter()
        .position(|witness| witness.fact == contact.id)
        .unwrap();
    let original_witness = changed_state.continuity.witnesses[witness_index].id;
    let mut wrong_id = *original_witness.as_bytes();
    wrong_id[0] ^= 0x80;
    let altered_witness = RecordId::from_bytes(&wrong_id).unwrap();
    assert_ne!(altered_witness, original_witness);
    changed_state.continuity.witnesses[witness_index].id = altered_witness;
    let changed = model::checkpoint(defended.basis(), changed_state).unwrap();
    changed
        .validate_resume(changed.basis(), &model::pins().unwrap())
        .unwrap();
    let mut undo_identity_only = changed.state().clone();
    undo_identity_only.continuity.witnesses[witness_index].id = original_witness;
    assert_eq!(undo_identity_only, defended.state().clone());
    let before_projection = changed.clone();
    for (role, principal) in [
        (LocalDemoRole::Player, first),
        (LocalDemoRole::Player, second),
        (
            LocalDemoRole::Display,
            MemberId::from_bytes(&DISPLAY).unwrap(),
        ),
    ] {
        let expected = journey_view(&defended, role, principal).unwrap();
        let narration = match expected.audience.as_ref().unwrap() {
            rpc::view_message::Audience::Player(player) => &player.narration,
            rpc::view_message::Audience::Display(display) => &display.narration,
        };
        assert!(narration.contains("The courier now looks to Brynn for support on the escort."));
        assert_eq!(journey_view(&restored, role, principal).unwrap(), expected);
        assert_eq!(journey_view(&restored, role, principal).unwrap(), expected);
        let expected_bytes = expected.encode_to_vec();
        assert_eq!(
            journey_view(&restored, role, principal)
                .unwrap()
                .encode_to_vec(),
            expected_bytes
        );
        assert_eq!(
            journey_view(&restored, role, principal)
                .unwrap()
                .encode_to_vec(),
            expected_bytes
        );
        assert_eq!(
            journey_view(&changed, role, principal),
            Err(RepositoryError::InvalidCandidate)
        );
    }
    assert_eq!(changed.state().draws, defended.state().draws);
    assert_eq!(changed, before_projection);
    assert_eq!(restored, original);
    assert_eq!(defended, original);
}

#[test]
fn real_prepared_completion_projects_exact_source_to_only_its_current_recipient() {
    let (pending, first, second) = pending();
    let before = pending.clone();
    let first_pending = player(&pending, first);
    let second_pending = player(&pending, second);
    let shared_pending = display(&pending);
    assert!(first_pending.private_clue.is_empty());
    assert!(second_pending.private_clue.is_empty());
    let intent = pending.state().intents.first().unwrap();
    let completion = courier_ai::execute(&pending, intent).unwrap();
    let current = courier_ai::stage_completion(
        &pending,
        &completion,
        courier_ai::completion_operation(intent).unwrap(),
    )
    .unwrap();
    let mut first_current = player(&current, first);
    assert_eq!(first_current.private_clue, courier_ai::RESPONSE);
    first_current.private_clue.clear();
    assert_eq!(first_current, first_pending);
    assert_eq!(player(&current, second), second_pending);
    assert_eq!(display(&current).audience, shared_pending.audience);
    for encoded in [
        display(&current).encode_to_vec(),
        player(&current, second).encode_to_vec(),
    ] {
        assert!(
            !encoded
                .windows(courier_ai::RESPONSE.len())
                .any(|bytes| bytes == courier_ai::RESPONSE.as_bytes())
        );
        assert!(
            !encoded
                .windows(b"private-courier-note".len())
                .any(|bytes| bytes == b"private-courier-note")
        );
    }
    assert_eq!(pending, before);
}

#[test]
fn current_canonical_audience_removal_omits_saved_cue_even_with_a_knowledge_grant() {
    let (current, first, second) = completed();
    let original = current.clone();
    let mut expected_first = player(&current, first);
    let mut expected_second = player(&current, second);
    assert_eq!(expected_first.private_clue, courier_ai::RESPONSE);
    assert!(expected_second.private_clue.is_empty());
    for view in [&expected_first, &expected_second] {
        assert!(view.action_available);
        assert_eq!(view.offers.len(), 2);
        assert_eq!(view.offers[1].offer_id, "opening-talk:bandit");
        assert_eq!(view.offers[1].label, "Talk to the man behind the courier");
        assert_eq!(
            view.offers[1].input_groups[0].options[0].label,
            "The man behind the courier"
        );
        assert!(
            !view
                .encode_to_vec()
                .windows(b"Dain Rusk".len())
                .any(|bytes| bytes == b"Dain Rusk")
        );
        assert_eq!(
            view.offers[0].action_kind,
            rpc::GameplayActionKind::DefendCourier as i32
        );
    }
    // Removing current disclosure also removes its encounter prerequisite for either player.
    // Exact expected views retain every other narration, character, scene and journey field.
    expected_first.private_clue.clear();
    // RP-03 adds public talk independently of the private fight prerequisite.
    for view in [&mut expected_first, &mut expected_second] {
        view.offers
            .retain(|offer| offer.action_kind != rpc::GameplayActionKind::DefendCourier as i32);
    }
    let expected_display = display(&current);
    for audience in [
        AudienceScope::Host,
        AudienceScope::Members(vec![MemberId::from_bytes(&PLAYER).unwrap()]),
    ] {
        let mut state = current.state().clone();
        let note = note_mut(&mut state);
        note.audience = audience;
        let id = note.id;
        state.knowledge.push(KnowledgeGrant {
            observer: first,
            fact: id,
            source: id,
        });
        let removed = model::checkpoint(current.basis(), state).unwrap();
        let before_projection = removed.clone();
        for (principal, expected) in [(first, &expected_first), (second, &expected_second)] {
            let projected = journey_view(&removed, LocalDemoRole::Player, principal).unwrap();
            assert_eq!(projected.revision, expected_display.revision);
            let Some(rpc::view_message::Audience::Player(view)) = projected.audience else {
                panic!("player projection required")
            };
            assert_eq!(&view, expected);
            assert!(view.private_clue.is_empty());
            assert_eq!(view.offers.len(), 1);
            assert_eq!(view.offers[0].offer_id, "opening-talk:bandit");
            assert!(view.action_available);
        }
        assert_eq!(display(&removed), expected_display);
        // A fresh projection, including a reconnect snapshot, never uses the old cue.
        assert!(player(&removed, first).private_clue.is_empty());
        assert!(player(&removed, second).private_clue.is_empty());
        assert_eq!(removed.state().draws, current.state().draws);
        assert_eq!(removed, before_projection);
    }
    assert_eq!(current, original);
}

#[test]
fn unsupported_selected_fact_never_becomes_clue_text_and_saved_record_is_still_validated() {
    let (current, first, second) = completed();
    let mut state = current.state().clone();
    note_mut(&mut state).value = FactValue::ContentEvent {
        definition: model::content("harbor").unwrap(),
        subjects: vec![],
    };
    let unsupported = model::checkpoint(current.basis(), state).unwrap();
    assert!(player(&unsupported, first).private_clue.is_empty());
    assert!(player(&unsupported, second).private_clue.is_empty());
    assert_eq!(display(&unsupported), display(&current));
    let mut state = current.state().clone();
    let decision = state
        .decisions
        .iter_mut()
        .find(|decision| decision.source_policy.as_str() == courier_ai::POLICY)
        .unwrap();
    decision.semantic_output = Some("Unapproved text must never reach a private view".to_owned());
    let changed = model::checkpoint(current.basis(), state).unwrap();
    assert_eq!(
        journey_view(&changed, LocalDemoRole::Player, first),
        Err(RepositoryError::InvalidCandidate)
    );
    assert!(player(&changed, second).private_clue.is_empty());
    assert_eq!(display(&changed), display(&current));
}

fn rebuild(
    current: &Checkpoint,
    pins: CheckpointPins,
    state: GameState,
    maximum_records: usize,
) -> Checkpoint {
    let recovery = model::recovery().unwrap();
    let mut limits = model::limits();
    limits.maximum_records = maximum_records;
    let resources = journey::resources_for(&state).unwrap();
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        current.basis(),
        pins,
        state,
        ReferenceInventory {
            rules: &recovery.rules,
            content: &recovery.content,
            resources: &resources,
            assets: &recovery.assets,
        },
        limits,
    )
    .unwrap()
}

#[test]
fn real_projection_refuses_unadmitted_current_pins_and_fact_scan_overflow() {
    let (current, first, second) = completed();
    let mut pins = current.pins().clone();
    pins.content.content_digest.0[0] ^= 1;
    let stale = rebuild(&current, pins, current.state().clone(), 4096);
    let mut state = current.state().clone();
    for ordinal in 0_u32..513 {
        let mut id = [0x7f; 16];
        id[..4].copy_from_slice(&ordinal.to_be_bytes());
        state.facts.push(GameFact {
            id: FactId::from_bytes(&id).unwrap(),
            revision: current.basis().revision,
            operation: OperationId::from_bytes(&[0x78; 16]).unwrap(),
            ordinal,
            cause: None,
            audience: AudienceScope::Shared,
            value: FactValue::ContentEvent {
                definition: model::content("harbor").unwrap(),
                subjects: vec![],
            },
        });
    }
    let overflow = rebuild(&current, current.pins().clone(), state, 4096);
    for (role, principal) in [
        (
            LocalDemoRole::Display,
            MemberId::from_bytes(&DISPLAY).unwrap(),
        ),
        (LocalDemoRole::Player, first),
        (LocalDemoRole::Player, second),
    ] {
        assert_eq!(
            journey_view(&stale, role, principal),
            Err(RepositoryError::InvalidCandidate)
        );
        assert_eq!(
            journey_view(&overflow, role, principal),
            Err(RepositoryError::Capacity)
        );
    }
}

#[test]
fn authenticated_member_selection_requires_current_membership_at_real_projection() {
    let (current, _, _) = completed();
    let unknown = MemberId::from_bytes(&[0x79; 16]).unwrap();
    assert_eq!(
        journey_view(&current, LocalDemoRole::Player, unknown),
        Err(RepositoryError::Unauthorized)
    );
}

#[test]
fn selected_reveal_still_requires_the_original_exact_single_recipient_contract() {
    let (current, first, second) = completed();
    for audience in [
        AudienceScope::Shared,
        AudienceScope::Members(vec![first, second]),
    ] {
        let mut state = current.state().clone();
        note_mut(&mut state).audience = audience;
        let changed = model::checkpoint(current.basis(), state).unwrap();
        assert_eq!(
            journey_view(&changed, LocalDemoRole::Player, first),
            Err(RepositoryError::InvalidCandidate)
        );
        assert_eq!(
            journey_view(&changed, LocalDemoRole::Player, second),
            Err(RepositoryError::InvalidCandidate)
        );
        assert_eq!(display(&changed), display(&current));
    }
}
