use df_display::{DisplayPhase, DisplayProjectionError, DisplaySceneKind, project_display_view};
use df_protocol::common::{
    CombatOutcome, CombatView, DisplayGameplayView, GameplayScene, HarborDestination, JourneyPhase,
    JourneyView, PartyMemberView,
};

fn party_member(name: &str, id: &[u8]) -> PartyMemberView {
    PartyMemberView {
        name: name.to_owned(),
        character_ready: true,
        hit_points: 8,
        maximum_hit_points: 11,
        active_turn: false,
        unconscious: false,
        participant_id: id.to_vec(),
    }
}

fn display_view() -> DisplayGameplayView {
    DisplayGameplayView {
        narration: "The harbor bell rings.".to_owned(),
        scene_asset: "assets/scenes/harbor.png".to_owned(),
        scene: Some(GameplayScene {
            destination: HarborDestination::LanternWharf as i32,
            title: "Lantern Wharf".to_owned(),
            scene_asset: "assets/scenes/harbor.png".to_owned(),
            speaker_portrait: String::new(),
        }),
        journey: Some(JourneyView {
            phase: JourneyPhase::Combat as i32,
            party: vec![
                party_member("Asha", &[0xA1; 16]),
                party_member("Borin", &[0xB2; 16]),
            ],
            speaker: "Mara".to_owned(),
            dialogue: "The courier points toward the wharf.".to_owned(),
            combat: Some(CombatView {
                participants: vec![
                    party_member("Asha", &[0xA1; 16]),
                    party_member("Raider", &[0xB3; 16]),
                ],
                active_actor: "Asha".to_owned(),
                round: 3,
                outcomes: vec![CombatOutcome {
                    actor_name: "Asha".to_owned(),
                    target_name: "Raider".to_owned(),
                    attack_die: 17,
                    attack_modifier: 5,
                    attack_total: 22,
                    target_armor_class: 13,
                    hit: true,
                    critical: false,
                    damage_dice: vec![6, 4],
                    damage: 10,
                    target_hit_points: 2,
                    knocked_out: false,
                    grazed: false,
                    source_revision: "private-outcome-revision".to_owned(),
                    actor_id: vec![0xC3; 16],
                    target_id: vec![0xD4; 16],
                }],
                finished: false,
                source_revision: "private-combat-revision".to_owned(),
                active_participant_id: vec![0xE5; 16],
            }),
            ..JourneyView::default()
        }),
    }
}

#[test]
fn generated_display_view_projects_only_selected_public_scene_party_combat_and_caption_values() {
    let projection = project_display_view(&display_view()).expect("fixture is valid");

    assert_eq!(projection.scene.kind, Some(DisplaySceneKind::LanternWharf));
    assert_eq!(projection.scene.title, "Lantern Wharf");
    assert_eq!(projection.phase, DisplayPhase::Combat);
    assert_eq!(projection.narration, "The harbor bell rings.");
    assert_eq!(projection.party.len(), 2);
    assert_eq!(projection.party[0].name, "Asha");
    assert_eq!(projection.party[0].maximum_hit_points, 11);
    let combat = projection.combat.expect("combat was supplied");
    assert_eq!(combat.round, 3);
    assert_eq!(combat.active_actor.as_deref(), Some("Asha"));
    assert_eq!(combat.participants[0].name, "Asha");
    assert_eq!(combat.participants[1].name, "Raider");
    assert_eq!(combat.outcomes[0].attack_die, 17);
    assert_eq!(combat.outcomes[0].attack_total, 22);
    assert_eq!(combat.outcomes[0].damage_dice, [6, 4]);
    assert_eq!(
        projection.dialogue_caption,
        Some(df_display::DisplayCaption {
            speaker: Some("Mara".to_owned()),
            text: "The courier points toward the wharf.".to_owned(),
        })
    );
}

#[test]
fn generated_display_view_never_retains_private_nested_fields_or_ids() {
    let mut view = display_view();
    view.scene_asset = "private-asset-sentinel".to_owned();
    view.scene.as_mut().expect("fixture has scene").scene_asset =
        "private-scene-asset-sentinel".to_owned();
    let journey = view.journey.as_mut().expect("fixture has a journey");
    journey.room_code = "private-room-sentinel".to_owned();
    journey.join_path = "/private/join-sentinel".to_owned();
    journey.creation = Some(df_protocol::common::CharacterCreationOffer {
        description: "private-creation-sentinel".to_owned(),
        source_revision: "private-creation-revision".to_owned(),
        ..Default::default()
    });
    journey.own_character = Some(df_protocol::common::CharacterSheet {
        name: "private-character-sentinel".to_owned(),
        source_revision: "private-sheet-revision".to_owned(),
        ..Default::default()
    });
    journey.party[0].participant_id = vec![241; 16];
    let combat = journey.combat.as_mut().expect("fixture has combat");
    combat.participants[0].participant_id = vec![242; 16];
    combat.active_participant_id = vec![243; 16];
    combat.outcomes[0].actor_id = vec![244; 16];
    combat.outcomes[0].target_id = vec![245; 16];
    let projection = project_display_view(&view).expect("fixture is valid");
    let rendered = format!("{projection:?}");

    for sentinel in [
        "private-room-sentinel",
        "private/join-sentinel",
        "private-creation-sentinel",
        "private-creation-revision",
        "private-character-sentinel",
        "private-sheet-revision",
        "private-asset-sentinel",
        "private-scene-asset-sentinel",
        "private-outcome-revision",
        "private-combat-revision",
        "241, 241",
        "242, 242",
        "243, 243",
        "244, 244",
        "245, 245",
    ] {
        assert!(!rendered.contains(sentinel), "leaked {sentinel}");
    }
}

#[test]
fn missing_or_unsupported_required_fields_fail_closed() {
    let mut view = display_view();
    view.scene = None;
    assert_eq!(
        project_display_view(&view),
        Err(DisplayProjectionError::MissingScene)
    );

    let mut view = display_view();
    view.journey = None;
    assert_eq!(
        project_display_view(&view),
        Err(DisplayProjectionError::MissingJourney)
    );

    let mut view = display_view();
    view.scene.as_mut().expect("fixture has scene").destination = 999;
    assert_eq!(
        project_display_view(&view),
        Err(DisplayProjectionError::UnsupportedScene)
    );

    let mut view = display_view();
    view.journey.as_mut().expect("fixture has journey").phase = 999;
    assert_eq!(
        project_display_view(&view),
        Err(DisplayProjectionError::UnsupportedPhase)
    );
}

#[test]
fn invalid_public_labels_and_resource_ranges_are_refused() {
    let mut view = display_view();
    view.scene.as_mut().expect("fixture has scene").title = " \n".to_owned();
    assert_eq!(
        project_display_view(&view),
        Err(DisplayProjectionError::InvalidSceneTitle)
    );

    let mut view = display_view();
    view.journey.as_mut().expect("fixture has journey").party[0].hit_points = 12;
    assert_eq!(
        project_display_view(&view),
        Err(DisplayProjectionError::InvalidHitPointRange)
    );
}

#[test]
fn unspecified_known_scene_has_no_invented_selection_and_caption_has_no_timing() {
    let mut view = display_view();
    view.scene.as_mut().expect("fixture has scene").destination =
        HarborDestination::Unspecified as i32;
    let projection = project_display_view(&view).expect("known unspecified scene is accepted");

    assert_eq!(projection.scene.kind, None);
    let caption = projection.dialogue_caption.expect("dialogue was supplied");
    assert_eq!(caption.speaker.as_deref(), Some("Mara"));
    assert_eq!(caption.text, "The courier points toward the wharf.");
}
