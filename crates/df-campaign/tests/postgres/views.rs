//! The reads behind the screens: the dashboard, a campaign's overview, its settings and its
//! sessions tab, over the real routes and store.

use axum::http::{Method, StatusCode};
use df_campaign::{
    config::Tunables, join_invitation::JoinRole, records::AccountStatus, store::SessionPhase,
};
use serde_json::{Value, json};

use crate::{
    closure::archive,
    fixture::{session_id, tenant_id},
    routes::{Response, Site, create_body, id_text, table, table_with},
};

async fn get(site: &Site, path: &str, account: df_types::AccountId) -> Response {
    site.send(Method::GET, path, Some(account), None, &[]).await
}

fn names(list: &Value, key: &str) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|item| item[key].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn a_new_account_sees_the_empty_state_and_then_its_campaigns_drafts_and_roles() {
    let site = Site::new("dashboard").await;
    let host = site.person(1).await;
    let guest = site.person(2).await;
    let empty = get(&site, "/campaigns", guest).await;
    assert_eq!(empty.status, StatusCode::OK, "{}", empty.text);
    assert_eq!(empty.body["empty"], true);
    // The host makes two campaigns; the guest plays in the first and is a stranger to the second.
    let first = site.world.campaign(host, 4).await;
    site.world
        .join(first.id, host, guest, JoinRole::Player)
        .await;
    site.world.clock.advance_ms(1_000);
    let second = site
        .post("/campaigns", host, &create_body("Second table"))
        .await;
    let second_id = second.body["campaign_id"].as_str().unwrap().to_owned();
    let draft = site.post("/campaigns/drafts", host, &json!({})).await;
    let host_view = get(&site, "/campaigns", host).await;
    assert_eq!(host_view.body["empty"], false);
    let campaigns = &host_view.body["campaigns"];
    // The most recently active campaign comes first.
    assert_eq!(campaigns[0]["campaign_id"], second_id.as_str());
    assert_eq!(
        names(campaigns, "name"),
        ["Second table", "The Sunken Bell"]
    );
    assert_eq!(campaigns[0]["role"], "host");
    assert_eq!(campaigns[0]["is_owner"], true);
    assert_eq!(campaigns[1]["seats_in_use"], 2);
    assert_eq!(campaigns[1]["seat_count"], 4);
    assert_eq!(campaigns[1]["status"], "active");
    assert_eq!(
        host_view.body["drafts"][0]["draft_id"],
        draft.body["draft_id"]
    );
    assert_eq!(host_view.body["drafts"][0]["step"], 1);
    // The guest sees only the campaign they play in, as a player who is not the owner.
    let guest_view = get(&site, "/campaigns", guest).await;
    assert_eq!(guest_view.body["campaigns"].as_array().unwrap().len(), 1);
    assert_eq!(guest_view.body["campaigns"][0]["role"], "player");
    assert_eq!(guest_view.body["campaigns"][0]["is_owner"], false);
    assert_eq!(guest_view.body["drafts"].as_array().unwrap().len(), 0);
    // An archived campaign is listed as archived; one the guest left is gone from their list.
    archive(&site.world, host, first.id).await;
    let after = get(&site, "/campaigns", host).await;
    let archived = after.body["campaigns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "The Sunken Bell")
        .unwrap();
    assert_eq!(archived["status"], "archived");
    site.cleanup().await;
}

#[tokio::test]
async fn a_left_campaign_and_a_deleting_one_are_not_listed() {
    use df_campaign::store::CampaignStore;
    let site = Site::new("dashboardgone").await;
    let host = site.person(1).await;
    let guest = site.person(2).await;
    let campaign = site.world.campaign(host, 4).await;
    site.world
        .join(campaign.id, host, guest, JoinRole::Player)
        .await;
    assert_eq!(
        get(&site, "/campaigns", guest).await.body["campaigns"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    site.world
        .store
        .leave_campaign(guest, campaign.id, site.world.now())
        .await
        .unwrap();
    assert_eq!(get(&site, "/campaigns", guest).await.body["empty"], true);
    // A campaign flagged for deletion disappears from its host's list.
    archive(&site.world, host, campaign.id).await;
    site.world
        .store
        .request_delete(
            host,
            campaign.id,
            "The Sunken Bell".into(),
            false,
            site.world.now(),
        )
        .await
        .unwrap();
    assert_eq!(get(&site, "/campaigns", host).await.body["empty"], true);
    site.cleanup().await;
}

#[tokio::test]
async fn the_dashboard_asks_for_live_sessions_of_a_bounded_number_and_never_hides_a_failure() {
    let tunables = Tunables {
        dashboard_session_lookups: 1,
        ..Tunables::default()
    };
    let site = Site::with_tunables("dashboardsession", tunables).await;
    let host = site.person(1).await;
    let older = site.world.campaign(host, 4).await;
    site.world.clock.advance_ms(1_000);
    let newer = site
        .post("/campaigns", host, &create_body("Newer"))
        .await
        .body["campaign_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let newer_id = df_types::CampaignId::from_hex(&newer).unwrap();
    site.sessions
        .set(session_id(1), newer_id, SessionPhase::Lobby);
    site.sessions
        .set(session_id(2), older.id, SessionPhase::Playing);
    let view = get(&site, "/campaigns", host).await;
    let entries = &view.body["campaigns"];
    // Only the most recently active campaign is asked.
    assert_eq!(entries[0]["session"]["state"], "live");
    assert_eq!(entries[0]["session"]["phase"], "lobby");
    assert_eq!(
        entries[0]["session"]["session_id"],
        id_text(session_id(1).as_bytes())
    );
    assert_eq!(entries[1]["session"]["state"], "not_checked");
    // An unreadable O1 is `unknown`, never "no session".
    site.sessions.fail_reads(true);
    let down = get(&site, "/campaigns", host).await;
    assert_eq!(down.body["campaigns"][0]["session"]["state"], "unknown");
    site.sessions.fail_reads(false);
    site.cleanup().await;
}

#[tokio::test]
async fn the_overview_lists_the_roster_the_counts_and_what_each_member_may_do() {
    let site = Site::new("overview").await;
    let table = table_with(&site, 6).await;
    let campaign = id_text(table.campaign.as_bytes());
    let host_view = get(&site, &format!("/campaigns/{campaign}"), table.host).await;
    assert_eq!(host_view.status, StatusCode::OK, "{}", host_view.text);
    let body = &host_view.body;
    assert_eq!(body["name"], "The Sunken Bell");
    assert_eq!(body["status"], "active");
    assert_eq!(body["seat_count"], 6);
    // The host, two co-hosts and a player hold seats; the contributor does not.
    assert_eq!(body["seats_in_use"], 4);
    assert_eq!(body["contributors_in_use"], 1);
    assert_eq!(body["maximum_contributors"], 12);
    assert_eq!(body["members"].as_array().unwrap().len(), 5);
    assert_eq!(body["members"][0]["role"], "host");
    assert_eq!(body["members"][0]["is_you"], true);
    assert_eq!(body["members"][0]["display_name"], "Person 1");
    assert_eq!(body["you"]["role"], "host");
    assert_eq!(body["is_owner"], true);
    assert_eq!(body["can_invite"], true);
    assert_eq!(body["can_start"], true);
    assert_eq!(body["can_edit_settings"], true);
    assert_eq!(body["pending_requests"], 0);
    assert_eq!(body["tabs"], json!(["overview", "sessions", "settings"]));
    assert_eq!(body["session"]["state"], "none");
    // A player sees the same roster but none of the host's controls.
    let player = get(&site, &format!("/campaigns/{campaign}"), table.player).await;
    assert_eq!(player.body["members"].as_array().unwrap().len(), 5);
    assert_eq!(player.body["you"]["role"], "player");
    assert_eq!(player.body["is_owner"], false);
    assert_eq!(player.body["can_invite"], false);
    assert_eq!(player.body["can_start"], false);
    assert_eq!(player.body["can_edit_settings"], false);
    assert!(player.body["pending_requests"].is_null());
    // The co-host with the invite bit decides requests; the one without sees no count.
    let cohost = get(&site, &format!("/campaigns/{campaign}"), table.cohost_all).await;
    assert_eq!(cohost.body["can_invite"], true);
    assert_eq!(cohost.body["can_edit_settings"], false);
    assert!(cohost.body["pending_requests"].is_number());
    let plain = get(&site, &format!("/campaigns/{campaign}"), table.cohost_none).await;
    assert_eq!(plain.body["can_invite"], false);
    assert!(plain.body["pending_requests"].is_null());
    site.cleanup().await;
}

#[tokio::test]
async fn the_pending_request_count_and_the_mounted_tabs_follow_the_configuration() {
    let tunables = Tunables {
        tab_funding_mounted: true,
        tab_characters_mounted: true,
        ..Tunables::default()
    };
    let site = Site::with_tunables("overviewtabs", tunables).await;
    let table = table(&site).await;
    let campaign = id_text(table.campaign.as_bytes());
    let rotated = site
        .post(
            &format!("/campaigns/{campaign}/code/rotate"),
            table.host,
            &json!({}),
        )
        .await;
    let code = rotated.body["code"].as_str().unwrap().to_owned();
    let guest = site.person(20).await;
    let request = site
        .post("/join/code/request", guest, &json!({"code": code}))
        .await;
    assert_eq!(request.status, StatusCode::OK, "{}", request.text);
    let view = get(&site, &format!("/campaigns/{campaign}"), table.host).await;
    assert_eq!(view.body["pending_requests"], 1);
    assert_eq!(
        view.body["tabs"],
        json!(["overview", "sessions", "characters", "funding", "settings"])
    );
    assert_eq!(view.body["room_codes"], true);
    site.cleanup().await;
}

#[tokio::test]
async fn strangers_former_members_drafts_and_closing_accounts_get_nothing() {
    let site = Site::new("overviewdenied").await;
    let table = table(&site).await;
    let campaign = id_text(table.campaign.as_bytes());
    for path in [
        format!("/campaigns/{campaign}"),
        format!("/campaigns/{campaign}/settings"),
        format!("/campaigns/{campaign}/sessions"),
    ] {
        let stranger = get(&site, &path, table.outsider).await;
        assert_eq!(stranger.status, StatusCode::NOT_FOUND, "{path}");
        assert!(!stranger.text.contains("The Sunken Bell"), "{path}");
        assert_eq!(
            get(&site, &path, table.host).await.status,
            StatusCode::OK,
            "{path}"
        );
    }
    // A draft is not a campaign, even for its owner.
    let draft = site
        .post("/campaigns/drafts", table.host, &json!({}))
        .await
        .body["draft_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        get(&site, &format!("/campaigns/{draft}"), table.host)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    // A member whose account is closing is refused, not shown the roster.
    site.world
        .db
        .set_account_status(table.player, "closing")
        .await;
    site.accounts.add(
        table.player,
        "Person 4",
        true,
        Some(tenant_id()),
        AccountStatus::Closing,
    );
    let closing = get(&site, &format!("/campaigns/{campaign}"), table.player).await;
    assert_eq!(closing.status, StatusCode::FORBIDDEN);
    assert!(!closing.text.contains("Person"));
    site.cleanup().await;
}

#[tokio::test]
async fn the_settings_tab_shows_the_wizards_values_and_only_the_host_may_edit() {
    let site = Site::new("settingsview").await;
    let owner = site.person(1).await;
    let player = site.person(2).await;
    let draft = site.post("/campaigns/drafts", owner, &json!({})).await.body["draft_id"]
        .as_str()
        .unwrap()
        .to_owned();
    for (step, body) in [
        (
            1,
            json!({"name": "Ember Court", "adventure_kind": "bundled", "content_rating": "gentle", "seat_count": 3}),
        ),
        (2, json!({"hero": "choose_in_lobby"})),
        (
            3,
            json!({"voice_default": "off", "picture_default": "automatic", "spend_preset": "custom", "custom_hourly_micro_dollars": 3_000_000, "session_ceiling_micro_dollars": 12_000_000}),
        ),
        (
            4,
            json!({"default_co_host": {"invite_players": true, "start_or_pause_session": true, "export_campaign": false}, "room_codes": true, "funding_model": "named_sponsor"}),
        ),
    ] {
        let saved = site
            .send(
                Method::PUT,
                &format!("/campaigns/drafts/{draft}/steps/{step}"),
                Some(owner),
                Some(&body),
                &[],
            )
            .await;
        assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    }
    let finished = site
        .post(
            &format!("/campaigns/drafts/{draft}/finish"),
            owner,
            &json!({"operation": crate::routes::operation()}),
        )
        .await;
    assert_eq!(finished.status, StatusCode::OK, "{}", finished.text);
    let campaign = df_types::CampaignId::from_hex(&draft).unwrap();
    site.world
        .join(campaign, owner, player, JoinRole::Player)
        .await;
    let settings = get(&site, &format!("/campaigns/{draft}/settings"), owner).await;
    assert_eq!(settings.status, StatusCode::OK, "{}", settings.text);
    let body = &settings.body;
    assert_eq!(body["name"], "Ember Court");
    assert_eq!(body["content_rating"], "gentle");
    assert_eq!(body["seat_count"], 3);
    assert_eq!(body["voice_default"], "off");
    assert_eq!(body["picture_default"], "automatic");
    assert_eq!(body["spend_preset"], "custom");
    assert_eq!(body["custom_hourly_micro_dollars"], 3_000_000);
    assert_eq!(body["session_ceiling_micro_dollars"], 12_000_000);
    assert_eq!(body["default_co_host"]["start_or_pause_session"], true);
    assert_eq!(body["room_codes"], true);
    assert_eq!(body["funding_model"], "named_sponsor");
    assert_eq!(body["editable"], true);
    // A player reads the same values and may not edit them.
    let read_only = get(&site, &format!("/campaigns/{draft}/settings"), player).await;
    assert_eq!(read_only.body["name"], "Ember Court");
    assert_eq!(read_only.body["editable"], false);
    site.cleanup().await;
}

#[tokio::test]
async fn the_sessions_tab_reports_the_live_session_or_that_it_cannot_tell() {
    let site = Site::new("sessionstab").await;
    let table = table(&site).await;
    let campaign = id_text(table.campaign.as_bytes());
    let none = get(
        &site,
        &format!("/campaigns/{campaign}/sessions"),
        table.player,
    )
    .await;
    assert_eq!(none.body["session"]["state"], "none");
    site.sessions.set(
        session_id(5),
        table.campaign,
        SessionPhase::PreparingFundingOpen,
    );
    let live = get(
        &site,
        &format!("/campaigns/{campaign}/sessions"),
        table.player,
    )
    .await;
    assert_eq!(live.body["session"]["state"], "live");
    assert_eq!(live.body["session"]["phase"], "preparing");
    site.sessions.fail_reads(true);
    let unknown = get(
        &site,
        &format!("/campaigns/{campaign}/sessions"),
        table.player,
    )
    .await;
    assert_eq!(unknown.body["session"]["state"], "unknown");
    site.sessions.fail_reads(false);
    site.cleanup().await;
}
