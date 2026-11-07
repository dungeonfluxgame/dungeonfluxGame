//! Tests of the world bible in the host: the fresh read of `world/bible.json`, the active place,
//! the context the persona prompts receive, the routes and the verbs.
//!
//! TODO(DEV-12): a concurrent edit of `bible.json` while a persona line is being worded, and a
//! `world place` race against `context_for`.

use serde_json::{Value, json};

use super::cli::{WorldCommand, parse};
use super::request::{PlaceSet, Quoted, Validated, WorldReply, WorldView};
use super::routes::path;
use super::{available_packs, context_for};
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
                "bulk": [{"min_qty": 5, "discount_permille": 100}],
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
    let packs: Vec<&str> = bible.voice_packs.iter().map(|pack| pack.id.as_str()).collect();
    assert_eq!(packs, ["market-trader", "road-weary-guard", "scholarly-traveller"]);
    let quote = bible.quote_at("market", "apple", 5).unwrap();
    assert_eq!((quote.total_cp, quote.floor_total_cp), (15, 13));
    assert_eq!(bible.place("market").unwrap().stalls[0].keeper, "fruit_vendor");
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
    let context = context_for(&host.state, &["market-trader".to_owned(), "nope".to_owned()]);
    assert_eq!(context.voice_packs.len(), 1);
    assert_eq!(context.voice_packs[0]["id"], "market-trader");
    assert_eq!(context.voice_packs[0]["samples"], json!(["Apples, dear?"]));
    assert_eq!(context.facts["place"]["id"], "market");
    assert_eq!(context.facts["currency"], "cp");
    let stall = &context.facts["stalls"][0];
    assert_eq!(stall["keeper"], "fruit_vendor");
    let apple = &stall["items"][0];
    assert_eq!(apple["id"], "apple");
    let quotes = apple["quotes"].as_array().unwrap();
    assert_eq!(quotes.len(), 2, "one unit, and the smallest lot of the one tier");
    assert_eq!(quotes[0]["qty"], 1);
    assert_eq!(quotes[0]["total_cp"], 3);
    assert_eq!(quotes[1]["qty"], 5);
    assert_eq!(quotes[1]["total_cp"], 15);
    assert_eq!(quotes[1]["floor_total_cp"], 13);

    let packs = available_packs(&host.state);
    assert_eq!(packs[0], json!({"id": "market-trader", "description": "brisk"}));

    done(set_place(&host, None).await);
    assert_eq!(context_for(&host.state, &[]).facts, Value::Null);
}

#[tokio::test]
async fn an_edit_of_a_price_shows_in_the_next_context_without_a_restart() {
    let host = host().await;
    host.write_asset("world/bible.json", &bible(3));
    done(set_place(&host, Some("market")).await);
    let price = |host: &Running| {
        context_for(&host.state, &[]).facts["stalls"][0]["items"][0]["quotes"][0]["total_cp"]
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
    assert_eq!(refusal(set_place(&host, Some("harbor")).await), "unknown_place");
    assert_eq!(refusal(set_place(&host, Some("Market!")).await), "unknown_place");
    let set = done(set_place(&host, Some("market")).await);
    assert_eq!(set.place.as_deref(), Some("market"));
    let state = std::fs::read_to_string(host.playground_path("assets/world/state.json")).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&state).unwrap(), json!({"place": "market"}));
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
        async move { get_json::<WorldReply<Quoted>>(port, &address).await.unwrap() }
    };
    assert_eq!(refusal(quote("apple", 5).await), "no_active_place");
    done(set_place(&host, Some("market")).await);
    let quoted = done(quote("apple", 5).await);
    assert_eq!((quoted.quote.bulk_each_cp, quoted.quote.total_cp), (3, 15));
    assert_eq!(quoted.quote.floor_total_cp, 13);
    assert_eq!(refusal(quote("plum", 1).await), "unknown_item");
    assert_eq!(refusal(quote("apple", 0).await), "zero_quantity");
    let bad: WorldReply<Quoted> = get_json(host.port, &format!("{}?item=apple", path::WORLD_QUOTE))
        .await
        .unwrap();
    assert_eq!(refusal(bad), "invalid_query");
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
    assert_eq!(parse(&words("place -")), Ok(WorldCommand::Place { place: None }));
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
    assert!(matches!(parse(&words("quote apple")), Err(CommandError::Usage(_))));
    assert!(matches!(parse(&words("place")), Err(CommandError::Usage(_))));
    assert!(matches!(parse(&words("sell apple")), Err(CommandError::Usage(_))));
    assert_eq!(
        crate::command::parse(&words("world place market")),
        Ok(crate::command::Command::World(WorldCommand::Place {
            place: Some("market".to_owned())
        }))
    );
}
