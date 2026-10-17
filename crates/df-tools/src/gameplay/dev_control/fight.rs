//! `dev fight`: whether a scene fight is open, every scene person's presence, the roster and
//! stances the latest SCENE-FIGHT audit line recorded, and the open encounter's combat view. Read
//! only. Restoring a downed or fled person and ending a fight are staged decisions this surface
//! does not add: they answer unavailable.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use df_model::checkpoint::GameState;
use serde_json::{Value, json};
use std::sync::Arc;

use super::{Outcome, Shared, authorized, failed, timeline, write_unavailable};
use crate::gameplay::{
    beat_script_runtime,
    journey::scene_fight::{self, Presence},
    pack::Scene,
};

fn presence_label(presence: Presence) -> &'static str {
    match presence {
        Presence::Standing => "standing",
        Presence::Down { dead: false } => "down",
        Presence::Down { dead: true } => "dead",
        Presence::Gone => "gone",
    }
}

/// The quoted value of ` key="..."` in an audit line.
fn quoted(line: &str, key: &str) -> Option<String> {
    let needle = format!(" {key}=\"");
    let rest = &line[line.find(&needle)? + needle.len()..];
    Some(rest[..rest.find('"')?].to_owned())
}

/// The comma-separated items of ` key=[...]` in an audit line; an empty list when absent.
fn bracketed(line: &str, key: &str) -> Vec<String> {
    let needle = format!(" {key}=[");
    let Some(index) = line.find(&needle) else {
        return Vec::new();
    };
    let rest = &line[index + needle.len()..];
    let Some(end) = rest.find(']') else {
        return Vec::new();
    };
    rest[..end]
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The roster a `SCENE-FIGHT aggressor=…` audit line records. Lines that name no aggressor (the
/// unavailable form) are not rosters and yield `None`.
pub(super) fn roster_of(line: &str) -> Option<Value> {
    Some(json!({
        "aggressor": quoted(line, "aggressor")?,
        "target": quoted(line, "target")?,
        "join": bracketed(line, "join"),
        "flee": bracketed(line, "flee"),
        "out": bracketed(line, "out"),
    }))
}

/// The newest roster the server recorded, with the audit sequence it came from.
fn latest_roster() -> Value {
    let audit = timeline::audit(1000, Some("SCENE-FIGHT"));
    audit["events"]
        .as_array()
        .and_then(|events| {
            events.iter().rev().find_map(|event| {
                let mut roster = roster_of(event["line"].as_str()?)?;
                roster["sequence"] = event["sequence"].clone();
                Some(roster)
            })
        })
        .unwrap_or(Value::Null)
}

/// The fight document for one checkpoint. The combat view is shown only while a fight is open.
pub(super) fn fight_document(
    game: &GameState,
    scene: &Scene,
    combat: Value,
    roster: Value,
) -> Result<Value, (StatusCode, Json<Value>)> {
    let open = scene_fight::open(game);
    let mut people = Vec::new();
    for person in &scene.cast {
        let presence = scene_fight::presence(game, &person.id)
            .map_err(|_| failed("scene person presence unavailable"))?;
        people.push(json!({
            "id": person.id,
            "name": person.name,
            "presence": presence_label(presence),
        }));
    }
    Ok(json!({
        "open": open,
        "scene": scene.title,
        "people": people,
        "roster": roster,
        "combat": if open { combat } else { Value::Null },
    }))
}

/// `GET /dev/fight`: the current scene fight, its people and the latest recorded roster.
pub(super) async fn read(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    let checkpoint = state.service.updates.borrow().clone();
    let scene = beat_script_runtime::scene(checkpoint.state())
        .map_err(|_| failed("current scene unavailable"))?;
    let (_, display) = super::display_view(&state).await?;
    let combat = super::reads::display_journey(&display)
        .remove("combat")
        .unwrap_or(Value::Null);
    let document = fight_document(checkpoint.state(), &scene, combat, latest_roster())?;
    Ok((StatusCode::OK, Json(document)))
}

/// `POST /dev/fight/edit`: restore and end are not performed by this surface.
pub(super) async fn unsupported_edit(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Outcome {
    authorized(&headers, &state.token)?;
    write_unavailable(
        "fight",
        "restoring a downed or fled person and ending a scene fight are staged decisions through the owner's override path, which is disabled server-side until the milestone 3 projection-safety proof",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::{
        adventure::{RulesMode, record, shop},
        beat_script_variety_tests as native, pack,
        startup_configuration::ContentRating,
    };

    #[test]
    fn roster_is_read_from_the_announced_line_and_refused_for_the_unavailable_form() {
        let line = "SCENE-FIGHT aggressor=\"Brynn\" target=\"corin\" join=[jessa] flee=[marek] out=[] rules=\"tactical.cover-centre-line: cover is adjudicated along cell centres\"";
        let roster = roster_of(line).expect("an announced fight has a roster");
        assert_eq!(roster["aggressor"], "Brynn");
        assert_eq!(roster["target"], "corin");
        assert_eq!(roster["join"], json!(["jessa"]));
        assert_eq!(roster["flee"], json!(["marek"]));
        assert_eq!(roster["out"], json!([]));

        assert!(roster_of("SCENE-FIGHT unavailable reason=\"busy\"").is_none());
    }

    #[test]
    fn fight_read_reports_no_open_fight_and_every_scene_person_standing_before_one() {
        let generated = shop::generate(46, RulesMode::Standard, ContentRating::default()).unwrap();
        let documents = generated.documents(true).unwrap();
        let (loaded, _record) = record::reload(&documents).unwrap();
        pack::with_test_pack(loaded, || {
            let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
            let scene = beat_script_runtime::scene(opening.state()).unwrap();
            let shown =
                fight_document(opening.state(), &scene, json!({"round": 1}), Value::Null).unwrap();

            assert_eq!(shown["open"], false);
            assert!(shown["combat"].is_null(), "no combat view outside a fight");
            assert!(shown["roster"].is_null());
            let people = shown["people"].as_array().unwrap();
            assert_eq!(people.len(), scene.cast.len());
            assert!(people.iter().all(|person| person["presence"] == "standing"));
        });
    }
}
