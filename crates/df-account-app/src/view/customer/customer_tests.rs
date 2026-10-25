use crate::{
    node::{Element, Node},
    route::Route,
    state::{
        AllocateEdit, AmountChoice, CHECKOUT_POLL_DELAYS_MS, CreditForm, CreditReturn,
        DashboardScreen, Field, Form, FundingScreen, Intent, Model, PaceEdit, Remote, Screen,
        SettingsScreen, StatementsScreen, Submit,
    },
    view::view,
    wire::{
        AccountView, ApiError, BalanceView, CampaignFunding, CheckoutStatus, CheckoutView,
        ClosureBlocker, CreditOffers, DeviceKind, DeviceSession, Direction, ErrorCode, FundingView,
        FundsState, LineStatus, OverviewView, PacePreset, PresetOffer, Quantity, Refusal,
        SessionFunding, SessionView, SettingsView, StatementCategory, StatementLine, TrialView,
    },
    wire_campaign::{
        CampaignCard, CampaignRole, ContentRating, DashboardView, InvitePreview, PendingInvitation,
    },
};

const FORBIDDEN_WORDS: [&str; 5] = ["table_fee", "multiplier", "markup", "provider", "margin"];
const WHEN_MS: u64 = 1_792_890_300_000;

// ------------------------------------------------------------------------------- sample builders

fn model_with(screen: Screen, is_verified: bool) -> Model {
    let mut model = Model::default();
    model.session = Remote::Ready(SessionView {
        csrf: "csrf".to_owned(),
        account: Some(AccountView {
            email: "ana@example.com".to_owned(),
            display_name: Some("Ana".to_owned()),
            is_verified,
            operator: None,
        }),
        dev_sign_in: None,
    });
    model.screen = screen;
    model
}

fn render(screen: Screen) -> Node {
    view(&model_with(screen, true))
}

fn text_of(node: &Node) -> String {
    node.text_content()
}

fn element_text(element: &Element) -> String {
    Node::Element(element.clone()).text_content()
}

fn failure() -> ApiError {
    ApiError::of(ErrorCode::Unavailable)
}

fn balance() -> BalanceView {
    BalanceView {
        purchased_available_micro_dollars: 12_349_999,
        promotional_available_micro_dollars: 7_500_000,
        allocated_micro_dollars: 3_000_000,
        reserved_micro_dollars: 450_000,
        consumed_this_month_micro_dollars: 12_340_001,
    }
}

fn overview(trial: TrialView) -> OverviewView {
    OverviewView {
        balance: balance(),
        trial,
    }
}

fn active_trial() -> TrialView {
    TrialView::Active {
        granted_micro_dollars: 10_000_000,
        remaining_micro_dollars: 7_500_000,
    }
}

fn card(name: &str, live_session_id: Option<&str>, is_archived: bool) -> CampaignCard {
    CampaignCard {
        campaign_id: format!("campaign-{name}"),
        name: name.to_owned(),
        role: CampaignRole::Host,
        host_name: "Ana".to_owned(),
        party_size: 4,
        last_played_at_ms: Some(1_792_890_300_000),
        last_saved_at_ms: None,
        live_session_id: live_session_id.map(str::to_owned),
        is_archived,
    }
}

fn invitation() -> PendingInvitation {
    PendingInvitation {
        preview: Some(InvitePreview {
            campaign_name: "The Sunken Keep".to_owned(),
            host_name: "Bram".to_owned(),
            scheduled_at_ms: None,
            content_rating: ContentRating::Teen,
        }),
    }
}

fn dashboard_screen() -> DashboardScreen {
    DashboardScreen {
        overview: Remote::Ready(overview(active_trial())),
        campaigns: Remote::Ready(DashboardView {
            campaigns: vec![
                card("Keep", Some("session-1"), false),
                card("Mines", None, true),
            ],
            can_create: true,
        }),
        pending: Remote::Ready(PendingInvitation { preview: None }),
        resend: Submit::Idle,
        is_resent: false,
        join_operation: Remote::Ready("op-join".to_owned()),
        join: Submit::Idle,
        stats: Remote::Ready(crate::wire::AccountStats::default()),
        latest_assets: Remote::Ready(crate::wire::AssetsView {
            assets: Vec::new(),
            next_cursor: None,
            total: 0,
        }),
        topups: Remote::Ready(Vec::new()),
    }
}

fn offers() -> CreditOffers {
    CreditOffers {
        is_test_mode: false,
        preset_amounts_cents: vec![1000, 2000, 5000],
        custom_minimum_cents: 500,
        custom_maximum_cents: 50_000,
    }
}

fn credit_screen() -> CreditForm {
    CreditForm {
        offers: Remote::Ready(offers()),
        choice: None,
        custom: String::new(),
        operation: Remote::Ready("op-credit".to_owned()),
        submit: Submit::Idle,
    }
}

fn checkout(status: CheckoutStatus) -> CreditReturn {
    CreditReturn {
        checkout_id: Some("checkout-1".to_owned()),
        status: Remote::Ready(CheckoutView {
            checkout_id: "checkout-1".to_owned(),
            amount_cents: 2000,
            status,
        }),
        polls: 0,
    }
}

fn preset_offers() -> Vec<PresetOffer> {
    vec![
        PresetOffer {
            preset: PacePreset::Story,
            ceiling_cents_per_hour: Some(300),
        },
        PresetOffer {
            preset: PacePreset::Immersive,
            ceiling_cents_per_hour: Some(750),
        },
        PresetOffer {
            preset: PacePreset::Cinematic,
            ceiling_cents_per_hour: Some(1500),
        },
        PresetOffer {
            preset: PacePreset::Custom,
            ceiling_cents_per_hour: None,
        },
    ]
}

fn session_funding() -> SessionFunding {
    SessionFunding {
        session_id: "session-1".to_owned(),
        pot_available_micro_dollars: 24_800_000,
        open_holds_micro_dollars: 700_000,
        spent_micro_dollars: 1_230_001,
        own_allocation_micro_dollars: 10_000_000,
        own_remaining_micro_dollars: 9_999_999,
        runway_at_ceiling_minutes: Some(195),
        runway_at_recent_rate_minutes: Some(45),
        funds: FundsState::Low,
        contributions_visible: true,
    }
}

fn campaign_funding(can_choose_pace: bool, session: Option<SessionFunding>) -> CampaignFunding {
    CampaignFunding {
        campaign_id: "campaign-keep".to_owned(),
        name: "The Keep".to_owned(),
        can_choose_pace,
        preset: PacePreset::Immersive,
        ceiling_cents_per_hour: 750,
        session,
    }
}

fn funding_view(campaigns: Vec<CampaignFunding>) -> FundingView {
    FundingView {
        presets: preset_offers(),
        is_proposed_pricing: true,
        custom_minimum_cents_per_hour: 100,
        custom_maximum_cents_per_hour: 5000,
        campaigns,
    }
}

fn funding_screen(campaigns: Vec<CampaignFunding>) -> FundingScreen {
    FundingScreen {
        funding: Remote::Ready(funding_view(campaigns)),
        edit: None,
        allocation: None,
        saved_campaign: None,
    }
}

fn pace_edit(preset: PacePreset) -> PaceEdit {
    PaceEdit {
        campaign_id: "campaign-keep".to_owned(),
        preset,
        custom: "9".to_owned(),
        operation: Remote::Ready("op-pace".to_owned()),
        submit: Submit::Idle,
    }
}

fn allocate_edit() -> AllocateEdit {
    AllocateEdit {
        session_id: "session-1".to_owned(),
        amount: String::new(),
        consents_to_host_spending: false,
        operation: Remote::Ready("op-allocate".to_owned()),
        submit: Submit::Idle,
    }
}

fn line(
    category: StatementCategory,
    direction: Direction,
    amount_micro_dollars: u64,
    status: LineStatus,
) -> StatementLine {
    StatementLine {
        line_id: format!("line-{amount_micro_dollars}"),
        at_ms: 1_792_890_300_000,
        category,
        campaign_name: Some("The Keep".to_owned()),
        quantity: Quantity::Seconds { seconds: 2538 },
        direction,
        amount_micro_dollars,
        status,
    }
}

fn statements_screen(lines: Vec<StatementLine>, next_cursor: Option<&str>) -> StatementsScreen {
    StatementsScreen {
        lines: Remote::Ready(lines),
        next_cursor: next_cursor.map(str::to_owned),
        more: Submit::Idle,
        filter: crate::wire::StatementFilter::All,
        from_text: String::new(),
        to_text: String::new(),
        totals: None,
        monthly: Vec::new(),
        per_campaign: Vec::new(),
    }
}

fn settings_view(blockers: Vec<ClosureBlocker>, devices: usize) -> SettingsView {
    let mut sessions = vec![DeviceSession {
        session_id: "aa".repeat(16),
        device: DeviceKind::Desktop,
        signed_in_at_ms: 1_792_890_300_000,
        last_seen_at_ms: 1_792_893_900_000,
        is_current: true,
    }];
    for _ in 1..devices {
        sessions.push(DeviceSession {
            session_id: "bb".repeat(16),
            device: DeviceKind::Phone,
            signed_in_at_ms: 1_792_800_000_000,
            last_seen_at_ms: 1_792_803_600_000,
            is_current: false,
        });
    }
    SettingsView {
        email: "ana@example.com".to_owned(),
        display_name: Some("Ana".to_owned()),
        sessions,
        closure_blockers: blockers,
        created_at_ms: 1_792_800_000_000,
        is_verified: true,
        terms_version: Some("terms-2026-10".to_owned()),
        terms_accepted_at_ms: Some(1_792_800_000_000),
        adult_attested_at_ms: Some(1_792_800_000_000),
        closure_state: crate::wire::ClosureState::Open,
        sign_in_methods: vec![crate::wire::SignInMethod::Password],
        email_preferences: crate::wire::EmailPreferences {
            product_news: false,
            session_reminders: true,
            transactional_always_on: true,
        },
        export_status: crate::wire::ExportStatus::NotRequested,
    }
}

fn settings_screen(settings: SettingsView) -> SettingsScreen {
    SettingsScreen {
        settings: Remote::Ready(settings),
        display_name: "Ana".to_owned(),
        profile_operation: Remote::Ready("op-profile".to_owned()),
        profile: Submit::Idle,
        is_profile_saved: false,
        revoke: Submit::Idle,
        revoked: None,
        close_password: String::new(),
        close_understood: false,
        close_operation: Remote::Ready("op-close".to_owned()),
        close: Submit::Idle,
        new_email: String::new(),
        email_password: String::new(),
        email_operation: Remote::Ready("op-email".to_owned()),
        email_change: Submit::Idle,
        is_email_change_sent: false,
        current_password: String::new(),
        new_password: String::new(),
        new_confirmation: String::new(),
        password_change: Submit::Idle,
        is_password_changed: false,
        product_news: false,
        session_reminders: true,
        preferences_operation: Remote::Ready("op-pref".to_owned()),
        preferences: Submit::Idle,
        is_preferences_saved: false,
        export_password: String::new(),
        export_operation: Remote::Ready("op-export".to_owned()),
        export: Submit::Idle,
        export_view: None,
    }
}

// ------------------------------------------------------------------------------- finding things

fn find_button(node: &Node, label: &str) -> Option<Element> {
    node.elements()
        .into_iter()
        .find(|element| element.tag == "button" && element_text(element) == label)
        .cloned()
}

fn find_link<'a>(node: &'a Node, label: &str) -> Option<&'a Element> {
    node.elements()
        .into_iter()
        .find(|element| element.tag == "a" && element_text(element) == label)
}

fn has_text(node: &Node, needle: &str) -> bool {
    text_of(node).contains(needle)
}

fn count_of(node: &Node, tag: &str) -> usize {
    node.elements()
        .into_iter()
        .filter(|element| element.tag == tag)
        .count()
}

fn input_for(node: &Node, field: Field) -> Option<&Element> {
    node.elements()
        .into_iter()
        .find(|element| element.tag == "input" && element.field == Some(field))
}

fn assert_labelled_inputs(node: &Node) {
    let labels: Vec<&str> = node
        .elements()
        .into_iter()
        .filter(|element| element.tag == "label")
        .filter_map(|element| element.attribute("for"))
        .collect();
    for input in node
        .elements()
        .into_iter()
        .filter(|element| element.tag == "input")
    {
        let id = input.attribute("id").unwrap_or("");
        assert!(!id.is_empty(), "an input has no id: {input:?}");
        assert!(
            labels.contains(&id),
            "input {id} has no label with a matching for"
        );
    }
}

fn assert_one_heading(node: &Node) {
    assert_eq!(count_of(node, "h1"), 1);
}

// -------------------------------------------------------------------------------- dashboard

#[test]
fn dashboard_while_loading_shows_a_loading_line_for_each_part() {
    let mut screen = dashboard_screen();
    screen.overview = Remote::Loading;
    screen.campaigns = Remote::Loading;
    screen.pending = Remote::Loading;
    let node = render(Screen::Dashboard(screen));
    let words = text_of(&node);
    assert!(words.contains("Loading your credit..."));
    assert!(words.contains("Loading your campaigns..."));
    assert!(words.contains("Checking for an invitation..."));
    assert_one_heading(&node);
}

#[test]
fn dashboard_parts_fail_and_load_independently() {
    let mut screen = dashboard_screen();
    screen.overview = Remote::Failed(failure());
    screen.pending = Remote::Loading;
    let node = render(Screen::Dashboard(screen));
    assert!(has_text(&node, "We could not reach DungeonFlux just now."));
    assert!(has_text(&node, "Checking for an invitation..."));
    assert!(
        find_link(&node, "Resume lobby").is_some(),
        "the campaign list still renders"
    );
}

#[test]
fn dashboard_shows_the_five_amounts_with_balances_rounded_down_and_use_rounded_up() {
    let node = render(Screen::Dashboard(dashboard_screen()));
    let ledger: Vec<String> = node
        .elements()
        .into_iter()
        .filter(|element| element.tag == "div" && element.attribute("class") == Some("ledger-item"))
        .map(element_text)
        .collect();
    let five = [
        "Available purchased credit $12.34",
        "Free play credit $7.50",
        "Set aside for tables $3.00",
        "Held for pending work $0.45",
        "Used this month $12.35",
    ];
    let positions: Vec<usize> = five
        .iter()
        .map(|wanted| {
            ledger
                .iter()
                .position(|item| item == wanted)
                .unwrap_or_else(|| panic!("{wanted} missing from {ledger:?}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "the five amounts keep their order: {ledger:?}"
    );
}

#[test]
fn dashboard_says_the_trial_state_in_plain_words() {
    let cases = [
        (
            TrialView::AwaitingVerification {
                amount_micro_dollars: 10_000_000,
            },
            "Your $10.00 of free play credit arrives when you verify your email.",
        ),
        (active_trial(), "Free play credit: $7.50 left of $10.00."),
        (
            TrialView::Spent {
                granted_micro_dollars: 10_000_000,
            },
            "You have used all $10.00 of your free play credit.",
        ),
        (
            TrialView::NotEligible,
            "This email address already received free play credit on another account",
        ),
    ];
    for (trial, expected) in cases {
        let mut screen = dashboard_screen();
        screen.overview = Remote::Ready(overview(trial));
        let node = render(Screen::Dashboard(screen));
        assert!(has_text(&node, expected), "missing: {expected}");
    }
}

#[test]
fn dashboard_links_to_add_credit_and_statements() {
    let node = render(Screen::Dashboard(dashboard_screen()));
    let credit = find_link(&node, "Add credit").map(|link| link.on_click.clone());
    assert!(credit.is_some());
    let statements = node.elements().into_iter().find(|element| {
        element.tag == "a"
            && element_text(element) == "Statements"
            && element.attribute("class") == Some("btn btn-outline")
    });
    assert_eq!(
        statements.and_then(|link| link.on_click.clone()),
        Some(Intent::Go(Route::Statements))
    );
}

#[test]
fn dashboard_asks_an_unverified_person_to_verify_with_a_resend_button() {
    let node = view(&model_with(Screen::Dashboard(dashboard_screen()), false));
    assert!(has_text(&node, "Verify your email to get $10 of free play"));
    let send = find_button(&node, "Send a new link");
    assert_eq!(
        send.and_then(|button| button.on_click),
        Some(Intent::ResendVerification)
    );
}

#[test]
fn dashboard_shows_no_verify_prompt_to_a_verified_person() {
    let node = render(Screen::Dashboard(dashboard_screen()));
    assert!(!has_text(&node, "Verify your email to get"));
    assert!(find_button(&node, "Send a new link").is_none());
}

#[test]
fn dashboard_resend_shows_sending_then_sent_then_refused() {
    let mut screen = dashboard_screen();
    screen.resend = Submit::Sending;
    let node = view(&model_with(Screen::Dashboard(screen.clone()), false));
    let sending = find_button(&node, "Sending a new link").expect("a sending button");
    assert!(sending.attribute("disabled").is_some());

    screen.resend = Submit::Idle;
    screen.is_resent = true;
    let node = view(&model_with(Screen::Dashboard(screen.clone()), false));
    assert!(has_text(&node, "A new link is on its way."));
    assert!(find_button(&node, "Send a new link").is_none());

    screen.is_resent = false;
    screen.resend = Submit::Refused(ApiError::of(ErrorCode::Throttled));
    let node = view(&model_with(Screen::Dashboard(screen), false));
    assert!(has_text(&node, "There is a problem"));
    assert!(has_text(&node, "Too many attempts in a short time."));
    assert!(
        find_button(&node, "Send a new link").is_some(),
        "the person can try again"
    );
}

#[test]
fn dashboard_offers_the_pending_invitation_with_a_join_button() {
    let mut screen = dashboard_screen();
    screen.pending = Remote::Ready(invitation());
    let node = render(Screen::Dashboard(screen));
    assert!(has_text(
        &node,
        "You were invited to The Sunken Keep by Bram. Join?"
    ));
    let join = find_button(&node, "Join this table").expect("a join button");
    assert_eq!(join.on_click, Some(Intent::AcceptPendingInvitation));
}

#[test]
fn dashboard_disables_joining_until_the_operation_is_ready_and_while_sending() {
    let mut screen = dashboard_screen();
    screen.pending = Remote::Ready(invitation());
    screen.join_operation = Remote::Loading;
    let node = render(Screen::Dashboard(screen.clone()));
    let join = find_button(&node, "Join this table").expect("a join button");
    assert!(join.attribute("disabled").is_some());

    screen.join_operation = Remote::Ready("op".to_owned());
    screen.join = Submit::Sending;
    let node = render(Screen::Dashboard(screen));
    let joining = find_button(&node, "Joining the table").expect("a joining button");
    assert!(joining.attribute("disabled").is_some());
}

#[test]
fn dashboard_shows_why_a_join_was_refused() {
    let mut screen = dashboard_screen();
    screen.pending = Remote::Ready(invitation());
    screen.join = Submit::Refused(ApiError::because(
        ErrorCode::Forbidden,
        crate::wire_campaign::Reason::Expired,
    ));
    let node = render(Screen::Dashboard(screen));
    assert!(has_text(&node, "There is a problem"));
    assert!(has_text(&node, "This invitation has expired."));
    assert!(find_button(&node, "Join this table").is_some());
}

#[test]
fn dashboard_shows_no_invitation_when_none_is_waiting() {
    let node = render(Screen::Dashboard(dashboard_screen()));
    assert!(!has_text(&node, "You have an invitation"));
}

#[test]
fn dashboard_lists_campaigns_with_resume_lobby_or_open_and_an_archived_tag() {
    let node = render(Screen::Dashboard(dashboard_screen()));
    let resume = find_link(&node, "Resume lobby").expect("a resume link");
    assert_eq!(
        resume.on_click,
        Some(Intent::Go(Route::Lobby {
            session_id: "session-1".to_owned()
        }))
    );
    let open = find_link(&node, "Open").expect("an open link");
    assert_eq!(
        open.on_click,
        Some(Intent::Go(Route::Campaign {
            campaign_id: "campaign-Mines".to_owned()
        }))
    );
    assert_eq!(open.attribute("aria-label"), Some("Open Mines"));
    let words = text_of(&node);
    assert!(words.contains("Mines Archived"));
    assert!(words.contains("Your role: Host."));
    assert!(words.contains("4 players."));
    assert!(words.contains("Last played 25 Oct 2026, 01:05."));
}

#[test]
fn dashboard_with_no_campaigns_shows_the_empty_state_and_a_room_code_link() {
    let mut screen = dashboard_screen();
    screen.campaigns = Remote::Ready(DashboardView {
        campaigns: Vec::new(),
        can_create: true,
    });
    let node = render(Screen::Dashboard(screen));
    assert!(has_text(&node, "Your first adventure starts with a table."));
    assert!(has_text(&node, "Invited already? Enter a room code."));
    let room = find_link(&node, "Enter a room code.").expect("a room code link");
    assert_eq!(room.on_click, Some(Intent::Go(Route::Join { token: None })));
}

#[test]
fn dashboard_offers_create_only_to_a_person_who_can_create() {
    let node = render(Screen::Dashboard(dashboard_screen()));
    let create = find_link(&node, "Create a campaign").expect("a create link");
    assert_eq!(create.on_click, Some(Intent::Go(Route::NewCampaign)));

    let mut screen = dashboard_screen();
    screen.campaigns = Remote::Ready(DashboardView {
        campaigns: Vec::new(),
        can_create: false,
    });
    let node = render(Screen::Dashboard(screen));
    assert!(find_link(&node, "Create a campaign").is_none());
    assert!(has_text(
        &node,
        "Creating a campaign needs a verified email address."
    ));
}

#[test]
fn dashboard_has_one_heading_named_your_tables() {
    let node = render(Screen::Dashboard(dashboard_screen()));
    assert_one_heading(&node);
    let heading = node
        .elements()
        .into_iter()
        .find(|element| element.tag == "h1");
    assert_eq!(heading.map(element_text).as_deref(), Some("Your tables"));
}

// --------------------------------------------------------------------------------- add credit

#[test]
fn add_credit_while_loading_and_failed_says_so_and_keeps_one_heading() {
    let mut screen = credit_screen();
    screen.offers = Remote::Loading;
    let node = render(Screen::AddCredit(screen.clone()));
    assert!(has_text(&node, "Loading amounts..."));
    assert_one_heading(&node);

    screen.offers = Remote::Failed(failure());
    let node = render(Screen::AddCredit(screen));
    assert!(has_text(&node, "We could not reach DungeonFlux just now."));
    assert!(find_button(&node, "Try again").is_some());
}

#[test]
fn add_credit_preset_buttons_choose_their_amount_and_show_the_selected_one() {
    let mut screen = credit_screen();
    screen.choice = Some(AmountChoice::Preset(2000));
    let node = render(Screen::AddCredit(screen));
    let buttons: Vec<&Element> = node
        .elements()
        .into_iter()
        .filter(|element| element.tag == "button" && element.attribute("aria-pressed").is_some())
        .collect();
    let intents: Vec<Option<Intent>> = buttons.iter().map(|b| b.on_click.clone()).collect();
    assert_eq!(
        intents,
        vec![
            Some(Intent::ChooseAmount(1000)),
            Some(Intent::ChooseAmount(2000)),
            Some(Intent::ChooseAmount(5000)),
        ]
    );
    let pressed: Vec<Option<&str>> = buttons
        .iter()
        .map(|b| b.attribute("aria-pressed"))
        .collect();
    assert_eq!(pressed, vec![Some("false"), Some("true"), Some("false")]);
    let words: Vec<String> = buttons.iter().map(|b| element_text(b)).collect();
    assert_eq!(words, vec!["$10", "$20 Selected", "$50"]);
}

#[test]
fn add_credit_custom_amount_is_a_labelled_dollar_field_with_the_allowed_range() {
    let node = render(Screen::AddCredit(credit_screen()));
    let input = input_for(&node, Field::CustomAmount).expect("a custom amount input");
    assert_eq!(input.attribute("inputmode"), Some("decimal"));
    assert!(has_text(&node, "Other amount in dollars"));
    assert!(has_text(&node, "Between $5 and $500."));
    assert_labelled_inputs(&node);
}

#[test]
fn add_credit_says_what_the_person_receives_for_each_kind_of_choice() {
    let mut screen = credit_screen();
    let node = render(Screen::AddCredit(screen.clone()));
    assert!(has_text(
        &node,
        "Choose an amount to see the credit you receive."
    ));

    screen.choice = Some(AmountChoice::Preset(2000));
    let node = render(Screen::AddCredit(screen.clone()));
    assert!(has_text(
        &node,
        "You receive $20.00 of DungeonFlux service credit"
    ));

    screen.choice = Some(AmountChoice::Custom);
    screen.custom = "12.5".to_owned();
    let node = render(Screen::AddCredit(screen.clone()));
    assert!(has_text(
        &node,
        "You receive $12.50 of DungeonFlux service credit"
    ));
    assert!(has_text(&node, "Selected"));

    screen.custom = "9999".to_owned();
    let node = render(Screen::AddCredit(screen));
    assert!(!has_text(&node, "You receive"));
    assert!(has_text(
        &node,
        "Enter an amount from $5 to $500 to see the credit you receive."
    ));
}

#[test]
fn add_credit_announces_the_amount_in_a_polite_live_region() {
    let node = render(Screen::AddCredit(credit_screen()));
    let live = node
        .elements()
        .into_iter()
        .find(|element| element.attribute("class") == Some("receive"));
    let live = live.expect("the receive line");
    assert_eq!(live.attribute("role"), Some("status"));
    assert_eq!(live.attribute("aria-live"), Some("polite"));
}

#[test]
fn add_credit_states_tax_and_the_transfer_and_refund_notes() {
    let node = render(Screen::AddCredit(credit_screen()));
    assert!(has_text(&node, "Tax, if any, is shown at checkout."));
    assert!(has_text(&node, "cannot be transferred to another person"));
    assert!(has_text(&node, "Eligible unused credit can be refunded"));
}

#[test]
fn add_credit_shows_the_test_mode_pill_only_in_test_mode() {
    let node = render(Screen::AddCredit(credit_screen()));
    assert!(!has_text(&node, "Test mode"));

    let mut screen = credit_screen();
    screen.offers = Remote::Ready(CreditOffers {
        is_test_mode: true,
        ..offers()
    });
    let node = render(Screen::AddCredit(screen));
    assert!(has_text(&node, "Test mode: no real card is charged"));
}

#[test]
fn add_credit_submits_the_credit_form_with_a_continue_to_payment_button() {
    let node = render(Screen::AddCredit(credit_screen()));
    let form = node
        .elements()
        .into_iter()
        .find(|element| element.tag == "form")
        .expect("a form");
    assert_eq!(form.on_submit, Some(Intent::Submit(Form::Credit)));
    let submit = find_button(&node, "Continue to payment").expect("a submit button");
    assert_eq!(submit.attribute("type"), Some("submit"));
    assert!(submit.attribute("disabled").is_none());
}

#[test]
fn add_credit_disables_the_submit_while_the_key_loads_and_while_sending() {
    let mut screen = credit_screen();
    screen.operation = Remote::Loading;
    let node = render(Screen::AddCredit(screen.clone()));
    let loading = find_button(&node, "Continue to payment").expect("a button");
    assert!(loading.attribute("disabled").is_some());

    screen.operation = Remote::Ready("op".to_owned());
    screen.submit = Submit::Sending;
    let node = render(Screen::AddCredit(screen));
    let sending = find_button(&node, "Opening the payment page").expect("a sending button");
    assert!(sending.attribute("disabled").is_some());
}

#[test]
fn add_credit_refusal_shows_the_summary_and_keeps_what_was_typed() {
    let mut screen = credit_screen();
    screen.choice = Some(AmountChoice::Custom);
    screen.custom = "2".to_owned();
    screen.submit = Submit::Refused(ApiError::refused(Refusal::AmountOutOfRange));
    let node = render(Screen::AddCredit(screen));
    assert!(has_text(&node, "There is a problem"));
    assert!(has_text(
        &node,
        "Choose an amount, or enter one within the range shown."
    ));
    let input = input_for(&node, Field::CustomAmount).expect("the input");
    assert_eq!(input.attribute("value"), Some("2"));
}

// ------------------------------------------------------------------------------ payment return

#[test]
fn payment_return_says_each_status_with_the_shared_sentences() {
    let cases = [
        (
            CheckoutStatus::Pending,
            "We are waiting for your card company to confirm the payment.",
        ),
        (CheckoutStatus::Paid, "Your credit has been added."),
        (
            CheckoutStatus::Failed,
            "The payment did not go through, and nothing was charged.",
        ),
        (
            CheckoutStatus::Cancelled,
            "You cancelled the payment. Nothing was charged.",
        ),
        (
            CheckoutStatus::Expired,
            "The payment page timed out. Nothing was charged.",
        ),
    ];
    for (status, sentence) in cases {
        let node = render(Screen::CreditReturn(checkout(status)));
        assert!(has_text(&node, sentence), "missing: {sentence}");
        assert_one_heading(&node);
    }
}

#[test]
fn payment_return_pending_says_it_takes_seconds_and_that_the_person_may_leave() {
    let node = render(Screen::CreditReturn(checkout(CheckoutStatus::Pending)));
    assert!(has_text(&node, "This usually takes a few seconds."));
    assert!(has_text(
        &node,
        "You can leave this page; the credit appears once."
    ));
    assert!(find_link(&node, "Go to your dashboard").is_some());
}

#[test]
fn payment_return_offers_check_again_only_after_polling_stopped() {
    let mut screen = checkout(CheckoutStatus::Pending);
    let node = render(Screen::CreditReturn(screen.clone()));
    assert!(find_button(&node, "Check again").is_none());

    screen.polls = CHECKOUT_POLL_DELAYS_MS.len() - 1;
    let node = render(Screen::CreditReturn(screen.clone()));
    assert!(find_button(&node, "Check again").is_none());

    screen.polls = CHECKOUT_POLL_DELAYS_MS.len();
    let node = render(Screen::CreditReturn(screen));
    let again = find_button(&node, "Check again").expect("a check again button");
    assert_eq!(again.on_click, Some(Intent::Retry));
    assert!(has_text(&node, "the credit appears once"));
}

#[test]
fn payment_return_paid_links_to_the_dashboard_and_more_credit_without_a_new_purchase() {
    let node = render(Screen::CreditReturn(checkout(CheckoutStatus::Paid)));
    let dashboard = find_link(&node, "Go to your dashboard").expect("dashboard link");
    assert_eq!(dashboard.on_click, Some(Intent::Go(Route::Dashboard)));
    let more = find_link(&node, "Add more credit").expect("add more credit link");
    assert_eq!(more.on_click, Some(Intent::Go(Route::AddCredit)));
    assert!(find_button(&node, "Check again").is_none());
    assert_eq!(
        count_of(&node, "form"),
        0,
        "the return page never starts a purchase"
    );
}

#[test]
fn payment_return_failed_cancelled_and_expired_offer_try_again_to_add_credit() {
    for status in [
        CheckoutStatus::Failed,
        CheckoutStatus::Cancelled,
        CheckoutStatus::Expired,
    ] {
        let node = render(Screen::CreditReturn(checkout(status)));
        let again = find_link(&node, "Try again").expect("a try again link");
        assert_eq!(again.on_click, Some(Intent::Go(Route::AddCredit)));
    }
}

#[test]
fn payment_return_announces_changes_in_a_polite_status_region_and_never_shows_the_id() {
    let node = render(Screen::CreditReturn(checkout(CheckoutStatus::Paid)));
    let live = node
        .elements()
        .into_iter()
        .find(|element| element.attribute("class") == Some("payment-status"))
        .expect("the live region");
    assert_eq!(live.attribute("role"), Some("status"));
    assert_eq!(live.attribute("aria-live"), Some("polite"));
    assert!(!has_text(&node, "checkout-1"));
    assert!(has_text(&node, "Credit amount: $20.00"));
}

#[test]
fn payment_return_loading_keeps_the_live_region_and_failure_offers_a_retry() {
    let mut screen = checkout(CheckoutStatus::Pending);
    screen.status = Remote::Loading;
    let node = render(Screen::CreditReturn(screen.clone()));
    assert!(has_text(&node, "Checking your payment..."));
    let live = node
        .elements()
        .into_iter()
        .find(|element| element.attribute("class") == Some("payment-status"));
    assert!(live.is_some());

    screen.status = Remote::Failed(failure());
    let node = render(Screen::CreditReturn(screen));
    assert!(find_button(&node, "Try again").is_some());
}

#[test]
fn payment_return_without_a_checkout_says_nothing_was_started_and_links_onward() {
    let screen = CreditReturn {
        checkout_id: None,
        status: Remote::Failed(ApiError::of(ErrorCode::NotFound)),
        polls: 0,
    };
    let node = render(Screen::CreditReturn(screen));
    assert!(has_text(&node, "We could not find a payment to show."));
    assert!(find_link(&node, "Add credit").is_some());
    assert_one_heading(&node);
}

// ---------------------------------------------------------------------------------- funding

#[test]
fn funding_while_loading_and_failed_says_so() {
    let mut screen = funding_screen(Vec::new());
    screen.funding = Remote::Loading;
    let node = render(Screen::Funding(screen.clone()));
    assert!(has_text(&node, "Loading table pace..."));
    assert_one_heading(&node);

    screen.funding = Remote::Failed(failure());
    let node = render(Screen::Funding(screen));
    assert!(find_button(&node, "Try again").is_some());
}

#[test]
fn funding_labels_proposed_pricing_only_while_it_is_proposed() {
    let node = render(Screen::Funding(funding_screen(Vec::new())));
    assert!(has_text(&node, "Proposed pilot pricing"));

    let mut view = funding_view(Vec::new());
    view.is_proposed_pricing = false;
    let mut screen = funding_screen(Vec::new());
    screen.funding = Remote::Ready(view);
    let node = render(Screen::Funding(screen));
    assert!(!has_text(&node, "Proposed pilot pricing"));
}

#[test]
fn funding_lists_the_presets_with_the_hourly_maximum_and_a_description() {
    let node = render(Screen::Funding(funding_screen(Vec::new())));
    let words = text_of(&node);
    assert!(words.contains("Story $3 an hour at most. Narration and text first"));
    assert!(words.contains("Immersive $7.50 an hour at most."));
    assert!(words.contains("Cinematic $15 an hour at most."));
    assert!(words.contains("Custom You choose, from $1 to $50 an hour."));
}

#[test]
fn funding_with_no_campaigns_points_to_the_dashboard() {
    let node = render(Screen::Funding(funding_screen(Vec::new())));
    assert!(has_text(&node, "You are not in a campaign yet."));
    assert!(find_link(&node, "Go to your dashboard").is_some());
}

#[test]
fn funding_lets_the_host_start_changing_the_pace() {
    let node = render(Screen::Funding(funding_screen(vec![campaign_funding(
        true, None,
    )])));
    let change = find_button(&node, "Change pace").expect("a change pace button");
    assert_eq!(
        change.on_click,
        Some(Intent::EditPace {
            campaign_id: "campaign-keep".to_owned()
        })
    );
    let words = text_of(&node);
    assert!(words.contains("Current pace Immersive"));
    assert!(words.contains("Hourly maximum $7.50 an hour at most"));
}

#[test]
fn funding_shows_a_non_host_the_pace_without_controls() {
    let node = render(Screen::Funding(funding_screen(vec![campaign_funding(
        false, None,
    )])));
    assert!(find_button(&node, "Change pace").is_none());
    assert!(has_text(&node, "Only the host chooses the pace."));
    assert!(has_text(&node, "Current pace Immersive"));
}

#[test]
fn funding_pace_form_has_a_radio_group_and_hides_the_custom_ceiling_until_custom() {
    let mut screen = funding_screen(vec![campaign_funding(true, None)]);
    screen.edit = Some(pace_edit(PacePreset::Immersive));
    let node = render(Screen::Funding(screen.clone()));
    let radios: Vec<&Element> = node
        .elements()
        .into_iter()
        .filter(|element| element.tag == "input" && element.field == Some(Field::PacePreset))
        .collect();
    assert_eq!(radios.len(), 4);
    let checked: Vec<Option<&str>> = radios
        .iter()
        .filter(|radio| radio.attribute("checked").is_some())
        .map(|radio| radio.attribute("value"))
        .collect();
    assert_eq!(checked, vec![Some("immersive")]);
    assert!(input_for(&node, Field::CustomCeiling).is_none());
    assert_labelled_inputs(&node);

    screen.edit = Some(pace_edit(PacePreset::Custom));
    let node = render(Screen::Funding(screen));
    let ceiling = input_for(&node, Field::CustomCeiling).expect("the custom ceiling input");
    assert_eq!(ceiling.attribute("value"), Some("9"));
    assert!(has_text(&node, "Between $1 and $50 an hour."));
    assert_labelled_inputs(&node);
}

#[test]
fn funding_pace_form_saves_or_cancels() {
    let mut screen = funding_screen(vec![campaign_funding(true, None)]);
    screen.edit = Some(pace_edit(PacePreset::Story));
    let node = render(Screen::Funding(screen));
    let form = node
        .elements()
        .into_iter()
        .find(|element| element.tag == "form")
        .expect("the pace form");
    assert_eq!(form.on_submit, Some(Intent::Submit(Form::Pace)));
    assert!(find_button(&node, "Save pace").is_some());
    let cancel = find_button(&node, "Cancel").expect("a cancel button");
    assert_eq!(cancel.on_click, Some(Intent::CancelEdit));
    assert!(
        find_button(&node, "Change pace").is_none(),
        "the form replaces the button"
    );
}

#[test]
fn funding_refused_ceiling_shows_the_summary_linking_to_the_ceiling_field() {
    let mut screen = funding_screen(vec![campaign_funding(true, None)]);
    let mut edit = pace_edit(PacePreset::Custom);
    edit.submit = Submit::Refused(ApiError::refused(Refusal::CeilingOutOfRange));
    screen.edit = Some(edit);
    let node = render(Screen::Funding(screen));
    let link = node
        .elements()
        .into_iter()
        .find(|element| element.tag == "a" && element.attribute("href") == Some("#custom-ceiling"));
    assert!(link.is_some(), "the summary links to the field");
    assert!(has_text(
        &node,
        "Enter an hourly maximum within the range shown."
    ));
}

#[test]
fn funding_confirms_a_saved_change_for_the_campaign() {
    let mut screen = funding_screen(vec![campaign_funding(true, None)]);
    screen.saved_campaign = Some("campaign-keep".to_owned());
    let node = render(Screen::Funding(screen));
    assert!(has_text(&node, "Your change to The Keep is saved."));
}

#[test]
fn funding_shows_the_session_funding_facts_in_words() {
    let node = render(Screen::Funding(funding_screen(vec![campaign_funding(
        true,
        Some(session_funding()),
    )])));
    let words = text_of(&node);
    assert!(words.contains("Funding Low"));
    assert!(words.contains("Available in the pot $24.80"));
    assert!(words.contains("Held for pending work $0.70"));
    assert!(words.contains("Spent so far $1.24"));
    assert!(words.contains("Your share of the pot $10.00"));
    assert!(words.contains("Left of your share $9.99"));
    assert!(words.contains("Lasts at the pace maximum About 3 hours 15 minutes"));
    assert!(words.contains("Lasts at the recent rate About 45 minutes"));
    assert!(words.contains("Other players can see what you add."));
}

#[test]
fn funding_names_each_funds_state_and_omits_unknown_runways() {
    for (funds, expected) in [
        (FundsState::Funded, "Funding Funded"),
        (FundsState::Low, "Funding Low"),
        (FundsState::FundingRequired, "Funding Funding required"),
    ] {
        let mut session = session_funding();
        session.funds = funds;
        session.runway_at_ceiling_minutes = None;
        session.runway_at_recent_rate_minutes = None;
        session.contributions_visible = false;
        let node = render(Screen::Funding(funding_screen(vec![campaign_funding(
            false,
            Some(session),
        )])));
        let words = text_of(&node);
        assert!(words.contains(expected), "missing: {expected}");
        assert!(!words.contains("Lasts at"));
        assert!(words.contains("Other players cannot see what you add."));
    }
}

#[test]
fn funding_offers_add_credit_to_this_table_for_a_campaign_with_a_session() {
    let node = render(Screen::Funding(funding_screen(vec![campaign_funding(
        false,
        Some(session_funding()),
    )])));
    let add = find_button(&node, "Add credit to this table").expect("an add credit button");
    assert_eq!(
        add.on_click,
        Some(Intent::EditAllocation {
            session_id: "session-1".to_owned()
        })
    );
    let without = render(Screen::Funding(funding_screen(vec![campaign_funding(
        false, None,
    )])));
    assert!(find_button(&without, "Add credit to this table").is_none());
}

#[test]
fn funding_allocate_form_asks_for_an_amount_and_the_host_spending_consent() {
    let mut screen = funding_screen(vec![campaign_funding(false, Some(session_funding()))]);
    screen.allocation = Some(allocate_edit());
    let node = render(Screen::Funding(screen));
    let form = node
        .elements()
        .into_iter()
        .find(|element| element.tag == "form")
        .expect("the allocate form");
    assert_eq!(form.on_submit, Some(Intent::Submit(Form::Allocate)));
    assert!(input_for(&node, Field::AllocateAmount).is_some());
    let consent = input_for(&node, Field::AllocateConsent).expect("the consent checkbox");
    assert_eq!(consent.attribute("type"), Some("checkbox"));
    assert!(has_text(
        &node,
        "I agree the host may spend up to this amount at this table's pace"
    ));
    assert!(find_button(&node, "Add to the table").is_some());
    assert_eq!(
        find_button(&node, "Cancel").and_then(|cancel| cancel.on_click),
        Some(Intent::CancelEdit)
    );
    assert_labelled_inputs(&node);
}

#[test]
fn funding_allocate_refusal_without_consent_links_to_the_checkbox() {
    let mut screen = funding_screen(vec![campaign_funding(false, Some(session_funding()))]);
    let mut edit = allocate_edit();
    edit.submit = Submit::Refused(ApiError::refused(Refusal::ConsentRequired));
    screen.allocation = Some(edit);
    let node = render(Screen::Funding(screen));
    let link = node.elements().into_iter().find(|element| {
        element.tag == "a" && element.attribute("href") == Some("#allocate-consent")
    });
    assert!(link.is_some());
    let checkbox = input_for(&node, Field::AllocateConsent).expect("the checkbox");
    assert_eq!(checkbox.attribute("aria-invalid"), Some("true"));
}

// -------------------------------------------------------------------------------- statements

#[test]
fn statements_while_loading_failed_or_empty_says_what_is_happening() {
    let mut screen = statements_screen(Vec::new(), None);
    screen.lines = Remote::Loading;
    let node = render(Screen::Statements(screen.clone()));
    assert!(has_text(&node, "Loading your statement..."));
    assert_one_heading(&node);

    screen.lines = Remote::Failed(failure());
    let node = render(Screen::Statements(screen.clone()));
    assert!(find_button(&node, "Try again").is_some());

    screen.lines = Remote::Ready(Vec::new());
    let node = render(Screen::Statements(screen));
    assert!(has_text(
        &node,
        "No charges yet. Your first session will appear here"
    ));
    assert_eq!(count_of(&node, "table"), 0);
}

#[test]
fn statements_table_has_a_caption_header_cells_and_labelled_stacking_cells() {
    let screen = statements_screen(
        vec![line(
            StatementCategory::TableTime,
            Direction::Charge,
            1_230_001,
            LineStatus::Settled,
        )],
        None,
    );
    let node = render(Screen::Statements(screen));
    let table = node
        .elements()
        .into_iter()
        .find(|element| element.tag == "table")
        .expect("a table");
    assert!(
        table
            .attribute("class")
            .is_some_and(|class| class.contains("data-stack"))
    );
    let caption = node
        .elements()
        .into_iter()
        .find(|element| element.tag == "caption");
    assert_eq!(
        caption.map(element_text).as_deref(),
        Some("Charges and credits")
    );
    let headers: Vec<String> = node
        .elements()
        .into_iter()
        .filter(|element| element.tag == "th")
        .map(element_text)
        .collect();
    assert_eq!(headers, vec!["When", "What", "Status", "Amount"]);
    let labels: Vec<&str> = node
        .elements()
        .into_iter()
        .filter(|element| element.tag == "td")
        .filter_map(|element| element.attribute("data-label"))
        .collect();
    assert_eq!(labels, vec!["When", "What", "Status", "Amount"]);
}

#[test]
fn statements_row_says_when_what_status_and_amount_in_words() {
    let screen = statements_screen(
        vec![line(
            StatementCategory::TableTime,
            Direction::Charge,
            1_230_001,
            LineStatus::Settled,
        )],
        None,
    );
    let node = render(Screen::Statements(screen));
    let row = node
        .elements()
        .into_iter()
        .find(|element| {
            element.tag == "tr"
                && element
                    .children
                    .iter()
                    .any(|c| matches!(c, Node::Element(e) if e.tag == "td"))
        })
        .map(element_text)
        .expect("a row");
    assert_eq!(
        row,
        "25 Oct 2026, 01:05 Active table time Campaign: The Keep 42m 18s Final Charge -$1.24"
    );
}

#[test]
fn statements_round_charges_up_and_credits_down_and_write_the_direction() {
    let screen = statements_screen(
        vec![
            line(
                StatementCategory::Narration,
                Direction::Charge,
                1,
                LineStatus::Settled,
            ),
            line(
                StatementCategory::Purchase,
                Direction::Credit,
                19_999_999,
                LineStatus::Settled,
            ),
        ],
        None,
    );
    let node = render(Screen::Statements(screen));
    let amounts: Vec<String> = node
        .elements()
        .into_iter()
        .filter(|element| element.tag == "td" && element.attribute("class") == Some("num"))
        .map(element_text)
        .collect();
    assert_eq!(amounts, vec!["Charge -$0.01", "Credit +$19.99"]);
}

#[test]
fn statements_status_is_a_pill_with_words_for_every_status() {
    let lines = vec![
        line(
            StatementCategory::Music,
            Direction::Charge,
            10_000,
            LineStatus::Settled,
        ),
        line(
            StatementCategory::Music,
            Direction::Charge,
            20_000,
            LineStatus::Provisional,
        ),
        line(
            StatementCategory::Music,
            Direction::Charge,
            30_000,
            LineStatus::Pending,
        ),
        line(
            StatementCategory::Refund,
            Direction::Credit,
            40_000,
            LineStatus::Refunded,
        ),
    ];
    let node = render(Screen::Statements(statements_screen(lines, None)));
    let pills: Vec<String> = node
        .elements()
        .into_iter()
        .filter(|element| {
            element.tag == "span"
                && element
                    .attribute("class")
                    .is_some_and(|class| class.starts_with("pill"))
        })
        .map(element_text)
        .collect();
    assert_eq!(
        pills,
        vec![
            "Final",
            "Provisional: can only go down",
            "Pending",
            "Refunded"
        ]
    );
}

#[test]
fn statements_explains_provisional_lines_only_when_there_are_some() {
    let provisional = statements_screen(
        vec![line(
            StatementCategory::TableTime,
            Direction::Charge,
            10_000,
            LineStatus::Provisional,
        )],
        None,
    );
    let node = render(Screen::Statements(provisional));
    assert!(has_text(
        &node,
        "can only go down: the final amount is never higher than shown"
    ));

    let settled = statements_screen(
        vec![line(
            StatementCategory::TableTime,
            Direction::Charge,
            10_000,
            LineStatus::Settled,
        )],
        None,
    );
    let node = render(Screen::Statements(settled));
    assert!(!has_text(&node, "never higher than shown"));
}

#[test]
fn statements_show_older_loads_the_next_page_and_shows_sending_and_failure() {
    let lines = vec![line(
        StatementCategory::TableTime,
        Direction::Charge,
        10_000,
        LineStatus::Settled,
    )];
    let mut screen = statements_screen(lines.clone(), Some("cursor"));
    let node = render(Screen::Statements(screen.clone()));
    let older = find_button(&node, "Show older").expect("a show older button");
    assert_eq!(older.on_click, Some(Intent::LoadOlderStatements));

    screen.more = Submit::Sending;
    let node = render(Screen::Statements(screen.clone()));
    let sending = find_button(&node, "Loading older lines").expect("a sending button");
    assert!(sending.attribute("disabled").is_some());

    screen.more = Submit::Refused(failure());
    let node = render(Screen::Statements(screen));
    assert!(has_text(&node, "We could not reach DungeonFlux just now."));
    assert!(
        find_button(&node, "Show older").is_some(),
        "the person can try again"
    );

    let last_page = statements_screen(lines, None);
    let node = render(Screen::Statements(last_page));
    assert!(find_button(&node, "Show older").is_none());
}

// ---------------------------------------------------------------------------------- settings

#[test]
fn settings_while_loading_or_failed_says_so_with_one_heading() {
    let mut screen = settings_screen(settings_view(Vec::new(), 1));
    screen.settings = Remote::Loading;
    let node = render(Screen::Settings(screen.clone()));
    assert!(has_text(&node, "Loading your settings..."));
    assert_one_heading(&node);

    screen.settings = Remote::Failed(failure());
    let node = render(Screen::Settings(screen));
    assert!(find_button(&node, "Try again").is_some());
}

#[test]
fn settings_profile_form_edits_the_display_name_and_shows_the_email_read_only() {
    let node = render(Screen::Settings(settings_screen(settings_view(
        Vec::new(),
        1,
    ))));
    assert!(has_text(&node, "Email address ana@example.com"));
    assert!(
        input_for(&node, Field::Email).is_none(),
        "the email is not editable"
    );
    let name = input_for(&node, Field::DisplayName).expect("a display name input");
    assert_eq!(name.attribute("value"), Some("Ana"));
    let form = node
        .elements()
        .into_iter()
        .find(|element| element.on_submit == Some(Intent::Submit(Form::Profile)));
    assert!(form.is_some());
    assert!(find_button(&node, "Save display name").is_some());
    assert_labelled_inputs(&node);
}

#[test]
fn settings_profile_shows_saved_and_refused_states() {
    let mut screen = settings_screen(settings_view(Vec::new(), 1));
    screen.is_profile_saved = true;
    let node = render(Screen::Settings(screen.clone()));
    assert!(has_text(&node, "Display name saved."));

    screen.is_profile_saved = false;
    screen.profile = Submit::Refused(ApiError::refused(Refusal::DisplayNameTooLong));
    let node = render(Screen::Settings(screen));
    let link = node
        .elements()
        .into_iter()
        .find(|element| element.tag == "a" && element.attribute("href") == Some("#display-name"));
    assert!(link.is_some());
    assert!(has_text(
        &node,
        "Use a display name of 40 characters or fewer."
    ));
}

#[test]
fn settings_lists_devices_with_this_device_tag_and_times() {
    let node = render(Screen::Settings(settings_screen(settings_view(
        Vec::new(),
        2,
    ))));
    let words = text_of(&node);
    assert!(words.contains("Computer or TV browser This device"));
    assert!(words.contains("Signed in 25 Oct 2026, 01:05. Last seen 25 Oct 2026, 02:05."));
    assert!(words.contains("Phone"));
    assert_eq!(words.matches("This device").count(), 1);
}

#[test]
fn settings_sign_out_other_devices_appears_only_when_there_are_other_devices() {
    let node = render(Screen::Settings(settings_screen(settings_view(
        Vec::new(),
        2,
    ))));
    let revoke = find_button(&node, "Sign out other devices").expect("a revoke button");
    assert_eq!(revoke.on_click, Some(Intent::RevokeOtherSessions));

    let node = render(Screen::Settings(settings_screen(settings_view(
        Vec::new(),
        1,
    ))));
    assert!(find_button(&node, "Sign out other devices").is_none());
}

#[test]
fn settings_revoke_shows_sending_the_result_and_a_refusal() {
    let mut screen = settings_screen(settings_view(Vec::new(), 2));
    screen.revoke = Submit::Sending;
    let node = render(Screen::Settings(screen.clone()));
    let sending = find_button(&node, "Signing out other devices").expect("a sending button");
    assert!(sending.attribute("disabled").is_some());

    screen.revoke = Submit::Idle;
    for (count, sentence) in [
        (0, "No other devices were signed in."),
        (1, "Signed out 1 other device."),
        (3, "Signed out 3 other devices."),
    ] {
        screen.revoked = Some(count);
        let node = render(Screen::Settings(screen.clone()));
        assert!(has_text(&node, sentence), "missing: {sentence}");
    }

    screen.revoked = None;
    screen.revoke = Submit::Refused(failure());
    let node = render(Screen::Settings(screen));
    assert!(has_text(&node, "There is a problem"));
}

#[test]
fn settings_explains_what_closing_the_account_does_in_a_separate_section() {
    let node = render(Screen::Settings(settings_screen(settings_view(
        Vec::new(),
        1,
    ))));
    let words = text_of(&node);
    assert!(words.contains("Close your account"));
    assert!(words.contains("It signs you out on every device."));
    assert!(words.contains("deleted after a grace period"));
    assert!(words.contains("If you sign in before the deletion date, your account reopens."));
    assert!(words.contains("Any free play credit is forfeited."));
    assert!(words.contains("Purchased credit must be refunded or used"));
    assert!(!words.contains("Before you can close your account:"));
    let close = node.elements().into_iter().find(|element| {
        element
            .attribute("class")
            .is_some_and(|c| c.contains("settings-close"))
    });
    assert!(close.is_some());
}

#[test]
fn settings_lists_each_closure_blocker_in_words() {
    let blockers = vec![
        ClosureBlocker::HostsActiveCampaign {
            campaign_name: "The Keep".to_owned(),
        },
        ClosureBlocker::PurchasedBalance {
            amount_micro_dollars: 12_349_999,
        },
        ClosureBlocker::PendingHolds {
            amount_micro_dollars: 450_000,
        },
        ClosureBlocker::OpenDispute,
    ];
    let node = render(Screen::Settings(settings_screen(settings_view(
        blockers, 1,
    ))));
    let words = text_of(&node);
    assert!(words.contains("Before you can close your account:"));
    assert!(words.contains("You host an active campaign: The Keep."));
    assert!(words.contains("You still have $12.34 of purchased credit."));
    assert!(words.contains("$0.45 is held for pending work."));
    assert!(words.contains("A payment dispute is open."));
}

#[test]
fn settings_close_form_has_the_checkbox_the_current_password_and_a_destructive_button() {
    let node = render(Screen::Settings(settings_screen(settings_view(
        Vec::new(),
        1,
    ))));
    let form = node
        .elements()
        .into_iter()
        .find(|element| element.on_submit == Some(Intent::Submit(Form::Close)))
        .expect("the close form");
    assert_eq!(form.attribute("aria-label"), Some("Close your account"));
    let checkbox = input_for(&node, Field::CloseUnderstood).expect("the understood checkbox");
    assert_eq!(checkbox.attribute("type"), Some("checkbox"));
    let password = input_for(&node, Field::ClosePassword).expect("the password input");
    assert_eq!(password.attribute("type"), Some("password"));
    assert_eq!(password.attribute("autocomplete"), Some("current-password"));
    let close = find_button(&node, "Close my account").expect("the close button");
    assert_eq!(close.attribute("type"), Some("submit"));
    assert!(
        close
            .attribute("class")
            .is_some_and(|class| class.contains("btn-danger"))
    );
    assert_labelled_inputs(&node);
}

#[test]
fn settings_close_refusals_link_to_the_checkbox_or_the_password_field() {
    let mut screen = settings_screen(settings_view(Vec::new(), 1));
    screen.close = Submit::Refused(ApiError::refused(Refusal::ConfirmationRequired));
    let node = render(Screen::Settings(screen.clone()));
    let link = node.elements().into_iter().find(|element| {
        element.tag == "a" && element.attribute("href") == Some("#close-understood")
    });
    assert!(link.is_some());

    screen.close = Submit::Refused(ApiError::refused(Refusal::WrongPassword));
    let node = render(Screen::Settings(screen));
    let link = node
        .elements()
        .into_iter()
        .find(|element| element.tag == "a" && element.attribute("href") == Some("#close-password"));
    assert!(
        link.is_some(),
        "the summary links to the close form's password field"
    );
    assert!(has_text(
        &node,
        "That email and password do not match an account."
    ));
}

#[test]
fn settings_close_button_says_closing_while_sending() {
    let mut screen = settings_screen(settings_view(Vec::new(), 1));
    screen.close = Submit::Sending;
    let node = render(Screen::Settings(screen));
    let button = find_button(&node, "Closing your account").expect("a sending button");
    assert!(button.attribute("disabled").is_some());
}

// ------------------------------------------------------------------------------------ all screens

fn every_screen() -> Vec<Screen> {
    let mut dashboard = dashboard_screen();
    dashboard.pending = Remote::Ready(invitation());
    let mut funding = funding_screen(vec![campaign_funding(true, Some(session_funding()))]);
    funding.edit = Some(pace_edit(PacePreset::Custom));
    funding.allocation = Some(allocate_edit());
    vec![
        Screen::Dashboard(dashboard),
        Screen::AddCredit(credit_screen()),
        Screen::CreditReturn(checkout(CheckoutStatus::Pending)),
        Screen::Funding(funding),
        Screen::Statements(statements_screen(
            vec![line(
                StatementCategory::TableTime,
                Direction::Charge,
                10_000,
                LineStatus::Provisional,
            )],
            Some("cursor"),
        )),
        Screen::Settings(settings_screen(settings_view(
            vec![ClosureBlocker::OpenDispute],
            2,
        ))),
        Screen::PlaySessions(crate::state::PlaySessionsScreen {
            sessions: Remote::Ready(vec![crate::wire::PlaySessionRow {
                session_id: "s1".to_owned(),
                campaign_id: "c1".to_owned(),
                campaign_name: "Keep".to_owned(),
                started_at_ms: WHEN_MS,
                ended_at_ms: Some(WHEN_MS + 7_200_000),
                duration_seconds: 7200,
                players: 4,
                spent_micro_dollars: 2_000_000,
            }]),
            next_cursor: None,
            more: Submit::Idle,
        }),
        Screen::Assets(crate::state::AssetsScreen {
            assets: Remote::Ready(vec![crate::wire::AssetRow {
                asset_id: "a1".to_owned(),
                kind: crate::wire::AssetKind::Portrait,
                campaign_id: Some("c1".to_owned()),
                campaign_name: Some("Keep".to_owned()),
                created_at_ms: WHEN_MS,
                size_bytes: 1234,
                thumbnail_url: Some("/app/media/thumb/a1".to_owned()),
                hold_state: crate::wire::AssetHoldState::Ready,
            }]),
            next_cursor: None,
            total: 1,
            kind: None,
            campaign: String::new(),
            more: Submit::Idle,
        }),
    ]
}

#[test]
fn the_new_screens_name_what_they_show_in_plain_words() {
    let sessions = render(Screen::PlaySessions(crate::state::PlaySessionsScreen {
        sessions: Remote::Ready(vec![crate::wire::PlaySessionRow {
            session_id: "s1".to_owned(),
            campaign_id: "c1".to_owned(),
            campaign_name: "Keep".to_owned(),
            started_at_ms: WHEN_MS,
            ended_at_ms: Some(WHEN_MS + 7_200_000),
            duration_seconds: 7200,
            players: 4,
            spent_micro_dollars: 2_000_000,
        }]),
        next_cursor: None,
        more: Submit::Idle,
    }));
    let words = text_of(&sessions);
    assert!(words.contains("Play sessions"), "{words}");
    assert!(words.contains("Keep"));
    let assets = render(Screen::Assets(crate::state::AssetsScreen {
        assets: Remote::Ready(vec![crate::wire::AssetRow {
            asset_id: "a1".to_owned(),
            kind: crate::wire::AssetKind::Music,
            campaign_id: None,
            campaign_name: None,
            created_at_ms: WHEN_MS,
            size_bytes: 99,
            thumbnail_url: None,
            hold_state: crate::wire::AssetHoldState::Held,
        }]),
        next_cursor: None,
        total: 1,
        kind: None,
        campaign: String::new(),
        more: Submit::Idle,
    }));
    let words = text_of(&assets);
    assert!(words.contains("Music"), "{words}");
    assert!(words.contains("On hold"), "{words}");
    // The settings page shows account details and every management form.
    let settings = render(Screen::Settings(settings_screen(settings_view(vec![], 1))));
    let words = text_of(&settings);
    for label in [
        "Your account",
        "Change email address",
        "Change password",
        "Email choices",
        "Download my data",
        "Connect Google",
        "Coming soon",
    ] {
        assert!(words.contains(label), "settings misses {label}: {words}");
    }
    // The dashboard shows stats labels and the funds section.
    let dashboard = render(Screen::Dashboard(dashboard_screen()));
    let words = text_of(&dashboard);
    for label in [
        "Hours played",
        "Spent in total",
        "Funds",
        "Top up",
        "Your account",
    ] {
        assert!(words.contains(label), "dashboard misses {label}: {words}");
    }
}

#[test]
fn every_customer_screen_has_exactly_one_heading_and_labelled_inputs() {
    for screen in every_screen() {
        let node = view(&model_with(screen, false));
        assert_one_heading(&node);
        assert_labelled_inputs(&node);
    }
}

#[test]
fn every_customer_screen_has_no_click_only_controls() {
    for screen in every_screen() {
        let node = view(&model_with(screen, false));
        for element in node.elements() {
            if element.on_click.is_some() {
                assert!(
                    element.tag == "button" || element.tag == "a",
                    "a click on a {} is not keyboard reachable",
                    element.tag
                );
            }
            if element.tag == "a" && element.on_click.is_some() {
                assert!(element.attribute("href").is_some());
            }
        }
    }
}

#[test]
fn customer_wire_samples_and_screens_never_carry_the_fee_multiplier_or_provider_price() {
    let samples = [
        serde_json::to_string(&overview(active_trial())),
        serde_json::to_string(&funding_view(vec![campaign_funding(
            true,
            Some(session_funding()),
        )])),
        serde_json::to_string(&offers()),
        serde_json::to_string(
            &statements_screen(
                vec![line(
                    StatementCategory::TableTime,
                    Direction::Charge,
                    10_000,
                    LineStatus::Settled,
                )],
                None,
            )
            .lines
            .ready()
            .cloned(),
        ),
        serde_json::to_string(&settings_view(vec![ClosureBlocker::OpenDispute], 2)),
        serde_json::to_string(&checkout(CheckoutStatus::Paid).status.ready().cloned()),
    ];
    for sample in samples {
        let json = sample.expect("a sample serialises").to_lowercase();
        for word in FORBIDDEN_WORDS {
            assert!(!json.contains(word), "wire sample mentions {word}: {json}");
        }
    }
    for screen in every_screen() {
        let words = text_of(&view(&model_with(screen, false))).to_lowercase();
        for word in FORBIDDEN_WORDS {
            assert!(!words.contains(word), "a screen mentions {word}");
        }
    }
}

#[test]
fn customer_screens_use_no_jargon_for_credit() {
    for screen in every_screen() {
        let words = text_of(&view(&model_with(screen, false))).to_lowercase();
        for word in ["ledger", " lot ", "reservation"] {
            assert!(!words.contains(word), "a screen says {word}");
        }
    }
}
