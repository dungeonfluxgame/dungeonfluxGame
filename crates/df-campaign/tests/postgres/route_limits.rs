//! Throttle layers, cookie retention, shared keys, host-only routes, membership visibility, audit
//! lines and the dev view, over the real routes and store.

use std::{
    net::IpAddr,
    sync::{Mutex, OnceLock},
};

use axum::http::{Method, StatusCode};
use df_campaign::{
    config::Tunables,
    join_invitation::JoinRole,
    records::AccountStatus,
    roles::{Delegation, Delegations},
    routes::{DEVICE_COOKIE, DEVICE_HEADER, Gate, ROUTES, RouteId, TICKET_COOKIE},
    service::ServiceKeys,
    store::{CampaignStore, SessionPhase},
};
use df_types::{AccountId, MemberId};
use serde_json::json;

use crate::{
    fixture::{session_id, tenant_id},
    routes::{
        ORIGIN, Site, approved_display, create_body, id_text, operation, paired_display,
        random_hex, redeem_display, shared_keys, table, table_with,
    },
};

fn ip(last: u8) -> IpAddr {
    format!("198.51.100.{last}").parse().unwrap()
}

#[tokio::test]
async fn invitation_links_keep_working_while_the_code_gate_is_spent() {
    let tunables = Tunables {
        lookup_limit_process_per_minute: 2,
        ..Tunables::default()
    };
    let site = Site::with_tunables("linksceiling", tunables).await;
    let host = site.person(1).await;
    let guest = site.person(2).await;
    let created = site.post("/campaigns", host, &create_body("Table")).await;
    let campaign = created.body["campaign_id"].as_str().unwrap().to_owned();
    let invitation = site
        .post(
            &format!("/campaigns/{campaign}/invitations"),
            host,
            &json!({"joins_as": "player", "lifetime_hours": null, "max_uses": null}),
        )
        .await;
    let token = invitation.body["token"].as_str().unwrap().to_owned();
    // The process ceiling for codes is spent by two lookups from two addresses.
    assert!(site.service.probe_guess_gate(ip(1), None));
    assert!(site.service.probe_guess_gate(ip(2), None));
    assert!(!site.service.probe_guess_gate(ip(3), None));
    let preview = site
        .send(
            Method::POST,
            "/join/preview",
            None,
            Some(&json!({"token": token})),
            &[("x-test-ip", "203.0.113.9".into())],
        )
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text);
    let accepted = site
        .post(
            "/join/accept",
            guest,
            &json!({"token": token, "operation": operation()}),
        )
        .await;
    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.text);
    site.cleanup().await;
}

#[tokio::test]
async fn previews_are_limited_per_address_and_a_throttled_preview_says_so() {
    let tunables = Tunables {
        preview_limit_per_ip_per_minute: 3,
        ..Tunables::default()
    };
    let site = Site::with_tunables("previewlimit", tunables).await;
    let send = || {
        let site = &site;
        async move {
            site.send(
                Method::POST,
                "/join/preview",
                None,
                Some(&json!({"token": "c".repeat(64)})),
                &[("x-test-ip", "203.0.113.50".into())],
            )
            .await
        }
    };
    for _ in 0..3 {
        assert_eq!(send().await.body, json!({"reason": "invitation_unknown"}));
    }
    let throttled = send().await;
    assert_eq!(throttled.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(throttled.body, json!({"reason": "throttled"}));
    site.cleanup().await;
}

#[tokio::test]
async fn accepting_is_limited_per_account_whatever_the_token() {
    let tunables = Tunables {
        accept_limit_per_account_per_hour: 3,
        ..Tunables::default()
    };
    let site = Site::with_tunables("acceptlimit", tunables).await;
    let guest = site.person(2).await;
    let attempt = || {
        let site = &site;
        async move {
            site.post(
                "/join/accept",
                guest,
                &json!({"token": random_hex() + &random_hex(), "operation": operation()}),
            )
            .await
        }
    };
    for _ in 0..3 {
        assert_eq!(attempt().await.status, StatusCode::NOT_FOUND);
    }
    assert_eq!(attempt().await.status, StatusCode::TOO_MANY_REQUESTS);
    site.cleanup().await;
}

#[tokio::test]
async fn creating_invitations_and_changing_members_are_limited_per_account() {
    let tunables = Tunables {
        invitation_create_limit_per_account_per_hour: 2,
        member_change_limit_per_account_per_minute: 2,
        ..Tunables::default()
    };
    let site = Site::with_tunables("changelimits", tunables).await;
    let table = table(&site).await;
    let campaign = id_text(table.campaign.as_bytes());
    let body = json!({"joins_as": "player", "lifetime_hours": null, "max_uses": null});
    for _ in 0..2 {
        let made = site
            .post(
                &format!("/campaigns/{campaign}/invitations"),
                table.host,
                &body,
            )
            .await;
        assert_eq!(made.status, StatusCode::OK);
    }
    let third = site
        .post(
            &format!("/campaigns/{campaign}/invitations"),
            table.host,
            &body,
        )
        .await;
    assert_eq!(third.status, StatusCode::TOO_MANY_REQUESTS);
    // Seat changes share the member-change limit.
    let seat = |seated: bool| {
        let site = &site;
        let campaign = campaign.clone();
        let host = table.host;
        async move {
            site.post(
                &format!("/campaigns/{campaign}/seat"),
                host,
                &json!({"seated": seated}),
            )
            .await
        }
    };
    assert_eq!(seat(false).await.status, StatusCode::OK);
    assert_eq!(seat(true).await.status, StatusCode::OK);
    assert_eq!(seat(false).await.status, StatusCode::TOO_MANY_REQUESTS);
    site.cleanup().await;
}

#[tokio::test]
async fn display_status_redeem_and_renew_each_have_their_own_limit() {
    let tunables = Tunables {
        display_status_limit_per_ip_per_minute: 2,
        display_redeem_limit_per_pairing_per_hour: 2,
        display_renew_limit_per_ticket_per_hour: 1,
        ..Tunables::default()
    };
    let site = Site::with_tunables("displaylimits", tunables).await;
    let table = table(&site).await;
    let session = session_id(1);
    site.sessions
        .set(session, table.campaign, SessionPhase::Lobby);
    let (device, ticket) = paired_display(&site, &table, session).await;
    // Redeem was used once by the helper; one more repeat is allowed, the next is throttled.
    let headers = |cookie: String| {
        vec![
            ("origin", ORIGIN.to_owned()),
            ("sec-fetch-site", "same-origin".to_owned()),
            (DEVICE_HEADER, "1".to_owned()),
            ("cookie", cookie),
        ]
    };
    let both = format!("{device}; {ticket}");
    let redeem = |cookie: String| {
        let site = &site;
        async move {
            site.send(
                Method::POST,
                "/display/redeem",
                None,
                Some(&json!({})),
                &headers(cookie),
            )
            .await
        }
    };
    assert_eq!(redeem(both.clone()).await.status, StatusCode::FORBIDDEN);
    let throttled = redeem(both.clone()).await;
    assert_eq!(throttled.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(
        throttled.cookies.is_empty(),
        "a throttle never signs a display out"
    );
    // Renewal: the first (too early) call is refused by the rules, the second by the limit.
    let renew = |cookie: String| {
        let site = &site;
        async move {
            site.send(
                Method::POST,
                "/display/renew",
                None,
                Some(&json!({})),
                &headers(cookie),
            )
            .await
        }
    };
    assert_eq!(renew(both.clone()).await.status, StatusCode::FORBIDDEN);
    assert_eq!(
        renew(both.clone()).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    // Status: two reads per address per minute.
    let status = |cookie: String| {
        let site = &site;
        async move {
            site.send(
                Method::GET,
                "/display/status",
                None,
                None,
                &[("cookie", cookie)],
            )
            .await
        }
    };
    assert_eq!(status(device.clone()).await.status, StatusCode::OK);
    assert_eq!(status(device.clone()).await.status, StatusCode::OK);
    assert_eq!(
        status(device.clone()).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    site.cleanup().await;
}

#[tokio::test]
async fn a_refusal_that_leaves_a_good_session_standing_keeps_both_display_cookies() {
    let site = Site::new("keepcookies").await;
    let table = table(&site).await;
    let session = session_id(1);
    site.sessions
        .set(session, table.campaign, SessionPhase::Lobby);
    let (device, ticket) = paired_display(&site, &table, session).await;
    let headers = |cookie: String| {
        vec![
            ("origin", ORIGIN.to_owned()),
            ("sec-fetch-site", "same-origin".to_owned()),
            (DEVICE_HEADER, "1".to_owned()),
            ("cookie", cookie),
        ]
    };
    let both = format!("{device}; {ticket}");
    // Renewing early is refused, and the display keeps both cookies.
    let early = site
        .send(
            Method::POST,
            "/display/renew",
            None,
            Some(&json!({})),
            &headers(both.clone()),
        )
        .await;
    assert_eq!(early.status, StatusCode::FORBIDDEN);
    assert!(early.cookies.is_empty(), "{:?}", early.cookies);
    // Repeating the redemption while the ticket cookie is still valid issues nothing and keeps them.
    let repeat = site
        .send(
            Method::POST,
            "/display/redeem",
            None,
            Some(&json!({})),
            &headers(both.clone()),
        )
        .await;
    assert_eq!(repeat.status, StatusCode::FORBIDDEN);
    assert!(repeat.cookies.is_empty(), "{:?}", repeat.cookies);
    // The pairing is intact: the ticket still verifies and the status says so.
    let status = site
        .send(
            Method::GET,
            "/display/status",
            None,
            None,
            &[("cookie", device.clone())],
        )
        .await;
    assert_eq!(status.body["state"], "redeemed");
    // Only an ended pairing signs the display out.
    site.post(
        &format!(
            "/campaigns/{}/sessions/{}/display/unpair",
            id_text(table.campaign.as_bytes()),
            id_text(session.as_bytes())
        ),
        table.host,
        &json!({"ticket_id": null}),
    )
    .await;
    let ended = site
        .send(
            Method::POST,
            "/display/renew",
            None,
            Some(&json!({})),
            &headers(both),
        )
        .await;
    assert_eq!(ended.status, StatusCode::FORBIDDEN);
    assert!(
        ended
            .cookies
            .iter()
            .any(|c| c.starts_with(DEVICE_COOKIE) && c.contains("Max-Age=0"))
    );
    assert!(
        ended
            .cookies
            .iter()
            .any(|c| c.starts_with(TICKET_COOKIE) && c.contains("Max-Age=0"))
    );
    site.cleanup().await;
}

#[tokio::test]
async fn replicas_with_the_same_configured_keys_resolve_each_others_codes_and_others_do_not() {
    let site = Site::new("sharedkeys").await;
    let table = table(&site).await;
    let guest = site.person(20).await;
    let (code, _) = site
        .service
        .rotate_code(table.host, table.campaign)
        .await
        .unwrap();
    let replica = site.another_service(shared_keys()).await;
    assert!(
        replica.lookup_code(ip(1), guest, &code.code).await.is_ok(),
        "a second replica with the configured keys resolves the code"
    );
    let stranger = site
        .another_service(ServiceKeys::from_hex(&"c3".repeat(32), &"d4".repeat(32)).unwrap())
        .await;
    assert_eq!(
        stranger
            .lookup_code(ip(2), guest, &code.code)
            .await
            .map(|_| ()),
        Err(df_campaign::errors::Failure::Refused(
            df_campaign::errors::Refusal::CodeUnknown
        )),
        "a process with its own random keys resolves nothing"
    );
    // Address hashes match across replicas, so a pending cap is not multiplied by replica count.
    let address: IpAddr = "2001:db8:1:2::9".parse().unwrap();
    assert_eq!(site.service.ip_keys(address), replica.ip_keys(address));
    site.cleanup().await;
}

#[test]
fn exactly_the_three_host_only_routes_carry_the_host_only_gate() {
    let mut host_only: Vec<_> = ROUTES
        .iter()
        .filter(|route| route.gate == Gate::HostOnly)
        .map(|route| route.id)
        .collect();
    host_only.sort_by_key(|id| format!("{id:?}"));
    assert_eq!(
        host_only,
        [
            RouteId::DemoteCoHost,
            RouteId::SetCoHost,
            RouteId::SetPlayerRole
        ]
    );
}

#[tokio::test]
async fn the_host_only_routes_admit_the_host_and_nobody_else() {
    let tunables = Tunables {
        maximum_seats: 64,
        ..Tunables::default()
    };
    let site = Site::with_tunables("hostonly", tunables).await;
    let table = table_with(&site, 64).await;
    let world = &site.world;
    let campaign = id_text(table.campaign.as_bytes());
    let identities: [(&str, AccountId, bool); 5] = [
        ("host", table.host, true),
        ("cohost_all", table.cohost_all, false),
        ("cohost_none", table.cohost_none, false),
        ("player", table.player, false),
        ("outsider", table.outsider, false),
    ];
    let mut next = 40_u8;
    for (label, account, allowed) in identities {
        for route in [
            RouteId::SetCoHost,
            RouteId::DemoteCoHost,
            RouteId::SetPlayerRole,
        ] {
            next += 1;
            let target = site.person(next).await;
            let member: MemberId = world
                .join(table.campaign, table.host, target, JoinRole::Player)
                .await;
            let (path, body) = match route {
                RouteId::SetCoHost => (
                    format!(
                        "/campaigns/{campaign}/members/{}/co-host",
                        id_text(member.as_bytes())
                    ),
                    json!({"invite_players": true, "start_or_pause_session": false, "export_campaign": false}),
                ),
                RouteId::DemoteCoHost => {
                    world
                        .store
                        .set_co_host(
                            table.host,
                            table.campaign,
                            member,
                            Delegations::NONE.with(Delegation::InvitePlayers),
                            world.now(),
                        )
                        .await
                        .unwrap();
                    (
                        format!(
                            "/campaigns/{campaign}/members/{}/co-host/demote",
                            id_text(member.as_bytes())
                        ),
                        json!({}),
                    )
                }
                _ => (
                    format!(
                        "/campaigns/{campaign}/members/{}/role",
                        id_text(member.as_bytes())
                    ),
                    json!({"role": "contributor"}),
                ),
            };
            let response = site.post(&path, account, &body).await;
            assert_eq!(
                response.status == StatusCode::OK,
                allowed,
                "{route:?} as {label}: {} {}",
                response.status,
                response.text
            );
        }
    }
    site.cleanup().await;
}

#[tokio::test]
async fn a_member_who_left_or_was_removed_gets_the_stranger_answer_from_my_membership() {
    let site = Site::new("formermember").await;
    let table = table(&site).await;
    let campaign = id_text(table.campaign.as_bytes());
    let me = |account: AccountId| {
        let site = &site;
        let campaign = campaign.clone();
        async move {
            site.send(
                Method::GET,
                &format!("/campaigns/{campaign}/me"),
                Some(account),
                None,
                &[],
            )
            .await
        }
    };
    assert_eq!(me(table.player).await.status, StatusCode::OK);
    let stranger = me(table.outsider).await;
    assert_eq!(stranger.status, StatusCode::NOT_FOUND);
    // Leaving.
    site.post(
        &format!("/campaigns/{campaign}/leave"),
        table.contributor,
        &json!({}),
    )
    .await;
    let left = me(table.contributor).await;
    assert_eq!(left.status, StatusCode::NOT_FOUND);
    assert_eq!(left.text, stranger.text);
    // Removal.
    let snapshot = site
        .world
        .store
        .member_snapshot(table.player, table.campaign)
        .await
        .unwrap()
        .unwrap();
    site.post(
        &format!(
            "/campaigns/{campaign}/members/{}/remove",
            id_text(snapshot.member.as_bytes())
        ),
        table.host,
        &json!({}),
    )
    .await;
    let removed = me(table.player).await;
    assert_eq!(removed.status, StatusCode::NOT_FOUND);
    assert_eq!(removed.text, stranger.text);
    site.cleanup().await;
}

#[tokio::test]
async fn an_outsider_learns_nothing_about_a_sessions_state_from_the_display_routes() {
    let site = Site::new("sessionprivacy").await;
    let table = table(&site).await;
    let campaign = id_text(table.campaign.as_bytes());
    let ended = session_id(2);
    let lobby = session_id(3);
    site.sessions
        .set(ended, table.campaign, SessionPhase::Ended);
    site.sessions
        .set(lobby, table.campaign, SessionPhase::Lobby);
    let pairing = random_hex();
    let mut bodies = Vec::new();
    for session in [session_id(9), ended, lobby] {
        for (leaf, body) in [
            ("approve", json!({"pairing_id": pairing})),
            ("unpair", json!({"ticket_id": null})),
        ] {
            let path = format!(
                "/campaigns/{campaign}/sessions/{}/display/{leaf}",
                id_text(session.as_bytes())
            );
            let response = site.post(&path, table.outsider, &body).await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{path}");
            bodies.push(response.text);
        }
    }
    bodies.dedup();
    assert_eq!(bodies.len(), 1, "every answer is the same");
    // A member with the right is told: an ended session cannot take a display.
    let told = site
        .post(
            &format!(
                "/campaigns/{campaign}/sessions/{}/display/approve",
                id_text(ended.as_bytes())
            ),
            table.host,
            &json!({"pairing_id": pairing}),
        )
        .await;
    assert_eq!(told.status, StatusCode::CONFLICT);
    site.cleanup().await;
}

#[tokio::test]
async fn the_verified_email_rule_applies_to_links_and_to_both_steps_of_the_code_path() {
    let tunables = Tunables {
        require_verified_email_to_join: true,
        ..Tunables::default()
    };
    let site = Site::with_tunables("verifiedjoin", tunables).await;
    let table = table(&site).await;
    let campaign = id_text(table.campaign.as_bytes());
    let unverified = site.world.account(60).await;
    site.accounts.add(
        unverified,
        "Unverified",
        false,
        Some(tenant_id()),
        AccountStatus::Active,
    );
    let invitation = site
        .post(
            &format!("/campaigns/{campaign}/invitations"),
            table.host,
            &json!({"joins_as": "player", "lifetime_hours": null, "max_uses": null}),
        )
        .await;
    let code = site
        .post(
            &format!("/campaigns/{campaign}/code/rotate"),
            table.host,
            &json!({}),
        )
        .await;
    let code = code.body["code"].as_str().unwrap().to_owned();
    let accept = site
        .post(
            "/join/accept",
            unverified,
            &json!({"token": invitation.body["token"], "operation": operation()}),
        )
        .await;
    let lookup = site
        .post("/join/code/lookup", unverified, &json!({"code": code}))
        .await;
    let request = site
        .post("/join/code/request", unverified, &json!({"code": code}))
        .await;
    for response in [accept, lookup, request] {
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{}", response.text);
        assert_eq!(response.body, json!({"reason": "not_allowed"}));
    }
    site.cleanup().await;
}

static AUDIT: OnceLock<Mutex<Vec<(String, String)>>> = OnceLock::new();

pub(crate) fn capture(event: &'static str, line: String) {
    AUDIT
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .push((event.to_owned(), line));
}

pub(crate) fn lines_for(needle: &str) -> Vec<(String, String)> {
    AUDIT
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, line)| line.contains(needle))
        .cloned()
        .collect()
}

#[tokio::test]
async fn audit_lines_carry_ids_and_closed_words_and_never_a_token_or_a_code() {
    df_observe::install_audit_sink(capture);
    let site = Site::new("auditlines").await;
    let host = site.person(1).await;
    let guest = site.person(2).await;
    let created = site.post("/campaigns", host, &create_body("Audited")).await;
    let campaign = created.body["campaign_id"].as_str().unwrap().to_owned();
    let invitation = site
        .post(
            &format!("/campaigns/{campaign}/invitations"),
            host,
            &json!({"joins_as": "player", "lifetime_hours": null, "max_uses": null}),
        )
        .await;
    let token = invitation.body["token"].as_str().unwrap().to_owned();
    site.post(
        "/join/accept",
        guest,
        &json!({"token": token, "operation": operation()}),
    )
    .await;
    let code = site
        .post(
            &format!("/campaigns/{campaign}/code/rotate"),
            host,
            &json!({}),
        )
        .await;
    let code = code.body["code"].as_str().unwrap().to_owned();
    let lines = lines_for(&campaign);
    let events: Vec<&str> = lines.iter().map(|(event, _)| event.as_str()).collect();
    for expected in [
        "campaign.created",
        "invite.created",
        "invite.accepted",
        "member.joined",
        "roomcode.rotated",
    ] {
        assert!(events.contains(&expected), "{expected} in {events:?}");
    }
    for (_, line) in &lines {
        assert!(!line.contains(&token), "{line}");
        assert!(
            !line.contains(&code) && !line.contains(&code.replace('-', "")),
            "{line}"
        );
    }
    let accepted = lines
        .iter()
        .find(|(event, _)| event == "invite.accepted")
        .unwrap();
    assert!(
        accepted.1.starts_with("INVITE accepted campaign="),
        "{}",
        accepted.1
    );
    assert!(accepted.1.contains(" reason=ok"));
    site.cleanup().await;
}

#[tokio::test]
async fn a_refused_join_is_audited_with_its_closed_reason() {
    df_observe::install_audit_sink(capture);
    let site = Site::new("auditrefused").await;
    let table = table(&site).await;
    let campaign = table.campaign;
    // An expired link: created, then the clock moves past it.
    let (created, _) = site
        .service
        .create_invitation(table.host, campaign, JoinRole::Player, Some(1), None)
        .await
        .unwrap();
    site.world.clock.advance_ms(2 * 3_600_000);
    let result = site.service.preview_token(ip(7), &created.token).await;
    assert!(result.is_err());
    let refused = lines_for("INVITE refused reason=expired");
    assert!(!refused.is_empty());
    site.cleanup().await;
}

#[tokio::test]
async fn the_dev_view_counts_campaigns_by_status_and_reports_throttle_saturation() {
    let site = Site::new("devview").await;
    let table = table(&site).await;
    site.post(
        &format!("/campaigns/{}/archive", id_text(table.campaign.as_bytes())),
        table.host,
        &json!({"typed_name": "The Sunken Bell"}),
    )
    .await;
    site.world.campaign(table.host, 3).await;
    let view = site.service.dev_campaigns().await.unwrap();
    assert!(view.by_status.contains(&("archived".to_owned(), 1)));
    assert!(view.by_status.contains(&("active".to_owned(), 1)));
    assert!(
        view.saturated_refusals
            .iter()
            .any(|(layer, _)| *layer == "code_ip")
    );
    let text = view.render();
    assert!(text.contains("campaigns by status") && text.contains("archived: 1"));
    site.cleanup().await;
}

fn display_headers(cookie: String) -> Vec<(&'static str, String)> {
    vec![
        ("origin", ORIGIN.to_owned()),
        ("sec-fetch-site", "same-origin".to_owned()),
        (DEVICE_HEADER, "1".to_owned()),
        ("cookie", cookie),
    ]
}

fn made_up_cookies() -> String {
    format!(
        "{DEVICE_COOKIE}={}; {TICKET_COOKIE}={}",
        random_hex() + &random_hex(),
        random_hex() + &random_hex()
    )
}

#[tokio::test]
async fn made_up_cookies_cannot_fill_the_redeem_and_renew_windows_and_a_real_pairing_still_redeems()
{
    // The per-pairing and per-ticket maps hold eight keys; the address layer would allow a
    // thousand calls a minute. Counting a window for every cookie the caller invents would fill
    // them in eight calls and stop every display for an hour.
    let tunables = Tunables {
        throttle_maximum_keys_tight: 8,
        display_ticket_limit_per_ip_per_minute: 1_000,
        ..Tunables::default()
    };
    let site = Site::with_tunables("cookieflood", tunables).await;
    let table = table(&site).await;
    let session = session_id(1);
    site.sessions
        .set(session, table.campaign, SessionPhase::Lobby);
    let device = approved_display(&site, &table, session).await;
    for round in 0..40 {
        for path in ["/display/redeem", "/display/renew"] {
            let reply = site
                .send(
                    Method::POST,
                    path,
                    None,
                    Some(&json!({})),
                    &display_headers(made_up_cookies()),
                )
                .await;
            assert_eq!(
                reply.status,
                StatusCode::FORBIDDEN,
                "{path} round {round}: {}",
                reply.text
            );
        }
    }
    // The pairing that exists redeems, and its renewal window still works.
    let redeemed = redeem_display(&site, &device).await;
    assert_eq!(redeemed.status, StatusCode::OK, "{}", redeemed.text);
    let view = site.service.dev_campaigns().await.unwrap();
    for (layer, refused) in view.saturated_refusals {
        assert_eq!(refused, 0, "layer {layer} refused for a full map");
    }
    site.cleanup().await;
}

#[tokio::test]
async fn redeem_and_renew_share_one_address_layer_that_never_signs_a_display_out() {
    let tunables = Tunables {
        display_ticket_limit_per_ip_per_minute: 3,
        ..Tunables::default()
    };
    let site = Site::with_tunables("ticketaddress", tunables).await;
    let call = |path: &'static str| {
        let site = &site;
        async move {
            site.send(
                Method::POST,
                path,
                None,
                Some(&json!({})),
                &display_headers(made_up_cookies()),
            )
            .await
        }
    };
    assert_eq!(call("/display/redeem").await.status, StatusCode::FORBIDDEN);
    assert_eq!(call("/display/renew").await.status, StatusCode::FORBIDDEN);
    assert_eq!(call("/display/redeem").await.status, StatusCode::FORBIDDEN);
    for path in ["/display/redeem", "/display/renew"] {
        let throttled = call(path).await;
        assert_eq!(throttled.status, StatusCode::TOO_MANY_REQUESTS, "{path}");
        assert!(throttled.cookies.is_empty(), "{path}");
    }
    // Another address is not affected.
    let other = site
        .send(Method::POST, "/display/redeem", None, Some(&json!({})), &{
            let mut headers = display_headers(made_up_cookies());
            headers.push(("x-test-ip", "203.0.113.200".into()));
            headers
        })
        .await;
    assert_eq!(other.status, StatusCode::FORBIDDEN);
    site.cleanup().await;
}

#[tokio::test]
async fn creating_campaigns_is_limited_per_account_before_any_work() {
    let tunables = Tunables {
        member_change_limit_per_account_per_minute: 2,
        campaigns_per_account: 50,
        ..Tunables::default()
    };
    let site = Site::with_tunables("createlimit", tunables).await;
    let host = site.person(1).await;
    for _ in 0..2 {
        let made = site.post("/campaigns", host, &create_body("Table")).await;
        assert_eq!(made.status, StatusCode::OK, "{}", made.text);
    }
    let third = site.post("/campaigns", host, &create_body("Table")).await;
    assert_eq!(
        third.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        third.text
    );
    site.cleanup().await;
}
