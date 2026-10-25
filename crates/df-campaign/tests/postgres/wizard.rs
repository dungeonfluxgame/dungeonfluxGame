//! The create wizard over the real routes and store: drafts, steps, resuming and finishing.

use axum::http::{Method, StatusCode};
use df_campaign::{config::Tunables, records::AccountStatus};
use serde_json::{Value, json};

use crate::{
    fixture::tenant_id,
    routes::{Response, Site, operation},
};

fn about(name: &str, seats: u32) -> Value {
    json!({"name": name, "adventure_kind": "bundled", "content_rating": "mature", "seat_count": seats})
}

fn table_step() -> Value {
    json!({
        "voice_default": "off",
        "picture_default": "automatic",
        "spend_preset": "custom",
        "custom_hourly_micro_dollars": 2_000_000,
        "session_ceiling_micro_dollars": 9_000_000
    })
}

fn joining_step() -> Value {
    json!({
        "default_co_host": {"invite_players": true, "start_or_pause_session": false, "export_campaign": true},
        "room_codes": false,
        "funding_model": "shared_pot"
    })
}

async fn put(site: &Site, path: &str, account: df_types::AccountId, body: &Value) -> Response {
    site.send(Method::PUT, path, Some(account), Some(body), &[])
        .await
}

async fn get(site: &Site, path: &str, account: df_types::AccountId) -> Response {
    site.send(Method::GET, path, Some(account), None, &[]).await
}

async fn start(site: &Site, account: df_types::AccountId) -> (String, Response) {
    let started = site.post("/campaigns/drafts", account, &json!({})).await;
    let id = started.body["draft_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    (id, started)
}

async fn save(
    site: &Site,
    account: df_types::AccountId,
    draft: &str,
    step: u8,
    body: &Value,
) -> Response {
    put(
        site,
        &format!("/campaigns/drafts/{draft}/steps/{step}"),
        account,
        body,
    )
    .await
}

async fn save_all(site: &Site, account: df_types::AccountId, draft: &str) {
    for (step, body) in [
        (1, about("The Sunken Bell", 5)),
        (2, json!({"hero": "choose_in_lobby"})),
        (3, table_step()),
        (4, joining_step()),
        (5, json!({})),
    ] {
        let saved = save(site, account, draft, step, &body).await;
        assert_eq!(saved.status, StatusCode::OK, "step {step}: {}", saved.text);
    }
}

#[tokio::test]
async fn a_draft_is_walked_through_five_steps_and_finishes_into_a_campaign() {
    let site = Site::new("wizardwalk").await;
    let owner = site.person(1).await;
    let (draft, started) = start(&site, owner).await;
    assert_eq!(started.status, StatusCode::OK, "{}", started.text);
    assert_eq!(started.body["step"], 1);
    assert_eq!(started.body["name"], "Untitled campaign");
    save_all(&site, owner, &draft).await;
    let resumed = get(&site, &format!("/campaigns/drafts/{draft}"), owner).await;
    assert_eq!(resumed.body["step"], 5);
    assert_eq!(resumed.body["name"], "The Sunken Bell");
    assert_eq!(resumed.body["seat_count"], 5);
    assert_eq!(resumed.body["content_rating"], "mature");
    assert_eq!(resumed.body["voice_default"], "off");
    assert_eq!(resumed.body["picture_default"], "automatic");
    assert_eq!(resumed.body["spend_preset"], "custom");
    assert_eq!(resumed.body["custom_hourly_micro_dollars"], 2_000_000);
    assert_eq!(resumed.body["funding_model"], "shared_pot");
    assert_eq!(resumed.body["room_codes"], false);
    assert_eq!(resumed.body["default_co_host"]["export_campaign"], true);
    let events_before = site.world.events_of_draft(&draft).await;
    assert_eq!(events_before, 0, "a draft has no events");
    let operation = operation();
    let finish = |operation: String| {
        let site = &site;
        let draft = draft.clone();
        async move {
            site.post(
                &format!("/campaigns/drafts/{draft}/finish"),
                owner,
                &json!({"operation": operation}),
            )
            .await
        }
    };
    let finished = finish(operation.clone()).await;
    assert_eq!(finished.status, StatusCode::OK, "{}", finished.text);
    assert_eq!(finished.body["campaign_id"], draft.as_str());
    // The campaign is active, the creator is its host, and `joined` is event number 1.
    let me = get(&site, &format!("/campaigns/{draft}/me"), owner).await;
    assert_eq!(me.body["role"], "host");
    assert_eq!(me.body["event_seq"], 1);
    // The chosen values survived the flip, including the sponsor rule for a shared pot.
    let (status, sponsor, hourly, step): (String, bool, i64, bool) =
        site.world.draft_columns(&draft).await;
    assert_eq!(status, "active");
    assert!(!sponsor, "a shared pot names no sponsor");
    assert_eq!(hourly, 2_000_000);
    assert!(step, "a campaign has no draft step (the column is null)");
    // Repeating the operation returns the same campaign; a different operation finds no draft.
    let repeat = finish(operation).await;
    assert_eq!(repeat.status, StatusCode::OK);
    assert_eq!(repeat.body["campaign_id"], draft.as_str());
    let other = finish(self::operation()).await;
    assert_eq!(other.status, StatusCode::NOT_FOUND);
    assert_eq!(site.world.events_of_draft(&draft).await, 1);
    site.cleanup().await;
}

#[tokio::test]
async fn a_named_sponsor_is_the_creator_and_steps_cannot_be_skipped_but_can_be_revisited() {
    let site = Site::new("wizardorder").await;
    let owner = site.person(1).await;
    let (draft, _) = start(&site, owner).await;
    // Step 3 before steps 1 and 2: refused.
    let skipped = save(&site, owner, &draft, 3, &table_step()).await;
    assert_eq!(skipped.status, StatusCode::BAD_REQUEST);
    assert_eq!(skipped.body["reason"], "invalid_input");
    // Finishing early says what is missing.
    let early = site
        .post(
            &format!("/campaigns/drafts/{draft}/finish"),
            owner,
            &json!({"operation": operation()}),
        )
        .await;
    assert_eq!(early.status, StatusCode::CONFLICT);
    assert_eq!(early.body["reason"], "draft_incomplete");
    save_all(&site, owner, &draft).await;
    let named = json!({
        "default_co_host": {"invite_players": false, "start_or_pause_session": false, "export_campaign": false},
        "room_codes": true,
        "funding_model": "named_sponsor"
    });
    // Going back changes that step and never moves the draft back.
    let back = save(&site, owner, &draft, 4, &named).await;
    assert_eq!(back.body["step"], 5);
    let redo = save(&site, owner, &draft, 1, &about("Renamed", 3)).await;
    assert_eq!(redo.body["step"], 5);
    assert_eq!(redo.body["name"], "Renamed");
    assert_eq!(redo.body["room_codes"], true);
    assert_eq!(site.world.draft_has_sponsor(&draft).await, Some(true));
    site.cleanup().await;
}

#[tokio::test]
async fn invalid_steps_are_refused_and_change_nothing() {
    let site = Site::new("wizardinvalid").await;
    let owner = site.person(1).await;
    let (draft, _) = start(&site, owner).await;
    for body in [
        about("   ", 4),
        about(&"x".repeat(81), 4),
        about("Fine", 0),
        about("Fine", 99),
        json!({"name": "Fine", "adventure_kind": "homebrew", "content_rating": "standard", "seat_count": 4}),
    ] {
        let reply = save(&site, owner, &draft, 1, &body).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{body}");
    }
    let unchanged = get(&site, &format!("/campaigns/drafts/{draft}"), owner).await;
    assert_eq!(unchanged.body["name"], "Untitled campaign");
    assert_eq!(unchanged.body["step"], 1);
    save(&site, owner, &draft, 1, &about("Fine", 4)).await;
    save(&site, owner, &draft, 2, &json!({"hero": "choose_in_lobby"})).await;
    // A custom preset without both figures, and a named preset with figures, are refused.
    for body in [
        json!({"voice_default": "on", "picture_default": "existing_art", "spend_preset": "custom", "custom_hourly_micro_dollars": null, "session_ceiling_micro_dollars": null}),
        json!({"voice_default": "on", "picture_default": "existing_art", "spend_preset": "story", "custom_hourly_micro_dollars": 5, "session_ceiling_micro_dollars": 9}),
    ] {
        let reply = save(&site, owner, &draft, 3, &body).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{body}");
    }
    // A step number outside 1 to 5, a malformed draft id and a malformed body are as expected.
    assert_eq!(
        save(&site, owner, &draft, 6, &json!({})).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&site, "/campaigns/drafts/not-an-id", owner)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        save(&site, owner, &draft, 1, &json!({"name": 5}))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    site.cleanup().await;
}

#[tokio::test]
async fn only_the_owner_reaches_a_draft_and_everyone_else_sees_it_as_missing() {
    let site = Site::new("wizardowner").await;
    let owner = site.person(1).await;
    let other = site.person(2).await;
    let (draft, _) = start(&site, owner).await;
    save_all(&site, owner, &draft).await;
    assert_eq!(
        get(&site, &format!("/campaigns/drafts/{draft}"), other)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        save(&site, other, &draft, 1, &about("Mine now", 4))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let finish = site
        .post(
            &format!("/campaigns/drafts/{draft}/finish"),
            other,
            &json!({"operation": operation()}),
        )
        .await;
    assert_eq!(finish.status, StatusCode::NOT_FOUND);
    // A draft is not a campaign: nobody is a member of it, not even its owner.
    let me = get(&site, &format!("/campaigns/{draft}/me"), owner).await;
    assert_eq!(me.status, StatusCode::NOT_FOUND);
    site.cleanup().await;
}

#[tokio::test]
async fn an_unverified_email_cannot_start_save_or_finish_a_draft() {
    let site = Site::new("wizardunverified").await;
    let owner = site.person(1).await;
    let (draft, _) = start(&site, owner).await;
    let unverified = site.world.account(2).await;
    site.accounts.add(
        unverified,
        "Not Yet",
        false,
        Some(tenant_id()),
        AccountStatus::Active,
    );
    let (_, refused) = start(&site, unverified).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.body["reason"], "email_unverified");
    let saved = save(&site, unverified, &draft, 1, &about("Nope", 4)).await;
    assert_ne!(saved.status, StatusCode::OK);
    let created = site
        .post(
            "/campaigns",
            unverified,
            &crate::routes::create_body("Quick"),
        )
        .await;
    assert_eq!(created.status, StatusCode::FORBIDDEN);
    assert_eq!(created.body["reason"], "email_unverified");
    site.cleanup().await;
}

#[tokio::test]
async fn drafts_are_capped_per_account_and_expired_ones_do_not_count() {
    let tunables = Tunables {
        drafts_per_account: 2,
        ..Tunables::default()
    };
    let site = Site::with_tunables("wizardcap", tunables).await;
    let owner = site.person(1).await;
    let (first, _) = start(&site, owner).await;
    let (_, second) = start(&site, owner).await;
    assert_eq!(second.status, StatusCode::OK);
    let (_, third) = start(&site, owner).await;
    assert_eq!(third.status, StatusCode::CONFLICT);
    assert_eq!(third.body["reason"], "drafts_full");
    // Thirty days untouched: the drafts are gone for every purpose, and the slots are free.
    site.world.clock.advance_ms(31 * 24 * 3_600_000);
    let expired = get(&site, &format!("/campaigns/drafts/{first}"), owner).await;
    assert_eq!(expired.status, StatusCode::NOT_FOUND);
    let (_, again) = start(&site, owner).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    site.cleanup().await;
}

#[tokio::test]
async fn saving_a_step_keeps_the_draft_alive_for_another_thirty_days() {
    let site = Site::new("wizardrenew").await;
    let owner = site.person(1).await;
    let (draft, _) = start(&site, owner).await;
    site.world.clock.advance_ms(20 * 24 * 3_600_000);
    assert_eq!(
        save(&site, owner, &draft, 1, &about("Still here", 4))
            .await
            .status,
        StatusCode::OK
    );
    site.world.clock.advance_ms(20 * 24 * 3_600_000);
    let alive = get(&site, &format!("/campaigns/drafts/{draft}"), owner).await;
    assert_eq!(alive.status, StatusCode::OK, "{}", alive.text);
    assert_eq!(alive.body["name"], "Still here");
    site.cleanup().await;
}

#[tokio::test]
async fn starting_and_saving_share_one_hourly_limit_and_a_draft_is_not_a_campaign_for_the_cap() {
    let tunables = Tunables {
        draft_save_limit_per_account_per_hour: 3,
        campaigns_per_account: 1,
        ..Tunables::default()
    };
    let site = Site::with_tunables("wizardlimit", tunables).await;
    let owner = site.person(1).await;
    let (draft, _) = start(&site, owner).await;
    for seats in [3, 4] {
        assert_eq!(
            save(&site, owner, &draft, 1, &about("Limit", seats))
                .await
                .status,
            StatusCode::OK
        );
    }
    let throttled = save(&site, owner, &draft, 1, &about("Limit", 5)).await;
    assert_eq!(throttled.status, StatusCode::TOO_MANY_REQUESTS);
    // Reading is not limited, and a pending draft does not use the campaign allowance.
    assert_eq!(
        get(&site, &format!("/campaigns/drafts/{draft}"), owner)
            .await
            .status,
        StatusCode::OK
    );
    let quick = site
        .post("/campaigns", owner, &crate::routes::create_body("Quick"))
        .await;
    assert_eq!(quick.status, StatusCode::OK, "{}", quick.text);
    // Finishing the draft is now one campaign too many.
    save_all_unlimited(&site, owner, &draft).await;
    let finish = site
        .post(
            &format!("/campaigns/drafts/{draft}/finish"),
            owner,
            &json!({"operation": operation()}),
        )
        .await;
    assert_eq!(finish.status, StatusCode::CONFLICT);
    assert_eq!(finish.body["reason"], "too_many_campaigns");
    site.cleanup().await;
}

/// Fills the draft through the store, so the route limit under test is not spent on setup.
async fn save_all_unlimited(site: &Site, owner: df_types::AccountId, draft: &str) {
    use df_campaign::{
        draft::{
            DraftStepData, FundingModel, HeroChoice, PictureDefault, SpendPreset, VoiceDefault,
        },
        records::{ContentRating, RoomCodes},
        roles::Delegations,
        store::CampaignStore,
    };
    let id = df_types::CampaignId::from_hex(draft).unwrap();
    let now = site.world.now();
    for data in [
        DraftStepData::About {
            name: "Full".into(),
            adventure_kind: "bundled".into(),
            content_rating: ContentRating::Standard,
            seat_count: 4,
        },
        DraftStepData::Hero(HeroChoice::ChooseInLobby),
        DraftStepData::Table {
            voice_default: VoiceDefault::On,
            picture_default: PictureDefault::ExistingArt,
            spend_preset: SpendPreset::Story,
            custom_hourly_micro_dollars: None,
            session_ceiling_micro_dollars: None,
        },
        DraftStepData::Joining {
            default_co_host_delegations: Delegations::NONE,
            room_codes: RoomCodes::Off,
            funding_model: FundingModel::NamedSponsor,
        },
    ] {
        site.world
            .store
            .save_draft_step(owner, id, data, now)
            .await
            .unwrap();
    }
}
