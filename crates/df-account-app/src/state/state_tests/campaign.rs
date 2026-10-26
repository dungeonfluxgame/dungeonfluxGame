use super::*;
use crate::{
    api::JoinDecision,
    wire::{Accepted, PacePreset, PresetOffer, Refusal},
    wire_campaign::{
        AdventureOption, ApproveRequestBody, CampaignCreated, CampaignOverview, CampaignRole,
        ContentRating, DraftView, FundingNote, InvitePreview, JoinAcceptRequest, JoinAccepted,
        LobbyOpened, LobbyView, MemberRow, OperationRequest, PendingInvitation, Reason,
        RoomCodeLookupRequest, RoomCodeState, SaveDraftRequest, TokenBody, WizardCatalog,
    },
};

fn catalog() -> WizardCatalog {
    WizardCatalog {
        adventures: vec![
            AdventureOption {
                adventure_id: "a1".to_owned(),
                name: "The Drowned Lantern".to_owned(),
                summary: "A tavern and a flooded crypt.".to_owned(),
                ratings: vec![ContentRating::Teen],
                rules_text: "Fifth edition, 2024 rules.".to_owned(),
            },
            AdventureOption {
                adventure_id: "a2".to_owned(),
                name: "Salt and Bone".to_owned(),
                summary: "A harbour town.".to_owned(),
                ratings: vec![ContentRating::Teen, ContentRating::Mature],
                rules_text: "Fifth edition, 2024 rules.".to_owned(),
            },
        ],
        maximum_seats: 6,
        presets: vec![PresetOffer {
            preset: PacePreset::Story,
            ceiling_cents_per_hour: Some(300),
        }],
        is_proposed_pricing: true,
        custom_minimum_cents_per_hour: 100,
        custom_maximum_cents_per_hour: 10_000,
    }
}

fn draft_view(step: u8) -> DraftView {
    DraftView {
        draft_id: "d1".to_owned(),
        step,
        draft: super::super::campaign_flow::empty_draft(),
    }
}

fn wizard() -> Model {
    let mut model = booted(Route::NewCampaign, signed_in(true));
    deliver(&mut model, Reply::WizardCatalog(Ok(catalog())));
    deliver(&mut model, Reply::Draft(Ok(draft_view(1))));
    issue(&mut model, OperationSlot::Primary, "op-create");
    model
}

fn fill_step_one(model: &mut Model) {
    type_into(model, Field::CampaignName, "The Drowned Lantern");
    type_into(model, Field::Adventure, "a1");
    type_into(model, Field::Rating, "teen");
}

#[test]
fn an_unverified_account_starts_no_draft() {
    let mut model = Model::default();
    model.update(Msg::Started {
        route: Route::NewCampaign,
        offset_east_minutes: 0,
    });

    let effects = deliver(&mut model, Reply::Session(Ok(signed_in(false))));

    assert!(calls(&effects).is_empty());
    assert!(matches!(model.screen, Screen::NewCampaign(_)));
}

#[test]
fn step_one_refuses_each_missing_piece_in_order_without_a_request() {
    let mut model = wizard();
    if let Screen::NewCampaign(screen) = &mut model.screen {
        screen.draft.content_rating = None;
    }
    let mut refusals = Vec::new();
    for fill in [
        None,
        Some((Field::CampaignName, "The Drowned Lantern")),
        Some((Field::Adventure, "a1")),
        Some((Field::Rating, "teen")),
    ] {
        if let Some((field, value)) = fill {
            type_into(&mut model, field, value);
        }
        let effects = submit(&mut model, Form::WizardStep);
        refusals.push(crate::copy::refused_form_error(&model).and_then(|error| error.refusal));
        if refusals.last() == Some(&None) {
            assert_eq!(calls(&effects).len(), 1);
        } else {
            assert!(calls(&effects).is_empty());
        }
    }

    assert_eq!(
        refusals,
        vec![
            Some(Refusal::CampaignNameRequired),
            Some(Refusal::AdventureRequired),
            Some(Refusal::ContentRatingRequired),
            None,
        ]
    );
}

#[test]
fn a_campaign_name_over_the_limit_is_refused() {
    let mut model = wizard();
    fill_step_one(&mut model);
    type_into(&mut model, Field::CampaignName, &"x".repeat(61));

    submit(&mut model, Form::WizardStep);

    assert_eq!(
        crate::copy::refused_form_error(&model),
        Some(ApiError::refused(Refusal::CampaignNameTooLong))
    );
}

#[test]
fn more_seats_than_the_catalog_allows_are_refused() {
    let mut model = wizard();
    fill_step_one(&mut model);
    type_into(&mut model, Field::SeatCount, "7");

    submit(&mut model, Form::WizardStep);

    assert_eq!(
        crate::copy::refused_form_error(&model),
        Some(ApiError::refused(Refusal::SeatsOutOfRange))
    );
}

#[test]
fn saving_a_step_puts_the_whole_draft_to_that_step_and_the_reply_moves_on() {
    let mut model = wizard();
    fill_step_one(&mut model);

    let effects = submit(&mut model, Form::WizardStep);
    let moved = deliver(&mut model, Reply::Draft(Ok(draft_view(1))));

    let Some(ApiCall::SaveDraft {
        draft_id,
        step,
        request: SaveDraftRequest { draft },
    }) = first_call(&effects)
    else {
        panic!("a save");
    };
    assert_eq!((draft_id.as_str(), *step), ("d1", 1));
    assert_eq!(draft.name, "The Drowned Lantern");
    let Screen::NewCampaign(screen) = &model.screen else {
        panic!("wizard");
    };
    assert_eq!(screen.step, 2);
    assert!(has_effect(&moved, &Effect::Focus(FocusTarget::Heading)));
}

#[test]
fn going_back_keeps_everything_typed() {
    let mut model = wizard();
    fill_step_one(&mut model);
    submit(&mut model, Form::WizardStep);
    deliver(&mut model, Reply::Draft(Ok(draft_view(1))));

    click(&mut model, Intent::WizardBack);

    let Screen::NewCampaign(screen) = &model.screen else {
        panic!("wizard");
    };
    assert_eq!(screen.step, 1);
    assert_eq!(screen.draft.name, "The Drowned Lantern");
}

#[test]
fn a_draft_in_progress_resumes_at_its_saved_step() {
    let mut model = booted(Route::NewCampaign, signed_in(true));
    deliver(&mut model, Reply::WizardCatalog(Ok(catalog())));
    let mut saved = draft_view(4);
    saved.draft.name = "Half done".to_owned();

    deliver(&mut model, Reply::Draft(Ok(saved)));

    let Screen::NewCampaign(screen) = &model.screen else {
        panic!("wizard");
    };
    assert_eq!(screen.step, 4);
    assert_eq!(screen.draft.name, "Half done");
}

#[test]
fn changing_the_adventure_drops_a_rating_it_does_not_allow() {
    let mut model = wizard();
    type_into(&mut model, Field::Adventure, "a2");
    type_into(&mut model, Field::Rating, "mature");

    type_into(&mut model, Field::Adventure, "a1");

    let Screen::NewCampaign(screen) = &model.screen else {
        panic!("wizard");
    };
    assert_eq!(screen.draft.content_rating, None);
}

#[test]
fn a_custom_pace_needs_a_ceiling_the_catalog_allows_before_leaving_step_three() {
    let mut model = wizard();
    fill_step_one(&mut model);
    for step in 1..3 {
        submit(&mut model, Form::WizardStep);
        deliver(&mut model, Reply::Draft(Ok(draft_view(step))));
    }
    type_into(&mut model, Field::PacePreset, "custom");
    type_into(&mut model, Field::CustomCeiling, "500");

    let effects = submit(&mut model, Form::WizardStep);

    assert!(calls(&effects).is_empty());
    assert_eq!(
        crate::copy::refused_form_error(&model),
        Some(ApiError::refused(Refusal::CeilingOutOfRange))
    );
}

#[test]
fn creating_the_campaign_uses_its_operation_id_once_and_opens_it() {
    let mut model = wizard();

    let effects = submit(&mut model, Form::CreateCampaign);
    let again = submit(&mut model, Form::CreateCampaign);
    let opened = deliver(
        &mut model,
        Reply::CampaignCreated(Ok(CampaignCreated {
            campaign_id: "0c".to_owned(),
        })),
    );

    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::FinishDraft {
            draft_id: "d1".to_owned(),
            request: OperationRequest {
                operation: "op-create".to_owned()
            }
        })
    );
    assert!(calls(&again).is_empty());
    assert!(matches!(model.screen, Screen::Campaign(_)));
    assert!(has_effect(
        &opened,
        &Effect::PushUrl("/app/campaigns/0c".to_owned())
    ));
}

fn overview() -> CampaignOverview {
    CampaignOverview {
        campaign_id: "0c".to_owned(),
        name: "The Drowned Lantern".to_owned(),
        my_role: CampaignRole::Host,
        can_invite: true,
        can_open_lobby: true,
        members: vec![MemberRow {
            display_name: "Ana".to_owned(),
            role: CampaignRole::Host,
            is_me: true,
        }],
        invitations: Vec::new(),
        join_requests: Vec::new(),
        room_code: RoomCodeState::Off,
        live_session_id: None,
        last_saved_at_ms: None,
    }
}

fn campaign_model() -> Model {
    let mut model = booted(
        Route::Campaign {
            campaign_id: "0c".to_owned(),
        },
        signed_in(true),
    );
    deliver(&mut model, Reply::Campaign(Ok(overview())));
    issue(&mut model, OperationSlot::Primary, "op-1");
    model
}

#[test]
fn an_invitation_link_is_kept_only_in_memory_for_the_screen_and_a_new_id_is_requested() {
    let mut model = campaign_model();

    let effects = click(
        &mut model,
        Intent::CreateInvitation(crate::wire_campaign::InvitationKind::Player),
    );
    let done = deliver(
        &mut model,
        Reply::InvitationCreated(Ok(crate::wire_campaign::InvitationCreated {
            invitation_id: "i1".to_owned(),
            link: "https://example.test/app/join#token=0a".to_owned(),
        })),
    );

    assert!(matches!(
        first_call(&effects),
        Some(ApiCall::CreateInvitation { .. })
    ));
    assert!(done.iter().any(|effect| matches!(
        effect,
        Effect::IssueOperation {
            slot: OperationSlot::Primary,
            ..
        }
    )));
    assert_eq!(
        calls(&done),
        vec![&ApiCall::Campaign {
            campaign_id: "0c".to_owned()
        }]
    );
}

#[test]
fn each_operation_id_is_used_for_one_action_only() {
    let mut model = campaign_model();

    let first = click(&mut model, Intent::OpenLobby);
    deliver(
        &mut model,
        Reply::LobbyOpened(Err(ApiError::because(
            ErrorCode::Forbidden,
            Reason::StaleRevision,
        ))),
    );
    // The refusal asked for a new id, which has not arrived: nothing can be sent yet.
    let blocked = click(&mut model, Intent::OpenLobby);
    issue(&mut model, OperationSlot::Primary, "op-2");
    let retried = click(&mut model, Intent::OpenLobby);

    assert!(first_call(&first).is_some());
    assert!(calls(&blocked).is_empty());
    let Some(ApiCall::OpenLobby { request, .. }) = first_call(&retried) else {
        panic!("a second lobby request");
    };
    assert_eq!(request.operation, "op-2");
}

#[test]
fn leaving_asks_first_and_a_refusal_names_the_reason() {
    let mut model = campaign_model();

    let asked = click(&mut model, Intent::AskLeave);
    assert!(calls(&asked).is_empty());
    let sent = click(&mut model, Intent::LeaveCampaign);
    deliver(
        &mut model,
        Reply::Left(Err(ApiError::of(ErrorCode::Forbidden))),
    );

    assert!(matches!(
        first_call(&sent),
        Some(ApiCall::LeaveCampaign { .. })
    ));
    let Screen::Campaign(screen) = &model.screen else {
        panic!("campaign");
    };
    assert!(!screen.is_confirming_leave);
    assert!(matches!(screen.action, Submit::Refused(_)));
}

#[test]
fn approving_a_room_code_request_sends_the_chosen_role() {
    let mut model = campaign_model();

    let effects = click(
        &mut model,
        Intent::DecideRequest {
            request_id: "r1".to_owned(),
            decision: RequestDecision::Approve(crate::wire_campaign::InvitationKind::Contributor),
        },
    );

    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::DecideJoinRequest {
            campaign_id: "0c".to_owned(),
            request_id: "r1".to_owned(),
            decision: JoinDecision::Approve(ApproveRequestBody {
                role: crate::wire_campaign::InvitationKind::Contributor,
                operation: "op-1".to_owned(),
            }),
        })
    );
}

fn preview() -> InvitePreview {
    InvitePreview {
        campaign_name: "The Drowned Lantern".to_owned(),
        host_name: "Ana".to_owned(),
        scheduled_at_ms: None,
        content_rating: ContentRating::Teen,
    }
}

fn join_route(token: &str) -> Route {
    Route::Join {
        token: Some(token.to_owned()),
    }
}

#[test]
fn opening_an_invitation_reads_its_preview_with_the_token_in_the_body() {
    let mut model = Model::default();
    model.update(Msg::Started {
        route: join_route("0a1b"),
        offset_east_minutes: 0,
    });

    let effects = deliver(&mut model, Reply::Session(Ok(anonymous())));

    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::JoinPreview(TokenBody {
            token: "0a1b".to_owned()
        }))
    );
    assert_eq!(model.route, join_route("0a1b"));
}

#[test]
fn a_signed_out_invitee_has_the_invitation_remembered_then_goes_to_sign_up() {
    let mut model = booted(join_route("0a1b"), anonymous());
    deliver(&mut model, Reply::JoinPreview(Ok(preview())));

    let held = click(
        &mut model,
        Intent::HoldInvitation {
            then: Route::SignUp,
        },
    );
    let went = deliver(&mut model, Reply::JoinHeld(Ok(Accepted { accepted: true })));

    assert_eq!(
        first_call(&held),
        Some(&ApiCall::JoinHold(TokenBody {
            token: "0a1b".to_owned()
        }))
    );
    assert!(matches!(model.screen, Screen::SignUp(_)));
    assert!(has_effect(
        &went,
        &Effect::PushUrl("/app/sign-up".to_owned())
    ));
}

#[test]
fn a_signed_out_person_cannot_accept_an_invitation_directly() {
    let mut model = booted(join_route("0a1b"), anonymous());

    assert!(calls(&click(&mut model, Intent::AcceptInvitation)).is_empty());
}

#[test]
fn a_signed_in_invitee_accepts_with_the_link_token_and_lands_on_the_campaign() {
    let mut model = booted(join_route("0a1b"), signed_in(true));
    issue(&mut model, OperationSlot::Primary, "op-join");
    deliver(&mut model, Reply::JoinPreview(Ok(preview())));

    let effects = click(&mut model, Intent::AcceptInvitation);
    deliver(
        &mut model,
        Reply::Joined(Ok(JoinAccepted {
            campaign_id: "0c".to_owned(),
        })),
    );

    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::JoinAccept(JoinAcceptRequest {
            token: Some("0a1b".to_owned()),
            operation: "op-join".to_owned(),
        }))
    );
    assert!(matches!(model.screen, Screen::Campaign(_)));
}

#[test]
fn a_refused_invitation_keeps_only_the_reason_and_no_table_details() {
    let mut model = booted(join_route("0a1b"), anonymous());
    let error = ApiError::because(ErrorCode::NotFound, Reason::Revoked);

    deliver(&mut model, Reply::JoinPreview(Err(error)));

    let Screen::Join(screen) = &model.screen else {
        panic!("join");
    };
    assert_eq!(screen.preview, Some(Remote::Failed(error)));
    assert_eq!(screen.code_preview, None);
}

#[test]
fn a_room_code_is_looked_up_then_requested_and_the_host_approves_it() {
    let mut model = booted(Route::Join { token: None }, signed_in(true));
    issue(&mut model, OperationSlot::Primary, "op-code");
    type_into(&mut model, Field::RoomCode, " 7KQ3MZ ");

    let looked_up = submit(&mut model, Form::CodeLookup);
    deliver(&mut model, Reply::JoinPreview(Ok(preview())));
    let asked = click(&mut model, Intent::AskToJoin);
    deliver(
        &mut model,
        Reply::CodeRequested(Ok(crate::wire_campaign::JoinRequested {
            campaign_name: "The Drowned Lantern".to_owned(),
        })),
    );

    assert_eq!(
        first_call(&looked_up),
        Some(&ApiCall::CodeLookup(RoomCodeLookupRequest {
            code: "7KQ3MZ".to_owned()
        }))
    );
    assert!(matches!(first_call(&asked), Some(ApiCall::CodeRequest(_))));
    let Screen::Join(screen) = &model.screen else {
        panic!("join");
    };
    assert!(screen.requested.is_some());
}

#[test]
fn a_room_code_cannot_be_looked_up_while_signed_out() {
    let mut model = booted(Route::Join { token: None }, anonymous());
    type_into(&mut model, Field::RoomCode, "7KQ3MZ");

    assert!(calls(&submit(&mut model, Form::CodeLookup)).is_empty());
}

#[test]
fn the_dashboard_offers_the_remembered_invitation_and_accepts_it_without_a_token() {
    let mut model = booted(Route::Dashboard, signed_in(true));
    let effects = deliver(
        &mut model,
        Reply::JoinPending(Ok(PendingInvitation {
            preview: Some(preview()),
        })),
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::IssueOperation {
            slot: OperationSlot::Secondary,
            ..
        }
    )));
    issue(&mut model, OperationSlot::Secondary, "op-pending");

    let accepted = click(&mut model, Intent::AcceptPendingInvitation);

    assert_eq!(
        first_call(&accepted),
        Some(&ApiCall::JoinAccept(JoinAcceptRequest {
            token: None,
            operation: "op-pending".to_owned(),
        }))
    );
}

fn lobby_view(poll_seconds: u32, has_started: bool) -> LobbyView {
    LobbyView {
        session_id: "5e".to_owned(),
        campaign_id: "0c".to_owned(),
        campaign_name: "The Drowned Lantern".to_owned(),
        can_manage: true,
        start_blockers: Vec::new(),
        members: Vec::new(),

        funding_note: FundingNote::NoChargeUntilHostStarts,
        has_started,
        poll_seconds,
    }
}

fn lobby_model() -> Model {
    let mut model = booted(
        Route::Lobby {
            session_id: "5e".to_owned(),
        },
        signed_in(true),
    );
    deliver(&mut model, Reply::Lobby(Ok(lobby_view(5, false))));
    issue(&mut model, OperationSlot::Primary, "op-lobby");
    model
}

#[test]
fn an_open_lobby_reads_itself_again_after_the_poll_interval_and_a_started_one_stops() {
    let mut open = booted(
        Route::Lobby {
            session_id: "5e".to_owned(),
        },
        signed_in(true),
    );
    let mut started = open.clone();

    let waiting = deliver(&mut open, Reply::Lobby(Ok(lobby_view(5, false))));
    let stopped = deliver(&mut started, Reply::Lobby(Ok(lobby_view(5, true))));
    let refreshed = open.update(Msg::TimerFired {
        generation: open.generation,
    });

    assert!(waiting.iter().any(|effect| matches!(
        effect,
        Effect::After {
            delay_ms: 5_000,
            ..
        }
    )));
    assert!(stopped.is_empty());
    assert_eq!(
        first_call(&refreshed),
        Some(&ApiCall::Lobby {
            session_id: "5e".to_owned()
        })
    );
}

#[test]
fn opening_the_lobby_from_the_campaign_goes_to_the_lobby_screen() {
    let mut model = campaign_model();
    click(&mut model, Intent::OpenLobby);

    deliver(
        &mut model,
        Reply::LobbyOpened(Ok(LobbyOpened {
            session_id: "5e".to_owned(),
        })),
    );

    assert!(matches!(model.screen, Screen::Lobby(_)));
}

fn host_quote() -> crate::wire_campaign::HostCreditView {
    crate::wire_campaign::HostCreditView {
        price_version: [7; 16],
        hourly_micro_dollars: 3_000_000,
        available_micro_dollars: 10_000_000,
        cap_micro_dollars: 3_000_000,
        terms_version: 1,
        consented_at_ms: None,
        is_fundable: false,
    }
}

#[test]
fn fundinghook_consent_requires_an_explicit_tick_and_posts_the_displayed_version() {
    let mut model = lobby_model();
    let mut lobby = lobby_view(3, false);
    lobby.funding_note = FundingNote::HostCredit(host_quote());
    deliver(&mut model, Reply::Lobby(Ok(lobby)));
    assert!(click(&mut model, Intent::Submit(Form::Action)).is_empty());
    model.update(Msg::Toggle(Field::FundingChoice, true));
    let effects = click(&mut model, Intent::Submit(Form::Action));
    assert!(
        matches!(first_call(&effects),Some(ApiCall::ConsentHostFunding { request, .. })
        if request.price_version == [7;16] && request.cap_micro_dollars == 3_000_000
        && request.terms_version == 1)
    );
    assert!(click(&mut model, Intent::Submit(Form::Action)).is_empty());
}

#[test]
fn fundinghook_a_changed_quote_clears_the_previous_tick() {
    let mut model = lobby_model();
    let mut lobby = lobby_view(3, false);
    lobby.funding_note = FundingNote::HostCredit(host_quote());
    deliver(&mut model, Reply::Lobby(Ok(lobby.clone())));
    model.update(Msg::Toggle(Field::FundingChoice, true));
    let mut changed = host_quote();
    changed.price_version = [8; 16];
    lobby.funding_note = FundingNote::HostCredit(changed);
    deliver(&mut model, Reply::Lobby(Ok(lobby)));
    assert!(click(&mut model, Intent::Submit(Form::Action)).is_empty());
    assert!(matches!(&model.screen,Screen::Lobby(screen) if !screen.consent_checked));
}

#[test]
fn fundinghook_a_sponsored_player_cannot_submit_a_host_consent() {
    let mut model = lobby_model();
    model.update(Msg::Toggle(Field::FundingChoice, true));
    assert!(click(&mut model, Intent::Submit(Form::Action)).is_empty());
}

#[test]
fn lobbyfix_play_now_has_one_request_without_any_wizard_fields() {
    let mut model = booted(Route::Dashboard, signed_in(true));
    issue(&mut model, OperationSlot::Primary, "play-once");
    let effects = click(&mut model, Intent::PlayNow);
    assert_eq!(
        calls(&effects),
        vec![&ApiCall::PlayNow(OperationRequest {
            operation: "play-once".to_owned()
        })]
    );
    assert!(matches!(model.screen, Screen::Dashboard(_)));
    assert!(click(&mut model, Intent::PlayNow).is_empty());
    deliver(
        &mut model,
        Reply::PlayNow(Err(ApiError::of(ErrorCode::UnknownOutcome))),
    );
    let retry = click(&mut model, Intent::PlayNow);
    assert_eq!(first_call(&retry), first_call(&effects));
}

#[test]
fn play_now_goes_to_the_pre_game_launcher_once_started_and_to_the_session_otherwise() {
    for (started, launcher) in [(true, true), (false, false)] {
        let mut model = booted(Route::Dashboard, signed_in(true));
        issue(&mut model, OperationSlot::Primary, "play-once");
        click(&mut model, Intent::PlayNow);
        let effects = deliver(
            &mut model,
            Reply::PlayNow(Ok(crate::wire_campaign::PlayNowView {
                campaign_id: "0c".to_owned(),
                game_unavailable: true,
                lobby: lobby_view(5, started),
            })),
        );
        assert_eq!(model.play_operation, Remote::Loading);
        if launcher {
            assert!(matches!(model.screen, Screen::Play(_)));
            assert!(calls(&effects).contains(&&ApiCall::GameLaunch {
                session_id: "5e".to_owned()
            }));
        } else {
            assert!(matches!(model.screen, Screen::Lobby(_)));
        }
        assert!(
            !calls(&effects)
                .iter()
                .any(|call| matches!(call, ApiCall::StartDraft | ApiCall::WizardCatalog))
        );
    }
}

#[test]
fn a_start_goes_to_the_launcher_and_a_refused_or_blocked_start_stays_on_the_session() {
    let mut model = lobby_model();
    click(&mut model, Intent::StartSession);
    deliver(&mut model, Reply::Lobby(Ok(lobby_view(5, true))));
    assert!(matches!(model.screen, Screen::Play(_)));
    for result in [
        Ok(lobby_view(5, false)),
        Err(ApiError::of(ErrorCode::Forbidden)),
    ] {
        let mut model = lobby_model();
        click(&mut model, Intent::StartSession);
        deliver(&mut model, Reply::Lobby(result));
        assert!(matches!(model.screen, Screen::Lobby(_)));
    }
}

fn launch_view(launcher_url: Option<&str>) -> crate::wire_campaign::GameLaunchView {
    crate::wire_campaign::GameLaunchView {
        session_id: "5e".to_owned(),
        campaign_name: "The Drowned Lantern".to_owned(),
        has_started: true,
        launcher_url: launcher_url.map(str::to_owned),
        poll_seconds: 3,
    }
}

#[test]
fn the_launcher_asks_again_until_the_game_is_reachable_then_holds_still() {
    let mut model = booted(
        Route::Play {
            session_id: "5e".to_owned(),
        },
        signed_in(true),
    );
    let waiting = deliver(&mut model, Reply::GameLaunch(Ok(launch_view(None))));
    assert!(waiting.iter().any(|effect| matches!(
        effect,
        Effect::After {
            delay_ms: 3_000,
            ..
        }
    )));
    let ready = deliver(
        &mut model,
        Reply::GameLaunch(Ok(launch_view(Some("http://127.0.0.1:8770/")))),
    );
    assert!(
        !ready
            .iter()
            .any(|effect| matches!(effect, Effect::After { .. }))
    );
}

#[test]
fn a_lobby_action_does_not_start_a_second_polling_chain() {
    let mut model = lobby_model();

    click(&mut model, Intent::StartSession);
    let effects = deliver(&mut model, Reply::Lobby(Ok(lobby_view(5, false))));

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::After { .. }))
    );
}
