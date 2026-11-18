//! Tests of the world bible in the host: the fresh read of `world/bible.json`, the active place,
//! the context the persona prompts receive, the routes and the verbs.
//!
//! TODO(DEV-12): a concurrent edit of `bible.json` while a persona line is being worded, and a
//! `world place` race against `context_for`.

use serde_json::{Value, json};

use super::cli::{WorldCommand, parse};
use super::request::{PlaceSet, Quoted, Validated, WorldReply, WorldView};
use super::routes::path;
use super::{
    PlaceItem, available_packs, check_quote, context_for, context_for_viewer, place_items,
};
use crate::command::CommandError;
use crate::dev_client::{get_json, post_json};
use crate::dev_http::DevReply;
use crate::test_support::{Running, STARTER_FILES, start};

const SHIPPED: &str = include_str!("../../../../assets/playground/world/bible.json");

fn bible(price: u64) -> String {
    json!({
        "items": [
            {"id": "apple", "name": "apple", "unit": "each", "base_value_cp": price},
            {"id": "pear", "name": "pear", "unit": "each", "base_value_cp": 4}
        ],
        "places": [{
            "id": "market", "name": "Market", "description": "Stalls.",
            "stalls": [{
                "keeper": "fruit_vendor", "sells": ["apple", "pear"],
                "bulk": [{"min_qty": 5, "discount_permille": 100, "posted": true}],
                "haggle_floor_permille": 850
            }]
        }],
        "voice_packs": [
            {"id": "market-trader", "description": "brisk", "samples": ["Apples, dear?"]},
            {"id": "road-weary-guard", "description": "terse"}
        ]
    })
    .to_string()
}

async fn host() -> Running {
    start(&STARTER_FILES).await
}

fn words(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_owned).collect()
}

fn done<T: std::fmt::Debug>(reply: WorldReply<T>) -> T {
    match reply {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    }
}

fn refusal<T: std::fmt::Debug>(reply: WorldReply<T>) -> &'static str {
    match reply {
        DevReply::Done { value } => panic!("done: {value:?}"),
        DevReply::Rejected { error, .. } => error.key(),
    }
}

async fn set_place(host: &Running, place: Option<&str>) -> WorldReply<PlaceSet> {
    post_json(host.port, path::WORLD_PLACE, &json!({"place": place}))
        .await
        .unwrap()
}

fn lines(host: &Running) -> Vec<String> {
    host.state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("WORLD"))
        .collect()
}

fn pack_ids(packs: Vec<Value>) -> Vec<String> {
    packs
        .iter()
        .map(|pack| pack["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn the_shipped_bible_is_valid_and_keys_its_packs_by_community() {
    let bible = df_content::world_bible::WorldBible::from_json_str(SHIPPED).unwrap();
    let packs: Vec<&str> = bible
        .voice_packs
        .iter()
        .map(|pack| pack.id.as_str())
        .collect();
    assert_eq!(
        packs,
        ["market-trader", "road-weary-guard", "scholarly-traveller"]
    );
    let quote = bible.quote_at("market", "apple", 5).unwrap();
    assert_eq!(
        (
            quote.ask_total_cp,
            quote.tier_total_cp,
            quote.floor_total_cp
        ),
        (15, 14, 13)
    );
    assert_eq!(
        bible.place("market").unwrap().stalls[0].keeper,
        "fruit_vendor"
    );
}

#[tokio::test]
async fn a_missing_bible_is_an_empty_one() {
    let host = host().await;
    assert!(available_packs(&host.state).is_empty());
    let context = context_for(&host.state, &["market-trader".to_owned()]);
    assert_eq!(context, super::WorldContext::empty());
    let counts: Validated = done(get_json(host.port, path::WORLD_VALIDATE).await.unwrap());
    assert_eq!((counts.items, counts.places, counts.voice_packs), (0, 0, 0));
    assert!(lines(&host).is_empty());
}

#[tokio::test]
async fn the_bible_is_read_fresh_on_every_call() {
    let host = host().await;
    host.write_asset("world/bible.json", &bible(3));
    assert_eq!(
        pack_ids(available_packs(&host.state)),
        ["market-trader", "road-weary-guard"]
    );
    host.write_asset(
        "world/bible.json",
        &bible(3).replace("road-weary-guard", "dockhand"),
    );
    assert_eq!(
        pack_ids(available_packs(&host.state)),
        ["market-trader", "dockhand"]
    );
}

#[tokio::test]
async fn a_malformed_bible_is_a_typed_error_and_the_context_stays_empty() {
    let host = host().await;
    host.write_asset("world/bible.json", "{ not json");
    let reply: WorldReply<Validated> = get_json(host.port, path::WORLD_VALIDATE).await.unwrap();
    assert_eq!(refusal(reply), "bible_invalid");
    assert_eq!(context_for(&host.state, &[]), super::WorldContext::empty());
    assert!(available_packs(&host.state).is_empty());
    let lines = lines(&host);
    assert!(lines.contains(&"WORLD refused verb=validate error=bible_invalid".to_owned()));
    assert!(lines.contains(&"WORLD context_unavailable error=bible_invalid".to_owned()));

    host.write_asset("world/bible.json", r#"{"items": [{"id": "Apple"}]}"#);
    let reply: WorldReply<Validated> = get_json(host.port, path::WORLD_VALIDATE).await.unwrap();
    assert_eq!(refusal(reply), "bible_invalid");
}

#[tokio::test]
async fn the_context_holds_the_facts_of_the_active_place_and_the_named_packs() {
    let host = host().await;
    host.write_asset("world/bible.json", &bible(3));
    assert_eq!(context_for(&host.state, &[]).facts, Value::Null);

    done(set_place(&host, Some("market")).await);
    let context = context_for(
        &host.state,
        &["market-trader".to_owned(), "nope".to_owned()],
    );
    assert_eq!(context.voice_packs.len(), 1);
    assert_eq!(context.voice_packs[0]["id"], "market-trader");
    assert_eq!(context.voice_packs[0]["samples"], json!(["Apples, dear?"]));
    assert_eq!(context.facts["place"]["id"], "market");
    assert_eq!(context.facts["currency"], "cp");
    let stall = &context.facts["stalls"][0];
    assert_eq!(stall["keeper"], "fruit_vendor");
    let apple = &stall["items"][0];
    assert_eq!(apple["id"], "apple");
    let quoted: Vec<u64> = apple["quotes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|quote| quote["qty"].as_u64())
        .collect();
    assert_eq!(
        quoted,
        [1, 2, 3, 5, 10, 15, 20],
        "the common lots and the smallest lot of the one tier, once each, in order"
    );
    assert_eq!(lot(apple, 1)["ask_total_cp"], 3);
    assert_eq!(lot(apple, 5)["ask_total_cp"], 15);
    assert_eq!(lot(apple, 5)["tier_total_cp"], 14);
    assert_eq!(lot(apple, 5)["floor_total_cp"], 13);
    assert_eq!(lot(apple, 10)["tier_total_cp"], 27);
    assert!(
        apple["rule"]
            .as_str()
            .unwrap()
            .contains("from 5 on, 100 permille off"),
        "the keeper's price rule is built from the bible's numbers"
    );
    assert_eq!(context.facts["viewer_role"], "keeper");

    let packs = available_packs(&host.state);
    assert_eq!(
        packs[0],
        json!({"id": "market-trader", "description": "brisk"})
    );

    done(set_place(&host, None).await);
    assert_eq!(context_for(&host.state, &[]).facts, Value::Null);
}

#[tokio::test]
async fn an_edit_of_a_price_shows_in_the_next_context_without_a_restart() {
    let host = host().await;
    host.write_asset("world/bible.json", &bible(3));
    done(set_place(&host, Some("market")).await);
    let price = |host: &Running| {
        context_for(&host.state, &[]).facts["stalls"][0]["items"][0]["quotes"][0]["ask_total_cp"]
            .clone()
    };
    assert_eq!(price(&host), 3);
    host.write_asset("world/bible.json", &bible(9));
    assert_eq!(price(&host), 9);
}

#[tokio::test]
async fn the_active_place_must_be_in_the_bible_and_none_clears_it() {
    let host = host().await;
    host.write_asset("world/bible.json", &bible(3));
    assert_eq!(
        refusal(set_place(&host, Some("harbor")).await),
        "unknown_place"
    );
    assert_eq!(
        refusal(set_place(&host, Some("Market!")).await),
        "unknown_place"
    );
    let set = done(set_place(&host, Some("market")).await);
    assert_eq!(set.place.as_deref(), Some("market"));
    let state = std::fs::read_to_string(host.playground_path("assets/world/state.json")).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&state).unwrap(),
        json!({"place": "market"})
    );
    let view: WorldView = done(get_json(host.port, path::WORLD).await.unwrap());
    assert_eq!(view.place.as_deref(), Some("market"));
    assert_eq!(view.facts["place"]["name"], "Market");
    assert_eq!(done(set_place(&host, None).await).place, None);
    let view: WorldView = done(get_json(host.port, path::WORLD).await.unwrap());
    assert!(view.place.is_none() && view.facts.is_null());
    assert!(lines(&host).contains(&"WORLD place place=none".to_owned()));
}

#[tokio::test]
async fn a_place_the_bible_no_longer_holds_gives_no_facts() {
    let host = host().await;
    host.write_asset("world/bible.json", &bible(3));
    done(set_place(&host, Some("market")).await);
    host.write_asset(
        "world/bible.json",
        &bible(3).replace("\"market\"", "\"quay\""),
    );
    assert_eq!(context_for(&host.state, &[]).facts, Value::Null);
}

#[tokio::test]
async fn a_quote_is_priced_at_the_active_place() {
    let host = host().await;
    host.write_asset("world/bible.json", &bible(3));
    let quote = |item: &'static str, qty: u32| {
        let address = format!("{}?item={item}&qty={qty}", path::WORLD_QUOTE);
        let port = host.port;
        async move {
            get_json::<WorldReply<Quoted>>(port, &address)
                .await
                .unwrap()
        }
    };
    assert_eq!(refusal(quote("apple", 5).await), "no_active_place");
    done(set_place(&host, Some("market")).await);
    let quoted = done(quote("apple", 5).await);
    let priced = quoted.quote;
    assert_eq!(
        (
            priced.ask_total_cp,
            priced.tier_total_cp,
            priced.floor_total_cp
        ),
        (15, 14, 13)
    );
    assert_eq!(refusal(quote("plum", 1).await), "unknown_item");
    assert_eq!(refusal(quote("apple", 0).await), "zero_quantity");
    let bad: WorldReply<Quoted> = get_json(host.port, &format!("{}?item=apple", path::WORLD_QUOTE))
        .await
        .unwrap();
    assert_eq!(refusal(bad), "invalid_query");
}

const MARKET_STATE: &str = r#"{"place": "market"}"#;

async fn market_host(bible: &str) -> Running {
    let host = host().await;
    host.write_asset("world/bible.json", bible);
    host.write_asset("world/state.json", MARKET_STATE);
    host
}

#[tokio::test]
async fn the_keeper_sees_the_floor_and_a_customer_sees_only_the_posted_facts() {
    let host = market_host(&bible(3)).await;
    let keeper = context_for_viewer(&host.state, &[], "fruit_vendor").facts;
    assert_eq!(keeper["viewer_role"], "keeper");
    assert!(keeper.get("note").is_none());
    assert_eq!(keeper["stalls"][0]["haggle_floor_permille"], 850);
    assert_eq!(
        lot(&keeper["stalls"][0]["items"][0], 5)["floor_total_cp"],
        13
    );

    let customer = context_for_viewer(&host.state, &[], "mara").facts;
    assert_eq!(customer["audience"], "public");
    assert_eq!(customer["viewer_role"], "customer");
    assert_eq!(
        customer["note"],
        "Ask the keeper for a quote; only the keeper quotes a price."
    );
    let stall = &customer["stalls"][0];
    assert!(stall.get("haggle_floor_permille").is_none());
    let apple = &stall["items"][0];
    assert_eq!(
        (apple["name"].clone(), apple["unit"].clone()),
        (json!("apple"), json!("each"))
    );
    assert_eq!(apple["ask_each_cp"], 3);
    assert_eq!(apple["quotes"][0], json!({"qty": 1, "ask_total_cp": 3}));
    assert_eq!(
        apple["quotes"][1],
        json!({"qty": 5, "ask_total_cp": 15, "discount_permille": 100, "tier_total_cp": 14})
    );
    assert!(
        apple["quotes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|lot| lot.get("floor_total_cp").is_none())
    );
}

#[tokio::test]
async fn an_unposted_bulk_tier_is_hidden_from_a_customer_and_shown_to_the_keeper() {
    let mut unposted: Value = serde_json::from_str(&bible(3)).unwrap();
    unposted["places"][0]["stalls"][0]["bulk"][0]["posted"] = json!(false);
    let host = market_host(&unposted.to_string()).await;
    let customer = context_for_viewer(&host.state, &[], "mara").facts;
    let quotes = customer["stalls"][0]["items"][0]["quotes"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(quotes, [json!({"qty": 1, "ask_total_cp": 3})]);
    let keeper = context_for_viewer(&host.state, &[], "fruit_vendor").facts;
    assert_eq!(
        lot(&keeper["stalls"][0]["items"][0], 5)["tier_total_cp"],
        14
    );
    assert_eq!(
        context_for(&host.state, &[]).facts,
        keeper,
        "tools see keeper level"
    );
}

#[tokio::test]
async fn a_quote_check_clamps_a_proposal_into_the_floor_and_the_tier_total() {
    let host = market_host(&bible(3)).await;
    let check = |item, qty, proposed| check_quote(&host.state, "fruit_vendor", item, qty, proposed);
    let low = check("apple", 5, 1).unwrap();
    assert_eq!(
        (
            low.proposed_total_cp,
            low.clamped_total_cp,
            low.floor_total_cp,
            low.ask_total_cp
        ),
        (1, 13, 13, 14)
    );
    assert_eq!(check("apple", 5, 99).unwrap().clamped_total_cp, 14);
    assert_eq!(check("apple", 5, 14).unwrap().clamped_total_cp, 14);
    assert_eq!(check("apple", 5, 13).unwrap().clamped_total_cp, 13);
    assert_eq!(check("pear", 1, 4).unwrap().clamped_total_cp, 4);
    assert_eq!(check("plum", 5, 14), None, "the stall does not sell it");
    assert_eq!(check("apple", 0, 14), None);
    assert_eq!(
        check_quote(&host.state, "mara", "apple", 5, 14),
        None,
        "mara keeps no stall"
    );
}

#[test]
fn the_verbs_read_their_words() {
    assert_eq!(parse(&words("")), Ok(WorldCommand::Show));
    assert_eq!(parse(&words("show")), Ok(WorldCommand::Show));
    assert_eq!(parse(&words("validate")), Ok(WorldCommand::Validate));
    assert_eq!(
        parse(&words("place market")),
        Ok(WorldCommand::Place {
            place: Some("market".to_owned())
        })
    );
    assert_eq!(
        parse(&words("place -")),
        Ok(WorldCommand::Place { place: None })
    );
    assert_eq!(
        parse(&words("quote apple 5")),
        Ok(WorldCommand::Quote {
            item: "apple".to_owned(),
            qty: 5
        })
    );
    assert!(matches!(
        parse(&words("quote apple many")),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        parse(&words("place Market")),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        parse(&words("quote apple")),
        Err(CommandError::Usage(_))
    ));
    assert!(matches!(
        parse(&words("place")),
        Err(CommandError::Usage(_))
    ));
    assert!(matches!(
        parse(&words("sell apple")),
        Err(CommandError::Usage(_))
    ));
    assert_eq!(
        crate::command::parse(&words("world place market")),
        Ok(crate::command::Command::World(WorldCommand::Place {
            place: Some("market".to_owned())
        }))
    );
}

/// The quote of `item` for a lot of `qty`, found by its quantity, not its place in the list.
fn lot(item: &Value, qty: u64) -> &Value {
    item["quotes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|quote| quote["qty"] == qty)
        .unwrap()
}

#[tokio::test]
async fn a_designed_place_is_the_active_one_and_offers_its_things_without_a_keeper() {
    let host = host().await;
    host.write_asset("world/bible.json", &bible(3));
    let square = json!({"place": {
        "id": "roam-03",
        "name": "a village square",
        "description": "Mood: quiet.",
        "things": [{"id": "well", "name": "the old well", "look": "mossy stones"}]
    }});
    let arrive = |body: Value| {
        let port = host.port;
        async move {
            post_json::<_, WorldReply<PlaceSet>>(port, path::WORLD_ARRIVE, &body)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        done(arrive(square.clone()).await).place.as_deref(),
        Some("roam-03")
    );
    assert_eq!(
        place_items(&host.state),
        vec![PlaceItem {
            id: "well".to_owned(),
            name: "the old well".to_owned(),
            tags: Vec::new(),
            look: Some("mossy stones".to_owned()),
            keeper: None,
            picture: None,
            lying: None,
        }]
    );
    let facts = context_for(&host.state, &[]).facts;
    assert_eq!(facts["place"]["name"], "a village square");
    assert_eq!(facts["in_sight"][0]["look"], "mossy stones");
    assert!(facts.get("viewer_role").is_none() && facts.get("note").is_none());
    let view: WorldView = done(get_json(host.port, path::WORLD).await.unwrap());
    assert_eq!(view.place.as_deref(), Some("roam-03"));
    assert!(lines(&host).contains(&"WORLD arrived place=roam-03 things=1".to_owned()));

    let mut bad = square;
    bad["place"]["things"][0]["id"] = json!("Old Well");
    assert_eq!(refusal(arrive(bad).await), "place_invalid");
    assert_eq!(place_items(&host.state)[0].id, "well");

    done(set_place(&host, Some("market")).await);
    let sold: Vec<(String, Option<String>)> = place_items(&host.state)
        .into_iter()
        .map(|offered| (offered.id, offered.keeper))
        .collect();
    let keeper = Some("fruit_vendor".to_owned());
    assert_eq!(
        sold,
        vec![
            ("apple".to_owned(), keeper.clone()),
            ("pear".to_owned(), keeper)
        ]
    );
}

#[tokio::test]
async fn a_thing_keeps_its_first_close_up_as_its_sheet_at_a_designed_place() {
    let host = host().await;
    let keep = |body: Value| {
        let port = host.port;
        async move {
            post_json::<_, WorldReply<PlaceSet>>(port, path::WORLD_PICTURE, &body)
                .await
                .unwrap()
        }
    };
    let picture = json!({"item": "well", "picture": "roam-1-2.png"});
    assert_eq!(refusal(keep(picture.clone()).await), "not_designed");
    let square = json!({"place": {"id": "roam-03", "name": "a square",
        "things": [{"id": "well", "name": "the old well", "look": "mossy stones"}]}});
    let _: PlaceSet = done(
        post_json(host.port, path::WORLD_ARRIVE, &square)
            .await
            .unwrap(),
    );
    done(keep(picture).await);
    assert_eq!(
        place_items(&host.state)[0].picture.as_deref(),
        Some("roam-1-2.png")
    );
    assert_eq!(
        refusal(keep(json!({"item": "cart", "picture": "x.png"})).await),
        "unknown_thing"
    );
}

#[tokio::test]
async fn a_place_designed_in_play_is_gone_back_to_as_it_was_left_and_a_thing_can_be_added() {
    let host = host().await;
    let arrive = |body: Value| {
        let port = host.port;
        async move {
            let set: PlaceSet = done(post_json(port, path::WORLD_ARRIVE, &body).await.unwrap());
            set
        }
    };
    arrive(json!({"place": {"id": "roam-03", "name": "a square",
        "things": [{"id": "well", "name": "the old well", "look": "mossy stones"}]},
        "feature": "the old well"}))
    .await;
    // A thing the runner saw in the picture becomes a real thing of the place.
    let cart = json!({"id": "cart", "name": "a hay cart", "look": "a cart piled with hay"});
    let added: PlaceSet = done(
        post_json(host.port, path::WORLD_THING_ADD, &cart)
            .await
            .unwrap(),
    );
    assert_eq!(added.place.as_deref(), Some("roam-03"));
    arrive(json!({"place": {"id": "roam-04", "name": "a mill",
        "things": [{"id": "wheel", "name": "the wheel", "look": "a dripping wheel"}]},
        "feature": "the wheel"}))
    .await;
    let back = done(set_place(&host, Some("roam-03")).await);
    assert_eq!(back.place.as_deref(), Some("roam-03"));
    let ids: Vec<String> = place_items(&host.state)
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(ids, ["well", "cart"], "the place as it was left");
    // A thing added again under its id is that thing as it is now (what its sign reads): it
    // replaces the one kept, so it reads the same the next time.
    let read = json!({"id": "well", "name": "the old well",
                      "look": "mossy stones. Its plaque reads: Drink and be thankful"});
    done::<PlaceSet>(
        post_json(host.port, path::WORLD_THING_ADD, &read)
            .await
            .unwrap(),
    );
    let items = place_items(&host.state);
    assert_eq!(items.len(), 2, "no second well");
    assert_eq!(
        items[0].look.as_deref(),
        Some("mossy stones. Its plaque reads: Drink and be thankful")
    );
    let directory = super::store::world_directory(&host.state);
    assert_eq!(
        super::store::read_visited(&directory).unwrap().len(),
        2,
        "the places visited stay"
    );
    assert_eq!(
        refusal(set_place(&host, Some("roam-99")).await),
        "unknown_place"
    );
}

/// Moves the paused clock on by `ms` and lets the fading task finish.
async fn pass(ms: u64) {
    tokio::time::advance(std::time::Duration::from_millis(ms)).await;
    for _ in 0..200 {
        tokio::task::yield_now().await;
        std::thread::yield_now();
    }
}

#[tokio::test(start_paused = true)]
async fn the_party_strip_shows_at_a_new_place_and_fades_out_fifteen_seconds_later() {
    let host = host().await;
    host.write_asset("world/bible.json", &bible(3));
    host.write_asset(
        "fixtures/one.json",
        concat!(
            r#"{"name": "One", "tv": {"place": "Here", "status": {"kind": "ok"}, "#,
            r#""elements": []}, "#,
            r#""phone": {"hero": "Pip", "status": {"kind": "ok"}}}"#
        ),
    );
    let hidden = |host: &Running| {
        host.state
            .playground()
            .snapshot()
            .scene
            .party_hidden
            .set_value()
            .copied()
    };
    let strip_lines = |host: &Running| {
        host.state
            .log()
            .after_sequence(0, usize::MAX)
            .into_iter()
            .map(|line| line.text)
            .filter(|text| text.starts_with("PLAYGROUND party_strip "))
            .collect::<Vec<_>>()
    };
    let shown_ms = super::routes::PARTY_STRIP_SHOWN.as_millis() as u64;
    done(set_place(&host, Some("market")).await);
    assert_eq!(hidden(&host), Some(false));
    pass(shown_ms - 1_000).await;
    assert_eq!(hidden(&host), Some(false), "still up a second before");
    // A new place before it fades shows it again and starts the wait anew.
    done(set_place(&host, Some("market")).await);
    pass(shown_ms - 1_000).await;
    assert_eq!(hidden(&host), Some(false));
    pass(1_000).await;
    assert_eq!(hidden(&host), Some(true));
    let lines = strip_lines(&host);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[0].starts_with("PLAYGROUND party_strip shown place=market at_ms="));
    assert!(lines[2].starts_with("PLAYGROUND party_strip hidden at_ms="));
    // Nothing but a new place brings it back.
    pass(shown_ms * 2).await;
    assert_eq!(hidden(&host), Some(true));
    assert_eq!(strip_lines(&host).len(), 3);
}

#[test]
fn world_arrive_reads_a_place_with_no_things_yet() {
    let words = |text: &str| text.split(' ').map(str::to_owned).collect::<Vec<_>>();
    let Ok(WorldCommand::Arrive(request)) = parse(&words(
        "arrive roam-00-hero-11 the road before the town gate | a walled town's open gate",
    )) else {
        panic!("world arrive reads");
    };
    assert_eq!(request.place.id, "roam-00-hero-11");
    assert_eq!(request.place.name, "the road before the town gate");
    assert_eq!(
        request.place.description,
        "In sight: a walled town's open gate."
    );
    assert!(request.place.things.is_empty() && request.shops.is_empty());
    for bad in ["arrive Road x | y", "arrive road x", "arrive road | y"] {
        assert!(parse(&words(bad)).is_err(), "{bad}");
    }
}

#[tokio::test]
async fn a_thing_dropped_from_the_bag_lies_in_sight_at_that_place_only() {
    let host = host().await;
    let _: Value = post_json(
        host.port,
        super::inventory::routes::path::INVENTORY_SET,
        &json!({"owner": "hero-11", "kind": "character", "items": [
            {"id": "dagger", "name": "Dagger", "look": "a plain dagger", "use": "kept",
             "count": 2}
        ]}),
    )
    .await
    .unwrap();
    let field = |id: &str| {
        json!({"place": {"id": id, "name": "a field",
            "things": [{"id": "old-oak", "name": "the old oak", "look": "a broad oak"}]}})
    };
    let arrive = |body: Value| {
        let port = host.port;
        async move {
            post_json::<_, WorldReply<PlaceSet>>(port, path::WORLD_ARRIVE, &body)
                .await
                .unwrap()
        }
    };
    done(arrive(field("field-1")).await);
    let applied = super::drop_own(&host.state, "hero-11", "dagger", 1).unwrap();
    assert_eq!(applied[0].after, 1);
    let lying = PlaceItem {
        id: "dagger".to_owned(),
        name: "Dagger".to_owned(),
        tags: Vec::new(),
        look: Some("a plain dagger. It lies on the ground here.".to_owned()),
        keeper: None,
        picture: None,
        lying: Some(1),
    };
    assert_eq!(place_items(&host.state)[1], lying);
    assert!(
        host.state
            .log()
            .after_sequence(0, usize::MAX)
            .iter()
            .any(|line| line.text == "INVENTORY left owner=hero-11 item=dagger qty=1 at=field-1")
    );
    done(arrive(field("field-2")).await);
    assert_eq!(place_items(&host.state).len(), 1);
    done(arrive(field("field-1")).await);
    assert_eq!(place_items(&host.state)[1], lying);
}

#[tokio::test]
async fn a_note_on_a_thing_is_told_after_its_look_and_replaces_only_its_own_kind() {
    let host = host().await;
    let gate = json!({"place": {"id": "road-1", "name": "the road before the gate",
        "things": [{"id": "gate", "name": "open timber gate", "look": "weathered timber"}]}});
    let _: WorldReply<PlaceSet> = post_json(host.port, path::WORLD_ARRIVE, &gate)
        .await
        .unwrap();
    let note = |label: &str, text: &str, name: Option<&str>| {
        let mut body = json!({"id": "gate", "label": label});
        body["text"] = json!(text);
        body["name"] = json!(name);
        body
    };
    for body in [
        note("state", "It stands open now.", None),
        note("read", "Read: \"Keep it shut\".", None),
        note("state", "It stands closed now.", Some("timber gate")),
    ] {
        let _: WorldReply<PlaceSet> = post_json(host.port, path::WORLD_THING_NOTE, &body)
            .await
            .unwrap();
    }
    let gate = &place_items(&host.state)[0];
    assert_eq!(gate.name, "timber gate");
    assert_eq!(
        gate.look.as_deref(),
        Some("weathered timber Read: \"Keep it shut\". It stands closed now.")
    );
    let unknown = note("state", "It stands open now.", None);
    let mut unknown = unknown;
    unknown["id"] = json!("well");
    assert_eq!(
        refusal(
            post_json::<_, WorldReply<PlaceSet>>(host.port, path::WORLD_THING_NOTE, &unknown)
                .await
                .unwrap()
        ),
        "unknown_thing"
    );
}
