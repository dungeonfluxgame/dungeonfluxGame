use df_player::{GameplayViewError, PlayerGameplayProjection};
use df_protocol::common::{
    DisplayGameplayView, GameplayActionOffer, PlayerGameplayView, RecoveryEpoch,
    SessionRevision as WireRevision, ViewMessage, view_message::Audience,
};
use df_types::{ClientBindingId, RecoveryEpoch as DomainEpoch, SessionRevision};
use df_ui::{CampaignView, ConceptScene, ExplorationInput, ExplorationLimits, ExplorationView};

fn revision(sequence: u64) -> SessionRevision {
    SessionRevision::new(
        DomainEpoch::new(4).expect("nonzero fixture epoch"),
        sequence,
    )
}

fn wire_revision(sequence: u64) -> WireRevision {
    WireRevision {
        epoch: Some(RecoveryEpoch { value: Some(4) }),
        sequence: Some(sequence),
    }
}

fn binding(value: u8) -> ClientBindingId {
    ClientBindingId::from_bytes(&[value; 16]).expect("nonzero binding fixture")
}

fn offer(id: &str, kind: i32) -> GameplayActionOffer {
    GameplayActionOffer {
        offer_id: id.to_owned(),
        action_kind: kind,
        label: "Ask the courier".to_owned(),
        destinations: Vec::new(),
        input_groups: Vec::new(),
    }
}

fn player_message(sequence: Option<u64>) -> ViewMessage {
    ViewMessage {
        revision: sequence.map(wire_revision),
        audience: Some(Audience::Player(PlayerGameplayView {
            offer_id: String::new(),
            narration: "The courier waits beneath the lanterns.".to_owned(),
            check: None,
            private_clue: "The sealed packet bears a second mark.".to_owned(),
            action_available: true,
            offers: vec![offer("opaque:ask-courier:v7", 4)],
            scene: None,
            journey: None,
        })),
    }
}

fn limits(max_choices: usize) -> ExplorationLimits {
    ExplorationLimits {
        campaign: df_ui::CampaignLimits {
            max_members: 4,
            max_objectives: 8,
            max_text_bytes: 128,
        },
        max_choices,
        max_text_bytes: 128,
        max_identifier_bytes: 64,
    }
}

fn template<'a>(scope: ClientBindingId, current_revision: SessionRevision) -> ExplorationView<'a> {
    ExplorationView {
        binding: scope,
        revision: current_revision,
        campaign: CampaignView {
            scene: ConceptScene::Harbor,
            chapter: "Chapter One",
            title: "Greyhaven",
            description: "The harbor district.",
            location: "Lantern Wharf",
            scene_label: "Harbor",
            narration: "Template narration",
            connection: "Connected",
            notice: "Old private notice",
            members: &[],
            objectives: &[],
        },
        npc: None,
        heading: "Exploration",
        choices: &[],
        draft_label: "Message",
        submit_label: "Send",
        draft_offer_id: Some("old-draft-offer"),
        pending: None,
        rejection: None,
    }
}

fn project(message: &ViewMessage) -> Result<PlayerGameplayProjection<'_>, GameplayViewError> {
    PlayerGameplayProjection::project(message, binding(1), revision(9), limits(12))
}

#[test]
fn player_projection_preserves_scope_revision_private_fact_and_opaque_offer() {
    let message = player_message(Some(9));
    let projection = project(&message).expect("matching player view is admitted");
    let view = projection
        .exploration_view(&template(binding(1), revision(9)))
        .expect("same-scope caller context composes");

    assert_eq!(view.binding, binding(1));
    assert_eq!(view.revision, revision(9));
    assert_eq!(
        view.campaign.narration,
        "The courier waits beneath the lanterns."
    );
    assert_eq!(
        view.campaign.notice,
        "The sealed packet bears a second mark."
    );
    assert_eq!(view.choices.len(), 1);
    assert_eq!(view.choices[0].id, "opaque:ask-courier:v7");
    assert_eq!(view.draft_offer_id, None);

    let accepted = projection
        .accept_input(
            binding(1),
            ExplorationInput::Choice {
                id: view.choices[0].id.to_owned(),
                revision: view.revision,
            },
        )
        .expect("exact advertised input is retained");
    assert_eq!(
        accepted,
        ExplorationInput::Choice {
            id: "opaque:ask-courier:v7".to_owned(),
            revision: revision(9),
        }
    );
}

#[test]
fn missing_stale_and_absent_nested_revisions_are_refused() {
    let missing = player_message(None);
    assert_eq!(
        project(&missing).err(),
        Some(GameplayViewError::MissingRevision)
    );

    let stale = player_message(Some(8));
    assert_eq!(
        project(&stale).err(),
        Some(GameplayViewError::StaleRevision)
    );

    let mut absent_epoch = player_message(Some(9));
    absent_epoch.revision = Some(WireRevision {
        epoch: None,
        sequence: Some(9),
    });
    assert_eq!(
        project(&absent_epoch).err(),
        Some(GameplayViewError::InvalidRevision)
    );

    let mut zero_epoch = player_message(Some(9));
    zero_epoch
        .revision
        .as_mut()
        .unwrap()
        .epoch
        .as_mut()
        .unwrap()
        .value = Some(0);
    assert_eq!(
        project(&zero_epoch).err(),
        Some(GameplayViewError::InvalidRevision)
    );
}

#[test]
fn display_or_missing_audience_never_produces_a_player_projection() {
    let display = ViewMessage {
        revision: Some(wire_revision(9)),
        audience: Some(Audience::Display(DisplayGameplayView {
            narration: "Public narration only".to_owned(),
            scene_asset: String::new(),
            scene: None,
            journey: None,
            ..Default::default()
        })),
    };
    assert_eq!(
        project(&display).err(),
        Some(GameplayViewError::DisplayAudience)
    );

    let missing = ViewMessage {
        revision: Some(wire_revision(9)),
        audience: None,
    };
    assert_eq!(
        project(&missing).err(),
        Some(GameplayViewError::MissingAudience)
    );
}

#[test]
fn mismatched_template_or_input_binding_is_refused() {
    let message = player_message(Some(9));
    let projection = project(&message).expect("matching player view is admitted");
    assert_eq!(
        projection
            .exploration_view(&template(binding(2), revision(9)))
            .err(),
        Some(GameplayViewError::BindingMismatch)
    );
    assert_eq!(
        projection.accept_input(
            binding(2),
            ExplorationInput::Choice {
                id: "opaque:ask-courier:v7".to_owned(),
                revision: revision(9),
            },
        ),
        Err(GameplayViewError::BindingMismatch)
    );
}

#[test]
fn absent_private_clue_clears_prior_notice_and_draft_contract() {
    let mut message = player_message(Some(9));
    if let Some(Audience::Player(player)) = message.audience.as_mut() {
        player.private_clue.clear();
    }
    let projection = project(&message).expect("matching player view is admitted");
    let view = projection
        .exploration_view(&template(binding(1), revision(9)))
        .expect("current projection clears stale private context");
    assert_eq!(view.campaign.notice, "No private notice.");
    assert_eq!(view.draft_offer_id, None);

    assert_eq!(
        projection.accept_input(
            binding(1),
            ExplorationInput::Draft {
                id: "opaque:ask-courier:v7".to_owned(),
                text: "hello".to_owned(),
                revision: revision(9),
            },
        ),
        Err(GameplayViewError::UnsupportedInput)
    );
}

#[test]
fn action_input_requires_the_exact_current_offer_and_revision() {
    let message = player_message(Some(9));
    let projection = project(&message).expect("matching player view is admitted");
    assert_eq!(
        projection.accept_input(
            binding(1),
            ExplorationInput::Choice {
                id: "opaque:ask-courier:v7".to_owned(),
                revision: revision(8),
            },
        ),
        Err(GameplayViewError::StaleRevision)
    );
    assert_eq!(
        projection.accept_input(
            binding(1),
            ExplorationInput::Choice {
                id: "invented-offer".to_owned(),
                revision: revision(9),
            },
        ),
        Err(GameplayViewError::UnofferedInput)
    );
}

#[test]
fn unsupported_or_malformed_offer_shapes_are_refused_before_projection() {
    let unsupported_mutations: [fn(&mut PlayerGameplayView); 4] = [
        |player| player.offers[0].action_kind = 0,
        |player| player.offers[0].action_kind = 99,
        |player| player.offers[0].destinations.push(Default::default()),
        |player| player.offers[0].input_groups.push(Default::default()),
    ];
    for mutate in unsupported_mutations {
        let mut message = player_message(Some(9));
        if let Some(Audience::Player(player)) = message.audience.as_mut() {
            mutate(player);
        }
        assert_eq!(
            project(&message).err(),
            Some(GameplayViewError::UnsupportedOffer)
        );
    }

    let invalid_mutations: [fn(&mut PlayerGameplayView); 3] = [
        |player| player.offers[0].offer_id.clear(),
        |player| player.offers[0].label.clear(),
        |player| player.offers.push(player.offers[0].clone()),
    ];
    for mutate in invalid_mutations {
        let mut message = player_message(Some(9));
        if let Some(Audience::Player(player)) = message.audience.as_mut() {
            mutate(player);
        }
        assert_eq!(
            project(&message).err(),
            Some(GameplayViewError::InvalidOffer)
        );
    }
}

#[test]
fn offer_count_and_text_limits_are_checked_before_choices_are_created() {
    let message = player_message(Some(9));
    let bounded = PlayerGameplayProjection::project(&message, binding(1), revision(9), limits(0));
    assert_eq!(bounded.err(), Some(GameplayViewError::OfferLimit));

    let mut oversized = player_message(Some(9));
    if let Some(Audience::Player(player)) = oversized.audience.as_mut() {
        player.offers[0].label = "x".repeat(129);
    }
    assert_eq!(
        project(&oversized).err(),
        Some(GameplayViewError::InvalidOffer)
    );

    let mut oversized_id = player_message(Some(9));
    if let Some(Audience::Player(player)) = oversized_id.audience.as_mut() {
        player.offers[0].offer_id = "x".repeat(65);
    }
    assert_eq!(
        project(&oversized_id).err(),
        Some(GameplayViewError::InvalidOffer)
    );
}

#[test]
fn stale_template_revision_cannot_supply_old_private_context() {
    let message = player_message(Some(9));
    let projection = project(&message).expect("matching player view is admitted");
    assert_eq!(
        projection
            .exploration_view(&template(binding(1), revision(8)))
            .err(),
        Some(GameplayViewError::StaleRevision)
    );
}
