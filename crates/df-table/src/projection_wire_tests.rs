//! What the wire's `MachineView` needs beside the playground's scene (#1687): the machine and its
//! state, a readout, a reason for any status but ok, tappable offers, a revision that always
//! exists, and every text inside the wire frame (#1686). Also the hero's portrait and colour.

use serde_json::{Value, json};

use crate::effects::TableEffect;
use crate::outcome::{Decline, DeclineCode};
use crate::private_data::{Narration, Secret};
use crate::projection::bounds::{MAXIMUM_TEXT_BYTES, OFFERS_BUDGET_BYTES, WIRE_FRAME_BYTES};
use crate::projection::{
    AttachmentError, Attachments, OfferEntry, OfferId, ProjectionStatus, StoryLine, project_seat,
    project_shared,
};
use crate::projection_tests::{at, fact_id, json_of, public_line, table_with_two_heroes, text};
use crate::table_state::{CastEntry, HeroColor, MediaName, TableState};
use crate::table_state_tests::{
    apply, creature, hero, machine, scene, seat, set_active, slot_with, table_in_scene_ten,
};

fn offer(id: &str, label: &str) -> OfferEntry {
    OfferEntry::new(OfferId::new(id).unwrap(), text(label))
}

fn seat_envelope(table: &TableState, attachments: &Attachments, number: u64) -> Value {
    json_of(&project_seat(&table.snapshot(), attachments, seat(number)).unwrap())
}

#[test]
fn the_view_names_the_machine_in_the_focus_scene_and_its_state() {
    let mut table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    let none = json_of(&project_shared(&table.snapshot(), &attachments));
    assert_eq!(none["machine_id"], "table");
    assert_eq!(none["state_label"], "waiting");
    apply(&mut table, 3, &[set_active(10, "explore")]).unwrap();
    attachments
        .set_state_label(machine("explore"), text("Presenting the gate"))
        .unwrap();
    let shared = json_of(&project_shared(&table.snapshot(), &attachments));
    assert_eq!(shared["machine_id"], "explore");
    assert_eq!(shared["state_label"], "Presenting the gate");
    let phone = seat_envelope(&table, &attachments, 1);
    assert_eq!(phone["machine_id"], "explore");
    assert_eq!(phone["state_label"], "Presenting the gate");
    let unlabelled = json_of(&project_shared(&table.snapshot(), &Attachments::new()));
    assert_eq!(
        unlabelled["state_label"], "waiting",
        "a named fallback, never blank"
    );
}

#[test]
fn the_table_machine_is_named_while_no_scene_has_a_group_machine() {
    let mut table = table_in_scene_ten();
    apply(
        &mut table,
        2,
        &[TableEffect::ActiveMachineSet {
            home: crate::table_state::MachineHome::Table,
            machine: machine("lobby"),
            slot: slot_with("lobby", "gathering"),
        }],
    )
    .unwrap();
    let shared = json_of(&project_shared(&table.snapshot(), &Attachments::new()));
    assert_eq!(shared["machine_id"], "lobby");
}

#[test]
fn a_reason_is_present_exactly_when_the_status_is_not_ok_and_reads_as_the_playgrounds_copy() {
    let table = table_with_two_heroes();
    let cases = [
        (ProjectionStatus::Ok, None),
        (ProjectionStatus::Reading, Some("Reading your move")),
        (ProjectionStatus::Rolling, Some("Rolling")),
        (ProjectionStatus::Telling, Some("Telling it")),
        (ProjectionStatus::ArtDelayed, Some("Picture catching up")),
        (
            ProjectionStatus::Degraded,
            Some("Some pictures are unavailable. Play continues."),
        ),
        (
            ProjectionStatus::ReadOnly,
            Some("Could not save. Try again."),
        ),
        (ProjectionStatus::Reconnecting, Some("Reconnecting")),
        (
            ProjectionStatus::waiting_for("Pip", "to answer a reaction").unwrap(),
            Some("Waiting for Pip to answer a reaction"),
        ),
    ];
    assert_eq!(cases.len(), crate::projection::StatusKind::ALL.len());
    for (status, expected) in cases {
        let mut attachments = Attachments::new();
        attachments.set_activity(status.clone()).unwrap();
        for envelope in [
            json_of(&project_shared(&table.snapshot(), &attachments)),
            seat_envelope(&table, &attachments, 1),
        ] {
            match expected {
                Some(sentence) => assert_eq!(envelope["reason"], sentence, "{status:?}"),
                None => assert!(envelope.get("reason").is_none(), "{status:?}"),
            }
        }
        assert_eq!(
            status
                .reason()
                .map(|reason| reason.as_str().to_owned())
                .as_deref(),
            expected
        );
    }
}

#[test]
fn a_paused_table_has_the_waiting_reason_on_every_view() {
    let mut table = table_with_two_heroes();
    apply(&mut table, 3, &[TableEffect::TablePaused]).unwrap();
    let attachments = Attachments::new();
    assert_eq!(
        json_of(&project_shared(&table.snapshot(), &attachments))["reason"],
        "Waiting for the host to resume the game"
    );
    assert_eq!(
        seat_envelope(&table, &attachments, 2)["reason"],
        "Waiting for the host to resume the game"
    );
}

#[test]
fn the_readout_is_the_machines_then_the_newest_line_s_then_a_plain_sentence() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    let plain = json_of(&project_shared(&table.snapshot(), &attachments));
    assert_eq!(plain["readout"], "The party is at scene 10.");
    attachments
        .push_public(
            public_line("Pip chose attack: done.").with_narration(Narration::new("A hit!")),
        )
        .unwrap();
    attachments
        .push_private(seat(1), public_line("You feel watched."))
        .unwrap();
    let shared = json_of(&project_shared(&table.snapshot(), &attachments));
    assert_eq!(
        shared["readout"], "Pip chose attack: done.",
        "the readout, not the words"
    );
    assert!(!shared.to_string().contains("watched"));
    assert_eq!(
        seat_envelope(&table, &attachments, 1)["readout"],
        "You feel watched."
    );
    assert_eq!(
        seat_envelope(&table, &attachments, 2)["readout"],
        "Pip chose attack: done."
    );
    attachments
        .set_seat_readout(seat(2), text("You can dodge."))
        .unwrap();
    assert_eq!(
        seat_envelope(&table, &attachments, 2)["readout"],
        "You can dodge."
    );
    attachments
        .set_shared_readout(text("One check is offered."))
        .unwrap();
    assert_eq!(
        json_of(&project_shared(&table.snapshot(), &attachments))["readout"],
        "Pip chose attack: done.",
        "a line is newer than the machine's standing words"
    );
}

#[test]
fn offers_travel_beside_the_seats_view_with_their_ids_and_split_into_the_phones_three_lists() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    attachments
        .set_seat_offers(
            seat(1),
            vec![
                offer("shoot", "Shoot the goblin").explained(text("Short bow, 30 ft")),
                offer("dodge", "Dodge"),
                offer("bow", "Short bow").in_group(text("Attack")),
                offer("dagger", "Dagger").in_group(text("Attack")),
                offer("second-wind", "Second Wind").unavailable(text("Already used this rest")),
            ],
        )
        .unwrap();
    let envelope = seat_envelope(&table, &attachments, 1);
    let ids: Vec<&str> = envelope["offers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|offer| offer["offer_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["shoot", "dodge", "bow", "dagger", "second-wind"]);
    let view = &envelope["view"];
    assert_eq!(
        view["suggested"],
        json!([{"label": "Shoot the goblin", "explain": "Short bow, 30 ft"}, {"label": "Dodge"}])
    );
    assert_eq!(
        view["groups"],
        json!([{"name": "Attack", "choices": [{"label": "Short bow"}, {"label": "Dagger"}]}])
    );
    assert_eq!(
        view["unavailable"],
        json!([{"label": "Second Wind", "reason": "Already used this rest"}])
    );
    assert!(
        !view.to_string().contains("shoot"),
        "no id in the playground's scene"
    );
    let other = seat_envelope(&table, &attachments, 2);
    assert!(
        other.get("offers").is_none(),
        "another seat's offers never reach this one"
    );
    assert!(!other.to_string().contains("Dagger"));
    let shared = json_of(&project_shared(&table.snapshot(), &attachments));
    assert!(shared.get("offers").is_none());
    assert!(!shared.to_string().contains("Dagger"));
}

#[test]
fn a_declines_alternatives_become_tappable_offers_and_two_offers_never_share_an_id() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    attachments
        .set_seat_offers(seat(1), vec![offer("alternative-1", "A published one")])
        .unwrap();
    let decline = Decline::new(DeclineCode::NotYourTurn, "It is Vex's turn.")
        .unwrap()
        .with_alternative("Wait for your turn")
        .unwrap()
        .with_alternative("Ask the host to skip")
        .unwrap();
    attachments.set_notice(seat(1), decline, at()).unwrap();
    let envelope = seat_envelope(&table, &attachments, 1);
    let pairs: Vec<(&str, &str)> = envelope["offers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|offer| {
            (
                offer["offer_id"].as_str().unwrap(),
                offer["label"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        pairs,
        [
            ("alternative-1", "A published one"),
            ("alternative-2", "Ask the host to skip")
        ],
        "an alternative whose id is taken is left out and the ids follow position"
    );
    let twice = attachments.set_seat_offers(seat(1), vec![offer("a", "One"), offer("a", "Two")]);
    assert_eq!(twice, Err(AttachmentError::DuplicateOffer));
}

#[test]
fn a_hero_has_a_portrait_and_a_colour_in_the_party_strip_and_on_the_stage() {
    let mut table = table_in_scene_ten();
    let pip: CastEntry = hero("pip", "Pip", 1, 10, 30)
        .with_portrait(MediaName::new("pip.webp").unwrap())
        .with_color(HeroColor::parse("#4f8cff").unwrap());
    apply(
        &mut table,
        2,
        &[
            TableEffect::CastEntered { entry: pip },
            TableEffect::CastEntered {
                entry: creature("goblin", "Goblin", 10, 60),
            },
        ],
    )
    .unwrap();
    let shared = json_of(&project_shared(&table.snapshot(), &Attachments::new()));
    assert_eq!(shared["view"]["party"][0]["color"], "#4f8cff");
    let elements = shared["view"]["elements"].as_array().unwrap();
    let pip_element = elements.iter().find(|e| e["id"] == "pip").unwrap();
    assert_eq!(pip_element["color"], "#4f8cff");
    assert_eq!(pip_element["fields"]["portrait"], "pip.webp");
    let goblin = elements.iter().find(|e| e["id"] == "goblin").unwrap();
    assert!(goblin.get("color").is_none() && goblin.get("fields").is_none());
}

#[test]
fn a_portrait_is_a_file_name_and_a_colour_is_six_hex_digits_checked_where_they_are_made() {
    assert!(MediaName::new("pip.webp").is_ok());
    assert!(MediaName::new("").is_err());
    assert!(MediaName::new("a/b.png").is_err());
    assert!(MediaName::new("a\\b.png").is_err());
    assert!(MediaName::new("a\nb.png").is_err());
    assert!(MediaName::new(&"p".repeat(257)).is_err());
    assert_eq!(HeroColor::parse("#4F8CFF").unwrap().to_hex(), "#4f8cff");
    for bad in ["4f8cff", "#4f8cf", "#4f8cfg", "red", ""] {
        assert!(HeroColor::parse(bad).is_err(), "{bad}");
    }
    let json = serde_json::to_string(
        &hero("pip", "Pip", 1, 10, 30).with_color(HeroColor::parse("#112233").unwrap()),
    )
    .unwrap();
    assert!(json.contains("\"#112233\""));
    assert!(serde_json::from_str::<CastEntry>(&json.replace("#112233", "nope")).is_err());
}

fn total_text(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        Value::Array(items) => items.iter().map(total_text).sum(),
        Value::Object(fields) => fields.values().map(total_text).sum(),
        Value::Null | Value::Bool(_) | Value::Number(_) => 0,
    }
}

/// The text the wire's `MachineView` carries once: the envelope's own fields, the status words, and
/// the audience's story, note and offers. The playground's lists of choices repeat the offers for
/// the fixture and are not on the wire, so they are not counted twice.
fn wire_text(envelope: &Value) -> usize {
    let view = &envelope["view"];
    let parts = [
        &envelope["machine_id"],
        &envelope["state_label"],
        &envelope["readout"],
        &envelope["reason"],
        &envelope["offers"],
        &view["status"],
        &view["hero"],
        &view["narration"],
        &view["note"],
        &view["place"],
        &view["caption"],
        &view["party"],
    ];
    parts.into_iter().map(total_text).sum()
}

/// Every owner publishes the most the limits allow, and the view still fits the wire frame.
fn fullest_attachments() -> Attachments {
    let mut attachments = Attachments::new();
    let longest = "m".repeat(MAXIMUM_TEXT_BYTES);
    attachments.set_place(scene(10), text(&longest)).unwrap();
    attachments.set_shared_readout(text(&longest)).unwrap();
    attachments
        .set_seat_readout(seat(1), text(&longest))
        .unwrap();
    attachments
        .set_state_label(machine("explore"), text(&longest))
        .unwrap();
    attachments
        .set_activity(
            ProjectionStatus::waiting_for("w", &"w".repeat(MAXIMUM_TEXT_BYTES - 20)).unwrap(),
        )
        .unwrap();
    for number in 0..60 {
        let line = StoryLine::new(text(&longest)).with_narration(Narration::new(&longest));
        attachments.push_public(line.clone()).unwrap();
        attachments.push_private(seat(1), line).unwrap();
        attachments
            .set_private_fact(
                seat(1),
                fact_id(u8::try_from(number + 1).unwrap()),
                Secret::new(longest.clone()),
            )
            .unwrap();
    }
    let per_offer = OFFERS_BUDGET_BYTES / 4;
    let offers = (0..4)
        .map(|number| {
            OfferEntry::new(
                OfferId::new(&format!("o{number}")).unwrap(),
                text(&"o".repeat(per_offer - 4)),
            )
        })
        .collect();
    attachments.set_seat_offers(seat(1), offers).unwrap();
    let mut decline = Decline::new(DeclineCode::NotViable, &longest).unwrap();
    for _ in 0..4 {
        decline = decline.with_alternative(&"a".repeat(per_offer)).unwrap();
    }
    attachments.set_notice(seat(1), decline, at()).unwrap();
    attachments
}

#[test]
fn the_fullest_possible_views_fit_the_wire_frame() {
    let mut table = table_with_two_heroes();
    apply(&mut table, 3, &[set_active(10, "explore")]).unwrap();
    let attachments = fullest_attachments();
    let seat_view = seat_envelope(&table, &attachments, 1);
    let shared_view = json_of(&project_shared(&table.snapshot(), &attachments));
    let seat_bytes = wire_text(&seat_view);
    let shared_bytes = wire_text(&shared_view);
    assert!(
        seat_bytes < WIRE_FRAME_BYTES,
        "a phone view takes {seat_bytes} bytes of text"
    );
    assert!(
        shared_bytes < WIRE_FRAME_BYTES,
        "the shared view takes {shared_bytes} bytes of text"
    );
    assert!(
        seat_view["truncation"]["older_story_entries"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(seat_view["truncation"]["older_facts"].as_u64().unwrap() > 0);
}

#[test]
fn a_truncation_marker_is_zero_when_nothing_was_left_off() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    attachments.push_public(public_line("one")).unwrap();
    let envelope = seat_envelope(&table, &attachments, 1);
    assert_eq!(
        envelope["truncation"],
        json!({"older_story_entries": 0, "older_facts": 0})
    );
}

fn creature_with(visibility: crate::table_state::Visibility, name: &str, hp: u32) -> TableEffect {
    let entry = CastEntry::new(
        crate::table_state_tests::element("lurker"),
        crate::table_state::CastKind::Npc,
        crate::table_state::Controller::Engine,
        scene(10),
        crate::table_state_tests::display_name(name),
        crate::table_state::Vitals::new(hp, 19).unwrap(),
    )
    .at(crate::table_state_tests::place(45, 25))
    .with_visibility(visibility);
    TableEffect::CastEntered { entry }
}

#[test]
fn a_threat_the_party_knows_of_and_cannot_see_is_a_marker_that_names_nothing() {
    use crate::table_state::Visibility;
    let mut table = table_with_two_heroes();
    apply(
        &mut table,
        3,
        &[creature_with(Visibility::Sensed, "Secret Ogre", 12)],
    )
    .unwrap();
    let shared = json_of(&project_shared(&table.snapshot(), &Attachments::new()));
    let elements = shared["view"]["elements"].as_array().unwrap();
    let marker = elements
        .iter()
        .find(|e| e["kind"] == "marker")
        .expect("a marker");
    assert_eq!(marker["id"], "marker-0");
    assert_eq!(marker["label"], "Something unseen");
    assert_eq!(
        (marker["x"].as_f64(), marker["y"].as_f64()),
        (Some(45.0), Some(25.0))
    );
    assert!(!shared.to_string().contains("Ogre") && !shared.to_string().contains("lurker"));
    assert_eq!(elements.len(), 4, "the goblin, two heroes and the marker");
}

#[test]
fn a_sensed_creature_changes_nothing_on_the_shared_screen_but_its_place() {
    use crate::table_state::Visibility;
    let build = |name: &str, hp: u32| {
        let mut table = table_with_two_heroes();
        apply(
            &mut table,
            3,
            &[creature_with(Visibility::Sensed, name, hp)],
        )
        .unwrap();
        json_of(&project_shared(&table.snapshot(), &Attachments::new()))
    };
    assert_eq!(build("Ogre", 12), build("Troll", 3));
}

#[test]
fn a_hidden_hero_sees_where_it_stands_on_its_own_phone_and_nobody_else_does() {
    use crate::table_state::{CastChange, Visibility};
    let mut table = table_with_two_heroes();
    let plain = seat_envelope(&table, &Attachments::new(), 1);
    assert!(
        plain.get("overlay").is_none(),
        "a seen hero needs no overlay"
    );
    apply(
        &mut table,
        3,
        &[TableEffect::CastChanged {
            cast: crate::table_state_tests::element("pip"),
            change: CastChange::Visibility {
                visibility: Visibility::Hidden,
            },
        }],
    )
    .unwrap();
    let own = seat_envelope(&table, &Attachments::new(), 1);
    assert_eq!(own["overlay"], json!({"x": 30.0, "y": 20.0}));
    assert!(
        seat_envelope(&table, &Attachments::new(), 2)
            .get("overlay")
            .is_none()
    );
    assert!(
        json_of(&project_shared(&table.snapshot(), &Attachments::new()))
            .get("overlay")
            .is_none()
    );
}
