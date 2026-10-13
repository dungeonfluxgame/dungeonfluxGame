//! Executable contract examples for existing closed UI state and variant APIs.
//! These checks qualify bounded presentation contracts, not DOM output or gameplay.
use df_ui::{
    CampaignLimits, CampaignMember, CampaignObjective, CampaignValidationError, CampaignView,
    CharacterGroup, CharacterLimits, CharacterOption, CharacterPhaseView, CharacterStatus,
    CharacterValidationError, ConceptScene, FeedbackView, JoinFeedback, JoinPhaseView, JoinStage,
    JoinStamp, JoinValidationError, LobbyOffer, LobbyParticipant, MAX_DRAFT_UTF16_UNITS,
    ObjectiveState, ThemeToken, validate_join_drafts,
};

fn campaign<'a>(scene: ConceptScene) -> CampaignView<'a> {
    CampaignView {
        scene,
        chapter: "Chapter",
        title: "Campaign",
        description: "Permitted description",
        location: "Location",
        scene_label: "Scene",
        narration: "Permitted narration",
        connection: "Connection reported by owner",
        notice: "Notice",
        members: &[],
        objectives: &[],
    }
}
fn limits() -> CampaignLimits {
    CampaignLimits {
        max_members: 2,
        max_objectives: 3,
        max_text_bytes: 128,
    }
}
fn join<'a>(
    stage: JoinStage,
    scene: ConceptScene,
    feedback: JoinFeedback<'a>,
) -> JoinPhaseView<'a> {
    JoinPhaseView {
        stamp: JoinStamp {
            generation: 1,
            sequence: 2,
        },
        stage,
        campaign: campaign(scene),
        panel_heading: "Join",
        panel_description: "Permitted invitation",
        invitation_label: "Invitation",
        name_label: "Name",
        join_label: "Request join",
        join_enabled: true,
        feedback,
        room: None,
        roster_heading: "Members",
        empty_roster: "No permitted members",
        participants: &[],
        away_heroes: &[],
        lobby_offer: None,
        pairing_fallback: "Pairing unavailable",
    }
}
fn character() -> CharacterPhaseView {
    CharacterPhaseView {
        generation: 1,
        owner_key: "member-view".into(),
        revision: 2,
        chapter: "Chapter".into(),
        title: "Character".into(),
        description: "Permitted description".into(),
        connection: "Connected".into(),
        status: CharacterStatus::Editing,
        status_message: "Owner reports editing".into(),
        editable: true,
        name: "Local draft".into(),
        flavor: String::new(),
        appearance: None,
        portrait: None,
        groups: vec![CharacterGroup {
            id: "supplied-group".into(),
            label: "Supplied group".into(),
            description: "Permitted choice".into(),
            options: vec![CharacterOption {
                id: "supplied-option".into(),
                label: "Supplied option".into(),
                description: "Permitted option".into(),
                enabled: true,
                availability: "Owner reports available".into(),
                portrait: None,
            }],
            selected: Some("supplied-option".into()),
        }],
        facts: Vec::new(),
        actions: Vec::new(),
    }
}
fn character_limits() -> CharacterLimits {
    CharacterLimits {
        max_groups: 1,
        max_options: 1,
        max_facts: 0,
        max_actions: 0,
        max_text_bytes: 128,
    }
}

#[test]
fn composed_fixed_scenes_and_typed_states_validate_without_a_descriptor_interpreter() {
    let objectives = [
        CampaignObjective {
            label: "Pending objective",
            state: ObjectiveState::Pending,
        },
        CampaignObjective {
            label: "Active objective",
            state: ObjectiveState::Active,
        },
        CampaignObjective {
            label: "Reported complete objective",
            state: ObjectiveState::Complete,
        },
    ];
    for scene in [
        ConceptScene::Harbor,
        ConceptScene::MaraHarbor,
        ConceptScene::Tavern,
        ConceptScene::Mountain,
        ConceptScene::SunkenHall,
        ConceptScene::Campfire,
    ] {
        assert!(scene.asset_path().starts_with("assets/"));
        assert!(!scene.asset_path().contains(".."));
        for stage in [JoinStage::Title, JoinStage::Invitation, JoinStage::Lobby] {
            for feedback in [
                JoinFeedback::None,
                JoinFeedback::Pending("Owner reports pending"),
                JoinFeedback::Rejected("Owner reports refusal"),
            ] {
                let mut view = join(stage, scene, feedback);
                view.campaign.objectives = &objectives;
                assert_eq!(view.validate(limits()), Ok(()));
                assert_eq!(view.stage, stage);
                assert_eq!(view.campaign.scene, scene);
                assert_eq!(view.feedback, feedback);
                assert_eq!(
                    view.stamp,
                    JoinStamp {
                        generation: 1,
                        sequence: 2
                    }
                );
            }
        }
    }
}

#[test]
fn theme_selection_is_a_fixed_client_owned_palette_not_supplied_css() {
    let values = [
        ThemeToken::Background.css_value(),
        ThemeToken::Surface.css_value(),
        ThemeToken::Text.css_value(),
        ThemeToken::SecondaryText.css_value(),
        ThemeToken::StoryAccent.css_value(),
        ThemeToken::EngineAccent.css_value(),
    ];
    for value in values {
        assert_eq!(value.len(), 7);
        assert!(value.starts_with('#'));
        assert!(value.bytes().skip(1).all(|byte| byte.is_ascii_hexdigit()));
        assert!(!value.contains(';'));
    }
    assert_ne!(
        ThemeToken::StoryAccent.css_value(),
        ThemeToken::EngineAccent.css_value()
    );
}

#[test]
fn a_lobby_offer_cannot_select_an_unsupported_component_stage() {
    for stage in [JoinStage::Title, JoinStage::Invitation] {
        let mut view = join(stage, ConceptScene::Harbor, JoinFeedback::None);
        view.lobby_offer = Some(LobbyOffer {
            key: "supplied-ready",
            label: "Request ready",
            enabled: true,
            pending: false,
        });
        assert_eq!(
            view.validate(limits()),
            Err(JoinValidationError::InvalidStage)
        );
        assert_eq!(view.stage, stage);
    }
    let mut allowed = join(
        JoinStage::Lobby,
        ConceptScene::Harbor,
        JoinFeedback::Pending("Owner reports pending"),
    );
    allowed.lobby_offer = Some(LobbyOffer {
        key: "supplied-ready",
        label: "Request ready",
        enabled: false,
        pending: true,
    });
    assert_eq!(allowed.validate(limits()), Ok(()));
    assert!(!allowed.lobby_offer.as_ref().unwrap().enabled);
    assert!(allowed.lobby_offer.as_ref().unwrap().pending);
}

#[test]
fn markup_shaped_labels_remain_literal_data_and_cannot_create_an_advertised_choice() {
    let literal = "<img src=x onerror=alert(1)>";
    let mut view = join(
        JoinStage::Invitation,
        ConceptScene::Harbor,
        JoinFeedback::Rejected(literal),
    );
    view.panel_heading = literal;
    view.campaign.title = literal;
    assert_eq!(view.validate(limits()), Ok(()));
    assert_eq!(view.panel_heading, literal);
    assert_eq!(view.campaign.title, literal);

    let mut draft = character();
    draft.groups[0].selected = Some(literal.into());
    assert_eq!(
        draft.validate(character_limits()),
        Err(CharacterValidationError::UnknownSelection)
    );
    assert_eq!(draft.groups[0].options[0].id, "supplied-option");
}

#[test]
fn pending_rejected_and_unknown_are_not_reported_readiness_or_success() {
    let mut view = character();
    for status in [
        CharacterStatus::Editing,
        CharacterStatus::Pending,
        CharacterStatus::Rejected,
        CharacterStatus::Ready,
        CharacterStatus::Locked,
    ] {
        view.status = status;
        assert_eq!(view.validate(character_limits()), Ok(()));
        assert_eq!(view.status, status);
        assert_eq!(view.groups[0].selected.as_deref(), Some("supplied-option"));
        if status == CharacterStatus::Ready {
            assert_eq!(status.label(), "Server reports ready");
        } else {
            assert_ne!(status.label(), "Server reports ready");
        }
    }
    let feedback = FeedbackView::Error {
        uncertainty: "Original operation remains unknown",
        message: "Delivery failed",
    };
    assert_eq!(
        feedback,
        FeedbackView::Error {
            uncertainty: "Original operation remains unknown",
            message: "Delivery failed"
        }
    );
    assert_ne!(
        FeedbackView::Uncertain("Lookup required"),
        FeedbackView::Pending("Lookup required")
    );
    assert_ne!(
        FeedbackView::Refused("Refused"),
        FeedbackView::Pending("Refused")
    );
}

#[test]
fn owner_bounds_and_duplicate_keys_refuse_before_reconciliation() {
    let members = [
        CampaignMember {
            key: "same",
            name: "One",
            role: "Player",
            sigil: "One",
        },
        CampaignMember {
            key: "same",
            name: "Two",
            role: "Player",
            sigil: "Two",
        },
    ];
    let mut view = campaign(ConceptScene::Campfire);
    view.members = &members;
    assert_eq!(
        view.validate(limits()),
        Err(CampaignValidationError::DuplicateMemberKey)
    );
    let participants = [
        LobbyParticipant {
            key: "same",
            name: "One",
            sigil: "One",
            readiness: "Pending",
            presence: "Present",
        },
        LobbyParticipant {
            key: "same",
            name: "Two",
            sigil: "Two",
            readiness: "Ready",
            presence: "Present",
        },
    ];
    let mut lobby = join(JoinStage::Lobby, ConceptScene::Campfire, JoinFeedback::None);
    lobby.participants = &participants;
    assert_eq!(
        lobby.validate(limits()),
        Err(JoinValidationError::DuplicateParticipant)
    );
    let oversized = "x".repeat(limits().max_text_bytes + 1);
    lobby.participants = &[];
    lobby.panel_description = &oversized;
    assert_eq!(
        lobby.validate(limits()),
        Err(JoinValidationError::ResourceLimit)
    );
    lobby.panel_description = " ";
    assert_eq!(
        lobby.validate(limits()),
        Err(JoinValidationError::EmptyText)
    );
}

#[test]
fn draft_storage_limits_use_native_utf16_units_without_interpreting_invitation_grammar() {
    let at_limit = "😀".repeat(MAX_DRAFT_UTF16_UNITS / 2);
    assert_eq!(
        validate_join_drafts(&at_limit, "<script>literal draft</script>"),
        Ok(())
    );
    let over_limit = format!("{at_limit}x");
    assert_eq!(
        validate_join_drafts(&over_limit, "Name"),
        Err(JoinValidationError::DraftTooLong)
    );
    assert_eq!(
        validate_join_drafts("Invitation", &over_limit),
        Err(JoinValidationError::DraftTooLong)
    );
}
