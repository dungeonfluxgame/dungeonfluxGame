use axum::http::StatusCode;
use df_account_app::wire::{CheckoutStatus, Quantity, StatementCategory};
use df_types::CheckoutId;
use serde_json::{Value, json};

use super::fixtures::{Browser, Harness, PASSWORD};
use crate::app_api::ports::Actor;

const FORBIDDEN_WORDS: [&str; 4] = ["table_fee", "multiplier", "markup", "provider"];

async fn actor(browser: &Browser<'_>, name: &str) -> Actor {
    Actor {
        account: browser.account_id().await,
        display_name: name.to_owned(),
        is_verified: true,
    }
}

async fn balance(browser: &mut Browser<'_>) -> Value {
    browser.get("/app/api/overview").await.json()["balance"].clone()
}

async fn checkout(browser: &mut Browser<'_>, cents: u32) -> (String, String) {
    let operation = browser.operation().await;
    let reply = browser
        .post(
            "/app/api/credit/checkouts",
            &json!({"amount_cents": cents, "operation": operation}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    (
        reply.json()["checkout_id"].as_str().unwrap().to_owned(),
        operation,
    )
}

fn checkout_id(text: &str) -> CheckoutId {
    CheckoutId::from_hex(text).unwrap()
}

#[tokio::test]
async fn the_ten_dollar_trial_is_granted_exactly_once_after_verification() {
    let harness = Harness::new().await;
    let mut browser = harness.browser();
    browser.sign_up_verified("robin@example.test").await;
    for _ in 0..3 {
        let overview = browser.get("/app/api/overview").await.json();
        assert_eq!(
            overview["balance"]["promotional_available_micro_dollars"],
            json!(10_000_000)
        );
        assert_eq!(overview["trial"]["state"], json!("active"));
    }
    let statements = browser.get("/app/api/statements").await.json();
    let trial_lines = statements["lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|line| line["category"] == json!("trial_credit"))
        .count();
    assert_eq!(trial_lines, 1);
}

#[tokio::test]
async fn an_unverified_account_awaits_its_trial_and_cannot_buy_credit() {
    let harness = Harness::new().await;
    let mut browser = harness.browser();
    browser.session().await;
    let operation = browser.operation().await;
    browser
        .post(
            "/app/api/sign-up",
            &json!({"email": "robin@example.test", "password": PASSWORD, "is_adult": true,
                "accepts_terms": true,
                "terms_version": df_account_app::wire::CURRENT_TERMS_VERSION,
                "operation": operation}),
        )
        .await;
    browser.sign_in("robin@example.test", PASSWORD).await;
    let overview = browser.get("/app/api/overview").await.json();
    assert_eq!(overview["trial"]["state"], json!("awaiting_verification"));
    assert_eq!(
        overview["balance"]["promotional_available_micro_dollars"],
        json!(0)
    );
    let operation = browser.operation().await;
    let reply = browser
        .post(
            "/app/api/credit/checkouts",
            &json!({"amount_cents": 1000, "operation": operation}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.code(), "not_verified");
}

#[tokio::test]
async fn the_checkout_return_never_credits_only_the_processor_webhook_does() {
    let harness = Harness::new().await;
    let mut browser = harness.browser();
    browser.sign_up_verified("robin@example.test").await;
    let (id, _) = checkout(&mut browser, 2_000).await;
    for _ in 0..2 {
        let status = browser
            .get(&format!("/app/api/credit/checkouts/{id}"))
            .await
            .json();
        assert_eq!(status["status"], json!("pending"));
        assert_eq!(
            balance(&mut browser).await["purchased_available_micro_dollars"],
            json!(0)
        );
    }
    assert!(
        harness
            .ports
            .checkout
            .deliver_webhook(checkout_id(&id), CheckoutStatus::Paid)
    );
    let status = browser
        .get(&format!("/app/api/credit/checkouts/{id}"))
        .await
        .json();
    assert_eq!(status["status"], json!("paid"));
    assert_eq!(
        balance(&mut browser).await["purchased_available_micro_dollars"],
        json!(20_000_000)
    );
    harness
        .ports
        .checkout
        .deliver_webhook(checkout_id(&id), CheckoutStatus::Paid);
    assert_eq!(
        balance(&mut browser).await["purchased_available_micro_dollars"],
        json!(20_000_000)
    );
}

#[tokio::test]
async fn a_cancelled_or_failed_checkout_credits_nothing() {
    let harness = Harness::new().await;
    let mut browser = harness.browser();
    browser.sign_up_verified("robin@example.test").await;
    let (id, _) = checkout(&mut browser, 1_000).await;
    harness
        .ports
        .checkout
        .deliver_webhook(checkout_id(&id), CheckoutStatus::Cancelled);
    let status = browser
        .get(&format!("/app/api/credit/checkouts/{id}"))
        .await
        .json();
    assert_eq!(status["status"], json!("cancelled"));
    assert_eq!(
        balance(&mut browser).await["purchased_available_micro_dollars"],
        json!(0)
    );
}

#[tokio::test]
async fn another_accounts_checkout_is_not_found_exactly_like_a_missing_one() {
    let harness = Harness::new().await;
    let mut robin = harness.browser();
    robin.sign_up_verified("robin@example.test").await;
    let (id, _) = checkout(&mut robin, 1_000).await;
    let mut sam = harness.browser();
    sam.sign_up_verified("sam@example.test").await;
    let foreign = sam.get(&format!("/app/api/credit/checkouts/{id}")).await;
    let missing = sam
        .get(&format!("/app/api/credit/checkouts/{}", "ab".repeat(16)))
        .await;
    let malformed = sam.get("/app/api/credit/checkouts/not-an-id").await;
    for reply in [&foreign, &missing, &malformed] {
        assert_eq!(reply.status, StatusCode::NOT_FOUND);
        assert_eq!(reply.body, foreign.body);
    }
}

#[tokio::test]
async fn a_repeated_operation_id_returns_the_first_checkout_instead_of_a_second() {
    let harness = Harness::new().await;
    let mut browser = harness.browser();
    browser.sign_up_verified("robin@example.test").await;
    let (first, operation) = checkout(&mut browser, 1_000).await;
    let again = browser
        .post(
            "/app/api/credit/checkouts",
            &json!({"amount_cents": 1_000, "operation": operation}),
        )
        .await;
    assert_eq!(again.json()["checkout_id"], json!(first));
    let other = checkout(&mut browser, 1_000).await.0;
    assert_ne!(other, first);
}

#[tokio::test]
async fn a_checkout_outside_the_offered_range_is_refused_and_creates_nothing() {
    let harness = Harness::new().await;
    let mut browser = harness.browser();
    browser.sign_up_verified("robin@example.test").await;
    for cents in [0, 100, 50_001] {
        let operation = browser.operation().await;
        let reply = browser
            .post(
                "/app/api/credit/checkouts",
                &json!({"amount_cents": cents, "operation": operation}),
            )
            .await;
        assert_eq!(
            reply.json()["refusal"],
            json!("amount_out_of_range"),
            "{cents}"
        );
    }
}

/// Robin hosts a campaign with an open lobby and holds purchased credit.
async fn funded_host(harness: &Harness) -> (Browser<'_>, String, String) {
    let mut robin = harness.browser();
    robin.sign_up_verified("robin@example.test").await;
    let who = actor(&robin, "Robin").await;
    let campaign = harness
        .ports
        .campaigns
        .seed_campaign(&who, "The Lantern Road");
    let session = harness.ports.campaigns.seed_lobby(campaign).unwrap();
    robin.get("/app/api/overview").await;
    harness
        .ports
        .finance
        .credit_purchase(who.account, 2_000, "seed-checkout");
    (
        robin,
        crate::app_api::ids::hex(campaign.as_bytes()),
        crate::app_api::ids::hex(session.as_bytes()),
    )
}

#[tokio::test]
async fn allocating_moves_credit_to_the_table_once_and_never_below_zero() {
    let harness = Harness::new().await;
    let (mut robin, _, session) = funded_host(&harness).await;
    let operation = robin.operation().await;
    let body = json!({"session_id": session, "amount_cents": 1_500, "consents_to_host_spending": true,
        "operation": operation});
    let first = robin.post("/app/api/funding/allocate", &body).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    let again = robin.post("/app/api/funding/allocate", &body).await;
    assert_eq!(again.json(), first.json());
    let overview = balance(&mut robin).await;
    assert_eq!(overview["allocated_micro_dollars"], json!(15_000_000));
    assert_eq!(
        overview["purchased_available_micro_dollars"],
        json!(15_000_000)
    );
    let pot = &first.json()["campaigns"][0]["session"];
    assert_eq!(pot["pot_available_micro_dollars"], json!(15_000_000));
    assert_eq!(pot["own_allocation_micro_dollars"], json!(15_000_000));
    let operation = robin.operation().await;
    let too_much = robin
        .post(
            "/app/api/funding/allocate",
            &json!({"session_id": session, "amount_cents": 1_000_000, "consents_to_host_spending": true,
                "operation": operation}),
        )
        .await;
    assert_eq!(too_much.json()["refusal"], json!("amount_out_of_range"));
    assert_eq!(balance(&mut robin).await, overview);
}

#[tokio::test]
async fn allocating_needs_consent_and_a_table_the_caller_belongs_to() {
    let harness = Harness::new().await;
    let (mut robin, _, session) = funded_host(&harness).await;
    let operation = robin.operation().await;
    let without_consent = robin
        .post(
            "/app/api/funding/allocate",
            &json!({"session_id": session, "amount_cents": 500, "consents_to_host_spending": false,
                "operation": operation}),
        )
        .await;
    assert_eq!(without_consent.json()["refusal"], json!("consent_required"));
    let mut sam = harness.browser();
    sam.sign_up_verified("sam@example.test").await;
    let operation = sam.operation().await;
    let foreign = sam
        .post(
            "/app/api/funding/allocate",
            &json!({"session_id": session, "amount_cents": 500, "consents_to_host_spending": true,
                "operation": operation}),
        )
        .await;
    assert_eq!(foreign.code(), "not_found");
}

#[tokio::test]
async fn only_the_host_chooses_the_pace_and_a_custom_ceiling_stays_in_range() {
    let harness = Harness::new().await;
    let (mut robin, campaign, _) = funded_host(&harness).await;
    let operation = robin.operation().await;
    let set = robin
        .post(
            "/app/api/funding/pace",
            &json!({"campaign_id": campaign, "preset": "cinematic",
                "custom_ceiling_cents_per_hour": null, "operation": operation}),
        )
        .await;
    assert_eq!(set.json()["campaigns"][0]["preset"], json!("cinematic"));
    assert_eq!(
        set.json()["campaigns"][0]["ceiling_cents_per_hour"],
        json!(2_000)
    );
    for ceiling in [Value::Null, json!(5), json!(99_999)] {
        let operation = robin.operation().await;
        let bad = robin
            .post(
                "/app/api/funding/pace",
                &json!({"campaign_id": campaign, "preset": "custom",
                    "custom_ceiling_cents_per_hour": ceiling, "operation": operation}),
            )
            .await;
        assert_eq!(
            bad.json()["refusal"],
            json!("ceiling_out_of_range"),
            "{ceiling}"
        );
    }
    let mut sam = harness.browser();
    sam.sign_up_verified("sam@example.test").await;
    let operation = sam.operation().await;
    let foreign = sam
        .post(
            "/app/api/funding/pace",
            &json!({"campaign_id": campaign, "preset": "story",
                "custom_ceiling_cents_per_hour": null, "operation": operation}),
        )
        .await;
    assert_eq!(foreign.code(), "not_found");
}

#[tokio::test]
async fn statements_page_with_a_cursor_newest_first() {
    let harness = Harness::new().await;
    let mut robin = harness.browser();
    robin.sign_up_verified("robin@example.test").await;
    let who = actor(&robin, "Robin").await;
    robin.get("/app/api/overview").await;
    for index in 0..24_u64 {
        harness.ports.finance.seed_charge(
            who.account,
            StatementCategory::TableTime,
            "The Lantern Road",
            Quantity::Seconds {
                seconds: 60 + index,
            },
            10_000,
        );
    }
    let first = robin.get("/app/api/statements").await.json();
    assert_eq!(first["lines"].as_array().unwrap().len(), 20);
    let cursor = first["next_cursor"].as_str().unwrap().to_owned();
    let second = robin
        .get(&format!("/app/api/statements?cursor={cursor}"))
        .await
        .json();
    assert_eq!(second["lines"].as_array().unwrap().len(), 5);
    assert_eq!(second["next_cursor"], Value::Null);
    assert_eq!(first["lines"][0]["category"], json!("table_time"));
    let bad = robin.get("/app/api/statements?cursor=nope").await;
    assert_eq!(bad.code(), "not_found");
}

#[tokio::test]
async fn settings_list_the_signed_in_devices_and_revoking_others_signs_them_out() {
    let harness = Harness::new().await;
    let mut laptop = harness.browser();
    laptop.sign_up_verified("robin@example.test").await;
    let mut phone = harness.browser();
    phone.sign_in("robin@example.test", PASSWORD).await;
    let settings = laptop.get("/app/api/settings").await.json();
    assert_eq!(settings["email"], json!("robin@example.test"));
    let sessions = settings["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions
            .iter()
            .filter(|session| session["is_current"] == json!(true))
            .count(),
        1
    );
    let revoked = laptop
        .post("/app/api/settings/sessions/revoke-others", &json!({}))
        .await;
    assert_eq!(revoked.json(), json!({"revoked": 1}));
    assert_eq!(phone.get("/app/api/overview").await.code(), "signed_out");
    assert_eq!(laptop.get("/app/api/overview").await.status, StatusCode::OK);
}

#[tokio::test]
async fn saving_a_profile_shows_the_name_and_refuses_a_too_long_or_control_name() {
    let harness = Harness::new().await;
    let mut browser = harness.browser();
    browser.sign_up_verified("robin@example.test").await;
    let operation = browser.operation().await;
    let saved = browser
        .post(
            "/app/api/settings/profile",
            &json!({"display_name": "Robin", "operation": operation}),
        )
        .await;
    assert_eq!(saved.json(), json!({"accepted": true}));
    assert_eq!(
        browser.session().await["account"]["display_name"],
        json!("Robin")
    );
    for (name, refusal) in [
        ("x".repeat(61), "display_name_too_long"),
        ("bad\u{7}name".to_owned(), "display_name_control_character"),
    ] {
        let operation = browser.operation().await;
        let reply = browser
            .post(
                "/app/api/settings/profile",
                &json!({"display_name": name, "operation": operation}),
            )
            .await;
        assert_eq!(reply.json()["refusal"], json!(refusal));
    }
}

#[tokio::test]
async fn closing_an_account_needs_the_password_and_no_blockers() {
    let harness = Harness::new().await;
    let (mut robin, _, _) = funded_host(&harness).await;
    let blockers = robin.get("/app/api/settings").await.json()["closure_blockers"].clone();
    assert!(
        blockers
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["kind"] == json!("hosts_active_campaign"))
    );
    assert!(
        blockers
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["kind"] == json!("purchased_balance"))
    );
    let operation = robin.operation().await;
    let blocked = robin
        .post(
            "/app/api/settings/close",
            &json!({"password": PASSWORD, "operation": operation}),
        )
        .await;
    assert_eq!(blocked.code(), "invalid");
    assert_eq!(robin.get("/app/api/overview").await.status, StatusCode::OK);

    let mut sam = harness.browser();
    sam.sign_up_verified("sam@example.test").await;
    let operation = sam.operation().await;
    let wrong = sam
        .post(
            "/app/api/settings/close",
            &json!({"password": "not the password", "operation": operation}),
        )
        .await;
    assert_eq!(wrong.json()["refusal"], json!("wrong_password"));
    let closed = sam
        .post(
            "/app/api/settings/close",
            &json!({"password": PASSWORD, "operation": operation}),
        )
        .await;
    assert_eq!(closed.status, StatusCode::OK, "{}", closed.body);
    assert!(closed.json()["delete_after_ms"].as_u64().unwrap() > 0);
    assert_eq!(sam.get("/app/api/overview").await.code(), "signed_out");
}

#[tokio::test]
async fn no_customer_reply_names_a_table_fee_a_multiplier_a_markup_or_a_provider() {
    let harness = Harness::new().await;
    let (mut robin, campaign, session) = funded_host(&harness).await;
    let who = actor(&robin, "Robin").await;
    harness.ports.finance.seed_charge(
        who.account,
        StatementCategory::Narration,
        "The Lantern Road",
        Quantity::Seconds { seconds: 90 },
        250_000,
    );
    harness.ports.finance.seed_charge(
        who.account,
        StatementCategory::SceneImage,
        "The Lantern Road",
        Quantity::Images { count: 2 },
        120_000,
    );
    let (checkout_id, _) = checkout(&mut robin, 1_000).await;
    let operation = robin.operation().await;
    let allocation = json!({"session_id": session, "amount_cents": 500,
        "consents_to_host_spending": true, "operation": operation});
    let mut bodies = vec![
        robin
            .post("/app/api/funding/allocate", &allocation)
            .await
            .body,
        robin.session().await.to_string(),
    ];
    let reads = [
        "overview",
        "credit/offers",
        "funding",
        "statements",
        "settings",
        "campaigns",
        "campaigns/new",
        "join/pending",
    ];
    for read in reads {
        bodies.push(robin.get(&format!("/app/api/{read}")).await.body);
    }
    for path in [
        format!("credit/checkouts/{checkout_id}"),
        format!("campaigns/{campaign}"),
        format!("sessions/{session}/lobby"),
    ] {
        bodies.push(robin.get(&format!("/app/api/{path}")).await.body);
    }
    let mut operations = Vec::new();
    for _ in 0..5 {
        operations.push(robin.operation().await);
    }
    let writes = [
        (
            "funding/pace".to_owned(),
            json!({"campaign_id": campaign, "preset": "cinematic",
                "custom_ceiling_cents_per_hour": null, "operation": operations[0]}),
        ),
        (
            "credit/checkouts".to_owned(),
            json!({"amount_cents": 2_000, "operation": operations[1]}),
        ),
        ("campaigns/drafts".to_owned(), json!({})),
        (
            format!("campaigns/{campaign}/invitations"),
            json!({"kind": "contributor", "operation": operations[2]}),
        ),
        (
            format!("sessions/{session}/lobby/start"),
            json!({"operation": operations[3]}),
        ),
        (
            "settings/profile".to_owned(),
            json!({"display_name": "Robin", "operation": operations[4]}),
        ),
        ("settings/sessions/revoke-others".to_owned(), json!({})),
    ];
    for (path, body) in writes {
        let reply = robin.post(&format!("/app/api/{path}"), &body).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}: {}", reply.body);
        bodies.push(reply.body);
    }
    assert!(bodies.iter().all(|body| body.len() > 2));
    for body in bodies {
        let text = body.to_ascii_lowercase();
        for word in FORBIDDEN_WORDS {
            assert!(!text.contains(word), "{word} in {body}");
        }
    }
}

#[tokio::test]
async fn statements_filter_totals_and_monthly_summary_cover_the_filtered_range() {
    let harness = Harness::new().await;
    let mut browser = harness.browser();
    browser.sign_up_verified("robin@example.test").await;
    let account = browser.account_id().await;
    harness.ports.finance.seed_credit_at(
        account,
        1_700_000_000_000,
        StatementCategory::Purchase,
        20_000_000,
    );
    harness.ports.finance.seed_charge_at(
        account,
        1_700_100_000_000,
        StatementCategory::TableTime,
        "The Lantern Road",
        Quantity::Seconds { seconds: 3600 },
        3_000_000,
    );
    let topups = browser
        .get("/app/api/statements?filter=top_ups")
        .await
        .json();
    assert!(
        topups["lines"]
            .as_array()
            .unwrap()
            .iter()
            .all(|line| line["direction"] == json!("credit"))
    );
    assert_eq!(
        topups["totals"]["topped_up_micro_dollars"],
        json!(20_000_000)
    );
    let spend = browser.get("/app/api/statements?filter=spend").await.json();
    assert_eq!(spend["totals"]["spent_micro_dollars"], json!(3_000_000));
    assert_eq!(
        spend["per_campaign"][0]["campaign_name"],
        json!("The Lantern Road")
    );
    assert!(!spend["monthly"].as_array().unwrap().is_empty());
    let ranged = browser
        .get("/app/api/statements?from_ms=1700000000000&to_ms=1700050000000")
        .await
        .json();
    assert_eq!(
        ranged["totals"]["topped_up_micro_dollars"],
        json!(20_000_000)
    );
    assert_eq!(ranged["totals"]["spent_micro_dollars"], json!(0));
    let bad = browser.get("/app/api/statements?filter=nope").await;
    assert_eq!(bad.code(), "invalid");
    for word in FORBIDDEN_WORDS {
        assert!(!topups.to_string().to_ascii_lowercase().contains(word));
    }
}

#[tokio::test]
async fn play_sessions_assets_and_stats_come_from_the_ports_and_are_paged() {
    let harness = Harness::new().await;
    let mut browser = harness.browser();
    browser.sign_up_verified("robin@example.test").await;
    let account = browser.account_id().await;
    harness.ports.campaigns.seed_play_session(
        account,
        df_account_app::wire::PlaySessionRow {
            session_id: "s1".to_owned(),
            campaign_id: "c1".to_owned(),
            campaign_name: "The Lantern Road".to_owned(),
            started_at_ms: 1_700_000_000_000,
            ended_at_ms: Some(1_700_007_200_000),
            duration_seconds: 7200,
            players: 4,
            spent_micro_dollars: 2_000_000,
        },
    );
    harness.ports.assets.seed_asset(
        account,
        df_account_app::wire::AssetRow {
            asset_id: "asset-1".to_owned(),
            kind: df_account_app::wire::AssetKind::Portrait,
            campaign_id: Some("c1".to_owned()),
            campaign_name: Some("The Lantern Road".to_owned()),
            created_at_ms: 1_700_000_000_000,
            size_bytes: 1234,
            thumbnail_url: Some("/app/media/thumb/asset-1".to_owned()),
            hold_state: df_account_app::wire::AssetHoldState::Ready,
        },
    );
    let sessions = browser.get("/app/api/play-sessions").await.json();
    assert_eq!(
        sessions["sessions"][0]["campaign_name"],
        json!("The Lantern Road")
    );
    assert_eq!(sessions["sessions"][0]["duration_seconds"], json!(7200));
    let assets = browser.get("/app/api/assets?kind=portrait").await.json();
    assert_eq!(assets["total"], json!(1));
    assert_eq!(assets["assets"][0]["kind"], json!("portrait"));
    let none = browser.get("/app/api/assets?kind=video").await.json();
    assert_eq!(none["total"], json!(0));
    let stats = browser.get("/app/api/account-stats").await.json();
    assert_eq!(stats["play_sessions"], json!(1));
    assert_eq!(stats["played_seconds_total"], json!(7200));
    assert_eq!(stats["assets_generated"], json!(1));
    // Another account sees none of it.
    let mut other = harness.browser();
    other.sign_up_verified("ana@example.test").await;
    assert_eq!(
        other.get("/app/api/play-sessions").await.json()["sessions"],
        json!([])
    );
    assert_eq!(other.get("/app/api/assets").await.json()["total"], json!(0));
    let mut visitor = harness.browser();
    assert_eq!(
        visitor.get("/app/api/play-sessions").await.code(),
        "signed_out"
    );
    assert_eq!(visitor.get("/app/api/assets").await.code(), "signed_out");
    assert_eq!(
        visitor.get("/app/api/account-stats").await.code(),
        "signed_out"
    );
}

#[tokio::test]
async fn settings_shows_account_details_and_the_management_forms_work() {
    let harness = Harness::new().await;
    let mut browser = harness.browser();
    browser.sign_up_verified("robin@example.test").await;
    let settings = browser.get("/app/api/settings").await.json();
    assert_eq!(settings["email"], json!("robin@example.test"));
    assert_eq!(settings["is_verified"], json!(true));
    assert!(settings["created_at_ms"].as_u64().unwrap() > 0);
    assert_eq!(settings["sign_in_methods"], json!(["password"]));
    assert_eq!(
        settings["email_preferences"]["transactional_always_on"],
        json!(true)
    );
    // Change password needs the current password and signs out other devices.
    let wrong = browser
        .post(
            "/app/api/settings/password",
            &json!({"current_password": "not the password", "password": "a brand new passphrase", "confirmation": "a brand new passphrase"}),
        )
        .await;
    assert_eq!(wrong.json()["refusal"], json!("wrong_password"));
    let changed = browser
        .post(
            "/app/api/settings/password",
            &json!({"current_password": PASSWORD, "password": "a brand new passphrase", "confirmation": "a brand new passphrase"}),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    browser.csrf = changed.json()["csrf"].as_str().unwrap().to_owned();
    // Email change only sends a confirmation; the address does not change yet.
    let operation = browser.operation().await;
    let sent = browser
        .post(
            "/app/api/settings/email",
            &json!({"new_email": "new@example.test", "password": "a brand new passphrase", "operation": operation}),
        )
        .await;
    assert_eq!(sent.json(), json!({"accepted": true}));
    let after = browser.get("/app/api/settings").await.json();
    assert_eq!(after["email"], json!("robin@example.test"));
    // Preferences and export.
    let operation = browser.operation().await;
    let saved = browser
        .post(
            "/app/api/settings/preferences",
            &json!({"product_news": true, "session_reminders": false, "operation": operation}),
        )
        .await;
    assert_eq!(saved.json(), json!({"accepted": true}));
    let after = browser.get("/app/api/settings").await.json();
    assert_eq!(after["email_preferences"]["product_news"], json!(true));
    let operation = browser.operation().await;
    let export = browser
        .post(
            "/app/api/settings/export",
            &json!({"password": "a brand new passphrase", "operation": operation}),
        )
        .await;
    assert_eq!(export.json()["status"], json!("requested"));
}

#[tokio::test]
async fn signing_out_one_device_and_everywhere_revokes_those_sessions() {
    let harness = Harness::new().await;
    let mut first = harness.browser();
    first.sign_up_verified("robin@example.test").await;
    let mut second = harness.browser();
    second.sign_in("robin@example.test", PASSWORD).await;
    let settings = first.get("/app/api/settings").await.json();
    let sessions = settings["sessions"].as_array().unwrap();
    assert!(sessions.len() >= 2, "{settings}");
    let other_id = sessions
        .iter()
        .find(|session| session["is_current"] == json!(false))
        .unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let revoked = first
        .post(
            "/app/api/settings/sessions/revoke",
            &json!({"session_id": other_id}),
        )
        .await;
    assert_eq!(revoked.json(), json!({"revoked": 1}));
    assert_eq!(second.get("/app/api/overview").await.code(), "signed_out");
    assert_eq!(first.get("/app/api/overview").await.status, StatusCode::OK);
    let all = first
        .post("/app/api/settings/sessions/revoke-all", &json!({}))
        .await;
    assert!(all.json()["revoked"].as_u64().unwrap() >= 1);
    assert_eq!(first.get("/app/api/overview").await.code(), "signed_out");
}
