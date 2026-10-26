use axum::http::StatusCode;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

use super::fixtures::{Browser, Harness, ORIGIN};
use crate::app_api::{ids::hex, ports::Actor};

async fn actor(browser: &Browser<'_>, name: &str) -> Actor {
    Actor {
        account: browser.account_id().await,
        display_name: name.to_owned(),
        is_verified: true,
    }
}

/// Robin hosts "The Lantern Road"; returns the campaign id.
async fn host_with_campaign(harness: &Harness) -> (Browser<'_>, String) {
    let mut robin = harness.browser();
    robin.sign_up_verified("robin@example.test").await;
    let who = actor(&robin, "Robin").await;
    let campaign = harness
        .ports
        .campaigns
        .seed_campaign(&who, "The Lantern Road");
    (robin, hex(campaign.as_bytes()))
}

async fn invite(host: &mut Browser<'_>, campaign: &str, kind: &str) -> Value {
    let operation = host.operation().await;
    host.post(
        &format!("/app/api/campaigns/{campaign}/invitations"),
        &json!({"kind": kind, "operation": operation}),
    )
    .await
    .json()
}

fn token_of(link: &str) -> String {
    link.split_once("#token=").unwrap().1.to_owned()
}

async fn accept(browser: &mut Browser<'_>, token: Option<&str>) -> super::fixtures::Reply {
    let operation = browser.operation().await;
    browser
        .post(
            "/app/api/join/accept",
            &json!({"token": token, "operation": operation}),
        )
        .await
}

#[tokio::test]
async fn another_accounts_campaign_and_lobby_are_not_found_exactly_like_missing_ones() {
    let harness = Harness::new().await;
    let (mut robin, campaign) = host_with_campaign(&harness).await;
    let session = harness
        .ports
        .campaigns
        .seed_lobby(crate::app_api::ids::campaign(&campaign).unwrap())
        .unwrap();
    let session = hex(session.as_bytes());
    assert_eq!(
        robin
            .get(&format!("/app/api/campaigns/{campaign}"))
            .await
            .status,
        StatusCode::OK
    );
    let mut sam = harness.browser();
    sam.sign_up_verified("sam@example.test").await;
    let missing = "ab".repeat(16);
    let paths = [
        (
            format!("/app/api/campaigns/{campaign}"),
            format!("/app/api/campaigns/{missing}"),
        ),
        (
            format!("/app/api/sessions/{session}/lobby"),
            format!("/app/api/sessions/{missing}/lobby"),
        ),
    ];
    for (foreign, absent) in paths {
        let foreign = sam.get(&foreign).await;
        let absent = sam.get(&absent).await;
        assert_eq!(foreign.status, StatusCode::NOT_FOUND);
        assert_eq!(foreign.body, absent.body);
    }
    let dashboard = sam.get("/app/api/campaigns").await.json();
    assert_eq!(dashboard["campaigns"], json!([]));
}

#[tokio::test]
async fn removed_wizard_endpoints_are_not_found_for_a_signed_in_account() {
    let harness = Harness::new().await;
    let mut host = harness.browser();
    host.sign_up_verified("wizard-removed@example.test").await;
    assert_eq!(
        host.get("/app/api/campaigns/new").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        host.post("/app/api/campaigns/drafts", &json!({}))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    for path in [
        "/app/api/campaigns/drafts/11111111111111111111111111111111/steps/1",
        "/app/api/campaigns/drafts/11111111111111111111111111111111/finish",
    ] {
        let request = if path.ends_with("/finish") {
            super::fixtures::Req::new(axum::http::Method::POST, path)
                .json(&json!({"operation": host.operation().await}))
        } else {
            super::fixtures::Req::new(axum::http::Method::PUT, path).json(&json!({}))
        };
        assert_eq!(
            host.send(request).await.status,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
}

#[tokio::test]
async fn an_invitation_link_is_shown_once_and_keeps_its_token_in_the_fragment() {
    let harness = Harness::new().await;
    let (mut host, campaign) = host_with_campaign(&harness).await;
    let created = invite(&mut host, &campaign, "player").await;
    let link = created["link"].as_str().unwrap();
    assert!(
        link.starts_with(&format!("{ORIGIN}/app/join#token=")),
        "{link}"
    );
    let token = token_of(link);
    let overview = host.get(&format!("/app/api/campaigns/{campaign}")).await;
    assert!(!overview.body.contains(&token));
    assert_eq!(overview.json()["invitations"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn only_a_host_or_co_host_can_invite() {
    let harness = Harness::new().await;
    let (mut host, campaign) = host_with_campaign(&harness).await;
    let link = invite(&mut host, &campaign, "player").await["link"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut sam = harness.browser();
    sam.sign_up_verified("sam@example.test").await;
    assert_eq!(
        accept(&mut sam, Some(&token_of(&link))).await.status,
        StatusCode::OK
    );
    let refused = invite(&mut sam, &campaign, "player").await;
    assert_eq!(refused["code"], json!("forbidden"));
}

#[tokio::test]
async fn a_held_invitation_survives_sign_in_and_joins_without_the_token_in_a_url() {
    let harness = Harness::new().await;
    let (mut host, campaign) = host_with_campaign(&harness).await;
    let link = invite(&mut host, &campaign, "player").await["link"]
        .as_str()
        .unwrap()
        .to_owned();
    let token = token_of(&link);
    let mut guest = harness.browser();
    guest.session().await;
    let preview = guest
        .post("/app/api/join/preview", &json!({"token": token}))
        .await;
    assert_eq!(
        preview.json(),
        json!({"campaign_name": "The Lantern Road", "host_name": "Robin",
            "scheduled_at_ms": null, "content_rating": "teen"})
    );
    let held = guest
        .post("/app/api/join/hold", &json!({"token": token}))
        .await;
    let cookie = held.set_cookie("__Host-df_join").unwrap();
    assert!(
        cookie.contains("HttpOnly") && cookie.contains("Secure") && cookie.contains("Max-Age=3600")
    );
    assert_eq!(
        guest.get("/app/api/join/pending").await.json()["preview"]["campaign_name"],
        json!("The Lantern Road")
    );
    let mut sam = harness.browser();
    sam.sign_up_verified("sam@example.test").await;
    sam.cookies.insert("__Host-df_join".into(), token.clone());
    let accepted = accept(&mut sam, None).await;
    assert_eq!(accepted.json()["campaign_id"], json!(campaign));
    assert!(
        accepted
            .set_cookie("__Host-df_join")
            .unwrap()
            .contains("Max-Age=0")
    );
    let dashboard = sam.get("/app/api/campaigns").await.json();
    assert_eq!(dashboard["campaigns"][0]["role"], json!("player"));
    let again = accept(&mut sam, Some(&token)).await;
    assert_eq!(again.json()["reason"], json!("already_joined"));
}

#[tokio::test]
async fn a_failed_invitation_carries_a_reason_and_never_a_preview() {
    let harness = Harness::new().await;
    let (mut host, campaign) = host_with_campaign(&harness).await;
    let created = invite(&mut host, &campaign, "player").await;
    let token = token_of(created["link"].as_str().unwrap());
    let mut guest = harness.browser();
    guest.session().await;
    let unknown = guest
        .post("/app/api/join/preview", &json!({"token": "ab".repeat(32)}))
        .await;
    let malformed = guest
        .post("/app/api/join/preview", &json!({"token": "not a token"}))
        .await;
    assert_eq!(unknown.json()["reason"], json!("no_longer_valid"));
    assert_eq!(unknown.body, malformed.body);
    let operation = host.operation().await;
    host.post(
        &format!(
            "/app/api/campaigns/{campaign}/invitations/{}/revoke",
            created["invitation_id"].as_str().unwrap()
        ),
        &json!({"operation": operation}),
    )
    .await;
    let revoked = guest
        .post("/app/api/join/preview", &json!({"token": token}))
        .await;
    assert_eq!(revoked.json()["reason"], json!("revoked"));
    for reply in [&unknown, &revoked] {
        assert!(!reply.body.contains("Lantern") && !reply.body.contains("Robin"));
        assert_eq!(reply.status, StatusCode::NOT_FOUND);
    }
    let second = invite(&mut host, &campaign, "player").await;
    harness.clock.advance_ms(8 * 24 * 60 * 60 * 1000);
    let expired = guest
        .post(
            "/app/api/join/preview",
            &json!({"token": token_of(second["link"].as_str().unwrap())}),
        )
        .await;
    assert_eq!(expired.json()["reason"], json!("expired"));
}

#[tokio::test]
async fn a_full_table_turns_players_away_but_not_contributors() {
    let harness = Harness::new().await;
    let (_host, campaign) = host_with_campaign(&harness).await;
    let id = crate::app_api::ids::campaign(&campaign).unwrap();
    for (index, name) in ["Ann", "Bo", "Cy"].iter().enumerate() {
        let account =
            df_types::AccountId::from_bytes(&[u8::try_from(index).unwrap() + 1; 16]).unwrap();
        harness.ports.campaigns.seed_member(
            id,
            &Actor {
                account,
                display_name: (*name).to_owned(),
                is_verified: true,
            },
            df_account_app::wire_campaign::CampaignRole::Player,
        );
    }
    let player = harness
        .ports
        .campaigns
        .seed_invitation(id, df_account_app::wire_campaign::InvitationKind::Player)
        .unwrap();
    let contributor = harness
        .ports
        .campaigns
        .seed_invitation(
            id,
            df_account_app::wire_campaign::InvitationKind::Contributor,
        )
        .unwrap();
    let mut sam = harness.browser();
    sam.sign_up_verified("sam@example.test").await;
    assert_eq!(
        accept(&mut sam, Some(&token_of(&player))).await.json()["reason"],
        json!("full")
    );
    let joined = accept(&mut sam, Some(&token_of(&contributor))).await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    assert_eq!(
        sam.get("/app/api/campaigns").await.json()["campaigns"][0]["role"],
        json!("contributor")
    );
}

#[tokio::test]
async fn a_room_code_request_waits_for_the_host_to_approve_or_deny() {
    let harness = Harness::new().await;
    let (mut host, campaign) = host_with_campaign(&harness).await;
    let operation = host.operation().await;
    let rotated = host
        .post(
            &format!("/app/api/campaigns/{campaign}/code/rotate"),
            &json!({"operation": operation}),
        )
        .await
        .json();
    let code = rotated["room_code"]["code"].as_str().unwrap().to_owned();
    let mut sam = harness.browser();
    sam.sign_up_verified("sam@example.test").await;
    let looked = sam
        .post(
            "/app/api/join/code/lookup",
            &json!({"code": code.to_lowercase()}),
        )
        .await;
    assert_eq!(looked.json()["campaign_name"], json!("The Lantern Road"));
    let wrong = sam
        .post("/app/api/join/code/lookup", &json!({"code": "ZZZZ-ZZZZ"}))
        .await;
    assert_eq!(wrong.json()["reason"], json!("no_longer_valid"));
    let operation = sam.operation().await;
    let request = sam
        .post(
            "/app/api/join/code/request",
            &json!({"code": code, "operation": operation}),
        )
        .await;
    assert_eq!(request.json(), json!({"campaign_name": "The Lantern Road"}));
    assert_eq!(
        sam.get("/app/api/campaigns").await.json()["campaigns"],
        json!([])
    );
    let overview = host
        .get(&format!("/app/api/campaigns/{campaign}"))
        .await
        .json();
    let request_id = overview["join_requests"][0]["request_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let operation = host.operation().await;
    let approved = host
        .post(
            &format!("/app/api/campaigns/{campaign}/requests/{request_id}/approve"),
            &json!({"role": "player", "operation": operation}),
        )
        .await;
    assert_eq!(approved.json()["members"].as_array().unwrap().len(), 2);
    assert_eq!(approved.json()["join_requests"], json!([]));
    assert_eq!(
        sam.get("/app/api/campaigns").await.json()["campaigns"][0]["role"],
        json!("player")
    );
}

#[tokio::test]
async fn the_host_cannot_leave_but_a_player_can() {
    let harness = Harness::new().await;
    let (mut host, campaign) = host_with_campaign(&harness).await;
    let link = invite(&mut host, &campaign, "player").await["link"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut sam = harness.browser();
    sam.sign_up_verified("sam@example.test").await;
    accept(&mut sam, Some(&token_of(&link))).await;
    let leave = format!("/app/api/campaigns/{campaign}/leave");
    let operation = host.operation().await;
    assert_eq!(
        host.post(&leave, &json!({"operation": operation}))
            .await
            .code(),
        "forbidden"
    );
    let operation = sam.operation().await;
    assert_eq!(
        sam.post(&leave, &json!({"operation": operation}))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        sam.get("/app/api/campaigns").await.json()["campaigns"],
        json!([])
    );
}

#[tokio::test]
async fn fundinghook_lobby_exposes_only_the_hosts_credit_and_waits_for_consent() {
    use crate::app_api::{ids, ports::FinancePort};
    use df_account_app::wire_campaign::{HostConsentRequest, HostCreditView};
    let harness = Harness::new().await;
    let (mut host, campaign) = host_with_campaign(&harness).await;
    let account = host.account_id().await;
    let session = harness
        .ports
        .campaigns
        .seed_lobby(ids::campaign(&campaign).unwrap())
        .unwrap();
    let quote = HostCreditView {
        price_version: [7; 16],
        hourly_micro_dollars: 3_000_000,
        available_micro_dollars: 10_000_000,
        cap_micro_dollars: 3_000_000,
        terms_version: 1,
        consented_at_ms: None,
        is_fundable: false,
    };
    harness
        .ports
        .finance
        .seed_host_credit(account, session, quote);
    let path = format!("/app/api/sessions/{}/lobby", ids::hex(session.as_bytes()));
    let waiting = host.get(&path).await.json();
    assert_eq!(
        waiting["funding_note"]["host_credit"]["available_micro_dollars"],
        10_000_000
    );
    assert!(
        waiting["start_blockers"]
            .as_array()
            .unwrap()
            .contains(&json!("funding_not_ready"))
    );
    let invitation = invite(&mut host, &campaign, "player").await;
    let mut player = harness.browser();
    player.sign_up_verified("funded-player@example.test").await;
    let token = token_of(invitation["link"].as_str().unwrap());
    assert_eq!(
        accept(&mut player, Some(&token)).await.status,
        StatusCode::OK
    );
    let private = player.get(&path).await.json();
    assert!(private["funding_note"].is_string());
    assert!(!private.to_string().contains("available_micro_dollars"));
    let operation = ids::operation(&host.operation().await).unwrap();
    harness
        .ports
        .finance
        .consent_host_credit(
            account,
            session,
            HostConsentRequest {
                price_version: quote.price_version,
                cap_micro_dollars: quote.cap_micro_dollars,
                terms_version: quote.terms_version,
                operation: ids::hex(operation.as_bytes()),
            },
            operation,
        )
        .await
        .unwrap();
    let approved = host.get(&path).await.json();
    assert_eq!(approved["funding_note"]["host_credit"]["is_fundable"], true);
    assert_eq!(
        approved["funding_note"]["host_credit"]["available_micro_dollars"],
        10_000_000
    );
    assert!(
        !approved["start_blockers"]
            .as_array()
            .unwrap()
            .contains(&json!("funding_not_ready"))
    );
}

#[tokio::test]
async fn fundinghook_fake_refuses_a_stale_quote_and_a_different_account() {
    use crate::app_api::{
        ids,
        ports::{FinancePort, PortError},
    };
    use df_account_app::wire_campaign::{HostConsentRequest, HostCreditView};
    let harness = Harness::new().await;
    let (host, campaign) = host_with_campaign(&harness).await;
    let account = host.account_id().await;
    let session = harness
        .ports
        .campaigns
        .seed_lobby(ids::campaign(&campaign).unwrap())
        .unwrap();
    harness.ports.finance.seed_host_credit(
        account,
        session,
        HostCreditView {
            price_version: [7; 16],
            hourly_micro_dollars: 3_000_000,
            available_micro_dollars: 3_000_000,
            cap_micro_dollars: 3_000_000,
            terms_version: 1,
            consented_at_ms: None,
            is_fundable: false,
        },
    );
    let operation = ids::operation(&"01".repeat(16)).unwrap();
    let stale = HostConsentRequest {
        price_version: [8; 16],
        cap_micro_dollars: 3_000_000,
        terms_version: 1,
        operation: ids::hex(operation.as_bytes()),
    };
    assert_eq!(
        harness
            .ports
            .finance
            .consent_host_credit(account, session, stale.clone(), operation)
            .await,
        Err(PortError::Forbidden)
    );
    let foreign = ids::account(&"ff".repeat(16)).unwrap();
    assert_eq!(
        harness
            .ports
            .finance
            .consent_host_credit(foreign, session, stale, operation)
            .await,
        Err(PortError::Forbidden)
    );
    assert!(
        harness
            .ports
            .finance
            .host_credit(foreign, session)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn fundingwire_router_records_host_consent_and_cancels_only_for_the_host() {
    use crate::app_api::ids;
    use df_account_app::wire_campaign::HostCreditView;
    let harness = Harness::new().await;
    let (mut host, campaign) = host_with_campaign(&harness).await;
    let account = host.account_id().await;
    let session = harness
        .ports
        .campaigns
        .seed_lobby(ids::campaign(&campaign).unwrap())
        .unwrap();
    let quote = HostCreditView {
        price_version: [7; 16],
        hourly_micro_dollars: 3_000_000,
        available_micro_dollars: 10_000_000,
        cap_micro_dollars: 3_000_000,
        terms_version: 1,
        consented_at_ms: None,
        is_fundable: false,
    };
    harness
        .ports
        .finance
        .seed_host_credit(account, session, quote);
    let lobby = format!("/app/api/sessions/{}/lobby", ids::hex(session.as_bytes()));
    let path = format!("{lobby}/funding/consent");
    let operation = host.operation().await;
    let request = json!({"price_version":quote.price_version, "cap_micro_dollars":quote.cap_micro_dollars,
        "terms_version":1, "operation":operation});
    let accepted = host.post(&path, &request).await;
    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.json());
    assert_eq!(
        accepted.json()["funding_note"]["host_credit"]["is_fundable"],
        true
    );
    assert_eq!(
        accepted.json()["funding_note"]["host_credit"]["available_micro_dollars"],
        10_000_000
    );
    let replay = host.post(&path, &request).await;
    assert_eq!(replay.status, StatusCode::OK);
    assert_eq!(
        replay.json()["funding_note"]["host_credit"]["consented_at_ms"],
        accepted.json()["funding_note"]["host_credit"]["consented_at_ms"]
    );
    let mut stranger = harness.browser();
    stranger
        .sign_up_verified("fundingwire-stranger@example.test")
        .await;
    let operation = stranger.operation().await;
    assert_ne!(
        stranger
            .post(&format!("{lobby}/cancel"), &json!({"operation":operation}))
            .await
            .status,
        StatusCode::OK
    );
    let operation = host.operation().await;
    assert_eq!(
        host.post(&format!("{lobby}/cancel"), &json!({"operation":operation}))
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn lobbyfix_play_now_uses_defaults_once() {
    let harness = Harness::new().await;
    let mut host = harness.browser();
    host.sign_up_verified("play-now@example.test").await;
    let operation = host.operation().await;
    let request = json!({"operation": operation});
    let first = host.post("/app/api/campaigns/play-now", &request).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    let view = first.json();
    let repeated = host
        .post("/app/api/campaigns/play-now", &request)
        .await
        .json();
    assert_eq!(repeated["campaign_id"], view["campaign_id"]);
    assert_eq!(repeated["lobby"]["session_id"], view["lobby"]["session_id"]);
    assert_eq!(view["game_unavailable"], true);
    // The new account's free trial covers the default pace's cap, so the same click starts it.
    assert_eq!(view["lobby"]["has_started"], true);
    let campaign = view["campaign_id"].as_str().unwrap();
    let settings = host
        .get(&format!("/app/api/campaigns/{campaign}/settings"))
        .await
        .json();
    let draft = &settings["draft"];
    assert_eq!(draft["name"], settings["catalog"]["adventures"][0]["name"]);
    assert_eq!(
        draft["adventure_id"],
        settings["catalog"]["adventures"][0]["adventure_id"]
    );
    assert_eq!(draft["content_rating"], "teen");
    assert_eq!(draft["seat_count"], 6);
    assert_eq!(draft["has_voice"], true);
    assert_eq!(draft["pictures"], "automatic");
    assert_eq!(draft["pace"], "immersive");
    assert_eq!(draft["join_mode"], "link_and_room_code");
    assert_eq!(draft["funding"], "sponsor");
    let overview = host.get("/app/api/overview").await.json();
    assert_eq!(overview["balance"]["allocated_micro_dollars"], 0);
    assert_eq!(overview["balance"]["reserved_micro_dollars"], 0);
}

#[tokio::test]
async fn play_now_with_enough_credit_approves_it_and_starts_in_the_same_click() {
    use crate::app_api::{ids, ports::FinancePort};
    let harness = Harness::new().await;
    let mut host = harness.browser();
    host.sign_up_verified("play-now-funded@example.test").await;
    let account = host.account_id().await;
    harness
        .ports
        .finance
        .credit_purchase(account, 2_000, "play-now-funded");
    let operation = host.operation().await;
    let request = json!({"operation": operation});
    let first = host.post("/app/api/campaigns/play-now", &request).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    let view = first.json();
    assert_eq!(view["lobby"]["has_started"], true, "{view}");
    let session = ids::session(view["lobby"]["session_id"].as_str().unwrap()).unwrap();
    let approved = harness
        .ports
        .finance
        .host_credit(account, session)
        .await
        .unwrap()
        .expect("the host's quote");
    assert!(approved.consented_at_ms.is_some());
    assert!(approved.is_fundable);
    // A retry of the same click reuses the campaign, session and recorded approval.
    let repeated = host
        .post("/app/api/campaigns/play-now", &request)
        .await
        .json();
    assert_eq!(repeated["campaign_id"], view["campaign_id"]);
    assert_eq!(repeated["lobby"]["session_id"], view["lobby"]["session_id"]);
    let after_retry = harness
        .ports
        .finance
        .host_credit(account, session)
        .await
        .unwrap()
        .expect("the host's quote");
    assert_eq!(after_retry.consented_at_ms, approved.consented_at_ms);
}

#[tokio::test]
async fn play_now_again_returns_to_the_running_table_instead_of_starting_another() {
    use crate::app_api::{ids, ports::FinancePort};
    let harness = Harness::new().await;
    let mut host = harness.browser();
    host.sign_up_verified("play-again@example.test").await;
    let account = host.account_id().await;
    let operation = host.operation().await;
    let first = host
        .post(
            "/app/api/campaigns/play-now",
            &json!({"operation": operation}),
        )
        .await
        .json();
    assert_eq!(first["lobby"]["has_started"], true, "{first}");
    let session = ids::session(first["lobby"]["session_id"].as_str().unwrap()).unwrap();
    let approved = harness
        .ports
        .finance
        .host_credit(account, session)
        .await
        .unwrap()
        .expect("the host's quote")
        .consented_at_ms;
    // A new click, with a new operation, while that table runs.
    let operation = host.operation().await;
    let again = host
        .post(
            "/app/api/campaigns/play-now",
            &json!({"operation": operation}),
        )
        .await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    let again = again.json();
    assert_eq!(again["campaign_id"], first["campaign_id"]);
    assert_eq!(again["lobby"]["session_id"], first["lobby"]["session_id"]);
    assert_eq!(
        host.get("/app/api/campaigns").await.json()["campaigns"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let after = harness
        .ports
        .finance
        .host_credit(account, session)
        .await
        .unwrap()
        .expect("the host's quote")
        .consented_at_ms;
    assert_eq!(after, approved);
}

#[tokio::test]
async fn lobbyfix_settings_save_two_changes_and_reuse_the_hosts_last_rating() {
    use super::fixtures::Req;
    use axum::http::Method;
    let harness = Harness::new().await;
    let mut host = harness.browser();
    host.sign_up_verified("settings-now@example.test").await;
    let operation = host.operation().await;
    let created = host
        .post(
            "/app/api/campaigns/play-now",
            &json!({"operation": operation}),
        )
        .await
        .json();
    let campaign = created["campaign_id"].as_str().unwrap();
    let path = format!("/app/api/campaigns/{campaign}/settings");
    let mut draft = host.get(&path).await.json()["draft"].clone();
    draft["name"] = json!("A renamed adventure");
    draft["content_rating"] = json!("mature");
    draft["has_voice"] = json!(false);
    let saved = host
        .send(Req::new(Method::PUT, &path).json(&json!({"draft": draft})))
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
    let read = host.get(&path).await.json();
    assert_eq!(read["draft"]["name"], "A renamed adventure");
    assert_eq!(read["draft"]["content_rating"], "mature");
    assert_eq!(read["draft"]["has_voice"], false);
    let operation = host.operation().await;
    let next = host
        .post(
            "/app/api/campaigns/play-now",
            &json!({"operation": operation}),
        )
        .await
        .json();
    let next = next["campaign_id"].as_str().unwrap();
    assert_eq!(
        host.get(&format!("/app/api/campaigns/{next}/settings"))
            .await
            .json()["draft"]["content_rating"],
        "mature"
    );
}

#[tokio::test]
async fn lobbyfix_membership_has_no_game_room_fields_or_input_routes() {
    let harness = Harness::new().await;
    let (mut host, campaign) = host_with_campaign(&harness).await;
    let session = harness
        .ports
        .campaigns
        .seed_lobby(crate::app_api::ids::campaign(&campaign).unwrap())
        .unwrap();
    let path = format!("/app/api/sessions/{}/lobby", hex(session.as_bytes()));
    let view = host.get(&path).await.json();
    assert_eq!(view["start_blockers"], json!([]));
    assert!(view.get("displays").is_none());
    let member = view["members"][0].as_object().unwrap();
    assert_eq!(member.len(), 2);
    assert!(member.contains_key("display_name") && member.contains_key("is_me"));
    for route in [
        "character",
        "ready",
        "unready",
        "microphone",
        "display/lookup",
        "display/approve",
        "display/unpair-all",
    ] {
        let reply = host.post(&format!("{path}/{route}"), &json!({})).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{route}");
    }
    for route in ["challenge", "redeem", "renew"] {
        assert_eq!(
            host.post(&format!("/app/api/display/{route}"), &json!({}))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
    }
}

#[tokio::test]
async fn lobbyfix_start_still_requires_host_authority_and_funding_but_no_hero_or_readiness() {
    let harness = Harness::new().await;
    let (mut host, campaign) = host_with_campaign(&harness).await;
    let session = harness
        .ports
        .campaigns
        .seed_lobby(crate::app_api::ids::campaign(&campaign).unwrap())
        .unwrap();
    let invitation = invite(&mut host, &campaign, "player").await;
    let mut player = harness.browser();
    player.sign_up_verified("room-player@example.test").await;
    accept(
        &mut player,
        Some(&token_of(invitation["link"].as_str().unwrap())),
    )
    .await;
    let path = format!("/app/api/sessions/{}/lobby/start", hex(session.as_bytes()));
    let operation = player.operation().await;
    assert_eq!(
        player
            .post(&path, &json!({"operation": operation}))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let operation = host.operation().await;
    let started = host.post(&path, &json!({"operation": operation})).await;
    assert_eq!(started.status, StatusCode::OK, "{}", started.body);
    assert_eq!(started.json()["has_started"], true);
    let launch_path = format!("/app/api/sessions/{}/game", hex(session.as_bytes()));
    let waiting = host.get(&launch_path).await;
    assert_eq!(waiting.status, StatusCode::OK, "{}", waiting.body);
    let waiting = waiting.json();
    assert_eq!(waiting["has_started"], true);
    assert_eq!(waiting["launcher_url"], Value::Null);
    assert!(!waiting.to_string().contains("DF_GAMEPLAY"));
    let session_hex = hex(session.as_bytes());
    let links = |display: &str| df_account_app::wire_campaign::GameLinks {
        display_url: display.to_owned(),
        player_url: format!("https://game.example.test/sessions/{session_hex}/player?room=ABCDEF"),
        room_code: Some("ABCDEF".to_owned()),
    };
    // A link carrying credentials is never handed to the browser.
    harness.ports.campaigns.seed_game_links(
        session,
        links("https://user:secret@game.example.test/display"),
    );
    assert_eq!(
        player.get(&launch_path).await.json()["launcher_url"],
        Value::Null
    );
    harness.ports.campaigns.seed_game_links(
        session,
        links(&format!(
            "https://game.example.test/sessions/{session_hex}/display"
        )),
    );
    let launcher = format!("https://launcher.example.test/sessions/{session_hex}");
    harness
        .ports
        .campaigns
        .seed_launcher_url(session, launcher.clone());
    // Placed, but the account only ever hands off to the game launcher: it never carries the table
    // or player links or the room code (planning/client-architecture.md "Game start flow").
    let manager_view = host.get(&launch_path).await.json();
    assert_eq!(manager_view["launcher_url"], launcher);
    let opened = player.get(&launch_path).await.json();
    let text = opened.to_string();
    assert!(!text.contains("/display") && !text.contains("/player") && !text.contains("ABCDEF"));
    // A plain member never receives the manager's launcher address.
    assert_eq!(opened["launcher_url"], Value::Null);
}

#[test]
fn a_configured_launcher_address_is_only_returned_to_a_session_manager() {
    let address = "https://launcher.example.test";
    assert_eq!(
        crate::app_api::campaign::game_launcher_url(false, true, true, None, Some(address)),
        None
    );
    assert_eq!(
        crate::app_api::campaign::game_launcher_url(true, true, true, None, Some(address)),
        Some(address.to_owned())
    );
    assert_eq!(
        crate::app_api::campaign::game_launcher_url(true, false, true, None, Some(address)),
        None
    );
    assert_eq!(
        crate::app_api::campaign::game_launcher_url(true, true, false, None, Some(address)),
        None
    );
    assert_eq!(
        crate::app_api::campaign::game_launcher_url(
            true,
            true,
            true,
            Some("https://placed.example.test/session"),
            Some(address),
        ),
        Some("https://placed.example.test/session".to_owned())
    );
    assert_eq!(
        crate::app_api::campaign::game_launcher_url(
            true,
            true,
            true,
            Some("javascript:alert(1)"),
            Some(address),
        ),
        None
    );
}

#[test]
fn game_unavailable_audits_repeat_only_when_the_reason_changes_or_a_minute_passes() {
    let now = Instant::now();
    let mut audits = crate::app_api::campaign::GameUnavailableAudits::new();
    assert!(audits.should_record("session-a", "runtime_unavailable", now));
    assert!(!audits.should_record(
        "session-a",
        "runtime_unavailable",
        now + Duration::from_secs(59)
    ));
    assert!(audits.should_record(
        "session-a",
        "runtime_deadline",
        now + Duration::from_secs(59)
    ));
    assert!(audits.should_record(
        "session-a",
        "runtime_deadline",
        now + Duration::from_secs(119)
    ));
}

#[test]
fn game_unavailable_audit_memory_evicts_the_oldest_session_at_its_bound() {
    let now = Instant::now();
    let mut audits = crate::app_api::campaign::GameUnavailableAudits::new();
    for index in 0..1_024 {
        assert!(audits.should_record(
            &format!("session-{index}"),
            "placement_missing",
            now + Duration::from_secs(index)
        ));
    }
    assert!(audits.should_record(
        "session-new",
        "placement_missing",
        now + Duration::from_secs(2_000)
    ));
    assert_eq!(audits.len(), 1_024);
    assert!(audits.should_record(
        "session-0",
        "placement_missing",
        now + Duration::from_secs(2_001)
    ));
}

#[tokio::test]
async fn lobbyfix_play_now_and_settings_preserve_csrf_and_same_origin_guards() {
    use super::fixtures::Req;
    use axum::http::Method;
    let harness = Harness::new().await;
    let mut host = harness.browser();
    host.sign_up_verified("csrf-play@example.test").await;
    let operation = host.operation().await;
    let request = json!({"operation": operation});
    assert_eq!(
        host.send(
            Req::new(Method::POST, "/app/api/campaigns/play-now")
                .json(&request)
                .csrf(None)
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        host.send(
            Req::new(Method::POST, "/app/api/campaigns/play-now")
                .json(&request)
                .origin(Some("https://foreign.example.test"))
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn lobbyfix_a_co_host_cannot_bypass_sponsor_consent_or_read_the_sponsors_wallet() {
    use crate::app_api::ids;
    use df_account_app::wire_campaign::HostCreditView;
    // A lobby still waiting for the sponsor's approval (Play now with enough credit starts at once).
    let harness = Harness::new().await;
    let (mut host, campaign) = host_with_campaign(&harness).await;
    let account = host.account_id().await;
    let campaign = ids::campaign(&campaign).unwrap();
    let session_id = harness.ports.campaigns.seed_lobby(campaign).unwrap();
    harness.ports.finance.seed_host_credit(
        account,
        session_id,
        HostCreditView {
            price_version: [7; 16],
            hourly_micro_dollars: 8_000_000,
            available_micro_dollars: 10_000_000,
            cap_micro_dollars: 8_000_000,
            terms_version: 1,
            consented_at_ms: None,
            is_fundable: false,
        },
    );
    let session = ids::hex(session_id.as_bytes());
    let mut co_host = harness.browser();
    co_host.sign_up_verified("co-host-now@example.test").await;
    harness.ports.campaigns.seed_member(
        campaign,
        &actor(&co_host, "Co-host").await,
        df_account_app::wire_campaign::CampaignRole::CoHost,
    );
    let path = format!("/app/api/sessions/{session}/lobby");
    let view = co_host.get(&path).await.json();
    assert_eq!(view["can_manage"], true);
    assert_eq!(view["start_blockers"], json!(["funding_not_ready"]));
    assert_eq!(view["funding_note"], "waiting_for_sponsor");
    assert!(!view.to_string().contains("available_micro_dollars"));
    let operation = co_host.operation().await;
    assert_eq!(
        co_host
            .post(&format!("{path}/start"), &json!({"operation": operation}))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(host.get(&path).await.json()["has_started"], false);
}

#[tokio::test]
async fn lobbyfix_a_failure_after_creation_is_unknown_and_retry_opens_the_same_campaign() {
    let harness = Harness::new().await;
    let mut host = harness.browser();
    host.sign_up_verified("partial-play@example.test").await;
    let operation = host.operation().await;
    let body = json!({"operation": operation});
    harness.ports.campaigns.fail_next_open();
    let partial = host.post("/app/api/campaigns/play-now", &body).await;
    assert_eq!(partial.code(), "unknown_outcome");
    let before = host.get("/app/api/campaigns").await.json();
    assert_eq!(before["campaigns"].as_array().unwrap().len(), 1);
    let campaign = before["campaigns"][0]["campaign_id"].clone();
    let retry = host.post("/app/api/campaigns/play-now", &body).await;
    assert_eq!(retry.status, StatusCode::OK, "{}", retry.body);
    assert_eq!(retry.json()["campaign_id"], campaign);
    assert_eq!(
        host.get("/app/api/campaigns").await.json()["campaigns"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
