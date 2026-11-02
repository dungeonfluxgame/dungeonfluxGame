use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::effects::TableEffect;
use crate::identity::{EvidenceRef, FactId, RecoveryEpoch, SessionRevision, TableId, TableScope};
use crate::machine::MachineReadout;
use crate::outcome::{Decline, DeclineCode, OutcomeKind};
use crate::ports::{ActionId, Audience};
use crate::private_data::{LengthClass, Narration, Secret};
use crate::projection::bounds::PHONE_NARRATION_LINES;
use crate::projection::{
    AttachmentError, AttachmentLimits, AttachmentVersion, Attachments, AudienceKind,
    AudienceProjection, NonEmptyText, ProjectionError, ProjectionStatus, StatusKind, StoryLine,
    StructuredReadout, Viewer, project, project_seat, project_shared,
};
use crate::table_state::{CastChange, SceneGeneration, TableState, Visibility};
use crate::table_state_tests::{
    apply, creature, display_name, element, hero, scene, seat, table_in_scene_ten,
};
use crate::testkit::correlation;

pub(crate) fn text(words: &str) -> NonEmptyText {
    NonEmptyText::new(words).expect("not blank")
}

pub(crate) fn public_line(words: &str) -> StoryLine {
    StoryLine::new(text(words))
}

pub(crate) fn json_of<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("serializes")
}

/// The revision of `table_with_two_heroes`: two commits in.
pub(crate) fn at() -> SessionRevision {
    correlation(2).revision
}

pub(crate) fn evidence(byte: u8) -> EvidenceRef {
    EvidenceRef::from_bytes(&[byte; 16]).expect("nonzero")
}

pub(crate) fn fact_id(byte: u8) -> FactId {
    FactId::from_bytes(&[byte; 16]).expect("nonzero")
}

/// Seats 1 and 2 in scene 10, with their heroes Pip and Vex and a seen goblin on the stage.
pub(crate) fn table_with_two_heroes() -> TableState {
    let mut table = table_in_scene_ten();
    apply(
        &mut table,
        2,
        &[
            TableEffect::CastEntered {
                entry: hero("pip", "Pip", 1, 10, 30),
            },
            TableEffect::CastEntered {
                entry: hero("vex", "Vex", 2, 10, 50),
            },
            TableEffect::CastEntered {
                entry: creature("goblin", "Goblin", 10, 70),
            },
        ],
    )
    .expect("the cast enters");
    table
}

fn shared_json(table: &TableState, attachments: &Attachments) -> Value {
    json_of(&project_shared(&table.snapshot(), attachments))
}

fn seat_json(table: &TableState, attachments: &Attachments, seat_number: u64) -> Value {
    let projection =
        project_seat(&table.snapshot(), attachments, seat(seat_number)).expect("a seated seat");
    json_of(&projection)
}

fn keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect()
}

fn set<'a>(names: &[&'a str]) -> BTreeSet<&'a str> {
    names.iter().copied().collect()
}

fn strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .expect("an array")
        .iter()
        .map(|item| item.as_str().expect("a string"))
        .collect()
}

/// Every string anywhere in the value has something to read. A null is allowed.
fn assert_no_blank_text(value: &Value, path: &str) {
    match value {
        Value::String(text) => assert!(!text.trim().is_empty(), "blank text at {path}"),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                assert_no_blank_text(item, &format!("{path}[{index}]"));
            }
        }
        Value::Object(fields) => {
            for (key, field) in fields {
                assert_no_blank_text(field, &format!("{path}.{key}"));
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

// The JSON shape the playground's fixture reads.

#[test]
fn the_shared_view_has_the_keys_of_the_playgrounds_tv_scene_and_nothing_else() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    attachments
        .set_place(scene(10), text("The cellar stairs"))
        .unwrap();
    attachments
        .push_public(public_line("Pip chose attack: done.").spoken_by(display_name("Pip")))
        .unwrap();
    let view = &shared_json(&table, &attachments)["view"];
    assert_eq!(
        keys(view),
        set(&["place", "status", "caption", "party", "elements"])
    );
    assert_eq!(view["place"], "The cellar stairs");
    assert_eq!(view["status"], json!({"kind": "ok"}));
    assert_eq!(keys(&view["caption"]), set(&["speaker", "text"]));
    let party = view["party"].as_array().unwrap();
    assert_eq!(party.len(), 2);
    assert_eq!(
        keys(&party[0]),
        set(&["name", "hit_points", "maximum_hit_points", "conditions"])
    );
    assert_eq!(party[0]["name"], "Pip");
    assert_eq!(party[0]["hit_points"], 7);
    assert_eq!(party[0]["maximum_hit_points"], 9);
    assert_eq!(party[0]["conditions"], json!([]));
    let elements = view["elements"].as_array().unwrap();
    assert_eq!(elements.len(), 3);
    assert_eq!(
        keys(&elements[0]),
        set(&[
            "id",
            "kind",
            "label",
            "x",
            "y",
            "shape",
            "width_units",
            "height_units"
        ])
    );
    assert_eq!(elements[0]["id"], "goblin");
    assert_eq!(elements[0]["kind"], "npc");
    assert_eq!(elements[0]["shape"], "circle");
    assert_eq!(elements[0]["x"], 70.0);
    assert_eq!(elements[0]["y"], 30.0);
}

#[test]
fn a_seat_view_has_the_keys_of_the_playgrounds_phone_scene_and_nothing_else() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    attachments
        .push_public(public_line("Pip chose attack: done."))
        .unwrap();
    let view = &seat_json(&table, &attachments, 1)["view"];
    assert_eq!(
        keys(view),
        set(&["hero", "status", "narration", "suggested"])
    );
    assert_eq!(view["hero"], "Pip");
    assert_eq!(strings(&view["narration"]), ["Pip chose attack: done."]);
    assert_eq!(view["suggested"], json!([]));
    attachments
        .set_private_fact(
            seat(1),
            fact_id(1),
            Secret::new("The rug hides a trapdoor.".to_owned()),
        )
        .unwrap();
    let with_note = &seat_json(&table, &attachments, 1)["view"];
    assert_eq!(
        keys(with_note),
        set(&["hero", "status", "narration", "suggested", "note"])
    );
    assert_eq!(keys(&with_note["note"]), set(&["text"]));
}

#[test]
fn a_projection_serializes_with_its_viewer_its_revision_and_its_view() {
    let table = table_with_two_heroes();
    let attachments = Attachments::new();
    let shared = shared_json(&table, &attachments);
    assert_eq!(
        keys(&shared),
        set(&[
            "viewer",
            "revision",
            "attachment_version",
            "machine_id",
            "state_label",
            "readout",
            "truncation",
            "view"
        ])
    );
    assert_eq!(shared["viewer"], json!({"kind": "shared"}));
    assert_eq!(shared["revision"], json!({"epoch": 1, "sequence": 2}));
    let seat_projection = seat_json(&table, &attachments, 2);
    assert_eq!(
        seat_projection["viewer"],
        json!({"kind": "seat", "seat": 2})
    );
}

#[test]
fn a_projection_before_the_first_commit_has_the_genesis_revision_and_still_has_words() {
    let table = TableState::seated(4, &[seat(1)]).expect("valid table");
    let projection = project_shared(&table.snapshot(), &Attachments::new());
    let genesis = SessionRevision::new(RecoveryEpoch::FIRST, 0);
    assert_eq!(projection.revision(), genesis);
    assert_eq!(
        json_of(&projection)["revision"],
        json!({"epoch": 1, "sequence": 0}),
        "sequence zero is a revision the wire accepts"
    );
    assert!(!projection.view().caption.text.as_str().is_empty());
}

#[test]
fn the_status_serializes_as_the_playgrounds_status() {
    assert_eq!(json_of(&ProjectionStatus::Ok), json!({"kind": "ok"}));
    assert_eq!(
        json_of(&ProjectionStatus::ArtDelayed),
        json!({"kind": "art_delayed"})
    );
    assert_eq!(
        json_of(&ProjectionStatus::ReadOnly),
        json!({"kind": "read_only"})
    );
    let waiting = ProjectionStatus::waiting_for("Pip", "to answer a reaction").unwrap();
    assert_eq!(
        json_of(&waiting),
        json!({"kind": "waiting_for", "who": "Pip", "what": "to answer a reaction"})
    );
    assert_eq!(waiting.kind(), StatusKind::WaitingFor);
    assert!(ProjectionStatus::waiting_for(" ", "to answer").is_err());
    assert!(ProjectionStatus::waiting_for("Pip", "").is_err());
    let all = [
        (ProjectionStatus::Ok, "ok"),
        (ProjectionStatus::Reading, "reading"),
        (ProjectionStatus::Rolling, "rolling"),
        (ProjectionStatus::Telling, "telling"),
        (ProjectionStatus::ArtDelayed, "art_delayed"),
        (ProjectionStatus::Degraded, "degraded"),
        (ProjectionStatus::ReadOnly, "read_only"),
        (ProjectionStatus::Reconnecting, "reconnecting"),
        (waiting, "waiting_for"),
    ];
    assert_eq!(all.len(), StatusKind::ALL.len());
    for (status, key) in all {
        assert_eq!(status.kind().key(), key);
        assert_eq!(json_of(&status)["kind"], key);
    }
}

// Every hero is always visible; a creature hidden from the party is left out (K.13, M2-D17, R06).

fn ogre_hidden(table: &mut TableState, turn: u64, visibility: Visibility) {
    apply(
        table,
        turn,
        &[
            TableEffect::CastEntered {
                entry: creature("ogre", "Ogre", 10, 40),
            },
            TableEffect::CastChanged {
                cast: element("ogre"),
                change: CastChange::Visibility { visibility },
            },
        ],
    )
    .expect("the ogre enters");
}

fn element_ids(view: &Value) -> Vec<&str> {
    view["elements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|element| element["id"].as_str().unwrap())
        .collect()
}

#[test]
fn a_creature_hidden_from_the_party_is_left_out_of_the_shared_view_and_leaves_no_mark() {
    for visibility in [Visibility::Hidden, Visibility::Invisible] {
        let mut table = table_with_two_heroes();
        let plain = shared_json(&table, &Attachments::new());
        ogre_hidden(&mut table, 3, visibility);
        let with_ogre = shared_json(&table, &Attachments::new());
        let mut without_revision = with_ogre.clone();
        without_revision["revision"] = plain["revision"].clone();
        assert_eq!(without_revision, plain, "{visibility:?}");
        assert!(!element_ids(&with_ogre["view"]).contains(&"ogre"));
        assert!(!with_ogre.to_string().contains("Ogre"));
    }
}

#[test]
fn a_creature_the_party_sees_is_drawn() {
    let mut table = table_with_two_heroes();
    ogre_hidden(&mut table, 3, Visibility::Seen);
    let view = &shared_json(&table, &Attachments::new())["view"];
    assert_eq!(element_ids(view), ["goblin", "ogre", "pip", "vex"]);
}

#[test]
fn a_hero_the_party_cannot_see_is_still_drawn_and_counted_with_a_hidden_chip() {
    for visibility in [Visibility::Hidden, Visibility::Invisible] {
        let mut table = table_with_two_heroes();
        apply(
            &mut table,
            3,
            &[TableEffect::CastChanged {
                cast: element("pip"),
                change: CastChange::Visibility { visibility },
            }],
        )
        .expect("changes");
        let projection = project_shared(&table.snapshot(), &Attachments::new());
        let view = json_of(projection.view());
        assert!(element_ids(&view).contains(&"pip"), "{visibility:?}");
        let pip_element = view["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|element| element["id"] == "pip")
            .unwrap();
        assert_eq!(pip_element["fields"], json!({"hidden": true}));
        let party = view["party"].as_array().unwrap();
        assert_eq!(party.len(), 2, "every hero is counted");
        assert_eq!(party[0]["conditions"], json!(["hidden"]));
        assert_eq!(party[1]["conditions"], json!([]));
        assert_eq!(projection.record().party_shown, 2);
        assert_eq!(projection.record().elements_shown, 3);
    }
}

#[test]
fn the_party_strip_keeps_a_hero_who_is_unplaced_or_in_another_scene() {
    let mut table =
        table_with_two_heroes().with_split_policy(crate::table_state::SplitPolicy::Allowed);
    apply(
        &mut table,
        3,
        &[
            TableEffect::SeatMoved {
                seat: seat(2),
                scene: scene(11),
            },
            TableEffect::CastChanged {
                cast: element("pip"),
                change: CastChange::Placement { placement: None },
            },
        ],
    )
    .expect("applies");
    let view = json_of(project_shared(&table.snapshot(), &Attachments::new()).view());
    assert_eq!(view["party"].as_array().unwrap().len(), 2, "both heroes");
    assert_eq!(
        element_ids(&view),
        ["goblin"],
        "the stage shows the focus scene's placed ones"
    );
}

#[test]
fn a_hero_with_conditions_shows_them_and_a_dying_hero_still_shows() {
    let mut table = table_with_two_heroes();
    apply(
        &mut table,
        3,
        &[
            TableEffect::CastChanged {
                cast: element("pip"),
                change: CastChange::Conditions {
                    conditions: vec![
                        crate::table_state::ConditionName::new("poisoned").unwrap(),
                        crate::table_state::ConditionName::new("prone").unwrap(),
                    ],
                },
            },
            TableEffect::CastChanged {
                cast: element("pip"),
                change: CastChange::HitPoints { hit_points: 0 },
            },
        ],
    )
    .expect("applies");
    let view = json_of(project_shared(&table.snapshot(), &Attachments::new()).view());
    assert_eq!(view["party"][0]["conditions"], json!(["poisoned", "prone"]));
    assert_eq!(view["party"][0]["hit_points"], 0);
}

// Never empty (H.15).

#[test]
fn the_shared_caption_is_the_newest_public_line_then_the_machines_readout_then_a_plain_sentence() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    attachments
        .set_place(scene(10), text("The cellar stairs"))
        .unwrap();
    let plain = shared_json(&table, &attachments);
    assert_eq!(
        plain["view"]["caption"],
        json!({"speaker": null, "text": "The party is at The cellar stairs."})
    );
    let readout = MachineReadout::new("One check is offered.").unwrap();
    attachments
        .set_shared_readout(NonEmptyText::from(&readout))
        .unwrap();
    assert_eq!(
        shared_json(&table, &attachments)["view"]["caption"]["text"],
        "One check is offered."
    );
    attachments
        .push_public(public_line("Pip chose attack: done."))
        .unwrap();
    attachments
        .push_public(public_line("Vex chose dodge: done.").spoken_by(display_name("Vex")))
        .unwrap();
    assert_eq!(
        shared_json(&table, &attachments)["view"]["caption"],
        json!({"speaker": null, "text": "Vex chose dodge: done."}),
        "the structured readout is the table's line and has no speaker"
    );
    attachments
        .push_public(
            public_line("Vex chose talk: done.")
                .spoken_by(display_name("Vex"))
                .with_narration(Narration::new("Hold the stairs!")),
        )
        .unwrap();
    assert_eq!(
        shared_json(&table, &attachments)["view"]["caption"],
        json!({"speaker": "Vex", "text": "Hold the stairs!"}),
        "the words are the speaker's"
    );
}

#[test]
fn a_blank_narration_with_a_speaker_shows_the_readout_with_no_speaker() {
    let table = table_with_two_heroes();
    for blank in [
        "", "  
 ",
    ] {
        let mut attachments = Attachments::new();
        attachments
            .push_public(
                public_line("Vex chose dodge: done.")
                    .spoken_by(display_name("Vex"))
                    .with_narration(Narration::new(blank)),
            )
            .unwrap();
        assert_eq!(
            shared_json(&table, &attachments)["view"]["caption"],
            json!({"speaker": null, "text": "Vex chose dodge: done."})
        );
    }
}

#[test]
fn a_blank_model_output_is_replaced_by_the_structured_readout_and_real_words_stand_in_for_it() {
    let table = table_with_two_heroes();
    for (narration, expected) in [
        ("", "Pip chose attack: done."),
        ("   \n\t ", "Pip chose attack: done."),
        (
            "The blade skids off the shield.",
            "The blade skids off the shield.",
        ),
    ] {
        let mut attachments = Attachments::new();
        attachments
            .push_public(
                public_line("Pip chose attack: done.").with_narration(Narration::new(narration)),
            )
            .unwrap();
        let caption = &shared_json(&table, &attachments)["view"]["caption"]["text"];
        assert_eq!(caption, expected);
    }
}

#[test]
fn a_table_with_nothing_to_say_still_says_where_it_is() {
    let lobby = TableState::seated(4, &[seat(1), seat(2)]).expect("valid table");
    let attachments = Attachments::new();
    let shared = shared_json(&lobby, &attachments);
    assert_eq!(shared["view"]["place"], "The table");
    assert_eq!(
        shared["view"]["caption"]["text"],
        "The game has not started. Seats at the table: 2."
    );
    let phone = seat_json(&lobby, &attachments, 1);
    assert_eq!(
        strings(&phone["view"]["narration"]),
        ["The game has not started. Seats at the table: 2."]
    );
    assert_eq!(phone["view"]["hero"], "Seat 1");
    let unnamed = table_with_two_heroes();
    assert_eq!(
        shared_json(&unnamed, &attachments)["view"]["place"],
        "Scene 10"
    );
}

#[test]
fn every_state_projects_to_something_non_blank_for_every_audience() {
    let mut paused = table_with_two_heroes();
    apply(&mut paused, 3, &[TableEffect::TablePaused]).expect("pauses");
    let mut idle = table_with_two_heroes();
    apply(&mut idle, 3, &[TableEffect::TableIdled]).expect("idles");
    let mut dead = table_with_two_heroes();
    apply(
        &mut dead,
        3,
        &[
            TableEffect::SeatLeft { seat: seat(1) },
            TableEffect::SeatLeft { seat: seat(2) },
        ],
    )
    .expect("everyone leaves");
    let tables = [
        TableState::seated(4, &[]).expect("empty"),
        TableState::seated(4, &[seat(1)]).expect("lobby"),
        table_in_scene_ten(),
        table_with_two_heroes(),
        paused,
        idle,
        dead,
    ];
    let mut busy = Attachments::new();
    busy.set_place(scene(10), text("The cellar")).unwrap();
    busy.set_activity(ProjectionStatus::Telling).unwrap();
    busy.push_public(public_line("Pip chose attack: done.").with_narration(Narration::new(" ")))
        .unwrap();
    busy.push_private(seat(1), public_line("You hear a click."))
        .unwrap();
    busy.set_notice(
        seat(2),
        Decline::with_default_reason(DeclineCode::NotYourTurn),
        at(),
    )
    .unwrap();
    for table in &tables {
        for attachments in [&Attachments::new(), &busy] {
            let snapshot = table.snapshot();
            let mut audiences = vec![Audience::Public];
            audiences.extend(snapshot.seats().iter().copied().map(Audience::Seat));
            for audience in audiences {
                let projection = project(&snapshot, attachments, audience).expect("projects");
                assert_no_blank_text(&json_of(&projection), "projection");
            }
        }
    }
}

#[test]
fn a_seat_that_is_not_at_the_table_has_no_view_and_never_an_empty_one() {
    let table = table_with_two_heroes();
    let result = project(
        &table.snapshot(),
        &Attachments::new(),
        Audience::Seat(seat(9)),
    );
    assert_eq!(
        result,
        Err(ProjectionError::SeatNotAtTable { seat: seat(9) })
    );
    assert_eq!(
        ProjectionError::SeatNotAtTable { seat: seat(9) }.to_string(),
        "seat 9 is not at the table"
    );
}

// The story a seat reads.

#[test]
fn a_seat_reads_the_public_lines_and_its_own_in_the_order_they_happened() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    attachments.push_public(public_line("one public")).unwrap();
    attachments
        .push_private(seat(1), public_line("two for Pip"))
        .unwrap();
    attachments
        .push_private(seat(2), public_line("three for Vex"))
        .unwrap();
    attachments.push_public(public_line("four public")).unwrap();
    attachments
        .push_private(seat(1), public_line("five for Pip"))
        .unwrap();
    let pip = seat_json(&table, &attachments, 1);
    assert_eq!(
        strings(&pip["view"]["narration"]),
        ["one public", "two for Pip", "four public", "five for Pip"]
    );
    let vex = seat_json(&table, &attachments, 2);
    assert_eq!(
        strings(&vex["view"]["narration"]),
        ["one public", "three for Vex", "four public"]
    );
    let tv = shared_json(&table, &attachments);
    assert_eq!(tv["view"]["caption"]["text"], "four public");
}

#[test]
fn a_phone_shows_the_last_twenty_lines_and_a_line_with_words_brings_its_speaker() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    for number in 0..60 {
        attachments
            .push_public(public_line(&format!("line {number}")))
            .unwrap();
    }
    let lines = seat_json(&table, &attachments, 1);
    let shown = strings(&lines["view"]["narration"]);
    assert_eq!(shown.len(), PHONE_NARRATION_LINES);
    assert_eq!(shown.first(), Some(&"line 40"));
    assert_eq!(shown.last(), Some(&"line 59"));
    attachments
        .push_public(
            public_line("Vex chose talk: done.")
                .spoken_by(display_name("Vex"))
                .with_narration(Narration::new("Hold the stairs!")),
        )
        .unwrap();
    let after = seat_json(&table, &attachments, 1);
    let shown = strings(&after["view"]["narration"]);
    assert_eq!(shown.len(), PHONE_NARRATION_LINES);
    assert_eq!(
        &shown[shown.len() - 2..],
        ["Vex chose talk: done.", "Vex: Hold the stairs!"],
        "the structured readout stands first and the words follow it"
    );
}

#[test]
fn a_decline_goes_to_its_own_seat_as_its_reason_with_the_alternatives_as_choices() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    let decline = Decline::new(DeclineCode::NotYourTurn, "It is Vex's turn.")
        .unwrap()
        .with_alternative("Wait for your turn")
        .unwrap()
        .with_alternative("Ask the host to skip")
        .unwrap();
    attachments.set_notice(seat(1), decline, at()).unwrap();
    let pip = seat_json(&table, &attachments, 1);
    assert_eq!(
        strings(&pip["view"]["narration"]).last(),
        Some(&"It is Vex's turn.")
    );
    assert_eq!(
        pip["view"]["suggested"],
        json!([{"label": "Wait for your turn"}, {"label": "Ask the host to skip"}])
    );
    let vex = seat_json(&table, &attachments, 2);
    assert!(!vex.to_string().contains("Vex's turn"));
    assert_eq!(vex["view"]["suggested"], json!([]));
    assert!(
        !shared_json(&table, &attachments)
            .to_string()
            .contains("Vex's turn")
    );
}

#[test]
fn a_seats_own_readout_stands_where_it_has_no_story_and_a_note_lists_its_facts_one_per_line() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    attachments
        .set_seat_readout(seat(1), text("You can make the lockpick check."))
        .unwrap();
    let pip = seat_json(&table, &attachments, 1);
    assert_eq!(
        strings(&pip["view"]["narration"]),
        ["You can make the lockpick check."]
    );
    attachments
        .set_private_fact(seat(1), fact_id(2), Secret::new("second fact".to_owned()))
        .unwrap();
    attachments
        .set_private_fact(seat(1), fact_id(1), Secret::new("first fact".to_owned()))
        .unwrap();
    attachments
        .set_private_fact(seat(1), fact_id(3), Secret::new("   ".to_owned()))
        .unwrap();
    let with_note = seat_json(&table, &attachments, 1);
    assert_eq!(
        with_note["view"]["note"]["text"], "second fact\nfirst fact",
        "the one learned first reads first"
    );
    let projection = project_seat(&table.snapshot(), &attachments, seat(1)).unwrap();
    assert_eq!(
        projection.record().facts_shown,
        2,
        "a blank fact is not shown"
    );
    attachments
        .set_private_fact(
            seat(1),
            fact_id(1),
            Secret::new("first, corrected".to_owned()),
        )
        .unwrap();
    assert_eq!(
        seat_json(&table, &attachments, 1)["view"]["note"]["text"],
        "second fact\nfirst, corrected",
        "setting a fact again replaces its text and makes it the newest"
    );
}

#[test]
fn the_status_is_the_pipelines_activity_and_a_pause_outranks_it() {
    let mut table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    attachments.set_activity(ProjectionStatus::Reading).unwrap();
    assert_eq!(
        shared_json(&table, &attachments)["view"]["status"],
        json!({"kind": "reading"})
    );
    assert_eq!(
        seat_json(&table, &attachments, 1)["view"]["status"],
        json!({"kind": "reading"})
    );
    apply(&mut table, 3, &[TableEffect::TablePaused]).expect("pauses");
    let waiting = json!({"kind": "waiting_for", "who": "the host", "what": "to resume the game"});
    assert_eq!(shared_json(&table, &attachments)["view"]["status"], waiting);
    assert_eq!(
        seat_json(&table, &attachments, 2)["view"]["status"],
        waiting
    );
    apply(&mut table, 4, &[TableEffect::TableResumed]).expect("resumes");
    apply(&mut table, 5, &[TableEffect::TableIdled]).expect("idles");
    assert_eq!(
        shared_json(&table, &attachments)["view"]["status"],
        json!({"kind": "reading"}),
        "an idle table shows what the pipeline shows"
    );
}

#[test]
fn a_projection_is_deterministic_byte_for_byte() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    attachments
        .set_place(scene(10), text("The cellar"))
        .unwrap();
    attachments.push_public(public_line("one")).unwrap();
    attachments
        .push_private(seat(2), public_line("two"))
        .unwrap();
    attachments
        .set_private_fact(seat(2), fact_id(1), Secret::new("secret".to_owned()))
        .unwrap();
    for audience in [
        Audience::Public,
        Audience::Seat(seat(1)),
        Audience::Seat(seat(2)),
    ] {
        let first = project(&table.snapshot(), &attachments, audience).unwrap();
        let second = project(&table.snapshot(), &attachments.clone(), audience).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );
    }
}

// What a projection says about itself, with no text (RES36).

#[test]
fn the_record_counts_what_was_shown_and_names_the_newest_line_by_class_and_evidence() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    attachments
        .push_public(public_line("Pip chose attack: done.").with_evidence(evidence(7)))
        .unwrap();
    attachments
        .push_private(
            seat(1),
            public_line("You hear a click behind the door.").with_evidence(evidence(8)),
        )
        .unwrap();
    let shared = project_shared(&table.snapshot(), &attachments);
    let record = shared.record();
    assert_eq!(record.viewer, Viewer::Shared);
    assert_eq!(record.elements_shown, 3);
    assert_eq!(record.party_shown, 2);
    assert_eq!(record.lines_shown, 1);
    assert_eq!(record.newest_line_length, LengthClass::Short);
    assert_eq!(
        record.evidence,
        Some(evidence(7)),
        "the TV never names a private specimen"
    );
    assert_eq!(record.status, StatusKind::Ok);
    let pip = project_seat(&table.snapshot(), &attachments, seat(1)).unwrap();
    assert_eq!(pip.record().lines_shown, 2);
    assert_eq!(pip.record().evidence, Some(evidence(8)));
    assert_eq!(pip.record().seat(), Some(seat(1)));
    assert_eq!(pip.record().newest_line_length, LengthClass::Short);
}

#[test]
fn a_projection_event_is_one_class_b_line_with_the_pinned_context_and_no_text() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    attachments
        .push_private(
            seat(1),
            public_line("SENTINEL-SECRET-7f3a whispered").with_evidence(evidence(8)),
        )
        .unwrap();
    attachments
        .set_private_fact(
            seat(1),
            fact_id(1),
            Secret::new("SENTINEL-SECRET-7f3a".to_owned()),
        )
        .unwrap();
    let scope = TableScope::of_table(TableId::new(1)).in_scene(scene(10));
    let projection = project_seat(&table.snapshot(), &attachments, seat(1)).unwrap();
    let event = projection.record().event(&scope, correlation(4));
    assert_eq!(event.class(), crate::event::EventClass::Verbose);
    assert_eq!(event.area(), "PROJECTION");
    let line = event.render_audit_line();
    assert!(
        line.starts_with("PROJECTION built table=1 scene=10 operation_id="),
        "{line}"
    );
    assert!(
        line.contains(" turn_seq=4 epoch=1 revision=4 audience=seat seat=1 status=ok elements=0"),
        "{line}"
    );
    assert!(line.contains(" lines=1 length=short evidence="), "{line}");
    assert!(!line.contains("SENTINEL"), "{line}");
    let shared = project_shared(&table.snapshot(), &attachments);
    let shared_line = shared
        .record()
        .event(&scope, correlation(4))
        .render_audit_line();
    assert!(
        shared_line.contains(" audience=shared status=ok elements=3 lines=0 length="),
        "{shared_line}"
    );
    assert!(!shared_line.contains(" seat="), "{shared_line}");
    let round_trip: crate::event::TableEvent =
        serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
    assert_eq!(round_trip, event);
}

#[test]
fn the_widest_projection_event_line_fits_the_reserved_capacity() {
    use crate::identity::{Correlation, OperationId, RecoveryEpoch, SessionRevision, TurnSeq};
    let scope = TableScope {
        table: TableId::new(u64::MAX),
        scene: Some(scene(u64::MAX)),
        machine: Some(crate::table_state_tests::machine(&"m".repeat(256))),
    };
    let correlation = Correlation {
        operation_id: OperationId::from_bytes(&[0xff; 16]).unwrap(),
        turn_seq: TurnSeq::new(u64::MAX),
        revision: SessionRevision::new(RecoveryEpoch::new(u64::MAX).unwrap(), u64::MAX),
    };
    let record = crate::projection::ProjectionRecord {
        viewer: Viewer::Seat {
            seat: seat(u64::MAX),
        },
        revision: SessionRevision::new(RecoveryEpoch::FIRST, 0),
        attachment_version: AttachmentVersion::default(),
        status: StatusKind::WaitingFor,
        elements_shown: usize::MAX,
        party_shown: usize::MAX,
        lines_shown: usize::MAX,
        facts_shown: usize::MAX,
        newest_line_length: LengthClass::Medium,
        evidence: Some(evidence(0xff)),
        work: crate::projection::ProjectionWork::default(),
    };
    let line = record.event(&scope, correlation).render_audit_line();
    assert!(
        line.len() <= crate::event::AUDIT_LINE_CAPACITY_BYTES,
        "{} bytes",
        line.len()
    );
}

#[test]
fn every_status_kind_and_audience_kind_has_a_distinct_key() {
    let mut keys: Vec<_> = StatusKind::ALL.iter().map(|kind| kind.key()).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), StatusKind::ALL.len());
    assert_eq!(AudienceKind::of(Viewer::Shared).key(), "shared");
    assert_eq!(
        AudienceKind::of(Viewer::Seat { seat: seat(1) }).key(),
        "seat"
    );
    assert_eq!(Viewer::of(Audience::Public), Viewer::Shared);
    assert_eq!(
        Viewer::of(Audience::Seat(seat(3))),
        Viewer::Seat { seat: seat(3) }
    );
}

// The structured readout and the never-empty text type (C.7, O.7).

#[test]
fn the_structured_readout_is_a_plain_sentence_from_typed_fields() {
    let who = display_name("Pip");
    let cases = [
        (OutcomeKind::Applied, "Pip chose look around: done."),
        (
            OutcomeKind::NeedsClarification,
            "Pip chose look around: needs a clearer choice.",
        ),
        (
            OutcomeKind::Declined,
            "Pip chose look around: not possible right now.",
        ),
        (
            OutcomeKind::Pending,
            "Pip chose look around: waiting for an answer.",
        ),
    ];
    assert_eq!(cases.len(), OutcomeKind::ALL.len());
    for (outcome, expected) in cases {
        let readout =
            StructuredReadout::new(who.clone(), ActionId::new("look_around").unwrap(), outcome);
        assert_eq!(readout.text().as_str(), expected);
    }
    let hyphen = StructuredReadout::new(
        who,
        ActionId::new("take-cover").unwrap(),
        OutcomeKind::Applied,
    );
    assert_eq!(hyphen.text().as_str(), "Pip chose take cover: done.");
}

#[test]
fn a_blank_text_is_refused_and_the_narration_is_replaced_by_the_readout_when_it_is_blank() {
    assert!(NonEmptyText::new("").is_err());
    assert!(NonEmptyText::new(" \n ").is_err());
    assert_eq!(NonEmptyText::new("Hello").unwrap().as_str(), "Hello");
    let readout = text("Pip chose attack: done.");
    let blank = NonEmptyText::narration_or_readout(&Narration::new("  "), &readout);
    assert_eq!(blank, readout);
    let words = NonEmptyText::narration_or_readout(&Narration::new("The goblin falls."), &readout);
    assert_eq!(words.as_str(), "The goblin falls.");
    assert_eq!(
        NonEmptyText::or_fallback("").as_str(),
        "Something happened."
    );
    assert_eq!(
        format!("{:?}", text("a private sentence")),
        "NonEmptyText(length=short)"
    );
}

#[test]
fn the_generation_type_prints_and_orders_as_a_number() {
    assert_eq!(SceneGeneration::new(7).to_string(), "7");
    assert!(SceneGeneration::new(8) > SceneGeneration::new(7));
}

#[test]
fn the_audience_projection_enum_reports_its_viewer_and_record() {
    let table = table_with_two_heroes();
    let attachments = Attachments::new();
    let shared = project(&table.snapshot(), &attachments, Audience::Public).unwrap();
    assert!(matches!(shared, AudienceProjection::Shared(_)));
    assert_eq!(shared.viewer(), Viewer::Shared);
    assert_eq!(shared.record().viewer, Viewer::Shared);
    let seated = project(&table.snapshot(), &attachments, Audience::Seat(seat(2))).unwrap();
    assert!(matches!(seated, AudienceProjection::Seat(_)));
    assert_eq!(seated.viewer(), Viewer::Seat { seat: seat(2) });
}

#[test]
fn a_projection_holds_no_table_effect_and_changes_nothing() {
    let table = table_with_two_heroes();
    let before = table.clone();
    let snapshot = table.snapshot();
    let attachments = Attachments::new();
    let _ = project(&snapshot, &attachments, Audience::Public).unwrap();
    let _ = project(&snapshot, &attachments, Audience::Seat(seat(1))).unwrap();
    assert_eq!(table, before);
    assert_eq!(attachments, Attachments::new());
}

// Every collection of attachments is bounded, and the story forgets its oldest line first.

fn small(story_entries: usize, facts_per_seat: usize, seats: usize, places: usize) -> Attachments {
    Attachments::with_limits(AttachmentLimits {
        story_entries,
        facts_per_seat,
        seats,
        places,
        ..AttachmentLimits::default()
    })
}

#[test]
fn a_story_queue_over_its_limit_forgets_the_oldest_line_first_and_the_queues_are_independent() {
    let table = table_with_two_heroes();
    let mut attachments = small(3, 10, 10, 10);
    for number in 1..=5 {
        attachments
            .push_public(public_line(&format!("public {number}")))
            .unwrap();
        attachments
            .push_private(seat(1), public_line(&format!("private {number}")))
            .unwrap();
    }
    let pip = seat_json(&table, &attachments, 1);
    assert_eq!(
        strings(&pip["view"]["narration"]),
        [
            "public 3",
            "private 3",
            "public 4",
            "private 4",
            "public 5",
            "private 5"
        ],
        "each queue kept its own newest three, in the order they happened"
    );
    let vex = seat_json(&table, &attachments, 2);
    assert_eq!(
        strings(&vex["view"]["narration"]),
        ["public 3", "public 4", "public 5"]
    );
}

#[test]
fn a_seat_over_the_seat_limit_is_refused_and_a_seat_already_known_still_publishes() {
    let mut attachments = small(10, 10, 2, 10);
    attachments
        .push_private(seat(1), public_line("one"))
        .unwrap();
    attachments
        .push_private(seat(2), public_line("two"))
        .unwrap();
    assert_eq!(
        attachments.push_private(seat(3), public_line("three")),
        Err(AttachmentError::TooManySeats)
    );
    assert_eq!(
        attachments.set_notice(
            seat(3),
            Decline::with_default_reason(DeclineCode::NotViable),
            at()
        ),
        Ok(()),
        "each kind of entry counts its own seats"
    );
    assert_eq!(
        attachments.set_seat_readout(seat(3), text("a readout")),
        Ok(())
    );
    attachments
        .push_private(seat(1), public_line("one again"))
        .unwrap();
    assert_eq!(
        attachments.set_private_fact(seat(1), fact_id(1), Secret::new("a".to_owned())),
        Ok(())
    );
    assert_eq!(
        attachments.set_private_fact(seat(2), fact_id(1), Secret::new("b".to_owned())),
        Ok(())
    );
    assert_eq!(
        attachments.set_private_fact(seat(3), fact_id(1), Secret::new("c".to_owned())),
        Err(AttachmentError::TooManySeats)
    );
    attachments
        .set_notice(
            seat(4),
            Decline::with_default_reason(DeclineCode::NotViable),
            at(),
        )
        .unwrap();
    assert_eq!(
        attachments.set_notice(
            seat(5),
            Decline::with_default_reason(DeclineCode::NotViable),
            at()
        ),
        Err(AttachmentError::TooManySeats)
    );
    assert_eq!(
        attachments.set_seat_readout(seat(4), text("a readout")),
        Ok(()),
        "the readouts have only seat 3 so far"
    );
    assert_eq!(
        attachments.set_seat_readout(seat(5), text("a readout")),
        Err(AttachmentError::TooManySeats)
    );
}

#[test]
fn a_seat_knows_at_most_its_limit_of_facts_and_a_fact_set_again_does_not_count_twice() {
    let mut attachments = small(10, 2, 10, 10);
    let set = |attachments: &mut Attachments, id: u8, text: &str| {
        attachments.set_private_fact(seat(1), fact_id(id), Secret::new(text.to_owned()))
    };
    assert_eq!(set(&mut attachments, 1, "a"), Ok(()));
    assert_eq!(set(&mut attachments, 2, "b"), Ok(()));
    assert_eq!(
        set(&mut attachments, 3, "c"),
        Err(AttachmentError::TooManyFacts)
    );
    assert_eq!(set(&mut attachments, 2, "b, corrected"), Ok(()));
    assert_eq!(
        attachments.set_private_fact(seat(2), fact_id(3), Secret::new("c".to_owned())),
        Ok(()),
        "the limit is per seat"
    );
}

#[test]
fn only_so_many_scenes_have_a_place_name_and_a_scene_named_again_is_renamed() {
    let mut attachments = small(10, 10, 10, 1);
    assert_eq!(attachments.set_place(scene(10), text("The cellar")), Ok(()));
    assert_eq!(
        attachments.set_place(scene(11), text("The attic")),
        Err(AttachmentError::TooManyScenes)
    );
    assert_eq!(
        attachments.set_place(scene(10), text("The deep cellar")),
        Ok(())
    );
    let table = table_with_two_heroes();
    assert_eq!(
        shared_json(&table, &attachments)["view"]["place"],
        "The deep cellar"
    );
}

#[test]
fn every_attachment_error_reads_as_a_distinct_sentence() {
    let errors = [
        AttachmentError::TooManySeats,
        AttachmentError::TooManyScenes,
        AttachmentError::TooManyFacts,
        AttachmentError::TooManyOffers,
        AttachmentError::DuplicateOffer,
        AttachmentError::TooManyMachines,
        AttachmentError::TextTooLong,
        AttachmentError::VersionExhausted,
    ];
    let sentences: BTreeSet<_> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(sentences.len(), errors.len());
    let defaults = AttachmentLimits::default();
    assert_eq!(
        defaults.story_entries,
        crate::projection::bounds::MAXIMUM_STORY_ENTRIES
    );
    assert_eq!(defaults.seats, crate::owner_queue::MAXIMUM_SEATS);
}
