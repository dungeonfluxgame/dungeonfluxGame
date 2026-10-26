use super::{
    clock::{TimeLeft, time_left},
    parts::{exact_dollars, multiplier, rate, reason_hint},
};
use crate::{
    api::ProviderSwitch,
    money,
    node::{Element, Node},
    route::{AdminRoute, Route},
    state::{
        AccountScreen, ActionTarget, AdminScreen, CreditGrantForm, Field, Intent, Model,
        ReasonAction, RefundForm, Remote, RowsScreen, Screen, SearchScreen, Submit,
    },
    view::view,
    wire::{
        AccountView, ActionOutcome, AdminAccountDetail, AdminAccountRow, AdminActionView,
        AdminSummary, ApiError, AuditAction, AuditRow, DisputeRow, DisputeStatus, ErrorCode,
        LedgerEntry, LedgerKind, LineStatus, OperatorCapability, OperatorView, ProviderHealth,
        ProviderRow, ReconciliationDifference, ReconciliationKind, ReconciliationView,
        RefundableLot, Refusal, ReviewDecision, ReviewFlag, ReviewItem, ReviewKind, SessionView,
        Standing,
    },
};

const NOW_MS: u64 = 1_792_890_300_000;
const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
const DAY_MS: u64 = 24 * HOUR_MS;

const ALL_CAPABILITIES: [OperatorCapability; 6] = [
    OperatorCapability::SupportInspect,
    OperatorCapability::Refund,
    OperatorCapability::GoodwillCredit,
    OperatorCapability::DisputeResponse,
    OperatorCapability::ProviderControl,
    OperatorCapability::ContentReview,
];

fn operator_model(capabilities: &[OperatorCapability], screen: AdminScreen) -> Model {
    let mut model = Model::default();
    model.session = Remote::Ready(SessionView {
        google_enabled: false,
        csrf: "csrf".to_owned(),
        account: Some(AccountView {
            email: "operator@example.com".to_owned(),
            display_name: None,
            is_verified: true,
            operator: Some(OperatorView {
                capabilities: capabilities.to_vec(),
                has_second_factor: true,
            }),
        }),
        dev_sign_in: None,
    });
    model.screen = Screen::Admin(screen);
    model.now_ms = NOW_MS;
    model
}

fn full_model(screen: AdminScreen) -> Model {
    operator_model(&ALL_CAPABILITIES, screen)
}

fn read_only_model(screen: AdminScreen) -> Model {
    operator_model(&[OperatorCapability::SupportInspect], screen)
}

fn elements(node: &Node) -> Vec<&Element> {
    node.elements()
}

fn text_of(element: &Element) -> String {
    Node::Element(element.clone()).text_content()
}

fn button_labels(node: &Node) -> Vec<String> {
    elements(node)
        .into_iter()
        .filter(|element| element.tag == "button")
        .map(text_of)
        .collect()
}

fn has_button(node: &Node, label: &str) -> bool {
    button_labels(node).iter().any(|found| found == label)
}

fn click_intents(node: &Node) -> Vec<Intent> {
    elements(node)
        .into_iter()
        .filter_map(|element| element.on_click.clone())
        .collect()
}

fn bound_fields(node: &Node) -> Vec<Field> {
    elements(node)
        .into_iter()
        .filter_map(|element| element.field)
        .collect()
}

fn count_tag(node: &Node, tag: &str) -> usize {
    elements(node)
        .into_iter()
        .filter(|element| element.tag == tag)
        .count()
}

fn has_link_to(node: &Node, route: &Route) -> bool {
    elements(node).into_iter().any(|element| {
        element.tag == "a"
            && element.attribute("href") == Some(route.path().as_str())
            && element.on_click == Some(Intent::Go(route.clone()))
    })
}

fn account_route(account_id: &str) -> Route {
    Route::Admin(AdminRoute::Account {
        account_id: account_id.to_owned(),
    })
}

fn failure() -> ApiError {
    ApiError::of(ErrorCode::Unavailable)
}

fn account_row() -> AdminAccountRow {
    AdminAccountRow {
        account_id: "acc-1".to_owned(),
        email_masked: "a***@example.com".to_owned(),
        display_name: Some("Avery".to_owned()),
        is_verified: true,
        standing: Standing::Active,
        balance_micro_dollars: 12_349_999,
        last_activity_at_ms: Some(NOW_MS),
        lifetime_topups_micro_dollars: 20_000_000,
        lifetime_spend_micro_dollars: 7_650_001,
        campaign_names: vec!["The Sunken Keep".to_owned()],
    }
}

fn ledger_entry() -> LedgerEntry {
    LedgerEntry {
        entry_id: "led-1".to_owned(),
        at_ms: NOW_MS,
        kind: LedgerKind::Usage,
        campaign_name: Some("The Sunken Keep".to_owned()),
        retail_micro_dollars: 12_340_000,
        table_fee_micro_dollars: 1_234_000,
        multiplier_permille: 1500,
        provider: Some("Alpha Voice".to_owned()),
        provider_cost_micro_dollars: 4_200,
        price_version: "v3-2026-10".to_owned(),
        status: LineStatus::Settled,
    }
}

fn lot() -> RefundableLot {
    RefundableLot {
        lot_id: "lot-1".to_owned(),
        purchased_at_ms: NOW_MS - DAY_MS,
        refundable_cents: 2000,
        card_suffix: "4242".to_owned(),
    }
}

fn detail() -> AdminAccountDetail {
    AdminAccountDetail {
        account: account_row(),
        ledger: vec![ledger_entry()],
        refundable_lots: vec![lot()],
        email: Some("avery@example.com".to_owned()),
        created_at_ms: NOW_MS - DAY_MS,
        terms_version: Some("terms-2026-10".to_owned()),
        terms_accepted_at_ms: Some(NOW_MS - DAY_MS),
        adult_attested_at_ms: Some(NOW_MS - DAY_MS),
        closure_state: crate::wire::ClosureState::Open,
        sign_in_methods: vec![crate::wire::SignInMethod::Password],
        sessions: Vec::new(),
        play_sessions: Vec::new(),
        assets: Vec::new(),
        balance: None,
        campaign_funding: Vec::new(),
        disputes: Vec::new(),
        audit: Vec::new(),
        statement_totals: crate::wire::StatementTotals::default(),
        monthly: Vec::new(),
    }
}

fn account_screen(detail: Remote<AdminAccountDetail>) -> AccountScreen {
    AccountScreen {
        account_id: "acc-1".to_owned(),
        detail,
        refund: RefundForm {
            lot_id: None,
            amount: String::new(),
            reason: String::new(),
            operation: Remote::Ready("op-1".to_owned()),
            submit: Submit::Idle,
        },
        credit: CreditGrantForm {
            amount: String::new(),
            reason: String::new(),
            support_case: String::new(),
            operation: Remote::Ready("op-2".to_owned()),
            submit: Submit::Idle,
        },
        last: None,
        tab: crate::wire::AdminTab::Details,
    }
}

fn account_model(capabilities: &[OperatorCapability]) -> Model {
    operator_model(
        capabilities,
        AdminScreen::Account(account_screen(Remote::Ready(detail()))),
    )
}

fn rows_screen<T>(rows: Remote<Vec<T>>) -> RowsScreen<T> {
    RowsScreen {
        rows,
        action: None,
        last: None,
    }
}

fn reason_action(target: ActionTarget) -> ReasonAction {
    ReasonAction {
        target,
        reason: String::new(),
        operation: Remote::Ready("op-3".to_owned()),
        submit: Submit::Idle,
    }
}

fn dispute(status: DisputeStatus, due_in_ms: u64) -> DisputeRow {
    DisputeRow {
        dispute_id: "dp-1".to_owned(),
        account_id: "acc-1".to_owned(),
        amount_cents: 2500,
        status,
        opened_at_ms: NOW_MS - DAY_MS,
        evidence_due_at_ms: NOW_MS + due_in_ms,
    }
}

fn provider(health: ProviderHealth) -> ProviderRow {
    ProviderRow {
        provider_id: "prov-1".to_owned(),
        name: "Alpha Voice".to_owned(),
        health,
        error_rate_permille: 25,
        open_exposure_micro_dollars: 3_500_000,
    }
}

fn review_item(item_id: &str, campaign: &str, deadline_ms: u64) -> ReviewItem {
    ReviewItem {
        item_id: item_id.to_owned(),
        kind: ReviewKind::UploadedPortrait,
        campaign_name: campaign.to_owned(),
        submitted_at_ms: NOW_MS - HOUR_MS,
        review_deadline_ms: deadline_ms,
        flag: ReviewFlag::PossibleRealPerson,
    }
}

fn audit_row() -> AuditRow {
    AuditRow {
        audit_id: "aud-1".to_owned(),
        at_ms: NOW_MS,
        operator: "operator-7".to_owned(),
        action: AuditAction::Refund,
        target: "acc-1".to_owned(),
        reason: "Duplicate charge reported in case 88".to_owned(),
    }
}

fn summary() -> AdminSummary {
    AdminSummary {
        accounts: 120,
        verified_accounts: 96,
        customer_liability_micro_dollars: 4_567_899_999,
        open_disputes: 2,
        reconciliation_differences: 1,
        providers_degraded: 1,
        review_pending: 3,
    }
}

fn every_screen() -> Vec<AdminScreen> {
    vec![
        AdminScreen::Summary(Remote::Ready(summary())),
        AdminScreen::Accounts(SearchScreen {
            query: "avery".to_owned(),
            results: Some(Remote::Ready(vec![account_row()])),
            submit: Submit::Idle,
            standing: String::new(),
            verified: String::new(),
            has_balance: String::new(),
            sort: "last_activity".to_owned(),
        }),
        AdminScreen::Account(account_screen(Remote::Ready(detail()))),
        AdminScreen::Disputes(RowsScreen {
            rows: Remote::Ready(vec![dispute(DisputeStatus::NeedsResponse, DAY_MS)]),
            action: Some(reason_action(ActionTarget::DisputeEvidence {
                dispute_id: "dp-1".to_owned(),
            })),
            last: None,
        }),
        AdminScreen::Reconciliation(Remote::Ready(ReconciliationView {
            last_run_at_ms: Some(NOW_MS),
            differences: vec![ReconciliationDifference {
                kind: ReconciliationKind::Payout,
                reference: "po_1".to_owned(),
                difference_micro_dollars: 1_500_000,
                found_at_ms: NOW_MS,
            }],
        })),
        AdminScreen::Providers(RowsScreen {
            rows: Remote::Ready(vec![provider(ProviderHealth::Healthy)]),
            action: Some(reason_action(ActionTarget::Provider {
                provider_id: "prov-1".to_owned(),
                switch: ProviderSwitch::Pause,
            })),
            last: None,
        }),
        AdminScreen::Review(RowsScreen {
            rows: Remote::Ready(vec![review_item("rv-1", "Alpha", NOW_MS + HOUR_MS)]),
            action: Some(reason_action(ActionTarget::Review {
                item_id: "rv-1".to_owned(),
                decision: ReviewDecision::Approve,
            })),
            last: None,
        }),
        AdminScreen::Audit(Remote::Ready(vec![audit_row()])),
    ]
}

// ---------------------------------------------------------------- the pure helpers

#[test]
fn a_deadline_that_has_passed_or_is_now_is_overdue() {
    assert_eq!(time_left(NOW_MS, NOW_MS), TimeLeft::Overdue);
    assert_eq!(time_left(NOW_MS - 1, NOW_MS), TimeLeft::Overdue);
    assert_eq!(time_left(0, NOW_MS), TimeLeft::Overdue);
}

#[test]
fn time_left_rounds_up_so_a_future_deadline_never_reads_zero() {
    assert_eq!(
        time_left(NOW_MS + 1, NOW_MS),
        TimeLeft::Left {
            days: 0,
            hours: 0,
            minutes: 1
        }
    );
    assert_eq!(time_left(NOW_MS + MINUTE_MS, NOW_MS).span(), "1m");
    assert_eq!(time_left(NOW_MS + MINUTE_MS + 1, NOW_MS).span(), "2m");
}

#[test]
fn time_left_changes_unit_at_the_hour_and_the_day() {
    assert_eq!(time_left(NOW_MS + 59 * MINUTE_MS, NOW_MS).span(), "59m");
    assert_eq!(time_left(NOW_MS + HOUR_MS, NOW_MS).span(), "1h 00m");
    assert_eq!(
        time_left(NOW_MS + 3 * HOUR_MS + 12 * MINUTE_MS, NOW_MS).span(),
        "3h 12m"
    );
    assert_eq!(
        time_left(NOW_MS + 24 * HOUR_MS - MINUTE_MS, NOW_MS).span(),
        "23h 59m"
    );
    assert_eq!(time_left(NOW_MS + DAY_MS, NOW_MS).span(), "1d 00h");
    assert_eq!(
        time_left(NOW_MS + 2 * DAY_MS + 3 * HOUR_MS, NOW_MS).due(),
        "Due in 2d 03h"
    );
    assert_eq!(time_left(NOW_MS, NOW_MS).due(), "Overdue");
}

#[test]
fn a_deadline_inside_a_day_counts_as_within_a_day() {
    assert!(time_left(NOW_MS + DAY_MS - MINUTE_MS, NOW_MS).is_within_a_day());
    assert!(!time_left(NOW_MS + DAY_MS, NOW_MS).is_within_a_day());
    assert!(!time_left(NOW_MS, NOW_MS).is_within_a_day());
}

#[test]
fn permille_values_read_as_multipliers_and_rates_with_integer_math() {
    assert_eq!(multiplier(1500), "1.50x");
    assert_eq!(multiplier(1000), "1.00x");
    assert_eq!(multiplier(250), "0.25x");
    assert_eq!(multiplier(1505), "1.505x");
    assert_eq!(multiplier(12_000), "12.00x");
    assert_eq!(rate(25), "2.5%");
    assert_eq!(rate(0), "0.0%");
    assert_eq!(rate(7), "0.7%");
    assert_eq!(rate(1000), "100.0%");
}

#[test]
fn exact_dollars_keep_every_micro_dollar_that_matters() {
    assert_eq!(exact_dollars(12_340_000), "$12.34");
    assert_eq!(exact_dollars(4_200), "$0.0042");
    assert_eq!(exact_dollars(0), "$0.00");
    assert_eq!(exact_dollars(1), "$0.000001");
    assert_eq!(exact_dollars(500_000), "$0.50");
}

#[test]
fn the_reason_hint_states_the_rule_and_counts_what_is_left() {
    assert_eq!(
        reason_hint(""),
        "1 to 500 characters, kept in the audit log. 500 left."
    );
    assert!(reason_hint("  abc  ").ends_with("497 left."));
    assert!(reason_hint(&"x".repeat(500)).ends_with("0 left."));
    assert!(reason_hint(&"x".repeat(503)).ends_with("3 too many."));
}

#[test]
fn the_console_source_never_uses_a_float() {
    for source in [
        include_str!("../admin.rs"),
        include_str!("accounts.rs"),
        include_str!("clock.rs"),
        include_str!("parts.rs"),
        include_str!("queues.rs"),
    ] {
        assert!(!source.contains("f64") && !source.contains("f32"));
    }
}

// ---------------------------------------------------------------- shell, navigation, gating

#[test]
fn the_header_says_who_is_signed_in_and_names_every_capability() {
    let page = view(&full_model(AdminScreen::Summary(Remote::Ready(summary()))));
    let text = page.text_content();
    assert!(text.contains("Signed in as operator"));
    for capability in ALL_CAPABILITIES {
        assert!(text.contains(crate::copy::capability_name(capability)));
    }
}

#[test]
fn a_full_operator_sees_every_area_in_the_navigation_and_the_current_one_is_marked() {
    let page = view(&full_model(AdminScreen::Audit(Remote::Loading)));
    let nav = elements(&page)
        .into_iter()
        .find(|element| element.attribute("aria-label") == Some("Operator console"))
        .map(text_of)
        .unwrap_or_default();
    for label in [
        "Summary",
        "Accounts",
        "Disputes",
        "Reconciliation",
        "Providers",
        "Review queue",
        "Audit log",
    ] {
        assert!(nav.contains(label), "{label}");
    }
    let current: Vec<String> = elements(&page)
        .into_iter()
        .filter(|element| element.attribute("aria-current") == Some("page"))
        .map(text_of)
        .collect();
    assert!(current.contains(&"Audit log".to_owned()));
}

#[test]
fn the_account_page_keeps_the_accounts_area_marked_current() {
    let page = view(&account_model(&ALL_CAPABILITIES));
    let current: Vec<String> = elements(&page)
        .into_iter()
        .filter(|element| {
            element.tag == "a"
                && element.attribute("aria-current") == Some("page")
                && element
                    .attribute("class")
                    .is_some_and(|class| class.contains("admin-link"))
        })
        .map(text_of)
        .collect();
    assert_eq!(current, vec!["Accounts".to_owned()]);
}

#[test]
fn the_navigation_hides_areas_the_capabilities_do_not_allow() {
    let page = view(&read_only_model(AdminScreen::Summary(Remote::Loading)));
    let text = page.text_content();
    for shown in ["Accounts", "Reconciliation", "Providers", "Audit log"] {
        assert!(text.contains(shown), "{shown}");
    }
    assert!(!text.contains("Disputes"));
    assert!(!text.contains("Review queue"));
    assert!(!has_link_to(&page, &Route::Admin(AdminRoute::Disputes)));
    assert!(!has_link_to(&page, &Route::Admin(AdminRoute::Review)));
}

#[test]
fn an_area_outside_the_capabilities_says_so_instead_of_showing_data() {
    let review = AdminScreen::Review(rows_screen(Remote::Ready(vec![review_item(
        "rv-1",
        "Alpha",
        NOW_MS + HOUR_MS,
    )])));
    let page = view(&read_only_model(review));
    let text = page.text_content();
    assert!(text.contains("Your operator access does not include this area"));
    assert!(!text.contains("Alpha"));
    assert_eq!(count_tag(&page, "h1"), 1);
}

#[test]
fn a_signed_in_person_without_operator_access_gets_the_not_permitted_page() {
    let mut model = full_model(AdminScreen::Summary(Remote::Loading));
    model.session = Remote::Ready(SessionView {
        google_enabled: false,
        csrf: "csrf".to_owned(),
        account: Some(AccountView {
            email: "a@example.com".to_owned(),
            display_name: None,
            is_verified: true,
            operator: None,
        }),
        dev_sign_in: None,
    });
    let page = view(&model);
    assert!(
        page.text_content()
            .contains("This page is for DungeonFlux operators")
    );
    assert!(!page.text_content().contains("Signed in as operator"));
}

#[test]
fn every_console_screen_has_one_h1_and_labels_every_input() {
    for screen in every_screen() {
        let page = view(&full_model(screen.clone()));
        assert_eq!(count_tag(&page, "h1"), 1, "{screen:?}");
        let label_targets: Vec<&str> = elements(&page)
            .into_iter()
            .filter(|element| element.tag == "label")
            .filter_map(|element| element.attribute("for"))
            .collect();
        for input in elements(&page)
            .into_iter()
            .filter(|element| element.tag == "input")
        {
            let id = input.attribute("id").unwrap_or_default();
            assert!(label_targets.contains(&id), "input {id} has no label");
        }
    }
}

fn unready<T>(error: Option<ApiError>) -> Remote<T> {
    error.map_or(Remote::Loading, Remote::Failed)
}

fn screens_in(error: Option<ApiError>) -> Vec<AdminScreen> {
    vec![
        AdminScreen::Summary(unready(error)),
        AdminScreen::Disputes(rows_screen(unready(error))),
        AdminScreen::Reconciliation(unready(error)),
        AdminScreen::Providers(rows_screen(unready(error))),
        AdminScreen::Review(rows_screen(unready(error))),
        AdminScreen::Audit(unready(error)),
        AdminScreen::Account(account_screen(unready(error))),
    ]
}

#[test]
fn every_loading_list_state_keeps_one_h1_and_says_it_is_loading() {
    for screen in screens_in(None) {
        let page = view(&full_model(screen));
        assert_eq!(count_tag(&page, "h1"), 1);
        assert!(elements(&page).into_iter().any(|element| {
            element.attribute("role") == Some("status") && text_of(element).ends_with("...")
        }));
    }
}

#[test]
fn every_failed_list_state_keeps_one_h1_and_offers_a_retry() {
    for screen in screens_in(Some(failure())) {
        let page = view(&full_model(screen));
        assert_eq!(count_tag(&page, "h1"), 1);
        assert!(has_button(&page, "Try again"));
        assert!(click_intents(&page).contains(&Intent::Retry));
    }
}

// ---------------------------------------------------------------- summary

#[test]
fn the_summary_shows_totals_liability_in_dollars_and_links_with_counts() {
    let page = view(&full_model(AdminScreen::Summary(Remote::Ready(summary()))));
    let text = page.text_content();
    assert!(text.contains("120"));
    assert!(text.contains("96"));
    assert!(text.contains(&money::balance(4_567_899_999)));
    assert!(text.contains("$4,567.89"));
    assert!(text.contains("2 open disputes"));
    assert!(text.contains("1 difference at the last run"));
    assert!(text.contains("1 degraded provider"));
    assert!(text.contains("3 uploads waiting"));
    for area in [
        AdminRoute::Disputes,
        AdminRoute::Reconciliation,
        AdminRoute::Providers,
        AdminRoute::Review,
        AdminRoute::Audit,
    ] {
        assert!(has_link_to(&page, &Route::Admin(area.clone())), "{area:?}");
    }
}

#[test]
fn the_summary_lists_only_the_areas_the_operator_may_open() {
    let page = view(&read_only_model(AdminScreen::Summary(Remote::Ready(
        summary(),
    ))));
    let text = page.text_content();
    assert!(!text.contains("open disputes"));
    assert!(!text.contains("uploads waiting"));
    assert!(text.contains("degraded provider"));
}

// ---------------------------------------------------------------- accounts

fn search_model(results: Option<Remote<Vec<AdminAccountRow>>>, submit: Submit) -> Model {
    full_model(AdminScreen::Accounts(SearchScreen {
        query: String::new(),
        results,
        submit,
        standing: String::new(),
        verified: String::new(),
        has_balance: String::new(),
        sort: "last_activity".to_owned(),
    }))
}

#[test]
fn the_search_form_has_a_labelled_query_a_hint_and_the_search_button() {
    let page = view(&search_model(None, Submit::Idle));
    let text = page.text_content();
    assert!(text.contains("Search by email, account id or campaign name"));
    assert!(text.contains("Email, account id or campaign"));
    assert!(has_button(&page, "Search accounts"));
    assert!(bound_fields(&page).contains(&Field::SearchQuery));
    assert!(text.contains("Results appear here after you search"));
}

#[test]
fn a_search_in_flight_shows_busy_and_a_loading_line() {
    let page = view(&search_model(Some(Remote::Loading), Submit::Sending));
    assert!(has_button(&page, "Searching"));
    let busy = elements(&page)
        .into_iter()
        .any(|element| element.tag == "button" && element.attribute("aria-busy") == Some("true"));
    assert!(busy);
    assert!(page.text_content().contains("Searching..."));
}

#[test]
fn a_failed_search_offers_a_retry_and_an_empty_one_says_what_to_try() {
    let failed = view(&search_model(Some(Remote::Failed(failure())), Submit::Idle));
    assert!(has_button(&failed, "Try again"));
    let empty = view(&search_model(Some(Remote::Ready(Vec::new())), Submit::Idle));
    assert!(
        empty
            .text_content()
            .contains("No accounts match that search")
    );
}

#[test]
fn search_results_show_the_masked_email_and_link_to_the_account() {
    let mut other = account_row();
    other.account_id = "acc-2".to_owned();
    other.email_masked = "b***@example.com".to_owned();
    other.is_verified = false;
    other.standing = Standing::Restricted;
    let page = view(&search_model(
        Some(Remote::Ready(vec![account_row(), other])),
        Submit::Idle,
    ));
    let text = page.text_content();
    assert!(text.contains("a***@example.com"));
    assert!(text.contains("Avery"));
    assert!(text.contains("Verified"));
    assert!(text.contains("Not verified"));
    assert!(text.contains("Active"));
    assert!(text.contains("Restricted"));
    assert!(text.contains("$12.34"));
    assert!(has_link_to(&page, &account_route("acc-1")));
    assert!(has_link_to(&page, &account_route("acc-2")));
}

#[test]
fn the_account_page_shows_facts_and_the_balance() {
    let page = view(&account_model(&ALL_CAPABILITIES));
    let text = page.text_content();
    assert!(text.contains("acc-1"));
    assert!(text.contains("a***@example.com"));
    assert!(text.contains("Avery"));
    assert!(text.contains("$12.34"));
    assert!(has_link_to(&page, &Route::Admin(AdminRoute::Accounts)));
}

#[test]
fn the_account_ledger_shows_the_full_price_breakdown() {
    let page = view(&account_model(&ALL_CAPABILITIES));
    let text = page.text_content();
    for expected in [
        "Usage",
        "The Sunken Keep",
        "$12.34",
        "$1.234",
        "1.50x",
        "Alpha Voice",
        "$0.0042",
        "v3-2026-10",
        "Final",
    ] {
        assert!(text.contains(expected), "{expected}");
    }
    for header in [
        "When",
        "Kind",
        "Campaign",
        "Retail",
        "Table fee",
        "Multiplier",
        "Provider",
        "Provider cost",
        "Price version",
        "Status",
    ] {
        assert!(
            elements(&page)
                .into_iter()
                .any(|element| element.tag == "th" && text_of(element) == header),
            "{header}"
        );
    }
}

#[test]
fn an_account_with_no_ledger_lines_says_so() {
    let mut empty = detail();
    empty.ledger.clear();
    let model = operator_model(
        &ALL_CAPABILITIES,
        AdminScreen::Account(account_screen(Remote::Ready(empty))),
    );
    assert!(
        view(&model)
            .text_content()
            .contains("There are no ledger lines yet")
    );
}

#[test]
fn a_refund_operator_sees_the_lot_list_and_the_refund_form() {
    let page = view(&account_model(&ALL_CAPABILITIES));
    let text = page.text_content();
    assert!(text.contains("Card ending 4242"));
    assert!(text.contains("Up to $20.00 can be refunded"));
    assert!(has_button(&page, "Choose this purchase"));
    assert!(has_button(&page, "Refund to the card"));
    assert!(click_intents(&page).contains(&Intent::ChooseLot {
        lot_id: "lot-1".to_owned()
    }));
    let fields = bound_fields(&page);
    assert!(fields.contains(&Field::RefundAmount));
    assert!(fields.contains(&Field::RefundReason));
}

#[test]
fn the_chosen_lot_is_marked_selected_and_sets_the_amount_limit() {
    let mut screen = account_screen(Remote::Ready(detail()));
    screen.refund.lot_id = Some("lot-1".to_owned());
    let model = operator_model(&ALL_CAPABILITIES, AdminScreen::Account(screen));
    let page = view(&model);
    let text = page.text_content();
    assert!(text.contains("Selected"));
    assert!(!has_button(&page, "Choose this purchase"));
    assert!(text.contains("At most $20.00 for the chosen purchase"));
}

#[test]
fn an_account_with_nothing_refundable_says_so_and_has_no_refund_form() {
    let mut none = detail();
    none.refundable_lots.clear();
    let model = operator_model(
        &ALL_CAPABILITIES,
        AdminScreen::Account(account_screen(Remote::Ready(none))),
    );
    let page = view(&model);
    assert!(
        page.text_content()
            .contains("Nothing on this account can be refunded")
    );
    assert!(!has_button(&page, "Refund to the card"));
}

#[test]
fn a_goodwill_operator_sees_the_no_cash_value_wording_and_the_grant_form() {
    let page = view(&account_model(&ALL_CAPABILITIES));
    let text = page.text_content();
    assert!(text.contains("Promotional credit, no cash value. There is no way to set a balance."));
    assert!(has_button(&page, "Grant credit"));
    let fields = bound_fields(&page);
    for field in [Field::CreditAmount, Field::CreditReason, Field::SupportCase] {
        assert!(fields.contains(&field));
    }
}

#[test]
fn a_read_only_operator_sees_the_ledger_but_no_refund_or_goodwill_controls() {
    let page = view(&account_model(&[OperatorCapability::SupportInspect]));
    let text = page.text_content();
    assert!(text.contains("1.50x"));
    assert!(!has_button(&page, "Refund to the card"));
    assert!(!has_button(&page, "Grant credit"));
    assert!(!has_button(&page, "Choose this purchase"));
    assert!(!text.contains("Goodwill credit"));
    let fields = bound_fields(&page);
    assert!(fields.is_empty(), "{fields:?}");
}

#[test]
fn a_refund_only_operator_sees_the_refund_form_but_not_the_goodwill_form() {
    let page = view(&account_model(&[
        OperatorCapability::SupportInspect,
        OperatorCapability::Refund,
    ]));
    assert!(has_button(&page, "Refund to the card"));
    assert!(!has_button(&page, "Grant credit"));
}

#[test]
fn a_goodwill_only_operator_sees_the_grant_form_but_not_the_refund_form() {
    let page = view(&account_model(&[
        OperatorCapability::SupportInspect,
        OperatorCapability::GoodwillCredit,
    ]));
    assert!(has_button(&page, "Grant credit"));
    assert!(!has_button(&page, "Refund to the card"));
}

#[test]
fn the_refund_and_goodwill_reason_fields_state_the_500_character_rule() {
    let page = view(&account_model(&ALL_CAPABILITIES));
    let hints = elements(&page)
        .into_iter()
        .filter(|element| {
            element
                .attribute("id")
                .is_some_and(|id| id.ends_with("-reason-hint"))
        })
        .map(text_of)
        .collect::<Vec<_>>();
    assert_eq!(hints.len(), 2);
    for hint in hints {
        assert!(hint.contains("1 to 500 characters"));
        assert!(hint.contains("500 left"));
    }
}

#[test]
fn the_reason_counter_follows_what_was_typed() {
    let mut screen = account_screen(Remote::Ready(detail()));
    screen.refund.reason = "Duplicate charge".to_owned();
    let model = operator_model(&ALL_CAPABILITIES, AdminScreen::Account(screen));
    assert!(view(&model).text_content().contains("484 left"));
}

fn refused_account(refusal: Refusal, refund_form: bool) -> Model {
    let mut screen = account_screen(Remote::Ready(detail()));
    let refused = Submit::Refused(ApiError::refused(refusal));
    if refund_form {
        screen.refund.submit = refused;
    } else {
        screen.credit.submit = refused;
    }
    operator_model(&ALL_CAPABILITIES, AdminScreen::Account(screen))
}

fn summary_links(page: &Node) -> Vec<String> {
    elements(page)
        .into_iter()
        .filter(|element| element.tag == "a")
        .filter_map(|element| element.attribute("href"))
        .filter(|href| href.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_refused_refund_reason_links_the_summary_to_the_reason_field() {
    let page = view(&refused_account(Refusal::ReasonRequired, true));
    let text = page.text_content();
    assert!(text.contains("There is a problem"));
    assert!(text.contains("Give a reason"));
    assert!(summary_links(&page).contains(&"#refund-reason".to_owned()));
    let reason = elements(&page)
        .into_iter()
        .find(|element| element.attribute("id") == Some("refund-reason"));
    assert_eq!(
        reason.and_then(|element| element.attribute("aria-invalid")),
        Some("true")
    );
}

#[test]
fn a_refused_refund_amount_links_to_the_amount_field() {
    let page = view(&refused_account(Refusal::AmountOutOfRange, true));
    assert!(summary_links(&page).contains(&"#refund-amount".to_owned()));
    assert!(page.text_content().contains("Choose an amount"));
}

#[test]
fn a_refused_goodwill_form_links_each_problem_to_its_own_field() {
    for (refusal, target) in [
        (Refusal::AmountOutOfRange, "#credit-amount"),
        (Refusal::SupportCaseRequired, "#support-case"),
        (Refusal::ReasonRequired, "#credit-reason"),
        (Refusal::ReasonTooLong, "#credit-reason"),
    ] {
        let page = view(&refused_account(refusal, false));
        assert!(
            summary_links(&page).contains(&target.to_owned()),
            "{refusal:?}"
        );
    }
}

#[test]
fn a_refund_in_flight_is_busy_and_cannot_be_pressed_twice() {
    let mut screen = account_screen(Remote::Ready(detail()));
    screen.refund.submit = Submit::Sending;
    let page = view(&operator_model(
        &ALL_CAPABILITIES,
        AdminScreen::Account(screen),
    ));
    assert!(has_button(&page, "Sending the refund"));
    assert!(!has_button(&page, "Refund to the card"));
    assert!(elements(&page).into_iter().any(|element| {
        element.tag == "button"
            && element.attribute("aria-busy") == Some("true")
            && element.attribute("disabled").is_some()
    }));
}

#[test]
fn the_last_action_shows_done_or_pending_in_words() {
    let mut screen = account_screen(Remote::Ready(detail()));
    screen.last = Some(AdminActionView {
        audit_id: "aud-9".to_owned(),
        outcome: ActionOutcome::Done,
    });
    let done = view(&operator_model(
        &ALL_CAPABILITIES,
        AdminScreen::Account(screen.clone()),
    ));
    assert!(done.text_content().contains("Done"));
    assert!(done.text_content().contains("aud-9"));
    assert!(
        !done
            .text_content()
            .contains("final status arrives by webhook")
    );

    screen.last = Some(AdminActionView {
        audit_id: "aud-10".to_owned(),
        outcome: ActionOutcome::Pending,
    });
    let pending = view(&operator_model(
        &ALL_CAPABILITIES,
        AdminScreen::Account(screen),
    ));
    assert!(pending.text_content().contains("Pending"));
    assert!(
        pending
            .text_content()
            .contains("Sent to the processor; the final status arrives by webhook.")
    );
}

#[test]
fn a_form_whose_operation_key_failed_says_so() {
    let mut screen = account_screen(Remote::Ready(detail()));
    screen.credit.operation = Remote::Failed(failure());
    let page = view(&operator_model(
        &ALL_CAPABILITIES,
        AdminScreen::Account(screen),
    ));
    assert!(has_button(&page, "Try again"));
}

// ---------------------------------------------------------------- disputes

fn disputes_model(rows: Remote<Vec<DisputeRow>>) -> Model {
    full_model(AdminScreen::Disputes(rows_screen(rows)))
}

#[test]
fn the_dispute_list_has_an_empty_state() {
    let page = view(&disputes_model(Remote::Ready(Vec::new())));
    assert!(page.text_content().contains("There are no disputes"));
}

#[test]
fn a_dispute_shows_amount_status_words_dates_and_the_account_link() {
    let page = view(&disputes_model(Remote::Ready(vec![dispute(
        DisputeStatus::NeedsResponse,
        2 * DAY_MS + 3 * HOUR_MS,
    )])));
    let text = page.text_content();
    assert!(text.contains("$25.00"));
    assert!(text.contains("Needs a response"));
    assert!(text.contains(&money::date_time(NOW_MS - DAY_MS, 0)));
    assert!(text.contains(&money::date_time(NOW_MS + 2 * DAY_MS + 3 * HOUR_MS, 0)));
    assert!(text.contains("Due in 2d 03h"));
    assert!(has_link_to(&page, &account_route("acc-1")));
}

#[test]
fn a_dispute_past_its_evidence_deadline_reads_overdue() {
    let mut row = dispute(DisputeStatus::NeedsResponse, 0);
    row.evidence_due_at_ms = NOW_MS - HOUR_MS;
    let page = view(&disputes_model(Remote::Ready(vec![row])));
    assert!(page.text_content().contains("Overdue"));
}

#[test]
fn only_a_dispute_that_needs_a_response_offers_evidence() {
    let page = view(&disputes_model(Remote::Ready(vec![
        dispute(DisputeStatus::NeedsResponse, DAY_MS),
        dispute(DisputeStatus::UnderReview, DAY_MS),
        dispute(DisputeStatus::Won, DAY_MS),
        dispute(DisputeStatus::Lost, DAY_MS),
    ])));
    let evidence = button_labels(&page)
        .into_iter()
        .filter(|label| label == "Submit evidence")
        .count();
    assert_eq!(evidence, 1);
    assert_eq!(page.text_content().matches("Due in").count(), 1);
    assert!(
        click_intents(&page).contains(&Intent::OpenAction(ActionTarget::DisputeEvidence {
            dispute_id: "dp-1".to_owned()
        }))
    );
}

#[test]
fn the_dispute_account_is_plain_text_without_the_inspect_capability() {
    let model = operator_model(
        &[OperatorCapability::DisputeResponse],
        AdminScreen::Disputes(rows_screen(Remote::Ready(vec![dispute(
            DisputeStatus::NeedsResponse,
            DAY_MS,
        )]))),
    );
    let page = view(&model);
    assert!(page.text_content().contains("acc-1"));
    assert!(!has_link_to(&page, &account_route("acc-1")));
}

#[test]
fn the_dispute_evidence_form_requires_a_reason_and_can_be_cancelled() {
    let mut screen = rows_screen(Remote::Ready(vec![dispute(
        DisputeStatus::NeedsResponse,
        DAY_MS,
    )]));
    screen.action = Some(reason_action(ActionTarget::DisputeEvidence {
        dispute_id: "dp-1".to_owned(),
    }));
    let page = view(&full_model(AdminScreen::Disputes(screen)));
    assert!(
        page.text_content()
            .contains("Send evidence for dispute dp-1")
    );
    assert!(has_button(&page, "Send evidence"));
    assert!(has_button(&page, "Cancel"));
    assert!(bound_fields(&page).contains(&Field::ActionReason));
    assert!(page.text_content().contains("1 to 500 characters"));
    assert!(click_intents(&page).contains(&Intent::CancelEdit));
}

// ---------------------------------------------------------------- reconciliation

fn reconciliation_model(view_state: Remote<ReconciliationView>) -> Model {
    full_model(AdminScreen::Reconciliation(view_state))
}

#[test]
fn reconciliation_with_no_differences_says_so_and_shows_the_last_run() {
    let page = view(&reconciliation_model(Remote::Ready(ReconciliationView {
        last_run_at_ms: Some(NOW_MS),
        differences: Vec::new(),
    })));
    let text = page.text_content();
    assert!(text.contains("No differences at the last run"));
    assert!(text.contains(&format!("Last run: {}", money::date_time(NOW_MS, 0))));
}

#[test]
fn reconciliation_that_never_ran_does_not_claim_there_are_no_differences() {
    let page = view(&reconciliation_model(Remote::Ready(ReconciliationView {
        last_run_at_ms: None,
        differences: Vec::new(),
    })));
    let text = page.text_content();
    assert!(text.contains("has not run yet"));
    assert!(!text.contains("No differences"));
}

#[test]
fn reconciliation_differences_show_kind_reference_amount_and_found_time() {
    let page = view(&reconciliation_model(Remote::Ready(ReconciliationView {
        last_run_at_ms: Some(NOW_MS),
        differences: vec![ReconciliationDifference {
            kind: ReconciliationKind::AgedReservation,
            reference: "res_77".to_owned(),
            difference_micro_dollars: 1_500_000,
            found_at_ms: NOW_MS - HOUR_MS,
        }],
    })));
    let text = page.text_content();
    assert!(text.contains("Reservation held too long"));
    assert!(text.contains("res_77"));
    assert!(text.contains("$1.50"));
    assert!(text.contains(&money::date_time(NOW_MS - HOUR_MS, 0)));
}

// ---------------------------------------------------------------- providers

fn providers_model(rows: Remote<Vec<ProviderRow>>, capabilities: &[OperatorCapability]) -> Model {
    operator_model(capabilities, AdminScreen::Providers(rows_screen(rows)))
}

#[test]
fn providers_show_health_words_error_rate_exposure_and_the_scope_of_a_pause() {
    let page = view(&providers_model(
        Remote::Ready(vec![provider(ProviderHealth::Degraded)]),
        &ALL_CAPABILITIES,
    ));
    let text = page.text_content();
    assert!(text.contains("Alpha Voice"));
    assert!(text.contains("Degraded"));
    assert!(text.contains("Error rate 2.5%"));
    assert!(text.contains("Open exposure $3.50"));
    assert!(text.contains("does not stop read-only access to games"));
}

#[test]
fn a_healthy_provider_offers_pause_and_a_paused_one_offers_resume() {
    let healthy = view(&providers_model(
        Remote::Ready(vec![provider(ProviderHealth::Healthy)]),
        &ALL_CAPABILITIES,
    ));
    assert!(has_button(&healthy, "Pause provider"));
    assert!(!has_button(&healthy, "Resume provider"));
    assert!(
        click_intents(&healthy).contains(&Intent::OpenAction(ActionTarget::Provider {
            provider_id: "prov-1".to_owned(),
            switch: ProviderSwitch::Pause
        }))
    );

    let paused = view(&providers_model(
        Remote::Ready(vec![provider(ProviderHealth::Paused)]),
        &ALL_CAPABILITIES,
    ));
    assert!(paused.text_content().contains("Paused by an operator"));
    assert!(has_button(&paused, "Resume provider"));
    assert!(!has_button(&paused, "Pause provider"));
    assert!(
        click_intents(&paused).contains(&Intent::OpenAction(ActionTarget::Provider {
            provider_id: "prov-1".to_owned(),
            switch: ProviderSwitch::Resume
        }))
    );
}

#[test]
fn a_read_only_operator_sees_providers_without_pause_or_resume() {
    let page = view(&providers_model(
        Remote::Ready(vec![provider(ProviderHealth::Healthy)]),
        &[OperatorCapability::SupportInspect],
    ));
    assert!(page.text_content().contains("Alpha Voice"));
    assert!(!has_button(&page, "Pause provider"));
    assert!(!has_button(&page, "Resume provider"));
    assert!(
        click_intents(&page)
            .iter()
            .all(|intent| !matches!(intent, Intent::OpenAction(_)))
    );
}

#[test]
fn the_provider_list_has_an_empty_state() {
    let page = view(&providers_model(
        Remote::Ready(Vec::new()),
        &ALL_CAPABILITIES,
    ));
    assert!(page.text_content().contains("No providers are configured"));
}

#[test]
fn the_pause_form_names_the_provider_and_requires_a_reason() {
    let mut screen = rows_screen(Remote::Ready(vec![provider(ProviderHealth::Healthy)]));
    screen.action = Some(reason_action(ActionTarget::Provider {
        provider_id: "prov-1".to_owned(),
        switch: ProviderSwitch::Pause,
    }));
    let page = view(&full_model(AdminScreen::Providers(screen)));
    let text = page.text_content();
    assert!(text.contains("Pause provider Alpha Voice"));
    assert!(text.contains("1 to 500 characters"));
    assert!(bound_fields(&page).contains(&Field::ActionReason));
    assert!(has_button(&page, "Cancel"));
}

#[test]
fn a_refused_reason_form_shows_the_summary_linked_to_the_reason_and_stays_open() {
    let mut screen = rows_screen(Remote::Ready(vec![provider(ProviderHealth::Paused)]));
    let mut action = reason_action(ActionTarget::Provider {
        provider_id: "prov-1".to_owned(),
        switch: ProviderSwitch::Resume,
    });
    action.submit = Submit::Refused(ApiError::refused(Refusal::ReasonRequired));
    screen.action = Some(action);
    let page = view(&full_model(AdminScreen::Providers(screen)));
    assert!(page.text_content().contains("Resume provider Alpha Voice"));
    assert!(summary_links(&page).contains(&"#action-reason".to_owned()));
    assert!(
        page.text_content()
            .contains("Give a reason; it is kept in the audit log.")
    );
}

#[test]
fn a_reason_form_in_flight_shows_busy() {
    let mut screen = rows_screen(Remote::Ready(vec![provider(ProviderHealth::Healthy)]));
    let mut action = reason_action(ActionTarget::Provider {
        provider_id: "prov-1".to_owned(),
        switch: ProviderSwitch::Pause,
    });
    action.submit = Submit::Sending;
    screen.action = Some(action);
    let page = view(&full_model(AdminScreen::Providers(screen)));
    assert!(has_button(&page, "Pausing provider"));
}

// ---------------------------------------------------------------- review queue

fn review_model(items: Vec<ReviewItem>) -> Model {
    full_model(AdminScreen::Review(rows_screen(Remote::Ready(items))))
}

#[test]
fn an_empty_review_queue_says_nothing_is_waiting() {
    let page = view(&review_model(Vec::new()));
    assert!(
        page.text_content()
            .contains("Nothing is waiting for review")
    );
}

#[test]
fn the_review_queue_is_sorted_with_the_soonest_deadline_first() {
    let page = view(&review_model(vec![
        review_item("rv-c", "Charlie", NOW_MS + 5 * DAY_MS),
        review_item("rv-a", "Alpha", NOW_MS + HOUR_MS),
        review_item("rv-p", "Past", NOW_MS - HOUR_MS),
        review_item("rv-b", "Bravo", NOW_MS + DAY_MS),
    ]));
    let text = page.text_content();
    let position = |name: &str| text.find(name).unwrap_or(usize::MAX);
    assert!(position("Past") < position("Alpha"));
    assert!(position("Alpha") < position("Bravo"));
    assert!(position("Bravo") < position("Charlie"));
}

#[test]
fn equal_review_deadlines_keep_a_stable_order_by_item_id() {
    let page = view(&review_model(vec![
        review_item("rv-2", "Second", NOW_MS + HOUR_MS),
        review_item("rv-1", "First", NOW_MS + HOUR_MS),
    ]));
    let text = page.text_content();
    assert!(text.find("First") < text.find("Second"));
}

#[test]
fn a_review_item_shows_the_absolute_and_relative_deadline_and_a_waiting_pill() {
    let deadline = NOW_MS + 3 * HOUR_MS + 12 * MINUTE_MS;
    let page = view(&review_model(vec![review_item("rv-1", "Alpha", deadline)]));
    let text = page.text_content();
    assert!(text.contains(&format!(
        "Review by {} (due in 3h 12m)",
        money::date_time(deadline, 0)
    )));
    assert!(text.contains("Due soon"));
    assert!(text.contains("May show a real person"));
    assert!(text.contains(&format!(
        "Submitted {}",
        money::date_time(NOW_MS - HOUR_MS, 0)
    )));
}

#[test]
fn a_review_item_more_than_a_day_out_is_waiting_not_due_soon() {
    let page = view(&review_model(vec![review_item(
        "rv-1",
        "Alpha",
        NOW_MS + 2 * DAY_MS,
    )]));
    let text = page.text_content();
    assert!(text.contains("Waiting"));
    assert!(!text.contains("Due soon"));
    assert!(text.contains("(due in 2d 00h)"));
}

#[test]
fn a_review_item_past_its_deadline_says_the_upload_will_be_rejected_unreviewed() {
    let page = view(&review_model(vec![review_item(
        "rv-1",
        "Alpha",
        NOW_MS - MINUTE_MS,
    )]));
    let text = page.text_content();
    assert!(text.contains("Past the deadline: the upload will be rejected unreviewed."));
    assert!(text.contains(&format!(
        "Review by {}",
        money::date_time(NOW_MS - MINUTE_MS, 0)
    )));
    assert!(!text.contains("(due in"));
}

#[test]
fn a_review_item_with_the_deadline_exactly_now_counts_as_past() {
    let page = view(&review_model(vec![review_item("rv-1", "Alpha", NOW_MS)]));
    assert!(page.text_content().contains("Past the deadline"));
}

#[test]
fn each_review_item_offers_approve_and_reject_and_the_page_explains_the_image_rule() {
    let page = view(&review_model(vec![review_item(
        "rv-1",
        "Alpha",
        NOW_MS + HOUR_MS,
    )]));
    assert!(has_button(&page, "Approve"));
    assert!(has_button(&page, "Reject"));
    let intents = click_intents(&page);
    assert!(intents.contains(&Intent::OpenAction(ActionTarget::Review {
        item_id: "rv-1".to_owned(),
        decision: ReviewDecision::Approve
    })));
    assert!(intents.contains(&Intent::OpenAction(ActionTarget::Review {
        item_id: "rv-1".to_owned(),
        decision: ReviewDecision::Reject
    })));
    let text = page.text_content();
    assert!(text.contains("shown only through the media gateway"));
    assert!(text.contains("Approving means the portrait may be shown to the campaign"));
}

#[test]
fn the_review_reason_forms_are_named_for_the_decision_and_require_a_reason() {
    for (decision, title, submit) in [
        (
            ReviewDecision::Approve,
            "Approve upload for Alpha",
            "Approve upload",
        ),
        (
            ReviewDecision::Reject,
            "Reject upload for Alpha",
            "Reject upload",
        ),
    ] {
        let mut screen = rows_screen(Remote::Ready(vec![review_item(
            "rv-1",
            "Alpha",
            NOW_MS + HOUR_MS,
        )]));
        screen.action = Some(reason_action(ActionTarget::Review {
            item_id: "rv-1".to_owned(),
            decision,
        }));
        let page = view(&full_model(AdminScreen::Review(screen)));
        assert!(page.text_content().contains(title));
        assert!(has_button(&page, submit));
        assert!(bound_fields(&page).contains(&Field::ActionReason));
        assert!(page.text_content().contains("1 to 500 characters"));
    }
}

#[test]
fn the_review_queue_uses_the_model_clock_not_a_real_one() {
    let mut model = review_model(vec![review_item("rv-1", "Alpha", NOW_MS + HOUR_MS)]);
    assert!(view(&model).text_content().contains("due in 1h 00m"));
    model.now_ms = NOW_MS + 2 * HOUR_MS;
    assert!(view(&model).text_content().contains("Past the deadline"));
}

#[test]
fn deadlines_follow_the_viewers_time_zone() {
    let mut model = review_model(vec![review_item("rv-1", "Alpha", NOW_MS + HOUR_MS)]);
    model.offset_east_minutes = -240;
    assert!(view(&model).text_content().contains(&format!(
        "Review by {}",
        money::date_time(NOW_MS + HOUR_MS, -240)
    )));
}

// ---------------------------------------------------------------- audit log

#[test]
fn the_audit_log_shows_when_who_what_target_and_reason() {
    let page = view(&full_model(AdminScreen::Audit(Remote::Ready(vec![
        audit_row(),
    ]))));
    let text = page.text_content();
    assert!(text.contains(&money::date_time(NOW_MS, 0)));
    assert!(text.contains("operator-7"));
    assert!(text.contains("Refund"));
    assert!(text.contains("acc-1"));
    assert!(text.contains("Duplicate charge reported in case 88"));
}

#[test]
fn an_empty_audit_log_says_so() {
    let page = view(&full_model(AdminScreen::Audit(Remote::Ready(Vec::new()))));
    assert!(
        page.text_content()
            .contains("No operator actions have been recorded yet")
    );
}
