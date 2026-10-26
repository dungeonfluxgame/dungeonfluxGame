use crate::{
    node::{Element, Node},
    route::Route,
    state::{
        CampaignScreen, CampaignSettingsScreen, Field, Form, Intent, JoinScreen, LobbyScreen,
        Model, PlayScreen, Remote, RequestDecision, Screen, Submit,
    },
    view::view,
    wire::{AccountView, ApiError, ErrorCode, PacePreset, SessionView},
    wire_campaign::{
        CampaignDraft, CampaignOverview, CampaignRole, ContentRating, FundingModel, FundingNote,
        GameLaunchView, HeroChoice, InvitationCreated, InvitationKind, InvitationRow,
        InvitePreview, JoinMode, JoinRequestRow, JoinRequested, LobbyMember, LobbyView, MemberRow,
        PictureDefault, Reason, RoomCodeState, StartBlocker,
    },
};

const SECRET_TOKEN: &str = "TOKEN-abc123xyz-secret";

fn signed_in(is_verified: bool) -> Model {
    let mut model = Model::default();
    model.session = Remote::Ready(SessionView {
        google_enabled: false,
        csrf: "csrf".to_owned(),
        account: Some(AccountView {
            email: "ana@example.com".to_owned(),
            display_name: Some("Ana".to_owned()),
            is_verified,
            operator: None,
        }),
        dev_sign_in: None,
    });
    model
}

fn signed_out() -> Model {
    let mut model = Model::default();
    model.session = Remote::Ready(SessionView {
        google_enabled: false,
        csrf: "csrf".to_owned(),
        account: None,
        dev_sign_in: None,
    });
    model
}

fn with_screen(mut model: Model, screen: Screen) -> Model {
    model.screen = screen;
    model
}

fn all_elements(tree: &Node) -> Vec<&Element> {
    tree.elements()
}

fn element_text(element: &Element) -> String {
    Node::Element(element.clone()).text_content()
}

/// The button or link whose whole visible text is `label`.
fn control<'a>(tree: &'a Node, label: &str) -> &'a Element {
    all_elements(tree)
        .into_iter()
        .find(|element| matches!(element.tag, "button" | "a") && element_text(element) == label)
        .unwrap_or_else(|| panic!("no control labelled {label:?} in: {}", tree.text_content()))
}

fn has_control(tree: &Node, label: &str) -> bool {
    all_elements(tree)
        .into_iter()
        .any(|element| matches!(element.tag, "button" | "a") && element_text(element) == label)
}

fn is_disabled(element: &Element) -> bool {
    element.attribute("disabled").is_some()
}

fn bound(tree: &Node, field: Field) -> Vec<&Element> {
    all_elements(tree)
        .into_iter()
        .filter(|element| element.field == Some(field))
        .collect()
}

fn count_tag(tree: &Node, tag: &str) -> usize {
    all_elements(tree)
        .into_iter()
        .filter(|element| element.tag == tag)
        .count()
}

fn assert_one_heading_and_labelled_inputs(tree: &Node) {
    assert_eq!(
        count_tag(tree, "h1"),
        1,
        "exactly one h1 in: {}",
        tree.text_content()
    );
    let labels: Vec<&str> = all_elements(tree)
        .into_iter()
        .filter(|element| element.tag == "label")
        .filter_map(|label| label.attribute("for"))
        .collect();
    for element in all_elements(tree) {
        if matches!(element.tag, "input" | "select" | "textarea") {
            let id = element.attribute("id").expect("an input has an id");
            assert!(labels.contains(&id), "input {id} has no label");
        }
    }
}

fn assert_no_pricing_internals(text: &str) {
    let lowered = text.to_lowercase();
    for word in [
        "multiplier",
        "provider",
        "markup",
        "table fee",
        "platform fee",
        " fee ",
    ] {
        assert!(
            !lowered.contains(word),
            "{word:?} must not reach a customer screen: {text}"
        );
    }
}

fn assert_token_nowhere(tree: &Node) {
    assert!(
        !tree.text_content().contains(SECRET_TOKEN),
        "token printed as text"
    );
    for element in all_elements(tree) {
        for (name, value) in &element.attributes {
            assert!(!value.contains(SECRET_TOKEN), "token in attribute {name}");
        }
    }
}

// ---------------------------------------------------------------- overview fixtures

fn overview_data() -> CampaignOverview {
    CampaignOverview {
        campaign_id: "camp-1".to_owned(),
        name: "Friday table".to_owned(),
        my_role: CampaignRole::Host,
        can_invite: true,
        can_open_lobby: true,
        members: vec![
            MemberRow {
                display_name: "Ana".to_owned(),
                role: CampaignRole::Host,
                is_me: true,
            },
            MemberRow {
                display_name: "Ben".to_owned(),
                role: CampaignRole::Player,
                is_me: false,
            },
        ],
        invitations: vec![InvitationRow {
            invitation_id: "inv-1".to_owned(),
            kind: InvitationKind::Player,
            uses: 1,
            maximum_uses: 4,
            expires_at_ms: 1_800_000_000_000,
        }],
        join_requests: vec![JoinRequestRow {
            request_id: "req-1".to_owned(),
            display_name: "Cleo".to_owned(),
            requested_at_ms: 1_790_000_000_000,
        }],
        room_code: RoomCodeState::Active {
            code: "WXYZ-2345".to_owned(),
        },
        live_session_id: None,
        last_saved_at_ms: Some(1_790_000_000_000),
    }
}

fn campaign_screen(overview: CampaignOverview) -> CampaignScreen {
    CampaignScreen {
        campaign_id: "camp-1".to_owned(),
        overview: Remote::Ready(overview),
        operation: Remote::Ready("operation-1".to_owned()),
        action: Submit::Idle,
        created_link: None,
        is_confirming_leave: false,
        settings: None,
        settings_error: None,
        can_edit_settings: false,
    }
}

fn overview_page(screen: CampaignScreen) -> Node {
    view(&with_screen(signed_in(true), Screen::Campaign(screen)))
}

// ---------------------------------------------------------------- overview

#[test]
fn the_overview_says_it_is_loading_and_has_one_heading() {
    let mut screen = campaign_screen(overview_data());
    screen.overview = Remote::Loading;
    let tree = overview_page(screen);
    assert_one_heading_and_labelled_inputs(&tree);
    assert!(tree.text_content().contains("Opening your campaign..."));
}

#[test]
fn a_failed_overview_offers_try_again() {
    let mut screen = campaign_screen(overview_data());
    screen.overview = Remote::Failed(ApiError::of(ErrorCode::NotFound));
    let tree = overview_page(screen);
    assert_one_heading_and_labelled_inputs(&tree);
    assert_eq!(control(&tree, "Try again").on_click, Some(Intent::Retry));
}

#[test]
fn the_overview_shows_the_name_role_last_save_and_members_with_a_you_tag() {
    let tree = overview_page(campaign_screen(overview_data()));
    assert_one_heading_and_labelled_inputs(&tree);
    let heading = all_elements(&tree)
        .into_iter()
        .find(|element| element.tag == "h1")
        .expect("h1");
    assert_eq!(element_text(heading), "Friday table");
    let text = tree.text_content();
    assert!(text.contains("Your role Host"));
    assert!(text.contains("Last saved"));
    assert!(text.contains("Ana You"));
    assert!(text.contains("Ben Player"));
}

#[test]
fn a_player_without_the_invite_right_sees_no_invitations_room_code_or_requests() {
    let mut overview = overview_data();
    overview.can_invite = false;
    overview.can_open_lobby = false;
    overview.my_role = CampaignRole::Player;
    let tree = overview_page(campaign_screen(overview));
    let text = tree.text_content();
    assert!(!text.contains("Create player invitation"));
    assert!(!text.contains("Room code"));
    assert!(!text.contains("People asking to join"));
    assert!(!text.contains("WXYZ-2345"));
    assert!(text.contains("Members"));
    assert!(!has_control(&tree, "Open the lobby"));
}

#[test]
fn the_host_can_create_player_and_contributor_invitations() {
    let tree = overview_page(campaign_screen(overview_data()));
    assert_eq!(
        control(&tree, "Create player invitation").on_click,
        Some(Intent::CreateInvitation(InvitationKind::Player))
    );
    assert_eq!(
        control(&tree, "Create contributor invitation").on_click,
        Some(Intent::CreateInvitation(InvitationKind::Contributor))
    );
}

#[test]
fn the_invitation_link_appears_only_once_one_was_created_and_says_it_is_shown_once() {
    let before = overview_page(campaign_screen(overview_data()));
    assert!(!has_control(&before, "Copy link"));
    assert_eq!(count_tag(&before, "code"), 0);

    let mut screen = campaign_screen(overview_data());
    screen.created_link = Some(InvitationCreated {
        invitation_id: "inv-2".to_owned(),
        link: "https://play.example.com/app/join#token=abc".to_owned(),
    });
    let after = overview_page(screen);
    assert!(
        after
            .text_content()
            .contains("This link is shown once; copy it now")
    );
    let copy = control(&after, "Copy link");
    assert_eq!(
        copy.on_click,
        Some(Intent::CopyText(
            "https://play.example.com/app/join#token=abc".to_owned()
        ))
    );
    let field = all_elements(&after)
        .into_iter()
        .find(|element| element.attribute("class") == Some("copy-field"))
        .expect("a copy field");
    assert!(element_text(field).contains("https://play.example.com/app/join#token=abc"));
    assert!(
        all_elements(&after).into_iter().all(|element| element
            .attributes
            .iter()
            .all(|(_, value)| !value.contains("#token="))),
        "the link is text only, never an attribute"
    );
}

#[test]
fn each_invitation_shows_its_use_count_and_can_be_revoked() {
    let tree = overview_page(campaign_screen(overview_data()));
    assert!(tree.text_content().contains("Used 1 of 4 times. Expires"));
    let revoke = control(&tree, "Revoke");
    assert_eq!(
        revoke.on_click,
        Some(Intent::RevokeInvitation {
            invitation_id: "inv-1".to_owned()
        })
    );
    assert!(
        revoke
            .attribute("aria-label")
            .is_some_and(|name| name.starts_with("Revoke"))
    );
}

#[test]
fn an_empty_invitation_list_says_so() {
    let mut overview = overview_data();
    overview.invitations.clear();
    assert!(
        overview_page(campaign_screen(overview))
            .text_content()
            .contains("No invitations yet.")
    );
}

#[test]
fn an_active_room_code_shows_in_a_code_box_with_rotate_and_the_approval_line() {
    let tree = overview_page(campaign_screen(overview_data()));
    let code = all_elements(&tree)
        .into_iter()
        .find(|element| element.attribute("class") == Some("code-box"))
        .expect("a code box");
    assert_eq!(element_text(code), "WXYZ-2345");
    assert_eq!(
        control(&tree, "Rotate code").on_click,
        Some(Intent::RotateCode)
    );
    assert!(
        tree.text_content()
            .contains("People who enter this code ask to join; you approve each one")
    );
}

#[test]
fn room_codes_that_are_off_are_explained_and_cannot_be_rotated() {
    let mut overview = overview_data();
    overview.room_code = RoomCodeState::Off;
    let tree = overview_page(campaign_screen(overview));
    assert!(
        tree.text_content()
            .contains("Room codes are off for this campaign")
    );
    assert!(!has_control(&tree, "Rotate code"));
}

#[test]
fn a_join_request_can_be_approved_as_player_or_contributor_or_denied() {
    let tree = overview_page(campaign_screen(overview_data()));
    assert!(tree.text_content().contains("Cleo"));
    let request_id = "req-1".to_owned();
    assert_eq!(
        control(&tree, "Approve as player").on_click,
        Some(Intent::DecideRequest {
            request_id: request_id.clone(),
            decision: RequestDecision::Approve(InvitationKind::Player),
        })
    );
    assert_eq!(
        control(&tree, "Approve as contributor").on_click,
        Some(Intent::DecideRequest {
            request_id: request_id.clone(),
            decision: RequestDecision::Approve(InvitationKind::Contributor),
        })
    );
    assert_eq!(
        control(&tree, "Deny").on_click,
        Some(Intent::DecideRequest {
            request_id,
            decision: RequestDecision::Deny
        })
    );
}

#[test]
fn no_waiting_requests_says_nobody_is_waiting() {
    let mut overview = overview_data();
    overview.join_requests.clear();
    assert!(
        overview_page(campaign_screen(overview))
            .text_content()
            .contains("Nobody is waiting to join.")
    );
}

#[test]
fn the_host_opens_the_lobby_when_none_is_live() {
    let tree = overview_page(campaign_screen(overview_data()));
    assert_eq!(
        control(&tree, "Open the lobby").on_click,
        Some(Intent::OpenLobby)
    );
    assert!(!has_control(&tree, "See who has joined"));
}

#[test]
fn a_live_lobby_is_linked_instead_of_opened_again() {
    let mut overview = overview_data();
    overview.live_session_id = Some("sess-1".to_owned());
    let tree = overview_page(campaign_screen(overview));
    let link = control(&tree, "See who has joined");
    assert_eq!(
        link.on_click,
        Some(Intent::Go(Route::Lobby {
            session_id: "sess-1".to_owned()
        }))
    );
    assert_eq!(link.attribute("href"), Some("/app/sessions/sess-1/lobby"));
    assert!(!has_control(&tree, "Open the lobby"));
}

#[test]
fn leaving_asks_for_confirmation_before_it_leaves() {
    let ask = overview_page(campaign_screen(overview_data()));
    assert_eq!(
        control(&ask, "Leave campaign").on_click,
        Some(Intent::AskLeave)
    );
    assert!(!has_control(&ask, "Yes, leave"));

    let mut screen = campaign_screen(overview_data());
    screen.is_confirming_leave = true;
    let confirm = overview_page(screen);
    assert!(
        confirm
            .text_content()
            .contains("Leave Friday table? You can be invited again.")
    );
    assert_eq!(
        control(&confirm, "Yes, leave").on_click,
        Some(Intent::LeaveCampaign)
    );
    assert_eq!(control(&confirm, "Stay").on_click, Some(Intent::CancelEdit));
    assert!(!has_control(&confirm, "Leave campaign"));
}

#[test]
fn a_refused_leave_is_shown_in_the_error_summary() {
    let mut screen = campaign_screen(overview_data());
    screen.action = Submit::Refused(ApiError::of(ErrorCode::Forbidden));
    let tree = overview_page(screen);
    assert!(tree.text_content().contains("There is a problem"));
    assert!(tree.text_content().contains("Your account cannot do this."));
}

#[test]
fn an_action_in_flight_disables_the_overview_actions() {
    let mut screen = campaign_screen(overview_data());
    screen.action = Submit::Sending;
    screen.operation = Remote::Loading;
    let tree = overview_page(screen);
    for label in [
        "Create player invitation",
        "Rotate code",
        "Approve as player",
        "Open the lobby",
        "Revoke",
    ] {
        let button = control(&tree, label);
        assert!(is_disabled(button), "{label}");
        assert_eq!(button.attribute("aria-busy"), Some("true"), "{label}");
    }
}

#[test]
fn the_overview_mentions_no_tab_that_does_not_exist_yet() {
    let text = overview_page(campaign_screen(overview_data())).text_content();
    // The account area gained Play sessions and Assets in round 2; the campaign page itself
    // still has no Characters tab or coming-soon copy.
    for word in ["Characters", "later releases"] {
        assert!(!text.contains(word), "{word}");
    }
}

#[test]
fn the_overview_shows_no_fee_multiplier_or_provider() {
    assert_no_pricing_internals(&overview_page(campaign_screen(overview_data())).text_content());
}

// ---------------------------------------------------------------- join fixtures

fn invite_preview() -> InvitePreview {
    InvitePreview {
        campaign_name: "Friday table".to_owned(),
        host_name: "Ana".to_owned(),
        scheduled_at_ms: Some(1_800_000_000_000),
        content_rating: ContentRating::Mature,
    }
}

fn join_screen(token: Option<&str>) -> JoinScreen {
    JoinScreen {
        preview: token.map(|_| Remote::Ready(invite_preview())),
        token: token.map(str::to_owned),
        code: String::new(),
        code_preview: None,
        operation: Remote::Ready("operation-1".to_owned()),
        submit: Submit::Idle,
        after_hold: None,
        requested: None,
    }
}

fn join_page(model: Model, screen: JoinScreen) -> Node {
    view(&with_screen(model, Screen::Join(screen)))
}

// ---------------------------------------------------------------- join

#[test]
fn an_invitation_preview_shows_name_host_and_time_without_a_rating() {
    let tree = join_page(signed_in(true), join_screen(Some(SECRET_TOKEN)));
    assert_one_heading_and_labelled_inputs(&tree);
    let text = tree.text_content();
    assert!(text.contains("You are invited to Friday table"));
    assert!(text.contains("Host Ana"));
    assert!(text.contains("Planned for"));
    assert!(!text.contains("Content rating"));
    assert!(!text.contains("Mature"));
}

#[test]
fn a_preview_without_a_scheduled_time_omits_that_row() {
    let mut screen = join_screen(Some(SECRET_TOKEN));
    screen.preview = Some(Remote::Ready(InvitePreview {
        scheduled_at_ms: None,
        ..invite_preview()
    }));
    assert!(
        !join_page(signed_in(true), screen)
            .text_content()
            .contains("Planned for")
    );
}

#[test]
fn a_signed_in_person_joins_with_one_button() {
    let tree = join_page(signed_in(true), join_screen(Some(SECRET_TOKEN)));
    assert_eq!(
        control(&tree, "Join this table").on_click,
        Some(Intent::AcceptInvitation)
    );
    assert!(!has_control(&tree, "Sign in to join"));
    assert!(tree.text_content().contains("You are signed in as Ana."));
}

#[test]
fn a_signed_out_visitor_can_hold_the_invitation_to_sign_in_or_to_sign_up() {
    let tree = join_page(signed_out(), join_screen(Some(SECRET_TOKEN)));
    assert_eq!(
        control(&tree, "Sign in to join").on_click,
        Some(Intent::HoldInvitation {
            then: Route::SignIn
        })
    );
    assert_eq!(
        control(&tree, "Create a free account to join").on_click,
        Some(Intent::HoldInvitation {
            then: Route::SignUp
        })
    );
    assert!(
        tree.text_content()
            .contains("We remember this invitation while you sign in")
    );
    assert!(!has_control(&tree, "Join this table"));
}

#[test]
fn the_invitation_token_is_never_printed_or_put_in_an_attribute() {
    for model in [signed_in(true), signed_out()] {
        assert_token_nowhere(&join_page(model, join_screen(Some(SECRET_TOKEN))));
    }
    let mut refused = join_screen(Some(SECRET_TOKEN));
    refused.preview = Some(Remote::Failed(ApiError::because(
        ErrorCode::NotFound,
        Reason::Revoked,
    )));
    assert_token_nowhere(&join_page(signed_in(true), refused));
    let mut sending = join_screen(Some(SECRET_TOKEN));
    sending.submit = Submit::Sending;
    assert_token_nowhere(&join_page(signed_out(), sending));
}

#[test]
fn a_refused_invitation_gives_the_reason_and_no_table_details() {
    for reason in [
        Reason::Expired,
        Reason::Revoked,
        Reason::Exhausted,
        Reason::Full,
        Reason::NoLongerValid,
    ] {
        let mut screen = join_screen(Some(SECRET_TOKEN));
        screen.preview = Some(Remote::Failed(ApiError::because(
            ErrorCode::NotFound,
            reason,
        )));
        let tree = join_page(signed_out(), screen);
        assert_one_heading_and_labelled_inputs(&tree);
        let text = tree.text_content();
        assert!(
            text.contains(crate::copy::reason_problem(reason).what),
            "{reason:?}"
        );
        assert!(!text.contains("Friday table"), "{reason:?}");
        assert!(!text.contains("Host"), "{reason:?}");
        assert!(!has_control(&tree, "Join this table"));
        assert!(!has_control(&tree, "Sign in to join"));
        assert!(!has_control(&tree, "Try again"));
    }
}

#[test]
fn an_invitation_that_could_not_be_checked_offers_try_again() {
    let mut screen = join_screen(Some(SECRET_TOKEN));
    screen.preview = Some(Remote::Failed(ApiError::of(ErrorCode::Unavailable)));
    let tree = join_page(signed_in(true), screen);
    assert_eq!(control(&tree, "Try again").on_click, Some(Intent::Retry));
}

#[test]
fn a_loading_invitation_says_so() {
    let mut screen = join_screen(Some(SECRET_TOKEN));
    screen.preview = Some(Remote::Loading);
    let tree = join_page(signed_in(true), screen);
    assert!(tree.text_content().contains("Checking your invitation..."));
    assert_token_nowhere(&tree);
}

#[test]
fn joining_in_flight_disables_the_join_buttons() {
    let mut screen = join_screen(Some(SECRET_TOKEN));
    screen.submit = Submit::Sending;
    let tree = join_page(signed_out(), screen.clone());
    assert!(is_disabled(control(&tree, "Sign in to join")));
    assert!(is_disabled(control(&tree, "Create a free account to join")));
    let signed = join_page(signed_in(true), screen);
    assert!(is_disabled(control(&signed, "Join this table")));
}

#[test]
fn a_refused_join_shows_the_reason_in_the_error_summary() {
    let mut screen = join_screen(Some(SECRET_TOKEN));
    screen.submit = Submit::Refused(ApiError::because(
        ErrorCode::Forbidden,
        Reason::AlreadyJoined,
    ));
    let tree = join_page(signed_in(true), screen);
    assert!(
        tree.text_content()
            .contains("You are already a member of this table.")
    );
}

#[test]
fn without_a_token_a_signed_out_visitor_is_asked_to_sign_in_to_use_a_room_code() {
    let tree = join_page(signed_out(), join_screen(None));
    assert_one_heading_and_labelled_inputs(&tree);
    assert!(tree.text_content().contains("Sign in to use a room code"));
    assert_eq!(
        control(&tree, "Sign in").attribute("href"),
        Some("/app/sign-in")
    );
    assert_eq!(
        control(&tree, "Create a free account").attribute("href"),
        Some("/app/sign-up")
    );
    assert!(bound(&tree, Field::RoomCode).is_empty());
}

#[test]
fn without_a_token_a_signed_in_person_gets_the_room_code_form() {
    let mut screen = join_screen(None);
    screen.code = "WXYZ-2345".to_owned();
    let tree = join_page(signed_in(true), screen);
    assert_one_heading_and_labelled_inputs(&tree);
    let form = all_elements(&tree)
        .into_iter()
        .find(|element| element.tag == "form")
        .expect("a form");
    assert_eq!(form.on_submit, Some(Intent::Submit(Form::CodeLookup)));
    let code = bound(&tree, Field::RoomCode);
    assert_eq!(code.len(), 1);
    assert_eq!(code[0].attribute("value"), Some("WXYZ-2345"));
    assert_eq!(
        control(&tree, "Look up code").attribute("type"),
        Some("submit")
    );
}

#[test]
fn a_refused_room_code_lookup_says_what_happened() {
    let mut screen = join_screen(None);
    screen.submit = Submit::Refused(ApiError::because(
        ErrorCode::NotFound,
        Reason::NoLongerValid,
    ));
    let tree = join_page(signed_in(true), screen);
    assert!(
        tree.text_content()
            .contains("This invitation or code is not valid.")
    );
}

#[test]
fn after_a_room_code_lookup_the_person_can_ask_to_join_or_use_another_code() {
    let mut screen = join_screen(None);
    screen.code = "WXYZ-2345".to_owned();
    screen.code_preview = Some(invite_preview());
    let tree = join_page(signed_in(true), screen);
    assert!(
        tree.text_content()
            .contains("You are invited to Friday table")
    );
    assert_eq!(
        control(&tree, "Ask to join").on_click,
        Some(Intent::AskToJoin)
    );
    assert_eq!(
        control(&tree, "Use another code").on_click,
        Some(Intent::ClearCodeLookup)
    );
    assert!(bound(&tree, Field::RoomCode).is_empty());
}

#[test]
fn after_asking_to_join_the_person_is_told_to_wait_for_the_host() {
    let mut screen = join_screen(None);
    screen.requested = Some(JoinRequested {
        campaign_name: "Friday table".to_owned(),
    });
    let tree = join_page(signed_in(true), screen);
    assert_one_heading_and_labelled_inputs(&tree);
    assert!(tree.text_content().contains(
        "Request sent to the host of Friday table. You will see the table on your dashboard \
         once they approve."
    ));
    assert!(!has_control(&tree, "Ask to join"));
}

// ---------------------------------------------------------------- lobby fixtures

fn lobby_member(name: &str, is_me: bool, _is_ready: bool) -> LobbyMember {
    LobbyMember {
        display_name: name.to_owned(),

        is_me,
    }
}

fn lobby_data() -> LobbyView {
    LobbyView {
        session_id: "sess-1".to_owned(),
        campaign_id: "camp-1".to_owned(),
        campaign_name: "Friday table".to_owned(),
        can_manage: true,
        start_blockers: vec![StartBlocker::FundingNotReady],
        members: vec![
            LobbyMember {
                ..lobby_member("Ana", true, true)
            },
            lobby_member("Ben", false, false),
        ],

        funding_note: FundingNote::NoChargeUntilHostStarts,
        has_started: false,
        poll_seconds: 3,
    }
}

fn lobby_screen(lobby: LobbyView) -> LobbyScreen {
    LobbyScreen {
        session_id: "sess-1".to_owned(),
        lobby: Remote::Ready(lobby),
        operation: Remote::Ready("operation-1".to_owned()),
        action: Submit::Idle,

        consent_checked: false,
    }
}

fn lobby_page(screen: LobbyScreen) -> Node {
    view(&with_screen(signed_in(true), Screen::Lobby(screen)))
}

// ---------------------------------------------------------------- lobby

#[test]
fn the_lobby_says_it_is_loading_and_has_one_heading() {
    let mut screen = lobby_screen(lobby_data());
    screen.lobby = Remote::Loading;
    let tree = lobby_page(screen);
    assert_one_heading_and_labelled_inputs(&tree);
    assert!(tree.text_content().contains("Opening the session..."));
}

#[test]
fn a_failed_lobby_offers_try_again() {
    let mut screen = lobby_screen(lobby_data());
    screen.lobby = Remote::Failed(ApiError::of(ErrorCode::Unavailable));
    assert_eq!(
        control(&lobby_page(screen), "Try again").on_click,
        Some(Intent::Retry)
    );
}

#[test]
fn the_lobby_says_once_that_it_updates_by_itself() {
    let text = lobby_page(lobby_screen(lobby_data())).text_content();
    assert_eq!(text.matches("This page updates by itself").count(), 1);
}

#[test]
fn the_lobby_funding_note_follows_the_server() {
    let text = lobby_page(lobby_screen(lobby_data())).text_content();
    assert!(text.contains("No charge until the host starts, except setup jobs you approved"));
    let mut waiting = lobby_data();
    waiting.funding_note = FundingNote::WaitingForSponsor;
    let text = lobby_page(lobby_screen(waiting)).text_content();
    assert!(text.contains("Waiting for the sponsor's funding approval"));
    assert!(!text.contains("No charge until"));
}

#[test]
fn start_blockers_are_listed_in_words_beside_a_disabled_start_button() {
    let tree = lobby_page(lobby_screen(lobby_data()));
    let text = tree.text_content();
    assert!(text.contains("funding"));
    let start = control(&tree, "Start the session");
    assert!(is_disabled(start));
    assert!(
        all_elements(&tree)
            .into_iter()
            .any(|element| element.attribute("id") == Some("start-blockers")),
        "the reason the button points at exists"
    );
}

#[test]
fn start_is_enabled_when_nothing_blocks_it() {
    let mut lobby = lobby_data();
    lobby.start_blockers.clear();
    let tree = lobby_page(lobby_screen(lobby));
    let start = control(&tree, "Start the session");
    assert!(!is_disabled(start));
    assert_eq!(start.on_click, Some(Intent::StartSession));
}

#[test]
fn a_member_who_cannot_manage_waits_for_the_host_when_nothing_blocks() {
    let mut lobby = lobby_data();
    lobby.can_manage = false;
    lobby.start_blockers.clear();
    assert!(
        lobby_page(lobby_screen(lobby))
            .text_content()
            .contains("Waiting for the host to start.")
    );
}

#[test]
fn a_started_session_links_to_the_pre_game_launcher() {
    let mut lobby = lobby_data();
    lobby.has_started = true;
    let tree = lobby_page(lobby_screen(lobby));
    assert!(tree.text_content().contains("The session has started"));
    let game = control(&tree, "Open the game");
    assert_eq!(game.tag, "a");
    assert_eq!(game.attribute("href"), Some("/app/sessions/sess-1/play"));
    assert!(!has_control(&tree, "Start the session"));
    // The launcher page opens in this tab; it opens the table and player screens in new ones.
    assert_eq!(game.attribute("target"), None);
}

#[test]
fn a_started_session_hides_the_launcher_link_from_a_member() {
    let mut lobby = lobby_data();
    lobby.has_started = true;
    lobby.can_manage = false;
    let tree = lobby_page(lobby_screen(lobby));
    assert!(tree.text_content().contains("The session has started"));
    assert!(!has_control(&tree, "Open the game"));
}

#[test]
fn a_member_is_told_to_join_from_the_table_screen() {
    let screen = PlayScreen {
        session_id: "sess-1".to_owned(),
        launch: Remote::Ready(GameLaunchView {
            session_id: "sess-1".to_owned(),
            campaign_name: "Friday table".to_owned(),
            has_started: true,
            can_manage: false,
            launcher_url: None,
            poll_seconds: 3,
        }),
        waiting_polls: 0,
        waiting_elapsed_ms: 0,
        next_poll_delay_ms: 0,
        polling_timed_out: false,
    };
    let tree = view(&with_screen(signed_in(true), Screen::Play(screen)));
    let text = tree.text_content();
    assert!(text.contains("The host runs the table"));
    assert!(
        text.contains("Join from the table screen with the room code the host shares with you.")
    );
}

#[test]
fn a_game_start_timeout_shows_a_retry_action() {
    let screen = PlayScreen {
        session_id: "sess-1".to_owned(),
        launch: Remote::Ready(GameLaunchView {
            session_id: "sess-1".to_owned(),
            campaign_name: "Friday table".to_owned(),
            has_started: true,
            can_manage: true,
            launcher_url: None,
            poll_seconds: 3,
        }),
        waiting_polls: 6,
        waiting_elapsed_ms: crate::state::MAXIMUM_GAME_START_WAIT_MS,
        next_poll_delay_ms: 30_000,
        polling_timed_out: true,
    };
    let tree = view(&with_screen(signed_in(true), Screen::Play(screen)));
    assert!(tree.text_content().contains("We couldn't start your game"));
    assert_eq!(control(&tree, "Try again").on_click, Some(Intent::Retry));
}

#[test]
fn an_unknown_or_ended_game_links_back_to_the_dashboard_without_retry() {
    let screen = PlayScreen {
        session_id: "sess-missing".to_owned(),
        launch: Remote::Failed(ApiError::of(ErrorCode::NotFound)),
        waiting_polls: 0,
        waiting_elapsed_ms: 0,
        next_poll_delay_ms: 0,
        polling_timed_out: false,
    };
    let tree = view(&with_screen(signed_in(true), Screen::Play(screen)));
    assert!(tree.text_content().contains("This game is no longer open"));
    assert!(tree.text_content().contains("This session may have ended"));
    assert_eq!(
        control(&tree, "Back to your games").on_click,
        Some(Intent::Go(Route::Dashboard))
    );
    assert!(!has_control(&tree, "Try again"));
}

#[test]
fn a_refused_lobby_action_shows_the_error_summary() {
    let mut screen = lobby_screen(lobby_data());
    screen.action = Submit::Refused(ApiError::because(
        ErrorCode::Forbidden,
        Reason::DisplayLimit,
    ));
    let tree = lobby_page(screen);
    assert!(tree.text_content().contains("There is a problem"));
    assert!(
        tree.text_content()
            .contains("This table already has as many screens paired")
    );
}

#[test]
fn the_lobby_shows_no_fee_multiplier_or_provider() {
    assert_no_pricing_internals(&lobby_page(lobby_screen(lobby_data())).text_content());
}

// ---------------------------------------------------------------- display

#[test]
fn every_campaign_screen_state_has_exactly_one_heading() {
    let pages = vec![
        overview_page(campaign_screen(overview_data())),
        lobby_page(lobby_screen(lobby_data())),
        join_page(signed_in(true), join_screen(None)),
        join_page(signed_out(), join_screen(Some(SECRET_TOKEN))),
    ];
    for tree in pages {
        assert_one_heading_and_labelled_inputs(&tree);
    }
}

#[test]
fn lobbyfix_website_membership_has_no_hero_ready_microphone_or_pairing_controls() {
    let tree = lobby_page(lobby_screen(lobby_data()));
    let text = tree.text_content();
    assert!(text.contains("Ana You") && text.contains("Ben"));
    for label in [
        "Hero:",
        "No hero yet",
        "Not ready",
        "Microphone",
        "Pair a display",
        "I am ready",
        "Show pairing code",
    ] {
        assert!(!text.contains(label), "{label}");
    }
    assert!(has_control(&tree, "Cancel"));
}

#[test]
fn campaign_settings_keep_funding_and_co_host_permissions_without_gameplay_controls() {
    let mut screen = campaign_screen(overview_data());
    screen.can_edit_settings = true;
    screen.settings = Some(Box::new(CampaignSettingsScreen {
        draft: CampaignDraft {
            name: "Friday table".to_owned(),
            adventure_id: None,
            content_rating: Some(ContentRating::Teen),
            seat_count: 6,
            hero_choice: HeroChoice::InLobby,
            has_voice: true,
            pictures: PictureDefault::Automatic,
            pace: PacePreset::Immersive,
            custom_ceiling_cents_per_hour: None,
            co_hosts_can_invite: true,
            co_hosts_can_start: false,
            co_hosts_can_export: false,
            join_mode: JoinMode::LinkAndRoomCode,
            funding: FundingModel::SharedPot,
        },
    }));
    let tree = overview_page(screen);
    for field in [
        Field::FundingChoice,
        Field::JoinModeChoice,
        Field::CoHostsCanInvite,
        Field::CoHostsCanStart,
        Field::CoHostsCanExport,
    ] {
        assert!(!bound(&tree, field).is_empty(), "{field:?}");
    }
    for label in [
        "Content rating",
        "Voice",
        "Pictures",
        "Table pace",
        "Hourly maximum",
    ] {
        assert!(!tree.text_content().contains(label), "{label}");
    }
    assert!(has_control(&tree, "Save settings"));
}

#[test]
fn an_action_in_flight_disables_the_lobby_controls() {
    let lobby = lobby_data();
    let mut screen = lobby_screen(lobby);
    screen.action = Submit::Sending;
    screen.operation = Remote::Loading;
    let tree = lobby_page(screen);
    for label in ["Start the session", "Cancel"] {
        let button = control(&tree, label);
        assert!(is_disabled(button), "{label}");
    }
    assert_eq!(
        control(&tree, "Start the session").attribute("aria-busy"),
        Some("true")
    );
}
