//! The fixture contract (plan O.16 step K0): the playground's `Fixture` is a serialisation of the
//! real shared-screen and seat projections of the table.
//!
//! Each test builds a real `TableState` through its public effects and the `Attachments` its
//! owners publish (`projection_samples`), calls `df_table::projection::project` for the shared
//! screen and for a seat, serialises the result as the host will, and parses the two views as the
//! `tv` and the `phone` of a `df_playground::fixture::Fixture`. A change to either side fails
//! here: a key renamed or dropped in `df-table`'s projection, a status that changes shape, a field
//! added to the playground's `Fixture`, or a type the playground reads differently.
//!
//! `NOT_PROJECTED_YET` lists, with an owner, every field the playground can draw that the
//! projection does not fill. A new `Fixture` field is in neither place, so the accounting test
//! names it.

use std::collections::BTreeSet;

use df_playground::copy::STATUS_KEYS;
use df_playground::fixture::{ElementSource, Fixture, Status};
use df_table::projection::{Attachments, ProjectionStatus, StatusKind};
use df_table::table_state::TableSnapshot;
use serde_json::{Value, json};

use crate::projection_samples::{
    PIP_SEAT, VEX_SEAT, busy_table, empty_table, paused_table, projection_of, published, seat,
    shared, table_with_two_heroes,
};

/// The `tv` and `phone` JSON for a seat: the `view` of each real projection, as it serialises.
fn views(snapshot: &TableSnapshot, attachments: &Attachments, seat_number: u64) -> (Value, Value) {
    let view_of = |audience| {
        let projection = projection_of(snapshot, attachments, audience);
        serde_json::to_value(&projection).expect("a projection serialises")["view"].clone()
    };
    (view_of(shared()), view_of(seat(seat_number)))
}

/// The whole of what a host adds to the two views to make a fixture: a name.
fn compose(tv: Value, phone: Value) -> String {
    json!({ "name": "Projection", "tv": tv, "phone": phone }).to_string()
}

fn parse(text: &str) -> Fixture {
    Fixture::parse(text).unwrap_or_else(|error| {
        panic!(
            "the playground refused the projection at line {} column {}: {}",
            error.line, error.column, error.message
        )
    })
}

fn busy_fixture(seat_number: u64) -> Fixture {
    let (snapshot, attachments) = busy_table();
    let (tv, phone) = views(&snapshot, &attachments, seat_number);
    parse(&compose(tv, phone))
}

#[test]
fn the_real_shared_screen_and_seat_projections_parse_as_a_fixture_with_their_content() {
    let fixture = busy_fixture(PIP_SEAT);
    assert_eq!(fixture.tv.place, "The cellar stairs");
    assert_eq!(fixture.tv.status, Status::Telling);
    let caption = fixture.tv.caption.expect("the caption");
    assert_eq!(caption.speaker.as_deref(), Some("Pip"));
    assert_eq!(caption.text, "Pip swings at the goblin and it yelps.");
    assert_eq!(fixture.tv.party.len(), 2);
    assert_eq!(fixture.tv.party[0].name, "Pip");
    assert_eq!(fixture.tv.party[0].conditions, ["prone"]);
    // The hidden hero is in the party strip with a hidden chip, never left out (K.13).
    assert_eq!(fixture.tv.party[1].name, "Vex");
    assert_eq!(fixture.tv.party[1].conditions, ["hidden"]);
    let ElementSource::Inline(elements) = &fixture.tv.elements else {
        panic!("a projection lists its elements inline");
    };
    let ids: Vec<&str> = elements.iter().map(|element| element.id.as_str()).collect();
    // The stage lists the cast in element-id order.
    assert_eq!(ids, ["goblin", "pip", "vex"]);
    assert_eq!(
        elements
            .iter()
            .find(|element| element.id == "vex")
            .expect("vex is on the stage")
            .fields
            .get("hidden"),
        Some(&Value::Bool(true)),
        "the hidden hero is marked on the stage"
    );

    assert_eq!(fixture.phone.hero, "Pip");
    assert_eq!(fixture.phone.status, Status::Telling);
    let story = fixture.phone.narration.join("\n");
    assert!(story.contains("You notice a loose stone."), "{story}");
    assert!(story.contains("It isn't your turn."), "{story}");
    let labels: Vec<&str> = fixture
        .phone
        .suggested
        .iter()
        .map(|choice| choice.label.as_str())
        .collect();
    assert_eq!(labels, ["Look around", "Wait for Vex", "Search the room"]);
    assert_eq!(
        fixture.phone.note.expect("Pip's private note").text,
        "The loose stone hides a key."
    );
}

#[test]
fn another_seat_gets_the_same_screen_and_none_of_pips_secrets() {
    let pip = busy_fixture(PIP_SEAT);
    let vex = busy_fixture(VEX_SEAT);
    assert_eq!(
        vex.tv, pip.tv,
        "the shared screen is the same for every seat"
    );
    assert_eq!(vex.phone.hero, "Vex");
    assert!(vex.phone.note.is_none() && vex.phone.suggested.is_empty());
    let story = vex.phone.narration.join("\n");
    assert!(
        !story.contains("loose stone") && !story.contains("your turn"),
        "{story}"
    );
}

#[test]
fn a_paused_table_and_an_empty_table_still_fill_every_slot() {
    let (snapshot, attachments) = paused_table();
    let (tv, phone) = views(&snapshot, &attachments, PIP_SEAT);
    let waiting = Status::WaitingFor {
        who: "the host".to_owned(),
        what: "to resume the game".to_owned(),
    };
    let fixture = parse(&compose(tv, phone));
    assert_eq!(fixture.tv.status, waiting);
    assert_eq!(fixture.phone.status, waiting);

    let (snapshot, attachments) = empty_table();
    let (tv, phone) = views(&snapshot, &attachments, PIP_SEAT);
    let fixture = parse(&compose(tv, phone));
    assert_eq!(fixture.tv.place, "The table");
    assert!(fixture.tv.caption.is_some(), "the caption is never absent");
    assert!(!fixture.phone.narration.is_empty());
}

#[test]
fn every_status_of_the_projection_composes_into_the_playground_status() {
    assert_eq!(STATUS_KEYS.len(), StatusKind::ALL.len());
    let statuses = [
        ProjectionStatus::Ok,
        ProjectionStatus::Reading,
        ProjectionStatus::Rolling,
        ProjectionStatus::Telling,
        ProjectionStatus::ArtDelayed,
        ProjectionStatus::Degraded,
        ProjectionStatus::ReadOnly,
        ProjectionStatus::Reconnecting,
        ProjectionStatus::waiting_for("Sam", "a choice").expect("words"),
    ];
    let snapshot = table_with_two_heroes().snapshot();
    for status in statuses {
        let key = status.kind().key();
        assert!(
            STATUS_KEYS.contains(&key),
            "{key} is not a playground status"
        );
        let mut attachments = published(snapshot.revision());
        attachments
            .set_activity(status.clone())
            .expect("room for an activity");
        let (tv, phone) = views(&snapshot, &attachments, PIP_SEAT);
        let fixture = parse(&compose(tv, phone));
        let written = serde_json::to_value(&fixture.phone.status).expect("a status serialises");
        assert_eq!(
            written["kind"], key,
            "the status key {key} changed in the playground"
        );
        assert_eq!(
            serde_json::to_value(&fixture.tv.status).expect("a status serialises"),
            written
        );
    }
}

/// Every key path under a value, with array elements merged under `[]` (every element adds its
/// paths) and a null leaf counted as a path of its own: `tv.caption.text`, `tv.party[].name`.
fn paths(prefix: &str, value: &Value, found: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                found.insert(path.clone());
                paths(&path, inner, found);
            }
        }
        Value::Array(items) => {
            for item in items {
                paths(&format!("{prefix}[]"), item, found);
            }
        }
        _ => {}
    }
}

/// What the playground can draw that the projection does not fill yet, and who builds it. A field
/// leaves this list when the projection (or, for the host's own moves, the host) fills it.
const NOT_PROJECTED_YET: [(&str, &str); 23] = [
    // The art and camera of a scene come from the scene builder and the Director (O.14, SB, G1).
    ("tv.backdrop", "scene builder (SB): the plate"),
    ("tv.card", "Director (G1): handouts and portraits"),
    ("tv.camera", "Director (G1): camera focus"),
    // A party strip colour and a stage element's look come with the cast's presentation (K.13).
    ("tv.party[].color", "cast presentation (K.13)"),
    ("tv.elements[].color", "cast presentation (K.13)"),
    ("tv.elements[].rotation_degrees", "Director (G1)"),
    ("tv.elements[].pivot", "Director (G1)"),
    ("tv.elements[].pivot.x_percent", "Director (G1)"),
    ("tv.elements[].pivot.y_percent", "Director (G1)"),
    // The phone's richer pages come with the machines that have goals, groups and handouts.
    ("phone.goal", "journal projection (K.4)"),
    ("phone.image", "scene builder (SB): handouts"),
    ("phone.groups[].choices[].explain", "machine offers (RES33)"),
    // The landing, lobby, loading, narration and roleplay presentation of the TV belongs to the
    // playground's UI track; the table's projection does not carry it.
    ("tv.brand", "playground UI track: landing brand"),
    (
        "tv.clues",
        "persona story: the clue board, pushed as a scene change",
    ),
    (
        "tv.story",
        "persona story: the title and the place reached, pushed as a scene change",
    ),
    (
        "tv.inventory",
        "world inventory: the owner's rows and their moves, pushed as a scene change",
    ),
    ("tv.home", "playground UI track: landing page"),
    ("tv.loading", "playground UI track: loading screen"),
    ("tv.lobby", "playground UI track: lobby"),
    ("tv.narration", "playground UI track: narration card"),
    ("tv.pills", "playground UI track: status pills"),
    ("tv.roleplay", "playground UI track: roleplay card"),
    ("tv.seats", "playground UI track: lobby seat cards"),
];

#[test]
fn the_fixture_holds_no_field_the_projection_does_not_account_for_and_the_reverse() {
    let tables = [busy_table(), paused_table(), empty_table()];
    let mut in_fixture = BTreeSet::new();
    let mut in_projection = BTreeSet::new();
    for (snapshot, attachments) in &tables {
        for seat_number in [PIP_SEAT, VEX_SEAT] {
            if snapshot
                .seat(df_table::identity::SeatId::new(seat_number))
                .is_none()
            {
                continue;
            }
            let (tv, phone) = views(snapshot, attachments, seat_number);
            let written = serde_json::to_value(parse(&compose(tv.clone(), phone.clone())))
                .expect("a fixture serialises");
            for part in ["tv", "phone"] {
                paths(part, &written[part], &mut in_fixture);
            }
            paths("tv", &tv, &mut in_projection);
            paths("phone", &phone, &mut in_projection);
        }
    }
    let listed: BTreeSet<&str> = NOT_PROJECTED_YET.iter().map(|(path, _)| *path).collect();
    let unaccounted: Vec<&String> = in_fixture
        .iter()
        .filter(|path| !in_projection.contains(*path) && !listed.contains(path.as_str()))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "the playground's Fixture has fields the projection does not fill and nobody owns: \
         {unaccounted:?}; fill them in df-table or add them to NOT_PROJECTED_YET with an owner"
    );
    let dropped: Vec<&String> = in_projection.difference(&in_fixture).collect();
    assert!(
        dropped.is_empty(),
        "the projection sends fields the playground drops: {dropped:?}"
    );
    // A field that the projection now fills must leave the list.
    let filled: Vec<&&str> = listed
        .iter()
        .filter(|path| in_projection.contains(**path))
        .collect();
    assert!(
        filled.is_empty(),
        "these fields are projected now, remove them from NOT_PROJECTED_YET: {filled:?}"
    );
}

/// The keys each view must carry. The playground defaults some of them when they are missing (a
/// phone with no `narration` parses), so parsing alone cannot tell that the projection stopped
/// sending one.
const TV_KEYS: [&str; 5] = ["place", "status", "caption", "party", "elements"];
const PHONE_KEYS: [&str; 4] = ["hero", "status", "narration", "suggested"];

/// What is wrong with a pair of views as a fixture, or `None` if the playground accepts them and
/// every key it depends on is there.
fn contract_problem(tv: &Value, phone: &Value) -> Option<String> {
    for (part, view, keys) in [("tv", tv, &TV_KEYS[..]), ("phone", phone, &PHONE_KEYS[..])] {
        for key in keys {
            if view.get(*key).is_none() {
                return Some(format!("{part} has no `{key}`"));
            }
        }
    }
    Fixture::parse(&compose(tv.clone(), phone.clone()))
        .err()
        .map(|error| error.message)
}

/// The real projection JSON of the busy table, changed by `edit`, and whether the contract holds.
fn accepted_after(edit: impl FnOnce(&mut Value, &mut Value)) -> bool {
    let (snapshot, attachments) = busy_table();
    let (mut tv, mut phone) = views(&snapshot, &attachments, PIP_SEAT);
    edit(&mut tv, &mut phone);
    contract_problem(&tv, &phone).is_none()
}

#[test]
fn the_playground_refuses_a_real_projection_that_has_drifted() {
    // The unchanged projection is accepted, so each refusal below is the edit's doing.
    assert!(accepted_after(|_, _| {}));
    // A key renamed on the projection side.
    assert!(!accepted_after(|tv, _| {
        let place = tv["place"].take();
        if let Some(map) = tv.as_object_mut() {
            map.remove("place");
            map.insert("location".to_owned(), place);
        }
    }));
    // A key dropped.
    assert!(!accepted_after(|_, phone| {
        if let Some(map) = phone.as_object_mut() {
            map.remove("narration");
        }
    }));
    // A key the playground does not know.
    assert!(!accepted_after(|tv, _| tv["weather"] = json!("rain")));
    assert!(!accepted_after(|_, phone| {
        phone["note"]["colour"] = json!("red");
    }));
    // A value of another kind: narration as an object, a number as text.
    assert!(!accepted_after(|_, phone| {
        phone["narration"] = json!({ "lines": [] });
    }));
    assert!(!accepted_after(|tv, _| {
        tv["party"][0]["hit_points"] = json!("seven");
    }));
    // A status that changes shape: the wait's words under other keys, or a kind that is unknown.
    assert!(!accepted_after(|tv, _| {
        tv["status"] = json!({ "kind": "waiting_for", "person": "Sam", "task": "a choice" });
    }));
    assert!(!accepted_after(
        |tv, _| tv["status"] = json!({ "kind": "thinking" })
    ));
}
