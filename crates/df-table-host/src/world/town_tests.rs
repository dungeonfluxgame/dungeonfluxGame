//! Tests of the town the party is in and of the stage notes of a walk: the record kept in the
//! world state, how a place designed in play and a place of the bible treat it, its route and
//! verb, and the audit line of each stage.

use df_content::world_bible::Place;
use serde_json::json;

use super::cli::{WorldCommand, parse};
use super::request::{ArriveRequest, PlaceSet, WorldReply};
use super::routes::path;
use super::{Town, TownView, WalkNoted, end_town, seed_town, town_of, visited_of};
use crate::dev_client::{get_json, post_json};
use crate::dev_http::DevReply;
use crate::test_support::{Running, STARTER_FILES, start};

fn town(id: &str, look: &str) -> Town {
    Town {
        id: id.to_owned(),
        look: look.to_owned(),
        created_at: "roam-01".to_owned(),
    }
}

fn lines(host: &Running, start: &str) -> Vec<String> {
    host.state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with(start))
        .collect()
}

async fn town_on_the_route(host: &Running) -> Option<Town> {
    let reply: WorldReply<TownView> = get_json(host.port, path::WORLD_TOWN).await.unwrap();
    match reply {
        DevReply::Done { value } => value.town,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    }
}

async fn arrive(host: &Running, id: &str) {
    let request = ArriveRequest {
        place: Place {
            id: id.to_owned(),
            name: "a market square".to_owned(),
            description: "Mood: busy.".to_owned(),
            stalls: Vec::new(),
            things: Vec::new(),
        },
        feature: None,
    };
    let reply: WorldReply<PlaceSet> = post_json(host.port, path::WORLD_ARRIVE, &request)
        .await
        .unwrap();
    assert!(matches!(reply, DevReply::Done { .. }), "arrived");
}

#[tokio::test]
async fn a_host_that_has_not_seeded_a_town_is_in_none() {
    let host = start(&STARTER_FILES).await;

    assert_eq!(town_of(&host.state), None);
    assert_eq!(town_on_the_route(&host).await, None);
    assert_eq!(end_town(&host.state).await.unwrap(), None);
    assert!(lines(&host, "WORLD town_").is_empty());
}

#[tokio::test]
async fn a_seeded_town_is_kept_while_the_party_goes_from_place_to_place_in_it() {
    let host = start(&STARTER_FILES).await;
    let seeded = town("town-roam-01", "red tile roofs and cobbled lanes");

    seed_town(&host.state, seeded.clone()).await.unwrap();
    arrive(&host, "roam-01").await;
    arrive(&host, "roam-02").await;

    assert_eq!(town_of(&host.state), Some(seeded.clone()));
    assert_eq!(town_on_the_route(&host).await, Some(seeded));
    assert_eq!(
        lines(&host, "WORLD town_"),
        ["WORLD town_seeded id=town-roam-01 created_at=roam-01"]
    );
}

#[tokio::test]
async fn leaving_ends_the_town_and_the_next_one_is_a_new_record() {
    let host = start(&STARTER_FILES).await;
    seed_town(&host.state, town("town-roam-01", "red roofs"))
        .await
        .unwrap();

    let left = end_town(&host.state).await.unwrap();
    seed_town(&host.state, town("town-roam-04", "grey slate roofs"))
        .await
        .unwrap();

    assert_eq!(left.unwrap().id, "town-roam-01");
    assert_eq!(town_of(&host.state).unwrap().look, "grey slate roofs");
    assert_eq!(
        lines(&host, "WORLD town_"),
        [
            "WORLD town_seeded id=town-roam-01 created_at=roam-01",
            "WORLD town_left id=town-roam-01",
            "WORLD town_seeded id=town-roam-04 created_at=roam-01"
        ]
    );
}

#[tokio::test]
async fn setting_the_place_to_nowhere_ends_the_town() {
    let host = start(&STARTER_FILES).await;
    seed_town(&host.state, town("town-roam-01", "red roofs"))
        .await
        .unwrap();

    let nowhere: WorldReply<PlaceSet> =
        post_json(host.port, path::WORLD_PLACE, &json!({"place": null}))
            .await
            .unwrap();

    assert!(matches!(nowhere, DevReply::Done { .. }));
    assert_eq!(town_of(&host.state), None);
}

#[test]
fn the_town_verb_takes_no_words() {
    let words =
        |line: &str| -> Vec<String> { line.split_whitespace().map(str::to_owned).collect() };

    assert_eq!(parse(&words("town")).unwrap(), WorldCommand::Town);
    assert!(parse(&words("town now")).is_err());
}

async fn note(host: &Running, body: serde_json::Value) -> WorldReply<WalkNoted> {
    post_json(host.port, path::WORLD_WALK, &body).await.unwrap()
}

#[tokio::test]
async fn a_stage_note_is_one_audit_line_with_the_playground_clock() {
    let host = start(&STARTER_FILES).await;

    let shown = note(
        &host,
        json!({"place": "roam-01", "stage": "fountain_square", "number": 1, "of": 2,
               "outcome": "shown", "up_ms": 6020}),
    )
    .await;
    let late = note(
        &host,
        json!({"place": "roam-01", "stage": "shopfronts", "number": 2, "of": 2,
               "outcome": "late"}),
    )
    .await;

    let (DevReply::Done { value: first }, DevReply::Done { value: second }) = (shown, late) else {
        panic!("both notes are taken");
    };
    let written = lines(&host, "WORLD walk_stage");
    assert_eq!(
        written[0],
        format!(
            "WORLD walk_stage place=roam-01 stage=fountain_square number=1 of=2 outcome=shown \
             at_ms={} up_ms=6020",
            first.at_ms
        )
    );
    assert_eq!(
        written[1],
        format!(
            "WORLD walk_stage place=roam-01 stage=shopfronts number=2 of=2 outcome=late at_ms={} \
             up_ms=-",
            second.at_ms
        )
    );
}

#[tokio::test]
async fn a_stage_note_with_no_id_or_an_outcome_outside_the_closed_words_is_refused() {
    let host = start(&STARTER_FILES).await;

    let no_id = note(
        &host,
        json!({"place": "Roam 01", "stage": "x", "number": 1, "of": 1, "outcome": "shown"}),
    )
    .await;
    let odd = note(
        &host,
        json!({"place": "roam-01", "stage": "x", "number": 1, "of": 1, "outcome": "dawdled"}),
    )
    .await;

    let keys: Vec<&str> = [no_id, odd]
        .iter()
        .map(|reply| match reply {
            DevReply::Rejected { error, .. } => error.key(),
            DevReply::Done { .. } => "done",
        })
        .collect();
    assert_eq!(keys, ["invalid_walk_stage", "invalid_body"]);
    assert!(lines(&host, "WORLD walk_stage").is_empty());
    assert_eq!(
        lines(&host, "WORLD refused verb=walk"),
        [
            "WORLD refused verb=walk error=invalid_walk_stage",
            "WORLD refused verb=walk error=invalid_body"
        ]
    );
}

#[tokio::test]
async fn the_features_of_the_places_arrived_at_are_kept_until_a_bible_place_is_made_active() {
    let host = start(&STARTER_FILES).await;
    let arrive_with = |id: &'static str, feature: Option<&'static str>| {
        let port = host.port;
        async move {
            let request = ArriveRequest {
                place: Place {
                    id: id.to_owned(),
                    name: format!("the place {id}"),
                    description: "Mood: quiet.".to_owned(),
                    stalls: Vec::new(),
                    things: Vec::new(),
                },
                feature: feature.map(str::to_owned),
            };
            let reply: WorldReply<PlaceSet> =
                post_json(port, path::WORLD_ARRIVE, &request).await.unwrap();
            assert!(matches!(reply, DevReply::Done { .. }), "arrived");
        }
    };
    assert!(visited_of(&host.state).is_empty());

    arrive_with("roam-01", Some("open_road")).await;
    arrive_with("roam-02", None).await;
    arrive_with("roam-03", Some("mill")).await;
    let kept: Vec<(String, String)> = visited_of(&host.state)
        .into_iter()
        .map(|place| (place.name, place.feature))
        .collect();
    assert_eq!(
        kept,
        [
            ("the place roam-01".to_owned(), "open_road".to_owned()),
            ("the place roam-03".to_owned(), "mill".to_owned())
        ]
    );

    let nowhere: WorldReply<PlaceSet> =
        post_json(host.port, path::WORLD_PLACE, &json!({"place": null}))
            .await
            .unwrap();
    assert!(matches!(nowhere, DevReply::Done { .. }));
    assert!(visited_of(&host.state).is_empty());
}
