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
    routes::{Gate, ROUTES, RouteId},
    service::ServiceKeys,
    store::{CampaignStore, SessionPhase},
};
use df_types::{AccountId, MemberId};
use serde_json::json;

use crate::{
    fixture::{session_id, tenant_id},
    routes::{Site, create_body, id_text, operation, random_hex, shared_keys, table, table_with},
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
async fn lobbyfix_retired_display_routes_reveal_no_session_state_to_any_membership() {
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
    assert_eq!(told.status, StatusCode::NOT_FOUND);
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

#[tokio::test]
async fn a_refusal_because_a_map_was_full_is_an_alert_line_and_ordinary_throttles_are_not() {
    df_observe::install_audit_sink(capture);
    // One account key fits; the second account's first change finds the map full. One loose address
    // key fits; a preview from a second prefix finds that map full.
    let tunables = Tunables {
        throttle_maximum_keys_account: 1,
        throttle_maximum_keys_loose: 1,
        member_change_limit_per_account_per_minute: 1,
        ..Tunables::default()
    };
    let site = Site::with_tunables("saturationalert", tunables).await;
    let first = site.person(1).await;
    let second = site.person(2).await;
    assert_eq!(
        site.post("/campaigns", first, &create_body("A"))
            .await
            .status,
        StatusCode::OK
    );
    // The first account's second change is an ordinary throttle: no alert.
    assert_eq!(
        site.post("/campaigns", first, &create_body("B"))
            .await
            .status,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert!(lines_for("campaign.change_saturated").is_empty());
    let full = site.post("/campaigns", second, &create_body("C")).await;
    assert_eq!(full.status, StatusCode::TOO_MANY_REQUESTS);
    let preview = |address: &'static str| {
        let site = &site;
        async move {
            site.send(
                Method::POST,
                "/join/preview",
                None,
                Some(&json!({"token": "c".repeat(64)})),
                &[("x-test-ip", address.into())],
            )
            .await
        }
    };
    assert_eq!(preview("203.0.113.5").await.status, StatusCode::NOT_FOUND);
    assert_eq!(
        preview("198.51.100.5").await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    // Every saturated line carries the alert field, and its event name is on the alert list.
    let mut saturated = 0;
    for (event, line) in lines_for("reason=saturated") {
        saturated += 1;
        assert!(line.contains("alert=yes"), "{line}");
        assert!(
            df_campaign::audit::ALERT_EVENTS.contains(&event.as_str()),
            "{event}"
        );
    }
    assert_eq!(saturated, 2);
    for (_, line) in lines_for("reason=throttled") {
        assert!(!line.contains("alert=yes"), "{line}");
    }
    site.cleanup().await;
}
